//! The top bar's tray: one list of everything the bar can show. Each item describes itself
//! once: its id, icon, name and state line, its group, its default visibility and when it is
//! needed now, and what opening it does. The bar and the tray never name an item: the view
//! model gets the list sorted, with each item's place in the bar resolved
//! ([`launcher_ui_api::TrayItemVm::in_bar`]), and the kit draws whatever is in it.
//!
//! The player chooses each item's visibility ([`Show`]). The choices are saved by item id
//! under `app.tray` in the config, one key per item: an item the player never chose has its
//! own default, and a saved choice for an id the core doesn't list is ignored.
//!
//! The items come from sources ([`SOURCES`]): the system areas, the downloads and the
//! shortcuts. A new source, such as a plugin's items, is one more function in that list.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::changes::Area;
use crate::config::ConfigService;
use crate::{CoreError, CoreResult};

/// Where an item shows, as the player chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Show {
    /// In the bar.
    Always,
    /// In the bar only while it matters.
    Needed,
    /// Only in the tray.
    Tray,
}

impl Show {
    pub fn as_str(self) -> &'static str {
        match self {
            Show::Always => "always",
            Show::Needed => "needed",
            Show::Tray => "tray",
        }
    }
}

/// The player's choices, by item id.
pub(crate) type Choices = BTreeMap<String, Show>;

/// Where the choices are in the config.
const KEY: [&str; 2] = ["app", "tray"];

/// Every item id the core lists, in the tray's order.
pub const IDS: [&str; 10] =
    ["network", "audio", "bluetooth", "drives", "vpn", "os-updates", "power", "downloads", "emulators", "themes"];

/// The saved choices. A value that isn't a choice is ignored, so the item keeps its default.
pub(crate) fn load(config: &ConfigService) -> Choices {
    let saved = config.get(&KEY);
    let Some(saved) = saved.as_object() else { return Choices::new() };
    saved.iter().filter_map(|(id, show)| Some((id.clone(), serde_json::from_value(show.clone()).ok()?))).collect()
}

/// Saves the player's choice for the item `id`, and only for it: the other items keep theirs.
pub(crate) fn choose(core: &crate::Core, id: &str, show: Show) -> CoreResult<()> {
    if !IDS.contains(&id) {
        return Err(CoreError::Invalid(format!("the tray has no item {id}")));
    }
    let key = [KEY[0], KEY[1], id];
    core.kernel().config().set(&key, Value::from(show.as_str())).map_err(|e| CoreError::Io(e.to_string()))?;
    core.changed(Area::Tray);
    Ok(())
}

/// The two groups, in the tray's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    /// The machine's areas and the downloads: what has a state to watch.
    Status,
    /// Pages: emulators and themes.
    Shortcuts,
}

/// One item, as its source describes it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub id: &'static str,
    pub group: Group,
    /// Where it shows until the player chooses.
    pub default: Show,
    /// It matters now: a "When needed" item is in the bar.
    pub needed: bool,
    /// The tray lists it. Only a battery the PC doesn't have isn't: it stays in the list so the
    /// bar can close its place as it goes.
    pub listed: bool,
    pub icon: String,
    pub name: String,
    pub text: String,
    /// loading | absent | unsupported | ready
    pub state: &'static str,
    pub reason: String,
    pub busy: bool,
    pub opens: &'static str,
    pub target: &'static str,
    pub value: String,
}

impl Item {
    /// A ready item with nothing to say yet.
    fn new(id: &'static str, group: Group, default: Show, name: &str, icon: &str) -> Self {
        Item {
            id,
            group,
            default,
            needed: false,
            listed: true,
            icon: icon.into(),
            name: name.into(),
            text: String::new(),
            state: "ready",
            reason: String::new(),
            busy: false,
            opens: "",
            target: "",
            value: String::new(),
        }
    }

    fn opens(mut self, opens: &'static str, target: &'static str) -> Self {
        self.opens = opens;
        self.target = target;
        self
    }

    /// The area can't be used here: never in the bar, with why.
    fn unusable(&self) -> bool {
        self.state == "absent" || self.state == "unsupported"
    }

    /// The bar shows it now, by the player's choice or the item's default.
    fn in_bar(&self, choices: &Choices) -> bool {
        if !self.listed || self.unusable() {
            return false;
        }
        match choices.get(self.id).copied().unwrap_or(self.default) {
            Show::Always => true,
            Show::Needed => self.needed,
            Show::Tray => false,
        }
    }
}

/// What the downloads and installs are doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Downloads {
    /// How far each running one is, 0 to 1; `None` when its size isn't known.
    pub running: Vec<Option<f32>>,
    /// The finished ones, by id: `download:3`, `install:5`, `torrent:<hash>`.
    pub finished: BTreeSet<String>,
}

/// What the items are built from.
pub(crate) struct Facts<'a> {
    pub system: &'a crate::system::SystemState,
    pub drives: &'a [crate::drives::Drive],
    pub downloads: Downloads,
    /// How many finished downloads the player hasn't seen: not since they finished, nor while on
    /// Downloads.
    pub unseen: usize,
    /// How many emulators are on, and the theme in use.
    pub emulators: usize,
    pub theme: String,
}

/// The sources of items, in the tray's order within a group.
pub(crate) const SOURCES: [fn(&Facts) -> Vec<Item>; 3] = [areas, downloads, shortcuts];

/// Every item, sorted: Status first, then Shortcuts, each source's items in its order.
pub(crate) fn items(facts: &Facts) -> Vec<Item> {
    let mut items: Vec<Item> = SOURCES.iter().flat_map(|source| source(facts)).collect();
    items.sort_by_key(|i| i.group);
    items
}

/// The tray for the view: every item, then the bar's and the tray's items by their place.
pub(crate) fn vm(facts: &Facts, choices: &Choices) -> (Vec<launcher_ui_api::TrayItemVm>, Vec<i32>, Vec<i32>) {
    let items = items(facts);
    let place = |pick: &dyn Fn(&Item) -> bool| {
        items.iter().enumerate().filter(|(_, i)| pick(i)).map(|(at, _)| at as i32).collect::<Vec<_>>()
    };
    let bar = place(&|i| i.in_bar(choices));
    let rows = place(&|i| i.listed);
    let vms = items
        .iter()
        .map(|i| launcher_ui_api::TrayItemVm {
            id: i.id.into(),
            icon: i.icon.clone(),
            name: i.name.clone(),
            text: i.text.clone(),
            group: match i.group {
                Group::Status => "status",
                Group::Shortcuts => "shortcuts",
            }
            .into(),
            show: choices.get(i.id).copied().unwrap_or(i.default).as_str().into(),
            in_bar: i.in_bar(choices),
            state: i.state.into(),
            reason: i.reason.clone(),
            busy: i.busy,
            opens: i.opens.into(),
            target: i.target.into(),
            value: i.value.clone(),
        })
        .collect();
    (vms, bar, rows)
}

/// What the downloads, the installs and the catalogs' downloads are doing now.
pub(crate) fn downloads_of(
    jobs: &[crate::Job],
    installs: &crate::game_installs::InstallsSnapshot,
    torrents: &crate::torrents::TorrentsSnapshot,
) -> Downloads {
    use crate::JobPhase as P;
    use crate::game_installs::GamePhase;
    let mut downloads = Downloads::default();
    let part = |done: u64, total: u64| (total > 0).then(|| (done as f32 / total as f32).min(1.0));
    for job in jobs {
        match job.phase {
            P::Queued | P::Downloading | P::Verifying | P::Unpacking => {
                downloads.running.push(job.total.and_then(|t| part(job.received, t)))
            }
            P::Done => _ = downloads.finished.insert(format!("download:{}", job.id.0)),
            P::Paused | P::Failed | P::Cancelled => {}
        }
    }
    for (id, job) in installs.jobs.iter() {
        match job.phase {
            GamePhase::Installing => downloads.running.push(part(job.done, job.total)),
            GamePhase::Done => _ = downloads.finished.insert(format!("install:{}", id.0)),
            GamePhase::Failed | GamePhase::Cancelled => {}
        }
    }
    for job in torrents.jobs.values() {
        if job.state.active() {
            downloads.running.push(part(job.done, job.total));
        } else if job.state == crate::torrents::TorrentState::Complete {
            downloads.finished.insert(format!("torrent:{}", job.id));
        }
    }
    downloads
}

/// The player has seen every download that is finished now: at the start, and as Downloads
/// opens or closes.
pub(crate) fn seen_now(core: &crate::Core) {
    let finished =
        downloads_of(&core.download_jobs(), &core.installs().snapshot(), &core.torrents().snapshot()).finished;
    core.state().downloads_seen.extend(finished);
}

/// The tray of the snapshot `s`, for the view.
pub(crate) fn view(s: &crate::snapshot::Snapshot) -> (Vec<launcher_ui_api::TrayItemVm>, Vec<i32>, Vec<i32>) {
    let downloads = downloads_of(&s.downloads, &s.installs, &s.torrents);
    // On Downloads the player sees them.
    let unseen = if s.state.stack.route() == &crate::Route::Downloads {
        0
    } else {
        downloads.finished.difference(&s.state.downloads_seen).count()
    };
    let theme = s.state.theme.as_ref().and_then(|t| s.plugins.manifest(t)).map(|m| m.name.clone()).unwrap_or_default();
    let facts = Facts {
        system: &s.state.system,
        drives: &s.drives,
        downloads,
        unseen,
        emulators: s.plugins.of_kind(launcher_api::PluginKind::Emulator).len(),
        theme,
    };
    vm(&facts, &s.tray)
}

/// A system area's item: its state, and its reason or "Checking…" while it can't say more.
/// `ready` fills in the rest from the area's data.
fn area_item(
    facts: &Facts,
    area: launcher_api::SystemArea,
    item: Item,
    ready: impl FnOnce(&mut Item, &launcher_api::AreaData),
) -> Item {
    use launcher_api::AreaState;
    let mut item = item;
    item.busy = facts.system.jobs.iter().any(|j| j.phase == crate::system::JobPhase::Running && j.kind.area() == area);
    match facts.system.area(area) {
        None => {
            item.state = "loading";
            item.text = "Checking…".into();
        }
        Some(AreaState::Absent) => {
            item.state = "absent";
            item.reason = crate::system::absent(area);
            item.text = item.reason.clone();
        }
        Some(AreaState::Unsupported(why)) => {
            item.state = "unsupported";
            item.reason = why.clone();
            item.text = why.clone();
        }
        Some(AreaState::Ready(data)) => ready(&mut item, data),
    }
    item
}

/// A running job of `kind` in the machine.
fn running(facts: &Facts, kind: crate::system::JobKind) -> bool {
    facts.system.jobs.iter().any(|j| j.phase == crate::system::JobPhase::Running && j.kind == kind)
}

/// The machine's areas: the network, the sound, Bluetooth, the drives, the VPN, the OS's
/// updates and the battery. The OS's installer has no item: its USB stick shows it.
fn areas(facts: &Facts) -> Vec<Item> {
    use Group::Status;
    use Show::{Always, Needed};
    use launcher_api::{AreaData as D, Connection, OsUpdate, SystemArea as A};
    let network = area_item(
        facts,
        A::Network,
        Item::new("network", Status, Always, "Internet", "wifi-4").opens("panel", "network"),
        |item, data| {
            let D::Network { status, .. } = data else { return };
            let status = status.clone().unwrap_or_default();
            let bars = status.signal.map_or(0, crate::view::bars);
            (item.icon, item.text, item.value) = match status.connection {
                Connection::Wifi => {
                    (format!("wifi-{bars}"), format!("Wi-Fi · {}", status.name), format!("wifi {bars}"))
                }
                Connection::Wired => ("wired".into(), format!("Cable · {}", status.name), "wired".into()),
                Connection::None => ("wifi-off".into(), "Not connected".into(), "none".into()),
            };
        },
    );
    let audio = area_item(
        facts,
        A::Audio,
        Item::new("audio", Status, Needed, "Sound", "volume-2").opens("panel", "sound"),
        |item, data| {
            let D::Audio { level, .. } = data else { return };
            let level = level.unwrap_or_default();
            item.needed = level.muted;
            item.icon = match level.volume {
                _ if level.muted => "volume-mute",
                0 => "volume-0",
                1..=49 => "volume-1",
                _ => "volume-2",
            }
            .into();
            (item.text, item.value) = if level.muted {
                ("Muted".into(), "muted".into())
            } else {
                (format!("{}%", level.volume), level.volume.to_string())
            };
        },
    );
    let bluetooth = area_item(
        facts,
        A::Bluetooth,
        Item::new("bluetooth", Status, Needed, "Bluetooth", "bluetooth").opens("panel", "bluetooth"),
        |item, data| {
            let D::Bluetooth { powered, devices } = data else { return };
            let connected = devices.iter().filter(|d| d.connected).count();
            item.needed = connected > 0 || item.busy;
            item.icon = if !powered {
                "bluetooth-off"
            } else if connected > 0 {
                "bluetooth-on"
            } else {
                "bluetooth"
            }
            .into();
            item.value = if *powered { format!("on {connected}") } else { "off".into() };
            item.text = if running(facts, crate::system::JobKind::Pair) {
                "Pairing…".into()
            } else if running(facts, crate::system::JobKind::ScanBluetooth) {
                "Looking for devices…".into()
            } else if !powered {
                "Off".into()
            } else if connected > 0 {
                format!("{connected} connected")
            } else {
                "On".into()
            };
        },
    );
    let drives = area_item(
        facts,
        A::Drives,
        Item::new("drives", Status, Needed, "Drives", "usb").opens("panel", "drives"),
        |item, _| {
            let removable = facts.drives.iter().filter(|d| d.usb).count();
            let ejecting = running(facts, crate::system::JobKind::Eject);
            item.needed = removable > 0 || ejecting;
            item.value = removable.to_string();
            item.text = if ejecting {
                "Ejecting…".into()
            } else if removable > 0 {
                format!("{removable} plugged in")
            } else if facts.drives.is_empty() {
                "No drives".into()
            } else {
                format!("{} inside this PC", facts.drives.len())
            };
        },
    );
    let vpn = area_item(
        facts,
        A::Vpn,
        Item::new("vpn", Status, Needed, "VPN", "shield").opens("settings", "Downloads"),
        |item, data| {
            let D::Vpn { vpns, .. } = data else { return };
            let on = vpns.iter().find(|v| v.connected);
            item.needed = on.is_some();
            (item.text, item.value) = match on {
                Some(vpn) => (format!("Connected · {}", vpn.name), "connected".into()),
                None => ("Not connected".into(), "off".into()),
            };
        },
    );
    let updates = area_item(
        facts,
        A::OsUpdates,
        Item::new(
            "os-updates",
            Status,
            Needed,
            &format!("{} updates", launcher_api::brand::PRODUCT_OS_NAME),
            "refresh",
        )
        .opens("settings", "System"),
        |item, data| {
            let D::OsUpdates { update } = data else { return };
            (item.text, item.value) = match update {
                None | Some(OsUpdate::Current) => ("Up to date".into(), "current".into()),
                Some(OsUpdate::Checking) => ("Looking for an update…".into(), "checking".into()),
                Some(OsUpdate::Staged { version }) => (format!("Restart to update to {version}"), "staged".into()),
                Some(OsUpdate::Failed { .. }) => ("Couldn't look for an update".into(), "failed".into()),
            };
            item.needed = matches!(update, Some(OsUpdate::Staged { .. }));
        },
    );
    let mut power = area_item(
        facts,
        A::Power,
        Item::new("power", Status, Always, "Battery", "battery").opens("power", ""),
        |item, data| {
            let D::Power { battery, .. } = data else { return };
            item.listed = battery.is_some();
            if let Some(b) = battery {
                item.text = if b.charging { format!("{}% · Charging", b.percent) } else { format!("{}%", b.percent) };
                item.value = if b.charging { format!("{} charging", b.percent) } else { b.percent.to_string() };
            }
        },
    );
    // A battery only shows on a PC that has one: the power area that can't be used has none to
    // show. While it is read, it may be there.
    if power.unusable() {
        power.listed = false;
    }
    vec![network, audio, bluetooth, drives, vpn, updates, power]
}

/// The downloads, the installs and the catalogs' downloads, as one item.
fn downloads(facts: &Facts) -> Vec<Item> {
    let mut item =
        Item::new("downloads", Group::Status, Show::Needed, "Downloads", "download").opens("screen", "downloads");
    let running = &facts.downloads.running;
    item.busy = !running.is_empty();
    item.needed = item.busy || facts.unseen > 0;
    let known: Vec<f32> = running.iter().flatten().copied().collect();
    item.text = if !running.is_empty() && !known.is_empty() {
        let percent = (known.iter().sum::<f32>() / known.len() as f32 * 100.0).round();
        format!("{} running, {percent}%", running.len())
    } else if !running.is_empty() {
        format!("{} running", running.len())
    } else if facts.unseen > 0 {
        format!("{} finished", facts.unseen)
    } else {
        "Nothing running".into()
    };
    item.value = format!("{} running, {} finished", running.len(), facts.unseen);
    vec![item]
}

/// Pages, not status: the emulators and the themes.
fn shortcuts(facts: &Facts) -> Vec<Item> {
    let mut emulators =
        Item::new("emulators", Group::Shortcuts, Show::Tray, "Emulators", "gamepad").opens("screen", "emulators");
    emulators.text = format!("{} turned on", facts.emulators);
    let mut themes = Item::new("themes", Group::Shortcuts, Show::Tray, "Themes", "palette").opens("screen", "themes");
    themes.text = facts.theme.clone();
    vec![emulators, themes]
}

#[cfg(test)]
mod tests;

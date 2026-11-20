use std::collections::BTreeMap;

use launcher_api::{
    AreaData, AreaState, AudioLevel, Battery, BluetoothDevice, Connection, NetworkStatus, OsUpdate, SystemArea, Vpn,
};
use launcher_ui_api::TrayItemVm;

use super::*;
use crate::system::SystemState;

/// A machine with every area ready: a wired network, the sound at 40, Bluetooth on with nothing
/// connected, no drive, no VPN connected, the OS up to date and a battery at 65%.
fn machine() -> SystemState {
    let mut system = SystemState::default();
    let ready = |data| AreaState::Ready(data);
    let areas = [
        (
            SystemArea::Network,
            ready(AreaData::Network {
                status: Some(NetworkStatus { connection: Connection::Wired, name: "Home".into(), signal: None }),
                wifi: Vec::new(),
            }),
        ),
        (
            SystemArea::Audio,
            ready(AreaData::Audio { outputs: Vec::new(), level: Some(AudioLevel { volume: 40, muted: false }) }),
        ),
        (SystemArea::Bluetooth, ready(AreaData::Bluetooth { powered: true, devices: Vec::new() })),
        (SystemArea::Drives, ready(AreaData::Drives { drives: Vec::new() })),
        (SystemArea::Vpn, ready(AreaData::Vpn { finds_vpns: true, can_import: false, vpns: Vec::new() })),
        (SystemArea::OsUpdates, ready(AreaData::OsUpdates { update: Some(OsUpdate::Current) })),
        (
            SystemArea::Power,
            ready(AreaData::Power { battery: Some(Battery { percent: 65, charging: false }), refused: None }),
        ),
        (SystemArea::OsInstall, AreaState::Absent),
    ];
    system.areas = areas.into_iter().collect();
    system
}

struct Tray {
    items: Vec<TrayItemVm>,
    bar: Vec<i32>,
    rows: Vec<i32>,
}

impl Tray {
    fn ids(&self, places: &[i32]) -> Vec<&str> {
        places.iter().map(|&i| self.items[i as usize].id.as_str()).collect()
    }
    fn bar(&self) -> Vec<&str> {
        self.ids(&self.bar)
    }
    fn rows(&self) -> Vec<&str> {
        self.ids(&self.rows)
    }
    fn item(&self, id: &str) -> &TrayItemVm {
        self.items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no {id}"))
    }
}

fn tray_of(
    system: &SystemState,
    drives: &[crate::drives::Drive],
    downloads: Downloads,
    unseen: usize,
    choices: &Choices,
) -> Tray {
    let facts =
        Facts { system, drives, downloads, unseen, emulators: 2, theme: "Classic".into(), vpn_chosen: String::new() };
    let (items, bar, rows) = vm(&facts, choices);
    Tray { items, bar, rows }
}

fn tray(system: &SystemState) -> Tray {
    tray_of(system, &[], Downloads::default(), 0, &Choices::new())
}

fn choices(pairs: &[(&str, Show)]) -> Choices {
    pairs.iter().map(|(id, show)| ((*id).to_owned(), *show)).collect()
}

fn data(system: &mut SystemState, area: SystemArea) -> &mut AreaData {
    match system.areas.get_mut(&area) {
        Some(AreaState::Ready(data)) => data,
        other => panic!("{area:?} isn't ready: {other:?}"),
    }
}

#[test]
fn the_tray_lists_the_status_items_then_the_shortcuts_each_in_its_group() {
    let tray = tray(&machine());
    let rows = tray.rows();
    assert_eq!(
        rows,
        ["network", "audio", "bluetooth", "drives", "vpn", "os-updates", "power", "downloads", "emulators", "themes"]
    );
    let groups: Vec<&str> = tray.items.iter().map(|i| i.group.as_str()).collect();
    assert_eq!(groups[..8], ["status"; 8]);
    assert_eq!(groups[8..], ["shortcuts"; 2]);
    assert_eq!(rows, IDS, "IDS lists every item in the tray's order");
}

#[test]
fn at_rest_the_bar_has_the_internet_and_the_battery() {
    let tray = tray(&machine());
    assert_eq!(tray.bar(), ["network", "power"]);
    let shows: BTreeMap<&str, &str> = tray.items.iter().map(|i| (i.id.as_str(), i.show.as_str())).collect();
    for id in ["network", "power"] {
        assert_eq!(shows[id], "always", "{id}");
    }
    for id in ["audio", "bluetooth", "drives", "vpn", "os-updates", "downloads"] {
        assert_eq!(shows[id], "needed", "{id}");
    }
    for id in ["emulators", "themes"] {
        assert_eq!(shows[id], "tray", "{id}");
    }
    assert!(
        tray.items
            .iter()
            .all(|i| i.in_bar == tray.bar.contains(&(tray.items.iter().position(|j| j.id == i.id).unwrap() as i32)))
    );
}

#[test]
fn each_choice_puts_an_item_in_the_bar_or_out_of_it() {
    let system = machine();
    let picked = choices(&[("network", Show::Tray), ("audio", Show::Always), ("themes", Show::Always)]);
    let tray = tray_of(&system, &[], Downloads::default(), 0, &picked);
    assert_eq!(tray.bar(), ["audio", "power", "themes"], "the bar keeps the tray's order");
    assert_eq!(tray.item("network").show, "tray");
    assert!(!tray.item("network").in_bar);
    // When needed, and not needed now: out of the bar.
    let picked = choices(&[("power", Show::Needed)]);
    assert_eq!(tray_of(&system, &[], Downloads::default(), 0, &picked).bar(), ["network"]);
}

#[test]
fn each_needed_now_rule_brings_its_item_into_the_bar() {
    let base = machine();
    let rest = tray(&base);
    let at_rest = rest.bar();

    let mut muted = base.clone();
    if let AreaData::Audio { level, .. } = data(&mut muted, SystemArea::Audio) {
        level.as_mut().unwrap().muted = true;
    }
    let tray_muted = tray(&muted);
    assert!(tray_muted.item("audio").in_bar, "muted");
    assert_eq!(tray_muted.item("audio").text, "Muted");

    let mut connected = base.clone();
    if let AreaData::Bluetooth { devices, .. } = data(&mut connected, SystemArea::Bluetooth) {
        devices.push(BluetoothDevice { address: "AA".into(), name: "DualSense".into(), paired: true, connected: true });
    }
    assert!(tray(&connected).item("bluetooth").in_bar, "a device is connected");

    let mut vpn = base.clone();
    if let AreaData::Vpn { vpns, .. } = data(&mut vpn, SystemArea::Vpn) {
        vpns.push(Vpn { name: "Proton".into(), kind: "WireGuard".into(), connected: true, ..Vpn::default() });
    }
    let tray_vpn = tray(&vpn);
    assert!(tray_vpn.item("vpn").in_bar, "the VPN is connected");
    assert_eq!(tray_vpn.item("vpn").text, "Connected · Proton");

    let mut staged = base.clone();
    *data(&mut staged, SystemArea::OsUpdates) =
        AreaData::OsUpdates { update: Some(OsUpdate::Staged { version: "2026.11.20".into() }) };
    assert!(tray(&staged).item("os-updates").in_bar, "an update waits for a restart");

    let stick = crate::drives::Drive { id: "STICK".into(), label: "STICK".into(), usb: true, ..Default::default() };
    let inside = crate::drives::Drive { id: "disk".into(), label: "Games disk".into(), ..Default::default() };
    let none = Downloads::default();
    assert!(
        !tray_of(&base, std::slice::from_ref(&inside), none.clone(), 0, &Choices::new()).item("drives").in_bar,
        "a disk inside the PC isn't news"
    );
    assert!(
        tray_of(&base, &[inside, stick], none.clone(), 0, &Choices::new()).item("drives").in_bar,
        "a removable drive is plugged in"
    );

    let running = Downloads { running: vec![Some(0.25), Some(0.55)], ..Downloads::default() };
    let tray_running = tray_of(&base, &[], running, 0, &Choices::new());
    assert!(tray_running.item("downloads").in_bar, "downloads run");
    assert_eq!(tray_running.item("downloads").text, "2 running, 40%");
    assert!(tray_running.item("downloads").busy);
    let tray_done = tray_of(&base, &[], none, 1, &Choices::new());
    assert!(tray_done.item("downloads").in_bar, "one finished and the player hasn't seen it");
    assert_eq!(tray_done.item("downloads").text, "1 finished");

    assert_eq!(at_rest, ["network", "power"], "none of them at rest");
}

#[test]
fn a_long_job_brings_its_item_into_the_bar() {
    let mut system = machine();
    let job = |kind| crate::system::JobState {
        id: 1,
        kind,
        target: String::new(),
        drive: String::new(),
        phase: crate::system::JobPhase::Running,
        percent: None,
        text: String::new(),
        cancel: Default::default(),
    };
    system.jobs = vec![job(crate::system::JobKind::ScanBluetooth)];
    assert!(tray(&system).item("bluetooth").in_bar, "a scan runs");
    system.jobs = vec![job(crate::system::JobKind::Eject)];
    let ejecting = tray(&system);
    assert!(ejecting.item("drives").in_bar, "an eject runs");
    assert!(ejecting.item("drives").busy);
}

#[test]
fn an_area_that_can_t_be_used_is_never_in_the_bar_and_says_why() {
    let mut system = machine();
    system.areas.insert(SystemArea::Bluetooth, AreaState::Unsupported("No Bluetooth adapter".into()));
    system.areas.insert(SystemArea::Vpn, AreaState::Absent);
    system.areas.insert(SystemArea::Network, AreaState::Unsupported("NetworkManager isn't installed".into()));
    let picked = choices(&[("bluetooth", Show::Always), ("vpn", Show::Always)]);
    let tray = tray_of(&system, &[], Downloads::default(), 0, &picked);
    assert_eq!(tray.bar(), ["power"], "not even when the player chose Always");
    let bluetooth = tray.item("bluetooth");
    assert_eq!((bluetooth.state.as_str(), bluetooth.reason.as_str()), ("unsupported", "No Bluetooth adapter"));
    assert_eq!(bluetooth.text, "No Bluetooth adapter", "its line says why");
    assert_eq!(tray.item("vpn").state, "absent");
    assert!(!tray.item("vpn").reason.is_empty(), "an absent area says why too");
    assert!(tray.rows().contains(&"bluetooth") && tray.rows().contains(&"vpn"), "the tray still lists them");
}

#[test]
fn a_loading_area_keeps_its_place_by_its_choice_and_is_never_needed_yet() {
    let mut system = machine();
    system.areas.remove(&SystemArea::Network);
    system.areas.remove(&SystemArea::Audio);
    system.areas.remove(&SystemArea::Power);
    let picked = choices(&[("audio", Show::Needed)]);
    let tray = tray_of(&system, &[], Downloads::default(), 0, &picked);
    assert_eq!(tray.bar(), ["network", "power"], "Always items show while they load; a needed one waits");
    for id in ["network", "audio", "power"] {
        assert_eq!(tray.item(id).state, "loading", "{id}");
        assert_eq!(tray.item(id).text, "Checking…", "{id}");
    }
    assert!(tray.rows().contains(&"power"), "the battery may be there: it is listed while it loads");
}

#[test]
fn a_pc_without_a_battery_doesn_t_list_one() {
    let mut system = machine();
    *data(&mut system, SystemArea::Power) = AreaData::Power { battery: None, refused: None };
    let tray = tray(&system);
    assert!(!tray.rows().contains(&"power"), "{:?}", tray.rows());
    assert_eq!(tray.bar(), ["network"]);
    let mut system = machine();
    system.areas.insert(SystemArea::Power, AreaState::Unsupported("No power here".into()));
    assert!(!self::tray(&system).rows().contains(&"power"), "nor where the power area can't be used");
}

#[test]
fn a_battery_says_its_level_and_whether_it_charges() {
    let mut system = machine();
    *data(&mut system, SystemArea::Power) =
        AreaData::Power { battery: Some(Battery { percent: 18, charging: true }), refused: None };
    let power = tray(&system).items.into_iter().find(|i| i.id == "power").unwrap();
    assert_eq!(
        (power.text.as_str(), power.value.as_str(), power.icon.as_str()),
        ("18% · Charging", "18 charging", "battery")
    );
}

#[test]
fn each_item_says_what_opening_it_does() {
    let tray = tray(&machine());
    let opens: Vec<(&str, &str, &str)> =
        tray.items.iter().map(|i| (i.id.as_str(), i.opens.as_str(), i.target.as_str())).collect();
    assert_eq!(
        opens,
        [
            ("network", "panel", "network"),
            ("audio", "panel", "sound"),
            ("bluetooth", "panel", "bluetooth"),
            ("drives", "panel", "drives"),
            ("vpn", "settings", "Downloads"),
            ("os-updates", "settings", "System"),
            ("power", "power", ""),
            ("downloads", "screen", "downloads"),
            ("emulators", "screen", "emulators"),
            ("themes", "screen", "themes"),
        ]
    );
}

#[test]
fn the_state_lines_say_what_each_item_is_now() {
    let tray = tray(&machine());
    let text = |id: &str| tray.item(id).text.clone();
    assert_eq!(text("network"), "Cable · Home");
    assert_eq!(text("audio"), "40%");
    assert_eq!(text("bluetooth"), "On");
    assert_eq!(text("drives"), "No drives");
    assert_eq!(text("vpn"), "Not connected");
    assert_eq!(text("power"), "65%");
    assert_eq!(text("downloads"), "Nothing running");
    assert_eq!(text("emulators"), "2 turned on");
    assert_eq!(text("themes"), "Classic");
    assert_eq!(tray.item("network").value, "wired");
    assert_eq!(tray.item("audio").value, "40");
    assert_eq!(tray.item("bluetooth").value, "on 0");
}

#[test]
fn saved_choices_are_read_by_id_and_the_rest_keep_their_default() {
    let config = ConfigService::in_memory();
    config
        .set(&["app", "tray"], serde_json::json!({"audio": "always", "gone-item": "always", "vpn": "sideways"}))
        .unwrap();
    let saved = load(&config);
    assert_eq!(saved.get("audio"), Some(&Show::Always));
    assert_eq!(saved.get("vpn"), None, "a value that isn't a choice is ignored");
    let tray = tray_of(&machine(), &[], Downloads::default(), 0, &saved);
    assert_eq!(tray.bar(), ["network", "audio", "power"]);
    assert_eq!(tray.item("vpn").show, "needed", "its default");
    assert!(tray.items.iter().all(|i| i.id != "gone-item"), "an id the core doesn't list is ignored");
}

#[test]
fn bluetooth_turned_off_has_nothing_connected_and_leaves_the_bar() {
    let mut system = machine();
    *data(&mut system, SystemArea::Bluetooth) = AreaData::Bluetooth {
        powered: false,
        devices: vec![BluetoothDevice {
            address: "AA".into(),
            name: "DualSense".into(),
            paired: true,
            connected: true,
        }],
    };
    let tray = tray(&system);
    let bluetooth = tray.item("bluetooth");
    assert!(!bluetooth.in_bar, "off: the device it last listed isn't connected");
    assert_eq!(
        (bluetooth.text.as_str(), bluetooth.value.as_str(), bluetooth.icon.as_str()),
        ("Off", "off", "bluetooth-off")
    );
}

#[test]
fn where_the_system_can_t_tell_vpns_apart_only_the_picked_one_counts() {
    let mut system = machine();
    *data(&mut system, SystemArea::Vpn) = AreaData::Vpn {
        finds_vpns: false,
        can_import: false,
        vpns: vec![Vpn { name: "Wi-Fi".into(), kind: "Network".into(), connected: true, ..Vpn::default() }],
    };
    let none = || Downloads::default();
    let facts = |chosen: &str| Facts {
        system: &system,
        drives: &[],
        downloads: none(),
        unseen: 0,
        emulators: 0,
        theme: String::new(),
        vpn_chosen: chosen.into(),
    };
    let in_bar = |facts: Facts| vm(&facts, &Choices::new()).0.into_iter().find(|i| i.id == "vpn").unwrap().in_bar;
    assert!(!in_bar(facts("")), "a network with an address isn't a VPN");
    assert!(!in_bar(facts("Proton")), "the picked one isn't connected");
    assert!(in_bar(facts("Wi-Fi")), "the picked one is connected");
}

#[test]
fn a_change_of_the_choices_moves_no_game_list() {
    assert!(!crate::view::moves_game_lists(crate::changes::Area::Tray));
}

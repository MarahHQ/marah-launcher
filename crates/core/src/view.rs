//! Building view models from a [`Snapshot`] of the app. [`build`] is pure: it takes no lock and
//! does no I/O, so a render is a function of the snapshot.

use std::collections::BTreeMap;

use launcher_api::{
    ActionStyle, Capability, CompatInfo, CompatLevel, GameKey, Platform, PluginId, PluginKind, SlotContent, SlotId,
    SlotItem, Tone,
};
use launcher_ui_api::*;
use serde_json::Value;

use crate::app::{Grid, Route};
use crate::changes::Area;
use crate::game_installs::InstallsSnapshot;
use crate::grid_query::{GridData, GridGames, GridQuery};
use crate::library::{Entry, LibrarySnapshot, PluginImage, image_ref};
use crate::order::sort_by_order;
use crate::ownership::Ownership;
use crate::room::Need;
use crate::snapshot::{Plugins, Snapshot};
use crate::torrents::TorrentJob;
use crate::{JobPhase, PluginStatus};

/// A plugin's image as the theme host fetches it.
fn plugin_image((plugin, source): PluginImage) -> ImageRef {
    image_ref(&plugin, &source)
}

/// One plugin's slot items as view models, in [`launcher_api::Order`] order.
pub(crate) fn slot_vms(plugin: &PluginId, slot: SlotId, items: Vec<SlotItem>) -> Vec<SlotVm> {
    let keyed = items.into_iter().map(|item| (item.id.clone(), item.order.clone(), item)).collect();
    sort_by_order(keyed).into_iter().map(|item| slot_vm(plugin, slot, item)).collect()
}

fn slot_vm(plugin: &PluginId, slot: SlotId, item: SlotItem) -> SlotVm {
    let mut vm = SlotVm {
        slot,
        plugin: plugin.clone(),
        label: format!("{plugin}:{}", item.id),
        id: item.id,
        kind: SlotKind::Badge,
        text: String::new(),
        short: String::new(),
        tone: Tone::Neutral,
        style: ActionStyle::Secondary,
        icon: None,
        rows: Vec::new(),
        action: None,
        args: Value::Null,
        scope: None,
    };
    match item.content {
        SlotContent::Badge { label, short, tone, icon } => {
            vm.short = if short.is_empty() { label.clone() } else { short };
            vm.text = label;
            vm.tone = tone;
            vm.icon = icon.map(|i| image_ref(plugin, &i));
        }
        SlotContent::Action { label, action, args, style, icon } => {
            vm.kind = SlotKind::Action;
            vm.text = label;
            vm.action = Some(action);
            vm.args = args;
            vm.style = style;
            vm.icon = icon.map(|i| image_ref(plugin, &i));
        }
        SlotContent::Section { title, rows } => {
            vm.kind = SlotKind::Section;
            vm.text = title;
            vm.rows = rows.into_iter().map(|r| RowVm { label: r.label, value: r.value, ..RowVm::plain() }).collect();
        }
        SlotContent::Block { title, rows } => {
            vm.kind = SlotKind::Block;
            vm.text = title;
            vm.rows = rows
                .into_iter()
                .map(|r| RowVm {
                    label: r.label,
                    value: r.value,
                    hint: r.hint,
                    tone: r.tone,
                    progress: r.progress.map_or(-1.0, |p| p.clamp(0.0, 1.0)),
                    action: r.action.unwrap_or_default(),
                    args: r.args,
                    switch: r.switch.map(|on| if on { "on" } else { "off" }.to_owned()).unwrap_or_default(),
                    input: r.input,
                    at: -1,
                })
                .collect();
        }
        SlotContent::SettingsPanel { title, scope } => {
            vm.kind = SlotKind::SettingsPanel;
            vm.text = title;
            vm.scope = Some(scope);
        }
    }
    vm
}

/// A compatibility level's words.
fn compat_label(level: CompatLevel) -> &'static str {
    match level {
        CompatLevel::Unknown => "Not tested",
        CompatLevel::Nothing => "Doesn't start",
        CompatLevel::Boots => "Boots",
        CompatLevel::Menus => "Menus",
        CompatLevel::Ingame => "In-game",
        CompatLevel::Playable => "Playable",
    }
}

fn compat_vm(info: &CompatInfo) -> CompatVm {
    CompatVm {
        level: info.level.as_str().into(),
        label: compat_label(info.level).into(),
        notes: info.notes.clone().unwrap_or_default(),
        tested: info.tested.clone().unwrap_or_default(),
        tested_text: String::new(),
    }
}

/// The game's square art (see [`LibrarySnapshot::tile`]).
fn cover_of(library: &LibrarySnapshot, entry: &Entry) -> Option<ImageRef> {
    library.tile(entry).map(plugin_image)
}

/// Whether `key`'s game can be uninstalled: its folder is known, isn't a whole player's folder,
/// is in one its emulator may write, and is there.
fn can_uninstall(s: &Snapshot, key: &GameKey) -> bool {
    let folder = s.library.detail(key).and_then(|d| d.details.folder.as_ref());
    folder.is_some_and(|f| {
        !f.path.is_root()
            && s.plugins.manifest(&key.plugin).is_some_and(|m| m.permissions.folder(&f.folder).is_some_and(|g| g.write))
            && s.files.game_folders.contains(key)
    })
}

/// A game's badges, from its plugin.
fn badges_of(library: &LibrarySnapshot, key: &GameKey) -> Vec<SlotVm> {
    slot_vms(&key.plugin, SlotId::GameBadge, library.badges(key).to_vec())
}

/// Whole days since the game was last played; `None` before it was.
fn last_played_days(s: &Snapshot, key: &GameKey) -> Option<u64> {
    let stats = s.sessions.plays.of(key);
    (stats.count > 0).then(|| crate::sessions::days_between(stats.last, s.now_secs))
}

/// A game's card. `selected`: the game selected on the card's screen.
fn card(s: &Snapshot, entry: &Entry, selected: Option<&GameKey>) -> GameCardVm {
    let library = &s.library;
    GameCardVm {
        key: entry.key.to_string(),
        title: entry.summary.title.clone(),
        platform: entry.summary.platform.clone(),
        system: s.plugins.system(&entry.summary.platform),
        cover: cover_of(library, entry),
        selected: selected == Some(&entry.key),
        badges: badges_of(library, &entry.key),
        release_year: entry.summary.release_year,
        rating: library.meta(&entry.key).and_then(|(_, m)| m.rating).map(|r| r.score),
        size_bytes: library.detail(&entry.key).and_then(|d| d.details.size_bytes),
        size: String::new(),
        last_played_days: last_played_days(s, &entry.key),
        last_played: String::new(),
        releases: 0,
        availability: String::new(),
        availability_text: String::new(),
        hidden: false,
        // A Library game's; `offer_card` gives an offer's.
        can: game_actions(s, &entry.key, None),
        consoles: Vec::new(),
    }
}

/// What can be done with the game `key`: a Library game (`offer` is `None`), or a Catalog
/// offer with the installed game it is, if any. The one place the rules live:
///
/// - Play: it is installed (an offer: the installed game it is), and that game's plugin can
///   start games. A missing emulator is no reason to hide it: Play installs the emulator first.
/// - Get: an offer that isn't installed. A release its store installs asks the store
///   ("Install in Steam"); a torrent's game downloads, from a release the player picks.
/// - Uninstall: the installed game, by its store's link when its plugin gives one and may open
///   it, else by deleting its folder when its plugin may.
/// - The main action: Play, else Get, else none. Uninstall is never the main one.
fn game_actions(
    s: &Snapshot,
    key: &GameKey,
    offer: Option<(&crate::releases::Offer, Option<&Entry>)>,
) -> GameActionsVm {
    let installed: Option<&GameKey> = match offer {
        None => Some(key),
        Some((_, game)) => game.map(|g| &g.key),
    };
    let manifest = |plugin: &PluginId| s.plugins.manifest(plugin);
    let play =
        installed.is_some_and(|k| manifest(&k.plugin).is_some_and(|m| m.capabilities.contains(&Capability::Launcher)));
    let mut actions = GameActionsVm { play, ..GameActionsVm::default() };
    if let Some(game) = installed {
        let details = s.library.detail(game).map(|d| &d.details);
        let link = details.and_then(|d| d.uninstall_link.as_deref());
        let store_removes = link.is_some_and(|l| manifest(&game.plugin).is_some_and(|m| m.permissions.allows_link(l)));
        if store_removes {
            let platform = details.map(|d| d.summary.platform.as_str()).unwrap_or_default();
            actions.uninstall = game.to_string();
            actions.uninstall_store = s.plugins.system(platform).name;
        } else if can_uninstall(s, game) {
            actions.uninstall = game.to_string();
        }
    }
    if let Some((offer, None)) = offer {
        match offer.releases.iter().find(|r| !r.store.is_empty()) {
            Some(release) => {
                actions.get_store = s.plugins.system(&release.platform).name;
                actions.get = format!("Install in {}", actions.get_store);
                actions.get_release = release.id.clone();
            }
            None if !offer.releases.is_empty() => actions.get = "Download".into(),
            None => {}
        }
    }
    actions.primary = if actions.play {
        "play".into()
    } else if !actions.get.is_empty() {
        "get".into()
    } else {
        String::new()
    };
    actions
}

/// A grid's cards. `selected`: the game selected on the grid's screen.
fn grid_vm(s: &Snapshot, data: &GridData, query: &GridQuery, selected: Option<&GameKey>) -> LibraryVm {
    let GridGames { shown, versions, all, systems, genres } = query.games(data);
    let library = data.library;
    let offers: std::collections::HashMap<&GameKey, &crate::releases::Offer> =
        library.offers.iter().map(|o| (&o.entry.key, o)).collect();
    let total = shown.len();
    let games: Vec<GameCardVm> = shown
        .into_iter()
        .take(query.shown)
        .map(|e| match offers.get(&e.key) {
            Some(offer) if query.grid == Grid::Catalog => {
                let card =
                    GameCardVm { hidden: library.hidden(&e.key), ..offer_card(s, data.ownership, offer, selected) };
                // "All" shows one card per game: the card stands for each of its versions.
                match versions.get(&e.key) {
                    Some(versions) => game_card(s, library, card, offer, versions, selected),
                    None => card,
                }
            }
            _ => card(s, e, selected),
        })
        .collect();
    let matches = if crate::search::fold(&query.query).trim().is_empty() {
        Vec::new()
    } else {
        games.iter().take(crate::BEST_MATCHES).map(|g| g.title.clone()).collect()
    };
    LibraryVm {
        selected: games.iter().position(|g| g.selected),
        matches,
        recent: s.prefs.recent_searches.clone(),
        games,
        total,
        all,
        systems,
        genres,
        sort: query.sort.as_str().into(),
        query: query.query.clone(),
    }
}

/// An offer's card in the Catalog's "All", for the `versions` of its game that the grid gave it
/// (see [`crate::grid_query::GridGames::versions`]): the consoles they are on, the card's own
/// first; selected when a version of the game is; new when a version is.
fn game_card(
    s: &Snapshot,
    library: &LibrarySnapshot,
    mut card: GameCardVm,
    offer: &crate::releases::Offer,
    versions: &[&crate::releases::Offer],
    selected: Option<&GameKey>,
) -> GameCardVm {
    let own = std::iter::once(offer);
    let others = versions.iter().copied().filter(|o| o.entry.key != offer.entry.key);
    card.consoles = own
        .chain(others)
        .map(|o| ConsoleVm {
            platform: o.entry.summary.platform.clone(),
            system: s.plugins.system(&o.entry.summary.platform),
        })
        .collect();
    card.selected = selected.is_some_and(|k| library.groups.versions(&offer.entry.key).contains(k));
    let new = |o: &&crate::releases::Offer| o.releases.iter().any(|r| crate::releases::is_new(&r.added, s.today()));
    let has = |id: &str| card.badges.iter().any(|b| b.id == id);
    if !has("installed") && !has("new") && versions.iter().any(new) {
        card.badges.push(badge(&offer.entry.key.plugin, "new", "New", Tone::Info));
    }
    card
}

/// How a version runs on this PC, in a few words: "Playable · Should run well", or why it
/// won't start.
fn version_how(version: &crate::game_groups::Version, here: Option<Platform>) -> String {
    use crate::game_groups::Speed;
    let fit = &version.fit;
    if !fit.starts {
        let for_here = version.offer.releases.iter().any(|r| crate::releases::not_here(&r.system, here).is_empty());
        return match (fit.speed, for_here) {
            (Speed::No, _) => "Won't start on this PC".into(),
            (_, false) => "Can't run here".into(),
            _ => compat_label(version.level).into(),
        };
    }
    let speed = match fit.speed {
        Speed::Good => "Should run well",
        Speed::Slow => "May run slowly",
        Speed::No => "Won't start",
        Speed::Unknown => "Can't tell yet",
    };
    format!("{} · {speed}", compat_label(version.level))
}

/// The versions of the Catalog game `key` is one of, the best for this PC first. Empty for a
/// game on one console.
fn versions_vm(s: &Snapshot, owned: &Ownership, key: &GameKey) -> Vec<GameVersionVm> {
    let library = &s.library;
    let keys = library.groups.versions(key);
    if keys.len() < 2 {
        return Vec::new();
    }
    let offers: Vec<&crate::releases::Offer> = keys.iter().filter_map(|k| library.offer(k)).collect();
    let pc = s.this_pc();
    let best = crate::game_groups::best_first(library, owned, &pc, &offers);
    let why = crate::game_groups::why_best(&best[0], best.get(1), compat_label);
    best.iter()
        .enumerate()
        .map(|(at, v)| {
            let summary = &v.offer.entry.summary;
            GameVersionVm {
                key: v.offer.entry.key.to_string(),
                platform: summary.platform.clone(),
                system: s.plugins.system(&summary.platform),
                // When no version can start here, none is the best.
                best: at == 0 && v.fit.starts,
                why: if at == 0 { why.clone() } else { String::new() },
                how: version_how(v, pc.system),
                current: &v.offer.entry.key == key,
                installed: owned.installed_game(v.offer).is_some(),
            }
        })
        .collect()
}

/// What clicking `notice` opens, for example "Open Downloads"; empty for nothing.
fn notice_opens(plugins: &Plugins, notice: &crate::notices::Notice) -> String {
    match &notice.route {
        Some(Route::Downloads) => "Open Downloads".into(),
        Some(Route::Catalog) => "Open the Catalog".into(),
        Some(Route::GameDetail(_)) => "Open the Game Hub".into(),
        Some(Route::Plugin(plugin)) => {
            format!("Open {}", plugins.manifest(plugin).map(|m| m.name.clone()).unwrap_or_default())
        }
        Some(Route::Settings) => "Open Settings".into(),
        _ if notice.drive.is_some() => "Open or use for games".into(),
        _ => String::new(),
    }
}

/// The cover of the game `notice` is about.
fn notice_art(library: &LibrarySnapshot, notice: &crate::notices::Notice) -> Option<ImageRef> {
    let game = notice.game.as_ref()?;
    library.listed(game).and_then(|e| cover_of(library, e))
}

fn notice_vm(s: &Snapshot, notice: &crate::notices::Notice) -> NoticeVm {
    NoticeVm {
        id: notice.id,
        text: notice.message.clone(),
        kind: notice.kind,
        art: notice_art(&s.library, notice),
        opens: notice_opens(&s.plugins, notice),
        drive: notice.drive.clone().unwrap_or_default(),
    }
}

/// The parts of the view that list games: the dearest to build, over thousands of games.
pub(crate) struct GameLists {
    pub home: HomeVm,
    pub detail: Option<GameDetailVm>,
    pub catalog: CatalogVm,
    pub downloads: DownloadsVm,
}

impl GameLists {
    /// The game lists of `view`.
    pub fn of(view: AppView) -> Self {
        GameLists { home: view.home, detail: view.detail, catalog: view.catalog, downloads: view.downloads }
    }
}

/// Whether a change in `area` can change the game lists ([`GameLists`]). The other areas reach
/// only the machine's settings, the notices, the launcher's update, the backups and the folder
/// browser, so a change there builds the rest of the view and keeps the lists as they were.
pub(crate) fn moves_game_lists(area: Area) -> bool {
    !matches!(area, Area::System | Area::Notices | Area::LauncherUpdate | Area::Backup | Area::Browser | Area::Tray)
}

/// The view models for the snapshot `s`.
pub(crate) fn build(s: &Snapshot) -> AppView {
    build_with(s, None)
}

/// The view models for the snapshot `s`, with `lists` as its game lists when given: they stand
/// for the snapshot's own when no change since they were built moves them
/// ([`moves_game_lists`]).
pub(crate) fn build_with(s: &Snapshot, lists: Option<GameLists>) -> AppView {
    let state = &s.state;
    let route = state.stack.route().clone();
    let (screen, game) = match &route {
        Route::Home => (ScreenId::Home, None),
        Route::Library => (ScreenId::Library, None),
        Route::GameDetail(g) => (ScreenId::GameDetail, Some(g.clone())),
        Route::Catalog => (ScreenId::Catalog, None),
        Route::Downloads => (ScreenId::Downloads, None),
        Route::Settings => (ScreenId::Settings, None),
        Route::Emulators => (ScreenId::Emulators, None),
        Route::Themes => (ScreenId::Themes, None),
        Route::Plugin(_) => (ScreenId::Plugin, None),
        Route::GameSettings(g) => (ScreenId::GameSettings, Some(g.clone())),
    };
    let lists = lists.unwrap_or_else(|| {
        let owned = s.ownership();
        let data = s.grid_data(&owned);
        GameLists {
            home: home_vm(s, &data),
            // The detail page of the game the player last opened stays available under
            // overlays. With no detail page open, the selected game's details, once loaded.
            detail: s.detail_game.as_ref().and_then(|key| detail_vm(s, &owned, key)),
            catalog: catalog_vm(s, &data, route == Route::Catalog),
            downloads: downloads_vm(s, &owned),
        }
    });
    let notices = &s.notices;
    let (tray, tray_bar, tray_rows) = crate::tray::view(s);
    let mut view = AppView {
        screen,
        game: game.map(|g| g.to_string()),
        can_go_back: state.stack.can_go_back(),
        nav_direction: match state.stack.moved() {
            crate::stack::Move::Forward => "forward",
            crate::stack::Move::Back => "back",
            crate::stack::Move::None => "",
        }
        .into(),
        home: lists.home,
        detail: lists.detail,
        catalog: lists.catalog,
        downloads: lists.downloads,
        settings: settings_vm(s),
        emulators: emulators_vm(s),
        plugin_page: match &route {
            Route::Plugin(plugin) => plugin_page_vm(s, plugin),
            _ => None,
        },
        running: running_vm(s),
        notices: notices.iter().map(|n| n.message.clone()).collect(),
        notice_cards: notices.iter().map(|n| notice_vm(s, n)).collect(),
        browser: browser_vm(s),
        question: state.startup.question.as_ref().map(|(_, q)| QuestionVm {
            title: q.title.clone(),
            text: q.text.clone(),
            yes: q.yes.clone(),
            no: q.no.clone(),
        }),
        install_prompt: s.builds.install_prompt.as_ref().map(|p| InstallPromptVm {
            plugin: p.game.plugin.clone(),
            emulator: p.emulator.clone(),
            version: p.version.clone(),
            game: p.title.clone(),
            size_bytes: p.size_bytes,
        }),
        boot: boot_vm(s),
        task: s.library.task.clone(),
        system: system_vm(s),
        tray,
        tray_bar,
        tray_rows,
        result_prompt: state.result_prompt.as_ref().map(|(key, _)| s.library.title(key)),
        ratings: crate::ratings::view(state.ratings.as_ref(), |key| s.library.title(key)),
        closing: state.closing.as_ref().map(|step| step.text.clone()),
        closing_game: state.closing.as_ref().and_then(|step| step.game.as_ref()).map(|(key, title)| {
            let art = s.library.art(key);
            ClosingGameVm {
                title: title.clone(),
                cover: art.cover.map(plugin_image),
                background: art.background.map(plugin_image),
            }
        }),
    };
    plugin_lists(&mut view, &s.plugins);
    crate::display::write(&mut view);
    view
}

/// A badge the core gives a card itself.
fn badge(plugin: &PluginId, id: &str, text: &str, tone: Tone) -> SlotVm {
    slot_vm(
        plugin,
        SlotId::GameBadge,
        SlotItem {
            id: id.into(),
            order: Default::default(),
            content: SlotContent::Badge { label: text.into(), short: String::new(), tone, icon: None },
        },
    )
}

fn offer_card(
    s: &Snapshot,
    owned: &Ownership,
    offer: &crate::releases::Offer,
    selected: Option<&GameKey>,
) -> GameCardVm {
    let mut card = card(s, &offer.entry, selected);
    let best = owned.best_release(offer);
    card.size_bytes = best.size_bytes;
    card.availability = crate::releases::availability(best.seeders).into();
    card.releases = offer.releases.len();
    let installed = owned.installed_game(offer);
    card.can = game_actions(s, &offer.entry.key, Some((offer, installed)));
    // A game its store installs has no torrent to be available.
    if offer.releases.iter().any(|r| !r.store.is_empty()) {
        card.availability = String::new();
    }
    let plugin = &offer.entry.key.plugin;
    if offer.releases.iter().any(|r| r.favorite) {
        card.badges.push(badge(plugin, "favorite", "Favorite", Tone::Info));
    }
    if installed.is_some() {
        card.badges.push(badge(plugin, "installed", "Installed", Tone::Success));
    } else if offer.releases.iter().any(|r| crate::releases::is_new(&r.added, s.today())) {
        card.badges.push(badge(plugin, "new", "New", Tone::Info));
    }
    // Every release is for another system.
    let here = s.plugins.host().platform;
    if offer.releases.iter().all(|r| !crate::releases::not_here(&r.system, here).is_empty()) {
        card.badges.push(badge(plugin, "not-here", "Can't run here", Tone::Warning));
    }
    card
}

/// The Catalog. Only the Catalog shows its grid: `on_screen` off, the last one stands.
fn catalog_vm(s: &Snapshot, data: &GridData, on_screen: bool) -> CatalogVm {
    let library = &s.library;
    let sources = library
        .catalogs
        .iter()
        .map(|c| {
            let l = &c.list;
            let copy = match l.source {
                launcher_api::ReleaseSource::Fetched => format!("fetched from {}", l.host),
                launcher_api::ReleaseSource::Cached => format!("the copy fetched before from {}", l.host),
                launcher_api::ReleaseSource::Bundled => "the bundled catalog".into(),
            };
            let mut parts = vec![format!("{}: {copy}", s.plugins.name(&c.emulator))];
            if !l.version.is_empty() {
                parts.push(format!("version {}", l.version));
            }
            if !l.updated.is_empty() {
                parts.push(format!("updated {}", l.updated.get(..10).unwrap_or(&l.updated)));
            }
            parts.push(if l.releases.len() == 1 {
                "1 release".into()
            } else {
                format!("{} releases", l.releases.len())
            });
            if let Some(problem) = &l.problem {
                parts.push(format!("the fetch failed: {problem}"));
            }
            parts.join(" · ")
        })
        .collect();
    let releases = library.catalogs.iter().flat_map(|c| &c.list.releases);
    // The releases of the games the Catalog has, not of the ones the player hid.
    let shown_releases: usize =
        library.offers.iter().filter(|o| !library.hidden(&o.entry.key)).map(|o| o.releases.len()).sum();
    // The newest count of seeders, else the newest date a catalog gives.
    let counted =
        releases.clone().filter_map(|r| crate::releases::days_of(&r.counted_at).map(|_| &r.counted_at[..10])).max();
    let dated = || {
        library
            .catalogs
            .iter()
            .filter_map(|c| crate::releases::days_of(&c.list.updated).map(|_| &c.list.updated[..10]))
            .max()
    };
    let games = s
        .state
        .catalog_grid
        .get(on_screen)
        .unwrap_or_else(|| grid_vm(s, data, &s.state.catalog_view, s.state.stack.selected_on(&Route::Catalog)));
    let buttons = |kind: SlotKind| -> Vec<SlotVm> {
        let all = library.catalog_buttons.iter();
        all.flat_map(|(plugin, items)| slot_vms(plugin, SlotId::CatalogAction, items.clone()))
            .filter(|s| s.kind == kind)
            .collect()
    };
    CatalogVm {
        available: s.plugins.catalog_available(),
        games,
        sources,
        releases: shown_releases,
        updated: counted.or_else(dated).unwrap_or_default().to_owned(),
        updated_text: String::new(),
        loading: !library.catalogs_read,
        actions: buttons(SlotKind::Action),
        status: buttons(SlotKind::Badge),
    }
}

/// Home's games and the Library's grid. Each shows the game selected on its own screen; the
/// Library's grid, until the Library opens, Home's.
fn home_vm(s: &Snapshot, data: &GridData) -> HomeVm {
    let stack = &s.state.stack;
    let home = stack.selected_on(&Route::Home);
    let games: Vec<GameCardVm> = s.library.library().map(|e| card(s, e, home)).collect();
    let selected = games.iter().position(|g| g.selected);
    let mut groups: Vec<GroupVm> = Vec::new();
    for game in &games {
        match groups.iter_mut().find(|g| g.platform == game.platform) {
            Some(group) => group.games.push(game.clone()),
            None => groups.push(GroupVm { platform: game.platform.clone(), games: vec![game.clone()], selected: None }),
        }
    }
    for group in &mut groups {
        group.selected = group.games.iter().position(|g| g.selected);
    }
    let grid = grid_vm(s, data, &s.state.library_view, stack.selected_on(&Route::Library).or(home));
    HomeVm { games, selected, loading: s.library.loading, groups, library: grid }
}

/// A plugin's settings panel, with its fields.
fn settings_panel_vm(panel: &crate::app::SettingsPanel) -> SettingsPanelVm {
    let slot = slot_vm(&panel.plugin, SlotId::SettingsPanel, panel.item.clone());
    SettingsPanelVm { fields: crate::settings_schema::fields(&panel.schema, &panel.values), slot }
}

/// A game's own settings, as its Game Hub shows them.
fn game_settings_vm(plugins: &Plugins, key: &GameKey, settings: &crate::library::GameSettings) -> SettingsPanelVm {
    let plugin = &key.plugin;
    let name = plugins.manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_default();
    let slot = SlotVm {
        slot: SlotId::SettingsPanel,
        plugin: plugin.clone(),
        id: "game".into(),
        label: format!("{plugin}:game"),
        kind: SlotKind::SettingsPanel,
        text: format!("{name} settings for this game"),
        short: String::new(),
        tone: Tone::Neutral,
        style: ActionStyle::Secondary,
        icon: None,
        rows: Vec::new(),
        action: None,
        args: Value::Null,
        scope: Some(launcher_api::ConfigScope::Game(key.local.clone())),
    };
    SettingsPanelVm { fields: crate::settings_schema::fields(&settings.schema, &settings.values), slot }
}

fn detail_vm(s: &Snapshot, owned: &Ownership, key: &GameKey) -> Option<GameDetailVm> {
    let (library, torrents) = (&s.library, &s.torrents);
    let d = library.detail(key)?;
    let details = &d.details;
    let summary = &details.summary;
    let manifest = s.plugins.manifest(&key.plugin);
    let offer = library.offer(key);
    let has_installer = manifest.is_some_and(|m| m.capabilities.contains(&Capability::Installer));
    let installed = s.builds.emulators.get(&key.plugin).is_some_and(|e| !e.installed.is_empty());
    // A Catalog offer plays as the installed game it is, and not at all before it is installed.
    let installed_as = offer.and_then(|o| owned.installed_game(o));
    let played = installed_as.map(|g| &g.key).unwrap_or(key);
    let can = game_actions(s, key, offer.map(|o| (o, installed_as)));
    let needs_emulator = has_installer && !installed;
    // The program, not the console: "Launches with the KytyPS5 emulator".
    let name = manifest.map(|m| m.emulator_name().to_owned()).unwrap_or_default();
    let stats = s.sessions.plays.of(key);
    let meta = library.meta(key);
    // A release's store data fills in, field by field, what the store's catalog leaves out.
    let metas = library.metadata(key);
    let text = |field: fn(&launcher_api::GameMetadata) -> &String| crate::game_words::first_text(&metas, field);
    let shown = library.art(key);
    let said = crate::game_words::words(details, &metas);
    let store_name = offer.map(|o| s.plugins.name(&o.emulator));
    let origin = match &store_name {
        Some(emulator) => crate::game_words::Origin::Catalog { emulator },
        None => crate::game_words::Origin::Local { program: &name },
    };
    let line = crate::game_words::home_line(details, &metas, meta.map(|(_, m)| m.tagline.as_str()), origin);
    let compatibility = offer.map(crate::releases::compatibility).unwrap_or_default();
    // An offer's details carry no community result: it is the installed game's, else the
    // emulator's list's for the offer's title id (see [`crate::game_groups::community`]).
    let community = match offer {
        Some(offer) => crate::game_groups::community(library, owned, offer),
        None => d.compat.as_ref(),
    };
    // Where the offer's downloads go, and the free space there.
    let download_folder = offer.and_then(|o| s.state.disks.download_folder(&o.emulator, &o.releases[0].platform));
    let room = download_folder.map(|f| f.room).unwrap_or_default();
    let free = room.download_free();
    // A release that has a download says so on its sheet instead.
    let no_room = |r: &launcher_api::Release| match r.size_bytes {
        Some(size) if !torrents.jobs.contains_key(&r.id) => room.short(Need::Download, size).unwrap_or_default(),
        _ => String::new(),
    };
    let releases = offer.map_or(&[][..], |o| &o.releases[..]);
    let (this_pc, pc_warning) = on_this_pc(s, offer.map_or(&key.plugin, |o| &o.emulator), releases);
    // A game that starts here but doesn't fit says so instead, from its best release.
    let this_pc = match (this_pc, offer.map(|o| no_room(owned.best_release(o))), free) {
        (Some(v), Some(short), Some(free)) if v.level != "no" && !short.is_empty() => Some(VerdictVm {
            level: "no".into(),
            headline: "Not enough space".into(),
            why: format!("{} free", crate::room::gigabytes(free)),
            ..v
        }),
        (this_pc, _, _) => this_pc,
    };
    // An offer's own result, else the installed game's (see [`crate::game_groups::player_result`]).
    let your_result = match offer {
        Some(offer) => crate::game_groups::player_result(library, owned, offer),
        None => library.result(key),
    };
    let home = s.plugins.host().home.as_ref().and_then(|h| h.to_str());
    let fact = |id: &str| details.facts.iter().find(|f| f.id == id).map(|f| f.value.clone()).unwrap_or_default();
    let rating = meta.and_then(|(_, m)| m.rating);
    Some(GameDetailVm {
        key: key.to_string(),
        title: summary.title.clone(),
        platform: summary.platform.clone(),
        system: s.plugins.system(&summary.platform),
        summary: summary_of(&line),
        description: said.description,
        developer: said.developer,
        genres: said.genres,
        cover: shown.cover.map(plugin_image),
        background: shown.background.map(plugin_image),
        logo: shown.logo.map(plugin_image),
        rating,
        rating_score: rating.map_or(0.0, |r| r.score),
        ratings: rating.map_or(0, |r| r.total),
        rating_text: String::new(),
        release: text(|m| &m.release),
        release_text: String::new(),
        year: String::new(),
        age_rating: meta.map(|(_, m)| m.age_rating.clone()).unwrap_or_default(),
        screenshots: shown.screenshots.into_iter().map(plugin_image).collect(),
        store_url: text(|m| &m.store_url),
        layer_source: compatibility.source,
        layer_rating: compatibility.rating,
        layer_url: compatibility.url,
        this_pc,
        pc_warning,
        your_result: your_result.map(CompatLevel::as_str).unwrap_or_default().to_owned(),
        has_folder: s.files.game_folders.contains(key),
        has_log: s.files.logs.contains(key),
        hidden: library.hidden(key),
        compat_page: community.and_then(|c| c.page.clone()).unwrap_or_default(),
        trailer_url: meta.map(|(_, m)| m.trailer.clone()).unwrap_or_default(),
        compat: community.map(compat_vm),
        badges: badges_of(library, key),
        actions: slot_vms(&key.plugin, SlotId::GameAction, d.actions.clone()),
        sections: slot_vms(&key.plugin, SlotId::GameDetailSection, d.sections.clone()),
        can: can.clone(),
        settings: d.settings.iter().map(|settings| game_settings_vm(&s.plugins, key, settings)).collect(),
        launch_hint: if can.play && needs_emulator { format!("Play installs {name} first") } else { String::new() },
        running: s.sessions.runs(played),
        size_bytes: details.size_bytes,
        size: String::new(),
        region: fact("region"),
        age: fact("age"),
        version: fact("version"),
        facts: details.facts.clone(),
        played_secs: stats.secs,
        played: String::new(),
        last_played_days: last_played_days(s, key),
        last_played: String::new(),
        releases: offer
            .map(|o| {
                let today = s.today();
                let best = &owned.best_release(o).id;
                crate::releases::ordered(&o.releases, installed_as.is_some())
                    .into_iter()
                    .map(|crate::releases::Ordered { release: r, label }| ReleaseVm {
                        version_label: label,
                        best: &r.id == best,
                        seeders: r.seeders,
                        leechers: r.leechers,
                        counted_at: r.counted_at.clone(),
                        counted_text: String::new(),
                        availability: crate::releases::availability(r.seeders).into(),
                        availability_text: String::new(),
                        id: r.id.clone(),
                        store: if r.store.is_empty() { String::new() } else { s.plugins.system(&r.platform).name },
                        version: r.version.clone(),
                        region: r.region.clone(),
                        size_bytes: r.size_bytes,
                        size: String::new(),
                        added: r.added.clone(),
                        added_text: String::new(),
                        new: crate::releases::is_new(&r.added, today),
                        description: r.description.clone(),
                        system: crate::releases::system_label(&r.system).into(),
                        not_here: crate::releases::not_here(&r.system, s.plugins.host().platform),
                        warning: r.install_warning.clone().unwrap_or_default(),
                        no_room: no_room(r),
                        download: torrents.jobs.get(&r.id).map(|j| j.state.as_str().to_owned()).unwrap_or_default(),
                        download_progress: torrents
                            .jobs
                            .get(&r.id)
                            .map(|j| if j.total > 0 { (j.done as f64 / j.total as f64).min(1.0) as f32 } else { 0.0 })
                            .unwrap_or(0.0),
                        download_text: String::new(),
                        title_text: String::new(),
                        peers_text: String::new(),
                        sheet_text: String::new(),
                        line_text: String::new(),
                        not_here_text: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        emulator: offer
            .map(|o| {
                s.plugins.manifest(&o.emulator).map_or_else(|| o.emulator.to_string(), |m| m.emulator_name().to_owned())
            })
            .unwrap_or_default(),
        download_folder: download_folder
            .map(|f| crate::display::short_home(&f.path.display().to_string(), home))
            .unwrap_or_default(),
        availability: offer
            .map(|o| crate::releases::availability(owned.best_release(o).seeders).to_owned())
            .unwrap_or_default(),
        availability_text: String::new(),
        download_error: torrents
            .download_error
            .as_ref()
            .filter(|(game, _)| game == key)
            .map(|(_, why)| why.clone())
            .unwrap_or_default(),
        versions: if offer.is_some() { versions_vm(s, owned, key) } else { Vec::new() },
    })
}

/// How `emulator`'s games run on this PC, and why they won't start, if they won't. An offer
/// whose `releases` are all for another system won't start, whatever the emulator needs.
/// `None` before the check, for a plugin that isn't an emulator, and when the system can't say.
fn on_this_pc(s: &Snapshot, emulator: &PluginId, releases: &[launcher_api::Release]) -> (Option<VerdictVm>, String) {
    let Some(manifest) = s.plugins.manifest(emulator).filter(|m| m.kind == PluginKind::Emulator) else {
        return (None, String::new());
    };
    let here = s.plugins.host().platform;
    if !releases.is_empty() && releases.iter().all(|r| !crate::releases::not_here(&r.system, here).is_empty()) {
        let mut systems: Vec<&str> = Vec::new();
        for label in releases.iter().map(|r| crate::releases::system_label(&r.system)) {
            if !systems.contains(&label) {
                systems.push(label);
            }
        }
        let why = format!("{} release", systems.join(" or "));
        // The release rows say why, so there is no warning for the whole emulator.
        let verdict = crate::hardware::Verdict::No(why);
        return (
            Some(crate::hardware::verdict_vm(emulator.to_string(), manifest.name.clone(), verdict)),
            String::new(),
        );
    }
    let Some(specs) = &s.state.hardware else { return (None, String::new()) };
    let verdict = crate::hardware::verdict(specs, &manifest.requires);
    let warning = match &verdict {
        crate::hardware::Verdict::Unknown(_) => return (None, String::new()),
        crate::hardware::Verdict::No(why) => crate::hardware::blocked_notice(&[(manifest.name.clone(), why.clone())]),
        _ => None,
    };
    (
        Some(crate::hardware::verdict_vm(emulator.to_string(), manifest.name.clone(), verdict)),
        warning.unwrap_or_default(),
    )
}

fn downloads_vm(s: &Snapshot, owned: &Ownership) -> DownloadsVm {
    let (torrents, installs) = (&s.torrents, &s.installs);
    // Game installs, newest first, then the downloads. A catalog download's install shows on
    // the download's own card (`torrent_vm`), so it has no card of its own.
    let on_a_card = |j: &crate::game_installs::GameJob| {
        torrents.jobs.values().any(|t| t.emulator == j.plugin && t.source == j.source)
    };
    let install_jobs = installs.jobs.iter().rev().filter(|(_, j)| !on_a_card(j)).map(|(id, j)| DownloadVm {
        id: id.0,
        plugin: j.plugin.clone(),
        label: j.label.clone(),
        progress: if j.phase == crate::game_installs::GamePhase::Done {
            1.0
        } else if j.total > 0 {
            (j.done as f32 / j.total as f32).min(1.0)
        } else {
            0.0
        },
        received: j.done,
        total: (j.total > 0).then_some(j.total),
        progress_text: String::new(),
        state: j.phase.as_str().into(),
        error: j.error.clone().unwrap_or_default(),
        can_pause: false,
        can_resume: false,
        can_cancel: j.phase == crate::game_installs::GamePhase::Installing,
    });
    let downloads = s.downloads.iter().rev().cloned().map(|j| DownloadVm {
        id: j.id.0,
        progress: match j.total {
            Some(t) if t > 0 => (j.received as f32 / t as f32).min(1.0),
            _ => 0.0,
        },
        plugin: j.plugin,
        label: j.label,
        received: j.received,
        total: j.total,
        progress_text: String::new(),
        state: j.phase.as_str().into(),
        error: j.error.unwrap_or_default(),
        can_pause: matches!(j.phase, JobPhase::Queued | JobPhase::Downloading),
        can_resume: j.phase == JobPhase::Paused,
        can_cancel: !j.phase.is_over(),
    });
    let jobs = install_jobs.chain(downloads).collect();
    let home = s.plugins.host().home.as_ref().and_then(|h| h.to_str()).map(str::to_owned);
    let mut cards: Vec<TorrentVm> = torrents
        .jobs
        .values()
        .map(|j| {
            // The sheet's sums: while it is Ready, `total` is the whole torrent's.
            let ready = j.state == crate::torrents::TorrentState::Ready;
            let checked =
                if ready { crate::torrents::picks::checked_bytes(&j.files, j.total, &j.skip) } else { j.total };
            let room = s.state.disks.room(&j.id);
            let free = |bytes: Option<u64>| bytes.map(|b| format!("{} free", crate::room::gigabytes(b)));
            // The same question the sheet's Download and Download and install ask on the click.
            let short = |need: Need| room.short(need, checked).filter(|_| ready).unwrap_or_default();
            let console = s.plugins.console_of(&j.emulator, &j.console);
            TorrentVm {
                folder: crate::display::short_home(&j.folder.display().to_string(), home.as_deref()),
                install_note: install_note(&s.plugins, &j.emulator, &console),
                install_folder: s
                    .files
                    .install_folders
                    .get(&(j.emulator.clone(), console.clone()))
                    .map(|path| crate::display::short_home(&path.display().to_string(), home.as_deref()))
                    .unwrap_or_default(),
                picked: if ready {
                    let on = j.file_count.saturating_sub(j.skip.len());
                    format!("{on} of {} files · {}", j.file_count, crate::display::size_text(checked))
                } else {
                    String::new()
                },
                download_free: free(room.download_free()).unwrap_or_default(),
                install_free: free(room.games_free()).unwrap_or_default(),
                download_room: short(Need::Download),
                install_room: short(Need::DownloadAndInstall),
                ..torrent_vm(&s.library, installs, owned, j)
            }
        })
        .collect();
    cards.reverse();
    // The installs stores do (Steam's), after the downloads: the same cards, with the store's
    // own page as their one action.
    cards.extend(s.state.store_installs.iter().map(|(plugin, install)| store_install_vm(&s.plugins, plugin, install)));
    let downloading = torrents.jobs.values().filter(|j| j.state.active()).count();
    let sharing = torrents.jobs.values().filter(|j| j.seeding).count();
    DownloadsVm { jobs, torrents: cards, seeding: torrents.sharing, downloading, sharing }
}

/// A button on a Downloads card (`TorrentVm::actions`). Its order here is the order the card
/// shows them in. It is not [`crate::TorrentAction`], the command that a few of them send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TorrentButton {
    Start,
    Pause,
    Play,
    Install,
    StopSeeding,
    Cancel,
    Remove,
    RemoveUnpacked,
    CancelInstall,
    OpenStore,
}

impl TorrentButton {
    /// The word a theme gets.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Pause => "pause",
            Self::Play => "play",
            Self::Install => "install",
            Self::StopSeeding => "stop-seeding",
            Self::Cancel => "cancel",
            Self::Remove => "remove",
            Self::RemoveUnpacked => "remove-unpacked",
            Self::CancelInstall => "cancel-install",
            Self::OpenStore => "open-store",
        }
    }
}

/// `buttons` as the words a theme gets, in [`TorrentButton`]'s order.
fn button_words(mut buttons: Vec<TorrentButton>) -> Vec<String> {
    buttons.sort();
    buttons.into_iter().map(|b| b.as_str().to_owned()).collect()
}

/// A store's install as a Downloads card: its phase and bytes, and the store's page.
fn store_install_vm(plugins: &Plugins, plugin: &PluginId, install: &launcher_api::StoreInstall) -> TorrentVm {
    use launcher_api::StorePhase as P;
    let store = plugins
        .manifest(plugin)
        .and_then(|m| m.system.as_ref().map(|s| s.name.clone()))
        .unwrap_or_else(|| plugin.to_string());
    let part =
        if install.total > 0 { (install.done as f64 / install.total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let installing = install.phase == P::Installing;
    TorrentVm {
        id: format!("store:{plugin}:{}", install.release),
        title: install.title.clone(),
        state: match install.phase {
            P::Queued => "queued",
            P::Downloading | P::Installing => "downloading",
            P::Paused => "paused",
        }
        .into(),
        progress: if installing { 1.0 } else { part },
        done: if installing { install.total } else { install.done },
        total: install.total,
        install: if installing { "installing".into() } else { String::new() },
        install_progress: if installing { part } else { 0.0 },
        actions: button_words(if install.open.is_empty() { Vec::new() } else { vec![TorrentButton::OpenStore] }),
        store,
        ..TorrentVm::default()
    }
}

/// What `emulator` installs from for `console`, as its manifest says, then what the host does
/// first.
fn install_note(plugins: &Plugins, emulator: &PluginId, console: &str) -> String {
    let own = plugins
        .manifest(emulator)
        .and_then(|m| m.played_for(console).map(|p| p.install_note.to_owned()))
        .unwrap_or_default();
    let archives = format!(
        "{} first unpacks the RAR, ZIP, 7z and TAR archives in the download, but not one with a password.",
        launcher_api::brand::PRODUCT_NAME
    );
    if own.is_empty() { archives } else { format!("{own} {archives}") }
}

fn torrent_vm(library: &LibrarySnapshot, installs: &InstallsSnapshot, owned: &Ownership, j: &TorrentJob) -> TorrentVm {
    use crate::torrents::TorrentState as S;
    let install = installs.latest(&j.emulator, &j.source);
    let installing = install.is_some_and(|g| g.phase == crate::game_installs::GamePhase::Installing);
    let installed = install.is_some_and(|g| g.phase == crate::game_installs::GamePhase::Done);
    // The installed game: the one this download's install reported, else the offer's, as its
    // Catalog card finds it (by title id, or by name: a PC game has no title id). The install's
    // record is gone after a restart.
    let offer = library.offer(&j.offer);
    let game = owned
        .installed_from(j)
        .or_else(|| offer.and_then(|o| owned.installed_game(o)))
        .map(|e| e.key.to_string())
        .unwrap_or_default();
    use crate::archives::UnpackPhase as U;
    let unpacking = j.hand_off.unpack.phase == U::Unpacking;
    use TorrentButton as B;
    let mut actions = Vec::new();
    match j.state {
        S::Ready | S::Paused | S::Failed | S::Cancelled => actions.push(B::Start),
        S::Checking | S::Downloading => actions.push(B::Pause),
        _ => {}
    }
    if j.state == S::Complete && !installing && !unpacking {
        actions.push(if !game.is_empty() { B::Play } else { B::Install });
    }
    if j.seeding || j.verifying {
        actions.push(B::StopSeeding);
    }
    // While its archives unpack, Cancel stops the unpacking.
    if !matches!(j.state, S::Complete | S::Cancelled) || unpacking {
        actions.push(B::Cancel);
    } else if !j.seeding && !j.verifying && !installing {
        actions.push(B::Remove);
    }
    if j.state == S::Complete && j.hand_off.unpack.phase == U::Unpacked && !installing {
        actions.push(B::RemoveUnpacked);
    }
    if installing {
        actions.push(B::CancelInstall);
    }
    let unpack_error =
        (j.hand_off.unpack.phase == U::Failed).then(|| j.hand_off.unpack.error.clone()).filter(|e| !e.is_empty());
    TorrentVm {
        store: String::new(),
        id: j.id.clone(),
        title: j.title.clone(),
        version: j.version.clone(),
        region: j.region.clone(),
        state: j.state.as_str().into(),
        state_text: String::new(),
        progress: if j.state == S::Complete {
            1.0
        } else if j.total > 0 {
            (j.done as f64 / j.total as f64).clamp(0.0, 1.0) as f32
        } else {
            0.0
        },
        done: j.done,
        total: j.total,
        download_rate: j.download_rate,
        upload_rate: j.upload_rate,
        seconds_left: match j.state {
            S::Downloading if j.download_rate > 0 => j.total.saturating_sub(j.done).div_ceil(j.download_rate),
            _ => 0,
        },
        peers: j.peers,
        detail: String::new(),
        seeding: j.seeding,
        verifying: j.verifying,
        waiting_vpn: j.waiting_vpn,
        // The stage that failed says why: the install, its unpacking, an install that didn't
        // start ("Download and install"), then the download.
        error: install
            .and_then(|g| g.error.clone())
            .or(unpack_error)
            .or_else(|| (!j.hand_off.install_error.is_empty()).then(|| j.hand_off.install_error.clone()))
            .unwrap_or_else(|| j.error.clone()),
        files: j.files.iter().map(|f| format!("{} · {}", f.path, crate::display::size_text(f.size))).collect(),
        file_count: j.file_count,
        file_summary: String::new(),
        folder: j.folder.display().to_string(),
        actions: button_words(actions),
        install: install.map(|g| g.phase.as_str().to_owned()).unwrap_or_default(),
        unpack: j.hand_off.unpack.phase.as_str().into(),
        unpack_progress: j.hand_off.unpack.progress,
        install_progress: install
            .map(|g| if g.total > 0 { (g.done as f64 / g.total as f64).min(1.0) as f32 } else { 0.0 })
            .unwrap_or(0.0),
        game: if installed || install.is_none() { game } else { String::new() },
        install_note: String::new(),
        install_folder: String::new(),
        install_error: j.hand_off.install_error.clone(),
        picks: crate::torrents::picks::picks(&j.files, &j.skip)
            .into_iter()
            .zip(&j.files)
            .map(|(p, f)| launcher_ui_api::TorrentPickVm {
                path: f.path.clone(),
                size: crate::display::size_text(f.size),
                kind: p.kind.as_str().into(),
                on: p.on,
                locked: p.locked,
            })
            .collect(),
        picked: String::new(),
        download_free: String::new(),
        install_free: String::new(),
        download_room: String::new(),
        install_room: String::new(),
        install_after: j.install_after.is_some(),
    }
}

/// Wi-Fi signal (0 to 100) in bars, 0 to 4.
pub(crate) fn bars(signal: u8) -> i32 {
    match signal {
        0..=19 => 1,
        20..=44 => 2,
        45..=69 => 3,
        _ => 4,
    }
}

/// A job's text for the player, and a finished install's flags: its plugin reports how the
/// install ended as [`launcher_api::Installed`] JSON.
fn job_text(job: &crate::system::JobState) -> launcher_ui_api::SystemJobVm {
    let mut vm = launcher_ui_api::SystemJobVm {
        id: 0,
        kind: String::new(),
        target: String::new(),
        state: String::new(),
        percent: -1,
        text: job.text.clone(),
        mok: false,
        nvidia_later: false,
    };
    if job.kind == crate::system::JobKind::InstallOs
        && job.phase == crate::system::JobPhase::Done
        && let Ok(installed) = serde_json::from_str::<launcher_api::Installed>(&job.text)
    {
        vm.text = format!("{} is installed", launcher_api::brand::PRODUCT_OS_NAME);
        vm.mok = installed.mok;
        vm.nvidia_later = installed.nvidia_later;
    }
    vm
}

/// The Drives panel's drives, each with its buttons: Open, Use for games, then Eject (a drive the player can take out) or Unmount, while it is
/// mounted; Mount while it isn't. Mount, Eject and Unmount only where drives can be mounted
/// (`can_act`).
fn drive_items(s: &Snapshot, can_act: bool) -> Vec<launcher_ui_api::DriveItemVm> {
    use crate::system::{JobKind, JobPhase};
    let mut items = Vec::new();
    for drive in &s.drives {
        let mut detail: Vec<String> = Vec::new();
        detail.extend(drive.size.map(crate::browser::drive_size_text));
        match &drive.mount {
            Some(_) => detail.extend(drive.free.map(|f| format!("{} free", crate::browser::drive_size_text(f)))),
            None => detail.push("Not mounted".into()),
        }
        if !drive.filesystem.is_empty() {
            detail.push(drive.filesystem.clone());
        }
        let mut actions: Vec<&str> = Vec::new();
        match &drive.mount {
            None if can_act => actions.push("mount"),
            None => {}
            Some(_) => {
                actions.push("open");
                actions.push("use");
                if can_act {
                    actions.push(if drive.usb { "eject" } else { "unmount" });
                }
            }
        }
        // A mount, unmount or eject job runs on the drive: its job names the drive.
        let busy = s.state.system.jobs.iter().find(|j| j.phase == JobPhase::Running && j.drive == drive.id);
        let busy = match busy.map(|j| j.kind) {
            Some(JobKind::Mount) => "mount",
            Some(JobKind::Unmount) => "unmount",
            Some(JobKind::Eject) => "eject",
            _ => "",
        };
        let used = match (drive.mount.as_ref(), drive.size, drive.free) {
            (Some(_), Some(size), Some(free)) if size > 0 => (size.saturating_sub(free) as f64 / size as f64) as f32,
            _ => -1.0,
        };
        let kind = if drive.device.starts_with("/dev/mmcblk") {
            "card"
        } else if drive.usb {
            "usb"
        } else {
            "internal"
        };
        items.push(launcher_ui_api::DriveItemVm {
            drive: drive.id.clone(),
            name: drive.label.clone(),
            detail: detail.join(" · "),
            path: drive.mount.as_ref().map(|m| m.display().to_string()).unwrap_or_default(),
            kind: kind.into(),
            used,
            actions: actions.into_iter().map(str::to_owned).collect(),
            busy: busy.into(),
        });
    }
    items
}

/// One area for the view: its state, why it can't be used, and whether long work runs in it.
fn area_vm(system: &crate::system::SystemState, area: launcher_api::SystemArea) -> launcher_ui_api::AreaVm {
    use launcher_api::{AreaData, AreaState};
    let (state, reason) = match system.area(area) {
        None => ("loading", String::new()),
        Some(AreaState::Absent) => ("absent", crate::system::absent(area)),
        Some(AreaState::Unsupported(why)) => ("unsupported", why.clone()),
        // A refusal always says why: an empty reason means restart and shut down work.
        Some(AreaState::Ready(AreaData::Power { refused: Some(why), .. })) if why.is_empty() => {
            ("ready", crate::system::absent(area))
        }
        Some(AreaState::Ready(AreaData::Power { refused, .. })) => ("ready", refused.clone().unwrap_or_default()),
        Some(AreaState::Ready(_)) => ("ready", String::new()),
    };
    let busy = system.jobs.iter().any(|j| j.phase == crate::system::JobPhase::Running && j.kind.area() == area);
    launcher_ui_api::AreaVm { name: area.name().into(), state: state.into(), reason, busy }
}

fn system_vm(s: &Snapshot) -> launcher_ui_api::SystemVm {
    use launcher_api::{AreaData, Connection, OsUpdate, SystemArea};
    use launcher_ui_api::{AudioOutputVm, BluetoothDeviceVm, InstallDiskVm, SystemJobVm, WifiVm};
    let system = &s.state.system;
    let place = s.plugins.host().place;
    let available = |a: SystemArea| system.data(a).is_some();
    let (network, wifi) = match system.data(SystemArea::Network) {
        Some(AreaData::Network { status, wifi }) => (status.clone().unwrap_or_default(), wifi.as_slice()),
        _ => (Default::default(), &[][..]),
    };
    let (bluetooth_on, devices) = match system.data(SystemArea::Bluetooth) {
        Some(AreaData::Bluetooth { powered, devices }) => (*powered, devices.as_slice()),
        _ => (false, &[][..]),
    };
    let (outputs, level) = match system.data(SystemArea::Audio) {
        Some(AreaData::Audio { outputs, level }) => (outputs.as_slice(), level.unwrap_or_default()),
        _ => (&[][..], Default::default()),
    };
    let battery = match system.data(SystemArea::Power) {
        Some(AreaData::Power { battery, .. }) => *battery,
        _ => None,
    };
    let update = match system.data(SystemArea::OsUpdates) {
        Some(AreaData::OsUpdates { update }) => update.as_ref(),
        _ => None,
    };
    let disks = match system.data(SystemArea::OsInstall) {
        Some(AreaData::OsInstall { disks }) => disks.as_slice(),
        _ => &[][..],
    };
    let (os_update, os_update_text) = match update {
        None => (String::new(), String::new()),
        Some(OsUpdate::Current) => {
            ("current".into(), format!("{} is up to date", launcher_api::brand::PRODUCT_OS_NAME))
        }
        Some(OsUpdate::Checking) => ("checking".into(), "Looking for an update…".into()),
        Some(OsUpdate::Staged { version }) => ("staged".into(), format!("Restart to update to {version}")),
        Some(OsUpdate::Failed { reason }) => ("failed".into(), format!("Couldn't look for an update: {reason}")),
    };
    launcher_ui_api::SystemVm {
        whole_screen: place.whole_screen,
        own_os: place.own_os,
        stick: place.stick,
        areas: SystemArea::ALL.iter().map(|&area| area_vm(system, area)).collect(),
        connection: match network.connection {
            Connection::None => "none",
            Connection::Wired => "wired",
            Connection::Wifi => "wifi",
        }
        .into(),
        network_name: network.name,
        signal_bars: if network.connection == Connection::Wifi { network.signal.map_or(0, bars) } else { 0 },
        wifi: wifi
            .iter()
            .map(|w| WifiVm {
                ssid: w.ssid.clone(),
                bars: bars(w.signal),
                secured: w.secured,
                saved: w.saved,
                active: w.active,
            })
            .collect(),
        volume: level.volume as i32,
        muted: level.muted,
        outputs: outputs
            .iter()
            .map(|o| AudioOutputVm { name: o.name.clone(), description: o.description.clone(), current: o.current })
            .collect(),
        bluetooth_on,
        connected_devices: devices.iter().filter(|d| d.connected).count() as i32,
        devices: devices
            .iter()
            .map(|d| BluetoothDeviceVm {
                address: d.address.clone(),
                name: d.name.clone(),
                paired: d.paired,
                connected: d.connected,
            })
            .collect(),
        battery_shown: battery.is_some(),
        battery_percent: battery.map_or(0, |b| b.percent as i32),
        battery_charging: battery.is_some_and(|b| b.charging),
        os_update,
        os_update_text,
        disks: disks
            .iter()
            .map(|d| InstallDiskVm {
                path: d.path.display().to_string(),
                size: crate::display::disk_size_text(d.bytes),
                windows: d.windows,
                usb: d.usb,
                model: d.model.clone(),
            })
            .collect(),
        drives_listing: !s.drives_listed,
        drives: drive_items(s, available(SystemArea::Drives)),
        jobs: system
            .jobs
            .iter()
            .map(|j| SystemJobVm {
                id: j.id as i32,
                kind: j.kind.name().into(),
                target: j.target.clone(),
                state: match j.phase {
                    crate::system::JobPhase::Running => "running",
                    crate::system::JobPhase::Done => "done",
                    crate::system::JobPhase::Failed => "failed",
                }
                .into(),
                percent: j.percent.map_or(-1, i32::from),
                ..job_text(j)
            })
            .collect(),
    }
}

fn launcher_update_vm(update: &crate::launcher_update::LauncherUpdateSnapshot) -> launcher_ui_api::LauncherUpdateVm {
    use crate::launcher_update::UpdateState;
    let current = update.current.to_owned();
    let (state, version, text) = match &update.state {
        _ if update.off => ("off", String::new(), "Updates are off: no release repo is set".to_owned()),
        UpdateState::Off => ("off", String::new(), "Updates are off: no release repo is set".to_owned()),
        UpdateState::Idle => {
            ("idle", String::new(), format!("Check for a newer {}", launcher_api::brand::PRODUCT_NAME))
        }
        UpdateState::Checking => ("checking", String::new(), "Checking…".to_owned()),
        UpdateState::UpToDate => {
            ("current", String::new(), format!("{} {current} is the newest", launcher_api::brand::PRODUCT_NAME))
        }
        UpdateState::Available { version, .. } => (
            "available",
            version.clone(),
            format!("{} {version} is out · download it", launcher_api::brand::PRODUCT_NAME),
        ),
        UpdateState::Downloading { version, size: None } => {
            ("downloading", version.clone(), format!("Downloading {} {version}…", launcher_api::brand::PRODUCT_NAME))
        }
        UpdateState::Downloading { version, size: Some((download, whole)) } => (
            "downloading",
            version.clone(),
            format!(
                "Downloading {} {version}: {}, only what changed…",
                launcher_api::brand::PRODUCT_NAME,
                crate::display::progress_text(*download, Some(*whole))
            ),
        ),
        UpdateState::Ready { version } => (
            "ready",
            version.clone(),
            format!("{} {version} is ready · restart to update", launcher_api::brand::PRODUCT_NAME),
        ),
        UpdateState::Installed { version } => (
            "installed",
            version.clone(),
            format!(
                "{name} {version} is installed · restart {name} to start it",
                name = launcher_api::brand::PRODUCT_NAME
            ),
        ),
        UpdateState::WithSystem { version } => (
            "system",
            version.clone(),
            format!("{} {version} is out · it comes with the next system update", launcher_api::brand::PRODUCT_NAME),
        ),
        UpdateState::Failed(why) => ("failed", String::new(), format!("Couldn't update: {why}")),
    };
    launcher_ui_api::LauncherUpdateVm { state: state.into(), current, version, text, method: update.method.to_owned() }
}

fn settings_vm(s: &Snapshot) -> SettingsVm {
    let (plugins, state, prefs) = (&s.plugins, &s.state, &s.prefs);
    let manifests: Vec<_> = plugins
        .of_kind(PluginKind::Emulator)
        .into_iter()
        .filter_map(|id| {
            plugins.manifest(&id).filter(|m| m.capabilities.contains(&Capability::Launcher)).map(|m| (id, m))
        })
        .collect();
    let hardware = crate::hardware::hardware_vm(
        state.hardware.as_ref(),
        manifests.iter().map(|(id, m)| (id.to_string(), m.name.clone(), &m.requires)),
    );
    let themes = plugins
        .of_kind(PluginKind::Theme)
        .into_iter()
        .map(|id| ThemeChoiceVm {
            name: plugins.manifest(&id).map(|m| m.name.clone()).unwrap_or_default(),
            current: state.theme.as_ref() == Some(&id),
            id,
        })
        .collect();
    let theme_options = match &state.theme {
        Some(theme) => {
            let options = plugins
                .manifest(theme)
                .and_then(|m| ThemeManifest::from_plugin(m).ok())
                .map(|m| m.theme.options)
                .unwrap_or_default();
            let schema = options_schema(&options);
            crate::settings_schema::fields(&schema, &s.theme_options)
        }
        None => Vec::new(),
    };
    SettingsVm {
        launcher_update: launcher_update_vm(&s.launcher_update),
        hardware,
        panels: state.settings.iter().map(settings_panel_vm).collect(),
        themes,
        theme_options,
        // The game folders. The other folders are on the pages of the plugins that ask for them.
        folders: folders_vm(s).into_iter().filter(|f| launcher_api::shared_folder(&f.name).is_some()).collect(),
        sounds: prefs.sounds,
        animations: prefs.animations,
        fullscreen: prefs.fullscreen,
        return_on_exit: prefs.return_on_exit,
        auto_update: prefs.auto_update,
        auto_install: prefs.auto_install,
        scale: prefs.scale,
        can_start_at_login: s.can_start_at_login,
        start_at_login: prefs.start_at_login,
        start_at_login_error: prefs.start_at_login_error.clone().unwrap_or_default(),
        vpn: vpn_vm(&s.vpn),
        backup: backup_vm(s),
    }
}

fn backup_vm(s: &Snapshot) -> launcher_ui_api::BackupVm {
    let backup = &s.backup;
    let home = s.plugins.host().home.as_deref().map(|h| h.display().to_string());
    let folder =
        s.backup_folder.as_ref().map(|f| crate::display::short_home(&f.display().to_string(), home.as_deref()));
    let job = backup.job.as_ref();
    launcher_ui_api::BackupVm {
        free: s
            .state
            .disks
            .backup_free
            .map(|b| format!("{} free", crate::browser::drive_size_text(b)))
            .unwrap_or_default(),
        folder: folder.unwrap_or_default(),
        running: job.is_some(),
        restoring: job.is_some_and(|j| j.restoring),
        progress: job.filter(|j| j.total > 0).map(|j| (j.done as f64 / j.total as f64).min(1.0) as f32).unwrap_or(0.0),
        status: if backup.error.is_empty() { backup.done.clone() } else { backup.error.clone() },
        failed: !backup.error.is_empty(),
        restart: backup.restart,
        backups: backup
            .backups
            .iter()
            .map(|b| launcher_ui_api::BackupFileVm {
                file: b.file.clone(),
                date: crate::display::date_time_text(b.made),
                size: crate::display::size_text(b.bytes),
                items: b.items.join(", "),
                before_restore: b.before_restore,
            })
            .collect(),
        holds: backup.holds.join(", "),
        large: backup.large.join(", "),
        default_folder: s
            .plugins
            .host()
            .home
            .as_ref()
            .map(|h| {
                crate::display::short_home(
                    &h.join(crate::backups::DEFAULT_FOLDER).display().to_string(),
                    home.as_deref(),
                )
            })
            .unwrap_or_default(),
    }
}

/// The folder browser, while it is open.
fn browser_vm(s: &Snapshot) -> launcher_ui_api::FolderBrowserVm {
    let Some(browser) = s.browser.as_deref() else { return Default::default() };
    let home = s.plugins.host().home.as_deref().map(|h| h.display().to_string());
    let home = home.as_deref();
    let full = |p: &std::path::Path| p.display().to_string();
    let (purpose, folder) = match &browser.purpose {
        crate::browser::Purpose::Restore => ("restore", String::new()),
        crate::browser::Purpose::Pick(name) => ("pick", name.clone()),
        crate::browser::Purpose::BackupFolder => ("pick", "backup".to_owned()),
        crate::browser::Purpose::SystemFiles(plugin, _) => ("system-files", plugin.to_string()),
    };
    let folder_title = match &browser.purpose {
        crate::browser::Purpose::BackupFolder => "Backup folder".to_owned(),
        _ => launcher_api::shared_folder(&folder).map(|f| f.title.to_owned()).unwrap_or_else(|| folder.clone()),
    };
    let title = match &browser.purpose {
        crate::browser::Purpose::Restore => "Restore a backup".to_owned(),
        crate::browser::Purpose::Pick(_) => format!("Add to {folder_title}"),
        crate::browser::Purpose::BackupFolder => "Choose the backup folder".to_owned(),
        crate::browser::Purpose::SystemFiles(plugin, console) => {
            let files = s.plugins.manifest(plugin).and_then(|m| match console {
                None => m.system_files.as_ref(),
                Some(c) => m.listed_console(c.as_str()).and_then(|c| c.system_files.as_ref()),
            });
            files.map(|f| format!("Import {}", f.name)).unwrap_or_else(|| "Import system files".to_owned())
        }
    };
    launcher_ui_api::FolderBrowserVm {
        open: true,
        purpose: purpose.into(),
        folder,
        folder_title,
        title,
        shown: crate::display::short_home(&full(&browser.at), home),
        parent: browser.at.parent().map(full).unwrap_or_default(),
        crumbs: crumbs(&browser.at, home),
        place: browser.places.iter().position(|p| p.path == browser.at).map_or(-1, |i| i as i32),
        places_loading: browser.places_loading,
        places: browser
            .places
            .iter()
            .map(|p| launcher_ui_api::BrowsePlaceVm {
                name: p.name.clone(),
                path: full(&p.path),
                kind: p.kind.as_str().to_owned(),
                detail: p.detail.clone(),
                drive: p.drive.clone().unwrap_or_default(),
                mounted: !p.path.as_os_str().is_empty(),
                usb: p.usb,
            })
            .collect(),
        entries: browser
            .shown()
            .map(|e| {
                let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut vm = launcher_ui_api::BrowseEntryVm {
                    name,
                    path: full(&e.path),
                    folder: e.backup.is_none() && e.archive.is_none(),
                    detail: e.detail.clone(),
                    ..Default::default()
                };
                if let Some(bytes) = e.archive {
                    vm.size = crate::display::size_text(bytes);
                }
                if let Some(b) = &e.backup {
                    vm.date = crate::display::date_time_text(b.made);
                    vm.size = crate::display::size_text(b.bytes);
                    vm.items = b.items.join(", ");
                    vm.computer.clone_from(&b.computer);
                    vm.newer = b.newer;
                }
                vm
            })
            .collect(),
        hidden: browser.hidden,
        filter: browser.filter.clone(),
    }
}

/// `at`'s path one part each, from the home folder (`~`) or the top.
fn crumbs(at: &std::path::Path, home: Option<&str>) -> Vec<launcher_ui_api::BrowseCrumbVm> {
    let mut out = Vec::new();
    for dir in at.ancestors() {
        let path = dir.display().to_string();
        let at_home = home.is_some_and(|h| std::path::Path::new(h) == dir);
        let name = match dir.file_name() {
            _ if at_home => "~".to_owned(),
            Some(name) => name.to_string_lossy().into_owned(),
            None => path.clone(),
        };
        out.push(launcher_ui_api::BrowseCrumbVm { name, path });
        if at_home {
            break;
        }
    }
    out.reverse();
    out
}

fn vpn_vm(vpn: &crate::vpn::VpnSnapshot) -> launcher_ui_api::VpnSettingsVm {
    launcher_ui_api::VpnSettingsVm {
        problem: vpn.problem.clone().unwrap_or_default(),
        only: vpn.only,
        finds: vpn.finds,
        can_import: vpn.can_import,
        route: vpn.route.clone().unwrap_or_default(),
        chosen: vpn.chosen.clone(),
        vpns: vpn
            .vpns
            .iter()
            .map(|v| launcher_ui_api::VpnVm {
                name: v.name.clone(),
                kind: v.kind.clone(),
                connected: v.connected,
                switchable: v.switchable,
            })
            .collect(),
        error: vpn.error.clone().unwrap_or_default(),
    }
}

fn folders_vm(s: &Snapshot) -> Vec<FolderVm> {
    let (plugins, host) = (&s.plugins, s.plugins.host());
    let mut by_name: BTreeMap<String, FolderVm> = BTreeMap::new();
    for plugin in plugins.order() {
        let Some(manifest) = plugins.manifest(plugin) else { continue };
        for grant in &manifest.permissions.filesystem {
            let home = host.home.as_deref();
            let folder = by_name.entry(grant.folder.clone()).or_insert_with(|| {
                let roots = s.files.roots.get(&grant.folder).map(Vec::as_slice).unwrap_or_default();
                let paths: Vec<String> = roots.iter().map(|p| p.display().to_string()).collect();
                FolderVm {
                    name: grant.folder.clone(),
                    title: String::new(),
                    help: String::new(),
                    text: paths.join("; "),
                    list: roots.iter().map(|p| folder_path_vm(s, p)).collect(),
                    shown: String::new(),
                    paths,
                    default: String::new(),
                    requested_by: Vec::new(),
                    requested_by_text: String::new(),
                    write: false,
                }
            });
            if folder.default.is_empty()
                && let Some(default) = grant.default_path(host.platform).and_then(|d| crate::folders::expand(d, home))
            {
                folder.default = default.display().to_string();
            }
            if let Some(shared) = launcher_api::shared_folder(&grant.folder) {
                folder.title = shared.title.into();
                folder.help = shared.help.into();
            }
            if folder.title.is_empty() {
                folder.title = grant.title.clone().unwrap_or_default();
            }
            if folder.help.is_empty() {
                folder.help = grant.help.clone().unwrap_or_default();
            }
            folder.requested_by.push(manifest.name.clone());
            folder.write |= grant.write;
        }
    }
    let home = host.home.as_deref().map(|h| h.display().to_string());
    by_name
        .into_values()
        .map(|mut f| {
            let paths: Vec<String> = if f.paths.is_empty() { vec![f.default.clone()] } else { f.paths.clone() };
            f.shown = paths
                .iter()
                .filter(|p| !p.is_empty())
                .map(|p| crate::display::short_home(p, home.as_deref()))
                .collect::<Vec<_>>()
                .join("; ");
            f
        })
        .collect()
}

/// One folder of a named host folder, with the free space on its drive.
fn folder_path_vm(s: &Snapshot, path: &std::path::Path) -> launcher_ui_api::FolderPathVm {
    let home = s.plugins.host().home.as_deref().map(|h| h.display().to_string());
    let there = s.state.disks.roots_free.contains_key(path);
    let free = match s.state.disks.roots_free.get(path) {
        Some(Some(bytes)) => format!("{} free", crate::browser::drive_size_text(*bytes)),
        Some(None) => String::new(),
        None => "Not there".into(),
    };
    launcher_ui_api::FolderPathVm {
        path: path.display().to_string(),
        shown: crate::display::short_home(&path.display().to_string(), home.as_deref()),
        free,
        there,
        scanning: s.state.scanning.contains(path),
    }
}

fn emulators_vm(s: &Snapshot) -> EmulatorsVm {
    let (plugins, builds) = (&s.plugins, &s.builds);
    // Home lists the systems by name, after All games: see [`Snapshot::home_emulators`].
    let emulators = s
        .home_emulators()
        .into_iter()
        .map(|(id, look)| {
            let emu = builds.emulators.get(&id).cloned().unwrap_or_default();
            let manifest = plugins.manifest(&id);
            let art = |path: Option<&launcher_api::RelPath>| {
                path.map(|path| ImageRef::PluginAsset { plugin: id.clone(), path: path.clone() })
            };
            EmulatorVm {
                name: manifest.map(|m| m.emulator_name().to_owned()).unwrap_or_default(),
                system: look.map(|l| l.id.clone()).unwrap_or_default(),
                system_name: look.map(|l| l.name.clone()).unwrap_or_default(),
                icon: art(look.and_then(|l| l.icon.as_ref())),
                cover: art(look.and_then(|l| l.cover.as_ref())),
                status: status_text(plugins.status(&id)),
                has_builds: manifest.is_some_and(|m| m.capabilities.contains(&Capability::Installer)),
                update: emu.update(),
                installed: emu.installed,
                available: emu.available,
                busy: builds.busy(&id),
                panels: slot_vms(&id, SlotId::EmulatorPanel, emu.panels),
                controllers: controllers(manifest.and_then(|m| m.played_for(look.map_or("", |l| l.id.as_str())))),
                controllers_text: String::new(),
                plugin: id,
            }
        })
        .collect::<Vec<_>>();
    let rows = plugins
        .order()
        .iter()
        .filter_map(|id| plugins.manifest(id).map(|m| (id, m)))
        .map(|(id, m)| PluginRowVm {
            id: id.clone(),
            name: m.name.clone(),
            kind: match m.kind {
                PluginKind::Emulator => "emulator".into(),
                PluginKind::Service => "service".into(),
                PluginKind::System => "system".into(),
                PluginKind::Theme => "theme".into(),
            },
            version: m.version.to_string(),
            status: status_text(plugins.status(id)),
            enabled: plugins.enabled(id),
            build: builds.emulators.get(id).and_then(|e| e.installed.last().cloned()).unwrap_or_default(),
            build_label: build_label(
                m.emulator_name(),
                builds.emulators.get(id).and_then(|e| e.installed.last()).map_or("", String::as_str),
            ),
            has_builds: m.capabilities.contains(&Capability::Installer),
            note: if plugins.elsewhere(id) {
                let names: Vec<&str> = m
                    .platforms
                    .iter()
                    .map(|p| match p {
                        Platform::Linux => "Linux",
                        Platform::Macos => "macOS",
                        Platform::Windows => "Windows",
                    })
                    .collect();
                format!("Made for {}, not this system: turn it on to use your own build", names.join(" and "))
            } else {
                String::new()
            },
        })
        .collect();
    EmulatorsVm { emulators, plugins: rows, emulator_plugins: Vec::new(), theme_plugins: Vec::new() }
}

/// The Emulators and the Themes lists, from every plugin, and the Plugin page's actions and
/// whether its theme is the one shown.
fn plugin_lists(view: &mut AppView, registry: &Plugins) {
    let shown = view.settings.themes.iter().find(|t| t.current).map(|t| t.id.clone());
    let plugins = &view.emulators.plugins;
    // Emulators, then the services that describe their games, each by name.
    let kind = |kind: &'static str| {
        let mut list: Vec<PluginRowVm> = plugins.iter().filter(|p| p.kind == kind).cloned().collect();
        list.sort_by_cached_key(|p| p.name.to_lowercase());
        list
    };
    view.emulators.emulator_plugins = kind("emulator").into_iter().chain(kind("service")).collect();
    // A theme's status says whether it is the one shown. The fallback theme comes first; the
    // others keep their order.
    view.emulators.theme_plugins = plugins
        .iter()
        .filter(|p| p.kind == "theme")
        .cloned()
        .map(|p| PluginRowVm {
            status: if shown.as_ref() == Some(&p.id) { "in use".into() } else { String::new() },
            ..p
        })
        .collect();
    let fallback = |id: &PluginId| {
        registry.manifest(id).and_then(|m| ThemeManifest::from_plugin(m).ok()).is_some_and(|m| m.theme.fallback)
    };
    view.emulators.theme_plugins.sort_by_key(|p| !fallback(&p.id));
    if let Some(page) = &mut view.plugin_page {
        page.actions = page.sections.iter().filter(|s| s.kind == SlotKind::Action).cloned().collect();
        page.blocks = page.sections.iter().filter(|s| s.kind == SlotKind::Block).cloned().collect();
        // The blocks' buttons take the page's focus in order, after its build and actions.
        let mut at = 0;
        for row in page.blocks.iter_mut().flat_map(|b| b.rows.iter_mut()).filter(|r| !r.action.is_empty()) {
            row.at = at;
            at += 1;
        }
        page.block_buttons = at;
        page.current = shown.as_ref() == Some(&page.plugin.id);
    }
}

/// `build` with the program's name `emulator` once: "shadPS4 v.0.20.0", and
/// "KytyPS5-2026-10-21-e2dd48a" as it is, as it starts with the name. A build in words ("Your
/// own build") comes after a dot. Empty for no build.
fn build_label(emulator: &str, build: &str) -> String {
    let named = build.get(..emulator.len()).is_some_and(|start| start.eq_ignore_ascii_case(emulator));
    let version = build.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit() || c == '.');
    if build.is_empty() || named {
        build.to_owned()
    } else if version {
        format!("{emulator} {build}")
    } else {
        format!("{emulator} · {build}")
    }
}

fn plugin_page_vm(s: &Snapshot, plugin: &PluginId) -> Option<PluginPageVm> {
    let state = &s.state;
    let all = emulators_vm(s);
    let row = all.plugins.into_iter().find(|p| &p.id == plugin)?;
    if !row.enabled {
        return Some(PluginPageVm {
            plugin: row,
            emulator: None,
            panels: Vec::new(),
            sections: Vec::new(),
            actions: Vec::new(),
            blocks: Vec::new(),
            block_buttons: 0,
            folders: Vec::new(),
            folders_after: 0,
            current: false,
        });
    }
    let asks: Vec<&str> = s
        .plugins
        .manifest(plugin)
        .map(|m| m.permissions.filesystem.iter().map(|g| g.folder.as_str()).collect())
        .unwrap_or_default();
    let panels: Vec<SettingsPanelVm> =
        state.settings.iter().filter(|p| &p.plugin == plugin).map(settings_panel_vm).collect();
    // The folders follow the field the manifest names, else the last one.
    let after_key =
        s.plugins.manifest(plugin).and_then(|m| m.permissions.filesystem.iter().find_map(|g| g.after.as_deref()));
    let fields = panels.first().map(|p| p.fields.as_slice()).unwrap_or_default();
    let folders_after = after_key
        .and_then(|key| fields.iter().position(|f| f.key == key))
        .unwrap_or(fields.len().saturating_sub(1)) as i32;
    Some(PluginPageVm {
        emulator: all.emulators.into_iter().find(|e| &e.plugin == plugin),
        panels,
        sections: {
            // The core's blocks come first: the switches of the consoles it plays, then the
            // "System files" of each console that is on. Then the plugin's own.
            let consoles = s.plugins.manifest(plugin).and_then(|m| consoles_block(&s.plugins, plugin, m));
            // One archive for all the consoles, then each console's own.
            let shared = s.plugins.manifest(plugin).and_then(|m| m.shared_system_files()).map(|files| {
                let status = s.builds.system_files.get(&(plugin.clone(), None)).cloned().unwrap_or_default();
                crate::system_files::block(files, None, &status)
            });
            let own_files = s.plugins.consoles_on(plugin).into_iter().filter_map(|played| {
                let files = played.system_files?;
                let console = played.listed.then(|| played.console.cloned()).flatten();
                let owner = (plugin.clone(), console);
                let status = s.builds.system_files.get(&owner).cloned().unwrap_or_default();
                Some(crate::system_files::block(files, owner.1.as_ref(), &status))
            });
            let own = s.builds.emulators.get(plugin).map(|e| e.panels.clone()).unwrap_or_default();
            let blocks = consoles.into_iter().chain(shared).chain(own_files).chain(own);
            slot_vms(plugin, SlotId::EmulatorPanel, blocks.collect())
        },
        folders: folders_vm(s)
            .into_iter()
            .filter(|f| launcher_api::shared_folder(&f.name).is_none() && asks.contains(&f.name.as_str()))
            .collect(),
        actions: Vec::new(),
        blocks: Vec::new(),
        block_buttons: 0,
        folders_after,
        current: false,
        plugin: row,
    })
}

/// The plugin page's "Consoles" block, for an emulator that plays several (`[[consoles]]`): a
/// switch for each, by its system's full name.
fn consoles_block(
    plugins: &Plugins,
    plugin: &PluginId,
    manifest: &launcher_api::PluginManifest,
) -> Option<launcher_api::SlotItem> {
    use launcher_api::{BlockRow, SlotContent, SlotItem};
    if manifest.consoles.is_empty() {
        return None;
    }
    let rows = manifest
        .consoles
        .iter()
        .map(|c| {
            let on = plugins.console_on(plugin, c.console.as_str());
            BlockRow {
                label: c.system.name.clone(),
                hint: if on { "Its games are in the app".into() } else { "Its games are hidden".into() },
                action: Some(launcher_api::HOST_SET_CONSOLE.to_owned()),
                args: serde_json::json!({ "console": c.console.as_str(), "on": !on }),
                switch: Some(on),
                ..Default::default()
            }
        })
        .collect();
    Some(SlotItem {
        id: "consoles".into(),
        order: launcher_api::Order::default(),
        content: SlotContent::Block { title: "Consoles".into(), rows },
    })
}

/// The most covers the boot screen's wall shows.
const BOOT_COVERS: usize = 30;

fn boot_vm(s: &Snapshot) -> BootVm {
    let library = &s.library;
    let booted = library.booted;
    let startup = &s.state.startup;
    let ready = booted && startup.over;
    let games = library.library().count();
    let step = |label: String, detail: String, state: &str| BootStepVm { label, detail, state: state.into() };
    let loaded = if booted { "done" } else { "working" };
    let mut steps =
        vec![step("Game folders".into(), if booted { Look::games(games) } else { "Looking…".into() }, loaded)];
    // The start's installs: each one's share of the way, from 0 to 1.
    let mut shares: Vec<f32> = Vec::new();
    for (plugin, emu) in &s.builds.emulators {
        let Some(manifest) = s.plugins.manifest(plugin) else { continue };
        if !manifest.capabilities.contains(&Capability::Installer) {
            continue;
        }
        let download = s.downloads.iter().rev().find(|j| &j.plugin == plugin && !j.phase.is_over());
        let (detail, state) = if s.builds.busy(plugin) {
            let share =
                download.and_then(|j| j.total.filter(|t| *t > 0).map(|t| (j.received as f32 / t as f32).min(1.0)));
            shares.push(share.unwrap_or(0.0));
            let detail = match (download, share) {
                (Some(_), Some(share)) => format!("Downloading · {}%", (share * 100.0) as u32),
                (Some(_), None) => "Downloading…".into(),
                (None, _) => "Installing…".into(),
            };
            (detail, "working")
        } else if startup.failed.contains(plugin) && emu.installed.is_empty() {
            ("Could not install".into(), "failed")
        } else if startup.failed.contains(plugin) {
            ("Could not update".into(), "failed")
        } else if emu.own && emu.installed.is_empty() {
            ("Own build: none picked yet".into(), loaded)
        } else if emu.own {
            ("Your own build".into(), "done")
        } else if startup.emulators.contains(plugin) && !startup.over {
            let next = if emu.installed.is_empty() { "Installs now" } else { "Updates now" };
            (next.into(), "waiting")
        } else if emu.installed.is_empty() && s.prefs.auto_install && !booted {
            ("Installs now".into(), "working")
        } else if emu.installed.is_empty() {
            ("Installs on first play".into(), loaded)
        } else if emu.update().is_some() {
            ("Update available".into(), loaded)
        } else {
            ("Latest build".into(), loaded)
        };
        // The system's name, as Home shows it ("PlayStation 5"), not the emulator's.
        let name = manifest.system.as_ref().map(|l| l.name.clone()).unwrap_or_else(|| manifest.name.clone());
        steps.push(step(name, detail, state));
    }
    for (_, work) in &startup.work {
        let state = match work.state {
            launcher_api::StartupState::Working => {
                shares.push(work.progress.unwrap_or(0.0));
                "working"
            }
            launcher_api::StartupState::Done => "done",
            launcher_api::StartupState::Failed => "failed",
        };
        let detail = match (work.state, work.progress) {
            (launcher_api::StartupState::Working, Some(p)) => format!("{} · {}%", work.detail, (p * 100.0) as u32),
            _ => work.detail.clone(),
        };
        steps.push(step(work.label.clone(), detail, state));
    }
    if let Some((done, total)) = library.describing {
        steps.push(BootStepVm {
            label: "Official artwork & details".into(),
            detail: if library.described {
                "Done".into()
            } else if total > 0 {
                format!("{done}/{total}")
            } else {
                "Looking…".into()
            },
            state: if library.described { "done".into() } else { "working".into() },
        });
    }
    steps.push(step("Covers & Home screen".into(), if booted { "Ready".into() } else { "Loading…".into() }, loaded));
    let installing = booted && !startup.over;
    BootVm {
        ready,
        installing,
        welcome: !s.prefs.welcomed,
        // The library is the first 60%, the start's installs the rest.
        progress: if ready {
            1.0
        } else if installing {
            let done = if shares.is_empty() { 0.0 } else { shares.iter().sum::<f32>() / shares.len() as f32 };
            0.6 + 0.4 * done
        } else if library.loading {
            0.6
        } else {
            0.2
        },
        status: if ready {
            format!("All set · {games} installed game{} found", if games == 1 { "" } else { "s" })
        } else if installing {
            "Installing emulators and updates…".into()
        } else {
            "Loading...".into()
        },
        steps,
        covers: library.library().filter_map(|e| cover_of(library, e)).take(BOOT_COVERS).collect(),
    }
}

/// Words for counts.
struct Look;

impl Look {
    /// "1 game", "5 games".
    fn games(n: usize) -> String {
        if n == 1 { "1 game".into() } else { format!("{n} games") }
    }
}

/// The newest game still running.
fn running_vm(snapshot: &Snapshot) -> Option<RunningVm> {
    let sessions = &snapshot.sessions;
    let session = *sessions.sessions.keys().next_back()?;
    let s = sessions.sessions.get(&session)?;
    let manifest = snapshot.plugins.manifest(&s.game.plugin);
    let art = snapshot.library.art(&s.game);
    Some(RunningVm {
        session: session.0,
        key: s.game.to_string(),
        title: s.title.clone(),
        cover: art.cover.map(plugin_image),
        background: art.background.map(plugin_image),
        plugin: s.game.plugin.clone(),
        emulator: manifest.as_ref().map(|m| m.emulator_name().to_owned()).unwrap_or_default(),
        page: manifest.as_ref().map(|m| m.name.clone()).unwrap_or_default(),
        state: if s.stopping {
            "stopping".into()
        } else if s.shown {
            "running".into()
        } else {
            "starting".into()
        },
        elapsed_secs: snapshot.now.saturating_duration_since(s.started).as_secs(),
        elapsed: String::new(),
        in_front: sessions.game_in_front,
        keymap: manifest
            .and_then(|m| {
                let platform = snapshot.library.listed(&s.game).map(|e| e.summary.platform.as_str());
                m.played_for(platform.unwrap_or_default())
            })
            .map(|p| p.keymap.to_vec())
            .unwrap_or_default(),
    })
}

/// What a console's games play with: a controller, and the keyboard when the manifest lists
/// its keys (`[[keymap]]`). None for a console with no keys listed.
fn controllers(played: Option<launcher_api::Played<'_>>) -> Vec<String> {
    match played {
        Some(p) if !p.keymap.is_empty() => {
            vec!["Controller (DualSense, DualShock 4, Xbox and most others)".into(), "Keyboard".into()]
        }
        _ => Vec::new(),
    }
}

fn status_text(status: Option<&PluginStatus>) -> String {
    match status {
        Some(PluginStatus::Active) => "active".into(),
        Some(PluginStatus::Failed(_)) => "failed".into(),
        _ => "loaded".into(),
    }
}

/// `text` on one line, cut after a word to at most `SUMMARY_CHARS` characters, with "…".
fn summary_of(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= launcher_ui_api::SUMMARY_CHARS {
        return line;
    }
    let cut: String = line.chars().take(launcher_ui_api::SUMMARY_CHARS).collect();
    format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_names_its_program_once() {
        assert_eq!(build_label("OrbitEmu", "OrbitEmu-2026-10-21-e2dd48a"), "OrbitEmu-2026-10-21-e2dd48a");
        assert_eq!(
            build_label("OrbitEmu", "orbitemu-2026-10-21-e2dd48a"),
            "orbitemu-2026-10-21-e2dd48a",
            "in any case"
        );
        assert_eq!(build_label("shadPS4", "v.0.20.0"), "shadPS4 v.0.20.0");
        assert_eq!(build_label("Emu", "1.0"), "Emu 1.0");
        assert_eq!(
            build_label("shadPS4", "Your own build · 0.3"),
            "shadPS4 · Your own build · 0.3",
            "words, not a version"
        );
        assert_eq!(build_label("OrbitEmu", ""), "", "none installed");
    }
}

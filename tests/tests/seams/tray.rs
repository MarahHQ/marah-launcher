//! Seam 3: the top bar's tray, in every built-in theme. The bar shows the tray items that matter
//! now, then Settings, then the ⌃ that opens the tray, then the clock. The tray lists every
//! item; the mouse, the keys and the pad each open it, move in it, open a row, change where an
//! item shows, and close it. The Edit view and Settings → Launcher → Top bar change the same
//! choices.

use crate::common::*;
use launcher_api::{AudioLevel, Battery, BluetoothDevice, Connection, NetworkStatus};
use launcher_core::Command;
use launcher_core::system::{Place, SystemCommand};
use launcher_testkit::FakeSystem;
use launcher_ui_host::PadButton;
use slint::platform::Key;

const WHOLE_SCREEN: Place = Place { whole_screen: true, own_os: true, stick: false };
const THEMES: [&str; 2] = ["classic", "default"];

/// A wired network, a controller connected, the sound at 40, a battery at 18%.
fn machine() -> FakeSystem {
    let system = FakeSystem::new();
    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wired, name: "Home".into(), signal: None };
        m.bluetooth_on = true;
        m.devices =
            vec![BluetoothDevice { address: "AA:BB".into(), name: "DualSense".into(), paired: true, connected: true }];
        m.level = AudioLevel { volume: 40, muted: false };
        m.battery = Some(Battery { percent: 18, charging: false });
    });
    system
}

/// `theme` over the machine, read, on Home, with the boot screen gone.
fn shown(theme: &str) -> Composed {
    shown_on(theme, machine())
}

/// `theme` over `system`, read, on Home, with the boot screen gone.
fn shown_on(theme: &str, system: FakeSystem) -> Composed {
    let plugin = system.plugin();
    let mut host = composed(|h| {
        h.place(WHOLE_SCREEN).plugin(plugin).plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
    });
    host.ui.start().unwrap();
    host.ui.switch(&id(theme)).unwrap();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::System(SystemCommand::Refresh));
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.settle();
    host
}

/// The ids of the items in the bar, as the bar shows them, by their place from left to right.
fn bar(host: &Composed) -> Vec<String> {
    let mut icons: Vec<(f32, String)> = launcher_core::TRAY_IDS
        .iter()
        .map(|id| format!("status:{id}"))
        .filter(|label| host.count(label) > 0)
        .map(|label| (host.first(&label).absolute_position().x, label["status:".len()..].to_owned()))
        .collect();
    icons.sort_by(|a, b| a.0.total_cmp(&b.0));
    icons.into_iter().map(|(_, id)| id).collect()
}

/// Where the item `id` shows, as the core keeps it.
fn show(host: &Composed, id: &str) -> String {
    host.core().view().tray_item(id).show.clone()
}

#[test]
fn the_bar_has_the_items_that_matter_then_settings_then_the_tray_then_the_clock() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.platform.set_local_time("12:45 PM");
        host.on_tick();
        host.settle();
        let view = host.core().view();
        let in_bar: Vec<String> = view.tray_bar.iter().map(|&i| view.tray[i as usize].id.clone()).collect();
        assert_eq!(in_bar, ["network", "bluetooth", "power"], "{theme}");
        assert_eq!(bar(&host), in_bar, "{theme}: the bar shows exactly the items in it, in order");
        let x = |label: &str| host.first(label).absolute_position().x;
        let right = |label: &str| x(label) + host.first(label).size().width;
        assert!(x("Settings") > x("status:power"), "{theme}: Settings after the items");
        assert!(x("tray") > x("Settings"), "{theme}: the ⌃ after Settings");
        assert!(x("clock") > x("tray"), "{theme}: the clock after the ⌃");
        // Directly after Settings: one icon's gap, nothing between.
        let gap = x("tray") - right("Settings");
        assert!(gap > 0.0 && gap < 40.0, "{theme}: the ⌃ sits right after Settings: gap {gap}");
        assert!(
            (host.first("tray").size().width - host.first("Settings").size().width).abs() < 0.5,
            "{theme}: the ⌃ is the size of Settings"
        );
        for id in ["network", "bluetooth"] {
            let icon = host.first(&format!("status:{id}")).size();
            assert!(
                (icon.width - host.first("Settings").size().width).abs() < 0.5
                    && (icon.width - icon.height).abs() < 0.5,
                "{theme}: {id}'s button is round, the size of Settings, not as wide as its place: {icon:?}"
            );
        }
    }
}

#[test]
fn the_mouse_the_keys_and_the_pad_open_the_tray_and_close_it() {
    for theme in THEMES {
        let mut host = shown(theme);
        // The mouse: a click on the ⌃, a click outside.
        host.click("tray");
        assert_eq!(host.value("tray"), "open", "{theme}");
        assert_eq!(host.value("tray-panel"), "status", "{theme}");
        assert_eq!(host.focused("tray-item"), ["network"], "{theme}: the focus starts on the first row");
        let (tray, panel) = (host.first("tray"), host.first("tray-panel"));
        let (t, p) = (tray.absolute_position(), panel.absolute_position());
        assert!(p.y > t.y + tray.size().height - 1.0, "{theme}: under the ⌃");
        assert!(
            ((p.x + panel.size().width) - (t.x + tray.size().width)).abs() < 1.0,
            "{theme}: its far edge on the ⌃'s: panel {p:?} {:?}, ⌃ {t:?} {:?}",
            panel.size(),
            tray.size()
        );
        host.click_at(200.0, 700.0);
        assert_eq!(host.value("tray"), "closed", "{theme}: a click outside closes it");

        // The keys: the ⌃ is the bar's last stop, L1 from Home; Enter opens it, Esc closes it.
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        assert_eq!(host.value("tray"), "open", "{theme}: Enter");
        host.navigate(Key::Escape);
        assert_eq!(host.value("tray"), "closed", "{theme}: Esc");

        // The pad: ✕ opens it, ○ closes it.
        host.pad(PadButton::South);
        assert_eq!(host.value("tray"), "open", "{theme}: ✕");
        host.pad(PadButton::East);
        assert_eq!(host.value("tray"), "closed", "{theme}: ○");
    }
}

#[test]
fn while_the_tray_is_open_the_bar_shows_no_focus_of_its_own() {
    for theme in THEMES {
        let mut host = shown(theme);
        // L1 from Home: the keys on the ⌃, the bar's last stop.
        host.pad(PadButton::LeftShoulder);
        assert!(host.lit("tray"), "{theme}: the ⌃ shows the focus");
        host.pad(PadButton::South);
        assert_eq!(host.value("tray"), "open", "{theme}");
        assert_eq!(host.focused("tray-item"), ["network"], "{theme}: the tray's first row has the focus");
        for label in ["tray", "Settings", "Search", "status:network", "status:bluetooth", "status:power"] {
            assert!(!host.lit(label), "{theme}: {label} shows no focus while the tray is open");
        }
        host.pad(PadButton::East);
        assert_eq!(host.value("tray"), "closed", "{theme}");
        assert!(host.lit("tray"), "{theme}: closed, the focus is back on the ⌃");
    }
}

#[test]
fn the_keys_the_pad_and_the_mouse_move_in_the_tray_and_open_a_row() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.press_first("tray");
        assert!(host.values("tray-group").contains(&"Shortcuts".to_owned()), "{theme}: Status first, then Shortcuts");
        // Down to the sound; Enter opens its panel, and the tray gives way.
        host.navigate(Key::DownArrow);
        assert_eq!(host.focused("tray-item"), ["audio"], "{theme}");
        host.navigate(Key::Return);
        assert_eq!(host.value("system-panel"), "sound", "{theme}");
        assert_eq!(host.count("tray-panel"), 0, "{theme}");
        host.navigate(Key::Escape);

        // The pad: the d-pad down to Downloads, ✕ opens its screen.
        host.press_first("tray");
        let rows = host.values("tray-item");
        let downloads = rows.iter().position(|r| r == "downloads").unwrap();
        for _ in 0..downloads {
            host.pad(PadButton::Down);
        }
        assert_eq!(host.focused("tray-item"), ["downloads"], "{theme}");
        host.pad(PadButton::Up);
        host.pad(PadButton::Down);
        host.pad(PadButton::South);
        assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Downloads, "{theme}");
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();

        // The mouse: a click on a row opens it.
        host.click("tray");
        host.click_value("tray-item", "emulators");
        assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Emulators, "{theme}");
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
    }
}

#[test]
fn each_input_changes_where_an_item_shows_and_it_comes_into_the_bar_and_goes() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.press_first("tray");
        // The keys: / on the sound, twice: When needed, Only here, Always. It comes into the bar.
        host.navigate(Key::DownArrow);
        host.type_text("/");
        assert_eq!(show(&host, "audio"), "tray", "{theme}");
        assert_eq!(host.value("tray-show:audio"), "tray", "{theme}: the row says so");
        host.type_text("/");
        assert_eq!(show(&host, "audio"), "always", "{theme}");
        assert_eq!(host.value("status:audio"), "40", "{theme}: in the bar");

        // The pad: △ on Bluetooth: Only here. It leaves the bar.
        host.pad(PadButton::Down);
        host.pad(PadButton::North);
        assert_eq!(show(&host, "bluetooth"), "tray", "{theme}");
        assert_eq!(host.count("status:bluetooth"), 0, "{theme}: out of the bar");

        // The mouse: a right-click on Themes: Always. It comes into the bar.
        host.right_click_value("tray-item", "themes");
        assert_eq!(show(&host, "themes"), "always", "{theme}");
        assert_eq!(host.count("status:themes"), 1, "{theme}");
        assert_eq!(host.value("tray"), "open", "{theme}: a right-click doesn't open the row");
        host.navigate(Key::Escape);
        assert_eq!(bar(&host), ["network", "audio", "power", "themes"], "{theme}");
        assert_eq!(
            host.core().kernel().config().get(&["app", "tray"]),
            serde_json::json!({"audio": "always", "bluetooth": "tray", "themes": "always"}),
            "{theme}"
        );
        for (item, show) in [("audio", "needed"), ("bluetooth", "needed"), ("themes", "tray")] {
            host.dispatch(Command::Tray { item: item.into(), show: serde_json::from_value(show.into()).unwrap() });
        }
        host.settle();
    }
}

#[test]
fn the_hints_say_what_the_tray_s_keys_do() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.press_first("tray");
        assert_eq!(host.values("hint"), ["Open", "Show in bar: Always", "Close"], "{theme}: on the internet");
        host.navigate(Key::DownArrow);
        assert_eq!(host.values("hint"), ["Open", "Show in bar: When needed", "Close"], "{theme}: on the sound");
        host.pad(PadButton::Down);
        assert_eq!(host.values("hint-key"), ["✕", "△", "○"], "{theme}: the pad's buttons");
        host.pad(PadButton::East);
    }
}

#[test]
fn an_unusable_item_s_row_says_why_and_opens_nothing() {
    for theme in THEMES {
        let system = machine();
        system.with(|m| m.unavailable.push(("vpn", "No VPN here".into())));
        let mut host = shown_on(theme, system);
        host.press_first("tray");
        let view = host.core().view();
        assert_eq!(view.tray_item("vpn").state, "unsupported", "{theme}");
        assert!(!view.tray_item("vpn").in_bar, "{theme}");
        host.click_value("tray-item", "vpn");
        assert_eq!(host.value("tray"), "open", "{theme}: it opens nothing");
        assert_eq!(host.focused("tray-item"), ["vpn"], "{theme}: it takes the focus");
        assert_eq!(host.values("hint"), ["Show in bar: When needed", "Close"], "{theme}: no Open");
        host.navigate(Key::Escape);
    }
}

#[test]
fn the_edit_view_and_the_settings_row_change_the_same_choice() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.press_first("tray");
        // Up from the first row is Edit.
        host.navigate(Key::UpArrow);
        assert_eq!(host.focused("tray-edit"), ["Edit"], "{theme}");
        host.navigate(Key::Return);
        assert_eq!(host.value("tray-panel"), "edit", "{theme}");
        assert_eq!(host.focused("tray-choice:network"), ["always"], "{theme}: each row's choice");
        // Down to the sound, Right: Only here.
        host.navigate(Key::DownArrow);
        host.navigate(Key::DownArrow);
        host.navigate(Key::RightArrow);
        assert_eq!(show(&host, "audio"), "tray", "{theme}");
        assert_eq!(host.focused("tray-choice:audio"), ["tray"], "{theme}");
        // The mouse: Always for Bluetooth.
        host.click_value("tray-choice:bluetooth", "always");
        assert_eq!(show(&host, "bluetooth"), "always", "{theme}");
        // Esc goes back to the list; again, it closes.
        host.navigate(Key::Escape);
        assert_eq!(host.value("tray-panel"), "status", "{theme}");
        host.navigate(Key::Escape);
        assert_eq!(host.count("tray-panel"), 0, "{theme}");

        // Settings → Launcher → Top bar opens the same view, and changes the same choice.
        host.dispatch(Command::Navigate(launcher_core::Route::Settings));
        host.settle();
        host.press_first("top-bar");
        assert_eq!(host.value("tray-panel"), "edit", "{theme}");
        host.press("tray-choice:audio", "always");
        assert_eq!(show(&host, "audio"), "always", "{theme}");
        host.navigate(Key::Escape);
        assert_eq!(host.count("tray-panel"), 0, "{theme}: from Settings, Esc closes it");
        assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Settings, "{theme}");
        for item in ["audio", "bluetooth"] {
            host.dispatch(Command::Tray { item: item.into(), show: launcher_core::TrayShow::Needed });
        }
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
    }
}

/// An item comes into the bar while the keys are on the ⌃: they stay on it.
#[test]
fn the_keys_stay_on_the_tray_when_an_item_comes_into_the_bar() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        host.navigate(Key::DownArrow);
        host.type_text("/");
        host.type_text("/");
        assert_eq!(host.count("status:audio"), 1, "{theme}");
        host.navigate(Key::Escape);
        host.navigate(Key::Return);
        assert_eq!(host.value("tray"), "open", "{theme}: Enter opens the tray again");
        host.navigate(Key::Escape);
        host.dispatch(Command::Tray { item: "audio".into(), show: launcher_core::TrayShow::Needed });
        host.settle();
    }
}

/// A row that opens a Settings section opens Settings on it, in sight; the battery opens the
/// power menu.
#[test]
fn a_row_opens_its_settings_section_and_the_battery_the_power_menu() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.press_first("tray");
        host.press("tray-item", "os-updates");
        // The page scrolls to the section a frame after it is laid out.
        host.settle();
        assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Settings, "{theme}");
        let lit = host.all("settings-tab:System").iter().any(|e| e.accessible_checked() == Some(true));
        assert!(lit, "{theme}: the rail lights System");
        let row = host.first("system-internet");
        let y = row.absolute_position().y;
        assert!(y >= 0.0 && y + row.size().height <= 900.0, "{theme}: System's first row is in sight: {y}");
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();

        host.press_first("status:power");
        assert!(host.count("power-menu") > 0, "{theme}: the battery opens the power menu");
        host.navigate(Key::Escape);
    }
}

/// The focused row stays in sight when the tray is taller than the window, in the list and in
/// the Edit view.
#[test]
fn the_focused_row_stays_in_sight_when_the_tray_scrolls() {
    for theme in THEMES {
        let mut host = shown(theme);
        host.dispatch(Command::Pref(launcher_core::Pref::Scale, serde_json::json!(1.4)));
        host.resize(1280.0, 720.0);
        host.settle();
        let in_sight = |host: &Composed| {
            let panel = host.first("tray-panel");
            let (p, ph) = (panel.absolute_position().y, panel.size().height);
            let row = host.all("tray-item").into_iter().find(|e| e.accessible_item_selected() == Some(true)).unwrap();
            let (r, rh) = (row.absolute_position().y, row.size().height);
            assert!(r >= p - 0.5 && r + rh <= p + ph + 0.5, "row {r}+{rh} in panel {p}+{ph}");
        };
        host.press_first("tray");
        // The rows below the card's edge aren't drawn: count them in the view.
        let rows = host.core().view().tray_rows.len();
        for _ in 1..rows {
            host.navigate(Key::DownArrow);
        }
        assert_eq!(host.focused("tray-item"), ["themes"], "{theme}");
        in_sight(&host);
        assert!(host.count("tray-item") < rows, "{theme}: the tray is taller than the window");
        // Back up to Edit, into the Edit view, down to the last row again.
        for _ in 0..rows {
            host.navigate(Key::UpArrow);
        }
        host.navigate(Key::Return);
        for _ in 0..rows {
            host.navigate(Key::DownArrow);
        }
        assert_eq!(host.focused("tray-item"), ["themes"], "{theme}: the Edit view");
        in_sight(&host);
        host.navigate(Key::Escape);
        host.navigate(Key::Escape);
        host.dispatch(Command::Pref(launcher_core::Pref::Scale, serde_json::json!(1.0)));
        host.resize(1600.0, 900.0);
        host.settle();
    }
}

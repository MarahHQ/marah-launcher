//! Seam 3: the themes' machine settings, over a fake system plugin. The top bar's network, sound,
//! Bluetooth and drives items (in the bar, or in its tray) and their panels, and Settings → System,
//! show wherever Marah Launcher runs, on a desktop too: each by its area's state. The battery shows
//! wherever the PC has one. On
//! a desktop Restart and Shut down ask in a dialog; as the whole screen they ask by a second press.
//! On Marah OS's USB stick, the Welcome and Install screens.

use std::path::PathBuf;

use crate::common::*;
use launcher_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection, InstallDisk, Installed, NetworkStatus, WifiNetwork,
};
use launcher_core::Command;
use launcher_core::system::{Place, SystemCommand};
use launcher_testkit::FakeSystem;
use slint_interpreter::ComponentHandle;

const WHOLE_SCREEN: Place = Place { whole_screen: true, own_os: true, stick: false };

fn machine() -> FakeSystem {
    let system = FakeSystem::new();
    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wired, name: "Wired connection 1".into(), signal: None };
        m.wifi = vec![
            WifiNetwork { ssid: "Home".into(), signal: 82, secured: true, saved: false, active: false },
            WifiNetwork { ssid: "Cafe".into(), signal: 30, secured: false, saved: false, active: false },
        ];
        m.passwords = vec![("Home".into(), "secret123".into())];
        m.bluetooth_on = true;
        m.devices =
            vec![BluetoothDevice { address: "AA:BB".into(), name: "DualSense".into(), paired: true, connected: true }];
        m.outputs =
            vec![AudioOutput { name: "hdmi".into(), description: "TV or monitor (HDMI)".into(), current: true }];
        m.level = AudioLevel { volume: 40, muted: false };
        m.battery = Some(Battery { percent: 18, charging: false });
    });
    system
}

/// The Classic theme over `system`, at `place`, with the machine read once.
fn shown(system: &FakeSystem, place: Place) -> Composed {
    let plugin = system.plugin();
    let mut host = composed(|h| {
        h.place(place).plugin(plugin).plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
    });
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::System(SystemCommand::Refresh));
    host.settle();
    host
}

fn refreshed(host: &mut Composed) {
    host.dispatch(Command::System(SystemCommand::Refresh));
    host.settle();
}

/// Opens the top bar's tray, then its row of the item `id`, as a click does.
fn from_the_tray(host: &mut Composed, id: &str) {
    host.press_first("tray");
    host.press("tray-item", id);
}

/// The tray's state line of the item `id`, with the tray open.
fn tray_text(host: &Composed, id: &str) -> String {
    let at = host.values("tray-item").iter().position(|v| v == id).unwrap_or_else(|| panic!("no tray row {id}"));
    host.values("tray-text")[at].clone()
}

/// On a desktop the top bar and its tray have every machine item, each by its area's state, and
/// Settings has its System section: the player manages the machine wherever the launcher runs.
/// The bar shows what matters now; the rest waits in the tray. (Before the tray, every icon was
/// in the bar.)
#[test]
fn on_a_desktop_the_top_bar_and_its_tray_show_the_machine_and_settings_has_system() {
    for theme in ["classic", "default"] {
        let mut host = shown(&machine(), Place::default());
        host.ui.switch(&id(theme)).unwrap();
        host.settle();
        assert_eq!(host.value("status:network"), "wired", "{theme}");
        assert_eq!(host.value("status:bluetooth"), "on 1", "{theme}: a controller is connected");
        assert_eq!(host.value("battery"), "18", "{theme}");
        assert_eq!(host.count("status:audio") + host.count("status:drives"), 0, "{theme}: nothing to say now");
        host.press_first("tray");
        for item in ["network", "audio", "bluetooth", "drives", "power"] {
            assert!(host.values("tray-item").contains(&item.to_owned()), "{theme}: the tray lists {item}");
        }
        assert_eq!(tray_text(&host, "audio"), "40%", "{theme}");
        host.navigate(slint::platform::Key::Escape);
        host.dispatch(Command::Navigate(launcher_core::Route::Settings));
        host.settle();
        host.settings_tab("System");
        assert_eq!(host.value("system-internet"), "wired", "{theme}: Settings has System");
        let icons = ["network", "sound", "bluetooth", "refresh"];
        assert_eq!(host.values("row-icon"), icons, "{theme}: each System row has its icon");
    }
}

#[test]
fn a_pc_without_a_battery_shows_none() {
    let system = machine();
    system.with(|m| m.battery = None);
    let host = shown(&system, WHOLE_SCREEN);
    assert_eq!(host.count("battery"), 0);
}

/// The sound comes into the bar while it is muted.
#[test]
fn as_the_whole_screen_the_top_bar_shows_the_network_bluetooth_and_a_muted_sound() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    assert_eq!(host.value("status:network"), "wired");
    assert_eq!(host.count("status:audio"), 0, "not muted: in the tray");
    assert_eq!(host.value("status:bluetooth"), "on 1");

    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wifi, name: "Home".into(), signal: Some(50) };
        m.level.muted = true;
    });
    refreshed(&mut host);
    assert_eq!(host.value("status:network"), "wifi 3");
    assert_eq!(host.value("status:audio"), "muted");
}

#[test]
fn the_network_panel_joins_a_secured_network_with_its_password() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.press_first("status:network");
    assert_eq!(host.value("system-panel"), "network");

    // Look for networks, then Home (secured, not saved) and Cafe.
    use slint::platform::Key;
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    assert_eq!(host.count("wifi-password"), 1, "Home asks for its password");
    for c in "secret123".chars() {
        host.type_text(&c.to_string());
    }
    host.navigate(Key::Return);
    assert_eq!(host.count("wifi-password"), 0, "the password went with the join");
    assert_eq!(system.read(|m| m.network.name.clone()), "Home");
    refreshed(&mut host);
    assert_eq!(host.value("system-job"), "Connected");

    host.navigate(Key::Escape);
    assert_eq!(host.count("system-panel"), 0, "Back closes the panel");
}

#[test]
fn a_wrong_password_says_so_in_the_panel() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.press_first("status:network");
    use slint::platform::Key;
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    for c in "nope".chars() {
        host.type_text(&c.to_string());
    }
    host.navigate(Key::Return);
    assert_eq!(host.value("system-job"), "The password is wrong");
    assert_eq!(system.read(|m| m.network.connection), Connection::Wired);
}

#[test]
fn the_sound_panel_changes_the_volume_with_left_and_right_and_mutes() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    from_the_tray(&mut host, "audio");
    use slint::platform::Key;
    host.navigate(Key::RightArrow);
    assert_eq!(system.read(|m| m.level.volume), 50);
    assert_eq!(host.value("volume"), "50");
    host.navigate(Key::LeftArrow);
    host.navigate(Key::LeftArrow);
    assert_eq!(system.read(|m| m.level.volume), 30);
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    assert!(system.read(|m| m.level.muted));
    assert_eq!(host.value("status:audio"), "muted", "muted, it comes into the bar");
}

#[test]
fn the_bluetooth_panel_turns_it_off_and_a_missing_adapter_says_why() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.press_first("status:bluetooth");
    assert_eq!(host.value("bluetooth-device"), "DualSense");
    host.navigate(slint::platform::Key::Return);
    assert!(!system.read(|m| m.bluetooth_on));
    assert_eq!(host.count("status:bluetooth"), 0, "off, with nothing connected: it leaves the bar");
    host.navigate(slint::platform::Key::Escape);
    host.press_first("tray");
    assert_eq!(tray_text(&host, "bluetooth"), "Off");
    host.navigate(slint::platform::Key::Escape);

    // Without an adapter the item is never in the bar; the tray lists it, says why and opens
    // nothing. Settings → System opens the panel, which says it too.
    let system = machine();
    system.with(|m| m.unavailable = vec![("bluetooth", "This PC has no Bluetooth adapter".into())]);
    let mut host = shown(&system, WHOLE_SCREEN);
    assert_eq!(host.count("status:bluetooth"), 0);
    from_the_tray(&mut host, "bluetooth");
    assert_eq!(host.count("system-panel"), 0, "the disabled item opens nothing");
    assert_eq!(tray_text(&host, "bluetooth"), "This PC has no Bluetooth adapter");
    host.navigate(slint::platform::Key::Escape);
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("System");
    host.press_first("system-bluetooth");
    assert_eq!(host.value("system-reason"), "This PC has no Bluetooth adapter");
}

#[test]
fn settings_system_opens_the_panels_and_restart_asks_again_first() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("System");
    assert_eq!(host.value("system-internet"), "wired");
    host.press_first("system-sound");
    assert_eq!(host.value("system-panel"), "sound");
    host.navigate(slint::platform::Key::Escape);
    host.settings_tab("Power");
    host.press_first("system-restart");
    assert_eq!(host.value("system-restart"), "armed");
    assert_eq!(system.read(|m| m.restarts), 0, "the first press only asks");
    host.press_first("system-restart");
    assert_eq!(system.read(|m| m.restarts), 1);
}

#[test]
fn in_the_marah_session_log_out_asks_again_then_leaves_for_the_login_screen() {
    let system = machine();
    let mut host = shown(&system, Place { whole_screen: true, own_os: false, stick: false });
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("Power");
    let asked = |host: &Composed| host.requests.borrow().contains(&launcher_ui_host::UiRequest::LogOut);
    host.first("system-log-out").invoke_accessible_default_action();
    host.settle();
    assert_eq!(host.value("system-log-out"), "armed");
    assert!(!asked(&host), "the first press only asks");
    host.first("system-log-out").invoke_accessible_default_action();
    assert!(asked(&host));
}

#[test]
fn as_the_whole_screen_quit_is_restart_marah_since_the_session_starts_it_again() {
    for (place, row) in
        [(Place { whole_screen: false, own_os: false, stick: false }, "quit"), (WHOLE_SCREEN, "restart-marah-launcher")]
    {
        let system = machine();
        let mut host = shown(&system, place);
        host.dispatch(Command::Navigate(launcher_core::Route::Settings));
        host.settle();
        host.settings_tab("Power");
        assert_eq!(host.count(if row == "quit" { "restart-marah-launcher" } else { "quit" }), 0, "{place:?}");
        host.first(row).invoke_accessible_default_action();
        assert!(host.requests.borrow().contains(&launcher_ui_host::UiRequest::Quit), "{place:?}");
    }
}

#[test]
fn on_marah_os_there_is_no_log_out_its_session_would_only_start_again() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("Power");
    assert_eq!(host.count("system-log-out"), 0);
}

/// The power menu's items, in order.
fn power_items(host: &Composed) -> Vec<String> {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "power-menu")
        .filter_map(|e| e.accessible_value())
        .map(|v| v.to_string())
        .collect()
}

fn power_item(host: &mut Composed, label: &str) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "power-menu")
        .find(|e| e.accessible_value().is_some_and(|v| v.starts_with(label)))
        .unwrap_or_else(|| panic!("no {label} in the power menu"));
    found.invoke_accessible_default_action();
    host.settle();
}

fn ctrl_p(host: &mut Composed) {
    let ctrl: slint::SharedString = slint::platform::Key::Control.into();
    let window = host.instance().window();
    window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: ctrl.clone() });
    window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "p".into() });
    window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "p".into() });
    window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text: ctrl });
    host.pump();
    host.settle();
}

#[test]
fn holding_the_guide_button_opens_the_power_menu_with_log_out_in_the_marah_session() {
    let system = machine();
    let mut host = shown(&system, Place { whole_screen: true, own_os: false, stick: false });
    assert!(power_items(&host).is_empty());
    host.ui.pad(launcher_ui_host::PadButton::GuideHeld).unwrap();
    host.pump();
    host.settle();
    assert_eq!(power_items(&host), ["Log out", "Restart Marah Launcher", "Restart", "Shut down"]);
    power_item(&mut host, "Log out");
    assert!(host.requests.borrow().contains(&launcher_ui_host::UiRequest::LogOut), "Log out acts at once");
}

#[test]
fn ctrl_p_opens_the_power_menu_and_shut_down_asks_again_first() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    ctrl_p(&mut host);
    assert_eq!(power_items(&host), ["Restart Marah Launcher", "Restart", "Shut down"], "Marah OS: no Log out");
    power_item(&mut host, "Shut down");
    assert_eq!(system.read(|m| m.shutdowns), 0, "the first press only asks");
    assert!(power_items(&host).contains(&"Press again to shut down".to_owned()));
    power_item(&mut host, "Press again to shut down");
    host.pump();
    host.settle();
    assert_eq!(system.read(|m| m.shutdowns), 1);
}

#[test]
fn on_a_desktop_the_power_menu_quits_restarts_and_shuts_down_and_escape_closes_it() {
    let system = machine();
    let mut host = shown(&system, Place::default());
    ctrl_p(&mut host);
    assert_eq!(power_items(&host), ["Quit Marah Launcher", "Restart", "Shut down"], "a desktop: no Log out");
    host.navigate(slint::platform::Key::Escape);
    assert!(power_items(&host).is_empty(), "closed");
    ctrl_p(&mut host);
    power_item(&mut host, "Quit Marah Launcher");
    assert!(host.requests.borrow().contains(&launcher_ui_host::UiRequest::Quit));
}

/// The confirmation dialog's state: what it asks about ("restart" or "shut-down"), or nothing.
fn asking(host: &Composed) -> Option<String> {
    (host.count("power-confirm") > 0).then(|| host.value("power-confirm"))
}

/// On a desktop, Restart and Shut down act on the whole PC: they ask in a dialog with the focus
/// on Cancel. Enter on Cancel, Esc, the pad's Back and a click outside cancel; the other choice
/// acts. In every theme.
#[test]
fn on_a_desktop_restart_asks_in_a_dialog_with_the_focus_on_cancel() {
    use slint::platform::Key;
    for theme in ["classic", "default"] {
        let system = machine();
        let mut host = shown(&system, Place::default());
        host.ui.switch(&id(theme)).unwrap();
        // The pointer reaches the screens once the boot screen is gone.
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.dispatch(Command::Navigate(launcher_core::Route::Settings));
        host.settle();
        host.settings_tab("Power");

        host.press_first("system-restart");
        assert_eq!(asking(&host).as_deref(), Some("restart"), "{theme}: the first press asks in a dialog");
        assert_eq!(host.focused("power-confirm-choice"), ["Cancel"], "{theme}: the focus starts on Cancel");
        let hints = ["Choose", "Select", "Cancel"];
        assert_eq!(host.values("hint"), hints, "{theme}: ✕ selects the focused choice, whichever it is");
        host.navigate(Key::Return);
        assert_eq!(asking(&host), None, "{theme}: Enter on Cancel closes it");
        assert_eq!(system.read(|m| m.restarts), 0, "{theme}");

        host.press_first("system-restart");
        host.navigate(Key::Escape);
        assert_eq!(asking(&host), None, "{theme}: Esc cancels");
        host.press_first("system-restart");
        host.pad(launcher_ui_host::PadButton::East);
        assert_eq!(asking(&host), None, "{theme}: the pad's Back cancels");

        host.press_first("system-restart");
        host.click("power-confirm-title");
        assert_eq!(asking(&host).as_deref(), Some("restart"), "{theme}: a click inside keeps it");
        host.click_at(4.0, 4.0);
        assert_eq!(asking(&host), None, "{theme}: a click outside cancels");
        assert_eq!(system.read(|m| m.restarts), 0, "{theme}: nothing restarted");

        // The choices sit side by side, Cancel on the left: Right and Tab go to Restart, Left
        // and Shift+Tab back.
        host.press_first("system-restart");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("power-confirm-choice"), ["Restart"], "{theme}: Right goes to Restart");
        host.navigate(Key::LeftArrow);
        assert_eq!(host.focused("power-confirm-choice"), ["Cancel"], "{theme}: Left goes back");
        host.navigate(Key::Tab);
        assert_eq!(host.focused("power-confirm-choice"), ["Restart"], "{theme}: Tab goes to Restart");
        host.navigate(Key::Backtab);
        assert_eq!(host.focused("power-confirm-choice"), ["Cancel"], "{theme}: Shift+Tab goes back");
        let choice = |label: &str| {
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "power-confirm-choice")
                .find(|e| e.accessible_value().is_some_and(|v| v == label))
                .unwrap_or_else(|| panic!("{label} in the dialog"))
                .absolute_position()
        };
        let (left, right) = (choice("Cancel"), choice("Restart"));
        assert!(left.x < right.x && left.y == right.y, "{theme}: Cancel is left of Restart, on one line");
        host.navigate(Key::Escape);

        host.press_first("system-restart");
        host.navigate(Key::DownArrow);
        assert_eq!(host.focused("power-confirm-choice"), ["Restart"], "{theme}");
        assert_eq!(host.values("hint"), hints, "{theme}: the same hints on Restart");
        host.navigate(Key::UpArrow);
        assert_eq!(host.focused("power-confirm-choice"), ["Cancel"], "{theme}: Up goes back");
        host.navigate(Key::DownArrow);
        host.pad(launcher_ui_host::PadButton::South);
        host.pump();
        host.settle();
        assert_eq!(system.read(|m| m.restarts), 1, "{theme}: the pad's ✕ on the other choice restarts");
    }
}

/// The keys reach Restart in Settings → Power on a desktop, and its dialog, as a player gets
/// there: Down to the row, Enter opens the dialog on Cancel, and a held Enter doesn't pick.
#[test]
fn on_a_desktop_the_keys_open_restart_s_dialog_and_a_held_enter_picks_nothing() {
    use slint::platform::{Key, WindowEvent};
    let system = machine();
    let mut host = shown(&system, Place::default());
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("Power");
    host.navigate(Key::RightArrow);
    // Quit, then Restart.
    host.navigate(Key::DownArrow);
    assert!(host.lit("system-restart"), "the keys are on Restart");
    host.navigate(Key::Return);
    assert_eq!(asking(&host).as_deref(), Some("restart"));
    assert_eq!(host.focused("power-confirm-choice"), ["Cancel"]);
    // Enter held down repeats: the dialog takes no repeat.
    let enter: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(WindowEvent::KeyPressRepeated { text: enter.clone() });
    host.settle();
    assert_eq!(asking(&host).as_deref(), Some("restart"), "a held Enter picks nothing");
    host.navigate(Key::DownArrow);
    host.instance().window().dispatch_event(WindowEvent::KeyPressRepeated { text: enter });
    host.settle();
    assert_eq!(system.read(|m| m.restarts), 0, "nor on the action");
    host.navigate(Key::Escape);
    assert_eq!(asking(&host), None);
    assert!(host.lit("system-restart"), "the keys are back on Restart");
}

/// From the power menu on a desktop, Shut down asks in the same dialog; a click on its choice acts.
#[test]
fn on_a_desktop_shut_down_from_the_power_menu_asks_in_the_dialog() {
    let system = machine();
    let mut host = shown(&system, Place::default());
    // The pointer reaches the screens once the boot screen is gone.
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.settle();
    ctrl_p(&mut host);
    power_item(&mut host, "Shut down");
    assert!(power_items(&host).is_empty(), "the power menu gives way to the dialog");
    assert_eq!(asking(&host).as_deref(), Some("shut-down"));
    assert_eq!(host.focused("power-confirm-choice"), ["Cancel"]);
    assert_eq!(system.read(|m| m.shutdowns), 0, "the first press only asks");
    let yes = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "power-confirm-choice")
        .find(|e| e.accessible_value().is_some_and(|v| v == "Shut down"))
        .expect("Shut down in the dialog");
    let (at, size) = (yes.absolute_position(), yes.size());
    host.click_at(at.x + size.width / 2.0, at.y + size.height / 2.0);
    host.pump();
    host.settle();
    assert_eq!(system.read(|m| m.shutdowns), 1);
}

#[test]
fn on_the_stick_welcome_leads_to_a_disk_a_firm_question_and_the_install() {
    let system = machine();
    system.with(|m| {
        m.disks = vec![InstallDisk {
            path: PathBuf::from("/dev/nvme0n1"),
            bytes: 512_000_000_000,
            windows: true,
            usb: false,
            model: "Samsung 970".into(),
        }];
        m.install_steps = vec![(40, "Copying Marah OS".into())];
        m.install_result = Some(Ok(Installed { edition: "minimal".into(), mok: true, nvidia_later: false }));
        m.hold_jobs = true;
    });
    let mut host = shown(&system, Place { whole_screen: true, own_os: true, stick: true });
    assert_eq!(host.value("install-os"), "welcome");
    assert_eq!(host.value("install-title"), "Welcome to Marah OS");

    use slint::platform::Key;
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    assert_eq!(host.value("install-os"), "disks");
    assert_eq!(host.value("install-disk"), "Samsung 970 · 512 GB");

    host.navigate(Key::Return);
    assert_eq!(host.value("install-os"), "confirm");
    assert_eq!(host.value("install-title"), "Erase Samsung 970 (512 GB)?");
    assert!(host.value("install-text").contains("including Windows"), "{}", host.value("install-text"));
    assert!(system.read(|m| m.installed_on.is_empty()), "nothing erased yet");

    host.navigate(Key::Return);
    assert_eq!(host.value("install-os"), "installing");
    assert_eq!(host.value("install-progress"), "40");
    assert_eq!(system.read(|m| m.installed_on.clone()), [PathBuf::from("/dev/nvme0n1")]);

    system.finish_jobs();
    host.settle();
    assert_eq!(host.value("install-os"), "done");
    assert!(host.value("install-text").contains("Enroll MOK"), "Secure Boot's key screen is explained");
    host.navigate(Key::Return);
    assert_eq!(system.read(|m| m.restarts), 1, "Restart");
}

/// The stick's install of Marah OS running in `theme`, held until the test ends it; the
/// installer ends it as "Cancelled", as it does when it stops at a cancel.
fn installing(theme: &str) -> (FakeSystem, Composed) {
    use slint::platform::Key;
    let system = machine();
    system.with(|m| {
        m.disks = vec![InstallDisk {
            path: PathBuf::from("/dev/nvme0n1"),
            bytes: 512_000_000_000,
            windows: false,
            usb: false,
            model: "Samsung 970".into(),
        }];
        m.install_steps = vec![(5, "Getting the disk ready".into())];
        m.install_result = Some(Err("Cancelled".into()));
        m.hold_jobs = true;
    });
    let mut host = shown(&system, Place { whole_screen: true, own_os: true, stick: true });
    host.ui.switch(&id(theme)).unwrap();
    // The library is in and its art loaded: the boot screen is gone, and takes no click.
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.load_images();
    host.settle();
    assert_eq!(host.count("boot"), 0, "{theme}");
    // Install on this PC, the disk, Erase and install.
    for key in [Key::DownArrow, Key::Return, Key::Return, Key::Return] {
        host.navigate(key);
    }
    assert_eq!(host.value("install-os"), "installing", "{theme}");
    (system, host)
}

/// The `CancelJob`s the theme asked for and the shell hasn't carried out yet.
fn cancels(host: &Composed) -> Vec<u64> {
    use launcher_ui_host::UiRequest;
    let requests = host.requests.borrow();
    let ids = requests.iter().filter_map(|r| match r {
        UiRequest::Core(Command::System(SystemCommand::CancelJob(id))) => Some(*id),
        _ => None,
    });
    ids.collect()
}

/// One press of the install's Cancel, `how`, with nothing carried out.
fn press_cancel(host: &mut Composed, how: &str) {
    use launcher_ui_host::PadButton;
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    match how {
        "click" => {
            let target = host.first("install-cancel");
            let (at, size) = (target.absolute_position(), target.size());
            let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
            let window = host.instance().window();
            window.dispatch_event(WindowEvent::PointerMoved { position });
            window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
            window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        }
        "Enter" => host.send_key(Key::Return),
        "Esc" => host.send_key(Key::Escape),
        "pad A" => host.shell.ui.pad(PadButton::South).unwrap(),
        "pad B" => host.shell.ui.pad(PadButton::East).unwrap(),
        _ => unreachable!("{how}"),
    }
}

/// While Marah OS installs, Cancel shows with the keys' focus, and the hints show its key. A
/// click, Enter or the pad's A on it, Esc and the pad's B each send one cancel for the install
/// job. Then it says "Stopping…", takes no press, and sends nothing more; the hints go. The
/// installer then decides: here it stopped, and the screen shows "Cancelled".
#[test]
fn while_it_installs_cancel_asks_the_installer_to_stop_once_in_every_theme() {
    const WAYS: [&str; 5] = ["click", "Enter", "pad A", "Esc", "pad B"];
    for theme in ["classic", "default"] {
        for how in WAYS {
            let (system, mut host) = installing(theme);
            let job = host.core().view().system.jobs.last().map(|j| j.id as u64).expect("the install job");
            assert_eq!(host.focused("install-cancel"), ["Cancel"], "{theme}: Cancel has the keys' focus");
            assert_eq!(host.first("install-cancel").accessible_enabled(), Some(true), "{theme}");
            assert!(host.values("hint").contains(&"Cancel".to_owned()), "{theme}: {:?}", host.values("hint"));
            // The pad's ○, or Esc when the keyboard was used last.
            let keys = host.values("hint-key");
            assert!(keys == ["○"] || keys == ["Esc"], "{theme}: the Back key cancels: {keys:?}");

            press_cancel(&mut host, how);
            assert_eq!(cancels(&host), [job], "{theme} {how}: one cancel, for the install job");
            host.pump();
            host.settle();
            assert_eq!(host.value("install-os"), "installing", "{theme} {how}: the installer decides");
            assert_eq!(host.value("install-cancel"), "Stopping…", "{theme} {how}");
            assert_eq!(host.first("install-cancel").accessible_enabled(), Some(false), "{theme} {how}");
            assert!(host.focused("install-cancel").is_empty(), "{theme} {how}: no ring on what can't be pressed");
            assert_eq!(host.count("hint"), 0, "{theme} {how}: no key does anything now");

            for again in WAYS {
                press_cancel(&mut host, again);
                assert!(cancels(&host).is_empty(), "{theme} {how}, then {again}: sent a second cancel");
                host.pump();
                host.settle();
            }
            assert_eq!(host.value("install-text"), "Getting the disk ready", "{theme}: the installer's own words");

            system.finish_jobs();
            host.settle();
            assert_eq!(host.value("install-os"), "failed", "{theme} {how}");
            assert_eq!(host.value("install-text"), "Cancelled", "{theme} {how}");
        }
    }
}

#[test]
fn on_the_stick_try_it_leaves_the_launcher_and_choose_partitions_starts_that_installer() {
    let system = machine();
    let mut host = shown(&system, Place { whole_screen: true, own_os: true, stick: true });
    use slint::platform::Key;
    host.navigate(Key::Return);
    assert_eq!(host.count("install-os"), 0, "Try it");

    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("System");
    let icons = ["network", "sound", "bluetooth", "refresh", "usb"];
    assert_eq!(host.values("row-icon"), icons, "each System row has its icon, Install too");
    host.press_first("system-install");
    assert_eq!(host.value("install-os"), "disks");
    // No disk: "Choose partitions instead" is first.
    host.navigate(Key::Return);
    assert_eq!(system.read(|m| m.partition_choices), 1);
}

/// On the stick, in every theme, the disks' step says why when the installer couldn't list the
/// disks, and "No disk of 40 GB or more…" when it listed none. The step still opens, and its
/// choices still work.
#[test]
fn on_the_stick_the_disks_step_says_why_the_installer_offers_no_disk() {
    use slint::platform::Key;
    const WHY: &str = "The USB stick Marah OS started from wasn't found, so no disk is offered.";
    const NONE: &str = "No disk of 40 GB or more is free to install on.";
    for theme in ["classic", "default"] {
        for (unavailable, said) in [(Some(WHY), WHY), (None, NONE)] {
            let system = machine();
            if let Some(why) = unavailable {
                system.with(|m| m.unavailable.push(("os_install", why.into())));
            }
            let mut host = shown(&system, Place { whole_screen: true, own_os: true, stick: true });
            host.ui.switch(&id(theme)).unwrap();
            host.settle();
            host.navigate(Key::DownArrow);
            host.navigate(Key::Return);
            assert_eq!(host.value("install-os"), "disks", "{theme}");
            assert_eq!(host.values("install-no-disk"), [said], "{theme}");

            // "Choose partitions instead" is first.
            host.navigate(Key::Return);
            assert_eq!(system.read(|m| m.partition_choices), 1, "{theme}");
        }
    }
}

#[test]
fn the_keys_reach_the_icons_at_the_end_of_the_top_bar_and_move_between_panels() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    use slint::platform::Key;
    host.navigate(Key::UpArrow);
    // Left of the first item is the last, the tray's ⌃: the bar goes round. Left again is
    // Settings, then the battery, then Bluetooth.
    for _ in 0..4 {
        host.navigate(Key::LeftArrow);
    }
    host.navigate(Key::Return);
    assert_eq!(host.value("system-panel"), "bluetooth");
    host.navigate(Key::LeftArrow);
    assert_eq!(host.value("system-panel"), "sound", "left goes to the panel beside");
    host.navigate(Key::Escape);
    assert_eq!(host.count("system-panel"), 0);
}

#[test]
fn l1_and_r1_reach_every_item_of_the_top_bar_again_and_again_round_the_ends() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    use slint::platform::Key;
    // Without a catalog the bar has seven stops: Home, Library, the internet, Bluetooth (a
    // controller is connected), the battery, Settings and the tray's ⌃. R1 three times from
    // Home is Bluetooth.
    for _ in 0..3 {
        host.navigate(Key::Tab);
    }
    host.navigate(Key::Return);
    assert_eq!(host.value("system-panel"), "bluetooth");
    host.navigate(Key::Escape);
    // Three more are the battery, Settings and the ⌃, the last; L1 three times goes back to
    // Bluetooth. Once round, still there.
    for _ in 0..3 {
        host.navigate(Key::Tab);
    }
    for _ in 0..3 {
        host.navigate(Key::Backtab);
    }
    for _ in 0..7 {
        host.navigate(Key::Backtab);
    }
    host.navigate(Key::Return);
    assert_eq!(host.value("system-panel"), "bluetooth", "L1 wraps and every stop answers again");
}

/// The bar ends with Settings, then the tray's ⌃, which the keys reach last. (Before the tray,
/// Settings was the last icon.)
#[test]
fn as_the_whole_screen_the_bar_ends_with_settings_then_the_tray_and_the_keys_reach_them_last() {
    use launcher_ui_api::ScreenId;
    use slint::platform::Key;
    let x = |host: &Composed, label: &str| host.first(label).absolute_position().x;
    let mut host = shown(&machine(), WHOLE_SCREEN);
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
        for icon in ["status:network", "status:bluetooth", "battery"] {
            assert!(x(&host, "Settings") > x(&host, icon), "{theme}: Settings is after {icon}");
        }
        assert!(x(&host, "tray") > x(&host, "Settings"), "{theme}: the ⌃ is after Settings");
        // L1 from Home goes round to the top bar's last item: the ⌃.
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        assert_eq!(host.value("tray"), "open", "{theme}: the last item opens the tray");
        host.navigate(Key::Escape);
        // The keys are still on the ⌃: one step back is Settings.
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        host.settle();
        assert_eq!(host.core().view().screen, ScreenId::Settings, "{theme}: the one before is Settings");
    }
}

/// As the whole screen, the top bar from its end: the clock, the tray's ⌃, Settings, then the
/// items in the bar (the battery, Bluetooth, the internet).
#[test]
fn as_the_whole_screen_the_top_bar_ends_with_the_clock_settings_then_the_machine() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.platform.set_local_time("9:38 PM");
    host.on_tick();
    host.settle();
    let x = |label: &str| host.first(label).absolute_position().x;
    let from_the_end = ["clock", "tray", "Settings", "status:power", "status:bluetooth", "status:network"];
    for pair in from_the_end.windows(2) {
        assert!(x(pair[0]) > x(pair[1]), "{} {} after {} {}", pair[0], x(pair[0]), pair[1], x(pair[1]));
    }
}

/// A USB stick, plugged in and mounted, and a drive inside the PC that isn't mounted.
fn with_drives(system: &FakeSystem, media: &std::path::Path) {
    let stick = media.join("STICK");
    std::fs::create_dir_all(&stick).unwrap();
    system.with(|m| {
        m.mount_at = Some(media.to_path_buf());
        m.drives = vec![
            launcher_api::SystemDrive {
                device: "/dev/sda1".into(),
                disk: "/dev/sda".into(),
                uuid: "1A2B-3C4D".into(),
                label: "STICK".into(),
                size: 64_000_000_000,
                filesystem: "exfat".into(),
                usb: true,
                mount: Some(stick),
                free: Some(40_000_000_000),
                ..launcher_api::SystemDrive::default()
            },
            launcher_api::SystemDrive {
                device: "/dev/nvme0n1p4".into(),
                disk: "/dev/nvme0n1".into(),
                uuid: "5A1C".into(),
                label: "Games disk".into(),
                size: 500_000_000_000,
                filesystem: "ntfs".into(),
                ..launcher_api::SystemDrive::default()
            },
        ];
    });
}

/// The drives' icon comes into the bar while a USB drive is plugged in, after Bluetooth; the keys
/// reach it before the battery and Settings, and its panel lists every drive and mounts, ejects
/// and opens them. With no drive to take out, it waits in the tray.
#[test]
fn the_drives_icon_and_its_panel_list_every_drive_in_every_theme() {
    use slint::platform::Key;
    let media = tempfile::tempdir().unwrap();
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh));
    host.settle();
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
        assert_eq!(host.count("status:drives"), 0, "{theme}: no drive to take out: in the tray");
    }
    // A start with the drives there: a drive plugged in while it runs would be mounted at once.
    with_drives(&system, media.path());
    let mut host = shown(&system, WHOLE_SCREEN);
    host.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh));
    refreshed(&mut host);
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
        let x = |label: &str| host.first(label).absolute_position().x;
        assert!(x("status:drives") > x("status:bluetooth") && x("Settings") > x("status:drives"), "{theme}");

        // L1 four times from Home: the ⌃, Settings, the battery, then the drives' icon.
        for _ in 0..4 {
            host.navigate(Key::Backtab);
        }
        host.navigate(Key::Return);
        host.settle();
        assert_eq!(host.value("system-panel"), "drives", "{theme}: the keys reach the drives' icon");
        // One row per drive, each with its buttons; the first button of the first drive has the
        // focus, and Right moves to the next button.
        assert_eq!(host.values("drive"), ["STICK", "Games disk"], "{theme}: one row per drive");
        assert_eq!(host.values("open"), ["STICK"], "{theme}");
        assert_eq!(host.values("use-for-games"), ["STICK"], "{theme}");
        assert_eq!(host.values("eject"), ["STICK"], "{theme}");
        assert_eq!(host.values("mount"), ["Games disk"], "{theme}");
        assert_eq!(host.count("unmount"), 0, "{theme}");
        assert_eq!(host.focused("open"), ["STICK"], "{theme}: the first button has the focus");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("use-for-games"), ["STICK"], "{theme}: Right moves to the next button");
        host.navigate(Key::Escape);
    }

    // The keys: Down to the drive inside the PC, Enter on its Mount.
    host.dispatch(Command::Navigate(launcher_core::Route::Home));
    host.settle();
    host.press_first("status:drives");
    host.navigate(Key::DownArrow);
    assert_eq!(host.focused("mount"), ["Games disk"], "Down moves to the next drive, on its first button");
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(system.read(|m| m.drive_actions.clone()), ["mount /dev/nvme0n1p4"]);
    // The mouse: a click on the stick's Use for games adds it to the game folders.
    host.press_first("use-for-games");
    host.settle();
    let games = host.core().kernel().config().get(&["app", "folders", "games"]);
    assert!(games.to_string().contains("STICK"), "{games}");
    // The pad: right twice to Eject on the stick, and ✕. The stick goes; the icon stays.
    host.navigate(Key::Escape);
    host.press_first("status:drives");
    host.pad(launcher_ui_host::PadButton::Right);
    assert_eq!(host.focused("use-for-games"), ["STICK"]);
    host.pad(launcher_ui_host::PadButton::Right);
    assert_eq!(host.focused("eject"), ["STICK"], "Eject is the last button");
    host.pad(launcher_ui_host::PadButton::South);
    host.settle();
    refreshed(&mut host);
    assert_eq!(system.read(|m| m.drive_actions.clone()), ["mount /dev/nvme0n1p4", "eject /dev/sda"]);
    assert_eq!(host.count("status:drives"), 0, "nothing to take out: it leaves the bar");
    host.navigate(Key::Escape);
    from_the_tray(&mut host, "drives");
    assert_eq!(host.values("drive"), ["Games disk"], "the drive inside the PC is still there");
}

/// On a desktop, with a USB drive plugged in, the drives' icon is in the bar with the others: the
/// keys reach it, its panel opens, and left goes to the panel beside it, Bluetooth's.
#[test]
fn on_a_desktop_the_drives_icon_is_there_and_opens_its_panel() {
    use slint::platform::Key;
    let media = tempfile::tempdir().unwrap();
    let system = machine();
    with_drives(&system, media.path());
    let mut host = shown(&system, Place::default());
    host.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh));
    refreshed(&mut host);
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
        assert_eq!(host.count("status:drives"), 1, "{theme}");
        assert_eq!(host.count("status:network") + host.count("status:bluetooth"), 2, "{theme}: the others too");
        for _ in 0..4 {
            host.navigate(Key::Backtab);
        }
        host.navigate(Key::Return);
        host.settle();
        assert_eq!(host.value("system-panel"), "drives", "{theme}: the keys reach it");
        host.navigate(Key::LeftArrow);
        assert_eq!(host.value("system-panel"), "bluetooth", "{theme}: the panel beside it");
        host.navigate(Key::Escape);
    }
}

/// Opened from a Settings row, a panel opens next to the row: under it, its far edges lined up,
/// and never over it. Esc closes it and the keys are on the row again; the page scrolling under
/// it closes it.
#[test]
fn a_panel_opened_from_a_settings_row_opens_next_to_the_row_in_every_theme() {
    use slint::platform::Key;
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Settings));
        host.settle();
        host.settings_tab("System");
        host.navigate(Key::RightArrow);
        host.navigate(Key::DownArrow);
        host.navigate(Key::Return);
        assert_eq!(host.value("system-panel"), "sound", "{theme}");
        let (row, panel) = (host.first("system-sound"), host.first("system-panel"));
        let (r, p) = (row.absolute_position(), panel.absolute_position());
        let (rs, ps) = (row.size(), panel.size());
        let apart = p.y >= r.y + rs.height || p.y + ps.height <= r.y;
        assert!(apart, "{theme}: the panel doesn't cover its row: row {r:?} {rs:?}, panel {p:?} {ps:?}");
        assert!(((p.x + ps.width) - (r.x + rs.width)).abs() < 1.0, "{theme}: the far edges line up");
        assert!(p.y > 0.0 && p.y + ps.height <= 900.0, "{theme}: inside the window");

        host.navigate(Key::Escape);
        assert_eq!(host.count("system-panel"), 0, "{theme}");
        assert!(host.lit("system-sound"), "{theme}: the keys are on the row again");
        host.navigate(Key::Return);
        assert_eq!(host.value("system-panel"), "sound", "{theme}: the row opens it again");
        host.wheel("settings-page", 3);
        assert_eq!(host.count("system-panel"), 0, "{theme}: the page scrolled under it: it closed");
    }
}

/// The machine `system` with every part's first read taking 30 seconds on the fake clock, at
/// `place` in `theme`: no read ends until the test moves the clock.
fn reading(system: &FakeSystem, place: Place, theme: &str) -> Composed {
    let slow = ["network.unavailable", "bluetooth.unavailable", "audio.unavailable", "power.unavailable"];
    let slow = slow.into_iter().chain(["drives.unavailable", "drives"]);
    system.with(|m| m.slow = slow.map(|r| (r, std::time::Duration::from_secs(30))).collect());
    let plugin = system.plugin();
    let mut host = composed(|h| {
        let h = h.place(place).call_timeout(std::time::Duration::from_secs(120)).plugin(plugin);
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
    });
    host.ui.start().unwrap();
    host.ui.switch(&id(theme)).unwrap();
    host.resize(1600.0, 900.0);
    host.settle();
    host
}

/// On a desktop the machine's own tick reads every area, with no Refresh asked: each item loads,
/// then shows its state. In the bar while they load: the internet and the battery, which always
/// show; the others come when they matter.
#[test]
fn on_a_desktop_the_tick_reads_every_icon() {
    for theme in ["classic", "default"] {
        let system = machine();
        let mut host = reading(&system, Place::default(), theme);
        for label in ["status:network", "battery"] {
            assert_eq!(host.value(label), "loading", "{theme}: {label}");
        }
        assert_eq!(host.count("status:bluetooth"), 0, "{theme}: not needed before it is read");
        host.pump();
        host.advance(std::time::Duration::from_secs(31));
        host.settle();
        assert_eq!(host.value("status:network"), "wired", "{theme}");
        assert_eq!(host.core().view().tray_item("audio").text, "40%", "{theme}");
        assert_eq!(host.value("status:bluetooth"), "on 1", "{theme}");
        assert_eq!(host.value("battery"), "18", "{theme}");
    }
}

/// Before its first read each part is loading: no reason shows, the items in the bar have
/// "loading" as their value, the tray says "Checking…" for each, and an item opens its panel on a
/// loading row. When the reads end each shows its state; Settings hasn't moved.
#[test]
fn each_status_icon_is_in_place_and_loading_until_its_part_is_read() {
    for theme in ["classic", "default"] {
        loading_until_read(theme);
    }
}

fn loading_until_read(theme: &str) {
    use slint::platform::Key;
    let system = machine();
    let mut host = reading(&system, WHOLE_SCREEN, theme);
    let vm = host.core().view().system;
    for area in ["network", "audio", "bluetooth", "power", "drives"] {
        assert_eq!(vm.area(area).state, "loading", "{vm:?}");
    }
    assert!(vm.drives_listing, "{vm:?}");
    assert!(vm.area("network").reason.is_empty() && vm.area("drives").reason.is_empty(), "no reason before the read");
    for label in ["status:network", "battery"] {
        assert_eq!(host.value(label), "loading", "{label}");
    }
    let settings = host.first("Settings").absolute_position().x;

    // The mouse: the tray says each is read; a click on a row opens the panel at once, on its
    // loading row, before its reads end.
    host.first("tray").invoke_accessible_default_action();
    host.settle();
    for item in ["network", "audio", "bluetooth", "drives"] {
        assert_eq!(tray_text(&host, item), "Checking…", "{item}");
    }
    assert!(host.count("loading-line") >= 4, "each row's line sweeps");
    // Invoked with the reads still running: nothing is carried out.
    let row = host.all("tray-item").into_iter().find(|r| r.accessible_value().as_deref() == Some("drives")).unwrap();
    row.invoke_accessible_default_action();
    host.settle();
    assert_eq!(host.value("system-panel"), "drives");
    assert_eq!(host.values("loading"), ["Reading drives…"]);
    assert_eq!(host.count("drive"), 0);
    // The keys: Escape closes it as an empty panel closes; from Home, four steps back (the ⌃,
    // Settings, the battery), the internet's icon has the focus and says "Checking…" under it;
    // Enter opens its panel.
    host.send_key(Key::Escape);
    host.settle();
    for _ in 0..4 {
        host.send_key(Key::Backtab);
    }
    host.settle();
    assert_eq!(host.count("status-checking"), 1, "the internet's icon has the keys' focus");
    host.send_key(Key::Return);
    host.settle();
    assert_eq!(host.value("system-panel"), "network", "Enter opens it");
    assert_eq!(host.values("loading"), ["Reading the network…"]);

    // The reads end: what the clicks and keys asked for runs, as the moving clock lets it.
    host.pump();
    host.advance(std::time::Duration::from_secs(31));
    host.settle();
    let vm = host.core().view().system;
    let read = |area: &str| vm.area(area).state != "loading";
    assert!(read("network") && read("drives") && !vm.drives_listing && vm.area("drives").ready(), "{vm:?}");
    assert_eq!(host.count("loading"), 0, "the panel fills in");
    assert_eq!(host.value("status:network"), "wired");
    assert_eq!(host.value("battery"), "18");
    assert_eq!(host.count("loading-line"), 0, "nothing loads");
    assert_eq!(host.first("Settings").absolute_position().x, settings, "Settings didn't move");
}

/// A part read slowly holds none of the others: the network still loads while the drives' icon
/// is ready.
#[test]
fn a_slow_network_read_leaves_the_drives_icon_ready() {
    let system = machine();
    system.with(|m| m.slow = vec![("network.unavailable", std::time::Duration::from_secs(30))]);
    let plugin = system.plugin();
    let mut host = composed(|h| h.place(WHOLE_SCREEN).call_timeout(std::time::Duration::from_secs(120)).plugin(plugin));
    host.ui.start().unwrap();
    host.run_until_idle();
    host.shell.render().unwrap();
    host.settle();
    assert_eq!(host.value("status:network"), "loading");
    let view = host.core().view();
    assert_ne!(view.tray_item("drives").state, "loading");
    assert_ne!(view.tray_item("audio").state, "loading");
}

/// Drives that turn out not to be managed here: the drives' item never comes into the bar
/// (before the tray, its icon stayed in the bar, disabled). The tray lists it, dimmed, with why:
/// the mouse reaches it, a click opens nothing, and the row says why.
#[test]
fn the_drives_item_stays_in_the_tray_when_drives_turn_out_unavailable() {
    for theme in ["classic", "default"] {
        let system = machine();
        system.with(|m| m.unavailable.push(("drives", "No udisks here".into())));
        let mut host = reading(&system, WHOLE_SCREEN, theme);
        assert_eq!(host.core().view().tray_item("drives").state, "loading", "{theme}");
        let settings = host.first("Settings").absolute_position().x;
        host.advance(std::time::Duration::from_secs(31));
        host.settle();
        assert_eq!(host.count("status:drives"), 0, "{theme}: never in the bar");
        assert_eq!(host.first("Settings").absolute_position().x, settings, "{theme}: Settings didn't move");

        // The pointer reaches the bar once the boot screen is gone.
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.settle();
        host.click("tray");
        assert_eq!(host.value("tray"), "open", "{theme}");
        assert_eq!(tray_text(&host, "drives"), "No udisks here", "{theme}: the row says why");
        host.click_value("tray-item", "drives");
        assert_eq!(host.count("system-panel"), 0, "{theme}: a click opens nothing");
        assert_eq!(host.value("tray"), "open", "{theme}: the tray stays");
        host.press("tray-item", "drives");
        assert_eq!(host.count("system-panel"), 0, "{theme}: nor does its default action");
        host.navigate(slint::platform::Key::Escape);
    }
}

/// An unusable item's reason is its own line in the tray, one line inside the window: a long
/// reason ends in "…", as tall as a short one. (Before the tray, it showed on a chip under the
/// disabled icon.)
#[test]
fn an_unusable_item_s_reason_is_one_line_in_the_tray_inside_the_window() {
    const LONG: &str = "Drives can't be managed here: this system has no udisks service running";
    for theme in ["classic", "default"] {
        let system = machine();
        system.with(|m| {
            m.unavailable.push(("bluetooth", "No adapter".into()));
            m.unavailable.push(("drives", LONG.into()));
        });
        let mut host = shown(&system, WHOLE_SCREEN);
        host.ui.switch(&id(theme)).unwrap();
        // The drives are read on their own tick.
        host.advance(std::time::Duration::from_secs(31));
        host.settle();
        host.press_first("tray");
        assert_eq!(tray_text(&host, "drives"), LONG, "{theme}");
        assert_eq!(tray_text(&host, "bluetooth"), "No adapter", "{theme}");
        assert_eq!(host.count("status-reason"), 0, "{theme}: no chip: the row says it");
        let window = host.shown().window();
        let right = window.size().width as f32 / window.scale_factor();
        let rows = host.values("tray-item");
        let line = |id: &str| {
            let at = rows.iter().position(|v| v == id).unwrap();
            let text = host.all("tray-text").swap_remove(at);
            (text.absolute_position().x, text.size().width, text.size().height)
        };
        let (x, width, long_height) = line("drives");
        assert!(x >= 0.0 && x + width <= right, "{theme}: inside the window: {x} + {width} > {right}");
        let (_, _, short_height) = line("bluetooth");
        assert_eq!(long_height, short_height, "{theme}: the long reason is on one line");
        host.navigate(slint::platform::Key::Escape);
    }
}

/// An unsupported item is never in the bar. In the tray it takes the keys' and the pad's focus
/// and says why, and its ✕ and Enter open nothing. Left and right from the panel beside it go
/// past it.
#[test]
fn an_unusable_item_takes_the_focus_in_the_tray_and_says_why_but_opens_nothing() {
    use slint::platform::Key;
    for theme in ["classic", "default"] {
        let system = machine();
        system.with(|m| m.unavailable.push(("bluetooth", "This PC has no Bluetooth adapter".into())));
        let mut host = shown(&system, WHOLE_SCREEN);
        host.ui.switch(&id(theme)).unwrap();
        host.settle();
        assert_eq!(host.count("status:bluetooth"), 0, "{theme}: never in the bar");
        host.press_first("tray");
        // The internet, the sound, then Bluetooth.
        host.navigate(Key::DownArrow);
        host.navigate(Key::DownArrow);
        assert_eq!(host.focused("tray-item"), ["bluetooth"], "{theme}");
        assert_eq!(tray_text(&host, "bluetooth"), "This PC has no Bluetooth adapter", "{theme}: it says why");
        host.pad(launcher_ui_host::PadButton::South);
        assert_eq!(host.count("system-panel"), 0, "{theme}: ✕ opens nothing");
        host.navigate(Key::Return);
        assert_eq!(host.count("system-panel"), 0, "{theme}: nor does Enter");
        assert_eq!(host.value("tray"), "open", "{theme}");
        host.navigate(Key::Escape);

        // The sound's panel, off its volume row: right goes past Bluetooth to the drives.
        from_the_tray(&mut host, "audio");
        assert_eq!(host.value("system-panel"), "sound", "{theme}");
        host.navigate(Key::DownArrow);
        host.navigate(Key::RightArrow);
        assert_eq!(host.value("system-panel"), "drives", "{theme}: past the disabled one");
        host.navigate(Key::Escape);
    }
}

/// With Animations off the loading line stands still, as a dotted line; with them on it moves.
#[test]
fn with_animations_off_the_loading_line_stands_still() {
    for theme in ["classic", "default"] {
        let system = machine();
        let mut host = reading(&system, WHOLE_SCREEN, theme);
        assert!(
            host.values("loading-line").iter().all(|v| v == "moving"),
            "{theme}: {:?}",
            host.values("loading-line")
        );
        host.dispatch(Command::Pref(launcher_core::Pref::Animations, serde_json::json!(false)));
        host.settle();
        let lines = host.values("loading-line");
        assert_eq!(
            lines.len(),
            2,
            "{theme}: the two items in the bar while they load, the internet and the battery: {lines:?}"
        );
        assert!(lines.iter().all(|v| v == "still"), "{theme}: {lines:?}");
    }
}

/// Settings → System while the parts are read: each row says "loading", not a state it doesn't
/// know yet, and no reason shows.
#[test]
fn the_system_rows_say_they_are_loading() {
    let system = machine();
    let mut host = reading(&system, WHOLE_SCREEN, "classic");
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("System");
    for label in ["system-internet", "system-sound", "system-bluetooth"] {
        assert_eq!(host.value(label), "loading", "{label}");
    }
}

/// A loading icon takes the pad's ✕, in both themes: it opens its panel.
#[test]
fn a_loading_icon_takes_the_pad() {
    use slint::platform::Key;
    for theme in ["classic", "default"] {
        let mut host = reading(&machine(), WHOLE_SCREEN, theme);
        assert_eq!(host.value("status:network"), "loading", "{theme}");
        // The ⌃, Settings, the battery, then the internet.
        for _ in 0..4 {
            host.send_key(Key::Backtab);
        }
        host.settle();
        host.pad(launcher_ui_host::PadButton::South);
        assert_eq!(host.value("system-panel"), "network", "{theme}: ✕ opens it");
    }
}

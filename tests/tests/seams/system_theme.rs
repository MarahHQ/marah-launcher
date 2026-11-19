//! Seam 3: the themes' machine settings, over a fake system plugin. The top bar's network, sound,
//! Bluetooth and drives icons and their panels, and Settings → System, show wherever Marah Launcher
//! runs, on a desktop too: each by its area's state. The battery shows wherever the PC has one. On
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

/// On a desktop the top bar has every machine icon, each by its area's state, and Settings has
/// its System section: the player manages the machine wherever the launcher runs.
#[test]
fn on_a_desktop_the_top_bar_shows_every_machine_icon_and_settings_has_system() {
    for theme in ["classic", "default"] {
        let mut host = shown(&machine(), Place::default());
        host.ui.switch(&id(theme)).unwrap();
        host.settle();
        assert_eq!(host.value("status:network"), "wired", "{theme}");
        assert_eq!(host.value("status:sound"), "40", "{theme}");
        assert_eq!(host.value("status:bluetooth"), "on 1", "{theme}");
        assert_eq!(host.count("status:drives"), 1, "{theme}");
        assert_eq!(host.value("battery"), "18", "{theme}");
        host.dispatch(Command::Navigate(launcher_core::Route::Settings));
        host.settle();
        host.settings_tab("System");
        assert_eq!(host.value("system-internet"), "wired", "{theme}: Settings has System");
    }
}

#[test]
fn a_pc_without_a_battery_shows_none() {
    let system = machine();
    system.with(|m| m.battery = None);
    let host = shown(&system, WHOLE_SCREEN);
    assert_eq!(host.count("battery"), 0);
}

#[test]
fn as_the_whole_screen_the_top_bar_shows_the_network_sound_and_bluetooth() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    assert_eq!(host.value("status:network"), "wired");
    assert_eq!(host.value("status:sound"), "40");
    assert_eq!(host.value("status:bluetooth"), "on 1");

    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wifi, name: "Home".into(), signal: Some(50) };
        m.level.muted = true;
    });
    refreshed(&mut host);
    assert_eq!(host.value("status:network"), "wifi 3");
    assert_eq!(host.value("status:sound"), "muted");
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
    host.press_first("status:sound");
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
    assert_eq!(host.value("status:sound"), "muted");
}

#[test]
fn the_bluetooth_panel_turns_it_off_and_a_missing_adapter_says_why() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.press_first("status:bluetooth");
    assert_eq!(host.value("bluetooth-device"), "DualSense");
    host.navigate(slint::platform::Key::Return);
    assert!(!system.read(|m| m.bluetooth_on));
    assert_eq!(host.value("status:bluetooth"), "off");

    // Without an adapter the icon is disabled and says why; Settings → System opens the panel,
    // which says it too.
    let system = machine();
    system.with(|m| m.unavailable = vec![("bluetooth", "This PC has no Bluetooth adapter".into())]);
    let mut host = shown(&system, WHOLE_SCREEN);
    host.press_first("status:bluetooth");
    assert_eq!(host.count("system-panel"), 0, "the disabled icon opens nothing");
    assert_eq!(host.description("status:bluetooth"), "This PC has no Bluetooth adapter");
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
    host.press_first("system-install");
    assert_eq!(host.value("install-os"), "disks");
    // No disk: "Choose partitions instead" is first.
    host.navigate(Key::Return);
    assert_eq!(system.read(|m| m.partition_choices), 1);
}

#[test]
fn the_keys_reach_the_icons_at_the_end_of_the_top_bar_and_move_between_panels() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    use slint::platform::Key;
    host.navigate(Key::UpArrow);
    // Left of the first item is the last, Settings: the bar goes round. Left again is the drives'
    // icon, and once more Bluetooth.
    for _ in 0..3 {
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
    // Without a catalog the bar has ten stops: Home, Library, three screen buttons, the four
    // icons (the drives' last) and Settings. R1 seven times from Home is Bluetooth.
    for _ in 0..7 {
        host.navigate(Key::Tab);
    }
    host.navigate(Key::Return);
    assert_eq!(host.value("system-panel"), "bluetooth");
    host.navigate(Key::Escape);
    // Two more are the drives' icon and Settings, the last; L1 twice goes back to Bluetooth.
    // Once round, still there.
    host.navigate(Key::Tab);
    host.navigate(Key::Tab);
    host.navigate(Key::Backtab);
    host.navigate(Key::Backtab);
    for _ in 0..10 {
        host.navigate(Key::Backtab);
    }
    host.navigate(Key::Return);
    assert_eq!(host.value("system-panel"), "bluetooth", "L1 wraps and every stop answers again");
}

#[test]
fn as_the_whole_screen_settings_is_the_last_icon_in_the_top_bar_and_the_keys_reach_it_last() {
    use launcher_ui_api::ScreenId;
    use slint::platform::Key;
    let x = |host: &Composed, label: &str| host.first(label).absolute_position().x;
    let mut host = shown(&machine(), WHOLE_SCREEN);
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
        for icon in ["Themes", "status:network", "status:sound", "status:bluetooth", "status:drives", "battery"] {
            assert!(x(&host, "Settings") > x(&host, icon), "{theme}: Settings is after {icon}");
        }
        // L1 from Home goes round to the top bar's last item: Settings.
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        assert_eq!(host.core().view().screen, ScreenId::Settings, "{theme}: the last item opens Settings");
        // From Home again, two steps back: the drives' icon, the one before it.
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        host.settle();
        host.navigate(Key::Backtab);
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        host.settle();
        assert_eq!(host.value("system-panel"), "drives", "{theme}: the one before is the drives' icon");
        host.navigate(Key::Escape);
    }
}

/// As the whole screen, the top bar from its end: the clock, Settings, the machine's icons
/// (bluetooth, sound, network), then the screen buttons.
#[test]
fn as_the_whole_screen_the_top_bar_ends_with_the_clock_settings_then_the_machine() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.platform.set_local_time("9:38 PM");
    host.on_tick();
    host.settle();
    let x = |label: &str| host.first(label).absolute_position().x;
    let from_the_end = ["clock", "Settings", "status:bluetooth", "status:sound", "status:network", "Themes"];
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

/// The drives' icon shows wherever drives can be managed, after Bluetooth, with or without a
/// USB drive; the keys reach it before Settings, and its panel lists every drive and mounts,
/// ejects and opens them.
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
        assert_eq!(host.count("status:drives"), 1, "{theme}: no drive yet, and the icon is there");
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

        // L1 twice from Home: Settings, then the drives' icon before it.
        host.navigate(Key::Backtab);
        host.navigate(Key::Backtab);
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
    assert_eq!(host.count("status:drives"), 1, "the drive inside the PC is still there");
    assert_eq!(host.values("drive"), ["Games disk"]);
}

/// On a desktop the drives' icon is there with the others: the keys reach it, its panel opens, and
/// left goes to the panel beside it, Bluetooth's.
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
        host.navigate(Key::Backtab);
        host.navigate(Key::Backtab);
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

/// On a desktop the machine's own tick reads every area, with no Refresh asked: each icon loads,
/// then shows its state.
#[test]
fn on_a_desktop_the_tick_reads_every_icon() {
    for theme in ["classic", "default"] {
        let system = machine();
        let mut host = reading(&system, Place::default(), theme);
        for label in ["status:network", "status:sound", "status:bluetooth", "status:drives", "battery"] {
            assert_eq!(host.value(label), "loading", "{theme}: {label}");
        }
        host.pump();
        host.advance(std::time::Duration::from_secs(31));
        host.settle();
        assert_eq!(host.value("status:network"), "wired", "{theme}");
        assert_eq!(host.value("status:sound"), "40", "{theme}");
        assert_eq!(host.value("status:bluetooth"), "on 1", "{theme}");
        assert_eq!(host.value("battery"), "18", "{theme}");
    }
}

/// Before its first read each part is loading: no reason shows, every icon is in place with
/// "loading" as its value, and it opens its panel on a loading row. When the reads end each
/// shows its state, and the bar hasn't moved.
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
    for label in ["status:network", "status:sound", "status:bluetooth", "status:drives", "battery"] {
        assert_eq!(host.value(label), "loading", "{label}");
    }
    let settings = host.first("Settings").absolute_position().x;

    // The mouse: a click opens the panel at once, on its loading row, before its reads end.
    host.first("status:drives").invoke_accessible_default_action();
    host.settle();
    assert_eq!(host.value("system-panel"), "drives");
    assert_eq!(host.values("loading"), ["Reading drives…"]);
    assert_eq!(host.count("drive"), 0);
    // The keys: Escape closes it as an empty panel closes; from Settings, the drives' icon has
    // the focus and says "Checking…" under it; Enter opens the panel again.
    host.send_key(Key::Escape);
    host.settle();
    host.send_key(Key::Backtab);
    host.send_key(Key::Backtab);
    host.settle();
    assert_eq!(host.count("status-checking"), 1, "the drives' icon has the keys' focus");
    host.send_key(Key::Return);
    host.settle();
    assert_eq!(host.value("system-panel"), "drives", "Enter opens it");
    assert_eq!(host.values("loading"), ["Reading drives…"]);

    // The reads end: what the clicks and keys asked for runs, as the moving clock lets it.
    host.pump();
    host.advance(std::time::Duration::from_secs(31));
    host.settle();
    let vm = host.core().view().system;
    let read = |area: &str| vm.area(area).state != "loading";
    assert!(read("network") && read("drives") && !vm.drives_listing && vm.area("drives").ready(), "{vm:?}");
    assert_eq!(host.count("loading"), 0, "the panel fills in");
    assert_eq!(host.values("drive").len(), 0, "no drive here");
    assert_eq!(host.value("status:network"), "wired");
    assert_eq!(host.value("battery"), "18");
    assert_eq!(host.count("loading-line"), 0, "nothing loads");
    assert_eq!(host.first("Settings").absolute_position().x, settings, "the bar didn't move");
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
    assert_ne!(host.value("status:drives"), "loading");
    assert_ne!(host.value("status:sound"), "loading");
}

/// Drives that turn out not to be managed here: the drives' icon loads, then stays in its place,
/// disabled, and the bar doesn't move. The mouse reaches it: a click opens nothing, and the
/// pointer over it shows why.
#[test]
fn the_drives_icon_shows_disabled_when_drives_turn_out_unavailable() {
    for theme in ["classic", "default"] {
        let system = machine();
        system.with(|m| m.unavailable.push(("drives", "No udisks here".into())));
        let mut host = reading(&system, WHOLE_SCREEN, theme);
        assert_eq!(host.value("status:drives"), "loading", "{theme}");
        let settings = host.first("Settings").absolute_position().x;
        host.advance(std::time::Duration::from_secs(31));
        host.settle();
        assert_eq!(host.count("status:drives"), 1, "{theme}: it stays");
        assert_eq!(host.value("status:drives"), "disabled", "{theme}");
        assert_eq!(host.description("status:drives"), "No udisks here", "{theme}");
        assert_eq!(host.first("Settings").absolute_position().x, settings, "{theme}: the bar didn't move");
        assert_eq!(host.count("status-reason"), 0, "{theme}: no reason before hover or focus");

        // The pointer reaches the bar once the boot screen is gone.
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.settle();
        host.click("status:drives");
        assert_eq!(host.count("system-panel"), 0, "{theme}: a click opens nothing");
        assert_eq!(host.values("status-reason"), ["No udisks here"], "{theme}: the hover shows why");
        host.first("status:drives").invoke_accessible_default_action();
        host.settle();
        assert_eq!(host.count("system-panel"), 0, "{theme}: nor does its default action");
    }
}

/// A disabled icon's reason is on one line, on a chip that stays inside the window: a long reason
/// under the drives' icon, near the window's edge, moves in, and is as tall as a short one.
#[test]
fn a_disabled_icon_s_reason_is_one_line_inside_the_window() {
    use slint::platform::Key;
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
        assert_eq!([host.value("status:bluetooth"), host.value("status:drives")], ["disabled", "disabled"], "{theme}");
        let window = host.shown().window();
        let right = window.size().width as f32 / window.scale_factor();
        let chip = |host: &Composed| {
            let chip = host.first("status-reason");
            (chip.absolute_position().x, chip.size().width, chip.size().height)
        };

        // From Settings, back to the drives' icon, then past it to Bluetooth's.
        host.send_key(Key::Backtab);
        host.send_key(Key::Backtab);
        host.settle();
        assert_eq!(host.values("status-reason"), [LONG], "{theme}");
        let (x, width, long_height) = chip(&host);
        assert!(x >= 0.0 && x + width <= right, "{theme}: the chip is inside the window: {x} + {width} > {right}");
        host.send_key(Key::Backtab);
        host.settle();
        assert_eq!(host.values("status-reason"), ["No adapter"], "{theme}");
        let (_, short_width, short_height) = chip(&host);
        assert!(short_width < width, "{theme}: the chip is as wide as its reason");
        assert_eq!(long_height, short_height, "{theme}: the long reason is on one line");
    }
}

/// An unsupported area's icon takes the keys' and the pad's focus, says why, and its ✕ and Enter
/// open nothing. Left and right from the panel beside it go past it.
#[test]
fn a_disabled_icon_takes_the_focus_and_says_why_but_opens_nothing() {
    use slint::platform::Key;
    for theme in ["classic", "default"] {
        let system = machine();
        system.with(|m| m.unavailable.push(("bluetooth", "This PC has no Bluetooth adapter".into())));
        let mut host = shown(&system, WHOLE_SCREEN);
        host.ui.switch(&id(theme)).unwrap();
        host.settle();
        assert_eq!(host.value("status:bluetooth"), "disabled", "{theme}");
        // From Settings, back past the drives' icon to Bluetooth's.
        host.send_key(Key::Backtab);
        host.send_key(Key::Backtab);
        host.send_key(Key::Backtab);
        host.settle();
        assert_eq!(host.values("status-reason"), ["This PC has no Bluetooth adapter"], "{theme}: the focus shows why");
        host.pad(launcher_ui_host::PadButton::South);
        assert_eq!(host.count("system-panel"), 0, "{theme}: ✕ opens nothing");
        host.navigate(Key::Return);
        assert_eq!(host.count("system-panel"), 0, "{theme}: nor does Enter");

        // The sound's panel, off its volume row: right goes past Bluetooth to the drives.
        host.press_first("status:sound");
        assert_eq!(host.value("system-panel"), "sound", "{theme}");
        host.navigate(Key::DownArrow);
        host.navigate(Key::RightArrow);
        assert_eq!(host.value("system-panel"), "drives", "{theme}: past the disabled one");
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
        assert_eq!(lines.len(), 5, "{theme}: the four icons and the battery: {lines:?}");
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
        assert_eq!(host.value("status:drives"), "loading", "{theme}");
        host.send_key(Key::Backtab);
        host.send_key(Key::Backtab);
        host.settle();
        host.pad(launcher_ui_host::PadButton::South);
        assert_eq!(host.value("system-panel"), "drives", "{theme}: ✕ opens it");
    }
}

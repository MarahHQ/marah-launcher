//! Seam 3: the Classic theme's machine settings, over a fake system plugin. The top bar's
//! network, sound and Bluetooth icons and their panels show when Marah Launcher is the whole screen;
//! the battery wherever the PC has one; Settings → System; and on Marah OS's USB stick, the
//! Welcome and Install screens.

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

#[test]
fn on_a_desktop_only_the_battery_shows_in_the_top_bar() {
    let host = shown(&machine(), Place::default());
    assert_eq!(host.count("status:network") + host.count("status:sound") + host.count("status:bluetooth"), 0);
    assert_eq!(host.value("battery"), "18");
    assert_eq!(host.count("system-internet"), 0, "no SYSTEM section in Settings");
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

    let system = machine();
    system.with(|m| m.unavailable = vec![("bluetooth", "This PC has no Bluetooth adapter".into())]);
    let mut host = shown(&system, WHOLE_SCREEN);
    host.press_first("status:bluetooth");
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
fn on_a_desktop_the_power_menu_only_quits_and_escape_closes_it() {
    let system = machine();
    let mut host = shown(&system, Place::default());
    ctrl_p(&mut host);
    assert_eq!(power_items(&host), ["Quit Marah Launcher"]);
    host.navigate(slint::platform::Key::Escape);
    assert!(power_items(&host).is_empty(), "closed");
    ctrl_p(&mut host);
    power_item(&mut host, "Quit Marah Launcher");
    assert!(host.requests.borrow().contains(&launcher_ui_host::UiRequest::Quit));
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

/// On a desktop the drives' icon is the only machine icon: the keys reach it, and its panel opens.
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
        assert_eq!(host.count("status:network") + host.count("status:bluetooth"), 0, "{theme}");
        host.navigate(Key::Backtab);
        host.navigate(Key::Backtab);
        host.navigate(Key::Return);
        host.settle();
        assert_eq!(host.value("system-panel"), "drives", "{theme}: the keys reach it");
        host.navigate(Key::LeftArrow);
        assert_eq!(host.value("system-panel"), "drives", "{theme}: no other panel beside it");
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

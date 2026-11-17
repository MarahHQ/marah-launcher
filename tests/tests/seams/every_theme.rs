//! Seam 3: one suite run against every built-in theme. Each theme stands alone and draws Settings,
//! the Game Hub, Downloads and the on-screen keyboard with the kit, so these work the same way
//! in each.

use crate::common::*;
use launcher_core::{Command, Route};
use launcher_ui_api::ScreenId;
use launcher_ui_host::PadButton;
use slint::platform::Key;
use slint_interpreter::ComponentHandle;

/// Two emulators' games.
fn two_emulators(h: launcher_testkit::FakeHost) -> launcher_testkit::FakeHost {
    h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
        .plugin(FakeCatalog::new("emu-b").system("PS5").games(["Gamma"]).build())
}

/// The built-in themes.
const THEMES: [&str; 2] = [theme_classic::ID, theme_default::ID];

/// Marah Launcher in `theme` with the plugins `build` adds, at 1600×900.
fn in_theme(theme: &str, build: impl FnOnce(launcher_testkit::FakeHost) -> launcher_testkit::FakeHost) -> Composed {
    let mut host = composed_first_run(build);
    host.ui.start_with(Some(id(theme))).unwrap();
    assert_eq!(host.ui.current(), Some(&id(theme)));
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.settle();
    host
}

#[test]
fn every_screen_renders_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| {
            two_emulators(h).built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        for (route, screen) in [
            (Route::Catalog, ScreenId::Catalog),
            (Route::Downloads, ScreenId::Downloads),
            (Route::Settings, ScreenId::Settings),
            (Route::Emulators, ScreenId::Emulators),
            (Route::Themes, ScreenId::Themes),
            (Route::Plugin(id("emu-a")), ScreenId::Plugin),
            (Route::Library, ScreenId::Library),
            (Route::Home, ScreenId::Home),
        ] {
            host.dispatch(Command::Navigate(route));
            host.settle();
            assert_eq!(host.core().view().screen, screen, "{theme}");
        }
        host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
        host.settle();
        assert_eq!(host.core().view().screen, ScreenId::GameDetail, "{theme}");
    }
}

#[test]
fn the_keys_move_through_the_settings_page_and_act_on_a_row_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, two_emulators);
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        // The Games tab, down to its first row: Enter on "Rescan game folders" scans again.
        host.settings_tab("Games");
        // Right, from the rail into the tab's rows.
        host.navigate(Key::RightArrow);
        // A press alone: `navigate` would carry out the request before the check.
        let text: slint::SharedString = Key::Return.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
        let asked = host.requests.borrow().iter().any(|r| {
            matches!(r, launcher_ui_host::UiRequest::Core(Command::Library(launcher_core::LibraryCmd::Refresh)))
        });
        assert!(asked, "{theme}: Enter on Rescan asks the core to scan");
        host.pump();
        // One page: R1 jumps on to the next section, Downloads: its first row, where Enter turns
        // "Download only through a VPN" on.
        host.navigate(Key::Tab);
        host.navigate(Key::Return);
        assert_eq!(host.first("vpn-only").accessible_value().as_deref(), Some("on"), "{theme}: R1 reached Downloads");
    }
}

/// A plugin's page is one page like Settings in every theme: the rail lists its sections, R1
/// jumps to its settings, and Up from their first row crosses back into General.
#[test]
fn a_plugin_s_page_is_one_page_with_a_rail_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
        host.dispatch(Command::Navigate(Route::Emulators));
        host.dispatch(Command::Navigate(Route::Plugin(id("emu-kyty"))));
        host.settle();
        let lit = |host: &Composed| {
            ["General", "Settings"]
                .into_iter()
                .find(|name| {
                    i_slint_backend_testing::ElementHandle::find_by_accessible_label(
                        host.instance(),
                        &format!("settings-tab:{name}"),
                    )
                    .any(|e| e.accessible_checked() == Some(true))
                })
                .map(str::to_owned)
        };
        assert_eq!(lit(&host).as_deref(), Some("General"), "{theme}: the rail lights General");
        host.navigate(Key::Tab);
        assert_eq!(lit(&host).as_deref(), Some("Settings"), "{theme}: R1 jumps to Settings");
        host.navigate(Key::Return);
        assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("true"), "{theme}");
        host.navigate(Key::UpArrow);
        assert_eq!(lit(&host).as_deref(), Some("General"), "{theme}: Up crosses into General");
    }
}

/// Settings → Launcher's Animations row: a press turns the theme's motion off, and the row
/// says so; a second turns it on again.
#[test]
fn the_animations_row_turns_motion_off_and_on_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, two_emulators);
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Launcher");
        assert_eq!(host.first("animations").accessible_value().as_deref(), Some("on"), "{theme}");
        host.first("animations").invoke_accessible_default_action();
        host.pump();
        host.settle();
        assert!(!host.core().view().settings.animations, "{theme}: saved off");
        assert_eq!(host.first("animations").accessible_value().as_deref(), Some("off"), "{theme}");
        host.first("animations").invoke_accessible_default_action();
        host.pump();
        host.settle();
        assert!(host.core().view().settings.animations, "{theme}: on again");
    }
}

#[test]
fn the_backup_tab_backs_up_and_lists_the_file_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = in_theme(theme, two_emulators);
        let folder = serde_json::json!(backups.path().display().to_string());
        host.dispatch(Command::Pref(launcher_core::Pref::BackupFolder, folder));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Backup");
        host.pump();
        host.settle();
        assert_eq!(host.count("backup"), 0, "{theme}");
        host.press_first("back-up");
        host.settle();
        assert_eq!(host.count("backup"), 1, "{theme}: the new backup is listed");
        host.press_first("backup-browse");
        assert_eq!(host.count("backup-browser"), 1, "{theme}: Restore from another folder opens the browser");
        assert_eq!(host.value("backup-browser"), backups.path().display().to_string(), "{theme}: at the backup folder");
    }
}

#[test]
fn a_settings_text_box_opens_the_on_screen_keyboard_and_circle_closes_it_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Games");
        host.navigate(Key::RightArrow);
        assert_eq!(host.count("folder:games"), 1, "{theme}");
        host.pad(PadButton::South);
        assert_eq!(host.count("osk"), 1, "{theme}: the text box opened the keyboard");
        host.pad(PadButton::East);
        assert_eq!(host.count("osk"), 0, "{theme}: Circle closed it");
    }
}

/// Marah Launcher in `theme`, with `backups` as the backup folder, a backup in `backups/old-pc`, and the
/// browser open at `backups`.
fn browsing(theme: &str, backups: &std::path::Path) -> Composed {
    use launcher_core::BackupCmd;
    let mut host = in_theme(theme, two_emulators);
    let old_pc = backups.join("old-pc");
    let folder = serde_json::json!(old_pc.display().to_string());
    host.dispatch(Command::Pref(launcher_core::Pref::BackupFolder, folder));
    host.dispatch(Command::Backup(BackupCmd::BackUp { large: false }));
    std::fs::create_dir_all(backups.join("photos")).unwrap();
    let folder = serde_json::json!(backups.display().to_string());
    host.dispatch(Command::Pref(launcher_core::Pref::BackupFolder, folder));
    host.dispatch(Command::Backup(BackupCmd::Browse(Some(backups.into()))));
    host.settle();
    host
}

/// A key as the window gets it, then carried out: "/" is △ on a pad.
fn text(host: &mut Composed, text: &str) {
    host.type_text(text);
}

#[test]
fn the_backup_browser_s_keys_move_open_and_go_up_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        assert_eq!(host.count("backup-browser"), 1, "{theme}: over everything");
        assert_eq!(host.values("browser-folder"), ["old-pc", "photos"], "{theme}: the folder with a backup first");
        assert_eq!(host.focused("browser-folder"), ["old-pc"], "{theme}");
        assert_eq!(host.description("browser-folder"), "1 backup", "{theme}");

        host.navigate(Key::DownArrow);
        assert_eq!(host.focused("browser-folder"), ["photos"], "{theme}");
        host.navigate(Key::UpArrow);
        host.navigate(Key::Return);
        assert!(host.value("backup-browser").ends_with("old-pc"), "{theme}: Enter opens the folder");
        assert_eq!(host.count("browser-backup"), 1, "{theme}");
        assert_eq!(host.values("browser-crumb").last().map(String::as_str), Some("old-pc"), "{theme}");

        host.navigate(Key::Escape);
        assert_eq!(host.value("backup-browser"), backups.path().display().to_string(), "{theme}: ○ goes up a folder");

        // The places: Left goes there, Enter goes to one, ○ there closes.
        host.navigate(Key::LeftArrow);
        assert_eq!(host.focused("browser-place"), ["Backup folder"], "{theme}: at the place the folder is");
        assert!(host.focused("browser-folder").is_empty(), "{theme}");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("browser-folder").len(), 1, "{theme}: back in the files");
        host.navigate(Key::LeftArrow);
        host.navigate(Key::Escape);
        assert_eq!(host.count("backup-browser"), 0, "{theme}: closed");
        assert!(!host.core().view().settings.backup.browser.open, "{theme}");
    }
}

#[test]
fn typing_filters_the_backup_browser_and_escape_clears_it_first_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        text(&mut host, "PHO");
        assert_eq!(host.value("browser-filter"), "PHO", "{theme}: every key typed before the core answered");
        assert_eq!(host.values("browser-folder"), ["photos"], "{theme}: in any case");
        host.navigate(Key::Backspace);
        assert_eq!(host.value("browser-filter"), "PH", "{theme}");
        text(&mut host, "zz");
        assert_eq!(host.count("browser-folder"), 0, "{theme}");
        assert_eq!(host.count("browser-empty"), 1, "{theme}: it says nothing holds it");
        host.navigate(Key::Escape);
        assert_eq!(host.count("browser-filter"), 0, "{theme}: ○ clears the text first");
        assert_eq!(host.count("browser-folder"), 2, "{theme}");
        assert_eq!(host.count("backup-browser"), 1, "{theme}: and stays open");
    }
}

#[test]
fn a_restore_from_the_backup_browser_asks_once_then_restores_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        host.navigate(Key::Return);
        assert_eq!(host.focused("browser-backup").len(), 1, "{theme}");

        // △ asks; ○ says no, and nothing is restored.
        text(&mut host, "/");
        assert_eq!(host.count("browser-confirm"), 1, "{theme}: △ asks first");
        host.navigate(Key::Escape);
        assert_eq!(host.count("browser-confirm"), 0, "{theme}");
        assert_eq!(host.count("backup-browser"), 1, "{theme}: the browser stays");

        // Enter asks too; Right goes to Cancel, Enter on it says no.
        host.navigate(Key::Return);
        assert_eq!(host.focused("browser-confirm-restore").len(), 1, "{theme}: Restore is first");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("browser-confirm-cancel").len(), 1, "{theme}");
        host.navigate(Key::Return);
        assert_eq!(host.count("browser-confirm"), 0, "{theme}");
        assert!(!host.core().view().settings.backup.restart, "{theme}: nothing restored");

        // Restore: the backup goes back, and the browser closes.
        host.navigate(Key::Return);
        host.navigate(Key::Return);
        host.until(launcher_core::Area::Backup, |core| core.view().settings.backup.restart);
        host.settle();
        assert_eq!(host.count("backup-browser"), 0, "{theme}: a restore closes the browser");
    }
}

#[test]
fn one_click_opens_and_a_double_click_opens_only_one_folder_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        // One click on a folder opens it.
        host.click("browser-folder");
        assert!(host.value("backup-browser").ends_with("old-pc"), "{theme}: one click opens");
        host.wait(std::time::Duration::from_millis(500));
        // One click on a backup asks to restore it.
        host.click("browser-backup");
        assert_eq!(host.count("browser-confirm"), 1, "{theme}: one click asks to restore");
        host.navigate(Key::Escape);
        host.wait(std::time::Duration::from_millis(500));
        // A double click on old-pc opens it, and its second click doesn't ask to restore the
        // backup now under the pointer.
        host.navigate(Key::Escape);
        host.wait(std::time::Duration::from_millis(500));
        let target = host.first("browser-folder");
        let (at, size) = (target.absolute_position(), target.size());
        let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
        let button = slint::platform::PointerEventButton::Left;
        for _ in 0..2 {
            let window = host.instance().window();
            window.dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
            window.dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
            host.pump();
        }
        host.settle();
        assert!(host.value("backup-browser").ends_with("old-pc"), "{theme}");
        assert_eq!(host.count("browser-confirm"), 0, "{theme}: the second click waited");
        host.wait(std::time::Duration::from_millis(500));
        host.press_first("browser-close");
        assert_eq!(host.count("backup-browser"), 0, "{theme}");
    }
}

#[test]
fn a_held_enter_on_a_backup_asks_and_does_not_restore_in_every_theme() {
    use slint::platform::WindowEvent;
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        host.navigate(Key::Return);
        assert_eq!(host.focused("browser-backup").len(), 1, "{theme}");
        let enter: slint::SharedString = Key::Return.into();
        let window = host.instance().window();
        window.dispatch_event(WindowEvent::KeyPressed { text: enter.clone() });
        for _ in 0..3 {
            window.dispatch_event(WindowEvent::KeyPressRepeated { text: enter.clone() });
        }
        window.dispatch_event(WindowEvent::KeyReleased { text: enter });
        host.pump();
        host.settle();
        assert_eq!(host.count("browser-confirm"), 1, "{theme}: the dialog still asks");
        assert!(!host.core().view().settings.backup.restart, "{theme}: nothing restored");
    }
}

/// While Log out, Restart or Shut down closes what runs, the screen is covered with the step it
/// is at, in every theme.
#[test]
fn while_log_out_closes_what_runs_the_screen_says_its_step_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h);
        assert_eq!(host.count("closing"), 0, "{theme}: nothing closes");
        host.dispatch(Command::System(launcher_core::system::SystemCommand::LogOut));
        host.settle();
        assert_eq!(host.value("closing"), "Logging out", "{theme}");
    }
}

/// While Log out closes a running game, the closing screen shows that game, as its launch did:
/// its title over its art. It takes every key, so no key hints show over it. A game that won't
/// end keeps the step on screen.
#[test]
fn closing_a_game_shows_the_game_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build()));
        host.test.windows.no_windows();
        host.dispatch(Command::Session(launcher_core::SessionCmd::Launch("emu-a:G0".parse().unwrap())));
        let title = host.core().view().running.expect("a running game").title;
        host.test.spawner.ignore_terminate();
        let core = host.core().clone();
        host.rt
            .spawn(async move { core.dispatch(Command::System(launcher_core::system::SystemCommand::LogOut)).await });
        host.until(launcher_core::Area::Sessions, |core| core.view().closing_game.is_some());
        host.settle();

        assert_eq!(host.value("closing-title"), title, "{theme}: the game's title");
        assert_eq!(host.value("closing-step"), "Closing…", "{theme}");
        assert!(host.values("hint").is_empty(), "{theme}: no key hints, the keys do nothing");
    }
}

/// A closing step about no game (a launcher left running, then Log out itself) shows the step as
/// the title, in every theme.
#[test]
fn a_step_about_no_game_shows_the_step_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h);
        host.dispatch(Command::System(launcher_core::system::SystemCommand::LogOut));
        host.settle();
        assert_eq!(host.value("closing-title"), "Logging out…", "{theme}");
        assert_eq!(host.count("closing-step"), 0, "{theme}: the title says it all");
    }
}

/// A launcher update found: a dot by Settings' Updates icon, in every theme; none before.
#[test]
fn a_launcher_update_puts_a_dot_by_updates_in_every_theme() {
    use launcher_core::launcher_update::{Install, package_key};
    const LATEST: &str = "https://github.com/owner/marah-launcher/releases/latest/download/latest.json";
    for theme in THEMES {
        let mut host = in_theme(theme, two_emulators);
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        assert_eq!(host.count("update-dot"), 0, "{theme}: no update, no dot");

        let url = "https://github.com/owner/marah-launcher/releases/download/v9.9.9/marah-launcher.tar.gz";
        let sha = host.test.http.serve(url, b"package".to_vec());
        let mut assets = serde_json::Map::new();
        assets.insert(package_key(Install::Folder).unwrap(), serde_json::json!({ "url": url, "sha256": sha }));
        let latest = serde_json::json!({ "version": "9.9.9", "assets": assets }).to_string();
        host.test.http.serve(LATEST, latest.into_bytes());
        host.core().kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();
        host.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check));
        host.settle();
        assert_eq!(host.core().view().settings.launcher_update.state, "available", "{theme}");
        assert_eq!(host.values("update-dot"), ["Updates"], "{theme}: the dot is by Updates");
    }
}

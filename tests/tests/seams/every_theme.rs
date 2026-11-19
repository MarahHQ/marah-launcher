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

/// A Catalog card's Download (`Games.act`) opens the game's Game Hub with the keys on its
/// first release, where the player picks what to download.
#[test]
fn a_card_s_download_opens_the_game_hub_on_its_first_release() {
    for theme in THEMES {
        let home = tempfile::tempdir().unwrap();
        let mut host = in_theme(theme, |h| {
            h.home(home.path())
                .plugin(FakeCatalog::kyty().playable().build())
                .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        let topics = serde_json::json!([{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a",
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }]);
        host.serve_catalog(&[("ps5", topics)]);
        host.dispatch(Command::Navigate(Route::Catalog));
        host.settle();
        let download = host.values("card-get");
        assert_eq!(download.len(), 1, "{theme}: {download:?}");

        host.press("card-get", &download[0]);
        assert_eq!(host.core().view().screen, ScreenId::GameDetail, "{theme}: its Game Hub");
        let release = format!("release-{}", host.core().view().detail.expect("its details").releases[0].id);
        assert_eq!(host.count(&release), 1, "{theme}: its first release shows");
        assert!(host.lit(&release), "{theme}: the keys are on its first release");
    }
}

/// On a Game Hub with versions and releases, a button in the row is lit only while the keys are
/// on the row: Down to a version or to a release leaves Options dark, and Up lights it again.
#[test]
fn the_game_hub_lights_options_only_while_the_keys_are_on_its_row_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| {
            h.plugin(FakeCatalog::kyty().playable().build())
                .plugin(FakeCatalog::new("emu-ps4").system("PS4").console("PS4").games([]).build())
                .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        let topic = |console: &str, id: u32, title_id: &str| {
            serde_json::json!({ "id": id.to_string(), "title": format!("[{console}] Astro Drift [EUR / v1.00]"),
                "size": "20 GB", "magnet": format!("magnet:?xt=urn:btih:{id:040x}"),
                "game_info": { "name": "Astro Drift", "title_id": title_id, "version": "1.00", "region": "EUR",
                    "release_year": "2024", "publisher": "Sony" } })
        };
        host.serve_catalog(&[
            ("ps5", serde_json::json!([topic("PS5", 1, "PPSA01234")])),
            ("ps4", serde_json::json!([topic("PS4", 2, "CUSA01234")])),
        ]);
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.settle();
        host.dispatch(Command::OpenGame("release-catalog:emu-ps4/CUSA01234".parse().unwrap()));
        host.settle();
        let detail = host.core().view().detail.expect("its details");
        assert_eq!(detail.versions.len(), 2, "{theme}: a version on each console");
        let release = format!("release-{}", detail.releases[0].id);
        assert!(host.lit("game-options"), "{theme}: the keys start on the row, on Options");

        host.navigate(Key::DownArrow);
        assert!(!host.lit("game-options"), "{theme}: the keys are on a version, not on Options");
        host.navigate(Key::DownArrow);
        assert!(host.lit(&release), "{theme}: the keys are on the first release");
        assert!(!host.lit("game-options"), "{theme}: and not on Options");

        host.navigate(Key::UpArrow);
        host.navigate(Key::UpArrow);
        assert!(host.lit("game-options"), "{theme}: back on the row, on Options");
    }
}

/// The installed game's Game Hub, with versions and releases, starts on Play, and the hints say
/// Play. Down on a version or on a release, Enter picks it: the hints say Select there, and
/// Play again back on the row.
#[test]
fn the_hints_say_play_only_while_the_keys_are_on_the_game_hub_s_row_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| {
            h.plugin(FakeCatalog::kyty().build())
                .plugin(
                    FakeCatalog::new("emu-ps4").system("PS4").console("PS4").games(["Astro Drift"]).playable().build(),
                )
                .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        let topic = |console: &str, id: u32, title_id: &str| {
            serde_json::json!({ "id": id.to_string(), "title": format!("[{console}] Astro Drift [EUR / v1.00]"),
                "size": "20 GB", "magnet": format!("magnet:?xt=urn:btih:{id:040x}"),
                "game_info": { "name": "Astro Drift", "title_id": title_id, "version": "1.00", "region": "EUR",
                    "release_year": "2024", "publisher": "Sony" } })
        };
        host.serve_catalog(&[
            ("ps5", serde_json::json!([topic("PS5", 1, "PPSA01234")])),
            ("ps4", serde_json::json!([topic("PS4", 2, "CUSA00000")])),
        ]);
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.settle();
        host.dispatch(Command::OpenGame("release-catalog:emu-ps4/CUSA00000".parse().unwrap()));
        host.settle();
        let detail = host.core().view().detail.expect("its details");
        assert!(detail.can.play, "{theme}: the installed game plays");
        assert_eq!(detail.versions.len(), 2, "{theme}: a version on each console");
        let release = format!("release-{}", detail.releases[0].id);
        let says_play = |host: &Composed| host.values("hint").iter().any(|h| h == "Play");
        assert!(says_play(&host), "{theme}: the keys start on Play: {:?}", host.values("hint"));

        host.navigate(Key::DownArrow);
        assert!(!says_play(&host), "{theme}: on a version, Enter picks it: {:?}", host.values("hint"));
        host.navigate(Key::DownArrow);
        assert!(host.lit(&release), "{theme}: the keys are on the first release");
        assert!(!says_play(&host), "{theme}: on a release, Enter picks it: {:?}", host.values("hint"));

        host.navigate(Key::UpArrow);
        host.navigate(Key::UpArrow);
        assert!(says_play(&host), "{theme}: back on the row, on Play: {:?}", host.values("hint"));
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

/// Settings opens on the first row of its first tab. The keys go down a row, to another screen,
/// and back: Settings opens on that first row again, not on the row the keys left.
#[test]
fn settings_opens_on_its_first_row_after_another_screen_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, two_emulators);
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        // The Launcher tab, the first: Full screen.
        let first = "fullscreen".to_owned();
        assert!(host.lit(&first), "{theme}: Settings opens on {first}");

        host.navigate(Key::DownArrow);
        assert!(!host.lit(&first), "{theme}: the keys left {first}");
        host.dispatch(Command::Navigate(Route::Home));
        host.settle();
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        assert!(host.lit(&first), "{theme}: back on {first}");
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
        assert_eq!(host.count("folder-browser"), 1, "{theme}: Restore from another folder opens the browser");
        assert_eq!(host.value("folder-browser"), backups.path().display().to_string(), "{theme}: at the backup folder");
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
        assert_eq!(host.count("folder-list"), 1, "{theme}: the folder's list opened");
        for _ in 0..host.values("folder-path").len() + 1 {
            host.pad(PadButton::Down);
        }
        host.pad(PadButton::South);
        assert_eq!(host.count("osk"), 1, "{theme}: Type a path opened the keyboard");
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
        assert_eq!(host.count("folder-browser"), 1, "{theme}: over everything");
        assert_eq!(host.values("browser-folder"), ["old-pc", "photos"], "{theme}: the folder with a backup first");
        assert_eq!(host.focused("browser-folder"), ["old-pc"], "{theme}");
        assert_eq!(host.description("browser-folder"), "1 backup", "{theme}");

        host.navigate(Key::DownArrow);
        assert_eq!(host.focused("browser-folder"), ["photos"], "{theme}");
        host.navigate(Key::UpArrow);
        host.navigate(Key::Return);
        assert!(host.value("folder-browser").ends_with("old-pc"), "{theme}: Enter opens the folder");
        assert_eq!(host.count("browser-backup"), 1, "{theme}");
        assert_eq!(host.values("browser-crumb").last().map(String::as_str), Some("old-pc"), "{theme}");

        host.navigate(Key::Escape);
        assert_eq!(host.value("folder-browser"), backups.path().display().to_string(), "{theme}: ○ goes up a folder");

        // The places: Left goes there, Enter goes to one, ○ there closes.
        host.navigate(Key::LeftArrow);
        assert_eq!(host.focused("browser-place"), ["Backup folder"], "{theme}: at the place the folder is");
        assert!(host.focused("browser-folder").is_empty(), "{theme}");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("browser-folder").len(), 1, "{theme}: back in the files");
        host.navigate(Key::LeftArrow);
        host.navigate(Key::Escape);
        assert_eq!(host.count("folder-browser"), 0, "{theme}: closed");
        assert!(!host.core().view().browser.open, "{theme}");
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
        assert_eq!(host.count("folder-browser"), 1, "{theme}: and stays open");
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
        assert_eq!(host.count("folder-browser"), 1, "{theme}: the browser stays");

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
        assert_eq!(host.count("folder-browser"), 0, "{theme}: a restore closes the browser");
    }
}

#[test]
fn one_click_opens_and_a_double_click_opens_only_one_folder_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        // One click on a folder opens it.
        host.click("browser-folder");
        assert!(host.value("folder-browser").ends_with("old-pc"), "{theme}: one click opens");
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
        assert!(host.value("folder-browser").ends_with("old-pc"), "{theme}");
        assert_eq!(host.count("browser-confirm"), 0, "{theme}: the second click waited");
        host.wait(std::time::Duration::from_millis(500));
        host.press_first("browser-close");
        assert_eq!(host.count("folder-browser"), 0, "{theme}");
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

/// A plugin that asks a question at the start, and keeps the answers.
#[derive(Default)]
struct Asker {
    answers: std::sync::Mutex<Vec<bool>>,
}

#[launcher_api::async_trait]
impl launcher_api::StartupWork for Asker {
    async fn status(&self) -> Option<launcher_api::StartupStep> {
        None
    }

    async fn question(&self) -> Option<launcher_api::StartupQuestion> {
        self.answers.lock().unwrap().is_empty().then(|| launcher_api::StartupQuestion {
            title: "Install the thing?".into(),
            text: "It downloads the thing.".into(),
            yes: "Install".into(),
            no: "Not now".into(),
        })
    }

    async fn answer(&self, yes: bool) {
        self.answers.lock().unwrap().push(yes);
    }
}

/// The asker, as a plugin with the startup capability.
fn asker(work: std::sync::Arc<Asker>) -> launcher_testkit::StubPlugin {
    launcher_testkit::StubPlugin::from_toml(
        "id = \"asker\"\nkind = \"service\"\nname = \"Asker\"\nversion = \"1.0.0\"\nhost_api = \"^0.3.2\"\n\
         capabilities = [\"startup\"]",
    )
    .provides(move |_| Ok(launcher_api::Provided { startup: Some(work.clone()), ..Default::default() }))
}

/// A plugin's question shows over the boot screen, in every theme; the boot waits for it, and
/// Enter answers its yes, Escape its no.
#[test]
fn a_plugin_s_question_shows_over_the_boot_screen_and_the_keys_answer_it() {
    for (theme, key, answer) in [(THEMES[0], Key::Return, true), (THEMES[1], Key::Escape, false)] {
        let work = std::sync::Arc::new(Asker::default());
        let mut host = in_theme(theme, |h| two_emulators(h).plugin(asker(work.clone())));
        assert_eq!(host.values("question"), ["Install the thing?"], "{theme}");
        assert_eq!(host.values("question-text"), ["It downloads the thing."], "{theme}");
        assert!(!host.core().view().boot.ready, "{theme}: the boot waits for the answer");

        host.navigate(key);
        host.settle();
        assert_eq!(*work.answers.lock().unwrap(), [answer], "{theme}");
        assert!(host.values("question").is_empty(), "{theme}: the question is gone");
        assert!(host.core().view().question.is_none(), "{theme}: asked once a run");
    }
}

/// Settings → Game folders: the folder's row opens its list, "Add folder" opens the folder browser
/// to pick one, and △ uses the folder shown: it joins the list.
#[test]
fn add_folder_picks_a_folder_in_the_browser_in_every_theme() {
    for theme in THEMES {
        let picked = tempfile::tempdir().unwrap();
        let mut host = in_theme(theme, |h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Games");
        host.navigate(Key::RightArrow);
        host.pad(PadButton::South);
        assert_eq!(host.count("folder-list"), 1, "{theme}");
        for _ in 0..host.values("folder-path").len() {
            host.pad(PadButton::Down);
        }
        assert_eq!(host.focused("folder-add").len(), 1, "{theme}: Add folder");
        host.pad(PadButton::South);
        host.settle();
        assert_eq!(host.count("folder-list"), 0, "{theme}: the list closed");
        assert_eq!(host.count("folder-browser"), 1, "{theme}: the browser opened");
        assert_eq!(host.count("browser-use"), 1, "{theme}: Use this folder shows when it picks");

        host.dispatch(Command::Browse(launcher_core::BrowseCmd::Go(picked.path().into())));
        host.settle();
        text(&mut host, "/");
        host.settle();
        assert_eq!(host.count("folder-browser"), 0, "{theme}: △ used the folder");
        let games = host.core().kernel().config().get(&["app", "folders", "games"]);
        assert!(games.as_array().unwrap().iter().any(|p| p == picked.path().to_str().unwrap()), "{theme}: {games}");
    }
}

/// Settings → Backup → Backup folder: its row opens the folder's list, as a game folder's does.
/// "Choose a folder" opens the folder browser, △ uses the folder shown; "Use the default folder"
/// goes back to it; "Type a path" opens the row's text box. With the pad, the mouse and the keys.
#[test]
fn the_backup_folder_is_chosen_like_a_game_folder_in_every_theme() {
    for theme in THEMES {
        let (backups, chosen) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut host = in_theme(theme, two_emulators);
        let folder = serde_json::json!(backups.path().display().to_string());
        host.dispatch(Command::Pref(launcher_core::Pref::BackupFolder, folder));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Backup");
        host.settle();
        // The mouse: a click on the row opens the list.
        host.press_first("backup-folder");
        assert_eq!(host.value("folder-list"), "backup", "{theme}: the backup folder's list");
        assert_eq!(host.count("folder-path") + host.count("folder-add"), 0, "{theme}: one folder, not a list");
        // The pad: ✕ on "Choose a folder" opens the browser at the backup folder.
        host.pad(PadButton::South);
        assert_eq!(host.count("folder-browser"), 1, "{theme}: Choose a folder opens the browser");
        assert_eq!(host.value("folder-browser"), backups.path().display().to_string(), "{theme}");
        host.dispatch(Command::Browse(launcher_core::BrowseCmd::Go(chosen.path().into())));
        host.settle();
        // The keys: / (△) uses the folder shown.
        text(&mut host, "/");
        host.settle();
        assert_eq!(host.count("folder-browser"), 0, "{theme}: △ used the folder");
        let saved = host.core().kernel().config().get(&["app", "backup_folder"]);
        assert_eq!(saved, serde_json::json!(chosen.path().display().to_string()), "{theme}");

        // The list again: down twice to "Use the default folder", and ✕.
        host.press_first("backup-folder");
        host.pad(PadButton::Down);
        host.pad(PadButton::Down);
        assert_eq!(host.focused("backup-default").len(), 1, "{theme}");
        host.pad(PadButton::South);
        host.settle();
        assert!(host.core().kernel().config().get(&["app", "backup_folder"]).is_null(), "{theme}: the default again");

        // "Type a path": the row's text box, with the folder in it.
        host.press_first("backup-folder");
        host.navigate(Key::DownArrow);
        host.navigate(Key::Return);
        assert_eq!(host.count("folder-list"), 0, "{theme}");
        assert_eq!(host.count("editor"), 1, "{theme}: Type a path opens the row's text box");
        host.navigate(Key::Escape);
    }
}

/// The About card and Made with Slint are the last thing on Settings' page, under Power's rows,
/// on a desktop and on the whole screen. The keys reach the end, and the rail lights Power there.
#[test]
fn the_about_card_comes_last_on_settings_in_every_theme() {
    for place in [
        launcher_core::system::Place::default(),
        launcher_core::system::Place { whole_screen: true, ..Default::default() },
    ] {
        for theme in THEMES {
            let mut host = in_theme(theme, |h| h.place(place));
            host.dispatch(Command::Navigate(Route::Settings));
            host.settle();
            host.settings_tab("Power");
            let y = |host: &Composed, label: &str| host.first(label).absolute_position().y;
            let quit = if place.whole_screen { "restart-marah-launcher" } else { "quit" };
            assert!(y(&host, "about") > y(&host, quit), "{theme}: the card is under Power's first row");
            // Down to Power's last row: the card comes into view, and the rail lights Power.
            for _ in 0..4 {
                host.navigate(Key::DownArrow);
            }
            let window = host.instance().window();
            let height = window.size().height as f32 / window.scale_factor();
            let card = host.first("about");
            assert!(
                card.absolute_position().y + card.size().height <= height + 1.0,
                "{theme}: the end of the page shows the card"
            );
            let lit =
                i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "settings-tab:Power")
                    .any(|e| e.accessible_checked() == Some(true));
            assert!(lit, "{theme}: the rail lights Power at the end");
        }
    }
}

/// A click outside a sheet closes it, as Back does; a click inside it, off its rows and buttons,
/// does nothing. The folder browser (a pick picks nothing then), a folder's list, the Drives
/// panel.
#[test]
fn a_click_outside_a_sheet_closes_it_and_one_inside_does_not_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
        let before = host.core().kernel().config().get(&["app", "folders", "games"]);
        host.dispatch(Command::Browse(launcher_core::BrowseCmd::Open {
            at: None,
            purpose: launcher_core::BrowsePurpose::Pick("games".into()),
        }));
        host.settle();
        assert_eq!(host.count("folder-browser"), 1, "{theme}");
        host.click("browser-title");
        assert_eq!(host.count("folder-browser"), 1, "{theme}: a click inside keeps it");
        host.click_at(4.0, 4.0);
        assert_eq!(host.count("folder-browser"), 0, "{theme}: a click outside closes it");
        assert_eq!(host.core().kernel().config().get(&["app", "folders", "games"]), before, "{theme}: nothing picked");

        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Games");
        host.navigate(Key::RightArrow);
        host.navigate(Key::Return);
        assert_eq!(host.count("folder-list"), 1, "{theme}");
        host.click("folder-list-title");
        assert_eq!(host.count("folder-list"), 1, "{theme}: a click inside keeps the list");
        host.click_at(4.0, 4.0);
        assert_eq!(host.count("folder-list"), 0, "{theme}: a click outside closes the list");
    }
}

/// A drive button's name shows on key focus or on hover, not all the time: the Drives panel
/// opened with the mouse shows none; Right shows the next button's.
#[test]
fn a_drive_button_s_name_shows_on_key_focus_not_all_the_time_in_every_theme() {
    let media = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(media.path().join("STICK")).unwrap();
    let system = launcher_testkit::FakeSystem::new();
    system.with(|m| {
        m.drives = vec![launcher_api::SystemDrive {
            device: "/dev/sda1".into(),
            disk: "/dev/sda".into(),
            uuid: "1A2B-3C4D".into(),
            label: "STICK".into(),
            size: 64_000_000_000,
            filesystem: "exfat".into(),
            usb: true,
            mount: Some(media.path().join("STICK")),
            free: Some(40_000_000_000),
            ..launcher_api::SystemDrive::default()
        }];
    });
    for theme in THEMES {
        let plugin = system.plugin();
        let mut host = in_theme(theme, |h| h.plugin(plugin));
        host.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh));
        host.dispatch(Command::System(launcher_core::system::SystemCommand::Refresh));
        host.settle();
        host.click("status:drives");
        assert_eq!(host.value("system-panel"), "drives", "{theme}");
        assert_eq!(host.count("drive-button-label"), 0, "{theme}: opened with the mouse, no name shows");
        host.navigate(Key::RightArrow);
        assert_eq!(host.values("drive-button-label"), ["Use for games"], "{theme}: Right shows the next button's name");
        host.navigate(Key::Escape);
        // A click outside the panel closes it; one inside, on a drive's row, keeps it.
        host.click("status:drives");
        host.click("drive");
        assert_eq!(host.count("system-panel"), 1, "{theme}: a click inside keeps it");
        host.click_at(4.0, 300.0);
        assert_eq!(host.count("system-panel"), 0, "{theme}: a click outside closes it");
    }
}

/// Animations off (the motion tokens at 0): a sheet is in place at its first frame, with no
/// time for it to move.
#[test]
fn with_animations_off_a_sheet_is_in_place_at_once_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
        host.dispatch(Command::Pref(launcher_core::Pref::Animations, false.into()));
        host.settle();
        host.dispatch(Command::Browse(launcher_core::BrowseCmd::Open {
            at: None,
            purpose: launcher_core::BrowsePurpose::Pick("games".into()),
        }));
        let first = host.first("browser-title").absolute_position().y;
        host.wait(std::time::Duration::from_secs(1));
        assert_eq!(host.first("browser-title").absolute_position().y, first, "{theme}: it didn't move");
        host.dispatch(Command::Browse(launcher_core::BrowseCmd::Close));
        assert_eq!(host.count("folder-browser"), 0, "{theme}: gone at once too");
    }
}

/// A sheet opened with the keys takes keys at once, while it still moves in.
#[test]
fn a_sheet_opened_with_the_keys_takes_keys_at_once_in_every_theme() {
    use slint::platform::WindowEvent;
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Games");
        host.navigate(Key::RightArrow);
        let paths = host.core().view().settings.folders[0].list.len();
        let press = |host: &Composed, key: Key| {
            let text: slint::SharedString = key.into();
            host.instance().window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            host.instance().window().dispatch_event(WindowEvent::KeyReleased { text });
        };
        // No time passes between the keys: the list is still on its way in.
        press(&host, Key::Return);
        for _ in 0..paths {
            press(&host, Key::DownArrow);
        }
        assert_eq!(host.focused("folder-add").len(), 1, "{theme}: the keys moved in the list at once");
        press(&host, Key::Escape);
    }
}

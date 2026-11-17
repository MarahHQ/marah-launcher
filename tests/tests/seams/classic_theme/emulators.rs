//! The emulator pages: the Emulators list, each plugin's page, and the install prompt.

use super::*;

#[test]
fn a_service_is_listed_after_the_emulators_with_nothing_to_install() {
    let mut host = composed(|h| {
        h.plugin(FakeStore::new("store").build()).plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    let rows: Vec<_> =
        host.core().view().emulators.plugins.iter().map(|p| (p.id.to_string(), p.kind.clone())).collect();
    assert!(rows.contains(&("store".into(), "service".into())), "{rows:?}");
    assert!(rows.contains(&("emu-a".into(), "emulator".into())), "{rows:?}");
    let listed: Vec<String> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "plugin:store")
            .chain(i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "plugin:emu-a"))
            .map(|e| e.accessible_label().unwrap_or_default().to_string())
            .collect();
    assert_eq!(listed.len(), 2, "both in the Emulators sheet");
}

#[test]
fn a_first_play_asks_before_the_emulator_downloads_and_enter_agrees() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    host.settle();

    assert_eq!(host.values("install-prompt"), ["Install Emu?"]);
    let says = host.values("install-prompt-text").concat();
    assert!(says.contains("Astro") && says.contains("1.0") && says.contains("300 MB"), "{says}");

    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let agreed =
        host.requests.borrow().iter().any(|r| matches!(r, launcher_ui_host::UiRequest::Core(Command::ConfirmInstall)));
    assert!(agreed, "Enter installs and plays");
}

#[test]
fn an_emulator_s_download_says_where_it_is_in_words() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    // The build isn't there: the download fails, and its card stays to say so.
    let core = host.core().clone();
    assert!(host.rt.block_on(core.dispatch(Command::ConfirmInstall)).is_err());
    host.dispatch(Command::Navigate(launcher_core::Route::Downloads));
    host.settle();
    assert_eq!(host.values("download-state"), ["Failed"], "a word for the player, not the core's \"failed\"");
}

#[test]
fn not_now_closes_the_install_prompt() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    host.settle();
    host.navigate(Key::Escape);
    assert!(host.values("install-prompt").is_empty(), "Escape says not now");
    assert!(host.core().view().install_prompt.is_none());
}

/// The Emulators list, then KytyPS5's sheet.
fn kyty_page() -> Composed {
    let mut host = composed(|h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    host.first("plugin:emu-kyty").invoke_accessible_default_action();
    host.pump();
    host.settle();
    host
}

#[test]
fn a_plugin_s_page_opens_from_plugins_and_runs_its_actions() {
    let host = kyty_page();
    assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Plugin);
    assert_eq!(host.first("plugin-enabled").accessible_value().as_deref(), Some("on"));
    // Slot widgets carry their plugin and slot id as the label: "emu-kyty:check" is "Check for new builds".
    host.first("emu-kyty:check").invoke_accessible_default_action();
    let asked = host.requests.borrow().iter().any(
        |r| matches!(r, launcher_ui_host::UiRequest::Core(Command::RunAction { action, .. }) if action == "check"),
    );
    assert!(asked, "the row runs the plugin's action");
}

#[test]
fn a_plugin_s_page_changes_its_settings() {
    let mut host = kyty_page();
    host.settings_tab("Settings");
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("false"));
    host.first("setting:own_build").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("true"));
}

/// The lit section of a plugin page's rail, among `names`.
fn lit_page_section(host: &Composed, names: &[&str]) -> String {
    names
        .iter()
        .find(|name| {
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(
                host.instance(),
                &format!("settings-tab:{name}"),
            )
            .any(|e| e.accessible_checked() == Some(true))
        })
        .expect("one section is lit")
        .to_string()
}

/// KytyPS5's sections: General, its one settings panel, then About (its keyboard controls).
const KYTY_SECTIONS: [&str; 3] = ["General", "Settings", "About"];

/// A plugin's page is one page like Settings: General, then its settings, each under its header,
/// with a rail of the two. Up from Turned on goes to the rail, Down there chooses Settings and
/// Right goes into its rows; Up from its first row goes back into General's last row.
#[test]
fn the_keys_move_down_a_plugin_page_s_one_page_and_across_its_sections() {
    use slint::platform::Key;
    let mut host = kyty_page();
    assert_eq!(host.count("settings-tab:General"), 1, "the rail lists General");
    assert_eq!(host.count("settings-tab:Settings"), 1, "and Settings");
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "Turned on has the focus");
    // Up from the first row to the rail, Down to Settings, Right into its first row.
    host.navigate(Key::UpArrow);
    host.navigate(Key::DownArrow);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "Settings", "Down on the rail chooses Settings");
    host.navigate(Key::RightArrow);
    host.navigate(Key::Return);
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("true"), "Enter changes the row");
    // Up from Settings' first row: General's last row, on the same page.
    host.navigate(Key::UpArrow);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "Up crosses into General");
    host.navigate(Key::DownArrow);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "Settings", "and Down back into Settings");
    host.navigate(Key::Return);
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("false"), "on its first row again");
}

#[test]
fn l1_and_r1_jump_between_the_sections_of_a_plugin_page() {
    use slint::platform::Key;
    let mut host = kyty_page();
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "General is first");
    host.navigate(Key::Tab);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "Settings", "R1 is the next section");
    host.navigate(Key::Return);
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("true"), "on Settings' first row");
    host.navigate(Key::Backtab);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "and L1 the one before");
    host.navigate(Key::Return);
    assert_eq!(host.first("plugin-enabled").accessible_value().as_deref(), Some("off"), "on Turned on");
}

/// Each section of a plugin's page is under its own header, on one page that scrolls as
/// Settings does: a jump to a section brings its header into view.
#[test]
fn a_plugin_s_page_has_a_header_for_each_section_and_a_jump_shows_it() {
    use slint::platform::Key;
    let mut host = slid_in(kyty_page());
    assert_eq!(host.count("settings-page"), 1);
    assert_eq!(host.values("settings-section").first().map(String::as_str), Some("General"));
    host.navigate(Key::Tab);
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
    host.settle();
    assert!(
        host.values("settings-section").iter().any(|s| s == "Settings"),
        "R1 scrolled the Settings header into view: {:?}",
        host.values("settings-section")
    );
}

/// A sheet once its slide in has ended: the mouse goes where it is drawn.
fn slid_in(mut host: Composed) -> Composed {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
    host.settle();
    host
}

/// The mouse wheel scrolls a plugin's page and the rail lights the section in view; a key then
/// lights the focused section again.
#[test]
fn the_rail_follows_a_plugin_page_the_mouse_wheel_scrolls() {
    use slint::platform::Key;
    let mut host = slid_in(kyty_page());
    host.wheel("settings-page", 60);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "About", "at the end of the page, the last section is lit");
    host.wheel("settings-page", -60);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "back at the top, the first section is lit");
    host.wheel("settings-page", 60);
    host.navigate(Key::DownArrow);
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "a key moves on from the focused row");
}

/// What a plugin says of itself (its blocks, an emulator's keyboard controls) is About, the
/// page's last section, after everything the player sets.
#[test]
fn a_plugin_page_ends_with_about() {
    let host = kyty_page();
    let names: Vec<String> = KYTY_SECTIONS
        .iter()
        .filter(|name| host.count(&format!("settings-tab:{name}")) == 1)
        .map(|name| name.to_string())
        .collect();
    assert_eq!(names, KYTY_SECTIONS, "the rail lists General, Settings, then About");
    let y = |name: &str| host.first(&format!("settings-tab:{name}")).absolute_position().y;
    assert!(y("About") > y("Settings") && y("Settings") > y("General"), "About is last on the rail");
}

/// Another plugin's page opens at its top, on its first row: not where the last page was.
#[test]
fn another_plugin_s_page_opens_at_its_top() {
    let mut host = composed(|h| {
        h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin))
            .built_in(launcher_api::PluginBox::code(emu_shadps4::ShadPlugin))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    host.first("plugin:emu-kyty").invoke_accessible_default_action();
    host.pump();
    let mut host = slid_in(host);
    host.wheel("settings-page", 60);
    assert_ne!(host.values("settings-section").first().map(String::as_str), Some("General"), "scrolled down");
    // From the list beside the page, as a player moves from plugin to plugin: the sheet stays.
    host.first("plugin:emu-shadps4").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(
        host.values("settings-section").first().map(String::as_str),
        Some("General"),
        "shadPS4's page at its top"
    );
    host.wheel("settings-page", 60);
    host.first("plugin:emu-kyty").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.values("settings-section").first().map(String::as_str), Some("General"), "at the top again");
    assert_eq!(lit_page_section(&host, &KYTY_SECTIONS), "General", "on its first section");
}

/// A theme's page with no options is one section: no rail.
#[test]
fn a_plugin_page_with_one_section_has_no_rail() {
    let mut host = classic_theme();
    host.dispatch(Command::Navigate(launcher_core::Route::Themes));
    host.dispatch(Command::Navigate(launcher_core::Route::Plugin(id("classic"))));
    host.settle();
    assert_eq!(host.values("settings-section"), ["Theme"]);
    assert_eq!(host.count("settings-tab:Theme"), 0, "one section: no rail");
    assert_eq!(host.count("theme-current"), 1);
}

/// A KytyPS5 game's own settings: one section under the header bar with the game's name.
#[test]
fn a_game_s_own_settings_are_one_section_under_the_header_bar() {
    use slint::platform::Key;
    let games = tempfile::tempdir().unwrap();
    let sce = games.path().join("PS5/Astro/sce_sys");
    std::fs::create_dir_all(&sce).unwrap();
    let param = serde_json::json!({
        "titleId": "PPSA01234", "contentVersion": "01.000.000",
        "localizedParameters": { "defaultLanguage": "en-US", "en-US": { "titleName": "Astro Bot" } },
    });
    std::fs::write(sce.join("param.json"), param.to_string()).unwrap();
    let mut host = composed(|h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] });
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Navigate(launcher_core::Route::GameSettings("emu-kyty:PPSA01234".parse().unwrap())));
    host.settle();
    assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::GameSettings);
    assert_eq!(host.value("game-settings-title"), "Astro Bot");
    assert_eq!(host.values("settings-section"), ["This game"]);
    assert_eq!(host.count("settings-tab:This game"), 0, "one section: no rail");
    // Down to Video out, and Return chooses its next value.
    let before = host.first("setting:video_out").accessible_value().map(|v| v.to_string());
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    let after = host.first("setting:video_out").accessible_value().map(|v| v.to_string());
    assert_ne!(before, after, "Down moved to Video out and Return changed it");
}

#[test]
fn kytyps5_s_own_build_folder_is_a_row_in_its_settings_under_the_release_repo() {
    use slint::platform::Key;
    let mut host = kyty_page();
    assert_eq!(host.count("settings-tab:Folders"), 0, "the page has no Folders section");
    host.settings_tab("Settings");
    assert_eq!(host.count("folder:kyty-build"), 1);
    assert_eq!(host.count("folder:kyty-data"), 1);
    // Right from the rail into Own build; then Release repo, its two folders, and Prereleases.
    host.navigate(Key::RightArrow);
    for _ in 0..4 {
        host.navigate(Key::DownArrow);
    }
    let before = host.first("setting:prereleases").accessible_value().map(|v| v.to_string());
    host.navigate(Key::Return);
    let after = host.first("setting:prereleases").accessible_value().map(|v| v.to_string());
    assert_ne!(before, after, "the row after the folder is Prereleases");
}

#[test]
fn shadps4_s_folders_are_rows_in_its_settings_under_use_your_own_build() {
    use slint::platform::Key;
    let mut host = composed(|h| h.built_in(launcher_api::PluginBox::code(emu_shadps4::ShadPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    host.first("plugin:emu-shadps4").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.count("settings-tab:Folders"), 0, "the page has no Folders section");
    host.settings_tab("Settings");
    assert_eq!((host.count("folder:shadps4-data"), host.count("folder:shadps4-build")), (1, 1));
    // Right from the rail into Own build; then the two folders, and Full screen.
    host.navigate(Key::RightArrow);
    for _ in 0..3 {
        host.navigate(Key::DownArrow);
    }
    let before = host.first("setting:fullscreen").accessible_value().map(|v| v.to_string());
    host.navigate(Key::Return);
    let after = host.first("setting:fullscreen").accessible_value().map(|v| v.to_string());
    assert_ne!(before, after, "the row after the folders is Full screen");
}

#[test]
fn a_plugin_turned_off_on_its_page_shows_only_the_switch() {
    let mut host = kyty_page();
    host.first("plugin-enabled").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("plugin-enabled").accessible_value().as_deref(), Some("off"));
    assert_eq!(host.count("setting:fullscreen"), 0, "its settings are hidden");
    assert_eq!(host.count("build"), 0);
}

#[test]
fn an_emulator_with_no_builds_is_not_called_not_installed() {
    let mut host = composed(|h| h.built_in(launcher_api::PluginBox::code(emu_pc::PcPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    let hint = host.first("plugin:emu-pc").accessible_description().unwrap_or_default().to_string();
    assert_eq!(hint, "No install needed", "PC games need no build");
}

#[test]
fn an_emulator_with_no_builds_has_no_build_row() {
    let mut host = composed(|h| h.built_in(launcher_api::PluginBox::code(emu_pc::PcPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.dispatch(Command::Navigate(launcher_core::Route::Plugin("emu-pc".parse().unwrap())));
    host.settle();
    assert!(host.core().view().plugin_page.is_some(), "PC's page");
    assert_eq!(host.count("build"), 0, "PC games need no build, so there is nothing to install");
    assert_eq!(host.count("uninstall"), 0);
}

#[test]
fn the_game_catalog_page_shows_its_sources_its_refresh_and_refresh_now() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games(["Alpha"]).build().console("PS5"))
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    // Sources with one PS5 forum, so the test doesn't depend on the project's own file, and
    // its feed and forum list.
    let sources = serde_json::json!({ "ps5": { "platform": "PS5",
        "forums": [{ "key": "ps5", "url": "https://xyz.net/forum/viewforum.php?f=546", "tag": "PS5" }] } });
    host.test.http.serve("https://xyz.dev/catalog-sources.json", sources.to_string());
    let feed = include_str!("../../../../plugins/services/release-catalog/fixtures/forum-546.atom");
    host.test.http.serve(&release_catalog::refresh::feed_url("546"), feed);
    let list = include_str!("../../../../plugins/services/release-catalog/fixtures/pvc-546.json");
    host.test.http.serve(&release_catalog::refresh::list_url("546"), list);
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: launcher_api::ConfigScope::Emulator,
        value: serde_json::json!({ "sources_url": "https://xyz.dev/catalog-sources.json",
            "refresh_from_forums": true }),
    });
    // Small enough for the blocks to fit under the settings, a "Show PS5 games" among them.
    host.dispatch(Command::Pref(Pref::Scale, 0.8.into()));
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.dispatch(Command::Navigate(launcher_core::Route::Plugin(id("release-catalog"))));
    host.settle();
    // The blocks: the theme's section cards carry no label a test can find, so the blocks are
    // checked in the view, and their place on the page by its label. They are in About, the
    // page's last section, after the settings.
    host.settings_tab("About");
    assert_eq!(host.count("plugin-blocks"), 1, "the page draws the service's blocks");
    let page = host.core().view().plugin_page.expect("the plugin's page");
    let labels: Vec<String> = page.sections.iter().map(|s| s.label.clone()).collect();
    for label in ["release-catalog:sources", "release-catalog:refresh"] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    // Refresh now is an action, in General at the top.
    host.settings_tab("General");
    assert_eq!(host.count("release-catalog:refresh-now"), 1, "the Refresh now row");
    assert_eq!(host.count("build"), 0, "a service has no build");

    // Refresh now asks the forums again. The refresh runs in the background: the runtime runs
    // it while the test waits.
    let feed = release_catalog::refresh::feed_url("546");
    let asked = |host: &Composed| host.test.http.sent().iter().filter(|u| **u == feed).count();
    // The refresh runs in the plugin's own task: the clock moves until it asked.
    let wait_for = |host: &Composed, count: usize| {
        for _ in 0..100 {
            if asked(host) >= count {
                break;
            }
            host.advance(std::time::Duration::from_millis(10));
        }
    };
    wait_for(&host, 1);
    let before = asked(&host);
    assert_eq!(before, 1, "the core's read refreshed the catalog");
    host.first("release-catalog:refresh-now").invoke_accessible_default_action();
    host.pump();
    host.settle();
    wait_for(&host, before + 1);
    // Once for the action. The core's read of the catalogs after it finds the copy fresh.
    host.advance(std::time::Duration::from_millis(100));
    assert_eq!(asked(&host), before + 1, "Refresh now asked the forums");
}

#[test]
fn a_section_card_grows_with_values_that_wrap() {
    let long = "Refreshed Oct 23, 2026 at 06:06 UTC: 16 new, 0 removed, and a few words more to wrap";
    let rows = [("PC", long), ("PS4", long), ("PS5", long)];
    let facts = FakeService::new("facts", "Facts").section(
        launcher_api::SlotId::EmulatorPanel,
        "refresh",
        "Catalog refresh",
        &rows,
    );
    let mut host = composed(|h| h.plugin(facts.build()));
    host.ui.start().unwrap();
    host.resize(700.0, 900.0);
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.dispatch(Command::Navigate(launcher_core::Route::Plugin(id("facts"))));
    host.settle();

    let cards: Vec<(f32, f32)> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "section-card")
            .map(|c| (c.absolute_position().y, c.absolute_position().y + c.size().height))
            .collect();
    let values: Vec<i_slint_backend_testing::ElementHandle> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "section-value").collect();
    assert_eq!((cards.len(), values.len()), (1, 3), "one card, its three values");
    assert!(values.iter().all(|v| v.size().height > 30.0), "each value wraps onto two lines or more");
    for value in &values {
        let bottom = value.absolute_position().y + value.size().height;
        assert!(bottom <= cards[0].1 + 0.5, "a value ends at {bottom}, below its card's {}", cards[0].1);
    }
}

#[test]
fn a_plugin_s_page_leaves_the_shared_folders_to_settings() {
    let mut host = composed(|h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.dispatch(Command::Navigate(launcher_core::Route::Plugin("emu-kyty".parse().unwrap())));
    host.settle();
    let page = host.core().view().plugin_page.expect("KytyPS5's page");
    let names: Vec<_> = page.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["kyty-build", "kyty-data"], "its own folders only: the shared ones are in Settings");
    assert_eq!(host.count("folder:packages"), 0);
    assert_eq!(host.count("folder:games"), 0);
}

#[test]
fn a_theme_s_sheet_in_themes_has_its_options_while_it_is_in_use() {
    // A theme with an option the player sees (the built-in themes have none now).
    let themes = tempfile::tempdir().unwrap();
    write_theme(
        themes.path(),
        "roomy",
        "[theme.options.density]\nlabel = \"Density\"\ntype = \"choice\"\ndefault = \"cozy\"\n\
         [theme.options.density.choices.cozy]\nmotion-fast = 160\n\
         [theme.options.density.choices.compact]\nmotion-fast = 120\n",
        &[],
    );
    let mut host = classic_theme_with(|h| h.discover(themes.path()));
    host.ui.switch(&"roomy".parse().unwrap()).unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Themes));
    host.settle();
    assert_eq!(host.count("plugin:emu-a"), 0, "Themes lists themes only");
    host.first("plugin:roomy").invoke_accessible_default_action();
    host.pump();
    host.settle();

    assert_eq!(host.first("theme-current").accessible_value().as_deref(), Some("on"));
    assert_eq!(host.first("option:density").accessible_value().as_deref(), Some("cozy"));
    host.first("option:density").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("option:density").accessible_value().as_deref(), Some("compact"));
}

#[test]
fn emulators_lists_emulators_only() {
    let mut host = classic_theme();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    assert_eq!(host.count("plugin:emu-a"), 1);
    assert_eq!(host.count("plugin:default"), 0, "themes are in Themes");
}

#[test]
fn right_to_left_the_install_prompt_has_install_and_play_on_the_right_and_the_keys_follow() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    host.settle();
    assert!(x_of(&host, "Install and play") > x_of(&host, "Not now"), "Install and play on the right");
    host.navigate(Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(asked.iter().any(|c| matches!(c, Command::ConfirmInstall)), "Right stays on it: {asked:?}");
}

#[test]
fn a_block_on_a_plugin_s_page_shows_its_bar_and_its_buttons_take_the_keys_after_the_build() {
    use launcher_api::BlockRow;
    use slint::platform::Key;
    let rows = vec![
        BlockRow { label: "Installed".into(), value: "23.5.0".into(), ..BlockRow::default() },
        BlockRow { label: "Downloading".into(), value: "50%".into(), progress: Some(0.5), ..BlockRow::default() },
        BlockRow {
            label: "Automatic updates".into(),
            action: Some("auto".into()),
            switch: Some(false),
            ..BlockRow::default()
        },
        BlockRow { label: "Update now".into(), action: Some("now".into()), ..BlockRow::default() },
    ];
    let service =
        FakeService::new("firm", "Firm").block(launcher_api::SlotId::EmulatorPanel, "firmware", "Firmware", rows);
    let mut host = composed(|h| h.plugin(service.clone().build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.dispatch(Command::Navigate(launcher_core::Route::Plugin(id("firm"))));
    host.settle();

    for label in ["Installed", "Downloading", "Automatic updates", "Update now"] {
        assert_eq!(host.count(&format!("block-row:{label}")), 1, "{label}");
    }
    assert_eq!(host.value("row-progress"), "50%");
    assert_eq!(host.value("block-row:Automatic updates"), "off");

    // Down from Turned on: the rows that only show are passed over.
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(service.ran(), ["auto", "now"]);
}

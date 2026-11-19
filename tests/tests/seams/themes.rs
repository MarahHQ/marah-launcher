//! Seam 3, composed host: theme loading, inheritance and the theme rules.

use crate::common::*;
use launcher_api::{Files, RelPath};
use launcher_core::Command;
use launcher_ui_api::{ScreenId, ThemeError, TokenValue};

#[test]
fn a_first_run_shows_the_default_theme_and_saves_no_choice() {
    let mut host = composed_first_run(|h| h);
    host.ui.start().unwrap();
    assert_eq!(host.ui.current(), Some(&id(theme_default::ID)));
    assert_eq!(host.core().kernel().config().get(&["app", "theme"]), serde_json::Value::Null);
    assert_eq!(host.ui.resolved().unwrap().theme.manifest.id(), &id("default"), "it stands alone");
}

#[test]
fn the_classic_theme_is_complete_and_compiles() {
    let files = theme_classic::ClassicTheme::files();
    for screen in ScreenId::ALL {
        assert!(files.read(&RelPath::new(screen.file()).unwrap()).is_some(), "classic has no {}", screen.file());
    }

    let mut host = composed(|h| h);
    host.ui.start().expect("the Classic theme starts");
    assert_eq!(host.ui.current(), Some(&id("classic")));
    // Every token has a value: merging would have failed otherwise.
    assert_eq!(host.ui.resolved().unwrap().merged.tokens.len(), launcher_ui_api::TOKENS.len());
}

#[test]
fn every_default_screen_renders() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    let routes = [
        (ScreenId::Home, "Home"),
        (ScreenId::Library, "Library"),
        (ScreenId::Catalog, "Catalog"),
        (ScreenId::Downloads, "Downloads"),
        (ScreenId::Settings, "Settings"),
        (ScreenId::Emulators, "Emulators"),
        (ScreenId::Themes, "Themes"),
    ];
    for (screen, route) in routes {
        let args = [
            slint_interpreter::Value::String("Navigate".into()),
            slint_interpreter::Value::String(format!("{route:?}").into()),
        ];
        host.instance().invoke_global("Host", "command", &args).unwrap();
        host.pump();
        assert_eq!(host.core().view().screen, screen);
    }
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    assert_eq!(host.core().view().screen, ScreenId::GameDetail);
}

#[test]
fn a_theme_s_own_card_and_token_show_and_the_tokens_it_leaves_out_keep_their_defaults() {
    let dir = tempfile::tempdir().unwrap();
    // Default's files with its own card and one token.
    write_theme(
        dir.path(),
        "own-card",
        "[theme.tokens]\ncolor-accent = \"#ff0000\"",
        &[(
            "components/card.slint",
            r#"import { GameCard } from "../types.slint";
import { Tokens } from "../tokens.slint";
export component Card inherits Rectangle {
    in property <GameCard> game;
    in property <length> cover-height;
    callback activated();
    width: 100px;
    height: 100px;
    background: Tokens.color-accent;
    accessible-role: text;
    accessible-label: "own-card";
}
"#,
        )],
    );
    let mut host = composed(|h| {
        h.discover(dir.path())
            .plugin(FakeCatalog::new("emu-kyty").system("PS4").games(["Alpha", "Beta", "Gamma"]).build().console("PS5"))
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.switch(&id("own-card")).expect("the theme loads");
    // A catalog with three games.
    let topics: Vec<serde_json::Value> = ["Alpha", "Beta", "Gamma"]
        .iter()
        .enumerate()
        .map(|(n, title)| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] {title}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "game_info": { "name": title } })
        })
        .collect();
    host.serve_catalog(&[("ps5", serde_json::json!(topics))]);
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));

    let resolved = host.ui.resolved().unwrap();
    assert_eq!(resolved.merged.tokens["color-accent"], TokenValue::Color(0xff0000ff));
    assert_eq!(resolved.merged.tokens["color-bg"], TokenValue::Color(0x05070fff), "the default");
    // The Catalog screen draws the theme's Card for each game.
    host.dispatch(Command::Navigate(launcher_core::Route::Catalog));
    host.dispatch(Command::CatalogSearch("a".into()));
    host.settle();
    assert_eq!(host.count("own-card"), 3);
}

#[test]
fn a_theme_that_extends_another_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_raw_theme(dir.path(), "extender", "extends = \"default\"", &[]);
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();

    assert!(!host.ui.themes().contains(&id("extender")), "it isn't offered");
    assert!(matches!(host.ui.switch(&id("extender")), Err(ThemeError::Unknown(_))));
}

#[test]
fn a_theme_without_every_screen_is_refused_with_the_missing_file_named() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "partial", "", &[]);
    std::fs::remove_file(dir.path().join("partial/screens/downloads.slint")).unwrap();
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();

    let error = host.ui.switch(&id("partial")).unwrap_err();
    assert!(matches!(&error, ThemeError::File { file, .. } if file == "screens/downloads.slint"), "{error}");
    assert_eq!(host.ui.current(), Some(&id("classic")), "the theme on screen stays");
}

#[test]
fn a_theme_built_for_another_ui_api_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "future", "ui_api = \"^9\"", &[]);
    let mut host = composed(|h| h.discover(dir.path()));

    assert!(matches!(host.ui.switch(&id("future")), Err(ThemeError::UiApi { .. })));
}

/// Version 0.5.0 of the contract added `Hub.reset` and `Hub.to-releases`, which the kit's shell
/// and `Games` use. A theme built for 0.4 may ship its own `kit/game-detail.slint` without them:
/// it is refused for its version, and never gets as far as a compile error.
#[test]
fn a_theme_built_for_0_4_is_refused_for_its_version_and_not_compiled() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "old", "ui_api = \"^0.4.1\"", &[("kit/game-detail.slint", "export global Hub {}")]);
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();

    let error = host.ui.switch(&id("old")).unwrap_err();
    assert!(matches!(&error, ThemeError::UiApi { .. }), "{error}");
    assert_eq!(
        error.to_string(),
        format!("theme old needs UI API ^0.4.1, this host has {}", launcher_ui_api::UI_API_VERSION)
    );
    assert_eq!(host.ui.current(), Some(&id("classic")), "the theme on screen stays");
}

#[test]
fn imports_that_leave_the_theme_folder_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("evil.slint"), "export component Evil inherits Rectangle {}").unwrap();
    let escape = format!(
        "import {{ Evil }} from \"../../../../../../../..{}/evil.slint\";\nexport component HomeScreen inherits Rectangle {{ Evil {{}} }}",
        outside.path().display()
    );
    write_theme(dir.path(), "escape", "", &[("screens/home.slint", &escape)]);
    let mut host = composed(|h| h.discover(dir.path()));

    let err = host.ui.switch(&id("escape")).unwrap_err();
    assert!(
        matches!(&err, ThemeError::Compile { diagnostics, .. } if diagnostics.contains("outside the theme")),
        "{err}"
    );
}

#[test]
fn image_urls_and_font_imports_in_markup_are_refused() {
    for (name, source) in [
        ("pic", "export component HomeScreen inherits Rectangle { Image { source: @image-url(\"/etc/x.png\"); } }"),
        ("font", "import \"/Library/Fonts/x.ttf\";\nexport component HomeScreen inherits Rectangle {}"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_theme(dir.path(), name, "", &[("screens/home.slint", source)]);
        let mut host = composed(|h| h.discover(dir.path()));
        let err = host.ui.switch(&id(name)).unwrap_err();
        assert!(matches!(err, ThemeError::File { .. }), "{name}: {err}");
    }
}

#[test]
fn a_theme_may_not_ship_the_files_the_host_serves() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "faker", "", &[("types.slint", "export global Vm {}")]);
    let mut host = composed(|h| h.discover(dir.path()));

    let err = host.ui.switch(&id("faker")).unwrap_err();
    assert!(matches!(&err, ThemeError::File { file, .. } if file == "types.slint"), "{err}");
}

#[test]
fn a_screen_that_exports_the_wrong_name_is_refused_with_the_file_named() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "misnamed", "", &[("screens/home.slint", "export component Home inherits Rectangle {}")]);
    let mut host = composed(|h| h.discover(dir.path()));

    let err = host.ui.switch(&id("misnamed")).unwrap_err();
    assert!(matches!(&err, ThemeError::File { file, .. } if file == "screens/home.slint"), "{err}");
}

#[test]
fn the_default_settings_screen_maps_a_folder_through_the_host() {
    let plugin = launcher_testkit::StubPlugin::from_toml(
        "id = \"pc\"\nkind = \"emulator\"\nname = \"PC games\"\nversion = \"1.0.0\"\nhost_api = \"^0.3\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"",
    );
    let mut host = composed(|h| h.plugin(plugin));
    host.ui.start().unwrap();
    let games = tempfile::tempdir().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("Games");

    assert_eq!(host.count("folder:games"), 1, "a row for the folder");
    // As the theme sends the folder's text.
    let args = serde_json::json!({ "name": "games", "paths": games.path().display().to_string() });
    host.instance()
        .invoke_global(
            "Host",
            "command",
            &[
                slint_interpreter::Value::String("SetFolders".into()),
                slint_interpreter::Value::String(args.to_string().into()),
            ],
        )
        .unwrap();
    host.pump();

    let saved = host.core().kernel().config().get(&["app", "folders", "games"]);
    assert_eq!(saved, serde_json::json!([games.path().display().to_string()]));
    let shown = host.first("folder:games").accessible_value().unwrap_or_default();
    assert_eq!(shown.as_str(), games.path().display().to_string());
}

#[test]
fn a_theme_reads_its_option_values_from_the_options_global() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "opts",
        "\
         [theme.options.motion]\nlabel = \"Motion\"\ntype = \"choice\"\ndefault = \"off\"\n\
         [theme.options.motion.choices.off]\n[theme.options.motion.choices.pan]\n\
         [theme.options.tint]\nlabel = \"Tint\"\ntype = \"color\"\ntoken = \"color-accent\"\ndefault = \"#ff0000\"",
        &[(
            "screens/home.slint",
            r#"import { Options } from "../options.slint";
export component HomeScreen inherits Rectangle {
    Text {
        text: Options.motion;
        color: Options.tint;
        accessible-role: text;
        accessible-label: "motion";
        accessible-value: self.text;
    }
}"#,
        )],
    );
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();
    host.ui.switch(&id("opts")).unwrap();
    assert_eq!(host.first("motion").accessible_value().as_deref(), Some("off"), "unset options read as their default");

    host.dispatch(Command::SetThemeOption { key: "motion".into(), value: "pan".into() });
    assert_eq!(host.first("motion").accessible_value().as_deref(), Some("pan"));

    host.dispatch(Command::SetThemeOption { key: "motion".into(), value: "no-such-choice".into() });
    assert_eq!(
        host.first("motion").accessible_value().as_deref(),
        Some("off"),
        "an unknown choice reads as the default"
    );
}

#[test]
fn the_default_settings_screen_edits_a_folder_with_the_keys() {
    use slint::platform::{Key, WindowEvent};
    let plugin = launcher_testkit::StubPlugin::from_toml(
        "id = \"pc\"\nkind = \"emulator\"\nname = \"PC games\"\nversion = \"1.0.0\"\nhost_api = \"^0.3\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"",
    );
    let mut host = composed(|h| h.plugin(plugin));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Settings));
    host.settle();
    host.settings_tab("Games");
    // Right, from the rail into the tab's rows.
    host.navigate(slint::platform::Key::RightArrow);
    let press = |host: &Composed, text: slint::SharedString| {
        use slint_interpreter::ComponentHandle;
        host.instance().window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(WindowEvent::KeyReleased { text });
    };

    // The focus starts on the folder's row. Enter opens its text box; Enter again saves it.
    press(&host, Key::Return.into());
    host.pump();
    host.settle();
    let typed = full_path("/games");
    for c in typed.chars() {
        press(&host, c.to_string().into());
    }
    press(&host, Key::Return.into());
    host.pump();
    host.settle();
    assert_eq!(host.core().kernel().config().get(&["app", "folders", "games"]), serde_json::json!([typed]));

    // The keys reach the sheet again: Down moves on to the Rescan row, and Enter there scans.
    press(&host, Key::DownArrow.into());
    press(&host, Key::Return.into());
    let asked =
        host.requests.borrow().iter().any(|r| {
            matches!(r, launcher_ui_host::UiRequest::Core(Command::Library(launcher_core::LibraryCmd::Refresh)))
        });
    assert!(asked, "Enter on Rescan, after the text box closed, asks the core to scan");
}

#[test]
fn the_default_theme_is_first_in_the_themes_list_whichever_is_in_use() {
    let y = |host: &Composed, theme: &str| host.first(&format!("plugin:{theme}")).absolute_position().y;
    let mut host = composed(|h| h);
    host.ui.start().unwrap();
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Themes));
        host.settle();
        assert!(y(&host, "default") < y(&host, "classic"), "with {theme} in use, Default is first");
    }
}

#[test]
fn a_hint_for_two_pad_buttons_is_as_wide_as_its_text_not_a_circle() {
    let width = |host: &Composed, key: &str| {
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "hint-key")
            .find(|e| e.accessible_value().as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no hint key {key}"))
            .size()
    };
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build()));
    host.ui.start().unwrap();
    for theme in ["classic", "default"] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Navigate(launcher_core::Route::Home));
        // A controller button: the hints show the pad's buttons.
        host.pad(launcher_ui_host::PadButton::Up);
        host.pad(launcher_ui_host::PadButton::Down);
        let one = width(&host, "✕");
        let two = width(&host, "L1  R1");
        assert_eq!(one.width, one.height, "{theme}: one button is a circle");
        assert!(two.width > two.height * 1.5, "{theme}: L1 R1 is a pill as wide as its text: {two:?}");
    }
}

#[test]
fn a_theme_gets_the_kit_beneath_its_files_and_its_own_kit_file_wins() {
    let kit =
        |name: &str| launcher_ui_api::contract::KIT.iter().find(|(n, _)| *n == name).map(|(_, text)| *text).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let own = format!("// This theme's own commands.\n{}", kit("commands.slint"));
    write_theme(dir.path(), "own-kit", "", &[("kit/commands.slint", own.as_str())]);
    write_theme(dir.path(), "broken-kit", "", &[("kit/commands.slint", "export global Commands {}\n")]);
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();
    let served = |host: &Composed| host.ui.resolved().unwrap().slint_sources().unwrap()["kit/commands.slint"].clone();
    assert_eq!(served(&host), kit("commands.slint"), "Classic ships none: the kit's");

    host.ui.switch(&id("own-kit")).expect("a theme with its own kit file loads");
    assert_eq!(served(&host), own, "the theme's copy replaces the kit's");

    // Proof that the theme's copy is the one compiled: an empty Commands breaks every caller.
    let error = host.ui.switch(&id("broken-kit")).unwrap_err();
    assert!(matches!(error, ThemeError::Compile { .. }), "{error}");
    assert_eq!(host.ui.current(), Some(&id("own-kit")), "the theme on screen stays");
}

#[test]
fn exactly_one_theme_is_the_fallback() {
    assert!(matches!(launcher_ui_host::ThemeSet::default().fallback(), Err(ThemeError::Fallback(_))), "none");

    let mut host = composed_first_run(|h| h);
    assert_eq!(host.ui.themes().fallback().unwrap(), id(theme_default::ID));
    host.ui.start().unwrap();
    assert_eq!(host.ui.current(), Some(&id(theme_default::ID)), "a first run shows the fallback theme");

    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "second", "fallback = true", &[]);
    let mut host = composed_first_run(|h| h.discover(dir.path()));
    let error = host.ui.start().unwrap_err();
    assert!(
        matches!(&error, ThemeError::Fallback(m) if m.contains("default") && m.contains("second")),
        "the error names both: {error}"
    );
}

#[test]
fn the_default_theme_works_with_the_classic_theme_removed() {
    let mut host = composed_default_only(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    assert!(!host.ui.themes().contains(&id(theme_classic::ID)));
    host.ui.start().expect("Default starts without Classic");
    assert_eq!(host.ui.current(), Some(&id(theme_default::ID)));
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    for route in ["Catalog", "Downloads", "Settings", "Emulators", "Themes", "Home"] {
        let args = [
            slint_interpreter::Value::String("Navigate".into()),
            slint_interpreter::Value::String(format!("{route:?}").into()),
        ];
        host.instance().invoke_global("Host", "command", &args).unwrap();
        host.pump();
        assert_eq!(format!("{:?}", host.core().view().screen), route, "the {route} screen");
    }
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    assert_eq!(host.core().view().screen, ScreenId::GameDetail);
}

/// The view properties no built-in theme, kit file or the fixture theme reads: data for a
/// theme that shows it. Each one here is on purpose; a property nothing reads that isn't here
/// fails the test below.
const READ_BY_NO_BUILT_IN_THEME: [&str; 5] = [
    // The selected game's key: the theme switch test reads it back through a new theme.
    "game",
    // The Game Catalog's sources; the built-in themes show them through the plugin's page.
    "catalog-sources",
    // The Settings sheet's sections as data; the built-in themes draw them from the kit.
    "settings",
    // The installed themes; the built-in themes list them through the plugin list.
    "themes",
    // A plugin's page is open; the built-in themes read the screen instead.
    "has-plugin-page",
];

/// Every `Vm` property is read by a Slint file: the kit, a built-in theme or the fixture theme.
/// A property that nothing draws is a field the core builds for no one.
#[test]
fn every_view_property_has_a_reader() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let types = std::fs::read_to_string(root.join("crates/ui-api/contract/types.slint")).unwrap();
    let vm = &types[types.find("export global Vm {").unwrap()..];
    let vm = &vm[..vm.find("\n}").unwrap()];
    let properties: Vec<&str> = vm
        .lines()
        .filter_map(|line| line.trim().strip_prefix("in-out property <"))
        .map(|rest| {
            let name = &rest[rest.find("> ").unwrap() + 2..];
            name[..name.find([':', ';']).unwrap_or(name.len())].trim()
        })
        .collect();
    assert!(properties.len() > 50, "the Vm properties: {properties:?}");
    let mut sources = String::new();
    for dir in ["crates/ui-api/kit", "plugins/themes", "tests/fixtures/themes"] {
        for file in crate::common::slint_files(&root.join(dir)) {
            sources.push_str(&std::fs::read_to_string(file).unwrap());
        }
    }
    let read = |name: &str| {
        let needle = format!("Vm.{name}");
        sources.match_indices(&needle).any(|(at, _)| {
            let next = sources[at + needle.len()..].chars().next();
            !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    };
    let unread: Vec<&&str> = properties.iter().filter(|p| !read(p) && !READ_BY_NO_BUILT_IN_THEME.contains(p)).collect();
    assert!(unread.is_empty(), "Vm properties no Slint file reads: {unread:?}");
    let listed_but_read: Vec<&&str> = READ_BY_NO_BUILT_IN_THEME.iter().filter(|p| read(p)).collect();
    assert!(listed_but_read.is_empty(), "read now, so off the list: {listed_but_read:?}");
}

/// "Animations" is the launcher's own setting: off, every theme's motion tokens are 0, so
/// nothing in any theme moves, and on again they come back.
#[test]
fn animations_off_stills_every_theme() {
    use slint_interpreter::Value;
    let mut host = composed(|h| h.discover(&fixture_themes()));
    host.ui.start().unwrap();
    let motion = |host: &Composed| host.instance().get_global_property("Tokens", "motion-normal").unwrap();
    for theme in ["classic", "default", "fixture-grid"] {
        host.ui.switch(&id(theme)).unwrap();
        host.ui.render().unwrap();
        assert_ne!(motion(&host), Value::Number(0.0), "{theme}: on by default");
        host.dispatch(Command::Pref(launcher_core::Pref::Animations, false.into()));
        host.ui.render().unwrap();
        assert_eq!(motion(&host), Value::Number(0.0), "{theme}: off");
        assert!(!host.core().view().settings.animations, "{theme}: the view says so");
        host.dispatch(Command::Pref(launcher_core::Pref::Animations, true.into()));
        host.ui.render().unwrap();
        assert_ne!(motion(&host), Value::Number(0.0), "{theme}: on again");
    }
}

/// A player who turned the built-in theme's old Animations option off has it saved under
/// `app.theme-options`, and a backup carries it: the theme, which has no such option now,
/// still loads, and the saved value is left out.
#[test]
fn a_saved_value_for_an_option_the_theme_no_longer_has_is_left_out() {
    let mut host = composed(|h| h);
    host.core()
        .kernel()
        .config()
        .set(&["app", "theme-options", "classic"], serde_json::json!({ "motion": "off" }))
        .unwrap();
    host.ui.start().unwrap();
    assert_eq!(host.ui.current(), Some(&id("classic")));
    let options = &host.ui.resolved().unwrap().merged.option_values;
    assert!(!options.contains_key("motion"), "{options:?}");
}

/// The kit's pages keep their state in globals, and a theme that only puts its screens inside
/// the kit's `ShellKeys` gets that state reset when the screen changes: a game's menu closes
/// with its screen, and the next Game Hub starts on its first button.
#[test]
fn a_theme_on_the_kit_s_shell_starts_each_screen_afresh() {
    for theme in ["fixture-grid", "classic", "default"] {
        let mut host = composed(|h| {
            h.discover(&fixture_themes())
                .plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
        });
        host.ui.start().unwrap();
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.resize(1600.0, 900.0);
        host.settle();
        host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
        host.settle();
        let first = host.focused("game-options");
        assert!(first.is_empty(), "{theme}: a button comes before Options");

        host.press("game-options", "Options");
        assert!(!host.values("game-menu").is_empty(), "{theme}: the game's menu is open");
        assert_eq!(host.focused("game-options"), ["Options"], "{theme}: the keys are on Options");
        host.dispatch(Command::Navigate(launcher_core::Route::Downloads));
        host.settle();
        assert!(host.values("game-menu").is_empty(), "{theme}: the game's menu closes with its screen");

        host.dispatch(Command::OpenGame("emu-a:G1".parse().unwrap()));
        host.settle();
        assert_eq!(host.focused("game-options"), first, "{theme}: the next Game Hub starts on its first button");
    }
}

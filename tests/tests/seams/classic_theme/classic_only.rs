//! Tests of the kit's pages that read the Classic theme's own screens around them: its top bar,
//! and its Home behind a sheet. The rest of these pages' tests run in the Default theme too
//! (`kit_in_default`).

use super::*;

#[test]
fn the_direction_option_moves_the_top_bar_to_the_right() {
    let mut host = classic_theme();
    let left = host.first("default-system").absolute_position().x;
    assert!(left < 400.0, "the systems start on the left: {left}");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.settle();
    let right = host.first("default-system").absolute_position().x;
    assert!(right > 800.0, "the systems move to the right: {right}");
}

#[test]
fn the_emulators_list_names_each_console_and_its_program_s_build() {
    let mut host = composed(|h| {
        h.plugin(
            FakeCatalog::new("emu-k")
                .named("PS5")
                .emulator("KytyPS5")
                .platform("PS5")
                .without_slots()
                .playable()
                .installed("KytyPS5-2026-10-21-e2dd48a")
                .games([])
                .build(),
        )
        .plugin(
            FakeCatalog::new("emu-s")
                .named("PS4")
                .emulator("shadPS4")
                .platform("PS5")
                .without_slots()
                .playable()
                .installed("v.0.20.0")
                .games([])
                .build(),
        )
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(launcher_core::Route::Emulators));
    host.settle();
    let hint = |plugin: &str| host.first(plugin).accessible_description().unwrap_or_default().to_string();
    assert_eq!(hint("plugin:emu-k"), "Installed · KytyPS5-2026-10-21-e2dd48a", "the build names its program once");
    assert_eq!(hint("plugin:emu-s"), "Installed · shadPS4 v.0.20.0");
    assert_eq!((host.count("PS5"), host.count("PS4")), (1, 1), "each row names its console");
    assert_eq!((host.count("KytyPS5"), host.count("shadPS4")), (0, 0), "and not its program");
}

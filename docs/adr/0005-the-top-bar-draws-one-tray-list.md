# The top bar draws one tray list

Status: accepted. It replaces the rule in ADR-0004 that a disabled status icon stays in the bar.

## Context

The top bar had a fixed set of buttons: Search, Downloads, Emulators and Themes, then four
status icons (network, sound, Bluetooth, drives), then the battery, then Settings. Each theme
wired each one: its place for the keys (`Nav.tabs + 4 + Sys.statuses`), its icon, what it opened.
The kit's `StatusBar` looped over four panels by number, and an area that couldn't be used kept
its icon in the bar, disabled (ADR-0004). A PC with no Bluetooth adapter showed a dim Bluetooth
icon forever; an icon for muted sound looked the same as one for sound at 40%. A new item, such
as the VPN or the OS's updates, meant editing both themes and the kit.

## Decision

- The core builds one list, the tray (`crates/core/src/tray.rs`). Each item describes itself
  once: a stable id, its icon, name and state line, its group (Status, then Shortcuts), its
  default visibility, its "needed now" rule, and what opening it does (`opens` and `target`: a
  panel, a screen, a Settings section, the power menu). The items come from sources (`SOURCES`):
  the system areas, the downloads and the shortcuts. Plugin items are not a source yet; one more
  function in that list adds them.
- Each item has one of three visibilities, chosen by the player: Always (in the bar), When needed
  (in the bar only while it matters), Only here (only in the tray). Every item is always in the
  tray. The view model gets the list sorted, with each item's `in-bar` resolved, and the bar's and
  the tray's items by their place (`Vm.tray`, `Vm.tray-bar`, `Vm.tray-rows`).
- An item whose area is absent or unsupported is never in the bar, whatever the player chose: the
  tray lists it dimmed, with its reason. A loading item set to Always shows loading in the bar;
  one set to When needed waits in the tray, since "needed" isn't known before the read.
- The choices are saved by id under `app.tray`, one key per item (`Command::Tray`). An item the
  player never chose has its own default; a saved id the core doesn't list is ignored. The
  backup holds them.
- "Downloads just finished" lasts until the player sees Downloads: the core keeps the finished
  downloads the player has seen (`AppState::downloads_seen`), filled at the start and whenever
  Downloads opens or closes. What finished before this start isn't news.
- The kit draws whatever is in the list: `StatusBar` for the bar, `kit/tray.slint` for the ⌃ and
  the popover, its keys and hints, and its Edit view. Settings → Launcher → Top bar opens the same
  Edit view. The bar's order is the items, Settings, the ⌃, then the clock where one shows.

## Consequences

- `UI_API_VERSION` goes from 0.7.0 to 0.8.0, as 0.6.0 went to 0.7.0 before either was released:
  the kit's `StatusBar`, `StatusButton` and `StatusState` changed their inputs, and both themes'
  `Nav` count other stops. `a_theme_built_for_0_7_is_refused_for_its_version_and_not_compiled`
  in `tests/tests/seams/themes.rs` guards it. The plugin API doesn't change.
- The tray is built outside the game lists (`view::GameLists`): a change of the choices
  (`Area::Tray`) or of the machine rebuilds no game list.
- A new item is one entry in a source and its id in `tray::IDS`; no theme changes.
- Tests guard it: the core's rules in `crates/core/src/tray/tests.rs`;
  `the_tray_s_choices_are_saved_by_id_and_kept_after_a_restart` in
  `tests/tests/seams/system.rs`;
  `the_tray_s_downloads_is_in_the_bar_while_one_runs_and_until_its_end_is_seen` in
  `tests/tests/seams/torrents.rs`; and the mouse, keys and pad in both themes in
  `tests/tests/seams/tray.rs`.

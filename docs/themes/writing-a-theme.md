# Writing a theme

This page takes a theme author from an empty folder to a theme the launcher loads, shows and
tests. The contract a theme codes against (the view model, the commands, the kit, the tokens)
is in [reference.md](reference.md). Terms follow [the glossary](../../GLOSSARY.md).

## What a theme is

A theme is a folder of Slint markup, a `theme.toml` manifest, and the images, fonts and sounds
it wants. It has no code. The launcher compiles the markup when it shows the theme.

- The theme draws every screen. It decides the layout, how the keys move within a screen, the
  colors, the motion and the sounds.
- The core owns everything else: the games, the downloads, the settings, and which screen
  shows. The host fills the `Vm` global with the view, and the theme asks for changes through
  the `Host` global.
- Every theme stands alone. It ships its own shell and every screen. It can import shared
  Slint files from the kit, which the host serves under `kit/`. The reasons are in
  [ADR 0003](../adr/0003-themes-stand-alone-with-a-shared-kit.md).

The launcher has two built-in themes, Default and Classic. They are compiled into the binary:
each theme crate's `build.rs` embeds every file under its `theme/` folder.

- **Default** (`default`, `plugins/themes/default/theme/`) is the fallback theme. Home is a
  list of systems on the far side of the reading direction: "All games" first, then each entry
  of `Vm.emulators`. The focused system's cover fills the screen behind the list. It sets two
  tokens, `color-accent` and `color-accent-soft`.
- **Classic** (`classic`, `plugins/themes/classic/theme/`) shows a hero and a row of game tiles
  on Home. [classic-theme.md](classic-theme.md) describes it file by file.

## Start from the smallest theme

`tests/fixtures/themes/fixture-grid/` is the smallest complete theme in the repository: a light
grid theme with a sidebar. Copy it to start. Its files are the files every theme has:

```text
fixture-grid/
  theme.toml
  shell.slint
  screens/home.slint
  screens/library.slint
  screens/game-detail.slint
  screens/catalog.slint
  screens/downloads.slint
  screens/settings.slint
  screens/emulators.slint
  screens/themes.slint
  screens/plugin.slint
  screens/game-settings.slint
  screens/running.slint
  components/card.slint
  fonts/Inter-Regular.ttf
  fonts/Inter-Bold.ttf
  fonts/OFL.txt
```

A theme must ship `shell.slint` and the ten screen files. Each screen file exports one
component under a fixed name: the table is in [reference.md](reference.md#screens). Other
`.slint` files and the asset files are the theme's choice. The fixture's
`screens/running.slint`, the overlay of a running game, is one of them.

`shell.slint` exports `Shell`, which inherits `Window`. The shell decides where each screen
shows. A screen can be one line that reuses a kit page:

```slint
import { DownloadsPage } from "../kit/downloads.slint";

// Downloads: the kit's page, drawn in this theme's tokens.
export component DownloadsScreen inherits DownloadsPage {}
```

Most themes put the kit's `ShellKeys` and `Overlays` in their shell, so the kit's pages and
sheets get their keys. [What a shell does for the kit](reference.md#what-a-shell-does-for-the-kit)
lists what to wire.

## theme.toml

The top of the file is the plugin manifest. A theme needs these keys. From
`tests/fixtures/themes/fixture-grid/theme.toml`:

```toml
id = "fixture-grid"
kind = "theme"
name = "Fixture grid"
version = "0.1.0"
host_api = "^0.4"
capabilities = ["theme"]

[runtime]
type = "data"

[theme]
ui_api = "^0.7"
```

- `id`: 1 to 64 lowercase letters, digits or `-`, starting with a letter. It can't be the id of
  a built-in plugin, such as `default` or `classic`.
- `host_api` must include the plugin API version, `API_VERSION` in `crates/api/src/lib.rs`
  (now `0.4.0`).
- `capabilities` is exactly `["theme"]`.
- `[runtime] type = "data"`: a theme has no code.
- `[theme] ui_api` must include the theme contract's version, `UI_API_VERSION` in
  `crates/ui-api/src/lib.rs` (now `0.7.0`).

The rest of `[theme]` is optional: input devices, tokens, breakpoints, assets, options and
system art. Every key is in [reference.md](reference.md#the-theme-section).

## Install a theme

Put the theme's folder in the extensions folder. The launcher reads it at start.

- The extensions folder is `--extensions <path>`, or `extensions` in the data folder without
  it. Where the data folder is, on each system, is in
  [Where the launcher keeps its data](../players/troubleshooting.md#where-the-launcher-keeps-its-data).
- The launcher also reads `extensions` next to the program, or `../Resources/extensions` in a
  macOS app bundle when that folder exists (`bundled_extensions` in `crates/app/src/args.rs`).

`Kernel::discover` (`crates/core/src/kernel.rs`) reads each subfolder that holds a
`plugin.toml` or a `theme.toml`, in sorted order. The built-in plugins come first. When two
plugins have the same id, the first one stays and the other is refused as a duplicate.

The kernel refuses a theme folder when:

- the manifest doesn't read;
- `host_api` doesn't include the host's API version;
- `platforms` is set and leaves out this system;
- `[runtime] type` is not `"data"` (`third-party plugins must be data-only`);
- `capabilities` is anything other than `["theme"]`;
- the launcher runs in safe mode.

The log names a refused plugin with `plugin not loaded` and the reason. A theme whose
`[theme]` section doesn't read is left out of the Themes list, and the log says
`theme skipped` (`ThemeSet::from_core` in `crates/ui-host/src/themes.rs`).

## See your edits live

Run the launcher with `--dev`, and with `--theme <id>` to show your theme for this run without
saving the choice:

```sh
cargo run -p launcher-app -- --windowed --dev --theme fixture-grid --extensions <folder>
```

Every 250 ms (`TICK` in `crates/ui-host/src/shell.rs`), the host compares the newest change
time of the theme's files with the last load. When a file changed, the host loads the theme
again.

- An edit that doesn't load keeps the last good version on screen. The log says
  `theme edit doesn't load; keeping the last good version`. The host doesn't try again until a
  file changes.
- Markup, images, fonts and sounds reload. `theme.toml` doesn't: the host read it at start.
  Restart the launcher after you change it.
- The built-in themes don't reload: they have no folder on disk. An edit to them needs a new
  build.

The other options are in [Command-line options](../players/troubleshooting.md#command-line-options).

## Switching, the fallback theme and safe mode

The player switches the theme on the Themes screen. A theme can do the same with
`Host.switch-theme(id)`. The kit's `PluginPage` calls it for "Use this theme".

A switch (`ThemeHost::switch` in `crates/ui-host/src/host.rs`) reads the theme and its option
values, decodes its images, sounds and system art, compiles it, makes the new theme in the same
window and binds the view. Then the core saves the choice, and the new theme shows.

- A failure at any step leaves the old theme on screen. The log says
  `the theme's request failed`. The player sees no message.
- Navigation, downloads and running games stay: they are in the core, not in the theme.

The fallback theme is the one installed theme with `fallback = true`. Default is that theme.

- The host starts the fallback theme when the player chose none, and when the chosen theme
  fails to load.
- The Themes list shows it first.
- No theme, or more than one, with `fallback = true` stops the start with
  `ThemeError::Fallback`. When more than one sets it, the error names them. So a theme in the
  extensions folder must not set it.

`--safe-mode` loads no theme folder and shows the fallback theme only. A switch to another
theme fails with `safe mode shows the fallback theme only`.

## Test a theme

Tests drive a theme the way a player does, through `ThemeDriver` in
`crates/testkit/src/driver.rs`. A test finds elements by their accessible label and value,
sends keys and clicks, and moves Slint's clock. It never reads `Vm` or calls the theme's
callbacks. So give each element that a test needs an `accessible-label`.

| Step | What it does |
| --- | --- |
| `count(label)`, `first(label)` | How many elements have the label, or the first one. |
| `value(label)`, `values(label)`, `description(label)` | Read accessible values. |
| `focused(label)`, `lit(label)` | Which elements show as focused. |
| `press(label, value)`, `press_first(label)` | Run an element's default action. |
| `click(label)`, `wheel(...)` | A mouse click in the middle of the element, or a wheel turn. |
| `tap(key)`, `navigate(key)` | Send a key. `navigate` also carries out what the theme asked. |
| `type_text(text)` | Type characters. |
| `settings_tab(name)` | Jump to a Settings section, with the focus on the rail. |
| `settle()`, `wait(duration)` | Move Slint's clock on. |

The theme tests are seam tests in `tests/tests/seams/` (package `launcher-tests`, test binary
`seams`):

- `every_theme.rs` runs one suite against each built-in theme in `THEMES`: every screen renders,
  Settings, Backup, the on-screen keyboard and the folder browser.
- `themes.rs` covers loading and the rules: the fallback theme, refused keys, a missing screen,
  a wrong `ui_api`, imports out of the folder, `@image-url` and font imports, served files, a
  wrong export, the `Options` global, kit replacement, and exactly one fallback theme.
- `classic_theme/` and `default_theme/` test each built-in theme's own screens and files.
  `kit_in_default/` runs Classic's tests of the kit's pages in the Default theme too. Each
  `theme_files.rs` checks that every animation time goes through `Look.timed`.
- `motion.rs` checks the kit and the built-in themes: with Animations off, nothing moves.
- `text_fit.rs` checks that a `Text` with a fixed font size fits the fixed height of the
  element it is in.

`write_theme` in `tests/tests/seams/common/mod.rs` writes a test theme: a copy of Default's
files under a new id, with your `[theme]` lines and files over it.

Run the theme tests:

```sh
cargo test -p launcher-tests --test seams themes::
cargo test -p launcher-tests --test seams every_theme::
```

Render every screen of the built-in themes offscreen, to PNG files in
`target/tmp/screenshots/`:

```sh
cargo test -p launcher-tests --test seams screenshots::
```

`make check` runs the format check, clippy and every test, as CI does. The other check
commands are in [development.md](../contributors/development.md#tests).

## Limitations

- `navigation` is read into the merged theme, but no host code reads it.
- Only the first `input` entry counts, at start. The host never sets `touch`. After the first
  controller button, `Vm.input-mode` stays `gamepad` until the launcher restarts.
- The host doesn't check that slot items set `accessible-label`.
- Dev mode doesn't read `theme.toml` again, and never reloads a built-in theme.
- A failed `Host.switch-theme` is in the log only. The player sees no message.
- A theme can't add screens. The screens are `ScreenId::ALL`.
- Classic's own files still use fixed hex colors, so a token change doesn't reach every part of
  it. See [classic-theme.md](classic-theme.md#tokens).
- The right-to-left layout is not finished. Both built-in themes hide their "Layout direction"
  option.

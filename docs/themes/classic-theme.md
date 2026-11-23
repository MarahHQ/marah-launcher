# The Classic theme

This page describes the built-in Classic theme (`classic`) file by file: what it ships, what it
draws, and where it differs from Default. It is for theme authors who want to read a complete
theme, and for contributors who change it. How any theme works is in
[writing-a-theme.md](writing-a-theme.md) and [reference.md](reference.md).

Text in quotes is the text on screen, copied from the `.slint` files.

## What it is

Classic shows a hero and a row of game tiles on Home, over the selected game's art. It is a
theme plugin compiled into the binary (`ClassicTheme` in `plugins/themes/classic/src/lib.rs`).
Its `build.rs` embeds every file under `plugins/themes/classic/theme/`.

It stands alone, as every theme does. It ships its shell and every screen. It draws Downloads,
the Game Hub, Settings, the plugin pages, the Emulators and Themes lists, the sheets and the
prompts with the kit. It is not the fallback theme: Default is.

## Files

All paths are under `plugins/themes/classic/theme/`.

| File | What it holds |
| --- | --- |
| `theme.toml` | The manifest. |
| `shell.slint` | `Shell`: the window, the backdrop, the kit's `ShellKeys` around the screens, the top bar, the hints, the notices and the task pill. |
| `screens/home.slint` | `HomeScreen`: the tile row and the hero. |
| `screens/library.slint` | `LibraryScreen`: `LibraryView` on the player's games. |
| `screens/catalog.slint` | `CatalogScreen`: `LibraryView` on the catalogs' games. |
| `screens/game-detail.slint` | `GameDetailScreen`: the kit's `GameDetailPage`. |
| `screens/downloads.slint` | `DownloadsScreen`: the kit's `DownloadsPage`. |
| `screens/settings.slint` | `SettingsScreen`: the kit's `SettingsPage`, with the theme's emblem. |
| `screens/emulators.slint` | `EmulatorsScreen`: the kit's `PluginListSheet`. |
| `screens/themes.slint` | `ThemesScreen`: the kit's `PluginListSheet`. |
| `screens/plugin.slint` | `PluginScreen`: the kit's `PluginPage`. |
| `screens/game-settings.slint` | `GameSettingsScreen`: the kit's `GameSettingsPage`. |
| `screens/running.slint` | `RunningScreen` and the `Splash` global: the launch splash, which the shell draws over the screen. It is not a screen of the contract. |
| `components/backdrop.slint` | `Backdrop`: the art behind every screen. |
| `components/boot.slint` | `BootScreen`: the boot screen and Welcome. |
| `components/card.slint` | `Card`: a Library or Catalog card. |
| `components/library-view.slint` | `LibraryView`: the header and the grid, for the Library and the Catalog. |
| `components/nav.slint` | `Nav`: where the keys are on Home, the Library and the Catalog, and what each key does there. |
| `components/top-bar.slint` | `TopBar`, `PadGlyph`, `PadArrows`, `Hints`: the top bar and the key hints. |
| `fonts/` | Inter (Regular, Medium, SemiBold, Bold) and Tajawal (Regular, Bold), with `OFL-Inter.txt` and `OFL-Tajawal.txt`. |
| `images/logo-mark.png` | The launcher's mark, its only logo inside the launcher. |
| `sounds/` | `move.wav`, `select.wav`, `back.wav`, `error.wav`, `start.wav`. |

### How Default differs

Default has the same files, with these differences:

- `theme.toml`: its own id and name, `fallback = true`, two tokens (`color-accent` and
  `color-accent-soft`) instead of all of them, and two more images.
- `shell.slint`: its first comment, and two lines that decide when Home's art and the game keys
  wait for the game's details.
- `screens/home.slint`, `screens/library.slint`, `components/nav.slint`,
  `components/top-bar.slint` and `components/library-view.slint` are Default's own.
- Default adds `components/chip-row.slint`, `components/systems-cover.slint`,
  `images/all-games-icon.png` and `images/all-games-cover.jpg`.

Every other file is the same in both themes.

## The manifest

From `theme.toml`:

| Key | Value |
| --- | --- |
| `id`, `name` | `classic`, `Classic` |
| `version` | `0.3.0` |
| `host_api` | `^0.6` |
| `activation` | `eager` |
| `ui_api` | `^0.8` |
| `input` | `["gamepad", "pointer"]` |
| `navigation` | `focus` |
| breakpoints | `compact` (`max_width = 900`), then `regular` |
| fonts | the six font files above |
| images | `logo-mark` |
| sounds | `move`, `select`, `back`, `error`, `start` |

It has no `fallback` key.

### Tokens

`[theme.tokens]` sets all 25 tokens, each to its default value. The defaults are the Classic
look, so Classic would look the same with no tokens at all. The table of tokens is in
[reference.md](reference.md#tokens).

Classic's own files also use fixed hex colors, which no token changes:

| File | Fixed colors (comments left out) |
| --- | --- |
| `components/boot.slint` | 29 |
| `components/top-bar.slint` | 29 |
| `components/library-view.slint` | 21 |
| `components/backdrop.slint` | 15 |
| `components/card.slint` | 12 |
| `screens/home.slint` | 2 |
| `screens/running.slint` | 1 |
| `shell.slint` | 1 |

### Options

| Option | Label | Type | Default | Sets |
| --- | --- | --- | --- | --- |
| `direction` | "Layout direction" | choice, `hidden = true` | `ltr` | `layout-direction`: `ltr` or `rtl` |

The option is hidden because the right-to-left layout is not finished (a comment in
`theme.toml` says so).

## Sizes

Classic lays everything out on a 1920×1080 canvas and scales it to the window, through the
kit's `Look` (`kit/look.slint`):

- `Look.k` is the window's height over 1080 px (at least 360 px), times the player's "Size of
  everything" (`Vm.viewport.size`). `Look.px` is `1px * Look.k`.
- `Look.t` is `Look.k`, but at least 0.75, for text that must stay readable. `Look.tpx` is
  `1px * Look.t`.
- The shell sets `Look.window-height` from its own height, so the first frame has the right
  size before the host's `Vm.viewport` comes.

## The shell

`shell.slint` exports `Shell`, which inherits `Window`:

- the title is `Brand.name`, and the background is `Tokens.color-bg`;
- the preferred size is 1280×720 px, and the smallest is 640×360 px;
- the font is `Tokens.font-family` at `Tokens.font-size-m`.

### Layers

From the back to the front:

1. `Backdrop`, with the art of the shown game. It dims (0.84) on every screen but Home, the
   Library, the Game Hub and the side sheets, which draw their own wash.
2. The kit's `ShellKeys`, which holds the screens:
   - `HomeScreen`, always there. It shows on Home and under the Library. Under a side sheet it
     dims to 0.3; on other screens it hides.
   - The Library, the Game Hub, the Catalog and Downloads, each while it is the screen. A kit
     `Lingering` keeps each one on screen while it goes out.
   - The side sheets: Emulators, Themes, a plugin's page, Settings and a game's settings. A
     plugin's page opens beside the Emulators or Themes list, which stays.
   - `TopBar`, on Home, the Library, the Catalog and Downloads.
   - The kit's `SystemSheet`, while a machine panel is open and the launcher is the whole
     screen.
   - `RunningScreen`.
   - The kit's `Overlays`, with the emblem.
3. `Hints`, the key hints, in a pill at the bottom.
4. The kit's `NoticeStack`, on the far side.
5. The task pill: `Vm.task` with a turning ring, at the bottom corner, on Home with no sheet.
6. The kit's `OnScreenKeyboard`, while it is open.
7. `BootScreen`, over everything, from the first frame.
8. The kit's `QuestionPrompt`, while `Vm.has-question`.

### Keys

The kit's `ShellKeys` answers the keys, in the order listed in
[reference.md](reference.md#what-a-shell-does-for-the-kit). Classic adds its own keys through
the callbacks:

- `theme-key`:
  - `/` (△) searches: the Catalog on its screen, else the Library.
  - Insert (□) plays the shown game's trailer. T does too, but not in the Library or the
    Catalog, where it types.
  - On Home, Tab and Shift+Tab (R1 and L1) move along the top bar, round its ends.
- `screen-key`: on Home, the Library and the Catalog, `Nav.key`.
- `screen-changed`: the Catalog and the Library open at their grid, or at the search box when
  the shell's search opened the Library; Home opens at its row.

### Hints

`Hints` shows the keys that do something now, in a pill at the bottom. It sits on the far side,
or on the near side under a side sheet. With a controller (`Vm.input-mode == "gamepad"`) it shows
the pad's buttons; with a keyboard, the keys. The face buttons are drawn as paths (`PadGlyph`,
`PadArrows`), because no font the theme ships has them.

The hints hide over the Library's and the Catalog's cards (they would cover the titles), while
a trailer plays with its controls hidden, over the folder browser, over the splash's keys, and
over the closing screen.

### Sounds

| Sound | When |
| --- | --- |
| `start` | A game starts; the Welcome screen closes. |
| `error` | A new notice of kind `error` comes. |
| `back` | Back, except on Home. |
| `move`, `select` | Moves and choices on Home, the Library and the Catalog (`Nav`). |

## Screens

### Home (`screens/home.slint`)

Home is a row of tiles, the selected one larger with its name beside it, and a hero over the
game's art. Past the last game is the Library tile. The tile of the running game has a green
frame and a PLAYING tag.

The hero comes in a piece at a time when the selection changes: the line over the title, the
title (or the game's logo from the store), the chips, the description, then the buttons. It
waits for the game's details, at most `Look.wait-for-details` (450 ms).

- While the game runs, the line over the title is "● PLAYING · " and the time played.
- The buttons while the game runs are "Resume", "Stop" and "Game Hub". Otherwise they are
  "Play" (when the game can start) and "Game Hub". Then "Store" when the game has a store page,
  and "Options" (the game's menu) once its details are in. The Library tile has "Open Library".
- Under the buttons: `Vm.detail.launch-hint`, such as a note that Play installs the emulator
  first.

### Library and Catalog (`screens/library.slint`, `screens/catalog.slint`)

Both are `LibraryView`, sliding in from the side the screens move from. The Library shows the
player's games, over Home. The Catalog shows the catalogs' games (`Vm.catalog`), and its bar
adds the catalog plugin's first button, such as a refresh. The search, filters and sort stay in
the core, so each screen opens again as the player left it.

### Keys on Home, the Library and the Catalog (`components/nav.slint`)

The keys act on one zone at a time:

| Zone | What it is |
| --- | --- |
| `tabs` | The top bar: the "Home" and "Library" tabs, Search, the tray items in the bar, Settings, and the tray's ⌃. |
| `row` | Home's tiles. |
| `actions` | The buttons under the game on Home. |
| `bar` | The grid's bar: search, sort, the card sizes, and in the Catalog the catalog plugin's button. |
| `filters` | The systems. |
| `genres` | The genres. |
| `grid` | The cards. |

Up and Down move between the zones; Enter opens or runs the focused item; Escape goes back.
Left and Right follow the reading direction. In the grid, Page Up and Page Down (L2 and R2)
move three rows. A letter or a digit typed in the grid starts a search with it.

A move shows at once. The core hears of the selection `Look.select-after` (100 ms) after the
last move: the kit's `ShellKeys` timer calls `Games.send`.

### The kit's screens

- **Game Hub** (`screens/game-detail.slint`): the kit's `GameDetailPage`. It opens out of the
  card the player was on (`Origin`): the cover grows from the card's place.
- **Downloads** (`screens/downloads.slint`): the kit's `DownloadsPage`.
- **Settings** (`screens/settings.slint`): the kit's `SettingsPage`, one page of sections with a
  rail of their icons. Its `logo` is the theme's mark (`Assets.logo-mark`).
- **Emulators** and **Themes** (`screens/emulators.slint`, `screens/themes.slint`): the kit's
  `PluginListSheet`, with these titles and lines:
  - "Emulators", "Open one to turn it on or off, install it and change its settings".
  - "Themes", "Open one to use it and change its options".
- **A plugin's page** (`screens/plugin.slint`): the kit's `PluginPage`.
- **Game settings** (`screens/game-settings.slint`): the kit's `GameSettingsPage`.

### Running (`screens/running.slint`)

While a game starts: its art dimmed behind its icon, its name, "Starting with" and the
emulator's name, and a turning ring. Without a controller, it lists the emulator's keyboard
keys (`Vm.running.keymap`), and the shell gives every key but Escape and Ctrl+Q to the game
(`Splash.keys`). It fades once the game's window shows. The screen under it stays as it was.

## Components

### Backdrop (`components/backdrop.slint`)

The art behind every screen: a deep blue glow, the game's art, and dark washes from the text
side, from the bottom and under the top bar. New art fades in over the old and settles from a
slight zoom (`Tokens.motion-slow` × 4). Once it settles, nothing is drawn every frame.

### Boot screen and Welcome (`components/boot.slint`)

A slowly drifting wall of the library's covers behind the emblem, a progress bar and
`Vm.boot.status`. It shows from the first frame until the library and Home's art are in, then
fades out. While the start's installs run, it shows a checklist with each install's progress.

On the first run it is Welcome: "Welcome", "Setting up" and the product's name, the checklist
of `Vm.boot.steps`, and a button that says "Setting up…" until it is ready, then "Start".

### Card (`components/card.slint`)

A 2:3 cover with the console's name on top, how the game runs and its rating at the bottom;
then two lines for the title, and the size. A game a catalog offers also shows its number of
releases beside the size, and how easy its best release is to download under it. On the
focused or pointed card: the main action, and in the Catalog an eye that hides or shows the
game. The focused card grows a little and gets the white ring.

### Library view (`components/library-view.slint`)

A header band with "Search games" ("Search the catalog" in the Catalog), the sort button and the
card sizes ("Comfortable", or "Roomy" in a window under 1000 px, and "Compact"). Under it: the
systems as chips with their counts, the count of games (in the Catalog with "List updated" and
the date), and "GENRE" with the genres. The grid of cards scrolls under the band, asks for the
next page as it nears the end, and keeps the selected card in view. The list of sorts opens
under the sort button, under "SORT BY".

An empty grid says "Loading catalog…", "No game matches" and the search, "No catalog lists any
game yet" or "No games here".

### Top bar (`components/top-bar.slint`)

On the reading side: the "Home" and "Library" tabs. On the far side: the running game, the
active download, Search, the kit's status bar (`StatusBar`: the tray items in the bar now,
wherever the launcher runs), Settings, the tray's ⌃ (`TrayButton`), and the clock. Search shows
only when there is a Catalog. Downloads, Emulators and Themes are tray items.

## Accessible labels

Tests find Classic's elements by their accessible labels. These are among them:

| Where | Labels |
| --- | --- |
| Home | `default-tile`, `default-library-tile`, `default-sub`, `default-title`, `default-played`, `default-description`, `default-size`, `your-result`, `default-resume`, `default-stop`, `game-options` |
| Library and Catalog | `default-game`, `default-count`, `catalog-updated`, `grid-empty`, `grid-progress`, `library-sort`, `sort-option`, `sort-current`, `density`, `genres`, `genres-more-start`, `genres-more-end`, `filter:<name>` |
| Card | `card-console`, `card-hide`, `card-play`, `card-get`, `card-size`, `card-releases`, `card-availability` |
| Top bar and shell | `default-system`, `clock`, `hint`, `hint-key`, `task` |
| Boot | `boot`, `welcome`, `welcome-step`, `welcome-start` |
| Running | `launch-input`, `launch-key` |

The `default-` labels are names only: Classic uses them too.

## Tests

`tests/tests/seams/classic_theme/` tests the theme, one module for each part: `catalog.rs`,
`downloads.rs`, `emulators.rs`, `game_hub.rs`, `home.rs`, `installs.rs`, `keyboard.rs`,
`library.rs`, `notices.rs`, `play.rs`, `releases.rs`, `settings.rs`, `theme_files.rs` and
`trailer.rs`. Tests that read Classic's own screens are in `classic_only.rs`. `theme_files.rs`
checks that every font the theme names ships with it, and that every animation time goes
through `Look.timed`. `every_theme.rs` runs its suite on
Classic and Default.

```sh
cargo test -p launcher-tests --test seams classic_theme::
```

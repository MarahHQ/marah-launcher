# Theme contract reference

This page lists everything a theme can rely on and everything the host checks: the files and
globals the host serves, the view model, the commands, the kit, the tokens and the rules. It is
for theme authors who already have a theme running; start with
[writing-a-theme.md](writing-a-theme.md). Terms follow [the glossary](../../GLOSSARY.md).

The contract is the crate `launcher-ui-api` (`crates/ui-api`). Its version is
`UI_API_VERSION`, now `0.7.0` (`crates/ui-api/src/lib.rs`). A theme's `ui_api` range must
include it.

## Contents

- [Screens](#screens)
- [Where the player is](#where-the-player-is)
- [The theme section](#the-theme-section)
- [The files the host serves](#the-files-the-host-serves)
- [The view model: Vm](#the-view-model-vm)
- [Host: what to ask for](#host-what-to-ask-for)
- [Torrent actions](#torrent-actions)
- [Slots](#slots)
- [The kit](#the-kit)
- [Tokens](#tokens)
- [Options](#options)
- [Breakpoints and the size of everything](#breakpoints-and-the-size-of-everything)
- [Keys and the gamepad](#keys-and-the-gamepad)
- [Images, fonts, sounds and system art](#images-fonts-sounds-and-system-art)
- [Rules the host checks](#rules-the-host-checks)

## Screens

The screens are `ScreenId::ALL` in `crates/ui-api/src/vm.rs`. Each screen file exports one
component under the name `ScreenId::component` gives (`crates/ui-api/src/contract.rs`):

| `Vm.screen` | File | Exported component |
| --- | --- | --- |
| `home` | `screens/home.slint` | `HomeScreen` |
| `library` | `screens/library.slint` | `LibraryScreen` |
| `game-detail` | `screens/game-detail.slint` | `GameDetailScreen` |
| `catalog` | `screens/catalog.slint` | `CatalogScreen` |
| `downloads` | `screens/downloads.slint` | `DownloadsScreen` |
| `settings` | `screens/settings.slint` | `SettingsScreen` |
| `emulators` | `screens/emulators.slint` | `EmulatorsScreen` |
| `themes` | `screens/themes.slint` | `ThemesScreen` |
| `plugin` | `screens/plugin.slint` | `PluginScreen` |
| `game-settings` | `screens/game-settings.slint` | `GameSettingsScreen` |

`shell.slint` exports `Shell`, which inherits `Window`. The host checks only that each file
exists and exports its name. A missing file stops the load with `ThemeError::File`, which names
the file: `screens/downloads.slint: is missing: a theme ships its shell and every screen`.

## Where the player is

The core owns the navigation: a stack of the screens the player came through, with Home at the
bottom. `Vm.screen` is the screen on top. A theme keeps no screen of its own. It sends
`Navigate` or `Back` and shows the screen that comes back.

- `Navigate` to `"Home"` goes back to the bottom of the stack: Back has nowhere to go.
- `Navigate` to a screen that is already on the stack goes back to that screen, as the player
  left it. Any other screen goes on top.
- `Back` closes the screen on top and shows the one under it.
- Each screen on the stack keeps its own selected game. Home's is `Vm.home.selected`. The
  Library's is `Vm.home.library.selected`: until the Library opens, it is Home's. The Catalog's
  is in `Vm.catalog`. `Select` selects a game on the screen on top. `OpenGame` also selects the
  game on the screen under its Game Hub, so Back comes back to the game.
- `Vm.nav-direction` says which way the last move went, `forward` or `back`.
- A running game is not a screen. `Vm.has-running` and `Vm.running` describe it, and the shell
  draws it over the screen, for example as a launch splash. Starting or ending a game doesn't
  change `Vm.screen`.

Focus inside a screen, such as which button has the keys, is the theme's.

## The theme section

The `[theme]` section of `theme.toml` is `ThemeSection` in `crates/ui-api/src/theme.rs`. It
refuses keys it doesn't know.

| Key | Type | Default | What it does |
| --- | --- | --- | --- |
| `ui_api` | version range | required | The contract versions the theme works with. A theme that doesn't match fails with `ThemeError::UiApi`. |
| `fallback` | bool | `false` | This theme is the fallback theme. Exactly one installed theme sets it, and Default does. |
| `input` | list | `["pointer"]` | The input devices the theme supports, the main one first: `pointer`, `gamepad`, `touch`. The first one is `Vm.input-mode` at start. |
| `navigation` | `pointer` or `focus` | `pointer` | How the player moves around. No host code reads it yet. |
| `tokens` | table | none | Values for [tokens](#tokens). A token the theme leaves out keeps its default. |
| `breakpoints` | array of tables | none | Named window sizes. See [Breakpoints](#breakpoints-and-the-size-of-everything). |
| `assets` | table | none | `images`, `fonts`, `sounds`. See [Images, fonts, sounds](#images-fonts-sounds-and-system-art). |
| `options` | table | none | Settings the player changes, which set tokens. See [Options](#options). |
| `systems` | table | none | A system's own icon and cover, by system id. See [System art](#images-fonts-sounds-and-system-art). |

The host refuses three keys, each with what to do instead (`ThemeManifest::from_plugin`):

| Key | Message |
| --- | --- |
| `extends` | `` `extends` is not supported: a theme stands alone; import the shared parts from kit/ instead of extending a theme `` |
| `screens` | `` `screens` is not supported: a theme ships every screen, so it doesn't list them; remove `screens` `` |
| `components` | `` `components` is not supported: a theme replaces a kit file by shipping kit/<name>.slint; remove `components` `` |

A theme with one of them is left out of the Themes list. The log has the message under
`theme skipped`.

## The files the host serves

The host serves five files at the root of every theme (`RESERVED_FILES` in
`crates/ui-api/src/contract.rs`). A theme imports them and never ships them: a theme that ships
one fails with `is served by the host; remove it`.

| File | Global | What it holds |
| --- | --- | --- |
| `types.slint` | `Vm`, `Host`, `Json` and the structs | The view model, the callbacks, and helpers to build command arguments. The source is `crates/ui-api/contract/types.slint`. |
| `tokens.slint` | `Tokens` | One property per token. It also imports the theme's fonts. |
| `assets.slint` | `Assets` | One `image` per name in `[theme.assets.images]`, and `loaded`. |
| `options.slint` | `Options` | One property per option: a `string` for a choice option, a `color` for a color option. |
| `brand.slint` | `Brand` | The product's names from `brand.toml`: `name`, `os-name`, `name-caps` and `env-prefix`. Text that names the product reads them, as in `"Quit " + Brand.name`. |

Import paths are relative to the importing file. The shell imports `"types.slint"`. A screen
imports `"../types.slint"`.

## The view model: Vm

`Vm` holds everything the theme shows. The host sets it after every change, and a theme only
reads it. It is the core's `AppView` (`crates/ui-api/src/vm.rs`) field for field, with
kebab-case names. The comments in `crates/ui-api/contract/types.slint` say what each field
and struct holds. Read that file as the reference.

| Topic | `Vm` properties |
| --- | --- |
| Where the player is | `screen`, `nav-direction`, `game`, `can-go-back` |
| Home and the Library | `home` (with `home.library`), `has-detail`, `detail` |
| The Catalog | `catalog`, `catalog-sources`, `catalog-releases`, `catalog-updated`, `catalog-loading`, `catalog-available`, `catalog-actions`, `catalog-status` |
| Downloads | `downloads`, `torrents`, `seed-downloads`, `torrents-downloading`, `torrents-sharing` |
| Settings | `vpn`, `backup`, `settings`, `theme-options`, `themes`, `folders`, `sounds`, `animations`, `fullscreen`, `return-on-exit`, `auto-update`, `auto-install`, `scale`, `can-start-at-login`, `start-at-login`, `start-at-login-error`, `hardware`, `launcher-update`, `system`, `monitors`, `monitor` |
| Plugins | `emulators`, `plugins`, `emulator-plugins`, `theme-plugins`, `has-plugin-page`, `plugin-page` |
| Trailers | `can-play-trailers`, `has-trailer`, `trailer`, `trailer-frame` |
| Prompts | `has-install-prompt`, `install-prompt`, `has-result-prompt`, `result-prompt`, `has-question`, `question`, `ratings`, `browser` (the folder browser) |
| Start | `boot`, `art-ready` |
| Status | `task`, `clock`, `has-running`, `running`, `closing`, `has-closing-game`, `closing-game`, `notices`, `notice-cards` |
| Window and input | `viewport`, `input-mode`, `pad-key`, `power-menu`, `pad-name` |

An optional part comes with a `has-` flag: read `detail` only when `has-detail` is true. Text
for the player, such as sizes, dates, a download's state and a release's title, comes ready to
show. It is display text, which the core builds in `display.rs`.

## Host: what to ask for

A theme changes nothing itself. It asks the host through the `Host` global in `types.slint`.

`Host.command(name, args)` sends a core `Command` (`crates/core/src/app.rs`). `name` is the
variant's name. `args` is its payload as JSON text, or `""` for a variant with no payload. The
host builds the value from the two (`command` in `crates/ui-host/src/host.rs`):

- With empty `args`, the value is the bare name: `Host.command("Back", "")` reads as `"Back"`.
- With `args`, the value is `{ name: args }`: `Host.command("Select", "\"emu:astro\"")` reads as
  `{"Select": "emu:astro"}`.

A command whose arguments aren't JSON, or don't fit the variant, is dropped. The log says
`the theme sent a command that doesn't read`. The theme gets no error.

Build the arguments with the `Json` global. `Json.text(s)` makes a JSON string with its
escapes. `Json.text-or-null(s)` gives `null` for `""`. `Json.flag(b)` gives `true` or `false`.

```slint
Host.command("Back", "");
Host.command("Select", Json.text(game.key));
Host.command("Session", "{\"Launch\":" + Json.text(game.key) + "}");
```

Use `kit/commands.slint` instead of writing the JSON. Its `Commands` global has one function
for each thing the player asks, such as `Commands.back()`, `Commands.launch(key)` and
`Commands.set-sounds(on)`. A test in `host.rs` calls each of its functions and fails when one
sends a command the core doesn't read, so its arguments are a working example.

### The commands

A variant with a sub-enum takes that enum, written as JSON the same way. The doc comments on
`Command` in `app.rs` say more about each variant.

| Variant | Payload |
| --- | --- |
| `Navigate` | A `Route`: `"Home"`, `"Library"`, `"Catalog"`, `"Downloads"`, `"Settings"`, `"Emulators"`, `"Themes"`, `{"GameDetail": key}`, `{"Plugin": id}`, `{"GameSettings": key}`. See [Where the player is](#where-the-player-is). |
| `Back` | None. |
| `Select`, `OpenGame`, `UninstallGame`, `AskResult` | A game key. |
| `Library` | `"Refresh"`, `{"Hide": {"game": key, "hidden": bool}}` (every version of the game), or `{"HideVersion": {"game": key, "hidden": bool}}` (that version alone). |
| `LibrarySearch`, `CatalogSearch` | Text. |
| `LibrarySystem`, `CatalogSystem` | A system id (`PS5`) or a grid filter (`ingame`, `installed`, `hidden`), or `null` to clear them. |
| `LibraryGenre`, `CatalogGenre` | A genre, or `null` for every genre. |
| `LibrarySort`, `CatalogSort` | `title`, `played`, `size`, `smallest`, `rating`, `release`, `compat` or `added`. |
| `LibraryMore`, `CatalogMore` | None. Shows the next page. |
| `RememberSearch` | Text. Keeps a recent search (`Library.recent`). Send it when a search is done, not at each key. |
| `ForgetSearches` | None. Forgets the recent searches. |
| `GridColumns` | `{"grid": "Library" or "Catalog", "columns": n}`. |
| `Session` | `{"Launch": key}`, `{"Stop": session}`, `{"ShowGame": session}`, `"ToggleFocus"`. |
| `ReportResult` | `"ingame"`, `"menus"`, `"boots"`, `"nothing"`, or `null` for Not now. |
| `Ratings` | Send ratings: `"Open"`, `{"Pick": key}` (picks a rating or leaves it out), `"Next"` (shows the first picked report), `"Send"` (opens the report on screen in the browser), `"Skip"` (sends nothing), `"Close"`. |
| `ConfirmInstall`, `CancelInstall` | None. |
| `AnswerQuestion` | `true` or `false`: the player's answer to a plugin's question at the start. |
| `Torrent` | `{"Download": {"offer": key, "release": id, "folder": path}}` (`folder` optional), `{"Act": {"id": id, "action": action}}`, `{"Pick": {"id": id, "file": n, "on": bool}}` (`file` `null` for every file), `{"Begin": {"id": id, "install": bool, "folder": path}}` (`install` and `folder` optional), `{"SeedDownloads": bool}`. See [Torrent actions](#torrent-actions). |
| `GameInstall` | `{"Download": {"id": id, "folder": path}}` (`folder` optional), `{"Package": {"plugin": id, "source": {"folder": name, "root": n, "path": path}}}` (a `HostPath`), `{"RemoveUnpacked": {"id": id}}`, `{"Cancel": {"id": id}}`. |
| `Vpn` | `{"SetOnly": bool}`, `{"Choose": name}`, `{"Connect": name}`, `{"Disconnect": name}`, `{"Import": path}`. |
| `Backup` | `"Refresh"`, `{"BackUp": {"large": bool}}`, `{"Restore": file}`, `{"RestoreFile": path}`, `{"Delete": file}`, `{"Browse": path or null}`, `"CloseBrowser"`, `{"ShowHidden": bool}`, `{"Filter": text}`. `Browse`, `CloseBrowser`, `ShowHidden` and `Filter` drive the folder browser for a restore, as `Browse` does. |
| `Browse` | The folder browser: `{"Open": {"at": path or null, "purpose": "Restore", {"Pick": folder} or "BackupFolder"}}`, `{"Go": path}`, `"Close"`, `{"ShowHidden": bool}`, `{"Filter": text}`, `{"Mount": drive}` (mounts a drive from the places, then opens it), `"Use"` (a pick: adds the folder shown to the named folder, or makes it the backup folder, and closes). |
| `Drive` | The drives, by a drive's id (`DriveRow.drive`): `"Refresh"`, `{"Mount": id}`, `{"Unmount": id}`, `{"Eject": id}`. Unmount and Eject are refused while a game runs from the drive, or a download or an install uses it; Eject only for a drive the player can take out. |
| `Notice` | `"DismissAll"`, `{"Dismiss": id}`, `{"Open": id}`. |
| `LauncherUpdate` | `"Check"`, `"Download"`. |
| `System` | `SystemCommand` (`crates/core/src/system.rs`): `"Refresh"`, `"ScanWifi"`, `{"JoinWifi": {"ssid": name, "password": text or null}}`, `{"ForgetWifi": name}`, `{"SetBluetooth": bool}`, `"ScanBluetooth"`, `{"Pair": address}`, `{"ConnectDevice": address}`, `{"DisconnectDevice": address}`, `{"RemoveDevice": address}`, `{"SetOutput": name}`, `{"SetVolume": 0-100}`, `{"SetMuted": bool}`, `"Restart"`, `"ShutDown"`, `"LogOut"`, `"CheckOsUpdate"`, `{"InstallOs": path}`, `"ChoosePartitions"`, `{"CancelJob": id}`. |
| `Pref` | A pair, such as `["sounds", false]`. The key is a `Pref` in snake case (`crates/core/src/preferences.rs`). The player sets `sounds`, `animations`, `fullscreen`, `return_on_exit`, `scale`, `start_at_login`, `auto_update`, `auto_install`, `monitor`, `welcomed` and `backup_folder`. The core sets the others. |
| `SetThemeOption` | `{"key": option, "value": text}`. |
| `SetConfig` | `{"plugin": id, "scope": scope, "value": json}`. A scope is `{"type": "emulator"}` or `{"type": "game", "game": text}`. Pass a settings panel's `scope` back unchanged. |
| `SetConfigField` | `{"plugin": id, "scope": scope, "key": field, "value": text}`. |
| `RunAction` | `{"plugin": id, "action": name, "game": key or null, "args": json}`. |
| `SetPluginEnabled` | `{"plugin": id, "enabled": bool}`. A theme stays on. |
| `SetFolders` | `{"name": folder, "paths": text}`, the folders separated by `;`, or a list of paths. |
| `AddFolder`, `RemoveFolder` | `{"name": folder, "path": path}`: adds a folder to the folders a named folder means (the default ones when the player chose none), or takes one out. The folder and its files stay. |
| `Download` | `{"plugin": id, "request": {"type": "emulator_build", "version": text or null}}`. |
| `PauseDownload`, `ResumeDownload`, `CancelDownload` | A download's id (a number). |
| `InstallEmulator` | `{"plugin": id, "version": text or null}`. |
| `UpdateEmulator` | `{"plugin": id}`. |
| `UninstallEmulator` | `{"plugin": id, "version": text}`. |
| `CheckUpdates` | None. |
| `ThemeSwitched` | The host sends it. A theme asks for a switch with `Host.switch-theme`. |

### The other callbacks

The other `Host` callbacks never reach the core. They go to the app shell (`UiRequest` in
`crates/ui-host/src/host.rs`).

| Callback | What it does |
| --- | --- |
| `switch-theme(id)` | Switches to the theme `id`. |
| `play-sound(name)` | Plays a sound from `[theme.assets.sounds]`. |
| `open-url(url)` | Opens a page in the system's browser. Only `https://` addresses; the host drops others. |
| `open-game-folder(key)`, `open-game-log(key)` | Opens a game's folder, or its emulator log. The theme names the game, never a path. |
| `open-download-folder(id)` | Opens a download's folder, once it is on disk. |
| `play-trailer(url)`, `pause-trailer()`, `seek-trailer(seconds)`, `close-trailer()` | The trailer player. `play-trailer` takes `https://` only. `seek-trailer` goes back with a negative number. |
| `grid-window(grid, first, last)` | Says which cards of a grid (`"library"` or `"catalog"`) are on screen. Only the covers near them load. |
| `quit()`, `restart()`, `log-out()` | Closes the launcher, starts it again, or ends the launcher's session. |

A theme with a grid calls `grid-window` when the visible cards change. Until it calls, the host
loads the covers near cards 0 to 29 (`Window::START` in `crates/ui-host/src/images.rs`).

## Torrent actions

Each `Torrent` in `Vm.torrents` has `actions`, a list of names in the order to show them. The
names are `start`, `pause`, `play`, `install`, `stop-seeding`, `cancel`, `remove`,
`remove-unpacked`, `cancel-install`, and `open-store` for an install that a store does.

The theme turns an action into a command:

| Action | Command |
| --- | --- |
| `start`, `pause`, `cancel`, `remove`, `stop-seeding`, `open-store` | `Torrent`, `{"Act": {"id": id, "action": Name}}`. `Name` is the action in Pascal case: `StopSeeding`, `OpenStore`. |
| `install` | `GameInstall`, `{"Download": {"id": id}}`. |
| `play` | `Session`, `{"Launch": game}`, with the torrent's `game`. |
| `remove-unpacked` | `GameInstall`, `{"RemoveUnpacked": {"id": id}}`. |
| `cancel-install` | `GameInstall`, `{"Cancel": {"id": id}}`. |

`start` on a `ready` torrent belongs to the download sheet: the kit opens the sheet first. The
sheet shows `picks` (each file, its `kind`, `on` and `locked`), `picked`, `download-free`,
`install-free`, `download-room` and `install-room`.

- `Commands.pick-file(id, n, on)` sends `Pick`. `n` is -1 for every file.
- `Commands.begin-download(id, install, folder)` sends `Begin`. With `install`, the game
  installs into `folder` once the download finishes, and the torrent's `install-after` is true.
- A `Begin` that the core refuses leaves the torrent `ready`, with the reason in
  `install-error`.

The core never lists `delete-files`. A theme sends it (`"DeleteFiles"`) when the player removes
a download with its files. `Commands.torrent-action(id, action)` in `kit/commands.slint` does
all of this, and `play` is `Commands.launch(game)`.

## Slots

A slot item is something a plugin adds to a screen. It reaches a theme as a `Slot` struct. How a
plugin fills a slot is in [the plugin reference](../plugins/reference.md).

| Where | `kind` | Draw it as |
| --- | --- | --- |
| `GameCard.badges`, `GameDetail.badges` | `badge` | A small tag. `short` is its text for small places. |
| `GameDetail.actions` | `action` | A button. |
| `GameDetail.sections` | `section` | A block of `rows`. |
| `PluginPage.blocks` | `block` | A titled list of rows on the plugin's page, after its build. |
| `SettingsPanel.slot` | `settings_panel` | A form of `fields`. |

`tone` is `neutral`, `info`, `success`, `warning` or `danger`. `style` is `primary` or
`secondary`. To run an action, send `RunAction` with the slot's `plugin`, `action` and `args`
and the game's key: `Commands.run-action(plugin, action, game, args)`. `args` and `scope` are
JSON text: pass them back unchanged.

Set `accessible-role: text` and `accessible-label: <slot>.label` on each slot item you draw. The
label is `<plugin-id>:<item-id>`. Tests find items by it. The kit's `SlotBadges`, `SlotActions`
and `SlotSections` do it.

## The kit

The kit is the shared Slint files the host serves under `kit/`, beneath the theme's own files.
The list is `KIT` in `crates/ui-api/src/contract.rs`. The files are in `crates/ui-api/kit/`.
Import a kit file by its path: `"kit/button.slint"` from the shell, `"../kit/button.slint"` from
a screen or a component.

Tests in `contract.rs` hold every kit file to three rules:

- It draws only from tokens. No hex color is in a kit file.
- It imports only the host's served files, Slint's `std-widgets.slint` and other kit files.
- It passes the same rules as a theme's files (see [Rules the host checks](#rules-the-host-checks)).

| File | Exports | What it gives |
| --- | --- | --- |
| `button.slint` | `Button` | A small round button. Primary and the current choice are white; others light up on hover and focus. |
| `closing.slint` | `Closing`, `ClosingScreen` | Covers the screen while Log out, Restart or Shut down closes what runs. It takes every key: hide your key hints while `Closing.shows`. |
| `commands.slint` | `Commands` | One function for each thing the player asks of the core. |
| `cover-wall.slint` | `CoverWall` | A slowly drifting wall of the library's covers, dimmed, as the boot and closing screens show it. |
| `downloads.slint` | `Dl`, `DownloadsPage` | The Downloads screen, its keys, the download sheet, and the install and remove sheets. |
| `folder-browser.slint` | `Browser`, `FolderBrowser` | The folder browser, for a restore or to pick a folder: places (a drive that isn't mounted mounts from here), folders and backups, details, its own keys, "Use this folder" when it picks, and one dialog before a restore. |
| `folder-list.slint` | `FolderList`, `FolderListSheet` | A named folder's list, which a `FolderRow` opens: each folder with the free space on its drive and Remove, then Add folder (the folder browser) and Type a path. With `FolderList.show-backup()`, the backup folder's: Choose a folder, Type a path, Use the default folder. |
| `game-detail.slint` | `Hub`, `GameDetailPage` | The Game Hub: buttons, releases, fact cards, technical details and screenshots. |
| `game-menu.slint` | `GameMenu`, `MenuRow`, `GameMenuSheet` | A game's menu, with Uninstall asking first. |
| `game-settings.slint` | `GameSettingsPage` | One game's own settings, on the same frame as Settings (`SettingsFrame`). |
| `game-stage.slint` | `GameStage` | A game on the whole screen, as its launch and its closing show it: its art, its cover, its title and a turning ring. |
| `games.slint` | `Games` | The games on screen and the theme's own selection of one. `select` moves it at once, `ShellKeys` sends it to the core `Look.select-after` after the last move, and `select-now` selects at once, for a click. Also the search, filters, sort and next page of the grid on screen, and a card's eye (`hide`) and main action (`act`). |
| `icon.slint` | `Icon` | The launcher's icons, drawn as paths. |
| `install-os.slint` | `Setup`, `InstallOs` | The OS's USB stick: welcome, the disks, the question before erasing, progress. |
| `install-prompt.slint` | `Prompt`, `InstallPrompt` | The prompt to install an emulator before a first play. |
| `keyboard.slint` | `Osk`, `TextField`, `OnScreenKeyboard` | A text field, and the on-screen keyboard it opens for a controller. A field with `picking` shows a row of picks over the keys, such as a search's matches or recent searches. |
| `lingering.slint` | `Lingering` | Keeps a screen or a sheet on screen while it goes out. |
| `list-item.slint` | `ListItem` | A row: a label, a line under it, a value at the end. |
| `loading.slint` | `LoadingLine`, `LoadingSpinner`, `LoadingRow`, `StatusSlot`, `StatusLoading`, `StatusState` | What shows while a system area is read for the first time: a status icon's thin line and its "Checking…" label, a panel's loading row, and the place that keeps a status icon in the top bar and closes, with its gap, when the icon goes. `StatusState` shows an area's state over its status icon: loading, or disabled with its reason. |
| `look.slint` | `Look` | Shared sizes (`k`, `px`, `t`, `tpx`), `rtl`, colors made from tokens, `timed`, `motion-on`, `clock`, the kinds of motion, and words and colors for ratings, results, sorts and states. |
| `motion.slint` | `Motion`, `Leaving`, `Popover`, `Sheet`, `Tooltip` | The kit's shared motion: its times, a part kept on screen while it fades out, a popover that grows from its anchor, a sheet whose card slides up over a fading wash, and a label that fades in on key focus or after a short hover. |
| `notices.slint` | `Notices`, `NoticeStack` | The notices on screen. Delete closes the newest. |
| `origin.slint` | `Origin` | The card the Game Hub opens from, for its cover to grow out of it. |
| `plugin-list.slint` | `PluginList`, `PluginListSheet` | The Emulators or Themes list as a side sheet, and its keys. |
| `plugin.slint` | `PluginPage` | One plugin's page, on `SettingsFrame`. For a theme, "Use this theme" and its options. |
| `pop.slint` | `Pop` | A pop-up: a wash over the screen and a card in its middle. |
| `power-ask.slint` | `PowerAsk`, `PowerAskSheet` | On a desktop, the dialog before Restart or Shut down: the focus starts on Cancel. Esc, the pad's ○ and a click outside cancel. `Overlays` shows it and `ShellKeys` sends it its keys. |
| `power-menu.slint` | `PowerMenu`, `PowerMenuSheet` | The power menu: Log out, restart the launcher, Restart, Shut down, or quit. |
| `progress-bar.slint` | `ProgressBar` | A thin progress bar. |
| `question-prompt.slint` | `Question`, `QuestionPrompt` | A plugin's question at the start, with its two buttons. |
| `result-prompt.slint` | `Ask`, `ResultPrompt` | "How far did it get?" after a game closes. |
| `reveal.slint` | `Reveal` | How far something has come in, as `move` and `fade` from 0 to 1. It comes in after its first frame and goes back out when `open` turns false. |
| `send-ratings.slint` | `Sending`, `SendRatings` | Send ratings: the ratings that wait, then each picked report in full. |
| `settings-frame.slint` | `Sections`, `PageScroll`, `SectionHead`, `PageSection`, `SettingsFrame` | The frame that Settings, a plugin's page and a game's own settings share: the header bar, the rail of section icons when there are two sections or more, and the page of sections. |
| `settings-layout.slint` | `LauncherRows`, `DownloadsRows`, `SystemRows`, `BackupRows`, `PowerRows` | Where each row of a Settings section is, so the keys know where to stop. |
| `settings-page.slint` | `SettingsPage` | Settings as a side sheet on `SettingsFrame`, with the rail that lights the focused section. Set `logo` to your mark for the About card, the last thing on the page, under Power: the built-in themes give it `Assets.logo-mark`. |
| `settings-rows.slint` | `Prefs`, `Tab`, `RailTab`, `Header`, `Row`, `Value`, `Editor`, `FieldRow`, `FolderRow` | The rows of a settings sheet, and where the keys are in it. The kit's sheets use the rail (`RailTab`); `Tab` stays for themes that draw tabs. |
| `settings-sections.slint` | `GamesSection`, `UpdatesSection`, `LauncherSection`, `DownloadsSection`, `PcSection`, `SystemSection`, `BackupSection`, `PowerSection` | The rows of each Settings section. |
| `sheet-frame.slint` | `SheetFrame` | A side sheet: the wash behind and the panel that slides in. A click on the wash goes back. |
| `shell.slint` | `Hint`, `ShellHints`, `Screens`, `ShellKeys`, `Overlays` | The shell's keys (`ShellKeys`), the key hints (`ShellHints.pick`), what opens over the screens (`Overlays`), which kind of screen is up (`Screens.home`, `.list`, `.sheet`), and the side a screen moves on (`Screens.side(leaving)`). |
| `shot-viewer.slint` | `Viewer`, `ShotViewer` | A game's screenshots, full screen. |
| `slot-actions.slint` | `SlotActions` | Plugin actions as round buttons. |
| `slot-badges.slint` | `SlotBadges` | Plugin badges as small tags with a dot in their tone. |
| `slot-sections.slint` | `SlotSections` | Plugin sections as fact cards. |
| `status-bar.slint` | `StatusBar`, `StatusButton` | The machine's status icons in one row, the same in every theme: the network, sound, Bluetooth and drives icons, then the battery. Put it in your top bar before Settings, and give it `focused`, the icon the keys are on (0 to `Sys.statuses` - 1, or -1). |
| `system-panel.slint` | `Sys`, `NetIcon`, `SoundIcon`, `BluetoothIcon`, `StorageIcon`, `BatteryIcon`, `SystemSheet` | The network, sound, Bluetooth and drives icons and their panels, wherever the launcher runs, and the battery. `Sys.statuses` is how many status icons there are (4). `Sys.toggle` and `Sys.show-status` open a status icon's panel, and open nothing for a disabled area. `Sys.loading`, `Sys.disabled` and `Sys.reason` read a panel's area. A notice with a `drive` opens the Drives panel. |
| `trailer-player.slint` | `Trailers`, `TrailerPlayer` | A trailer in the window, with its keys. |
| `widgets.slint` | `FocusRing`, `KeyCap`, `BackButton`, `Stars`, `Chip`, `Pill`, `Kicker`, `Switch`, `Shimmer`, `Cover` | Small widgets. |

### What a shell does for the kit

The kit's pages and sheets have no keys of their own. Each one has a global with a `key`
function, and the shell sends it the keys. `ShellKeys` (`kit/shell.slint`) does this for every
theme: put your screens inside it and give it the focus (`forward-focus`).

`ShellKeys` answers the keys in this order and stops at the first that takes the key:

1. Before any element sees the key: the closing screen takes every key. Then the folder browser
   (`Browser.key`), a folder's list (`FolderList.key`), Send ratings (`Sending.key`) and the
   on-screen keyboard (`Osk.key`), while each is open. Ctrl+P, for the power menu, still gets
   past the first three.
2. A held Enter does nothing: it would play, download or install a second time.
3. The power menu (`PowerMenu.key`), and Ctrl+P to open it.
4. A plugin's question at the start (`Question.key`), the Welcome screen, and the boot screen
   while the start's installs run.
5. The OS installer (`Setup.key`) and a machine panel (`Sys.key`).
6. The launch splash, while it lists the game's keys (`splash-keys`): it takes every key but
   Escape and Ctrl+Q.
7. Delete closes the newest notice.
8. The trailer (`Trailers.key`), the result prompt (`Ask.key`), the game's menu
   (`GameMenu.key`), the screenshot viewer (`Viewer.key`), and your list of sorts (`sort-key`
   while `sort-open`).
9. Menu opens the game's menu while `game-keys` is set.
10. The install prompt (`Prompt.key`).
11. Ctrl+Q quits. Ctrl+D opens Downloads.
12. Your `theme-key`: a search key, a trailer key, your own top bar.
13. L1 and R1 (Shift+Tab and Tab): on a sheet with a rail, the section before or after; else
    the screen before or after, in the order Home, Catalog, Downloads, Emulators, Themes,
    Settings.
14. Your `screen-key`: your own screens, such as a row on Home.
15. The kit's pages: `Prefs.key` on a settings sheet, `PluginList.key` on Emulators and Themes,
    `Hub.key` on the Game Hub, `Dl.key` on Downloads.
16. Escape or Backspace goes back, with the `back` sound except on Home.

`ShellKeys` also runs the timer that sends a move among the games to the core (`Games.send`),
and moves the cursor back to the core's selection when that changes for another reason
(`Games.core-moved`).

When `Vm.screen` changes, `ShellKeys` starts the kit's pages afresh: the Game Hub on its first
button (`Hub.reset`), Downloads on its first row (`Dl.reset`), a settings sheet on its first row
(`Prefs.reset`), and the game menu and the screenshot viewer closed. It then calls
`screen-changed`, where your theme sets its own focus for the new screen. To open the Game Hub
on its releases, set `Hub.to-releases` before you open it.
`plugins/themes/default/theme/shell.slint` shows each callback in use.

The shell also does these things:

- Put `Overlays` over your screens, the size of the window, with `logo` set to your emblem. It
  shows the prompts, the game menu, the trailer, the screenshot viewer, a folder's list, the
  folder browser, the
  OS installer, the power menu and the closing screen while each is open.
- Put `QuestionPrompt` over your boot screen while `Vm.has-question`. The start waits for the
  answer, and `ShellKeys` gives it every key. A theme without it leaves the player unable to
  answer.
- Keep a screen on screen while it goes out with a `Lingering` (`open: Vm.screen ==
  "library"`): show it while `shown`, and pass it `open`. The kit's Game Hub and Downloads pages
  fade out while `open` is false. Put an empty `TouchArea` over a screen that is going, so a
  click on it does nothing.
- Read the side a screen moves on with `Screens.side(leaving)` when the screen opens and when it
  starts to leave, and keep it for that move. Back then mirrors a forward move.
- Set `Look.window-height` from the window's height, so the first frame has the right size.

`tests/fixtures/themes/fixture-grid/shell.slint` is a shell with a layout of its own that does
no more than this.

### Replacing a kit file

Ship a file at the same path, for example `kit/commands.slint`. The host lays the kit down
first, then the theme's files over it, so the theme's copy wins (`ResolvedTheme::slint_sources`
in `crates/ui-host/src/themes.rs`). Every file that imports it, the kit's own included, gets the
theme's copy.

- The host checks the theme's copy as a theme file, not as a kit file, so it may use hex colors.
- Keep its exports. A kit file that imports it still needs them, or the theme doesn't compile.

A kit file that gains a member another kit file uses is a breaking change for a theme with its
own copy, so the contract's minor version goes up. For example, version 0.5.0 added
`Hub.reset()` and `Hub.to-releases` to `kit/game-detail.slint`, which the kit's `ShellKeys`
and `Games` use. A theme with its own copy of that file adds both, then sets
`ui_api = "^0.5"`.

Version 0.5.1 added `line-text` and `not-here-text` to `Release`. It only adds fields, so a
theme on `^0.5.0` still loads.

Version 0.6.0 moved the backup browser out of Settings → Backup: it is the folder browser now.

- `Vm.backup.browser` is `Vm.browser`, of the struct `FolderBrowser` (was `BackupBrowser`), with
  `purpose`, `folder` and `folder-title`. `BrowsePlace` has `drive` and `mounted`.
- `kit/backup-browser.slint` is `kit/folder-browser.slint`, and its component `BackupBrowser`
  is `FolderBrowser`. Its accessible label is `folder-browser`.
- `FolderBrowser` has `title`. `Backup` has `free`, the free space on the backup folder's drive.
- `Folder` has `list`, each a `FolderPath`. `FolderRow` takes `at` and `on-page`, and opens the
  folder's list (`kit/folder-list.slint`) instead of a text box.
- `System` has `drives-available` and `drives` (each a `DriveItem`, one per
  drive, with its `actions` as buttons and the `busy` one). `Notice`
  has `drive`. A job's kind can be `mount`, `unmount` or `eject`.
- A theme with its own top bar shows the drives' icon, and counts it among its machine icons
  so the keys reach it.
- `drives-listing` is true until the drives' first list read ends. `FolderBrowser` has
  `places-loading`, true while the browser reads the drives for its places.

A theme built for 0.5 that reads `Vm.backup.browser` or imports `kit/backup-browser.slint`
changes those names, then sets `ui_api = "^0.6"`.

Version 0.7.0 put every system area in one list
([ADR-0004](../adr/0004-every-system-area-has-one-interface.md)):

- `System` has `areas`, each an `AreaItem`: its `name`, its `state` (`loading`, `absent`,
  `unsupported` or `ready`), its `reason` and whether it is `busy`. The list always holds every
  area, in the order of the `Areas` global in `types.slint`: read one as
  `Vm.system.areas[Areas.bluetooth]`.
- `network-loading`, `audio-loading`, `bluetooth-loading`, `power-loading`, `drives-loading`
  and their `*-reason` are gone: read the area's `state` and `reason`. `network-available`,
  `audio-available`, `bluetooth-available`, `drives-available` and `power-available` are gone
  too: an area works here when its `state` is `ready`. Restart and shut down work when the
  power area is ready and its `reason` is empty.
- An area that can't be used here (absent or unsupported) no longer hides its icon: the icon
  shows disabled (see [Status icons](#status-icons)).
- The machine's controls show wherever the launcher runs, on a desktop too: the network, sound,
  Bluetooth and drives icons, Settings → System, and Restart and Shut down. Whether a control
  works comes from its area's state only. `whole-screen` is a fact of the session now: Quit
  restarts the launcher, the game menu opens nothing outside it, and Restart and Shut down ask
  by a second press. On a desktop they ask in a dialog (`kit/power-ask.slint`).
- The kit draws the status icons: `StatusBar` in `kit/status-bar.slint`. Both built-in themes
  use it, and their `Nav.statuses` is `Sys.statuses`, 4.
- Settings' tabs come from one list, `Prefs.settings-tabs`, the same in every place. System is
  always section 5, Backup 6 and Power 7 (`Prefs.backup-section`, `Prefs.power-section`).
  `Prefs.sections` is 8 in Settings.

A theme built for 0.6 that reads `Vm.system.*-loading`, `*-reason` or `*-available` reads
`Vm.system.areas` instead, then sets `ui_api = "^0.7"`.

#### Status icons

The kit's `StatusBar` (`kit/status-bar.slint`) draws every status icon in the same order in every
theme, wherever the launcher runs. A theme with a top bar of its own puts it before its Settings
button. Every status icon is in place from the first frame, so the top bar never shifts.
The kit's `StatusState` (`kit/loading.slint`) shows an area's state over its icon. Put it in the
icon's button with the area's place, and read its `dim`, `cursor`, `disabled` and `lit`:

- **Loading:** while its area loads, the icon dims, and a 2 px line (`LoadingLine`) sweeps under
  it. With Animations off the line is dotted and stands still. Its accessible value is `loading`,
  and "Checking…" shows under it on hover or key focus (`StatusLoading`). The battery shows an
  empty, dim battery with "--%" and the same line. A loading icon can be focused and clicked. Its
  panel opens at once on a loading row (`LoadingRow`, accessible label `loading`: "Reading
  drives…", "Reading the network…") that the rows replace when the read ends. The folder
  browser's places show "Reading drives…" under them while `places-loading`.
- **Disabled:** an area that is absent or unsupported keeps its icon in place, dimmer, with the
  plain pointer (`cursor`) in place of the hand and no hover fill. A click, Enter or the pad's ✕
  opens nothing. The icon still takes the keys' and the pad's focus and the pointer's hover, and
  then its reason shows under it on one line, on a chip that stays inside the window (`TipChip`
  in `kit/motion.slint`, accessible label `status-reason`). Its focus is a ring that `StatusState`
  draws round the dim icon: give the solid focus fill, and the glyph's colour on it, only while
  `lit` (focused and not disabled). The built-in themes give it the accessible
  value `disabled`, `accessible-enabled: false`, and the reason as its description. Left and
  right in an open panel go past a disabled one.
- **An icon that goes:** only what isn't there at all closes its place: a PC with no battery
  fades the battery out, and its place (`StatusSlot`) closes with its gap, in the motion times.
  Put each icon in a `StatusSlot` with the gap after it, and keep the slot while it fades out
  with `Leaving`.

### Motion

The kit's parts move through shared pieces in `kit/motion.slint`, so every theme gets the same
motion, and **Animations off** (the motion tokens at 0) stops all of it: each part is then in
place at its first frame.

| Piece | Used by | Motion | Time (`Motion`) |
| --- | --- | --- | --- |
| `Popover` | the system panels and the Drives panel | grows from 0.96 at its anchor's side and fades in; fades out | `popover-in` 120 ms, `popover-out` 90 ms |
| `Sheet` | the folder browser, a folder's list, the dialog before Restart or Shut down | the wash fades in, the card slides up 16 px and fades in; both fade out | `sheet-in` 150 ms, `sheet-out` 110 ms |
| `Leaving` | the shells and `Overlays` | keeps a part on screen while it fades out | the part's out time |
| `Tooltip` | the Drives panel's buttons | fades in at once on key focus, after `tip-wait` under the pointer | `tip-in` 80 ms, `tip-wait` 250 ms, `tip-out` 60 ms |
| Settings' rail | the lit square and the focus ring | one square moves to the section | `focus` 90 ms |
| Rows and buttons | the browser, a folder's list, the Drives panel | the focus and hover tint ease; a press gives to 0.97 | `focus` 90 ms, `press` 70 ms |
| Bars | the Drives panel's usage bars, the volume | the width eases | `bar` 120 ms |
| Lists | the browser's folders, the drives | each row comes in a `Look.stagger` after the one before, at most `stagger-cap` (6) steps | `Look` |

Enter eases out strongly (`Motion.enter`, `cubic-bezier(0.2, 0, 0, 1)`); exit eases in. Motion never holds input: a part that goes takes no key or click, and the keys act on what
opened at once.

## Tokens

A token is a named design value. Slint reads it as `Tokens.<name>`. The list and the defaults
are `TOKENS` in `crates/ui-api/src/tokens.rs`. The defaults are the Classic look. A theme sets
only the tokens it changes, in `[theme.tokens]`. A name that isn't in the list fails the load
with `no such token`.

| Token | Kind | Default |
| --- | --- | --- |
| `color-bg` | color | `#05070f` |
| `color-surface` | color | `#121a31` |
| `color-surface-raised` | color | `#1a2440` |
| `color-text` | color | `#f5f7fb` |
| `color-text-muted` | color | `#b9c3da` |
| `color-accent` | color | `#0a6cff` |
| `color-focus` | color | `#ffffff` |
| `color-success` | color | `#3ddc97` |
| `color-warning` | color | `#ffb547` |
| `color-danger` | color | `#ff5a6a` |
| `color-info` | color | `#7ab4ff` |
| `color-overlay` | color | `#03050ccc` |
| `color-panel` | color | `#0b1020` |
| `color-text-faint` | color | `#7d88a3` |
| `color-on-focus` | color | `#0a0d18` |
| `color-accent-soft` | color | `#8dbaff` |
| `color-caution` | color | `#ffd24d` |
| `color-tint` | color | `#ffffff` |
| `color-shade` | color | `#000000` |
| `font-family` | text | `Inter` |
| `font-size-m` | length | `19` |
| `motion-fast` | duration | `90` |
| `motion-normal` | duration | `150` |
| `motion-slow` | duration | `220` |
| `layout-direction` | text | `ltr` |

The comments in `tokens.rs` say what some tokens are for:

- `color-panel`: deeper than the surface: sheets, hints, the backs of panels.
- `color-text-faint`: less than muted: hints and the third line of a row.
- `color-on-focus`: text and marks drawn on `color-focus`.
- `color-accent-soft`: the accent, light: links, and the glow of a focused field.
- `color-caution`: between success and warning: a game that reaches its menus.
- `color-tint`, `color-shade`: light and dark marks laid over anything, at an alpha the kit
  picks.
- `layout-direction`: `"ltr"` or `"rtl"`, which side a theme puts its leading edge on.
  `Look.rtl` reads it.

There are no tokens for sizes, spaces, radii or shadows. The kit sizes everything as
`N * Look.px`, which follows the window and the player's "Size of everything". Write those
values in your own files.

Write each kind in `theme.toml` as `TokenValue::parse` reads it:

- color: `"#rrggbb"` or `"#rrggbbaa"`;
- length: a number, in logical pixels;
- duration: a number, in milliseconds;
- int: a whole number;
- text: a string.

A value of the wrong kind fails the load with `ThemeError::Token`, which names the token.

**Animations.** "Animations" is the launcher's own setting (Settings → Launcher,
`Vm.animations`). With it off, the host sets every motion token to 0, whatever the theme set,
and `Look.timed(...)` gives 0 too. So use the motion tokens or `Look.timed` for every animation
time. What moves on its own, such as a turning ring or a drifting wall of covers, reads the time
from `Look.clock`, never from `animation-tick()`: `Look.clock` stands still with Animations off.
Don't offer an Animations option of your own.

## Options

An option is a setting the player changes on the theme's page. It sets tokens. Option names use
lowercase letters, digits and `-`, and start with a letter.

A choice option sets a group of tokens for each choice. For example, a theme that offers a
calmer pace:

```toml
[theme.options.pace]
label = "Pace"
type = "choice"
default = "lively"

[theme.options.pace.choices.lively]
motion-normal = 260
motion-slow = 420

[theme.options.pace.choices.calm]
motion-normal = 400
motion-slow = 640
```

A color option sets one color token. From `tests/fixtures/themes/fixture-grid/theme.toml`:

```toml
[theme.options.accent]
label = "Accent color"
type = "color"
token = "color-accent"
default = "#0f9d8a"
```

`hidden = true` keeps an option out of the Settings list. It still works when something else
sets it.

How the values apply (`merge` in `crates/ui-api/src/theme.rs`):

- Until the player sets an option, the theme's own `[theme.tokens]` (or the defaults) stay.
  `default` only says what `Options` holds.
- Once the player sets an option, its tokens go over the theme's.
- A color value that doesn't read leaves the token as it was, and the log says
  `theme option ignored`.
- An unknown choice sets no tokens. `Options` then holds the default.

`Vm.theme-options` lists the shown theme's options as fields. `Commands.set-theme-option(key,
value)` sets one. A change sets `Tokens` and `Options` again on the running theme, with no
compile.

A theme reads its option values from the `Options` global. Cut down from the test
`a_theme_reads_its_option_values_from_the_options_global` in `tests/tests/seams/themes.rs`:

```slint
import { Options } from "../options.slint";
export component HomeScreen inherits Rectangle {
    Text {
        text: Options.motion;
        color: Options.tint;
    }
}
```

## Breakpoints and the size of everything

A breakpoint is a named window size. `[[theme.breakpoints]]` lists them in order. The first one
whose limits all hold is the current one (`Breakpoint::matches`). A breakpoint with no limits
always holds, so put it last. When none holds, the name is `""`.

| Key | Limit |
| --- | --- |
| `name` | The name `Vm.viewport.breakpoint` holds. |
| `min_width`, `max_width` | The window's width, in logical pixels. |
| `min_aspect`, `max_aspect` | Width divided by height. |
| `min_scale` | Physical pixels per logical pixel. |

The built-in themes have `compact` (`max_width = 900`) and then `regular`.

`Vm.viewport` holds `width`, `height`, `scale` (the screen's pixel density), `aspect`,
`breakpoint` and `size`. `size` is the player's "Size of everything", from 0.8 to 1.4, where 1
is the theme's own size (`MIN_SCALE` and `MAX_SCALE` in `crates/core/src/preferences.rs`). An
environment variable can set it for one run: see
[Troubleshooting](../players/troubleshooting.md).

A theme that doesn't multiply its sizes by `size` ignores the setting. The kit's `Look.k` does
it: it maps a 1920×1080 canvas to the window, times `size`. Write a size as `N * Look.px` for N
pixels at 1080p.

## Keys and the gamepad

A theme needs no gamepad code. The host turns each controller button into a key press on the
window (`PadButton::action` in `crates/ui-host/src/input.rs`). So a keyboard does what a
controller does.

| Button | Key the theme gets |
| --- | --- |
| D-pad and left stick | `Key.UpArrow`, `Key.DownArrow`, `Key.LeftArrow`, `Key.RightArrow` |
| South (A, ✕) | `Key.Return` |
| East (B, ○) | `Key.Escape` |
| North (Y, △) | `"/"` |
| West (X, □) | `Key.Insert` |
| Start (Options) | `Key.Menu` |
| Select (Create, Share, View) | `Key.Delete` |
| Left shoulder (L1) | `Key.Backtab` |
| Right shoulder (R1) | `Key.Tab` |
| Left trigger (L2) | `Key.PageUp` |
| Right trigger (R2) | `Key.PageDown` |

The Guide button (PS or Xbox) is not a key:

- A tap sends `Navigate` to `Home`. While a game runs, a tap does nothing.
- Held for 600 ms (`GUIDE_HOLD` in `crates/app/src/gamepad.rs`), it stops the running games.
  With no game running, it adds one to `Vm.power-menu`. A theme opens its power menu when that
  number changes.

While a game's window is in front, or the launcher's window has no focus, the other buttons do
nothing.

A held button or stick repeats after 380 ms, then every 85 ms (`gamepad.rs`). A repeat comes as
a repeated key press, so `event.repeat` is true. Ignore repeats of Enter: a held ✕ would play,
download or install again. `ShellKeys` does this.

`Vm.pad-key` is true while the host delivers a controller's key, and false for a keyboard's.
The kit's text fields open the on-screen keyboard only for a controller. `Vm.input-mode` is the
first entry of `input` at start, and `gamepad` once a controller button comes in.
`Vm.pad-name` is the controller's name, or empty for none.

## Images, fonts, sounds and system art

Declare each file in `[theme.assets]`. Paths are relative to the theme folder and use `/`. From
`plugins/themes/default/theme/theme.toml`:

```toml
[theme.assets]
fonts = [
    "fonts/Inter-Regular.ttf",
    "fonts/Inter-Medium.ttf",
    "fonts/Inter-SemiBold.ttf",
    "fonts/Inter-Bold.ttf",
    "fonts/Tajawal-Regular.ttf",
    "fonts/Tajawal-Bold.ttf",
]

[theme.assets.images]
logo = "images/logo.png"
logo-mark = "images/logo-mark.png"
all-games-icon = "images/all-games-icon.png"
all-games-cover = "images/all-games-cover.jpg"

[theme.assets.sounds]
move = "sounds/move.wav"
select = "sounds/select.wav"
back = "sounds/back.wav"
error = "sounds/error.wav"
start = "sounds/start.wav"
```

**Images.** Each name becomes `Assets.<name>`. A name uses lowercase letters, digits and `-`.
The host decodes PNG and JPEG, and shrinks an image to fit 1920×1080 (`MAX_WIDTH` and
`MAX_HEIGHT` in `crates/ui-host/src/images.rs`). A missing or broken image stops the load with
`ThemeError::Asset`. Game art comes from `Vm` (`GameCard.cover` and the like), not from
`Assets`.

The built-in themes declare two images of the launcher: `logo`, the full emblem, for the
closing screen (`Overlays`), and `logo-mark`, the gamepad alone, for the loading screen and the
About card in Settings. Both are PNGs at 512 px: `logo-mark.png` is
`assets/logo-mark-static.svg` drawn by `scripts/make-icons.sh`. The host draws no SVG, and the
mark's wink (`assets/logo-mark.svg`) is a CSS animation, so the themes use the still mark.

**Fonts.** The host registers each font file through imports in the served `tokens.slint`. Set
`font-family` to the family name inside the files, such as `"Inter"`. A font path must stay
inside the theme folder. A built-in theme has no folder, so the host writes its fonts once to
`launcher-fonts/<theme>/` in the system's temp folder.

**Sounds.** A sound's name is what `Host.play-sound(name)` takes. The launcher plays WAV and Ogg
Vorbis (the `rodio` features in the workspace `Cargo.toml`). The host plays nothing when the
player turned sounds off or the name isn't declared. The theme gets no error. The built-in
themes play `move`, `select`, `back`, `error` and `start`.

**System art.** Each entry of `Vm.emulators` has `system` (such as `PS5`), `icon` (square) and
`cover` (wide). The emulator plugin ships the two pictures. A theme can put its own in their
place, by system id, as PNG or JPEG files:

```toml
[theme.systems.PS5]
icon = "images/ps5-icon.png"
cover = "images/ps5-cover.jpg"
```

The host decodes them when it loads the theme. A picture left out stays the plugin's. A missing
or broken file stops the load with `ThemeError::Asset`. The key is the system, so the picture
stays when another emulator plays the system.

## Rules the host checks

Before it compiles, the host checks each of the theme's `.slint` files with `contract::lint`
(`crates/ui-api/src/contract.rs`). Comments don't count. A file that breaks a rule stops the
load with `ThemeError::File`, which names the file.

- No `@image-url`. Show images from `Assets`, from `Vm`, or from slot icons.
- Every `import` and `export ... from` names a `.slint` file. A font import is refused: list
  fonts in `theme.toml`.
- A screen file exports its component under the name in [the table](#screens). `shell.slint`
  exports `Shell`.

The host also refuses a theme that ships a [served file](#the-files-the-host-serves), or that
misses the shell or a screen.

The compiler serves the files from a virtual folder, `/launcher-theme` (`VFS_ROOT` in
`crates/ui-host/src/compile.rs`), and reads nothing from disk. An import that leaves the folder
fails with `is outside the theme`. A compile error stops the load with `ThemeError::Compile`,
which lists each error with its file, line and column.

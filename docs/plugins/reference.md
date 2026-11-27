# Plugin reference

This page lists every part of the plugin contract: the manifest, the checks, the plugin context,
permissions, each capability, backup, startup work, settings, errors, the shared `emulator-kit`
library, the test kit and the built-in plugins. It is for plugin authors who know the steps in
[Writing a plugin](writing-a-plugin.md) and need the details.

The contract is the crate `launcher-api` (`crates/api`). Each trait and type below is in
`crates/api/src` unless the text names another file. Terms follow the
[glossary](../../GLOSSARY.md).

## Contents

- [The plugin traits](#the-plugin-traits)
- [The manifest](#the-manifest)
- [What the kernel checks](#what-the-kernel-checks)
- [The plugin context](#the-plugin-context)
- [Permissions and host folders](#permissions-and-host-folders)
- [Capabilities](#capabilities)
- [Backup](#backup)
- [Startup work](#startup-work)
- [Settings and the plugin page](#settings-and-the-plugin-page)
- [Errors](#errors)
- [Shared code: emulator-kit](#shared-code-emulator-kit)
- [Testing](#testing)
- [The built-in plugins](#the-built-in-plugins)

## The plugin traits

In `plugin.rs`:

| Trait or type | What it is |
| --- | --- |
| `Plugin` | Every plugin: `manifest()`, and `shutdown()` with a default body. |
| `CodePlugin` | A plugin with code: `init(ctx)` returns `Provided`. Emulators, services and system plugins. |
| `ThemePlugin` | A theme: `files()` returns its files. It has no `init`. |
| `PluginBox` | A plugin as the host registers it: `PluginBox::code(...)` or `PluginBox::theme(...)`. |
| `PluginFactory` | Registers a built-in plugin at compile time: `PluginFactory::code(...)` or `PluginFactory::theme(...)`, inside `inventory::submit!`. |
| `Provided` | One `Option` field for each capability with code, and `assets`. |

`Provided.assets` holds read-only files (`Arc<dyn Files>`) that slot items and catalog entries
point at as `ImageSource::Asset`. `launcher_api::system_art!()` serves the crate's
`assets/icon.png` and `assets/cover.jpg` as `icon.png` and `cover.jpg`, the names a manifest's
`[system]` uses.

## The manifest

A plugin with code has `plugin.toml`. A theme has `theme.toml`. Both use one format,
`PluginManifest` in `manifest.rs`, parsed with `PluginManifest::from_toml`.

A top-level key the parser doesn't know is not an error. It goes into `extra`, the table of
sections for one kind only, such as a theme's `[theme]`. So a misspelt top-level key is ignored.
These sections refuse keys they don't know: `[permissions]`, `[[permissions.filesystem]]`,
`[[dependencies]]`, `[runtime]`, `[requires]`, `[requires.recommended]`, `[[keymap]]` and
`[system]`.

The PS5 plugin's manifest, cut short (`plugins/emulators/kytyps5/plugin.toml`):

```toml
id = "emu-kyty"
kind = "emulator"
name = "PS5"
emulator = "KytyPS5"
version = "0.1.0"
host_api = "^0.7"
platforms = ["linux", "macos", "windows"]
console = "PS5"
capabilities = ["catalog", "downloader", "installer", "configurator", "launcher", "compatibility", "slots", "game_installer", "backup"]
install_note = "Installs a PS5 game from its folder, an exFAT image or an unsigned PS5 package (.pkg). ..."

[[keymap]]
button = "Cross"
keys = "J"

[requires]
vulkan = "1.3"

[requires.recommended]
cpu_threads = 12
memory_gb = 14.5
gpu_memory_gb = 7.5
discrete_gpu = true

[permissions]
links = ["https://github.com/KytyPS5/KytyPS5/issues/new?"]
network = ["api.github.com", "github.com", "*.githubusercontent.com", "kytyps5.github.io"]
process = ["builds/*/kyty_emulator", "folder:kyty-build", "system:kyty_emulator"]

[[permissions.filesystem]]
folder = "games"
write = true

[[permissions.filesystem]]
folder = "kyty-data"
default = "~/.local/share/KytyPS5"
default_on = { macos = "~/Library/Application Support/KytyPS5", windows = "~/AppData/Roaming/KytyPS5" }
title = "KytyPS5 saves"
help = "Saves of every KytyPS5 build that has --data-folder."
write = true

[system]
id = "PS5"
name = "PlayStation 5"
icon = "icon.png"
cover = "cover.jpg"
```

### Top-level keys

| Key | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The plugin's id: 1 to 64 lowercase ASCII letters, digits or `-`, starting with a letter. It names the plugin's data folder and its settings. |
| `kind` | yes | `emulator`, `service`, `theme` or `system`. |
| `name` | yes | What the player sees: in the plugin lists, on the plugin page, and where a sentence means the console. An emulator names its console: `PS5`. |
| `emulator` | no | For an emulator, the name of the program it installs and runs, such as `KytyPS5`. Sentences about the program use it. Leave it out when the program has no name of its own: `name` is used. |
| `version` | yes | The plugin's semver version. |
| `host_api` | yes | A semver range of the API versions the plugin works with, such as `^0.7`. The host refuses the plugin when the range doesn't include `API_VERSION`, now `0.7.0`. |
| `platforms` | no | `linux`, `macos`, `windows`. Empty means every system. |
| `console` | no | For an emulator, the console it plays: ASCII letters, digits and `-`, kept in uppercase (`ps5` and `PS5` are one console). The host gives the emulator that console's releases. Not the systems the plugin runs on. |
| `capabilities` | no | The capabilities `init` provides, in snake case. See [Capabilities](#capabilities). |
| `permissions` | no | What the plugin may do outside its data folder. See [Permissions and host folders](#permissions-and-host-folders). |
| `dependencies` | no | `[[dependencies]]` entries. See below. |
| `activation` | no | `lazy` (default): `init` runs the first time the host needs a capability. `eager`: `init` runs at boot. |
| `runtime` | no | `[runtime]` with `type = "native"` (default: compiled into the binary) or `type = "data"` (a theme with no code). |
| `assets` | no | A folder of images, relative to the plugin's folder on disk. The host reads it only for a plugin from a folder. A built-in plugin gives `Provided.assets` instead. |
| `config_schema` | no | A JSON value. The parser keeps it, but the host doesn't read it: settings come from the `configurator` capability. |
| `requires` | no | What the computer needs for the plugin's games. See below. |
| `keymap` | no | `[[keymap]]` entries. See below. |
| `install_note` | no | For an emulator with a game installer: what it installs from, in one or two sentences. The install sheet shows it. |
| `system` | no | `[system]`: the system the plugin's games show under. See below. |
| `shared_data` | no | A data folder this plugin shares with every plugin that names the same one, such as `retroarch`: lowercase letters, digits and `-`. Its storage, downloads and programs live in `shared/<name>` in the app's data folder, not in `plugins/<id>`. The host runs one install among them at a time, and after one, it reads the builds of each. Only a built-in plugin may name one: a plugin from disk that does is refused. |
| `system_files` | no | `[system_files]`: files the emulator needs besides its program and the games, which the player imports from one archive on its page. See below. |
| `consoles` | no | `[[consoles]]`: for an emulator that plays several consoles, each console in place of `console`, `[system]`, `keymap` and `install_note`. See below. |

### `[[dependencies]]`

| Key | Meaning |
| --- | --- |
| `id` | The plugin it needs. |
| `version` | A semver range. Default: any version. |
| `optional` | `true`: loaded first when present, ignored when missing. Default `false`. |

The kernel orders plugins so dependencies load first. It rejects a plugin whose required
dependency is missing, has the wrong version, failed to load, or is part of a cycle. No built-in
plugin has dependencies: plugins share code through `emulator-kit`, a library.

### `[requires]` and `[requires.recommended]`

The host compares these with this computer (`crates/core/src/hardware.rs`). Games don't start
without `vulkan` or `avx2`. Below `recommended`, they may run slowly.

| Key | Meaning |
| --- | --- |
| `vulkan` | The Vulkan version the graphics card must support, such as `"1.3"`. |
| `avx2` | The processor must have AVX2. Not checked where the system can't say. |
| `recommended.cpu_threads` | Processor threads. |
| `recommended.memory_gb` | Memory in GB. |
| `recommended.gpu_memory_gb` | Graphics memory in GB. |
| `recommended.discrete_gpu` | A graphics card of its own, not one built into the processor. |

### `[[keymap]]`

Each entry has `button` (such as `"Cross"`) and `keys` (such as `"J"`). Both are required. The
launch splash lists them in this order. The contract suite checks that each button is listed
once and that neither value is empty.

### `[system]`

| Key | Meaning |
| --- | --- |
| `id` | The code a game's `platform` has: `PS5`, `PC`, `Steam`. |
| `name` | The system's full name: `PlayStation 5`. |
| `color` | `#rrggbb`. Without it, a theme draws the system in its neutral look. |
| `icon` | A square picture in the plugin's assets. |
| `cover` | A wide picture in the plugin's assets, which a theme can show behind the system. |

A theme can put its own pictures in place of `icon` and `cover`: see
[the theme reference](../themes/reference.md).

### `[system_files]`

Firmware, keys, BIOS files or system modules, which the player imports from one archive on the
emulator's page. The host does the import: the plugin needs no code for it. The page gets a
block with the files' state and an **Import…** row (`HOST_IMPORT_SYSTEM_FILES`), which opens
the folder browser on archives. The import unpacks the archive through the host's `Archives`
port, checks `SHA256SUMS` when the archive has one, and copies each part into its folder. It
writes nothing until every part is checked, and puts every part back when one can't be written.
The code is in `crates/core/src/system_files.rs`. From `plugins/emulators/lichii/plugin.toml`:

```toml
[system_files]
name = "Switch firmware and keys"
archive = "switch-firmware-keys.zip"

[[system_files.part]]
from = "firmware"
folder = "lichii-data"
into = "nand/system/Contents/registered"
files = ["*.nca"]
replace = true

[[system_files.part]]
from = "keys"
folder = "lichii-data"
into = "keys"
files = ["prod.keys", "title.keys"]
required = ["prod.keys"]
replace = true
```

| Key | Meaning |
| --- | --- |
| `name` | What the page calls them: "Switch firmware and keys". |
| `archive` | The archive's usual name, which the page's hint names. |
| `optional` | Games play without them (BIOS files): the page doesn't warn while they are missing. |
| `part.from` | The folder in the archive, at its top or in the one folder the archive holds. |
| `part.folder` | The `[[permissions.filesystem]]` folder the files go into, which the plugin must write. Without it, the plugin's data folder (the shared one with `shared_data`). |
| `part.into` | The folder inside that folder. Never empty: the import works beside it. |
| `part.files` | The file names the part takes, `*` for any text. Empty takes every file. |
| `part.required` | File names the part must have. Without them, one file is enough. |
| `part.replace` | The target folder is swapped whole, so no old file stays (a firmware). Without it, the part's files replace the files of the same name, and the folder's other files stay. |

### `[[consoles]]`

An emulator that plays several consoles lists each one. The player turns each on or off with the
switches of the **Consoles** block on the plugin's page
([`launcher_api::HOST_SET_CONSOLE`](../../crates/api/src/slots.rs)). The host saves the choice
under `app.consoles.<plugin>.<console>`. A console that is off has no Home entry, no catalog
and no system-files block, and its games are out of the app. Nothing of it is deleted, and it
comes back when the player turns it on. A console the player never changed is as its `on` says,
so a console a new version adds comes on by itself.

```toml
[[consoles]]
console = "NES"
install_note = "Installs an NES game from its ROM file."
[consoles.system]
id = "NES"
name = "Nintendo Entertainment System"
icon = "nes/icon.png"
[[consoles.keymap]]
button = "A"
keys = "X"
[consoles.system_files]
name = "NES BIOS files"
archive = "nes-bios.zip"
[[consoles.system_files.part]]
from = "system"
into = "system"

[[consoles]]
console = "ATARI2600"
on = false
[consoles.system]
id = "ATARI2600"
name = "Atari 2600"
```

| Key | Meaning |
| --- | --- |
| `console` | The console, as the top-level `console` says one. Its games' `platform` is this code. |
| `system` | Its `[system]`. Its `id` must be the `console` code. |
| `keymap`, `install_note`, `system_files` | As the top-level keys say, for this console's games. The launch splash shows the keys of the game's console. |
| `on` | The console is on until the player turns it off. With `false`, it is off until they turn it on. `true` by default. |

Beside `[[consoles]]`, a top-level `[system_files]` is one archive for all the consoles: the page
shows one block for it (`PluginManifest::shared_system_files`). A console can also have its own
in `[consoles.system_files]`.

The host reads either form with `PluginManifest::played`, and the console of a game with
`played_for(platform)`. The games of two consoles share the plugin's ids, so a plugin gives each
an id the console's code begins, such as `NES/Contra`.

## What the kernel checks

`validate` in `crates/core/src/kernel.rs` rejects a plugin at boot when:

- the host is in safe mode and the plugin is third-party;
- `host_api` doesn't match the host's API version;
- it is a theme, and `platforms` doesn't include this system;
- `[system]` `color` isn't `#rrggbb`;
- a folder grant's `folder` or `was` isn't lowercase letters, digits and `-`;
- a folder grant has both `write = true` and `listed_in`;
- `[system_files]`, or one of `[[consoles]]`, has no name, archive or part, a part's `into` is
  empty, or a part's `folder` isn't a folder grant with `write = true`;
- a plugin that isn't an emulator lists `[[consoles]]`; or a manifest with `[[consoles]]` also
  has `console`, `[system]`, `keymap` or `install_note`; or it lists a console
  twice; or a console's `[consoles.system]` id isn't its console code, or its `color` isn't
  `#rrggbb`;
- the plugin is third-party with `runtime` `native` ("third-party plugins must be data-only");
- a plugin that isn't `system` declares a system capability, or lists `long_running` or `polkit`;
- an entry of `long_running` or `polkit` isn't also in `process`;
- a theme declares a capability other than `theme`, or doesn't declare `theme`;
- a plugin that isn't a theme declares `theme`, or has `runtime` `data` ("... plugin needs code").

An emulator made for other systems is not rejected. It loads turned off, so the player can turn
it on with their own build.

At `init`, the host compares `Provided` with `capabilities`. When they differ, it calls
`shutdown` and rejects the plugin with `CapabilityMismatch`. Each rejection is in the boot report
and in the log as `plugin not loaded`.

## The plugin context

`init` gets a `PluginContext` (`context.rs`). It is the plugin's only way to the host.

| Method | Type | What it gives |
| --- | --- | --- |
| `plugin()` | `PluginId` | The plugin's own id. |
| `config()` | `ConfigStore` | The plugin's settings, for each `ConfigScope`. `get` returns `Null` when nothing is stored. |
| `storage()` | `Storage` | Files in the plugin's data folder. |
| `folders()` | `Folders` | The host folders the manifest asks for. |
| `programs()` | `Programs` | Find and run the programs `permissions.process` allows. |
| `http()` | `HttpClient` | HTTP to the hosts in `permissions.network`. |
| `log()` | `Logger` | The host's log, tagged with the plugin's id: `debug`, `info`, `warn`, `error`. |
| `events()` | `EventSink` | Publish `Event::Custom`, and subscribe to every event. |
| `calendar()` | `Calendar` | `unix_secs()`: the time in seconds since 1970, from the core's clock. Read it for what you keep on disk, such as when you fetched a copy. A test moves it. |
| `services()` | `ServiceRegistry` | Host services, by type. |

### Storage

Each plugin has a data folder, `<data-dir>/plugins/<id>`. `Storage` reads and writes in it:
`read`, `write` (makes the parent folders), `write_at`, `exists`, `list`, `remove`, `rename` and
`set_executable`. Every path is a `RelPath`, so no path can leave the folder. Where the data
folder is: see [the architecture](../contributors/architecture.md#data-folder).

### Programs

- `Programs::find` returns a program's real path, or `None`.
- `Programs::output` runs a program with arguments and no input, and returns its standard output:
  at most `MAX_PROGRAM_OUTPUT` (64 KiB). The caller gives a timeout, at most `MAX_PROGRAM_TIME`
  (10 seconds). A program past its time is killed, and the call fails with
  `PluginError::Timeout`. Use it to see whether a build is there and to read its version.
- `Programs::stream` runs a program with no time limit and passes each line it writes to a
  callback. When the `cancelled` callback says so, the host kills the program. A plugin that
  wants a time limit says it is cancelled at the limit. It is for system plugins only. The
  program must be in `permissions.long_running`, and in `permissions.polkit` to run as the
  administrator.

A program is a `Location` (`plans.rs`): `Location::Data(RelPath)` in the data folder,
`Location::Folder(HostPath)` in a host folder, or `Location::System(name)` on the system's
`PATH`.

### HTTP

`HttpClient::send` takes an `HttpRequest` (`Get`, `Head` or `Post`, with headers and a body) and
returns an `HttpResponse`. `HttpRequest::get(url)` is the short form. `HttpResponse::json` fails
with `PluginError::Network` on a status outside 200 to 299. The host refuses a host that isn't
in `permissions.network`.

### Events

`EventSink::publish(topic, payload)` sends an `Event::Custom`. A plugin can publish no other
event. `EventSink::subscribe()` returns a stream of every `Event` (`event.rs`): plugin,
download, install, process, library, state and theme events.

Two topics ask the host to read the plugin again:

- `RELEASES_CHANGED` (`"releases.changed"`): a `releases` plugin's catalogs changed between the
  host's asks. The host asks for the catalogs again.
- `SLOTS_CHANGED` (`"slots.changed"`): the plugin's `emulator.panel` changed between the host's
  asks, for example when work in the background ends. The host asks for the panel again.

### Services

`ServiceRegistry::get::<T>()` returns a host service. The host puts three in the registry of
every plugin:

- `HostInfo`: the API version, the platform, whether safe mode is on, and `runs_32bit` (the
  system has the loader for 32-bit Linux programs).
- `VpnGate`: "Download only through a VPN", as the host sees it. Call `check()` before you fetch
  what the player downloads in bulk. `Ok(VpnPass::Open)` or `Ok(VpnPass::Unbound { .. })` lets
  you fetch. `Err(VpnBlock::Waiting { .. })` means wait and try again.
  `Err(VpnBlock::NoVpn { .. })` means skip the fetch. The gate only reports the state: the
  plugin's HTTP doesn't go through the VPN.
- `Controllers`: the game controllers connected now, as SDL3 sees them. Each call runs a probe,
  so it may take a second.

Plugins never reach each other through the registry.

## Permissions and host folders

`[permissions]` lists what the plugin may do outside its data folder. The host enforces every
entry (`Permissions` in `manifest.rs`).

| Key | Meaning |
| --- | --- |
| `network` | Hosts for HTTP: an exact name (`github.com`), or a suffix pattern (`*.github.com`, which also matches `github.com`). Case doesn't matter. |
| `filesystem` | `[[permissions.filesystem]]` grants for named host folders. See below. |
| `process` | Programs the plugin may start. Empty means none. See below. |
| `long_running` | System plugins only: `system:` entries of `process` that may run with no time limit through `Programs::stream`, such as `system:bluetoothctl`. |
| `polkit` | System plugins only: `system:` entries of `process` that may run as the administrator through `pkexec`. |
| `links` | Links the host may open with the system's handler, by how they start: `steam://install/`, or a compatibility list's report form. A link must be one line of plain characters. Empty means none. |

### Process rules

A `process` entry has one of three forms (`Permissions::allows_program`):

- A path in the data folder, split by `/`. A part may be `*` (any name) or `*<ending>` (any name
  with that ending), such as `builds/*/kyty_emulator` or `builds/*/*.AppImage`. The rule and the
  path have the same number of parts.
- `folder:<name>`: any file in the host folder `<name>`. The manifest must also grant that
  folder.
- `system:<name>`: a program on the system's `PATH`, by file name, such as `system:wine`. A name
  with `/`, `\` or `:` never matches.

A `LaunchSpec`, `Programs::find` and `Programs::output` all go through these rules.

### Host folders

A host folder is a named folder outside the data folder, such as `games` or `kyty-build`. The
plugin asks for it with a grant. The player maps each name to one or more real folders. The host
saves the mapping under `app.folders.<name>` as a list of paths.

| Grant key | Meaning |
| --- | --- |
| `folder` | The name: lowercase letters, digits and `-`. Plugins that ask for the same name share the player's mapping. |
| `write` | `true` to write. Default `false`: read only. |
| `default` | Where the folder is until the player maps it. One path (`"~/Games"`), used even when it doesn't exist yet. Or a list (`["~/KytyPS5", "~/src/KytyPS5"]`): each folder in it that exists, in order. `~/` is the home folder. |
| `default_on` | A default for some systems: `{ macos = "~/Library/Application Support/X" }`. It wins over `default` on that system. |
| `title` | What Settings calls the folder. The name when none. |
| `help` | A line under the title. |
| `after` | On the plugin page, the key of the setting this folder's row follows. At the end of the settings when none. |
| `was` | The folder's name before a rename. At the first start with it, the host copies the player's folders saved under it to the new name, when the new name has none. |
| `listed_in` | More folders, read from a file in a default folder: `{ file, key, holds }`. Each value of `key` in `file` (Valve's text format) that is an absolute path to a folder holding `holds`. Read only. |

Two host folders are shared (`SHARED_FOLDERS`). Settings shows each once, for every plugin that
asks for it, with the host's default, title and help, not the plugin's.

| Name | Default | Title |
| --- | --- | --- |
| `games` (`GAMES_FOLDER`) | `~/Games` | Game folders |
| `packages` (`PACKAGES_FOLDER`) | `~/Downloads` | Packages to install |

Installs go into the console's folder (`PS5`, `PS4`) inside the first `games` folder. A plugin
looks for packages in its console's folder inside each `packages` folder.

### `RelPath` and `HostPath`

`RelPath` (`path.rs`) is a path inside a folder the host owns. It is relative, uses `/`, and has
no `.`, `..` or empty parts, and no `\` or `:`. Joining it to its base folder can never leave
that folder. The empty path (`RelPath::root()`) is the base folder. `RelPath::new` returns
`InvalidPath` for anything else, and `?` turns that into `PluginError::InvalidInput`.

`HostPath` (`context.rs`) is a path in a host folder:

```rust
pub struct HostPath {
    pub folder: String, // the grant's name
    pub root: u32,      // which of the player's folders for that name, from 0
    pub path: RelPath,  // a path inside it
}
```

`Folders::roots(name)` lists the player's folders for a name, each with its `root` index and its
real path to show. The other `Folders` methods take a `HostPath`: `list`, `size`, `exists`,
`read_range` (at most `MAX_FOLDER_READ`, 16 MiB, for each call), `write`, `write_at`, `rename`,
`remove` and `set_executable`. The host refuses a name the manifest doesn't grant, a write without
`write = true`, and a symlink out of the folder.

## Capabilities

The manifest's `capabilities` list names them in snake case. `init` returns an implementation
for each one in `Provided`.

| Capability | `Provided` field | Trait | File |
| --- | --- | --- | --- |
| `catalog` | `catalog` | `Catalog` | `catalog.rs` |
| `downloader` | `downloader` | `Downloader` | `plans.rs` |
| `installer` | `installer` | `Installer` | `plans.rs` |
| `configurator` | `configurator` | `Configurator` | `config.rs` |
| `launcher` | `launcher` | `Launcher` | `plans.rs` |
| `compatibility` | `compatibility` | `Compatibility` | `extras.rs` |
| `slots` | `slots` | `Slots` | `slots.rs` |
| `game_installer` | `game_installer` | `GameInstaller` | `games.rs` |
| `metadata` | `metadata` | `Metadata` | `catalog.rs` |
| `releases` | `releases` | `Releases` | `releases.rs` |
| `backup` | `backup` | `Backup` | `backup.rs` |
| `startup` | `startup` | `StartupWork` | `startup.rs` |
| `theme` | none | `ThemePlugin` | `plugin.rs` |
| `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install`, `drives` | `areas`, keyed by `SystemArea` | `AreaSettings`, one for each area | `system.rs` |

### Catalog

`Catalog` lists, searches and describes the games one emulator can run:

- `list(PageRequest)` returns a `Page<GameSummary>`. `PageRequest::ALL` asks for every game.
- `search(&SearchQuery)` returns the matching games.
- `details(game)` returns `GameDetails`, or `NotFound`.
- `artwork(game, ArtworkKind)` returns an `ImageSource` for `Cover`, `Background`, `Logo` or
  `Icon`, or `None`.

`GameSummary.id` is the plugin's own id for the game. The host makes it global as a `GameKey`,
written `plugin:local`. Ids must be unique: the contract suite checks it.

`GameDetails` has the game's folder (`folder`), other folders that belong to it (`also`, which
an uninstall deletes with it), and `uninstall_link` for a game its store removes. Its `facts` are
`GameFact` values. Themes know the ids `region`, `age`, `version`, `title-id`, `content-id` and
`folder`, and show other facts by their label.

An `ImageSource` is a URL (`Url`), an asset (`Asset(RelPath)`), or a file in a host folder the
plugin may read (`Folder(HostPath)`).

### Downloader and Installer

These two install emulator builds. The host runs the steps:

1. It asks `Downloader::plan(&DownloadRequest::EmulatorBuild { version })` for a `DownloadPlan`:
   a label and a list of `Artifact` values.
2. It downloads each artifact, checks its `sha256` (and an Ed25519 `signature` when one is
   given), writes it to `dest` in the data folder, and unpacks it when `unpack` asks. The formats
   are `TarGz`, `Zip`, `AppImageInZip`, `AppImage`, `SevenZip` and `AppImageIn7z`. The
   AppImage formats run the AppImage's `--appimage-extract`, on Linux only; `AppImageIn7z`
   unpacks the 7z, then extracts its one AppImage where it is. A 7z can unpack only some of
   its paths: `unpack.only` lists them, each a file or a folder with everything in it. The host
   refuses `only` for the other formats.
3. It calls `Installer::install(version, files)` or `Installer::update(from, to, files)` with
   the `DownloadedArtifact` values.

`Installer` also has `available()`, `installed()` and `uninstall(version)`. The default `update`
installs `to`, then uninstalls `from`, so a failed install leaves `from` in place.
`InstalledVersion.own` marks an own build: the host never updates it by itself.

### Launcher

`Launcher::prepare(game, config)` returns a `LaunchSpec`. `config` is the game's merged settings:
the emulator's, then the game's. The host starts the process only when `permissions.process`
allows `program`. It then watches the process and stops it.

`LaunchSpec` fields:

- `program`: a `Location`.
- `args`, `env`: each value a `LaunchArg`, text or a path (`LaunchArg::path(...)`) the host turns
  into a real path.
- `cwd`: `None` means the program's folder. A system program needs one.
- `game_process`: for a program that hands the game to another one and may exit at once, such as
  Steam's client. Text in the command line of the game's own process. On Windows the host
  looks for the text in the path of each process's program, in any case and with `/` and `\`
  alike. The host then follows that process.
- `game_env`: with `game_process`, a `(key, value)` environment entry that every process of the
  game carries, such as `("SteamAppId", "570")`. Set it when the program that runs the game moves
  it out of the found process's tree, as Steam's runtime does. The host then counts each process
  with that entry as the game's. `None`: the game is the found process's tree.
- `relaunch`: an exit `code` and an `env`. When the program exits with that code, the host
  starts it again in the same session, with `env` added.
- `sandbox`: a `Sandbox` with the `Location`s the game may `read` and may `write`. On Linux the
  host runs the whole command through bubblewrap, and the game sees only the system, the
  display, the sound, an empty home folder and those folders. Each folder goes through the same
  permission rules as a path argument. The host makes the `write` folders first. Without `bwrap`
  the launch fails. Ask for a sandbox on Linux only: on other systems the launch fails.

The other methods have default bodies:

- `open_emulator(config)` returns a `LaunchSpec` for the emulator's own window with no game, or
  `None`.
- `on_exit(game, &ExitInfo)` runs after the game ends.
- `close_left_running()` runs at Log out, Restart and Shut down while a program that one of the
  plugin's launches left running still runs, such as Steam's client. Close it cleanly. The host
  waits for it, then ends it when it doesn't close.

### Configurator

`Configurator` describes and checks settings for a `ConfigScope`: `Emulator` or
`Game(<local id>)`.

- `schema(scope)`: a JSON Schema, usually from `schemars::schema_for!`.
- `defaults(scope)`: the default values.
- `validate(scope, value)`: `InvalidInput` with a message the player can read.

See [Settings and the plugin page](#settings-and-the-plugin-page).

### Compatibility

`Compatibility::status(game)` returns `CompatInfo`: a `CompatLevel` (`unknown`, `nothing`,
`boots`, `menus`, `ingame`, `playable`), and optional `notes`, `source` (the system it was tested
on), `page` and `tested` (`YYYY-MM-DD`). An unknown game must fail: the contract suite checks it.

`Compatibility::report(game, &ResultReport)` (API 0.3.3, default `None`) fills in the list's
report form with the player's own result: the level, the emulator build, the system, the
processor, graphics and memory, and the end of the emulator's log with the personal data taken
out. It returns a `ReportForm`: the list's name, the form's link with its own query
(`https://github.com/<owner>/<repo>/issues/new?template=<file>`), the title, the fields in the
form's order (`id`, `label`, `value`), and a `note` for what the player still does on the form,
such as its checklist. Return `None` for a list that takes no reports through a form.
`permissions.links` must allow the form's link.

The host takes personal data out of the title and every field again, shows the player the whole
report, and opens the form in the browser only when the player says yes. The player submits it
there. The host holds no account.

`Compatibility::result_for_title(title_id)` (API 0.3.4, default `None`) returns the list's
`CompatInfo` for a title id, such as `CUSA03173`, whether the player has the game or not. After
each catalog read, the host asks the emulator about each Catalog offer's title ids and keeps the
first result. The Catalog's best version, its In-game filter, its Compat sort and the Game Hub's
Compatibility fact read it. The installed game's `status` and the player's own result come first.

The host asks about thousands of offers, so the method must be cheap. Look the id up in a list
held in memory, and never call the network once for each id. A failed call ends the host's
look-ups in that plugin until the next catalog read.

`emulator-kit`'s `compat` module holds shadPS4's and KytyPS5's lists in memory, reads an id the
way the lists do (`compat::title_key`), and builds their report forms (`compat::shad_report`,
`compat::kyty_report`).

### Slots

A slot is a named place in the UI that plugins fill with data. The theme decides where and how
to draw it. `Slots::items(&SlotRequest)` returns `SlotItem` values, and
`Slots::run_action(action, game, args)` runs the button the player picked.

| Slot (`SlotId`) | Name | Where |
| --- | --- | --- |
| `GameBadge` | `game.badge` | Small labels on a game's card and Game Hub. |
| `GameAction` | `game.action` | Buttons in a game's Game Hub. |
| `GameDetailSection` | `game.detail-section` | Blocks of facts in a game's Game Hub. |
| `SettingsPanel` | `settings.panel` | A settings form. |
| `EmulatorPanel` | `emulator.panel` | Blocks on the plugin page. |
| `CatalogAction` | `catalog.action` | Buttons, and badges that say what they did, on the Catalog screen, from a `releases` plugin. |

The three `game.*` slots are for one game. The host asks only the plugin that owns the game, with
`SlotRequest.game` set.

A `SlotItem` has an `id` (unique in the plugin), an `Order` (`priority`, `before`, `after`) and
`SlotContent`: `Badge`, `Action`, `Section`, `Block` or `SettingsPanel`.

A `Block` is a titled list of `BlockRow`s, on the plugin page only, after the build. A row has a
`label`, a `value`, a `hint` under the label, and a `tone`. `progress` (0 to 1) adds a bar under
it. A row with an `action` is a button: picking it calls `run_action`, as an `Action` does. With
`switch` set, the row shows a switch. With `input` set, picking the row opens an editor, and the
typed line goes to `run_action` as the args (a JSON string).

A slot action has the plugin's call timeout. Run long work in the background, show how it goes
in the panel, and publish `SLOTS_CHANGED` when it changes.

Three action names make the host do the work itself instead of calling `run_action`:

- `HOST_OPEN_SETTINGS` (`host:open-settings`): opens the Settings screen.
- `HOST_OPEN_EMULATOR` (`host:open-emulator`): starts `Launcher::open_emulator` as a session.
  No game starts until it closes, and its time is not play time.
- `HOST_INSTALL_GAME` (`host:install-game`): installs the package in `args` (a `HostPath`) with
  the plugin's game installer, as a job in Downloads.

How a theme draws slots is in [the theme reference](../themes/reference.md).

### GameInstaller

`GameInstaller` installs games from packages, disc images and finished downloads:

- `candidates()` lists the `Installable` sources in the folders the plugin may read.
- `begin(source, job)` checks `source`, starts the install in the background, and returns the
  work folder at once. A source that isn't a package fails here with `InvalidInput`. A game that
  is already installed fails here with `Precondition`. When `begin` takes longer than the call
  timeout, the host cancels the job: work that `begin` started stops at its next look at
  `cancelled()`.

The work writes into the work folder and moves it into place at the end. It reports through the
`InstallJob`: `progress(done, total)`, `cancelled()`, and `finished(result)` exactly once. On an
error, the work folder must be gone. `games_folder()` gives the folder the player picked.
`take(from, to)` moves a folder instead of copying it, when the host agrees. The host removes a
work folder left behind when the app stopped before the end.

An emulator on the kit gives `emulator_kit::game_install::install` the trees to copy and where
they go. The kit refuses an installed game, clears each work folder (`<dest>.part`), copies the
files in the background, and moves each work folder into place. `contract::game_install` checks
the refusal.

### Metadata

`Metadata::describe(&GameRef)` returns `GameMetadata` for a game, by its console ids, or `None`
when the store doesn't have it. `GameMetadata` has words (title, description, tagline, publisher,
genres, release date, age rating), a `Rating`, art (`icon`, `cover`, `background`, `logo`,
`screenshots`), `store_url` and `trailer`. Every field may be empty.

### Releases

`Releases::releases(&Console)` returns the `ReleaseList` for one console, or `None` when the
plugin has no catalog for it. The host gives each console's list to the emulator whose manifest
names that console. A `Release` is one copy of a game: a torrent, or a store's install (its
`store` link, which `permissions.links` must allow). What the catalog only shows goes in `facts`.
`install_warning` says why a release may not install. The release stays listed.

`store_installs()` returns the installs a store is doing now, for a catalog whose store installs
its releases. The host shows them in Downloads. Default: none.

### System capabilities

`network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install` and `drives` are
for system plugins only. Each is a system area, a `SystemArea`, and every area has the same
interface ([ADR-0004](../adr/0004-every-system-area-has-one-interface.md)):

```rust
#[async_trait]
pub trait AreaSettings: Send + Sync {
    async fn state(&self) -> PluginResult<AreaState>;
    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()>;
}

pub enum AreaState {
    Absent,              // this machine has nothing to manage here
    Unsupported(String), // it can't be used here, and why: "No Bluetooth adapter"
    Ready(AreaData),     // it works, with the area's data
}
```

A plugin puts one `AreaSettings` per area it manages in `Provided::areas`, keyed by the
`SystemArea`. Each key's capability must be in the manifest.

- `state()` reads the area. The host calls it on its tick, every 5 seconds, and after each
  action. `AreaData` has one kind of data per area: `Network { status, wifi }`,
  `Vpn { finds_vpns, can_import, vpns }`, `Bluetooth { powered, devices }`,
  `Audio { outputs, level }`, `Power { battery, refused }`, `OsUpdates { update }`,
  `OsInstall { disks }` and `Drives { drives }`.
- `act()` runs one `AreaAction`, which belongs to one area (`AreaAction::area`). An adapter
  returns `PluginError::Unsupported` for an action it can't run on this system.
- Quick actions answer within the call timeout and leave the job alone. Long actions
  (`AreaAction::long`: joining a network, a Bluetooth scan, pairing, installing the OS,
  mounting, unmounting, ejecting) start in the background, return at once, and report through
  the `SystemJob` the host passes in: `progress`, `done`, `failed` and `cancelled`.

The host uses the first active system plugin that provides each area. It shows an area that no
plugin provides as absent. Until an area's first read ends, the host shows it as loading: a
plugin never reports that. A read that fails changes nothing; when it is the area's first, the
host shows the area as unsupported, with the error.

The drives area is `drives`, not `storage`: storage is a plugin's own data folder.

- `Drives { drives }` lists each drive with a file system as a `SystemDrive`: its device
  (`/dev/sdb1`), its whole disk (`/dev/sdb`), its UUID, label, model, size, file system, whether
  the player can take it out (`usb`), where it is mounted, its free space, and `system` for a
  drive the system mounted for itself (`/`, `/home`, swap). The host never offers to unmount a
  `system` drive, and lists only the others.
- `Mount`, `Unmount` and `Eject` are long. A mount's `done` text is the folder the drive is
  mounted at. `Eject` takes the whole disk: it unmounts each part, then powers the disk off.
- While the area is unsupported, the host lists the drives in the folders the system mounts
  them in, and can't mount one.

The built-in This PC plugin provides it on Linux with UDisks: `lsblk --json --list -b` for the
list, `udisksctl mount|unmount|power-off -b <device> --no-user-interaction` for the actions.
`udisksctl` runs as the player, not through `polkit`: the UDisks service asks polkit itself.

The power area's `refused` says why restart and shut down can't be used here, while the battery
still shows. The VPN area is unsupported where downloads can't be bound to a VPN ("Not available
on Windows"). Before its first read the host knows nothing of it, so "Download only through a
VPN" still holds and the downloads wait. Errors are `PluginError`s. The host shows their text as
it is.

## Backup

The `backup` capability names what of a plugin's data a backup holds. The player backs up and
restores in Settings → Backup ([Backup and restore](../players/backup.md)). The plugin only names
its items. The host reads and writes the files.

```rust
#[async_trait]
pub trait Backup: Send + Sync {
    async fn items(&self) -> PluginResult<Vec<BackupItem>>;
}
```

| `BackupItem` field | Meaning |
| --- | --- |
| `name` | Its name in the backup, the same on every computer: 1 to 64 lowercase letters, digits and `-` (`BackupItem::valid_name`), such as `saves`. |
| `title` | What Settings calls it, such as "PS5 saves". |
| `place` | A `BackupPlace`: `Data(RelPath)` in the plugin's data folder, or `Folder(HostPath)` in a host folder the manifest grants. A file, or a folder and everything in it. |
| `skip` | Paths below `place` to leave out, from the item's top: `cache`, or `GAME/log`. |
| `large` | The item is large, such as Wine prefixes or DLC. It goes into a backup only when the player turns on "Include large items". |

Build an item with `BackupItem::new(name, title, place)`, then `.skip(&[...])` and `.large()`.

How the host uses the items:

- **Back up.** The host asks each active plugin with `backup` for its items and finds each
  item's real path through the plugin's data folder or its grants. It leaves out, with a line in
  the log, an item with a bad name or a place the grants refuse. It leaves out the large items
  unless the player asked for them. Links never go into a backup.
- **Restore.** The host calls `items()` again on this computer and matches each item in the
  backup to the item of the same plugin and name here. It writes the item to that item's place
  now, so a backup made on one computer restores on another whose folders are elsewhere. An item
  the plugin no longer names is left out. The host writes a restored item even into a read-only
  grant: the host does the writing, and the plugin named the place. So a plugin needs no
  `write = true` for a folder it only reads, such as one with keys.

The file format and the restore's order are in
[the architecture](../contributors/architecture.md).

Most plugins give the same items every time. `emulator_kit::backup::Items::provided` answers
`items()` with a fixed list. From `plugins/emulators/shadps4/src/lib.rs`:

```rust
backup: Some(emulator_kit::backup::Items::provided(vec![
    // Its settings, keys, saves and trophies; not its caches, logs and downloads, and not
    // the PS4 system files, which the player imports again from their archive.
    BackupItem::new(
        "data",
        "PS4 settings and saves",
        BackupPlace::Folder(HostPath::new(DATA, 0, RelPath::root())),
    )
    .skip(&["cache", "log", "shader", "temp", "download", "addcont", "sys_modules", "fonts"]),
    // DLC can take many GB: only when asked for.
    BackupItem::new("dlc", "PS4 DLC", BackupPlace::Folder(HostPath::new(DATA, 0, RelPath::new("addcont")?)))
        .large(),
])),
```

Back up the plugin's own data only. Leave out what the host keeps itself: the plugin's settings
are in the host's `config.json` already. Use `skip` for what the emulator makes again.

## Startup work

The `startup` capability is work a plugin does on its own when the launcher starts, such as a
download the plugin needs before its games play. The boot screen shows it as a row of its
checklist and waits until it is over.

```rust
#[async_trait]
pub trait StartupWork: Send + Sync {
    async fn status(&self) -> Option<StartupStep>;
    // Both have default bodies.
    async fn question(&self) -> Option<StartupQuestion> { None }
    async fn answer(&self, yes: bool) {}
}
```

- The core asks for `status` after it reads the plugin's slots at the start, then every half
  second while the work runs (`READ_EVERY` in `crates/core/src/startup.rs`). `None` means no work
  this run: the boot screen doesn't wait.
- A `StartupStep` has a `label` ("Compatibility list"), a `detail` of a few words, a `progress` from
  0 to 1 when known, and a `state`: `Working`, `Done` or `Failed`.
- The boot screen goes when no work runs, or after two minutes in which nothing moved
  (`STALL_LIMIT`). The work goes on behind Home.
- Work that downloads something the player didn't ask for asks first (`question`, API 0.3.2). A
  `StartupQuestion` has a `title` ("Update the compatibility list?"), a `text` of a sentence or
  two, and its `yes` and `no` buttons ("Install", "Not now"). The core shows it over the boot
  screen, one plugin at a time, and the boot waits for the answer. While it is asked, `status`
  says `None`.
- The core calls `answer` with the player's choice, and doesn't ask that plugin again in this
  run. With yes, the plugin starts its work: its `status` says `Working` before `answer` returns.
- After an answer, the core asks the next plugin that has a question, in the plugins' order,
  also when the answer came after the boot screen stopped waiting.

## Settings and the plugin page

The host stores a plugin's settings in its `config.json`: under `plugins.<id>.emulator` for
`ConfigScope::Emulator`, and `plugins.<id>.game:<local id>` for `ConfigScope::Game`. The plugin
reads them with `ctx.config().get(scope)`.

A settings form needs two capabilities:

- `configurator`, for the schema and the check of a new value.
- `slots`, with a `settings.panel` item whose content is `SlotContent::SettingsPanel { title,
  scope }`.

The host reads the schema as the panel's fields (`crates/core/src/settings_schema.rs`). It shows
booleans, numbers, strings and string choices, and leaves other properties out. Fields come in
the order of the schema's `x-order` list, then the rest by name. With `schemars`, set the order
on the settings type, as `plugins/emulators/shadps4/src/lib.rs` does:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", deny_unknown_fields)]
#[schemars(crate = "launcher_api::schemars", extend("x-order" = ["own_build", "fullscreen"]))]
pub struct Settings { /* ... */ }
```

An optional field left empty is unset, so the emulator's own value applies. Before the host
saves a value, it calls `validate`.

A game's own settings, opened from its Game Hub, need only `configurator`: the host asks for the
schema of `ConfigScope::Game(<local id>)` and shows it when it has properties.

### The plugin page

Each plugin has a page under Emulators or Themes. For a plugin that is turned on, the page holds
(`PluginPageVm` in `crates/ui-api/src/vm.rs`):

- its builds, for an emulator;
- its settings panels from `settings.panel`;
- its `emulator.panel` items: sections, actions and blocks;
- the host folders it asks for, except the shared `games` and `packages`, which are in Settings.
  With settings, a folder row comes after the field its grant's `after` names, or after the last
  field.

A plugin that is turned off shows only its row. A theme can't be turned off.

## Errors

A plugin returns `PluginError` (`error.rs`):

| Variant | Use |
| --- | --- |
| `NotFound` | No such game, version or file. |
| `InvalidInput` | Bad input. `InvalidPath`, `InvalidId` and `serde_json::Error` turn into it. |
| `PermissionDenied` | The host refused, or the plugin may not. |
| `Network` | HTTP failed. |
| `Storage` | A file operation failed. |
| `Unsupported` | Not on this system, or not for this game. |
| `Precondition` | Something must happen first, such as installing a build. |
| `OwnBuildMissing { has_downloaded, folder, program }` | The own build is turned on, but it isn't in the folder or on the `PATH`. The host words it for the player. |
| `Timeout`, `Panicked` | Set by the host's guard. |
| `Other` | Anything else. |

## Shared code: emulator-kit

`plugins/shared/emulator-kit` is a library, not a plugin. The PS5, PS4, Switch and PC plugins use
it. It depends on `launcher-api` as a plugin does, plus `tokio`, and the vendored Kraken decoder
(`third_party/oozextract`) for PS5 packages.

A console emulator describes itself, and the kit runs the rest. The plugin gives a `Flavor` for
its builds, which the `BuildManager` installs, updates and rolls back. It gives a `ConsoleSpec`
for its games, compatibility list and page, which the `ConsoleEmulator` serves. A plugin with a
block of its own on its panel gives a `Panel` with `with_panel`: the
`ConsoleEmulator` shows its items after its own and offers it each action first. The plugin keeps
its settings type, its launch arguments and what its game installs copy. The kit reaches the host
only through the host facades in `host::Host`, never the whole `PluginContext`, so its tests run
on in-memory doubles.

| Module | What it holds |
| --- | --- |
| `archive_sets` | The archive sets of a download, by name, as the host unpacks them. |
| `args` | Extra arguments the player types, split as a shell would. |
| `backup` | `Items`: a fixed list of backup items. |
| `builds` | `Flavor`, `BuildManager`: official builds from GitHub releases, and the own build. |
| `cache` | JSON from the web, kept in the plugin's storage for a while. |
| `compat` | Community compatibility lists, by title id, and their report forms. |
| `console` | `ConsoleSpec`, `ConsoleEmulator`, `Panel`: a console emulator as a plugin. |
| `controls` | A console emulator's keyboard keys, for its panel. |
| `exfat` | Read-only exFAT disk images. |
| `fail` | A small error type for the package readers. |
| `game_install` | The steps of a game install that are the same for every emulator: `install`. |
| `games` | Installed console games in a host folder (PS5, PS4 and Switch files). |
| `github` | GitHub releases, as the REST API returns them. |
| `host` | `Host`: the host facades the kit uses. |
| `installs` | Installing a game from a package or disc image: the trees a `GameInstaller` reads. |
| `pkg5` | A reader for PS5 debug packages. Retail packages are refused. |
| `read_at` | What the package readers read from: a file on disk or in a host folder. |
| `settings` | An emulator's settings type, stored by the host, read over this system's defaults. |
| `sfo` | PS4 `param.sfo` metadata. |
| `versions` | Emulator versions under `builds/<tag>`, with the current build and the one before it, in `versions.json`. |

The `fixtures` feature adds builders of synthetic packages and exFAT images for other crates'
tests.

## Testing

`launcher-testkit` (`crates/testkit`) runs a real core on a fake host:

- `FakeHost`: a real core over a temporary data folder, with fake HTTP, fake processes and a
  `FakeClock`. Add plugins with `.plugin(StubPlugin)`, `.built_in(PluginBox)` or
  `.third_party(PluginBox)`, then `.boot_core().await` for a `TestCore`, or `.boot().await` for
  the kernel only. `.http().serve(url, bytes)` answers a URL.
- `StubPlugin`: a plugin from a manifest and behaviour the test picks.
  `StubPlugin::from_toml(...)`, then `.provides(|ctx| Ok(Provided { ... }))`. It can also
  `panics_on_init`, `hangs_on_init`, `fails_on_init` and `hangs_on_shutdown`, and it counts
  `init_calls` and `shutdown_calls`.
- `FakeBackup`: backup items a test sets. `StubPlugin::backs_up(id, permissions, backup)` is a
  plugin that provides only `backup`, with `permissions` as TOML for its `[permissions]`.
- `FakeCatalog`: an emulator with a catalog of made-up games, built with `.build()`. `FakeStore`
  builds a store, and `FakeService` a service that fills slots. `FakeEmulator` is an emulator a
  test scripts, from its catalog to its launch.
- `contract::emulator(&test, &plugin_id)` runs the contract of each capability the plugin
  declares: catalog, downloader, configurator, compatibility, keymap, slots, installer and
  launcher. `contract::game_install(&test, &plugin_id, &source)` checks a game installer. A
  broken rule panics with `contract <name>: ...`.

A backup test, as `tests/tests/seams/backup.rs` does it:

```rust
let items = vec![
    BackupItem::new("saves", "Saves", BackupPlace::Folder(HostPath::new("emu-saves", 0, RelPath::root())))
        .skip(&["cache"]),
];
let plugin = StubPlugin::backs_up(
    "emu-b",
    "[[permissions.filesystem]]\nfolder = \"emu-saves\"\nwrite = true\n",
    FakeBackup::new(items),
);
let test = FakeHost::new().built_in(plugin.boxed()).boot_core().await;
```

Then map the folders with `Command::SetFolders`, set `Pref::BackupFolder`, and send
`Command::Backup(BackupCmd::BackUp { large: false })`.

The commands that run the tests are in [Development](../contributors/development.md#tests).

## The built-in plugins

Each one is a working example. The capabilities are the ones in each `plugin.toml`.

| Plugin | Folder | Id | Systems | Capabilities |
| --- | --- | --- | --- | --- |
| PS5: KytyPS5 | `plugins/emulators/kytyps5` | `emu-kyty` | Linux, macOS, Windows | `catalog`, `downloader`, `installer`, `configurator`, `launcher`, `compatibility`, `slots`, `game_installer`, `backup` |
| PS4: shadPS4 | `plugins/emulators/shadps4` | `emu-shadps4` | Linux, macOS, Windows | the same as KytyPS5 |
| Switch: Lichii | `plugins/emulators/lichii` | `emu-lichii` | Linux, macOS, Windows | `catalog`, `downloader`, `installer`, `configurator`, `launcher`, `slots`, `game_installer`, `backup` |
| PC | `plugins/emulators/pc` | `emu-pc` | Linux, macOS, Windows | `catalog`, `launcher`, `slots`, `game_installer`, `configurator`, `backup` |
| Steam | `plugins/emulators/steam` | `emu-steam` | Linux, macOS, Windows | `catalog`, `launcher`, `slots`, `releases`, `metadata` |
| Game Catalog | `plugins/services/release-catalog` | `release-catalog` | Linux, macOS, Windows | `releases`, `configurator`, `slots` |
| PlayStation Store | `plugins/services/playstation-store` | `playstation-store` | Linux, macOS, Windows | `metadata` |
| This PC | `plugins/system/host` | `system` | Linux, macOS, Windows | `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install`, `drives` |

Notes on each:

- **KytyPS5.** Builds come from KytyPS5's GitHub releases, through the kit's build manager.
  Games are PS5 game folders in the `games` folders. It installs PS5 game folders, exFAT images
  and unsigned PS5 packages. A build with `--data-folder` keeps its saves in the shared
  `kyty-data` folder, which is the plugin's backup item.
- **shadPS4.** Builds come from shadPS4's official releases. On Linux the build is an AppImage
  in a zip (`AppImageInZip`), which the host takes out and extracts. Games are PS4 game folders.
  Backup: its data folder without caches and logs, and its DLC as a large item. Its
  `[system_files]` take the PS4 system modules and fonts from `ps4-system-files.zip`.
- **Lichii.** Builds are Lichii's AppImage (`AppImage`). Games are Switch game files. Its backup
  items are in its folders `lichii-data` and `lichii-config` and in its own data folder. It grants
  `lichii-config` read only: the restore writes there anyway. Its `[system_files]` take the
  Switch firmware and the keys of the same version from `switch-firmware-keys.zip`.
- **PC.** No builds of its own. It installs PC games from a download folder, finds what starts
  them, and runs a Windows game through umu-launcher or Wine on Linux and macOS. On Linux a
  Windows game runs in a sandbox. Backup: each game's Wine prefix, as a large item. The details
  are in [Archives and PC games](../contributors/archives-and-pc-games.md).
- **Steam.** Its library is the games Steam installed, read from the `steam` grant, which uses
  `listed_in` for Steam's libraries on other drives. Play hands the game to Steam and follows it
  with `LaunchSpec::game_process`: on Linux by Steam's reaper, on macOS and Windows by the game's
  folder. Install and uninstall open `steam://` links that `permissions.links` allows.
- **Game Catalog.** One catalog file for each console (`ps5.json`, `ps4.json`, `pc.json`,
  `switch.json`), from a catalog address, a copy fetched before, or the copy bundled from
  `assets/catalog/`. How the files are made is in `scripts/catalog/README.md`.
- **PlayStation Store.** It describes PS4 and PS5 games by title id from Sony's public catalog
  API.
- **This PC.** It runs the system's own tools. On Linux it provides every system capability. On
  macOS it runs only the tools macOS ships (`src/macos.rs`; their output is read in
  `src/macmap.rs`) and provides:
  - `network`: how the Mac is online (`scutil --nwi`, with the ports of `networksetup
    -listallhardwareports`), the saved Wi-Fi networks (`-listpreferredwirelessnetworks`), and
    joining one (`-setairportnetwork`). macOS has no built-in tool that scans since 14.4
    (`airport` does nothing), so Scan is refused and the list holds the saved networks, with no
    signal. Since macOS 14 it hides the joined network's name from a program without Location
    Services: the network then shows as "Wi-Fi". Forgetting a network needs an administrator,
    so it is refused.
  - `vpn`: every network with an address, and the player picks the one the VPN uses. The VPN's
    own app connects it. `scutil --nc start` and `stop` could switch the VPNs System Settings
    knows, but those have no interface to pick, and while the system can't tell VPNs apart a
    row's only action is the pick, so they aren't listed. Adding, connecting and disconnecting a
    VPN are refused.
  - `bluetooth`: whether it is on, and the paired devices (`system_profiler SPBluetoothDataType
    -json`, macOS 12 and later). macOS has no built-in tool that turns Bluetooth on or off,
    pairs, connects or forgets a device, so those are refused.
  - `audio`: the volume and mute (`osascript`, `get volume settings`). macOS has no built-in
    tool that lists or chooses the outputs (`SwitchAudioSource` isn't built in), so the list is
    empty and choosing one is refused.
  - `power`: the battery (`pmset -g batt`), and restart and shut down through System Events
    (`osascript`). `shutdown` needs root. macOS asks once whether the launcher may control
    System Events; after "Don't Allow" the message says where to allow it.
  - `drives`: the volumes on the external disks (`diskutil list -plist external`, then
    `diskutil info -plist` for each), with label, size, free space and mount state, and mount,
    unmount and eject (`diskutil mount`, `unmount`, `eject`). An APFS volume ejects with the
    physical disk that holds its container. The Mac's own disks aren't listed. No plist crate is
    in the workspace, so `macmap.rs` reads the XML that `diskutil` prints.
  - OS updates and the OS install are absent.

  On Windows it uses Windows' own calls
  (`src/windows.rs`, through the `windows` crate) and provides:
  - `network`: how the PC is online, the Wi-Fi networks in reach, and joining and forgetting
    one, through the WLAN API. Not `netsh`: it prints in the language Windows is set to. Since
    its fall 2024 release, Windows 11 lists the networks to a program only with the player's
    consent to location; without it the list is empty and Scan says why. A network that signs in with an account (802.1X) is joined in Windows'
    own settings.
  - `bluetooth`: the radio on and off, the paired devices, a scan, pairing and forgetting,
    through WinRT, for classic and Bluetooth LE devices. Pairing accepts only a device that asks
    nothing; one that asks for a PIN or a code is paired in Windows' own settings. WinRT has no
    call to connect or disconnect a paired device: Windows connects it when it is on.
  - `audio`: the outputs and the default output's volume and mute, through Core Audio. Windows
    has no public call to change the default output, so choosing one is refused.
  - `power`: the battery (`GetSystemPowerStatus`), and restart and shut down
    (`shutdown /r /t 0`, `shutdown /s /t 0`).
  - `drives`: each volume with a letter, named "Games (E:)", with its free space, and the eject
    of a USB drive (Configuration Manager's `CM_Request_Device_EjectW` on the first device
    Windows marks removable, as "Safely remove" does). Windows mounts drives by itself, so
    mount and unmount are refused. A network drive is listed by its letter only.
  - `vpn`: unsupported, as librqbit can't bind a download to an interface there. OS updates and
    the OS install are absent.

The two themes, Default and Classic (`plugins/themes/default`, `plugins/themes/classic`), are in
[Writing a theme](../themes/writing-a-theme.md).

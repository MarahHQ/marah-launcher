# Writing a plugin

This page takes you from an empty folder to a built-in plugin that loads, runs and passes its
tests. It is for Rust developers who add an emulator, a game source or a service to the
launcher. Every key, trait and rule it names is in full in the [plugin reference](reference.md).

A theme is a plugin too, but it has no code: see [Writing a theme](../themes/writing-a-theme.md).
How plugins sit in the core is in [the architecture](../contributors/architecture.md). The words
this page uses are in the [glossary](../../GLOSSARY.md).

## What a plugin is

A plugin is a Rust type that gives the host its manifest. The host reads the manifest, checks it,
and then calls the plugin only through the traits in `launcher-api` (`crates/api`). A plugin
depends on `launcher-api` alone. That crate re-exports `async_trait`, `serde`, `serde_json`,
`schemars`, `semver` and `inventory`, so the plugin needs no other crate for them.

A plugin with code implements two traits (`crates/api/src/plugin.rs`):

- `Plugin`: `manifest()`, and `shutdown()`, which has a default body.
- `CodePlugin`: `init(ctx)`. It returns the plugin's capabilities as a `Provided` value.

A capability is one thing a plugin does for the host, such as list games (`catalog`) or start
them (`launcher`). The manifest declares the capabilities, and `init` must provide exactly those.

### Kinds

The manifest's `kind` is `emulator`, `service`, `system` or `theme`. The kind decides only the
privileges ([ADR 0001](../adr/0001-capabilities-follow-the-capability-not-the-kind.md)):

- Any plugin with code may provide any data capability, such as `catalog`, `releases`,
  `metadata`, `slots` or `backup`.
- Only a `system` plugin may provide the system capabilities (network, VPN, Bluetooth, sound,
  power, OS updates, OS install), and run programs with no time limit or through polkit.
- A `theme` has no code and declares only `theme`.

Pick `emulator` for a plugin that runs games, and `service` for one that adds data to games
other plugins find, such as a store's art.

### Built in, not on disk

A plugin with code is always built into the launcher's binary. The host has no runtime for code
from disk. A plugin folder on disk can only be a theme: the kernel refuses a third-party manifest
with `[runtime] type = "native"`, and the only loader for a folder accepts `kind = "theme"` with
`type = "data"`. So you write a plugin as a crate in this repository.

## 1. Make the crate

A built-in plugin is one crate under `plugins/<group>/<name>/`, with `plugin.toml` beside its
`Cargo.toml`. The groups are `emulators`, `services`, `system`, `themes` and `shared`. Copy one of
the plugins there to start. The PC plugin (`plugins/emulators/pc`) is a small one.

Its `Cargo.toml` takes `version`, `edition`, `rust-version`, `publish` and `license` from the
workspace, and sets `[lints] workspace = true`.

`src/lib.rs` holds the plugin. This sketch follows `plugins/emulators/pc/src/lib.rs`:

```rust
use std::sync::Arc;

use launcher_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for MyPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let me = Arc::new(My { ctx });
        Ok(Provided {
            catalog: Some(me.clone()),
            launcher: Some(me),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(MyPlugin)) }
```

`inventory::submit!` registers the plugin's factory at compile time. The `inventory` feature of
`launcher-api` is on by default.

Keep the `PluginContext` that `init` gets, or the parts of it you need. It is the plugin's only
way to the host: its settings, its data folder, host folders, programs, HTTP, the log, events
and host services. See [The plugin context](reference.md#the-plugin-context).

An emulator for a console can use the shared library `emulator-kit` instead of writing builds,
games and pages itself. See [emulator-kit](reference.md#shared-code-emulator-kit).

## 2. Write the manifest

`plugin.toml` names the plugin, says what it provides, and lists what it may do outside its
data folder. A short manifest for an emulator:

```toml
id = "emu-example"
kind = "emulator"
name = "PS5"
emulator = "Example"
version = "0.1.0"
host_api = "^0.7"
platforms = ["linux", "macos", "windows"]
console = "PS5"
capabilities = ["catalog", "launcher"]

[permissions]
network = ["api.github.com", "github.com", "*.githubusercontent.com"]
process = ["builds/*/example"]

[[permissions.filesystem]]
folder = "games"
write = true

[system]
id = "PS5"
name = "PlayStation 5"
icon = "icon.png"
cover = "cover.jpg"
```

- `id` names the plugin's data folder and its settings. It stays the same for the plugin's life.
- `host_api` is a semver range. It must include the host's `API_VERSION`, `0.7.0` now
  (`crates/api/src/lib.rs`).
- `capabilities` must match what `init` returns, or the host refuses the plugin.
- `[permissions]` is the plugin's whole reach: hosts for HTTP, programs it may start, and named
  host folders. The host refuses anything else.

Every key is in [The manifest](reference.md#the-manifest).

`[system]` points at `icon.png` and `cover.jpg` in the plugin's assets. Give them from `init`
with `assets: Some(launcher_api::system_art!())`, which serves the crate's `assets/icon.png`
and `assets/cover.jpg`. `scripts/make-system-art.sh` makes the two files.

## 3. Register the crate

Nothing in the app calls a plugin crate, so the binary links it only when you name it.

1. Add the crate's path to `members` in the root `Cargo.toml`, and to
   `[workspace.dependencies]`: `my-plugin = { path = "plugins/<group>/<name>" }`.
2. Add `my-plugin.workspace = true` to `[dependencies]` in `crates/app/Cargo.toml`, in the block
   under `# Linked so their compile-time factories register. Don't remove as "unused".`
3. Add `use my_plugin as _;` to the list under `// Linked for their compile-time plugin
   factories.` in `crates/app/src/main.rs`.
4. For seam tests, add the crate to `[dev-dependencies]` in `tests/Cargo.toml`.

Without steps 2 and 3, the factory never links and the plugin never loads.

## 4. Know the lifecycle

The host takes each plugin through three steps:

1. **Load.** The host builds the plugin and reads `manifest()`. The kernel checks the manifest
   and rejects a plugin that breaks a rule. See
   [What the kernel checks](reference.md#what-the-kernel-checks).
2. **Init.** The host calls `init(ctx)` once. With `activation = "eager"` it does so at boot.
   With `lazy`, the default, it does so the first time it needs one of the plugin's
   capabilities. Dependencies init first.
3. **Shutdown.** The host calls `shutdown()` at most once for each activation, after the
   plugins that depend on it.

Every call into a plugin runs under a timeout and a panic guard (`crates/core/src/guard.rs`).
The timeout is 30 seconds for a call and 5 seconds for `shutdown` (`HostConfig::new` in
`crates/core/src/kernel.rs`). So start long work in the background and return at once. A panic
becomes `PluginError::Panicked`, and a call past its time becomes `PluginError::Timeout`.

A failed `init` marks the plugin failed. The host publishes `Event::PluginFailed`, and every
later call to the plugin gets an error. A rejected plugin shows in the log as `plugin not
loaded`, with the reason.

## 5. Test it

The test kit (`launcher-testkit`, `crates/testkit`) runs a real core on a fake host:

- `FakeHost` boots a real core over a temporary data folder, with fake HTTP, fake processes
  and a `FakeClock`. Add a plugin with `.built_in(PluginBox)`, then call `.boot_core().await`
  for a `TestCore`. `.http().serve(url, bytes)` answers a URL.
- `contract::emulator(&test, &plugin_id)` runs the contract of each capability the plugin
  declares: catalog, downloader, configurator, compatibility, keymap, slots, installer and
  launcher. A broken rule panics with `contract <name>: ...`.
- `contract::game_install(&test, &plugin_id, &source)` checks a game installer.

Run the contract suite on your plugin, as `tests/tests/seams/kyty.rs` does for KytyPS5:

```rust
let test = FakeHost::new().built_in(launcher_api::PluginBox::code(MyPlugin)).boot_core().await;
contract::emulator(&test, &PluginId::new("emu-example").unwrap()).await;
```

Put the plugin's seam tests in `tests/tests/seams/`, one module for each area. The other test
helpers (`StubPlugin`, `FakeCatalog`, `FakeEmulator`, `FakeBackup`) are in
[Testing](reference.md#testing).

Run the checks before you open a pull request. The commands are in
[Development](../contributors/development.md#tests).

## Where to look next

Each built-in plugin is a working example of some capabilities:

| To see | Read |
| --- | --- |
| A console emulator on `emulator-kit`, with a compatibility list | `plugins/emulators/kytyps5` |
| Startup work with a question, and backup items | `plugins/emulators/shadps4` |
| System files from an archive (`[system_files]`), and an AppImage build | `plugins/emulators/lichii` |
| A launcher with a sandbox, and a game installer for downloads | `plugins/emulators/pc` |
| A game another program starts, and store links | `plugins/emulators/steam` |
| A `releases` service with settings | `plugins/services/release-catalog` |
| A `metadata` service | `plugins/services/playstation-store` |
| The system capabilities | `plugins/system/host` |

The list of built-in plugins and their capabilities is in
[The built-in plugins](reference.md#the-built-in-plugins).

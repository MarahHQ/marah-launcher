//! shadPS4, the PS4 emulator, as a plugin.
//!
//! - Builds: shadPS4's official release. On Linux, a zip that holds an AppImage: the host
//!   checks the zip's SHA-256, takes the AppImage out and runs its `--appimage-extract`, so
//!   `builds/<tag>/squashfs-root/AppRun` runs without FUSE. On Windows, a zip with
//!   `shadPS4.exe`. On macOS, a zip with `shadps4` next to its Vulkan libraries. A new build
//!   runs once with `--help` before it becomes current; one that doesn't print shadPS4's
//!   name is removed. The kit's build manager runs the flow over [`ShadFlavor`].
//! - Games: PS4 game folders (`sce_sys/param.sfo`, category `gd`) in the player's
//!   game folders. Updates and add-ons have a param.sfo too, and are skipped.
//! - shadPS4 keeps saves and settings in its own folder (`~/.local/share/shadPS4`, on macOS
//!   `~/Library/Application Support/shadPS4`), which
//!   every build shares, so a build change moves nothing.
//! - Compatibility: the community list from `shadps4-compatibility`.
//! - Installs: PS4 packages and unpacked game folders, in [`install`].
//! - The PS4's system modules and fonts, which some games need: the start asks to install them
//!   when they are missing, in [`system_files`].

use std::sync::Arc;

use emulator_kit::builds::{Flavor, OwnBuild, ReleaseSource};
use emulator_kit::console::{CompatList, ConsoleEmulator, ConsoleSpec, Packages};
use emulator_kit::games::Console;
use emulator_kit::host::Host;
use emulator_kit::installs::Format;
use emulator_kit::{compat, versions};

mod first_user;
mod install;
pub mod key_map;
pub mod pkg4;
mod pkg4_data;
pub mod system_files;
use launcher_api::schemars::JsonSchema;
use launcher_api::serde::{Deserialize, Serialize};
use launcher_api::serde_json::Value;
use launcher_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");
// Nightly pre-releases come out daily; 30 still reaches the last official release.
pub const RELEASES: &str = "https://api.github.com/repos/shadps4-emu/shadPS4/releases?per_page=30";
/// The community list's page, for the player.
pub const LIST_PAGE: &str = "https://github.com/shadps4-compatibility/shadps4-game-compatibility/issues";
pub const COMPATIBILITY: &str = "https://github.com/shadps4-compatibility/shadps4-game-compatibility/releases/latest/download/compatibility_data.json";
pub const GAMES: &str = launcher_api::GAMES_FOLDER;
/// The console: it names its page, and its folder in the games and packages folders.
const CONSOLE: &str = "PS4";
/// Where you keep packages to install.
pub const PACKAGES: &str = launcher_api::PACKAGES_FOLDER;
/// shadPS4's data folder, where DLC goes (`addcont/<title id>/<label>`).
pub const DATA: &str = "shadps4-data";

/// shadPS4 settings. Its other settings live in its own config file.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", deny_unknown_fields)]
#[schemars(crate = "launcher_api::schemars", extend("x-order" = ["own_build", "fullscreen"]))]
pub struct Settings {
    #[serde(default = "yes")]
    #[schemars(title = "Full screen")]
    pub fullscreen: bool,
    /// Runs the shadPS4 you built or installed: from the folder you map as shadps4-build, or
    /// from your PATH. Installing an official build turns this off. Saves are shared: shadPS4
    /// keeps them in its own folder.
    #[serde(default)]
    #[schemars(title = "Use your own build")]
    pub own_build: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { fullscreen: true, own_build: false }
    }
}

impl emulator_kit::settings::Settings for Settings {
    type Game = GameSettings;

    fn own_build(&self) -> bool {
        self.own_build
    }
}

/// Per-game settings: anything set here wins over the shadPS4 settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", deny_unknown_fields)]
#[schemars(crate = "launcher_api::schemars")]
pub struct GameSettings {
    #[serde(default)]
    #[schemars(title = "Full screen")]
    pub fullscreen: Option<bool>,
}

/// The official build for this system: a zip with an AppImage on Linux, a zip with
/// `shadPS4.exe` on Windows, a zip with `shadps4` and its Vulkan libraries on macOS.
pub fn asset_matches(name: &str, platform: Option<Platform>) -> bool {
    let prefix = match platform {
        Some(Platform::Windows) => "shadps4-win64",
        Some(Platform::Macos) => "shadps4-macos",
        _ => "shadps4-linux",
    };
    name.starts_with(prefix) && name.ends_with(".zip")
}

/// "v.0.18.0" -> "0.18.0".
pub fn pretty(tag: &str) -> String {
    tag.trim_start_matches('v').trim_start_matches('.').to_owned()
}

/// The program a build starts.
fn program(tag: &str, platform: Option<Platform>) -> PluginResult<RelPath> {
    match platform {
        Some(Platform::Windows) => Ok(versions::folder(tag)?.join("shadPS4.exe")?),
        Some(Platform::Macos) => Ok(versions::folder(tag)?.join("shadps4")?),
        _ => Ok(versions::folder(tag)?.join("squashfs-root")?.join("AppRun")?),
    }
}

// shadPS4's default key layout, for the keyboard screen. shadPS4 keeps the
// layout in its own settings file, so the player's may differ.
pub const KEYS: &[(&str, &str)] = &[
    ("Cross", "N (or keypad 2)"),
    ("Circle", "B (or keypad 6)"),
    ("Square", "V (or keypad 4)"),
    ("Triangle", "C (or keypad 8)"),
    ("D-pad", "Arrow keys"),
    ("Options", "Enter"),
    ("Left stick", "W A S D"),
    ("Right stick", "I J K L"),
    ("L1 / R1", "Q / U"),
    ("L2 / R2", "E / O"),
    ("L3 / R3", "X / M"),
    ("Touchpad", "Space"),
];

/// The folder you map to your own shadPS4 build.
pub const OWN_BUILD: &str = "shadps4-build";
/// What your own build's program may be called, in its folder.
const OWN_PROGRAMS: [&str; 5] =
    ["Shadps4-sdl.AppImage", "Shadps4-qt.AppImage", "shadps4", "build/shadps4", "shadPS4.exe"];

/// shadPS4's builds. Every build shares shadPS4's own data folder, so it needs no hooks.
pub struct ShadFlavor {
    pub platform: Option<Platform>,
}

#[async_trait]
impl Flavor for ShadFlavor {
    type Settings = Settings;
    const NAME: &'static str = "shadPS4";
    const CONSOLE: &'static str = CONSOLE;

    fn release_source(&self, _settings: &Settings) -> PluginResult<ReleaseSource> {
        Ok(ReleaseSource { url: RELEASES.into(), cache: RelPath::new("cache/releases.json")?, prereleases: false })
    }

    fn asset_matches(&self, name: &str) -> bool {
        asset_matches(name, self.platform)
    }

    fn archive_format(&self, _asset: &str) -> ArchiveFormat {
        if self.platform == Some(Platform::Linux) || self.platform.is_none() {
            ArchiveFormat::AppImageInZip
        } else {
            ArchiveFormat::Zip
        }
    }

    fn pretty(&self, tag: &str) -> String {
        pretty(tag)
    }

    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
        let program = program(tag, self.platform)?;
        if !storage.exists(&program).await? {
            return Err(PluginError::Other(format!("shadPS4 {tag} has no {program}")));
        }
        Ok(program)
    }

    /// The test run's `--help` must print shadPS4's name.
    fn runs(&self, help: &str) -> bool {
        help.contains("shadPS4")
    }

    fn own_build(&self) -> OwnBuild {
        OwnBuild { folder: OWN_BUILD, itself: false, files: OWN_PROGRAMS.map(String::from).to_vec(), system: "shadps4" }
    }
}

pub struct ShadPlugin;

#[async_trait]
impl Plugin for ShadPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for ShadPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        init(ctx, system_files::table()).await
    }
}

/// The plugin with another list of PS4 system files in place of the real one: for tests, which
/// serve fake files.
#[cfg(feature = "fixtures")]
pub struct ShadWithFiles(pub Vec<system_files::SystemFile>);

#[cfg(feature = "fixtures")]
#[async_trait]
impl Plugin for ShadWithFiles {
    fn manifest(&self) -> PluginManifest {
        ShadPlugin.manifest()
    }
}

#[cfg(feature = "fixtures")]
#[async_trait]
impl CodePlugin for ShadWithFiles {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        init(ctx, self.0.clone()).await
    }
}

/// The plugin's parts, with `files` as the list of PS4 system files.
async fn init(ctx: PluginContext, files: Vec<system_files::SystemFile>) -> PluginResult<Provided> {
    let host = Host::of(&ctx);
    let spec = ConsoleSpec {
        console: Console::Ps4,
        packages: Packages::Formats(&[Format::Ps4Package]),
        compat: Some(CompatList { url: COMPATIBILITY, page: LIST_PAGE, parse: compat::shad }),
        keys: KEYS,
        section: "shadps4",
        version_row: true,
        own_row: false,
    };
    let system = system_files::SystemFiles::new(host.clone(), ctx.events().clone(), files);
    let emu =
        ConsoleEmulator::start(ShadFlavor { platform: host.platform }, spec, host).await?.with_panel(system.clone());
    let emu = Arc::new(emu);
    let shad = Arc::new(Shad { emu: emu.clone() });
    Ok(Provided {
        catalog: Some(shad.clone()),
        downloader: Some(emu.builds().clone()),
        installer: Some(emu.builds().clone()),
        configurator: Some(emu.clone()),
        launcher: Some(shad.clone()),
        compatibility: Some(emu.clone()),
        game_installer: Some(shad),
        slots: Some(emu),
        backup: Some(emulator_kit::backup::Items::provided(vec![
            // Its settings, keys, saves and trophies; not its caches, logs and downloads, and not
            // the PS4 system files, which the System files block downloads again.
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
        startup: Some(system),
        assets: Some(launcher_api::system_art!()),
        ..Provided::default()
    })
}

inventory::submit! { PluginFactory::code(|| Box::new(ShadPlugin)) }

/// A PS1 game that Sony sells for PS4: its folder has the PS1 emulator's `config-title.txt` and
/// the PS1 disc as a `.cue` in `data/`. A folder that can't be read counts as a PS4 game.
async fn ps1_game(host: &Host, dir: &HostPath) -> bool {
    let read = async {
        if !host.folders.exists(&dir.join("config-title.txt")?).await? {
            return Ok(false);
        }
        let data = host.folders.list(&dir.join("data")?).await?;
        PluginResult::Ok(data.iter().any(|e| !e.is_dir && e.name.to_ascii_lowercase().ends_with(".cue")))
    };
    read.await.unwrap_or(false)
}

struct Shad {
    emu: Arc<ConsoleEmulator<ShadFlavor>>,
}

#[async_trait]
impl Launcher for Shad {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.emu.games().find(self.emu.host().folders.as_ref(), game).await?;
        let builds = self.emu.builds();
        // `config` is the emulator settings with the game's settings on top.
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        let pad = if ps1_game(self.emu.host(), &found.dir).await { key_map::Pad::Ps1 } else { key_map::Pad::Ps4 };
        key_map::set_up(self.emu.host(), pad, &found.title_id).await;
        first_user::set_up(self.emu.host()).await;
        // shadPS4 takes the game folder with -g; -f true|false sets full screen.
        let args = vec!["-g".into(), LaunchArg::path(found.dir), "-f".into(), settings.fullscreen.to_string().into()];
        Ok(LaunchSpec {
            program: run.program,
            args,
            env: Default::default(),
            cwd: run.cwd,
            game_process: None,
            game_env: None,
            relaunch: None,
            sandbox: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_zip_and_its_tag() {
        let linux = Some(Platform::Linux);
        assert!(asset_matches("shadps4-linux-sdl-0.20.0.zip", linux));
        assert!(!asset_matches("shadps4-win64-sdl-2026-0.20.0.zip", linux));
        assert!(!asset_matches("shadps4-linux-0.18.0.tar.gz", linux));
        assert!(asset_matches("shadps4-win64-sdl-2026-0.20.0.zip", Some(Platform::Windows)));
        let mac = Some(Platform::Macos);
        assert!(asset_matches("shadps4-macos-sdl-2026-0.20.0.zip", mac));
        assert!(asset_matches("shadps4-macos-sdl-2026-10-17-e6c79a5.zip", mac));
        assert!(!asset_matches("shadps4-linux-sdl-0.20.0.zip", mac));
        assert_eq!(pretty("v.0.18.0"), "0.18.0");
    }
}

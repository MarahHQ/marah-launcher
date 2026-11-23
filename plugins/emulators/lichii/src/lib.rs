//! Lichii, the Switch and Switch 2 emulator (a fork of Eden and yuzu), as a plugin.
//!
//! - Builds: Lichii's GitHub releases, all of them pre-releases, none of them the `-pgo` files.
//!   - Linux: the bare AppImage (`lichii-linux-<arch>.AppImage`): the host checks its SHA-256
//!     and runs its `--appimage-extract`, so `builds/<tag>/squashfs-root/AppRun`, the Lichii
//!     program, runs without FUSE. The release's tar.gz needs FFmpeg libraries from the system,
//!     so it isn't used.
//!   - macOS: `lichii-macos-arm64.zip`, Lichii's app: `lichii.app/Contents/MacOS/lichii`. Lichii
//!     has no Intel Mac build.
//!   - Windows: `lichii-windows-x86_64.zip`: `lichii.exe` at its top.
//!
//!   A new build gets no test run: Lichii's `--help` opens its window and never ends.
//! - Games: Switch game files (`.nsp`, `.nsz`, `.xci`, `.xcz`, `.nup`) in the player's game
//!   folders, named by their file names (see [`emulator_kit::games::switch_file`]).
//! - Lichii keeps keys, firmware, saves and settings in its own folder (`~/.local/share/lichii`,
//!   on Windows `%APPDATA%\lichii`), which every build shares. Builds before build-150 used
//!   Eden's (`~/.local/share/eden`).
//!   The player imports the firmware and the keys of the same version from one archive on the
//!   emulator's page (`[system_files]` in the manifest): the host copies them.
//! - Installs: Switch game files from a download, in [`install`].
//! - No compatibility list: Lichii's community reports have no list to download.

use std::sync::Arc;

use emulator_kit::builds::{Flavor, OwnBuild, ReleaseSource};
use emulator_kit::console::{ConsoleEmulator, ConsoleSpec, Packages};
use emulator_kit::games::Console;
use emulator_kit::host::Host;
use emulator_kit::versions;
use launcher_api::schemars::JsonSchema;
use launcher_api::serde::{Deserialize, Serialize};
use launcher_api::serde_json::Value;
use launcher_api::*;

mod config;
mod install;
mod nsz;

const MANIFEST: &str = include_str!("../plugin.toml");
/// Every release is a pre-release; a build comes out several times a day.
pub const RELEASES: &str = "https://api.github.com/repos/zortos293/Lichii/releases?per_page=30";
/// The console, as its games' platform and the catalog name it: the manifest's `[system]` id.
const CONSOLE: &str = "SWITCH";
/// The folder of Switch games in the games folder.
pub const FOLDER: &str = "Switch";
/// Lichii's settings folder, where its window saves `qt-config.ini`.
pub const CONFIG: &str = "lichii-config";
/// Lichii's data folder, which every Lichii build shares: keys, firmware, saves and mods.
pub const DATA: &str = "lichii-data";
/// The exit code eden-cli asks to be started again with (`kRestartExitCode` in Lichii).
const RESTART_CODE: i32 = 75;

/// Lichii settings. Its other settings live in its own config file.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", deny_unknown_fields)]
#[schemars(crate = "launcher_api::schemars", extend("x-order" = ["own_build", "fullscreen", "gpu_split", "airplane_mode"]))]
pub struct Settings {
    #[serde(default = "yes")]
    #[schemars(title = "Full screen")]
    pub fullscreen: bool,
    /// How Lichii splits a Switch 2 game's GPU work between threads (EDEN_NX2_SPLIT). It wins
    /// over Lichii's nx2_switches.txt files.
    #[serde(default)]
    #[schemars(title = "Split the GPU work")]
    pub gpu_split: GpuSplit,
    /// Turns off the internet for games, as on a Switch in airplane mode. Off by default, so
    /// online games can connect. For the games Marah Launcher starts with an official build: your own
    /// build and Lichii's window use the airplane mode set in Lichii, which is on by default.
    #[serde(default)]
    #[schemars(title = "Airplane mode")]
    pub airplane_mode: bool,
    /// Runs the Lichii you built or installed: from the folder you map as lichii-build, or
    /// from your PATH. Installing an official build turns this off. Saves are shared: Lichii
    /// keeps them in its data folder.
    #[serde(default)]
    #[schemars(title = "Use your own build")]
    pub own_build: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { fullscreen: true, gpu_split: GpuSplit::default(), airplane_mode: false, own_build: false }
    }
}

impl emulator_kit::settings::Settings for Settings {
    type Game = GameSettings;

    fn own_build(&self) -> bool {
        self.own_build
    }
}

/// Per-game settings: anything set here wins over the Lichii settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", deny_unknown_fields)]
#[schemars(crate = "launcher_api::schemars")]
pub struct GameSettings {
    #[serde(default)]
    #[schemars(title = "Full screen")]
    pub fullscreen: Option<bool>,
    #[serde(default)]
    #[schemars(title = "Split the GPU work")]
    pub gpu_split: Option<GpuSplit>,
}

/// Lichii's split of a Switch 2 game's GPU work (EDEN_NX2_SPLIT, NX2_GPU_THREAD_SPLIT.md in
/// Lichii). Lichii's own default, 2, leaves some games black on some PCs (Mario Kart World on
/// AMD with Mesa's radv, build 94), so Marah Launcher starts from one thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde")]
#[schemars(crate = "launcher_api::schemars")]
pub enum GpuSplit {
    /// One thread (works in every game)
    #[default]
    #[serde(rename = "0")]
    Off,
    /// Engine work on a second thread
    #[serde(rename = "1")]
    Methods,
    /// Engine work and macros on a second thread (fastest; black screen in some games)
    #[serde(rename = "2")]
    Macros,
}

impl GpuSplit {
    /// The value of EDEN_NX2_SPLIT.
    pub fn value(self) -> &'static str {
        match self {
            Self::Off => "0",
            Self::Methods => "1",
            Self::Macros => "2",
        }
    }
}

/// The official build for this system and processor (`arch`, as Rust names it), not the
/// `-pgo` one: the Linux AppImage, or the macOS or Windows zip. Lichii names Apple Silicon
/// `arm64`.
pub fn asset_matches(name: &str, platform: Option<Platform>, arch: &str) -> bool {
    match platform {
        Some(Platform::Linux) | None => name == format!("lichii-linux-{arch}.AppImage"),
        Some(Platform::Macos) => {
            let arch = if arch == "aarch64" { "arm64" } else { arch };
            name == format!("lichii-macos-{arch}.zip")
        }
        Some(Platform::Windows) => name == format!("lichii-windows-{arch}.zip"),
    }
}

/// Where a build keeps its programs, in its folder: the AppImage's program and eden-cli, the
/// app's on macOS, the zip's top on Windows.
fn programs(platform: Option<Platform>) -> (&'static str, &'static str) {
    match platform {
        Some(Platform::Linux) | None => ("squashfs-root/AppRun", "squashfs-root/usr/bin/eden-cli"),
        Some(Platform::Macos) => ("lichii.app/Contents/MacOS/lichii", "lichii.app/Contents/MacOS/eden-cli"),
        Some(Platform::Windows) => ("lichii.exe", "eden-cli.exe"),
    }
}

/// The program a build starts: Lichii's window.
fn program(tag: &str, platform: Option<Platform>) -> PluginResult<RelPath> {
    Ok(versions::folder(tag)?.join(programs(platform).0)?)
}

/// The folder you map to your own Lichii build.
pub const OWN_BUILD: &str = "lichii-build";
/// What your own build's program may be called, in its folder.
const OWN_PROGRAMS: [&str; 5] =
    ["lichii", "lichii.AppImage", "build/bin/lichii", "lichii.exe", "lichii.app/Contents/MacOS/lichii"];

/// Lichii's builds. Every build shares Lichii's data folder, so it needs no hooks.
pub struct LichiiFlavor {
    pub platform: Option<Platform>,
}

#[async_trait]
impl Flavor for LichiiFlavor {
    type Settings = Settings;
    const NAME: &'static str = "Lichii";
    const CONSOLE: &'static str = CONSOLE;
    const FOLDER: &'static str = FOLDER;
    const TEST_RUN: bool = false;
    const HAS_WINDOW: bool = true;

    fn release_source(&self, _settings: &Settings) -> PluginResult<ReleaseSource> {
        Ok(ReleaseSource { url: RELEASES.into(), cache: RelPath::new("cache/releases.json")?, prereleases: true })
    }

    fn asset_matches(&self, name: &str) -> bool {
        asset_matches(name, self.platform, std::env::consts::ARCH)
    }

    fn archive_format(&self, asset: &str) -> ArchiveFormat {
        if asset.ends_with(".zip") { ArchiveFormat::Zip } else { ArchiveFormat::AppImage }
    }

    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
        let program = program(tag, self.platform)?;
        if !storage.exists(&program).await? {
            return Err(PluginError::Other(format!("Lichii {tag} has no {program}")));
        }
        Ok(program)
    }

    /// Not used: Lichii's builds get no test run ([`Flavor::TEST_RUN`]).
    fn runs(&self, _help: &str) -> bool {
        true
    }

    fn own_build(&self) -> OwnBuild {
        OwnBuild { folder: OWN_BUILD, itself: false, files: OWN_PROGRAMS.map(String::from).to_vec(), system: "lichii" }
    }
}

pub struct LichiiPlugin;

#[async_trait]
impl Plugin for LichiiPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for LichiiPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let host = Host::of(&ctx);
        let spec = ConsoleSpec {
            console: Console::Switch,
            // A Switch game file, not an update or DLC.
            packages: Packages::Named(|name| emulator_kit::games::switch_file(name).is_some()),
            compat: None,
            // Lichii's keyboard keys aren't listed here: its own window sets them.
            keys: &[],
            section: "lichii",
            version_row: true,
            own_row: false,
        };
        let platform = host.platform;
        let emu = ConsoleEmulator::start(LichiiFlavor { platform }, spec, host).await?;
        let emu = Arc::new(emu);
        let controllers = ctx.services().get::<Controllers>().map(|c| (*c).clone());
        let lichii = Arc::new(Lichii { emu: emu.clone(), controllers, platform });
        Ok(Provided {
            catalog: Some(emu.clone()),
            downloader: Some(emu.builds().clone()),
            installer: Some(emu.builds().clone()),
            configurator: Some(emu.clone()),
            launcher: Some(lichii.clone()),
            game_installer: Some(lichii.clone()),
            slots: Some(emu.clone()),
            backup: Some(backup_items()),
            assets: Some(launcher_api::system_art!()),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(LichiiPlugin)) }

/// What a backup holds of Lichii: from Lichii's data folder, the keys, the NAND (firmware and
/// saves), the SD card, amiibo and mods; and the settings its window saves. A backup an older
/// version made may hold `firmware-settings` too: a restore skips it.
fn backup_items() -> std::sync::Arc<dyn Backup> {
    let data = |path: &str| BackupPlace::Folder(HostPath::new(DATA, 0, RelPath::new(path).expect("a plain path")));
    emulator_kit::backup::Items::provided(vec![
        BackupItem::new("keys", "Switch keys", data("keys")),
        BackupItem::new("nand", "Switch firmware and saves", data("nand")),
        BackupItem::new("sdmc", "Switch SD card", data("sdmc")),
        BackupItem::new("amiibo", "amiibo", data("amiibo")),
        BackupItem::new("load", "Switch mods", data("load")),
        BackupItem::new(
            "settings",
            "Lichii settings",
            BackupPlace::Folder(HostPath::new(CONFIG, 0, RelPath::new(config::QT_CONFIG).expect("a plain path"))),
        ),
    ])
}

struct Lichii {
    emu: Arc<ConsoleEmulator<LichiiFlavor>>,
    /// The host's controllers; `None` on a host without them.
    controllers: Option<Controllers>,
    platform: Option<Platform>,
}

impl Lichii {
    /// The command-line program of the official build `tag`, when it has one.
    async fn cli(&self, tag: &str) -> Option<RelPath> {
        let cli = versions::folder(tag).ok()?.join(programs(self.platform).1).ok()?;
        self.emu.host().storage.exists(&cli).await.ok()?.then_some(cli)
    }
}

#[async_trait]
impl Launcher for Lichii {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.emu.games().find(self.emu.host().folders.as_ref(), game).await?;
        let builds = self.emu.builds();
        // `config` is the emulator settings with the game's settings on top.
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        // An official build starts the game in its command-line program: no Lichii window
        // before or after, and the controllers from Marah Launcher's settings file. Your own build
        // starts Lichii as it is.
        let cli = match &run.tag {
            Some(tag) => self.cli(tag).await,
            None => None,
        };
        let mut args = Vec::new();
        // eden-cli exits with 75 when a game starts another of its programs (a collection
        // of games, a restart after a setting): it is started again with
        // EDEN_CLI_RESTART_HANDOFF set, and reads where the game was from Lichii's folder.
        let relaunch = cli.is_some().then(|| Relaunch {
            code: RESTART_CODE,
            env: [("EDEN_CLI_RESTART_HANDOFF".to_owned(), "1".to_owned())].into(),
        });
        let program = match cli {
            Some(cli) => {
                let connected = match &self.controllers {
                    Some(controllers) => controllers.connected().await,
                    None => Vec::new(),
                };
                let file = config::write(self.emu.host(), &connected, &settings).await?;
                args.extend(["-c".into(), LaunchArg::path(Location::Data(file))]);
                Location::Data(cli)
            }
            None => run.program,
        };
        // Lichii takes the game file with -g; -f starts it in full screen.
        if settings.fullscreen {
            args.push("-f".into());
        }
        args.extend(["-g".into(), LaunchArg::path(found.file.unwrap_or(found.dir))]);
        let env = [("EDEN_NX2_SPLIT".to_owned(), settings.gpu_split.value().into())].into();
        Ok(LaunchSpec { program, args, env, cwd: run.cwd, game_process: None, game_env: None, relaunch, sandbox: None })
    }

    /// Lichii's window: its controls, keys, firmware and graphics settings. A game started from
    /// it gets the same GPU split as one Marah Launcher starts.
    async fn open_emulator(&self, config: &Value) -> PluginResult<Option<LaunchSpec>> {
        let builds = self.emu.builds();
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        let env = [("EDEN_NX2_SPLIT".to_owned(), settings.gpu_split.value().into())].into();
        Ok(Some(LaunchSpec {
            program: run.program,
            args: Vec::new(),
            env,
            cwd: run.cwd,
            game_process: None,
            game_env: None,
            relaunch: None,
            sandbox: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_appimage_for_this_processor_and_no_other_asset() {
        let linux = Some(Platform::Linux);
        assert!(asset_matches("lichii-linux-x86_64.AppImage", linux, "x86_64"));
        assert!(asset_matches("lichii-linux-aarch64.AppImage", linux, "aarch64"));
        assert!(!asset_matches("lichii-linux-x86_64-pgo.AppImage", linux, "x86_64"));
        assert!(!asset_matches("lichii-linux-x86_64.tar.gz", linux, "x86_64"));
        assert!(!asset_matches("lichii-linux-aarch64.AppImage", linux, "x86_64"));
        assert!(!asset_matches("eglantine-nx2-build-34-linux-x86_64.AppImage", linux, "x86_64"));
        assert!(!asset_matches("lichii-windows-x86_64.zip", linux, "x86_64"));
    }

    #[test]
    fn the_macos_zip_for_apple_silicon_and_the_windows_zip_and_no_pgo_one() {
        let (mac, windows) = (Some(Platform::Macos), Some(Platform::Windows));
        assert!(asset_matches("lichii-macos-arm64.zip", mac, "aarch64"), "Lichii names Apple Silicon arm64");
        assert!(!asset_matches("lichii-macos-arm64.dmg", mac, "aarch64"), "the zip, not the disk image");
        assert!(!asset_matches("lichii-macos-arm64.zip", mac, "x86_64"), "no Intel Mac build");
        assert!(!asset_matches("lichii-linux-aarch64.AppImage", mac, "aarch64"));
        assert!(asset_matches("lichii-windows-x86_64.zip", windows, "x86_64"));
        assert!(!asset_matches("lichii-windows-x86_64-pgo.zip", windows, "x86_64"));
        assert!(!asset_matches("lichii-macos-arm64.zip", windows, "x86_64"));
    }
}

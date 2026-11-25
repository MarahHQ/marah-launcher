//! RetroArch, the one program the retro consoles' plugins (`emu-nes`, `emu-genesis`, `emu-ps1`)
//! share. Each plugin names its console ([`RetroConsole`]); the kit runs the rest.
//!
//! - One copy: the plugins share a data folder (`shared_data = "retroarch"` in their manifests),
//!   so the builds, `versions.json` and the configs below are one, whichever plugin installs.
//!   The host runs one install of them at a time.
//! - Builds: RetroArch's stable release from libretro's build server, pinned here with each
//!   file's size and SHA-256 ([`PinnedBuild`]): the server publishes no checksums, so a new
//!   RetroArch comes with a new release of the launcher. A build is RetroArch, then the cores of every
//!   retro plugin out of the release's pack of cores (`pack/`), only those. Linux keeps the
//!   AppImage, its menu's assets, its controller profiles and the three consoles' databases;
//!   Windows keeps the whole program. macOS has no pack of cores: it gets no build.
//! - Configs: `retroarch.cfg`, made once and RetroArch's own from then (what the player changes
//!   in RetroArch's menu stays), and `launcher.cfg`, written at each start and loaded over it
//!   (`--appendconfig`): the folders, all relative to the shared data folder, which games
//!   start in. On Linux a game starts with `XDG_CONFIG_HOME` in the shared data folder
//!   ([`CONFIG_HOME`]): RetroArch writes what no config names (its core options, cheats,
//!   history) under it, so it never reads or writes the player's own `~/.config/retroarch`.
//!   For that reason a new build gets no `--help` test run either: the host runs it without
//!   that variable, and it would make the player's folder. The checksums say the files are
//!   the ones pinned.
//! - Games: the console's files in its folder of each games folder ([`RomSystem`]).

use std::marker::PhantomData;
use std::sync::Arc;

use launcher_api::schemars::JsonSchema;
use launcher_api::serde::{Deserialize, Serialize};
use launcher_api::serde_json::Value;
use launcher_api::*;

use crate::builds::{Flavor, OwnBuild, PinnedBuild, PinnedFile, ReleaseSource};
use crate::console::{ConsoleEmulator, ConsoleSpec, Packages};
use crate::game_install::{self, Part};
use crate::games::{Console, RomSystem, rom_title};
use crate::host::Host;
use crate::installs::{self, DownloadFile, HostTree};
use crate::versions;

/// The RetroArch release every retro plugin installs.
pub const VERSION: &str = "1.22.2";
/// The release's folder on libretro's build server.
const SERVER: &str = "https://buildbot.libretro.com/stable/1.22.2";
/// The cores of every retro plugin: a build holds them all, so any plugin's install serves
/// the others.
pub const CORES: [&str; 3] = ["fceumm", "genesis_plus_gx", "pcsx_rearmed"];

/// The Linux build's folder, and RetroArch's own folder beside its AppImage.
const LINUX: &str = "RetroArch-Linux-x86_64";
const LINUX_HOME: &str = "RetroArch-Linux-x86_64/RetroArch-Linux-x86_64.AppImage.home/.config/retroarch";
/// The Windows build's folder.
const WINDOWS: &str = "RetroArch-Win64";
/// The folder of a build the pack of cores unpacks into.
const PACK: &str = "pack";

/// RetroArch's value for Start and Select together, as a gamepad combo.
pub const MENU_COMBO: &str = "4";
/// RetroArch's `XDG_CONFIG_HOME` on Linux, in the shared data folder.
pub const CONFIG_HOME: &str = "config-home";
/// The base config, RetroArch's own once made, and the one written at each start over it.
pub const BASE_CONFIG: &str = "retroarch.cfg";
pub const START_CONFIG: &str = "launcher.cfg";
/// The folders RetroArch keeps the player's things in, in the shared data folder.
/// RetroArch keeps each core's saves and save states in a folder named after the core
/// ([`RetroConsole::CORE_NAME`]), and its options and remaps under `config` the same way.
pub const DATA_DIRS: [(&str, &str); 8] = [
    ("system_directory", "system"),
    ("savefile_directory", "saves"),
    ("savestate_directory", "states"),
    ("screenshot_directory", "screenshots"),
    ("playlist_directory", "playlists"),
    ("log_dir", "logs"),
    ("rgui_config_directory", "config"),
    ("input_remapping_directory", "config/remaps"),
];

/// One retro console, as its plugin names it.
pub trait RetroConsole: Send + Sync + 'static {
    /// The console, as the manifest's `console` (`NES`).
    const CONSOLE: &'static str;
    /// Its game files, and its folder in each games folder.
    const SYSTEM: &'static RomSystem;
    /// Its core, one of [`CORES`] (`fceumm`).
    const CORE: &'static str;
    /// The id of its section on a game's page.
    const SECTION: &'static str;
    /// The core's name, as RetroArch names its folders of saves, states and options
    /// (`FCEUmm`).
    const CORE_NAME: &'static str;
    /// The console has BIOS files the player may add (`system/`), which backups keep.
    const BIOS: bool = false;
}

/// What a backup keeps of console `C`, from the shared folder: its saves and save states, its
/// core's options and remaps, its BIOS files, and RetroArch's own settings, which every retro
/// plugin lists. RetroArch's builds download again.
pub fn backup_items<C: RetroConsole>() -> Vec<BackupItem> {
    let data = |path: String| BackupPlace::Data(RelPath::new(path).expect("a plain path"));
    let (core, console) = (C::CORE_NAME, C::CONSOLE);
    let mut items = vec![
        BackupItem::new("saves", format!("{console} saves"), data(format!("saves/{core}"))),
        BackupItem::new("states", format!("{console} save states"), data(format!("states/{core}"))),
        BackupItem::new("options", format!("{core} options"), data(format!("config/{core}"))),
        BackupItem::new("remaps", format!("{core} button remaps"), data(format!("config/remaps/{core}"))),
    ];
    if C::BIOS {
        items.push(BackupItem::new("bios", format!("{console} BIOS files"), data("system".into())));
    }
    items.push(BackupItem::new("retroarch-settings", "RetroArch settings", data(BASE_CONFIG.into())));
    items
}

/// What the player sets for a retro console's games.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", default)]
#[schemars(crate = "launcher_api::schemars")]
pub struct RetroSettings {
    /// Use your own RetroArch (in your PATH, or a folder you map) instead of the official one.
    /// Its own cores serve when no official build is installed.
    pub own_build: bool,
    /// Start games in full screen.
    pub fullscreen: bool,
    /// How the games' pixels fill the screen.
    pub picture: Picture,
}

impl Default for RetroSettings {
    fn default() -> Self {
        Self { own_build: false, fullscreen: true, picture: Picture::default() }
    }
}

/// A game's own settings: a set one wins over the console's.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde", default)]
#[schemars(crate = "launcher_api::schemars")]
pub struct RetroGameSettings {
    /// Start this game in full screen.
    pub fullscreen: Option<bool>,
    /// How this game's pixels fill the screen.
    pub picture: Option<Picture>,
}

/// How RetroArch draws a game's pixels on the screen. The consoles' pictures are small (the
/// NES's is 256×240), so RetroArch scales them up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "launcher_api::serde")]
#[schemars(crate = "launcher_api::schemars")]
pub enum Picture {
    /// Sharp pixels
    Sharp,
    /// Smooth edges
    #[default]
    Smooth,
    /// Old TV (CRT)
    Crt,
}

/// The shader language a RetroArch video driver reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderLanguage {
    Glsl,
    Slang,
}

/// The shader language of the video driver `driver` (`video_driver` in `retroarch.cfg`). With
/// no driver named, RetroArch's default on `platform`: `gl` on Linux, `d3d11` on Windows.
/// `None`: a driver that reads neither, which gets no shader.
pub fn shader_language(platform: Option<Platform>, driver: Option<&str>) -> Option<ShaderLanguage> {
    let driver = driver.unwrap_or(match platform {
        Some(Platform::Windows) => "d3d11",
        _ => "gl",
    });
    match driver {
        "gl" => Some(ShaderLanguage::Glsl),
        "glcore" | "vulkan" | "d3d10" | "d3d11" | "d3d12" | "metal" => Some(ShaderLanguage::Slang),
        _ => None,
    }
}

/// A picture's preset in one shader language, and every file it reads: its passes, the files
/// they include and its lookup images. All are in RetroArch's shaders folder.
struct Shader {
    picture: Picture,
    language: ShaderLanguage,
    preset: &'static str,
    files: &'static [&'static str],
}

/// The shaders a build keeps. Smooth edges is hq2x: it smooths the steps on the edges and keeps
/// the art's texture, where xBRZ rounds it into blobs. Old TV is crt-geom.
const SHADERS: [Shader; 4] = [
    Shader {
        picture: Picture::Smooth,
        language: ShaderLanguage::Glsl,
        preset: "shaders_glsl/hqx/hq2x.glslp",
        files: &[
            "shaders_glsl/hqx/shader-files/hqx-pass1.glsl",
            "shaders_glsl/hqx/shader-files/hqx-pass2.glsl",
            "shaders_glsl/hqx/resources/hq2x.png",
        ],
    },
    Shader {
        picture: Picture::Crt,
        language: ShaderLanguage::Glsl,
        preset: "shaders_glsl/crt/crt-geom.glslp",
        files: &["shaders_glsl/crt/shaders/crt-geom.glsl"],
    },
    Shader {
        picture: Picture::Smooth,
        language: ShaderLanguage::Slang,
        preset: "shaders_slang/edge-smoothing/hqx/hq2x.slangp",
        files: &[
            "shaders_slang/stock.slang",
            "shaders_slang/edge-smoothing/hqx/shaders/pass1.slang",
            "shaders_slang/edge-smoothing/hqx/shaders/hq2x.slang",
            "shaders_slang/edge-smoothing/hqx/shaders/pass2.inc",
            "shaders_slang/interpolation/shaders/bicubic/bicubic-x.slang",
            "shaders_slang/interpolation/shaders/bicubic/bicubic-y.slang",
            "shaders_slang/edge-smoothing/hqx/resources/hq2x.png",
        ],
    },
    Shader {
        picture: Picture::Crt,
        language: ShaderLanguage::Slang,
        preset: "shaders_slang/crt/crt-geom.slangp",
        files: &["shaders_slang/crt/shaders/crt-geom.slang"],
    },
];

/// The preset of `picture` in `language`, in RetroArch's shaders folder. Sharp pixels need none.
pub fn preset(picture: Picture, language: ShaderLanguage) -> Option<&'static str> {
    SHADERS.iter().find(|s| s.picture == picture && s.language == language).map(|s| s.preset)
}

/// The preset of `picture` in build `tag`, for the video driver in `retroarch.cfg`
/// (`base_config`).
pub fn shader(
    platform: Option<Platform>,
    tag: &str,
    picture: Picture,
    base_config: &str,
) -> PluginResult<Option<RelPath>> {
    let driver = config_value(base_config, "video_driver");
    let Some(preset) = shader_language(platform, driver.as_deref()).and_then(|l| preset(picture, l)) else {
        return Ok(None);
    };
    let shaders = match platform {
        Some(Platform::Windows) => versions::folder(tag)?.join(WINDOWS)?,
        _ => own_folder(platform, tag)?,
    };
    Ok(Some(shaders.join("shaders")?.join(preset)?))
}

/// The value of `key` in a RetroArch config's `key = "value"` lines.
fn config_value(config: &str, key: &str) -> Option<String> {
    config.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
    })
}

impl crate::settings::Settings for RetroSettings {
    type Game = RetroGameSettings;

    fn own_build(&self) -> bool {
        self.own_build
    }
}

/// The pinned files of the build for `platform`, in unpack order; none for a system libretro
/// has no pack of cores for (macOS).
pub fn files(platform: Option<Platform>) -> Vec<PinnedFile> {
    let file = |name: &str, path: &str, size, sha256: &str, format, into, only: Vec<String>| PinnedFile {
        name: name.into(),
        url: format!("{SERVER}/{path}"),
        size,
        sha256: sha256.into(),
        format,
        into,
        only,
    };
    match platform {
        Some(Platform::Linux) | None => {
            let home = |part: &str| format!("{LINUX_HOME}/{part}");
            let mut kept = vec![format!("{LINUX}/RetroArch-Linux-x86_64.AppImage"), home("assets"), home("autoconfig")];
            kept.extend(DATABASES.iter().map(|db| home(&format!("database/rdb/{db}.rdb"))));
            kept.extend(
                SHADERS
                    .iter()
                    .flat_map(|s| std::iter::once(s.preset).chain(s.files.iter().copied()))
                    .map(|f| home(&format!("shaders/{f}"))),
            );
            vec![
                file(
                    "RetroArch-linux.7z",
                    "linux/x86_64/RetroArch.7z",
                    179_361_448,
                    "7d62da9a21397d6e1b9490785cedbeafd262781b50115076736fbe8a77ef30e9",
                    ArchiveFormat::AppImageIn7z,
                    None,
                    kept,
                ),
                file(
                    "RetroArch_cores-linux.7z",
                    "linux/x86_64/RetroArch_cores.7z",
                    274_237_400,
                    "4b7ed8dc97d4bf035fce182c64b5658c7662e2e9e5d42129538afbd4b6096307",
                    ArchiveFormat::SevenZip,
                    Some(PACK),
                    CORES.iter().map(|c| format!("{LINUX_HOME}/cores/{c}_libretro.so")).collect(),
                ),
            ]
        }
        Some(Platform::Windows) => vec![
            file(
                "RetroArch-windows.7z",
                "windows/x86_64/RetroArch.7z",
                202_509_078,
                "b2139b1d0f9d4526dc6b5ce23cbb3efdc766096fa6f2c3df016818b486ac6372",
                ArchiveFormat::SevenZip,
                None,
                Vec::new(),
            ),
            file(
                "RetroArch_cores-windows.7z",
                "windows/x86_64/RetroArch_cores.7z",
                229_761_684,
                "86b871e11b9b4772ac644b40a38f2c8e9449da1f355eae7da08aa061148547b0",
                ArchiveFormat::SevenZip,
                Some(PACK),
                CORES.iter().map(|c| format!("{WINDOWS}/cores/{c}_libretro.dll")).collect(),
            ),
        ],
        Some(Platform::Macos) => Vec::new(),
    }
}

/// The databases of the retro consoles' games, by name, which a build keeps: they name a game
/// by its file's checksum.
pub const DATABASES: [&str; 3] =
    ["Nintendo - Nintendo Entertainment System", "Sega - Mega Drive - Genesis", "Sony - PlayStation"];

/// The program in build `tag`, on `platform`.
pub fn program(platform: Option<Platform>, tag: &str) -> PluginResult<RelPath> {
    let folder = versions::folder(tag)?;
    Ok(match platform {
        Some(Platform::Windows) => folder.join(WINDOWS)?.join("retroarch.exe")?,
        _ => folder.join(LINUX)?.join("squashfs-root")?.join("AppRun")?,
    })
}

/// The core `core` in build `tag`, on `platform`.
pub fn core(platform: Option<Platform>, tag: &str, core: &str) -> PluginResult<RelPath> {
    let pack = versions::folder(tag)?.join(PACK)?;
    Ok(match platform {
        Some(Platform::Windows) => pack.join(&format!("{WINDOWS}/cores/{core}_libretro.dll"))?,
        _ => pack.join(&format!("{LINUX_HOME}/cores/{core}_libretro.so"))?,
    })
}

/// RetroArch's own folder in build `tag`: its menu's assets and its controller profiles.
fn own_folder(platform: Option<Platform>, tag: &str) -> PluginResult<RelPath> {
    let folder = versions::folder(tag)?;
    Ok(match platform {
        Some(Platform::Windows) => folder.join(WINDOWS)?,
        _ => folder.join(LINUX_HOME)?,
    })
}

/// What `launcher.cfg` holds for a start with build `tag` (`None`: your own build, which keeps
/// its own assets), in RetroArch's `key = "value"` lines.
pub fn start_config(
    platform: Option<Platform>,
    tag: Option<&str>,
    fullscreen: bool,
    picture: Picture,
) -> PluginResult<String> {
    let mut lines: Vec<(String, String)> =
        DATA_DIRS.iter().map(|(key, dir)| ((*key).to_owned(), (*dir).to_owned())).collect();
    if let Some(tag) = tag {
        let own = own_folder(platform, tag)?;
        lines.push(("assets_directory".into(), own.join("assets")?.as_str().to_owned()));
        lines.push(("joypad_autoconfig_dir".into(), own.join("autoconfig")?.as_str().to_owned()));
        lines.push(("content_database_path".into(), own.join("database/rdb")?.as_str().to_owned()));
    }
    // Each core's saves and states in a folder of its name, where backups find them.
    lines.push(("sort_savefiles_enable".into(), "true".into()));
    lines.push(("sort_savestates_enable".into(), "true".into()));
    lines.push(("video_fullscreen".into(), fullscreen.to_string()));
    // Sharp pixels are each the same size: whole steps of scale, with a black border where the
    // screen is left. Axis 1 takes whole steps across too, the nearest to the console's shape:
    // RetroArch's default steps the height alone, which leaves columns of two widths. A shader
    // scales to the whole screen itself, and RetroArch loads the one the start names
    // (`--set-shader`) only with shaders on.
    let sharp = picture == Picture::Sharp;
    lines.push(("video_scale_integer".into(), sharp.to_string()));
    if sharp {
        lines.push(("video_scale_integer_axis".into(), "1".into()));
    } else {
        lines.push(("video_shader_enable".into(), "true".into()));
    }
    // A game starts when it is asked for; the menu is RetroArch's own.
    lines.push(("pause_nonactive".into(), "false".into()));
    // A player with a gamepad alone opens RetroArch's menu, which quits too, with Start and
    // Select together: RetroArch has no combo by default.
    lines.push(("input_menu_toggle_gamepad_combo".into(), MENU_COMBO.into()));
    Ok(lines.into_iter().map(|(key, value)| format!("{key} = \"{value}\"\n")).collect())
}

/// The pinned files of a system's build: [`files`], or a test's own.
pub type Files = fn(Option<Platform>) -> Vec<PinnedFile>;

/// RetroArch for the console `C`.
pub struct RetroArchFlavor<C> {
    pub platform: Option<Platform>,
    files: Files,
    console: PhantomData<fn() -> C>,
}

impl<C> RetroArchFlavor<C> {
    pub fn new(platform: Option<Platform>, files: Files) -> Self {
        Self { platform, files, console: PhantomData }
    }
}

#[async_trait]
impl<C: RetroConsole> Flavor for RetroArchFlavor<C> {
    type Settings = RetroSettings;
    const NAME: &'static str = "RetroArch";
    const CONSOLE: &'static str = C::CONSOLE;
    const FOLDER: &'static str = C::SYSTEM.folder;
    // Its `--help` would make the player's `~/.config/retroarch`: see the module's notes.
    const TEST_RUN: bool = false;

    fn release_source(&self, _settings: &RetroSettings) -> PluginResult<ReleaseSource> {
        Err(PluginError::Precondition(format!("RetroArch's builds come with {}", launcher_api::brand::PRODUCT_NAME)))
    }

    fn pinned(&self) -> Option<Vec<PinnedBuild>> {
        let files = (self.files)(self.platform);
        Some(if files.is_empty() { Vec::new() } else { vec![PinnedBuild { tag: VERSION.into(), files }] })
    }

    fn asset_matches(&self, _name: &str) -> bool {
        false
    }

    fn archive_format(&self, _asset: &str) -> ArchiveFormat {
        ArchiveFormat::SevenZip
    }

    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
        let program = program(self.platform, tag)?;
        match storage.exists(&program).await? {
            true => Ok(program),
            false => Err(PluginError::NotFound(format!("no RetroArch in build {tag}"))),
        }
    }

    fn runs(&self, help: &str) -> bool {
        help.contains("RetroArch")
    }

    fn own_build(&self) -> OwnBuild {
        OwnBuild {
            folder: "retroarch-build",
            itself: true,
            files: vec!["retroarch".into(), "retroarch.exe".into(), "squashfs-root/AppRun".into()],
            system: "retroarch",
        }
    }
}

/// A retro console's plugin: its games, RetroArch's builds, and how a game starts and installs.
pub struct Retro<C: RetroConsole> {
    emu: Arc<ConsoleEmulator<RetroArchFlavor<C>>>,
}

impl<C: RetroConsole> Retro<C> {
    /// What the plugin provides the host.
    pub async fn provided(ctx: &PluginContext) -> PluginResult<Provided> {
        Self::provided_with(ctx, files).await
    }

    /// [`Self::provided`], with the builds' files from `files`: a test serves its own, since the
    /// host checks each file against its SHA-256.
    pub async fn provided_with(ctx: &PluginContext, files: Files) -> PluginResult<Provided> {
        let host = Host::of(ctx);
        let spec = ConsoleSpec {
            console: Console::Rom(C::SYSTEM),
            packages: Packages::Named(|name| C::SYSTEM.is_game(name)),
            compat: None,
            // RetroArch's keyboard keys, as the manifest lists them.
            keys: &[],
            section: C::SECTION,
            version_row: false,
            own_row: false,
        };
        let flavor = RetroArchFlavor::<C>::new(host.platform, files);
        let emu = Arc::new(ConsoleEmulator::start(flavor, spec, host).await?);
        let retro = Arc::new(Self { emu: emu.clone() });
        Ok(Provided {
            catalog: Some(emu.clone()),
            downloader: Some(emu.builds().clone()),
            installer: Some(emu.builds().clone()),
            configurator: Some(emu.clone()),
            launcher: Some(retro.clone()),
            game_installer: Some(retro),
            slots: Some(emu.clone()),
            backup: Some(crate::backup::Items::provided(backup_items::<C>())),
            ..Provided::default()
        })
    }

    /// Makes the folders the configs name, and `retroarch.cfg` when there is none. Gives
    /// `launcher.cfg`, written for this start, and the shader preset of `picture`: none for
    /// sharp pixels, for your own build, or for a build installed without its shaders.
    async fn configs(
        &self,
        tag: Option<&str>,
        fullscreen: bool,
        picture: Picture,
    ) -> PluginResult<(RelPath, RelPath, Option<RelPath>)> {
        let host = self.emu.host();
        let storage = host.storage.as_ref();
        for dir in DATA_DIRS.iter().map(|(_, dir)| *dir).chain([CONFIG_HOME]) {
            let keep = RelPath::new(format!("{dir}/.keep"))?;
            if !storage.exists(&keep).await? {
                storage.write(&keep, b"").await?;
            }
        }
        let base = RelPath::new(BASE_CONFIG)?;
        if !storage.exists(&base).await? {
            storage.write(&base, b"config_save_on_exit = \"true\"\n").await?;
        }
        let start = RelPath::new(START_CONFIG)?;
        storage.write(&start, start_config(host.platform, tag, fullscreen, picture)?.as_bytes()).await?;
        let preset = match tag {
            Some(tag) if picture != Picture::Sharp => {
                let base_config = String::from_utf8_lossy(&storage.read(&base).await?).into_owned();
                match shader(host.platform, tag, picture, &base_config)? {
                    Some(preset) if storage.exists(&preset).await? => Some(preset),
                    _ => None,
                }
            }
            _ => None,
        };
        Ok((base, start, preset))
    }
}

/// The command line of a start: the configs, the shader preset if any, the core, then the game.
pub fn start_args(
    base: RelPath,
    start: RelPath,
    shader: Option<RelPath>,
    core: LaunchArg,
    game: HostPath,
) -> Vec<LaunchArg> {
    let mut args = vec![
        "--config".into(),
        LaunchArg::path(Location::Data(base)),
        "--appendconfig".into(),
        LaunchArg::path(Location::Data(start)),
    ];
    if let Some(shader) = shader {
        args.extend(["--set-shader".into(), LaunchArg::path(Location::Data(shader))]);
    }
    args.extend(["-L".into(), core, LaunchArg::path(Location::Folder(game))]);
    args
}

#[async_trait]
impl<C: RetroConsole> Launcher for Retro<C> {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.emu.games().find(self.emu.host().folders.as_ref(), game).await?;
        let builds = self.emu.builds();
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        let platform = self.emu.host().platform;
        // The official build's core; with your own build, the official one's when it is
        // installed, else your RetroArch's own, by name.
        let installed = builds.versions().state().await.current;
        let core = match installed {
            Some(tag) => LaunchArg::path(Location::Data(core(platform, &tag, C::CORE)?)),
            None => C::CORE.into(),
        };
        let (base, start, shader) = self.configs(run.tag.as_deref(), settings.fullscreen, settings.picture).await?;
        let args = start_args(base, start, shader, core, found.file.unwrap_or(found.dir));
        let env = match platform {
            Some(Platform::Windows) => Default::default(),
            _ => [("XDG_CONFIG_HOME".to_owned(), LaunchArg::path(Location::Data(RelPath::new(CONFIG_HOME)?)))].into(),
        };
        Ok(LaunchSpec {
            program: run.program,
            args,
            env,
            // The configs' folders are relative to the shared data folder.
            cwd: Some(Location::Data(RelPath::root())),
            game_process: None,
            game_env: None,
            relaunch: None,
            sandbox: None,
        })
    }
}

/// The console's files to install from `source`, each named by its own file name, and the
/// title of the install's folder. A ROM's zip beside the ROM the host unpacked from it is left
/// out: they are one game. The title is the game's, or the discs' when they are one game's
/// ("Saga (Disc 1)" and "Saga (Disc 2)" are "Saga"); else the folder the games are in; else the
/// download's name. A download with no game, or with two files of one name in different
/// folders, is refused.
async fn rom_files(
    system: &RomSystem,
    console: &str,
    folders: &dyn Folders,
    source: &HostPath,
) -> PluginResult<(Vec<DownloadFile>, String)> {
    let name = |path: &HostPath| path.path.file_name().unwrap_or_default().to_owned();
    // Each file with the folder it is in, in the download.
    let found: Vec<(DownloadFile, String)> = match installs::Download::scan(folders, source).await? {
        Some(download) => download
            .files
            .into_iter()
            .filter(|f| system.is_file(&f.rel))
            .map(|f| {
                let folder = f.rel.rsplit_once('/').map(|(dir, _)| dir.rsplit('/').next().unwrap_or(dir).to_owned());
                (DownloadFile { rel: name(&f.path), ..f }, folder.unwrap_or_default())
            })
            .collect(),
        None => siblings(system, folders, source).await?.into_iter().map(|f| (f, String::new())).collect(),
    };
    // A zip beside a ROM of its title is that ROM, packed.
    let unpacked = |zip: &str| {
        found.iter().any(|(f, _)| {
            !f.rel.to_ascii_lowercase().ends_with(".zip")
                && system.is_game(&f.rel)
                && rom_title(&f.rel) == rom_title(zip)
        })
    };
    let found: Vec<(DownloadFile, String)> = found
        .iter()
        .filter(|(f, _)| !(f.rel.to_ascii_lowercase().ends_with(".zip") && unpacked(&f.rel)))
        .cloned()
        .collect();
    let mut names = std::collections::HashSet::new();
    if let Some((twice, _)) = found.iter().find(|(f, _)| !names.insert(f.rel.to_ascii_lowercase())) {
        return Err(PluginError::InvalidInput(format!(
            "Two files are named {} in {}; choose a single release folder",
            twice.rel, source.path
        )));
    }
    let games: Vec<&(DownloadFile, String)> = found.iter().filter(|(f, _)| system.is_game(&f.rel)).collect();
    if games.is_empty() {
        return Err(PluginError::InvalidInput(format!("{} holds no {console} game to install", source.path)));
    }
    let titles: std::collections::BTreeSet<String> = games.iter().map(|(f, _)| rom_title(&f.rel)).collect();
    let in_folders: std::collections::BTreeSet<&str> = games.iter().map(|(_, folder)| folder.as_str()).collect();
    let title = match (titles.iter().next(), in_folders.iter().next()) {
        (Some(one), _) if titles.len() == 1 => one.clone(),
        (_, Some(folder))
            if in_folders.len() == 1 && !folder.is_empty() && *folder != crate::archive_sets::UNPACKED =>
        {
            rom_title(folder)
        }
        _ => rom_title(&name(source)),
    };
    Ok((found.into_iter().map(|(f, _)| f).collect(), title))
}

/// The file `source`, and the files beside it that are parts of the same game: the console's
/// files whose names start with its name ("Racer (Europe).bin" for "Racer (Europe).cue").
async fn siblings(system: &RomSystem, folders: &dyn Folders, source: &HostPath) -> PluginResult<Vec<DownloadFile>> {
    let own = source.path.file_name().unwrap_or_default().to_owned();
    let stem = own.rsplit_once('.').map_or(own.as_str(), |(s, _)| s).to_owned();
    let size = installs::file_size(folders, source).await?;
    let mut files = vec![DownloadFile { path: source.clone(), rel: own.clone(), size }];
    let parent = match source.path.as_str().rsplit_once('/') {
        Some((dir, _)) => HostPath::new(&source.folder, source.root, RelPath::new(dir)?),
        None => HostPath::new(&source.folder, source.root, RelPath::root()),
    };
    for entry in folders.list(&parent).await.unwrap_or_default() {
        let part = !entry.is_dir && entry.name != own && entry.name.starts_with(&stem) && system.is_file(&entry.name);
        if part && let Ok(path) = parent.join(&entry.name) {
            files.push(DownloadFile { path, rel: entry.name, size: entry.size });
        }
    }
    Ok(files)
}

#[async_trait]
impl<C: RetroConsole> GameInstaller for Retro<C> {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        self.emu.candidates().await
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.emu.host().folders.clone();
        let (files, title) = rom_files(C::SYSTEM, C::CONSOLE, folders.as_ref(), source).await?;
        let base = installs::games_folder(folders.as_ref(), job.as_ref(), C::SYSTEM.folder).await?;
        let dest = base.join(&title)?;
        let tree = HostTree::of(folders.clone(), files);
        game_install::install(folders, job, vec![Part::new(dest, title, Box::new(tree))]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_system_s_build_is_retroarch_then_only_the_retro_cores_of_its_pack() {
        let linux = files(Some(Platform::Linux));
        assert_eq!(linux.len(), 2);
        assert_eq!(linux[0].format, ArchiveFormat::AppImageIn7z);
        assert_eq!(linux[0].into, None, "RetroArch unpacks into the build's folder");
        assert!(linux[0].only.iter().any(|o| o.ends_with("/autoconfig")), "the controller profiles");
        assert!(linux[0].only.iter().any(|o| o.ends_with("Sony - PlayStation.rdb")));
        let shaders: Vec<&String> = linux[0].only.iter().filter(|o| o.contains("/shaders/")).collect();
        assert_eq!(shaders.len(), 16, "the two pictures' presets and their files in two languages: {shaders:?}");
        assert!(shaders.iter().all(|o| o.starts_with(&format!("{LINUX_HOME}/shaders/shaders_"))), "{shaders:?}");
        assert_eq!(linux[1].into, Some(PACK));
        assert_eq!(linux[1].only.len(), CORES.len());
        let windows = files(Some(Platform::Windows));
        assert!(windows[0].only.is_empty(), "the whole program");
        assert!(windows[1].only.iter().all(|o| o.ends_with("_libretro.dll")));
        assert!(files(Some(Platform::Macos)).is_empty(), "no pack of cores for macOS");
        for file in linux.iter().chain(&windows) {
            assert!(file.url.starts_with("https://buildbot.libretro.com/stable/1.22.2/"), "{}", file.url);
            assert_eq!(file.sha256.len(), 64);
        }
    }

    #[test]
    fn the_program_and_a_core_are_in_the_build_s_folder() {
        assert_eq!(
            program(Some(Platform::Linux), "1.22.2").unwrap().as_str(),
            "builds/1.22.2/RetroArch-Linux-x86_64/squashfs-root/AppRun"
        );
        assert_eq!(
            program(Some(Platform::Windows), "1.22.2").unwrap().as_str(),
            "builds/1.22.2/RetroArch-Win64/retroarch.exe"
        );
        assert_eq!(
            core(Some(Platform::Windows), "1.22.2", "fceumm").unwrap().as_str(),
            "builds/1.22.2/pack/RetroArch-Win64/cores/fceumm_libretro.dll"
        );
        assert!(
            core(Some(Platform::Linux), "1.22.2", "pcsx_rearmed")
                .unwrap()
                .as_str()
                .ends_with("cores/pcsx_rearmed_libretro.so")
        );
    }

    #[test]
    fn a_start_s_config_names_the_folders_relative_to_the_shared_data_folder() {
        let config = start_config(Some(Platform::Linux), Some("1.22.2"), true, Picture::Sharp).unwrap();
        assert!(config.contains("savefile_directory = \"saves\"\n"), "{config}");
        assert!(config.contains("rgui_config_directory = \"config\"\n"), "core options where backups find them");
        assert!(config.contains("input_remapping_directory = \"config/remaps\"\n"), "{config}");
        assert!(config.contains("sort_savefiles_enable = \"true\"\n"), "saves by core, for backups");
        assert!(config.contains("sort_savestates_enable = \"true\"\n"), "{config}");
        assert!(config.contains("joypad_autoconfig_dir = \"builds/1.22.2/RetroArch-Linux-x86_64/"), "{config}");
        assert!(config.contains("video_fullscreen = \"true\"\n"));
        assert!(config.contains("input_menu_toggle_gamepad_combo = \"4\"\n"), "the menu from a gamepad");
        let own = start_config(Some(Platform::Linux), None, false, Picture::Crt).unwrap();
        assert!(!own.contains("assets_directory"), "your own build keeps its own: {own}");
        assert!(own.contains("video_fullscreen = \"false\""));
        assert!(config.contains("video_scale_integer = \"true\"\n"), "sharp pixels: {config}");
        assert!(config.contains("video_scale_integer_axis = \"1\"\n"), "whole steps across too: {config}");
        assert!(!config.contains("video_shader_enable"), "the player's own shaders stay as they are: {config}");
        assert!(own.contains("video_scale_integer = \"false\"\n"), "a shader fills the screen: {own}");
        assert!(own.contains("video_shader_enable = \"true\"\n"), "else RetroArch skips the preset: {own}");
    }

    #[test]
    fn a_picture_s_preset_is_in_the_shader_language_of_the_player_s_video_driver() {
        let linux = Some(Platform::Linux);
        let windows = Some(Platform::Windows);
        assert_eq!(shader_language(linux, None), Some(ShaderLanguage::Glsl), "RetroArch's default on Linux is gl");
        assert_eq!(shader_language(windows, None), Some(ShaderLanguage::Slang), "and d3d11 on Windows");
        assert_eq!(shader_language(linux, Some("vulkan")), Some(ShaderLanguage::Slang));
        assert_eq!(shader_language(linux, Some("sdl2")), None, "a driver with no shaders");
        assert_eq!(preset(Picture::Sharp, ShaderLanguage::Glsl), None, "sharp pixels need no shader");
        assert_eq!(RetroSettings::default().picture, Picture::Smooth, "smooth edges unless the player picks");

        let gl = shader(linux, "1.22.2", Picture::Crt, "video_driver = \"gl\"\n").unwrap().unwrap();
        assert_eq!(gl.as_str(), format!("builds/1.22.2/{LINUX_HOME}/shaders/shaders_glsl/crt/crt-geom.glslp"));
        let vulkan = shader(linux, "1.22.2", Picture::Smooth, "video_driver = \"vulkan\"\n").unwrap().unwrap();
        assert!(vulkan.as_str().ends_with("shaders/shaders_slang/edge-smoothing/hqx/hq2x.slangp"), "{vulkan:?}");
        let win = shader(windows, "1.22.2", Picture::Crt, "").unwrap().unwrap();
        assert_eq!(win.as_str(), "builds/1.22.2/RetroArch-Win64/shaders/shaders_slang/crt/crt-geom.slangp");
        assert_eq!(shader(linux, "1.22.2", Picture::Sharp, "").unwrap(), None);
        assert_eq!(shader(linux, "1.22.2", Picture::Crt, "video_driver = \"sdl2\"\n").unwrap(), None);
    }

    #[test]
    fn each_shader_s_files_are_in_its_own_language_s_folder() {
        for shader in SHADERS {
            let folder = match shader.language {
                ShaderLanguage::Glsl => "shaders_glsl/",
                ShaderLanguage::Slang => "shaders_slang/",
            };
            for file in std::iter::once(&shader.preset).chain(shader.files) {
                assert!(file.starts_with(folder), "{file} with {:?}", shader.language);
            }
        }
    }

    #[test]
    fn a_start_gives_the_configs_then_the_core_then_the_game() {
        let game = HostPath::new("games", 0, RelPath::new("NES/Game.nes").unwrap());
        let args = start_args(
            RelPath::new("a.cfg").unwrap(),
            RelPath::new("b.cfg").unwrap(),
            None,
            "fceumm".into(),
            game.clone(),
        );
        let words: Vec<String> = args
            .iter()
            .map(|a| match a {
                LaunchArg::Text(t) => t.clone(),
                LaunchArg::Path { path } => format!("{path:?}"),
            })
            .collect();
        assert_eq!(words[0], "--config");
        assert_eq!(words[2], "--appendconfig");
        assert_eq!((words[4].as_str(), words[5].as_str()), ("-L", "fceumm"));
        assert_eq!(args.last(), Some(&LaunchArg::path(Location::Folder(game.clone()))));
        let preset = RelPath::new("builds/1.22.2/crt.glslp").unwrap();
        let shaded = start_args(
            RelPath::new("a.cfg").unwrap(),
            RelPath::new("b.cfg").unwrap(),
            Some(preset.clone()),
            "fceumm".into(),
            game,
        );
        assert_eq!(shaded[4], LaunchArg::Text("--set-shader".into()));
        assert_eq!(shaded[5], LaunchArg::path(Location::Data(preset)));
        assert_eq!(shaded[6], LaunchArg::Text("-L".into()));
    }
}

//! The retro consoles' plugins on a fake Linux host: one RetroArch in the data folder they
//! share, the ROM files of each console's own folder, and the launch. The builds' files are the
//! test's own, served with their checksums, in place of the 180 MB the real ones are.

use std::sync::OnceLock;

use emulator_kit::builds::PinnedFile;
use emulator_kit::retroarch::{self, Retro, RetroConsole};
use launcher_api::*;
use launcher_core::{Command, SessionCmd};
use launcher_testkit::{FakeHost, FakeSpawner, TestCore, contract};

const RETROARCH: &str = "https://buildbot.libretro.com/test/ra.7z";
const CORES: &str = "https://buildbot.libretro.com/test/cores.7z";
const LINUX_HOME: &str = "RetroArch-Linux-x86_64/RetroArch-Linux-x86_64.AppImage.home/.config/retroarch";

/// The checksums of the two files the fake server serves, once it has.
static SHAS: OnceLock<(String, String)> = OnceLock::new();

/// The test's build: RetroArch's AppImage, then the cores into `pack`, as the real one is.
fn test_files(_platform: Option<Platform>) -> Vec<PinnedFile> {
    let (ra, cores) = SHAS.get().expect("the files are served first").clone();
    let file = |name: &str, url: &str, sha256, format, into| PinnedFile {
        name: name.into(),
        url: url.into(),
        size: 2,
        sha256,
        format,
        into,
        only: Vec::new(),
    };
    vec![
        file("ra.7z", RETROARCH, ra, ArchiveFormat::AppImageIn7z, None),
        file("cores.7z", CORES, cores, ArchiveFormat::SevenZip, Some("pack")),
    ]
}

/// A retro plugin of console `C`, as its crate's, on the test's build files.
struct TestRetro<C>(PluginManifest, std::marker::PhantomData<fn() -> C>);

#[async_trait]
impl<C: RetroConsole> Plugin for TestRetro<C> {
    fn manifest(&self) -> PluginManifest {
        self.0.clone()
    }
}

#[async_trait]
impl<C: RetroConsole> CodePlugin for TestRetro<C> {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        Retro::<C>::provided_with(&ctx, test_files).await
    }
}

fn plugin<C: RetroConsole>(manifest: PluginManifest) -> PluginBox {
    PluginBox::code(TestRetro::<C>(manifest, std::marker::PhantomData))
}

fn nes() -> PluginId {
    PluginId::new("emu-nes").unwrap()
}

fn ps1() -> PluginId {
    PluginId::new("emu-ps1").unwrap()
}

/// Does what the AppImage does when the host runs `--appimage-extract`: unpacks itself.
fn extract_appimages(spawner: FakeSpawner) {
    tokio::spawn(async move {
        let mut n = 0;
        loop {
            n += 1;
            let process = spawner.wait_for(n).await;
            if process.command.args == ["--appimage-extract"] {
                let root = process.command.cwd.join("squashfs-root");
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(root.join("AppRun"), b"\x7fELF retroarch").unwrap();
                process.exit(0);
            }
        }
    });
}

/// The shared data folder.
fn shared(test: &TestCore) -> std::path::PathBuf {
    test.data_dir().join("shared/retroarch")
}

fn app_run(test: &TestCore) -> std::path::PathBuf {
    shared(test).join("builds").join(retroarch::VERSION).join("RetroArch-Linux-x86_64/squashfs-root/AppRun")
}

/// The NES and PS1 plugins, the test's build files, and a games folder with NES and PS1 games,
/// and files of theirs elsewhere.
async fn boot() -> (TestCore, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let ra = host.http().serve(RETROARCH, b"7z".to_vec());
    let cores = host.http().serve(CORES, b"7c".to_vec());
    let _ = SHAS.set((ra, cores));
    host.archives().holds(
        "ra.7z",
        &[
            ("RetroArch-Linux-x86_64/RetroArch-Linux-x86_64.AppImage", b"\x7fELF" as &[u8]),
            (&format!("{LINUX_HOME}/autoconfig/udev/pad.cfg"), b"pad"),
            (&format!("{LINUX_HOME}/shaders/shaders_glsl/crt/crt-geom.glslp"), b"shader0 = shaders/crt-geom.glsl"),
        ],
    );
    host.archives().holds(
        "cores.7z",
        &[
            (&format!("{LINUX_HOME}/cores/fceumm_libretro.so"), b"\x7fELF fceumm" as &[u8]),
            (&format!("{LINUX_HOME}/cores/pcsx_rearmed_libretro.so"), b"\x7fELF pcsx"),
        ],
    );
    let games = tempfile::tempdir().unwrap();
    let file = |rel: &str| {
        let path = games.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"rom").unwrap();
    };
    file("NES/Super Game (World).nes");
    file("NES/Collection/Other Game (USA) [!].zip");
    file("NES/readme.txt");
    // A .zip outside NES/ is another console's.
    file("Arcade/Not NES.zip");
    file("PS1/Big Game/Big Game (Disc 1).cue");
    file("PS1/Big Game/Big Game (Disc 1).bin");
    let test = host
        .built_in(plugin::<emu_nes::Nes>(emu_nes::NesPlugin.manifest()))
        .built_in(plugin::<emu_ps1::Ps1>(emu_ps1::Ps1Plugin.manifest()))
        .boot_core()
        .await;
    extract_appimages(test.spawner.clone());
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    (test, games)
}

/// Boots, and installs RetroArch through the NES plugin.
async fn installed() -> (TestCore, tempfile::TempDir) {
    let (test, games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: nes(), version: None }).await.unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    (test, games)
}

async fn installed_versions(test: &TestCore, plugin: &PluginId) -> Vec<String> {
    let provided = test.kernel.provided(plugin).await.unwrap();
    let installer = provided.installer.clone().unwrap();
    installer.installed().await.unwrap().into_iter().map(|v| v.version).collect()
}

#[tokio::test]
async fn one_install_of_retroarch_serves_every_retro_plugin() {
    let (test, _games) = installed().await;

    assert!(app_run(&test).is_file(), "RetroArch is in the shared data folder");
    let core = shared(&test).join("builds").join(retroarch::VERSION).join("pack").join(LINUX_HOME);
    assert!(core.join("cores/fceumm_libretro.so").is_file() && core.join("cores/pcsx_rearmed_libretro.so").is_file());
    assert!(!test.data_dir().join("plugins/emu-nes").exists(), "no folder of its own");
    assert_eq!(installed_versions(&test, &nes()).await, [retroarch::VERSION]);
    assert_eq!(installed_versions(&test, &ps1()).await, [retroarch::VERSION], "the PS1 plugin has it too");
    assert_eq!(test.spawner.spawned().len(), 1, "only the AppImage's unpacking ran: no --help");
    let fetched = test.http.requests().iter().filter(|(url, _)| url == RETROARCH).count();
    assert_eq!(fetched, 1, "RetroArch is downloaded once");
}

#[tokio::test]
async fn each_console_s_games_are_the_rom_files_in_its_own_folder() {
    let (test, _games) = installed().await;
    let mut keys: Vec<String> = test.core.view().home.games.iter().map(|g| g.key.clone()).collect();
    keys.sort();
    assert_eq!(keys, ["emu-nes:Other Game", "emu-nes:Super Game", "emu-ps1:Big Game"]);
    let platforms: Vec<String> = test.core.view().home.games.iter().map(|g| g.platform.clone()).collect();
    assert!(platforms.iter().all(|p| p == "NES" || p == "PS1"), "{platforms:?}");
    let home = test.core.view().home;
    let card = home.games.iter().find(|g| g.key == "emu-nes:Super Game").unwrap();
    let cover = format!("{:?}", card.cover);
    let url = "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Game%20%28World%29.png";
    assert!(cover.contains(url), "the box art libretro names after the file: {cover}");
}

#[tokio::test]
async fn a_game_starts_in_retroarch_with_its_console_s_core_and_the_launcher_s_configs() {
    let (test, games) = installed().await;
    let spawned = test.spawner.spawned().len();

    test.core.dispatch(Command::Session(SessionCmd::Launch("emu-nes:Super Game".parse().unwrap()))).await.unwrap();

    let process = test.spawner.wait_for(spawned + 1).await;
    assert_eq!(process.command.program, app_run(&test));
    assert_eq!(process.command.cwd, shared(&test), "the configs' folders are relative to it");
    let core = shared(&test)
        .join("builds")
        .join(retroarch::VERSION)
        .join("pack")
        .join(LINUX_HOME)
        .join("cores/fceumm_libretro.so");
    let game = games.path().join("NES/Super Game (World).nes");
    let expected: Vec<String> = [
        "--config".to_owned(),
        shared(&test).join("retroarch.cfg").display().to_string(),
        "--appendconfig".to_owned(),
        shared(&test).join("launcher.cfg").display().to_string(),
        "-L".to_owned(),
        core.display().to_string(),
        game.display().to_string(),
    ]
    .into();
    assert_eq!(process.command.args, expected);
    let start = std::fs::read_to_string(shared(&test).join("launcher.cfg")).unwrap();
    assert!(start.contains("savefile_directory = \"saves\""), "{start}");
    assert!(start.contains("video_fullscreen = \"true\""), "{start}");
    assert!(shared(&test).join("saves").is_dir(), "the folders the config names are there");
    let home = shared(&test).join("config-home").display().to_string();
    assert_eq!(process.command.env.get("XDG_CONFIG_HOME"), Some(&home), "never the player's own RetroArch folder");
}

/// Sets the NES's picture, starts its game, and gives the start's arguments.
async fn start_with_picture(test: &TestCore, picture: &str) -> Vec<String> {
    std::fs::write(shared(test).join("retroarch.cfg"), "video_driver = \"gl\"\n").unwrap();
    let set = Command::SetConfigField {
        plugin: nes(),
        scope: ConfigScope::Emulator,
        key: "picture".into(),
        value: picture.into(),
    };
    test.core.dispatch(set).await.unwrap();
    let spawned = test.spawner.spawned().len();
    test.core.dispatch(Command::Session(SessionCmd::Launch("emu-nes:Super Game".parse().unwrap()))).await.unwrap();
    test.spawner.wait_for(spawned + 1).await.command.args
}

/// The old-TV picture starts with the build's CRT preset in the shader language of the video
/// driver, GLSL for `gl`, with shaders on.
#[tokio::test]
async fn the_old_tv_picture_starts_with_the_build_s_crt_preset_for_the_video_driver() {
    let (test, _games) = installed().await;
    let args = start_with_picture(&test, "Crt").await;
    let preset = shared(&test)
        .join("builds")
        .join(retroarch::VERSION)
        .join(LINUX_HOME)
        .join("shaders/shaders_glsl/crt/crt-geom.glslp");
    let at = args.iter().position(|a| a == "--set-shader").unwrap_or_else(|| panic!("a shader: {args:?}"));
    assert_eq!(args[at + 1], preset.display().to_string());
    assert_eq!(args[at + 2], "-L", "before the core: {args:?}");
    let start = std::fs::read_to_string(shared(&test).join("launcher.cfg")).unwrap();
    assert!(start.contains("video_shader_enable = \"true\""), "{start}");
    assert!(start.contains("video_scale_integer = \"false\""), "{start}");
}

/// The game's page offers the three pictures after "As the emulator", and the one set for the
/// game wins over the emulator's at its start.
#[tokio::test]
async fn a_game_s_own_picture_shows_on_its_page_and_wins_at_its_start() {
    let (test, _games) = installed().await;
    std::fs::write(shared(&test).join("retroarch.cfg"), "video_driver = \"gl\"\n").unwrap();
    let game: GameKey = "emu-nes:Super Game".parse().unwrap();
    test.core.dispatch(Command::Navigate(launcher_core::Route::GameSettings(game.clone()))).await.unwrap();
    let detail = test.core.view().detail.expect("the game's details");
    let picture = detail.settings.iter().flat_map(|p| &p.fields).find(|f| f.key == "picture").expect("picture");
    assert_eq!(picture.choice_labels, ["As the emulator", "Sharp pixels", "Smooth edges", "Old TV (CRT)"]);
    let crt = picture.choices[3].clone();

    let scope = ConfigScope::Game(game.local.clone());
    test.core
        .dispatch(Command::SetConfigField { plugin: nes(), scope, key: "picture".into(), value: crt })
        .await
        .unwrap();
    let spawned = test.spawner.spawned().len();
    test.core.dispatch(Command::Session(SessionCmd::Launch(game))).await.unwrap();
    let args = test.spawner.wait_for(spawned + 1).await.command.args;
    assert!(args.iter().any(|a| a == "--set-shader"), "the game's old TV, not the emulator's smooth edges: {args:?}");
}

/// A build installed without a picture's preset starts the game with no shader, never with a
/// path that isn't there.
#[tokio::test]
async fn a_picture_whose_preset_the_build_lacks_starts_with_no_shader() {
    let (test, _games) = installed().await;
    let args = start_with_picture(&test, "Smooth").await;
    assert!(!args.iter().any(|a| a == "--set-shader"), "no preset in this build: {args:?}");
}

#[tokio::test]
async fn the_nes_plugin_meets_every_contract() {
    let (test, _games) = installed().await;
    contract::emulator(&test, &nes()).await;
}

/// A PS1 disc image in the packages folder is offered by its .cue, and installs with its .bin
/// into its own folder in PS1/.
#[tokio::test]
async fn a_disc_image_in_the_packages_folder_installs_with_its_tracks_into_its_own_folder() {
    let (test, games) = installed().await;
    let packages = tempfile::tempdir().unwrap();
    std::fs::create_dir(packages.path().join("PS1")).unwrap();
    std::fs::write(packages.path().join("PS1/Racer (Europe).cue"), b"FILE \"Racer (Europe).bin\" BINARY").unwrap();
    std::fs::write(packages.path().join("PS1/Racer (Europe).bin"), b"track").unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![packages.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::Navigate(launcher_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == ps1()).unwrap();
    let offers: Vec<_> = emu.panels.iter().filter(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).collect();
    assert_eq!(offers.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Install Racer (Europe).cue"]);

    let start = Command::RunAction {
        plugin: ps1(),
        action: HOST_INSTALL_GAME.into(),
        game: None,
        args: offers[0].args.clone(),
    };
    test.core.dispatch(start).await.unwrap();
    test.until(launcher_core::Area::Installs, |core| {
        core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing")
    })
    .await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    assert_eq!(job.state, "done", "{}", job.error);
    let folder = games.path().join("PS1/Racer");
    assert_eq!(std::fs::read(folder.join("Racer (Europe).cue")).unwrap(), b"FILE \"Racer (Europe).bin\" BINARY");
    assert_eq!(std::fs::read(folder.join("Racer (Europe).bin")).unwrap(), b"track");
}

/// The other retro plugins' pages show an install at once, with no read of the library between.
#[tokio::test]
async fn the_other_retro_pages_show_an_install_at_once() {
    let (test, _games) = boot().await;
    let page = |id: &PluginId| {
        let view = test.core.view();
        view.emulators.emulators.iter().find(|e| &e.plugin == id).map(|e| e.installed.clone()).unwrap_or_default()
    };
    assert!(page(&ps1()).is_empty());

    test.core.dispatch(Command::InstallEmulator { plugin: nes(), version: None }).await.unwrap();

    assert_eq!(page(&ps1()), [retroarch::VERSION], "the PS1 page shows it");
}

/// The Library's system chips go by the label they show, A to Z: GENESIS, NES, PS1. Home keeps
/// its own order, by the systems' full names. Neither follows the order the plugins load in.
#[tokio::test]
async fn the_library_s_system_chips_go_by_their_label_and_home_by_full_name() {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let games = tempfile::tempdir().unwrap();
    for rel in ["NES/Alpha (World).nes", "PS1/Beta (USA).cue", "GENESIS/Gamma (USA).md"] {
        let path = games.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"rom").unwrap();
    }
    let test = host
        .built_in(PluginBox::code(emu_genesis::GenesisPlugin))
        .built_in(PluginBox::code(emu_ps1::Ps1Plugin))
        .built_in(PluginBox::code(emu_nes::NesPlugin))
        .boot_core()
        .await;
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let view = test.core.view();
    let home: Vec<&str> = view.emulators.emulators.iter().map(|e| e.system.as_str()).collect();
    assert_eq!(home, ["NES", "PS1", "GENESIS"], "Home: by the systems' full names");
    let chips: Vec<&str> = view.home.library.systems.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(chips, ["", "GENESIS", "NES", "PS1"], "All, then the labels A to Z");
}

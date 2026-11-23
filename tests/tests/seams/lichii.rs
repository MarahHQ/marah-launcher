//! The Lichii plugin on a fake Linux host: the bare AppImage of a pre-release, Switch game
//! files (but not their updates) from the player's folder, and the launch.

use emu_lichii::{LichiiPlugin, RELEASES};
use launcher_api::*;
use launcher_api::{Controller, HOST_OPEN_EMULATOR};
use launcher_core::{Command, SessionCmd};
use launcher_testkit::{FakeHost, FakeHttp, FakeSpawner, TestCore, contract};

const TAG: &str = "build-93";

fn lichii() -> PluginId {
    PluginId::new("emu-lichii").unwrap()
}

/// Does what a real AppImage does when the host runs `--appimage-extract`: unpacks itself.
fn extract_appimages(spawner: FakeSpawner) {
    tokio::spawn(async move {
        let mut n = 0;
        loop {
            n += 1;
            let process = spawner.wait_for(n).await;
            if process.command.args == ["--appimage-extract"] {
                let root = process.command.cwd.join("squashfs-root");
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(root.join("AppRun"), b"\x7fELF lichii").unwrap();
                process.exit(0);
            }
        }
    });
}

/// A pre-release of `tag` with the assets Lichii publishes, whose AppImage download redirects
/// to GitHub's CDN.
fn release(http: &FakeHttp, tag: &str) -> serde_json::Value {
    let arch = std::env::consts::ARCH;
    let name = format!("lichii-linux-{arch}.AppImage");
    let download = format!("https://github.com/zortos293/Lichii/releases/download/{tag}/{name}");
    let cdn = format!("https://release-assets.githubusercontent.com/lichii/{tag}/{name}");
    let sha = http.serve(&cdn, b"\x7fELF AppImage".to_vec());
    http.redirect(&download, &cdn);
    let other = |n: &str| serde_json::json!({ "name": n, "browser_download_url": "https://github.com/x", "size": 1 });
    serde_json::json!({ "tag_name": tag, "prerelease": true, "published_at": "2026-11-06T21:04:53Z", "assets": [
        other(&format!("lichii-linux-{arch}-pgo.AppImage")),
        other(&format!("lichii-linux-{arch}.tar.gz")),
        other("lichii-windows-x86_64.zip"),
        { "name": name, "browser_download_url": download, "size": 0, "digest": format!("sha256:{sha}") },
    ]})
}

async fn boot() -> (TestCore, tempfile::TempDir) {
    boot_with_saved(None).await
}

/// [`boot`], with what an earlier run of Marah Launcher saved as Lichii's settings.
async fn boot_with_saved(saved: Option<serde_json::Value>) -> (TestCore, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    if let Some(saved) = saved {
        let config = serde_json::json!({ "plugins": { "emu-lichii": { "emulator": saved } } });
        std::fs::write(host.data_dir().join("config.json"), config.to_string()).unwrap();
    }
    let releases = serde_json::json!([release(host.http(), TAG)]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    let switch = games.path().join("Switch");
    let file = |rel: &str, bytes: &[u8]| {
        let path = switch.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    // An install's folder: the game, its update and its DLC.
    file("Celeste/Celeste [01002B30028F6000][v0].nsp", b"PFS0 game");
    file("Celeste/Celeste [01002B30028F6800][v262144].nsp", b"PFS0 update");
    file("Celeste/Celeste Farewell [01002B30028F7001][v0].nsp", b"PFS0 dlc");
    // A file name without a title id says it is an update.
    file("Kirby/Kirby Air Riders.nup", b"NUP game");
    file("Kirby/Kirby Air Riders Update 1.1.nsp", b"PFS0 update");
    // A game file of its own, beside the others.
    file("Loose.xci", b"HEAD cartridge");
    // An install still being written.
    file("Pending.part/Pending.nsp", b"PFS0 half");
    // As deep as the scan reads.
    file("Collection/Zelda/Zelda.nsp", b"PFS0 game");
    file("notes.txt", b"not a game");
    let test = host.built_in(launcher_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    extract_appimages(test.spawner.clone());
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    (test, games)
}

/// Lichii keeps its data and settings in its own folders from build-150 on, and eden-cli
/// refuses every game while Eden's old folder is there without an import. So Marah Launcher writes
/// firmware, reads settings and backs up there.
#[tokio::test]
async fn lichii_s_folders_default_to_lichii_s_own() {
    let home = tempfile::tempdir().unwrap();
    let host = FakeHost::new().home(home.path()).platform(Some(Platform::Linux));
    let test = host.built_in(launcher_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    test.core.dispatch(Command::Navigate(launcher_core::Route::Plugin(lichii()))).await.unwrap();
    let folders = test.core.view().plugin_page.expect("Lichii's page").folders;
    let default = |name: &str| folders.iter().find(|f| f.name == name).map(|f| f.default.clone());
    assert_eq!(default("lichii-data"), Some(home.path().join(".local/share/lichii").display().to_string()));
    assert_eq!(default("lichii-config"), Some(home.path().join(".config/lichii").display().to_string()));
}

#[tokio::test]
async fn lichii_meets_every_contract() {
    let (test, _games) = boot().await;
    contract::emulator(&test, &lichii()).await;
}

#[tokio::test]
async fn switch_game_files_are_listed_under_the_catalog_s_console_and_updates_are_not() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let home = test.core.view().home;
    let mut keys: Vec<&str> = home.games.iter().map(|g| g.key.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        ["emu-lichii:01002B30028F6000", "emu-lichii:Kirby Air Riders", "emu-lichii:Loose", "emu-lichii:Zelda"]
    );
    assert!(home.games.iter().all(|g| g.platform == "SWITCH"), "the catalog's console, so its offers match");
}

#[tokio::test]
async fn the_appimage_installs_without_a_test_run_and_starts_the_game_file_in_full_screen() {
    let (test, games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    let app_run = test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("squashfs-root/AppRun");
    assert!(app_run.is_file());
    assert_eq!(test.spawner.spawned().len(), 1, "only the extract ran: Lichii's --help never ends");

    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();

    let process = test.spawner.wait_for(2).await;
    assert_eq!(process.command.program, app_run);
    let game = games.path().join("Switch/Celeste/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(process.command.args, ["-f".to_owned(), "-g".to_owned(), game]);
    // Lichii's split GPU thread leaves Switch 2 games black on some PCs: off unless turned on.
    assert_eq!(process.command.env.get("EDEN_NX2_SPLIT").map(String::as_str), Some("0"));
}

#[tokio::test]
async fn a_game_whose_settings_choose_another_gpu_split_starts_with_it() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let game = "emu-lichii:01002B30028F6000";
    test.core
        .dispatch(Command::SetConfig {
            plugin: lichii(),
            // A game's settings are under its id in the plugin.
            scope: ConfigScope::Game("01002B30028F6000".into()),
            value: serde_json::json!({ "gpu_split": "2" }),
        })
        .await
        .unwrap();

    test.core.dispatch(Command::Session(SessionCmd::Launch(game.parse().unwrap()))).await.unwrap();

    let process = test.spawner.wait_for(2).await;
    assert_eq!(process.command.env.get("EDEN_NX2_SPLIT").map(String::as_str), Some("2"));
}

#[tokio::test]
async fn uninstalling_a_game_deletes_its_folder_with_its_update_and_dlc_and_a_loose_file_alone() {
    let (test, games) = boot().await;
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    for key in ["emu-lichii:01002B30028F6000", "emu-lichii:Loose"] {
        let game: GameKey = key.parse().unwrap();
        test.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();
        test.core.dispatch(Command::UninstallGame(game)).await.unwrap();
    }
    let switch = games.path().join("Switch");
    assert!(!switch.join("Celeste").exists(), "the game's folder, with its update and DLC");
    assert!(!switch.join("Loose.xci").exists(), "the loose game file");
    assert!(switch.join("Kirby/Kirby Air Riders.nup").is_file(), "other games stay");
    assert!(switch.join("Collection/Zelda/Zelda.nsp").is_file(), "other games stay");
}

#[tokio::test]
async fn a_game_file_in_the_packages_folder_is_offered_and_installs_into_its_own_folder() {
    let (test, games) = boot().await;
    let packages = tempfile::tempdir().unwrap();
    std::fs::create_dir(packages.path().join("Switch")).unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974000][v0].nsz"), b"NCZ game").unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974800][v65536].nsp"), b"PFS0 update").unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![packages.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::Navigate(launcher_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii()).unwrap();
    let offers: Vec<&str> =
        emu.panels.iter().filter(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).map(|p| p.text.as_str()).collect();
    assert_eq!(offers, ["Install Hades [0100535012974000][v0].nsz"], "the game, not its update");
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).unwrap();

    let start =
        Command::RunAction { plugin: lichii(), action: HOST_INSTALL_GAME.into(), game: None, args: offer.args.clone() };
    test.core.dispatch(start).await.unwrap();
    test.until(launcher_core::Area::Installs, |core| {
        core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing")
    })
    .await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    assert_eq!(job.state, "done", "{}", job.error);
    let installed = games.path().join("Switch/Hades/Hades [0100535012974000][v0].nsz");
    assert_eq!(std::fs::read(installed).unwrap(), b"NCZ game");
}

#[tokio::test]
async fn a_switch_game_file_keeps_the_game_install_contract() {
    let (test, _games) = boot().await;
    let packages = tempfile::tempdir().unwrap();
    std::fs::create_dir(packages.path().join("Switch")).unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974000][v0].nsp"), b"PFS0 game").unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![packages.path().into()] })
        .await
        .unwrap();
    let source = HostPath::new("packages", 0, RelPath::new("Switch/Hades [0100535012974000][v0].nsp").unwrap());
    contract::game_install(&test, &lichii(), &source).await;
}

/// The Emulators screen's button that opens Lichii's own window, if its page has one.
fn open_button(test: &TestCore) -> Option<launcher_ui_api::SlotVm> {
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii())?.clone();
    emu.panels.into_iter().find(|p| p.action.as_deref() == Some(HOST_OPEN_EMULATOR))
}

#[tokio::test]
async fn open_lichii_starts_its_own_window_without_a_game_and_no_game_starts_meanwhile() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    test.core.dispatch(Command::Navigate(launcher_core::Route::Emulators)).await.unwrap();
    let button = open_button(&test).expect("an Open Lichii button");
    assert_eq!(button.text, "Open Lichii");

    let open =
        Command::RunAction { plugin: lichii(), action: HOST_OPEN_EMULATOR.into(), game: None, args: button.args };
    test.core.dispatch(open).await.unwrap();

    let process = test.spawner.wait_for(2).await;
    let app_run = test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("squashfs-root/AppRun");
    assert_eq!(process.command.program, app_run);
    assert!(process.command.args.is_empty(), "no game: {:?}", process.command.args);
    let running = test.core.view().running.expect("the running screen");
    assert_eq!(running.title, "Lichii");

    let play = Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap()));
    let refused = test.core.dispatch(play).await.unwrap_err().to_string();
    assert!(refused.contains("Lichii is running"), "{refused}");

    process.exit(0);
    test.until(launcher_core::Area::Sessions, |core| core.view().running.is_none()).await;
    assert!(test.core.view().home.games.iter().all(|g| g.last_played.is_empty()), "the window is not play time");
}

/// Where the build of `tag` keeps Lichii's program.
fn app_run(test: &TestCore) -> std::path::PathBuf {
    test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("squashfs-root/AppRun")
}

/// An Xbox 360 pad as SDL maps it on Linux, from SDL's controller database.
fn xbox(port: u32) -> Controller {
    Controller {
        name: "Xbox 360 Controller".into(),
        guid: "030000005e0400008e02000014010000".into(),
        port,
        mapping: "a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b8,leftshoulder:b4,\
                  leftstick:b9,lefttrigger:a2,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b10,righttrigger:a5,\
                  rightx:a3,righty:a4,start:b7,x:b2,y:b3"
            .into(),
    }
}

/// Lichii's own settings file, as its window writes it: the keyboard for player 1.
const QT_CONFIG: &str = "[Renderer]\nbackend\\default=false\nbackend=1\n\n[Controls]\n\
player_0_connected\\default=true\nplayer_0_connected=true\n\
player_0_button_a\\default=true\nplayer_0_button_a=\"engine:keyboard,code:67,toggle:0\"\n";

/// A core with Lichii's official build, its eden-cli, Lichii's `qt-config.ini` ([`QT_CONFIG`])
/// and Celeste, and `controllers` connected.
async fn with_eden_cli(controllers: Vec<Controller>) -> (TestCore, tempfile::TempDir, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(Platform::Linux)).controllers(controllers);
    let releases = serde_json::json!([release(host.http(), TAG)]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(games.path().join("Switch")).unwrap();
    std::fs::write(games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp"), b"PFS0 game").unwrap();
    let eden = tempfile::tempdir().unwrap();
    std::fs::write(eden.path().join("qt-config.ini"), QT_CONFIG).unwrap();
    let test = host.built_in(launcher_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    extract_appimages(test.spawner.clone());
    for (name, path) in [("games", games.path()), ("lichii-config", eden.path())] {
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![path.into()] }).await.unwrap();
    }
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    std::fs::create_dir_all(app_run(&test).parent().unwrap().join("usr/bin")).unwrap();
    std::fs::write(app_run(&test).parent().unwrap().join("usr/bin/eden-cli"), b"\x7fELF eden-cli").unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    (test, games, eden)
}

/// Starts Celeste and returns its eden-cli command: the second process, after the AppImage's
/// unpacking.
async fn launch_celeste(test: &TestCore) -> launcher_testkit::SpawnedProcess {
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();
    test.spawner.wait_for(2).await
}

#[tokio::test]
async fn games_start_in_eden_cli_with_a_settings_file_that_gives_each_controller_a_player() {
    let (test, games, _eden) = with_eden_cli(vec![xbox(0), xbox(1)]).await;

    let process = launch_celeste(&test).await;
    assert_eq!(process.command.program, app_run(&test).parent().unwrap().join("usr/bin/eden-cli"));
    let args = &process.command.args;
    assert_eq!(args[0], "-c");
    let game = games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(args[2..], ["-f".to_owned(), "-g".to_owned(), game]);
    let file = std::fs::read_to_string(&args[1]).unwrap();
    let guid = "030000005e0400008e02000014010000";
    for line in [
        "[Renderer]".to_owned(),
        "backend=1".to_owned(),
        "airplane_mode\\default=false".to_owned(),
        "airplane_mode=false".to_owned(),
        "player_0_connected=true".to_owned(),
        "player_0_button_a\\default=false".to_owned(),
        format!("player_0_button_a=\"engine:sdl,port:0,guid:{guid},button:1\""),
        format!("player_0_button_b=\"engine:sdl,port:0,guid:{guid},button:0\""),
        format!("player_0_button_x=\"engine:sdl,port:0,guid:{guid},button:3\""),
        format!("player_0_button_zl=\"engine:sdl,port:0,guid:{guid},axis:2,threshold:0.5,invert:+\""),
        format!("player_0_button_dup=\"engine:sdl,port:0,guid:{guid},hat:0,direction:up\""),
        format!(
            "player_0_lstick=\"engine:sdl,port:0,guid:{guid},axis_x:0,axis_y:1,offset_x:0.000000,offset_y:0.000000,invert_x:+,invert_y:+\""
        ),
        "player_1_connected\\default=false".to_owned(),
        "player_1_connected=true".to_owned(),
        format!("player_1_button_a=\"engine:sdl,port:1,guid:{guid},button:1\""),
    ] {
        assert!(file.lines().any(|l| l == line), "{line} in:\n{file}");
    }
    assert!(!file.contains("engine:keyboard,code:67"), "player 1's keyboard keys give way to the controller");
    assert!(!file.contains("player_2_button_a="), "no third controller: player 3 stays as it was");
}

#[tokio::test]
async fn settings_show_airplane_mode_as_a_switch_that_is_off() {
    let (test, _games, _eden) = with_eden_cli(Vec::new()).await;
    test.core.dispatch(Command::Navigate(launcher_core::Route::Settings)).await.unwrap();
    let view = test.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == lichii()).expect("the Lichii panel");
    let keys: Vec<_> = panel.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["own_build", "fullscreen", "gpu_split", "airplane_mode"]);
    let field = panel.fields.iter().find(|f| f.key == "airplane_mode").expect("an airplane mode field");
    assert_eq!(field.label, "Airplane mode");
    assert_eq!(field.kind, launcher_ui_api::FieldKind::Toggle);
    assert_eq!(field.value, "false");
}

/// Lichii's settings file that can't be read stops the game, and says why: before, the game
/// started with Lichii's defaults, airplane mode on among them.
#[tokio::test]
async fn an_unreadable_lichii_settings_file_stops_the_launch_and_says_why() {
    let (test, _games, eden) = with_eden_cli(Vec::new()).await;
    std::fs::remove_file(eden.path().join("qt-config.ini")).unwrap();
    std::fs::create_dir(eden.path().join("qt-config.ini")).unwrap();

    let launch = Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap()));
    let refused = test.core.dispatch(launch).await;

    let shown = format!("{refused:?} {:?}", test.core.view().notices);
    assert!(shown.contains("qt-config.ini"), "the reason names the file: {shown}");
    assert_eq!(test.spawner.spawned().len(), 1, "only the AppImage's unpacking ran, no game");
}

/// No Lichii settings file yet (Lichii's window never saved one): the game starts with
/// Marah Launcher's settings over Lichii's defaults.
#[tokio::test]
async fn a_game_starts_without_a_lichii_settings_file() {
    let (test, _games, eden) = with_eden_cli(Vec::new()).await;
    std::fs::remove_file(eden.path().join("qt-config.ini")).unwrap();

    let process = launch_celeste(&test).await;

    let file = std::fs::read_to_string(&process.command.args[1]).unwrap();
    assert!(file.lines().any(|l| l == "airplane_mode=false"), "{file}");
}

#[tokio::test]
async fn a_game_starts_in_airplane_mode_once_the_player_turns_it_on() {
    let (test, _games, _eden) = with_eden_cli(Vec::new()).await;
    let on = Command::SetConfigField {
        plugin: lichii(),
        scope: ConfigScope::Emulator,
        key: "airplane_mode".into(),
        value: "true".into(),
    };
    test.core.dispatch(on).await.unwrap();

    let process = launch_celeste(&test).await;

    let file = std::fs::read_to_string(&process.command.args[1]).unwrap();
    assert!(file.lines().any(|l| l == "airplane_mode=true"), "airplane mode on in:\n{file}");
    assert!(!file.lines().any(|l| l == "airplane_mode=false"), "{file}");
}

/// A PFS0 of `files`, as an NSP or an NSZ holds them.
fn pfs0(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let (mut names, mut entries, mut offset) = (Vec::new(), Vec::new(), 0u64);
    for (name, data) in files {
        entries.extend_from_slice(&offset.to_le_bytes());
        entries.extend_from_slice(&(data.len() as u64).to_le_bytes());
        entries.extend_from_slice(&(names.len() as u32).to_le_bytes());
        entries.extend_from_slice(&[0; 4]);
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        offset += data.len() as u64;
    }
    let mut out = b"PFS0".to_vec();
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend(entries);
    out.extend(names);
    for (_, data) in files {
        out.extend_from_slice(data);
    }
    out
}

#[tokio::test]
async fn a_solid_nsz_installs_as_the_nsp_lichii_can_play() {
    let (test, games) = boot().await;
    // An NCA of a header and one section that isn't encrypted, and its solid NCZ, as nsz makes it.
    let header = vec![0x5Au8; 0x4000];
    let body: Vec<u8> = (0..0x2000u32).map(|i| (i % 256) as u8).collect();
    let mut ncz = header.clone();
    ncz.extend_from_slice(b"NCZSECTN");
    ncz.extend_from_slice(&1u64.to_le_bytes());
    for field in [0x4000u64, body.len() as u64, 1, 0] {
        ncz.extend_from_slice(&field.to_le_bytes());
    }
    ncz.extend_from_slice(&[0; 32]);
    ncz.extend(zstd::encode_all(body.as_slice(), 3).unwrap());
    let nsz = pfs0(&[("program.ncz", ncz), ("game.tik", b"ticket".to_vec())]);
    let packages = tempfile::tempdir().unwrap();
    std::fs::create_dir(packages.path().join("Switch")).unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974000][v0].nsz"), nsz).unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![packages.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::Navigate(launcher_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii()).unwrap();
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).unwrap();

    let start =
        Command::RunAction { plugin: lichii(), action: HOST_INSTALL_GAME.into(), game: None, args: offer.args.clone() };
    test.core.dispatch(start).await.unwrap();
    test.until(launcher_core::Area::Installs, |core| {
        core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing")
    })
    .await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    assert_eq!(job.state, "done", "{}", job.error);

    let folder = games.path().join("Switch/Hades");
    assert!(!folder.join("Hades [0100535012974000][v0].nsz").exists(), "no solid NSZ, which Lichii can't play");
    let nsp = std::fs::read(folder.join("Hades [0100535012974000][v0].nsp")).unwrap();
    let nca = [header, body].concat();
    assert_eq!(&nsp[..4], b"PFS0");
    assert!(nsp.windows(b"program.nca".len()).any(|w| w == b"program.nca"), "the NCZ is an NCA again");
    assert_eq!(&nsp[nsp.len() - nca.len() - 6..nsp.len() - 6], nca.as_slice(), "the NCA, then the ticket");
    assert_eq!(&nsp[nsp.len() - 6..], b"ticket");
}

#[tokio::test]
async fn a_game_that_starts_another_of_its_programs_is_started_again_in_the_same_session() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    let cli = app_run(&test).parent().unwrap().join("usr/bin/eden-cli");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(&cli, b"\x7fELF eden-cli").unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();

    // eden-cli exits with 75 when the game starts another of its programs (Sonic x Shadow
    // Generations' two games): its launcher starts it again with EDEN_CLI_RESTART_HANDOFF set.
    let first = test.spawner.wait_for(2).await;
    assert_eq!(first.command.env.get("EDEN_CLI_RESTART_HANDOFF"), None);
    first.exit(75);
    let again = test.spawner.wait_for(3).await;
    assert_eq!(again.command.program, first.command.program);
    assert_eq!(again.command.args, first.command.args, "the same game");
    assert_eq!(again.command.env.get("EDEN_CLI_RESTART_HANDOFF").map(String::as_str), Some("1"));
    let running = test.core.view().running.expect("still running, in the same session");
    assert_eq!(running.key, "emu-lichii:01002B30028F6000");
    assert!(test.core.view().notices.iter().all(|n| !n.contains("code 75")), "no error for a restart");

    again.exit(0);
    test.until(launcher_core::Area::Sessions, |core| core.view().running.is_none()).await;
    assert_eq!(test.spawner.spawned().len(), 3, "a normal exit ends the game");
}

#[tokio::test]
async fn after_stop_a_game_asking_to_start_again_is_not_started() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    let cli = app_run(&test).parent().unwrap().join("usr/bin/eden-cli");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(&cli, b"\x7fELF eden-cli").unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();
    let first = test.spawner.wait_for(2).await;
    let session = launcher_api::SessionId(test.core.view().running.expect("running").session);

    test.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();
    first.exit(75);
    test.until(launcher_core::Area::Sessions, |core| core.view().running.is_none()).await;
    assert_eq!(test.spawner.spawned().len(), 2, "Stop ends the game, whatever its code");
}

/// On Lichii's panel, a build action goes on to the build panel: "Use your own build" turns the
/// setting on.
#[tokio::test]
async fn a_build_action_on_lichii_s_panel_turns_its_setting_on() {
    let (test, _games) = boot().await;
    let own =
        Command::RunAction { plugin: lichii(), action: "use-own".into(), game: None, args: serde_json::Value::Null };
    test.core.dispatch(own).await.unwrap();

    test.core.dispatch(Command::Navigate(launcher_core::Route::Settings)).await.unwrap();
    let view = test.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == lichii()).expect("the Lichii panel");
    assert_eq!(panel.fields.iter().find(|f| f.key == "own_build").unwrap().value, "true");
}

/// Lichii lists no keyboard keys (its own window sets them), so its panel has no Keyboard
/// controls block: an empty one would say nothing. Its page has the Switch firmware and keys
/// block, which imports their archive; the launcher downloads neither.
#[tokio::test]
async fn lichii_s_page_has_the_firmware_and_keys_import_and_no_empty_keyboard_block() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::Navigate(launcher_core::Route::Plugin(lichii()))).await.unwrap();
    let view = test.core.view();
    let page = view.plugin_page.as_ref().expect("Lichii's page");
    let ids: Vec<_> = page.blocks.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids.first(), Some(&"system-files"), "{ids:?}");
    assert!(!ids.contains(&"keyboard") && !ids.contains(&"firmware"), "{ids:?}");
    let block = &page.blocks[0];
    assert_eq!(block.text, "Switch firmware and keys");
    let status = block.rows.iter().find(|r| r.label == "Status").expect("a status row");
    assert_eq!(status.value, "Missing");
    let import = block.rows.iter().find(|r| r.label == "Import…").expect("an import row");
    assert_eq!(import.action, HOST_IMPORT_SYSTEM_FILES);
    assert!(import.hint.contains("switch-firmware-keys.zip"), "{}", import.hint);
    assert!(test.core.view().question.is_none(), "the start asks nothing about firmware");
}

/// Lichii's name for this processor in its macOS and Windows files: `arm64` for Apple Silicon.
fn lichii_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    }
}

/// A release of [`TAG`] with Lichii's files, where `zip` (`lichii-macos-…zip` or
/// `lichii-windows-…zip`) holds `files` and its download redirects to GitHub's CDN.
fn zip_release(http: &FakeHttp, zip: &str, files: &[(&str, &[u8])]) -> serde_json::Value {
    let download = format!("https://github.com/zortos293/Lichii/releases/download/{TAG}/{zip}");
    let cdn = format!("https://release-assets.githubusercontent.com/lichii/{TAG}/{zip}");
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        // The programs keep their run bit, as in Lichii's zips.
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        archive.start_file(*name, options).unwrap();
        std::io::Write::write_all(&mut archive, bytes).unwrap();
    }
    let sha = http.serve(&cdn, archive.finish().unwrap().into_inner());
    http.redirect(&download, &cdn);
    let other = |n: &str| serde_json::json!({ "name": n, "browser_download_url": "https://github.com/x", "size": 1 });
    let arch = std::env::consts::ARCH;
    serde_json::json!({ "tag_name": TAG, "prerelease": true, "published_at": "2026-11-06T21:04:53Z", "assets": [
        other(&format!("lichii-linux-{arch}.AppImage")),
        other(&format!("lichii-windows-{arch}-pgo.zip")),
        other("lichii-macos-arm64.dmg"),
        { "name": zip, "browser_download_url": download, "size": 0, "digest": format!("sha256:{sha}") },
    ]})
}

/// A core on `platform` with Lichii's `zip` installed from a release, and Celeste.
async fn installed_from_zip(platform: Platform, zip: &str, files: &[(&str, &[u8])]) -> (TestCore, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(platform));
    let releases = serde_json::json!([zip_release(host.http(), zip, files)]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(games.path().join("Switch")).unwrap();
    std::fs::write(games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp"), b"PFS0 game").unwrap();
    let test = host.built_in(launcher_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    test.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    (test, games)
}

/// Starts Celeste and returns the first process: a zip build has no unpacking to run first.
async fn launch_celeste_first(test: &TestCore) -> launcher_testkit::SpawnedProcess {
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();
    test.spawner.wait_for(1).await
}

/// On a Mac, Lichii's app comes in a zip: games start in the eden-cli inside the app, and
/// Open Lichii starts the app's own program.
#[tokio::test]
async fn on_macos_the_app_from_the_zip_installs_and_games_start_in_its_eden_cli() {
    let zip = format!("lichii-macos-{}.zip", lichii_arch());
    let files: &[(&str, &[u8])] = &[
        ("lichii.app/Contents/MacOS/lichii", b"\xcf\xfa\xed\xfe lichii"),
        ("lichii.app/Contents/MacOS/eden-cli", b"\xcf\xfa\xed\xfe eden-cli"),
    ];
    let (test, games) = installed_from_zip(Platform::Macos, &zip, files).await;
    let app = test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("lichii.app/Contents/MacOS");
    assert!(app.join("lichii").is_file());

    let process = launch_celeste_first(&test).await;
    assert_eq!(process.command.program, app.join("eden-cli"));
    let game = games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(process.command.args[0], "-c");
    assert_eq!(process.command.args[2..], ["-f".to_owned(), "-g".to_owned(), game]);
}

/// On Windows, Lichii's zip holds lichii.exe and eden-cli.exe at its top.
#[tokio::test]
async fn on_windows_the_zip_installs_and_games_start_in_its_eden_cli() {
    let zip = format!("lichii-windows-{}.zip", std::env::consts::ARCH);
    let files: &[(&str, &[u8])] =
        &[("lichii.exe", b"MZ lichii"), ("eden-cli.exe", b"MZ eden-cli"), ("qt.conf", b"[Paths]\n")];
    let (test, games) = installed_from_zip(Platform::Windows, &zip, files).await;
    let build = test.data_dir().join("plugins/emu-lichii/builds").join(TAG);
    assert!(build.join("lichii.exe").is_file());

    let process = launch_celeste_first(&test).await;
    assert_eq!(process.command.program, build.join("eden-cli.exe"));
    let game = games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(process.command.args[2..], ["-f".to_owned(), "-g".to_owned(), game]);
}

/// Open Lichii on Windows starts lichii.exe, Lichii's window.
#[tokio::test]
async fn on_windows_open_lichii_starts_lichii_exe() {
    let zip = format!("lichii-windows-{}.zip", std::env::consts::ARCH);
    let files: &[(&str, &[u8])] = &[("lichii.exe", b"MZ lichii"), ("eden-cli.exe", b"MZ eden-cli")];
    let (test, _games) = installed_from_zip(Platform::Windows, &zip, files).await;
    test.core.dispatch(Command::Navigate(launcher_core::Route::Emulators)).await.unwrap();
    let button = open_button(&test).expect("an Open Lichii button");
    let open =
        Command::RunAction { plugin: lichii(), action: HOST_OPEN_EMULATOR.into(), game: None, args: button.args };
    test.core.dispatch(open).await.unwrap();
    let process = test.spawner.wait_for(1).await;
    assert_eq!(process.command.program, test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("lichii.exe"));
}

/// On Windows, Lichii keeps its data in %APPDATA%\lichii, and its settings in a config folder
/// inside it.
#[tokio::test]
async fn on_windows_lichii_s_folders_are_in_app_data() {
    let home = tempfile::tempdir().unwrap();
    let host = FakeHost::new().home(home.path()).platform(Some(Platform::Windows));
    let test = host.built_in(launcher_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    test.core.dispatch(Command::Navigate(launcher_core::Route::Plugin(lichii()))).await.unwrap();
    let folders = test.core.view().plugin_page.expect("Lichii's page").folders;
    let default = |name: &str| folders.iter().find(|f| f.name == name).map(|f| f.default.clone());
    let roaming = home.path().join("AppData/Roaming/lichii");
    assert_eq!(default("lichii-data"), Some(roaming.display().to_string()));
    assert_eq!(default("lichii-config"), Some(roaming.join("config").display().to_string()));
}

/// Lichii has no build for every processor (none for an Intel Mac): Install finds no version.
#[tokio::test]
async fn a_system_without_a_lichii_build_has_no_version_to_install() {
    let host = FakeHost::new().platform(Some(Platform::Macos));
    let other = |n: &str| serde_json::json!({ "name": n, "browser_download_url": "https://github.com/x", "size": 1 });
    let releases = serde_json::json!([{ "tag_name": TAG, "prerelease": true, "assets": [
        other("lichii-linux-x86_64.AppImage"), other("lichii-windows-x86_64.zip"),
    ]}]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let test = host.built_in(launcher_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    let refused = test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap_err();
    assert!(refused.to_string().contains("has no version to install"), "{refused}");
}

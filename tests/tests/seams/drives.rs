//! Seam 1, core app API: the drives. A system plugin with the `drives` capability lists them and
//! mounts, unmounts and ejects them; without one, the drives are the folders the system mounts
//! them in. A drive plugged in mounts on its own with a notice, a drive in use isn't unmounted,
//! a game folder's drive mounts at the start, and a drive taken out doesn't drop its games.

use std::path::{Path, PathBuf};

use emu_pc::PcPlugin;
use launcher_api::{GameKey, PluginBox, SystemDrive};
use launcher_core::system::{Place, SystemCommand};
use launcher_core::{Area, Command, DriveCmd, LibraryCmd, SessionCmd};
use launcher_testkit::{FakeHost, FakeSystem, TestCore};

fn stick(mount: Option<&Path>) -> SystemDrive {
    SystemDrive {
        device: "/dev/sda1".into(),
        disk: "/dev/sda".into(),
        uuid: "1A2B-3C4D".into(),
        label: "STICK".into(),
        model: "Ultra".into(),
        size: 64_000_000_000,
        filesystem: "exfat".into(),
        usb: true,
        mount: mount.map(Path::to_path_buf),
        free: Some(40_000_000_000),
        system: false,
    }
}

fn internal() -> SystemDrive {
    SystemDrive {
        device: "/dev/nvme0n1p4".into(),
        disk: "/dev/nvme0n1".into(),
        uuid: "5A1C".into(),
        label: "Games disk".into(),
        filesystem: "ntfs".into(),
        size: 500_000_000_000,
        ..SystemDrive::default()
    }
}

async fn drive(t: &TestCore, command: DriveCmd) -> launcher_core::CoreResult<()> {
    t.core.dispatch(Command::Drive(command)).await
}

/// The launcher as the whole screen: its own session, with no desktop.
fn whole_screen() -> Place {
    Place { whole_screen: true, ..Place::default() }
}

/// A machine with `drives`, mounting into a folder of the test's, with the launcher as the whole
/// screen.
async fn boot(system: &FakeSystem, drives: Vec<SystemDrive>, media: &Path) -> TestCore {
    boot_in(system, drives, media, whole_screen()).await
}

async fn boot_in(system: &FakeSystem, drives: Vec<SystemDrive>, media: &Path, place: Place) -> TestCore {
    let media = media.to_path_buf();
    system.with(|m| {
        m.drives = drives;
        m.mount_at = Some(media);
    });
    let host = FakeHost::new().place(place).plugin(system.plugin()).built_in(PluginBox::code(PcPlugin));
    let t = host.boot_core().await;
    drive(&t, DriveCmd::Refresh).await.unwrap();
    t
}

#[tokio::test]
async fn a_drive_plugged_in_mounts_on_its_own_and_a_notice_names_it() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let t = boot(&system, vec![internal()], media.path()).await;
    assert!(system.read(|m| m.drive_actions.is_empty()), "a drive that was there at the start stays as it is");

    system.with(|m| m.drives.push(stick(None)));
    drive(&t, DriveCmd::Refresh).await.unwrap();
    t.until(Area::Notices, |core| core.view().notices.iter().any(|n| n == "Drive STICK found")).await;
    assert_eq!(system.read(|m| m.drive_actions.clone()), ["mount /dev/sda1"]);
    assert!(media.path().join("STICK").is_dir());
}

#[tokio::test]
async fn on_a_desktop_a_plugged_drive_is_listed_and_the_desktop_mounts_it() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let t = boot_in(&system, Vec::new(), media.path(), Place::default()).await;
    system.with(|m| m.drives.push(stick(None)));
    drive(&t, DriveCmd::Refresh).await.unwrap();
    assert!(system.read(|m| m.drive_actions.is_empty()), "no mount of its own");
    assert!(t.core.view().notices.is_empty(), "no notice of its own");
    t.core.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();
    assert_eq!(t.core.view().system.drives.len(), 1, "listed, to mount");
}

#[tokio::test]
async fn the_drives_panel_has_each_drive_s_actions_and_the_notice_opens_it() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let t = boot(&system, vec![internal()], media.path()).await;
    system.with(|m| m.drives.push(stick(None)));
    drive(&t, DriveCmd::Refresh).await.unwrap();
    t.until(Area::Notices, |core| core.view().notices.iter().any(|n| n == "Drive STICK found")).await;
    t.core.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();

    let vm = t.core.view();
    let card = vm.notice_cards.iter().find(|n| n.text == "Drive STICK found").unwrap();
    assert_eq!((card.drive.as_str(), card.opens.as_str()), ("1A2B-3C4D", "Open or use for games"));
    assert!(vm.system.drives_available);
    let rows: Vec<(&str, &str, Vec<String>)> =
        vm.system.drives.iter().map(|r| (r.name.as_str(), r.kind.as_str(), r.actions.clone())).collect();
    assert_eq!(
        rows,
        [
            ("Games disk", "internal", vec!["mount".to_owned()]),
            ("STICK", "usb", vec!["open".to_owned(), "use".to_owned(), "eject".to_owned()]),
        ]
    );
    assert_eq!(vm.system.drives[0].detail, "500.0 GB · Not mounted · ntfs");
    assert_eq!(vm.system.drives[0].used, -1.0, "not mounted: the system doesn't say");
    assert_eq!(vm.system.drives[1].used, 0.375, "24 GB of 64 GB used");
    assert_eq!(vm.system.drives[1].path, media.path().join("STICK").display().to_string());
    assert!(vm.system.drives.iter().all(|d| d.busy.is_empty()), "no action runs");
}

#[tokio::test]
async fn a_plugged_drive_that_doesn_t_mount_says_why_in_its_notice() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let t = boot(&system, Vec::new(), media.path()).await;
    system.with(|m| {
        m.drives.push(stick(None));
        m.mount_refusals.push(("/dev/sda1".into(), "Not authorized to perform operation".into()));
    });
    drive(&t, DriveCmd::Refresh).await.unwrap();
    let said = "Drive STICK found, but it didn't mount: Not authorized to perform operation";
    t.until(Area::Notices, |core| core.view().notices.iter().any(|n| n == said)).await;
}

#[tokio::test]
async fn a_stick_ejects_its_whole_disk_and_an_internal_drive_only_unmounts() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let at = media.path().join("STICK");
    std::fs::create_dir_all(&at).unwrap();
    let t = boot(&system, vec![stick(Some(&at)), internal()], media.path()).await;

    let refused = drive(&t, DriveCmd::Eject("5A1C".into())).await.unwrap_err().to_string();
    assert!(refused.contains("Games disk can't be taken out: unmount it instead"), "{refused}");
    drive(&t, DriveCmd::Mount("5A1C".into())).await.unwrap();
    drive(&t, DriveCmd::Unmount("5A1C".into())).await.unwrap();
    drive(&t, DriveCmd::Eject("1A2B-3C4D".into())).await.unwrap();
    assert_eq!(
        system.read(|m| m.drive_actions.clone()),
        ["mount /dev/nvme0n1p4", "unmount /dev/nvme0n1p4", "eject /dev/sda"]
    );
    let gone = drive(&t, DriveCmd::Unmount("nope".into())).await.unwrap_err().to_string();
    assert!(gone.contains("no drive nope here"), "{gone}");
}

#[tokio::test]
async fn without_udisks_the_drives_are_the_mounted_folders_and_mounting_says_what_to_install() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    system.with(|m| m.unavailable = vec![("drives", "UDisks (udisksctl) isn't installed".into())]);
    let t = boot(&system, vec![internal()], media.path()).await;
    let refused = drive(&t, DriveCmd::Mount("5A1C".into())).await.unwrap_err().to_string();
    assert!(refused.contains("no drive 5A1C"), "the mount folders don't list it: {refused}");
    assert!(system.read(|m| m.drive_actions.is_empty()));
    t.core.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();
    let vm = t.core.view().system;
    assert!(!vm.drives_available);
    assert_eq!(vm.drives_reason, "UDisks (udisksctl) isn't installed");
    assert!(
        vm.drives.iter().flat_map(|r| &r.actions).all(|a| a == "open" || a == "use"),
        "no Mount or Eject: {:?}",
        vm.drives
    );
}

/// A PC game in `games`, which the PC plugin lists.
fn pc_game(games: &Path, name: &str) -> GameKey {
    let folder = games.join("PC").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("start.sh"), "#!/bin/sh\n").unwrap();
    format!("emu-pc:{name}").parse().unwrap()
}

/// Sets the game folders, reads the games, and opens `game`, so its folder is known.
async fn play_from(t: &TestCore, games: PathBuf, game: &GameKey) {
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games] }).await.unwrap();
    t.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();
    t.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();
}

#[tokio::test]
async fn a_drive_a_game_runs_from_isn_t_unmounted() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let at = media.path().join("STICK");
    let game = pc_game(&at.join("Games"), "Celeste");
    let t = boot(&system, vec![stick(Some(&at))], media.path()).await;
    play_from(&t, at.join("Games"), &game).await;

    t.core.dispatch(Command::Session(SessionCmd::Launch(game))).await.unwrap();
    let running = t.host.spawner.wait_for(1).await;
    let refused = drive(&t, DriveCmd::Eject("1A2B-3C4D".into())).await.unwrap_err().to_string();
    assert!(refused.contains("Close Celeste first: it runs from this drive"), "{refused}");
    assert!(system.read(|m| m.drive_actions.is_empty()));

    running.exit(0);
    t.until(Area::Sessions, |core| core.view().running.is_none()).await;
    drive(&t, DriveCmd::Eject("1A2B-3C4D".into())).await.unwrap();
}

#[tokio::test]
async fn a_game_folder_s_drive_mounts_before_the_games_are_read_and_a_drive_taken_out_keeps_its_games() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let at = media.path().join("STICK");
    let game = pc_game(&at.join("Games"), "Celeste");
    let t = boot(&system, vec![stick(Some(&at))], media.path()).await;
    play_from(&t, at.join("Games"), &game).await;
    let config = t.host.data_dir().join("config.json");
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(saved["app"]["folder_drives"][at.join("Games").to_str().unwrap()], "1A2B-3C4D");

    // Taken out: the plugin finds nothing there, and the library keeps the game.
    let away = media.path().join("away");
    std::fs::rename(&at, &away).unwrap();
    system.with(|m| m.drives.clear());
    drive(&t, DriveCmd::Refresh).await.unwrap();
    t.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();
    let titles: Vec<String> = t.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert!(titles.contains(&"Celeste".to_owned()), "{titles:?}");

    // The next start, with the drive plugged in and not mounted: it mounts first.
    let system = FakeSystem::new();
    system.with(|m| {
        m.drives = vec![stick(None)];
        m.mount_at = Some(media.path().to_path_buf());
    });
    let host = FakeHost::new().plugin(system.plugin()).built_in(PluginBox::code(PcPlugin));
    std::fs::copy(&config, host.data_dir().join("config.json")).unwrap();
    let next = host.boot_core().await;
    next.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();
    assert_eq!(system.read(|m| m.drive_actions.clone()), ["mount /dev/sda1"]);
    assert!(at.is_dir(), "mounted where the folder was");
}

#[tokio::test]
async fn while_an_action_runs_the_drive_says_which() {
    let (system, media) = (FakeSystem::new(), tempfile::tempdir().unwrap());
    let t = boot(&system, vec![internal()], media.path()).await;
    system.with(|m| m.hold_jobs = true);
    let core = t.core.clone();
    let mounting = tokio::spawn(async move { core.dispatch(Command::Drive(DriveCmd::Mount("5A1C".into()))).await });
    t.until(Area::System, |core| core.view().system.drives.first().is_some_and(|d| d.busy == "mount")).await;
    system.finish_jobs();
    mounting.await.unwrap().unwrap();
    let drive = t.core.view().system.drives[0].clone();
    assert_eq!(drive.busy, "", "it ended");
    assert_eq!(drive.actions, ["open", "use", "unmount"], "mounted: Open, Use for games, Unmount");
}

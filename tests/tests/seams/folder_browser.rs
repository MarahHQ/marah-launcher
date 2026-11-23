//! Seam 1, core app API: the folder browser to pick a folder, and a named folder's list. The
//! browser lists folders only when it picks, mounts a drive from its places and opens it, and
//! "Use this folder" adds the folder to the list. The list says the free space on each folder's
//! drive, and a folder can be added and taken out.

use std::path::Path;

use launcher_api::SystemDrive;
use launcher_core::{BrowseCmd, BrowsePurpose, Command};
use launcher_testkit::{FakeHost, FakeSystem, StubPlugin, TestCore};

/// A plugin that asks for the game folders.
fn games_plugin() -> StubPlugin {
    StubPlugin::from_toml(
        "id = \"pc\"\nkind = \"emulator\"\nname = \"PC games\"\nversion = \"1.0.0\"\nhost_api = \"^0.6\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"",
    )
}

async fn browse(t: &TestCore, command: BrowseCmd) -> launcher_core::CoreResult<()> {
    t.core.dispatch(Command::Browse(command)).await
}

fn pick(at: Option<&Path>) -> BrowseCmd {
    BrowseCmd::Open { at: at.map(Path::to_path_buf), purpose: BrowsePurpose::Pick("games".into()) }
}

fn games(t: &TestCore) -> serde_json::Value {
    t.core.kernel().config().get(&["app", "folders", "games"])
}

#[tokio::test]
async fn a_pick_lists_folders_only_and_use_adds_the_folder_shown_to_the_list() {
    let (mine, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let t = FakeHost::new().plugin(games_plugin()).boot_core().await;
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![mine.path().into()] }).await.unwrap();
    std::fs::create_dir_all(other.path().join("PC")).unwrap();
    let backup = format!("{}2026-01-01-000000.tar.gz", launcher_api::brand::FILES_BACKUP_PREFIX);
    std::fs::write(other.path().join(backup), b"").unwrap();

    browse(&t, pick(None)).await.unwrap();
    let shown = t.core.view().browser;
    assert_eq!((shown.purpose.as_str(), shown.folder.as_str()), ("pick", "games"));
    assert_eq!(shown.folder_title, "Game folders");
    assert_eq!(shown.shown, mine.path().display().to_string(), "it opens at the first game folder");
    assert!(shown.places.iter().all(|p| p.kind != "backups" && p.kind != "recent"), "{:?}", shown.places);

    browse(&t, BrowseCmd::Go(other.path().into())).await.unwrap();
    let names: Vec<(String, bool)> = t.core.view().browser.entries.into_iter().map(|e| (e.name, e.folder)).collect();
    assert_eq!(names, [("PC".to_owned(), true)], "no backup when it picks");

    browse(&t, BrowseCmd::Use).await.unwrap();
    assert!(!t.core.view().browser.open, "Use closes it");
    let both = [mine.path().display().to_string(), other.path().display().to_string()];
    assert_eq!(games(&t), serde_json::json!(both));
    browse(&t, pick(Some(other.path()))).await.unwrap();
    browse(&t, BrowseCmd::Use).await.unwrap();
    assert_eq!(games(&t), serde_json::json!(both), "a folder that is there already isn't added again");
}

#[tokio::test]
async fn use_is_refused_when_the_browser_restores() {
    let at = tempfile::tempdir().unwrap();
    let t = FakeHost::new().plugin(games_plugin()).boot_core().await;
    assert!(browse(&t, BrowseCmd::Use).await.is_err(), "nothing open");
    browse(&t, BrowseCmd::Open { at: Some(at.path().into()), purpose: BrowsePurpose::Restore }).await.unwrap();
    let refused = browse(&t, BrowseCmd::Use).await.unwrap_err().to_string();
    assert!(refused.contains("not to pick a folder"), "{refused}");
}

#[tokio::test]
async fn a_drive_that_isn_t_mounted_is_a_place_that_mounts_then_opens() {
    let (media, start) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let system = FakeSystem::new();
    system.with(|m| {
        m.mount_at = Some(media.path().to_path_buf());
        m.drives = vec![SystemDrive {
            device: "/dev/sda1".into(),
            disk: "/dev/sda".into(),
            uuid: "1A2B-3C4D".into(),
            label: "STICK".into(),
            size: 64_000_000_000,
            filesystem: "exfat".into(),
            usb: true,
            ..SystemDrive::default()
        }];
    });
    let t = FakeHost::new().plugin(system.plugin()).plugin(games_plugin()).boot_core().await;
    t.core.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh)).await.unwrap();

    browse(&t, pick(Some(start.path()))).await.unwrap();
    let stick = t.core.view().browser.places.into_iter().find(|p| p.kind == "drive" && p.name == "STICK").unwrap();
    assert!(!stick.mounted && stick.path.is_empty());
    assert!(stick.usb, "a stick's place says so, for its icon");
    assert_eq!((stick.drive.as_str(), stick.detail.as_str()), ("1A2B-3C4D", "64.0 GB"));

    browse(&t, BrowseCmd::Mount(stick.drive)).await.unwrap();
    assert_eq!(system.read(|m| m.drive_actions.clone()), ["mount /dev/sda1"]);
    let shown = t.core.view().browser;
    assert_eq!(shown.shown, media.path().join("STICK").display().to_string(), "the browser opens the drive");
    assert!(shown.places.iter().any(|p| p.name == "STICK" && p.mounted));
}

#[tokio::test]
async fn the_list_says_each_folder_s_free_space_and_a_folder_comes_and_goes() {
    let (there, gone) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap().path().join("away"));
    let t = FakeHost::new().free_space(120_500_000_000).plugin(games_plugin()).boot_core().await;
    // A folder typed as Settings always took it: separated by ;.
    t.core
        .dispatch(
            serde_json::from_value(serde_json::json!({ "SetFolders": { "name": "games", "paths": there.path() } }))
                .unwrap(),
        )
        .await
        .unwrap();
    let add = Command::AddFolder { name: "games".into(), path: gone.clone() };
    t.core.dispatch(add).await.unwrap();

    let folder = t.core.view().settings.folders.into_iter().find(|f| f.name == "games").unwrap();
    let list: Vec<(String, String, bool)> = folder.list.into_iter().map(|p| (p.path, p.free, p.there)).collect();
    assert_eq!(
        list,
        [
            (there.path().display().to_string(), "120.5 GB free".to_owned(), true),
            (gone.display().to_string(), "Not there".to_owned(), false),
        ]
    );

    t.core.dispatch(Command::RemoveFolder { name: "games".into(), path: gone.clone() }).await.unwrap();
    assert_eq!(games(&t), serde_json::json!([there.path().display().to_string()]));
    let again = t.core.dispatch(Command::RemoveFolder { name: "games".into(), path: gone }).await;
    assert!(again.is_err(), "not in the list any more");
    assert!(there.path().is_dir(), "Remove leaves the folder");
}

#[tokio::test]
async fn the_backup_folder_is_chosen_in_the_browser_and_says_its_free_space() {
    let (home, chosen) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let t = FakeHost::new().home(home.path()).free_space(120_500_000_000).boot_core().await;
    std::fs::create_dir_all(home.path().join(launcher_core::DEFAULT_BACKUP_FOLDER)).unwrap();
    std::fs::create_dir_all(chosen.path().join("old-pc")).unwrap();
    let backup = format!("{}2026-01-01-000000.tar.gz", launcher_api::brand::FILES_BACKUP_PREFIX);
    std::fs::write(chosen.path().join(backup), b"").unwrap();
    // The free space is read ahead, on the next tick of the machine's settings.
    t.host.clock.advance(std::time::Duration::from_secs(5)).await;
    t.until(launcher_core::Area::Hardware, |core| core.view().settings.backup.free == "120.5 GB free").await;

    let open = BrowseCmd::Open { at: None, purpose: BrowsePurpose::BackupFolder };
    browse(&t, open).await.unwrap();
    let shown = t.core.view().browser;
    assert_eq!((shown.purpose.as_str(), shown.title.as_str()), ("pick", "Choose the backup folder"));
    let default = home.path().join(launcher_core::DEFAULT_BACKUP_FOLDER);
    assert_eq!(shown.shown, "~/".to_owned() + launcher_core::DEFAULT_BACKUP_FOLDER, "it opens at the backup folder");
    assert!(default.is_dir());

    browse(&t, BrowseCmd::Go(chosen.path().into())).await.unwrap();
    let names: Vec<String> = t.core.view().browser.entries.into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["old-pc"], "folders only");
    browse(&t, BrowseCmd::Use).await.unwrap();
    assert!(!t.core.view().browser.open);
    let saved = t.core.kernel().config().get(&["app", "backup_folder"]);
    assert_eq!(saved, serde_json::json!(chosen.path().display().to_string()), "one folder, not a list");

    // "Use the default folder": empty goes back to it.
    t.core.dispatch(Command::Pref(launcher_core::Pref::BackupFolder, serde_json::Value::Null)).await.unwrap();
    assert_eq!(t.core.view().settings.backup.folder, "~/".to_owned() + launcher_core::DEFAULT_BACKUP_FOLDER);
}

/// A stick plugged in since the last read of the drives: the browser lists it, and mounts it.
#[tokio::test]
async fn a_drive_plugged_in_just_now_mounts_from_the_browser() {
    let (media, start) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let system = FakeSystem::new();
    system.with(|m| m.mount_at = Some(media.path().to_path_buf()));
    let t = FakeHost::new().plugin(system.plugin()).plugin(games_plugin()).boot_core().await;
    t.core.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh)).await.unwrap();
    // The start's own read of the drives has run: the next read is the tick's, 5 s away.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    system.with(|m| {
        m.drives = vec![SystemDrive {
            device: "/dev/sda1".into(),
            disk: "/dev/sda".into(),
            uuid: "NEW".into(),
            label: "NEW".into(),
            filesystem: "exfat".into(),
            usb: true,
            ..SystemDrive::default()
        }]
    });
    browse(&t, pick(Some(start.path()))).await.unwrap();
    assert!(t.core.view().browser.places.iter().any(|p| p.drive == "NEW"), "listed");
    browse(&t, BrowseCmd::Mount("NEW".into())).await.unwrap();
    assert_eq!(t.core.view().browser.shown, media.path().join("NEW").display().to_string());
}

/// Building the view asks the drives nothing: their free space is read ahead, so a drive that
/// hangs can't hold the screen. A change of the folders reads it again.
#[tokio::test]
async fn the_view_reads_the_drives_free_space_ahead_not_on_each_build() {
    let folder = tempfile::tempdir().unwrap();
    let host = FakeHost::new().free_space(120_500_000_000).plugin(games_plugin());
    let t = host.boot_core().await;
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![folder.path().into()] }).await.unwrap();
    // The drives panic if a build asks them anything.
    launcher_testkit::no_drive_calls(|| {
        for _ in 0..5 {
            let view = t.core.view();
            assert_eq!(view.settings.folders[0].list[0].free, "120.5 GB free");
            assert_eq!(view.settings.backup.free, "", "no backup folder there");
        }
    });
}

/// A drive inside the PC is a place without the USB mark: its icon is a disk.
#[tokio::test]
async fn a_drive_inside_the_pc_is_a_place_that_isn_t_usb() {
    let start = tempfile::tempdir().unwrap();
    let system = FakeSystem::new();
    system.with(|m| {
        m.drives = vec![SystemDrive {
            device: "/dev/sdb1".into(),
            disk: "/dev/sdb".into(),
            uuid: "INSIDE".into(),
            label: "Games disk".into(),
            filesystem: "ntfs".into(),
            ..SystemDrive::default()
        }]
    });
    let t = FakeHost::new().plugin(system.plugin()).plugin(games_plugin()).boot_core().await;
    browse(&t, pick(Some(start.path()))).await.unwrap();
    let place = t.core.view().browser.places.into_iter().find(|p| p.drive == "INSIDE").unwrap();
    assert!(!place.usb);
}

/// The browser opens at once with the drives last read, and says its places are still loading
/// while it reads the drives; the drive it finds comes into the places when the read ends.
#[tokio::test]
async fn the_browser_opens_at_once_and_its_places_load_while_the_drives_are_read() {
    let start = tempfile::tempdir().unwrap();
    let system = FakeSystem::new();
    let host = FakeHost::new().call_timeout(std::time::Duration::from_secs(120));
    let t = host.plugin(system.plugin()).plugin(games_plugin()).boot_core().await;
    t.core.dispatch(Command::Drive(launcher_core::DriveCmd::Refresh)).await.unwrap();
    system.with(|m| {
        m.drives = vec![SystemDrive {
            device: "/dev/sda1".into(),
            uuid: "NEW".into(),
            label: "NEW".into(),
            usb: true,
            ..SystemDrive::default()
        }];
        m.slow = vec![("drives", std::time::Duration::from_secs(30))];
    });
    let core = t.core.clone();
    let open = tokio::spawn(async move { core.dispatch(Command::Browse(pick(Some(start.path())))).await });
    t.clock.run_until_idle().await;
    let shown = t.core.view().browser;
    assert!(shown.open && shown.places_loading, "open, with its places loading");
    assert!(!shown.places.iter().any(|p| p.drive == "NEW"), "not read yet");
    t.clock.advance(std::time::Duration::from_secs(31)).await;
    open.await.unwrap().unwrap();
    let shown = t.core.view().browser;
    assert!(!shown.places_loading);
    assert!(shown.places.iter().any(|p| p.drive == "NEW"), "the drive read comes in");
}

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use launcher_api::SystemJob;

use super::*;

const GB: u64 = 1_000_000_000;

/// Drives by the folder they hold, each with its free bytes; a drive's id is its place in the
/// list, and its size is ten times its free space.
struct Space(Vec<(PathBuf, u64)>);

impl DriveSpace for Space {
    fn free_bytes(&self, path: &Path) -> Option<u64> {
        self.0.iter().find(|(drive, _)| path.starts_with(drive)).map(|(_, free)| *free)
    }

    fn size_bytes(&self, path: &Path) -> Option<u64> {
        self.free_bytes(path).map(|free| free * 10)
    }

    fn drive_id(&self, path: &Path) -> Option<u64> {
        self.0.iter().position(|(drive, _)| path.starts_with(drive)).map(|at| at as u64)
    }
}

/// An adapter that lists what the test sets, or fails while `None`, and mounts at
/// `/media/<label>`.
#[derive(Default)]
struct Fake {
    list: Mutex<Option<Vec<Drive>>>,
    acted: Mutex<Vec<String>>,
}

impl Fake {
    fn set(&self, list: Option<Vec<Drive>>) {
        *self.list.lock().unwrap() = list;
    }
}

#[async_trait]
impl Adapter for Fake {
    fn kind(&self) -> &'static str {
        "fake"
    }

    async fn list(&self) -> CoreResult<Vec<Drive>> {
        self.list.lock().unwrap().clone().ok_or_else(|| CoreError::Invalid("lsblk failed".into()))
    }

    async fn mount(&self, drive: &Drive, _: Arc<dyn SystemJob>) -> CoreResult<PathBuf> {
        self.acted.lock().unwrap().push(format!("mount {}", drive.id));
        let at = PathBuf::from("/media").join(&drive.label);
        let mut list = self.list.lock().unwrap();
        for d in list.iter_mut().flatten().filter(|d| d.id == drive.id) {
            d.mount = Some(at.clone());
        }
        Ok(at)
    }

    async fn unmount(&self, drive: &Drive, _: Arc<dyn SystemJob>) -> CoreResult<()> {
        self.acted.lock().unwrap().push(format!("unmount {}", drive.id));
        Ok(())
    }

    async fn eject(&self, drive: &Drive, _: Arc<dyn SystemJob>) -> CoreResult<()> {
        self.acted.lock().unwrap().push(format!("eject {}", drive.id));
        Ok(())
    }
}

/// A job no one watches.
struct NoJob;

impl SystemJob for NoJob {
    fn progress(&self, _: Option<u8>, _: &str) {}
    fn done(&self, _: &str) {}
    fn failed(&self, _: &str) {}
    fn cancelled(&self) -> bool {
        false
    }
}

fn job() -> Arc<dyn SystemJob> {
    Arc::new(NoJob)
}

fn drive(id: &str, mount: Option<&Path>) -> Drive {
    Drive { id: id.into(), label: id.into(), mount: mount.map(Path::to_path_buf), ..Drive::default() }
}

fn with(space: Vec<(PathBuf, u64)>) -> (Drives, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    (Drives::new(Arc::new(Space(space)), fake.clone()), fake)
}

#[test]
fn a_folder_that_is_not_there_yet_is_on_the_drive_of_the_folder_above() {
    let home = tempfile::tempdir().unwrap();
    let downloads = home.path().join("Downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let space = Space(vec![(downloads.clone(), GB), (home.path().to_owned(), 8 * GB)]);
    assert_eq!(free(&space, &downloads.join("PS5/torrent-1")), Some(GB));
    assert_eq!(size(&space, &downloads.join("PS5/torrent-1")), Some(10 * GB));
    assert_eq!(free(&space, &home.path().join("Games/PS5")), Some(8 * GB));
}

#[test]
fn two_folders_share_a_drive_only_when_the_system_says_so() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (home.path().join("a"), home.path().join("b"));
    std::fs::create_dir_all(&a).unwrap();
    let space = Space(vec![(a.clone(), GB), (home.path().to_owned(), GB)]);
    assert!(same_drive(&space, &b, &home.path().join("c")), "both on the home drive");
    assert!(!same_drive(&space, &a, &b), "a has a drive of its own");
    assert!(!same_drive(&crate::NoDriveSpace, &b, &b), "the system doesn't say");
}

#[tokio::test]
async fn the_list_has_each_mounted_drive_s_free_space_and_size() {
    let stick = tempfile::tempdir().unwrap();
    let (drives, fake) = with(vec![(stick.path().to_owned(), 3 * GB)]);
    fake.set(Some(vec![drive("STICK", Some(stick.path())), drive("DISK", None)]));
    let list = drives.list().await.unwrap();
    assert_eq!((list[0].free, list[0].size), (Some(3 * GB), Some(30 * GB)));
    assert_eq!((list[1].free, list[1].size), (None, None), "not mounted: the system says nothing");
}

#[tokio::test]
async fn the_first_read_finds_what_is_there_then_each_read_says_what_came_and_went() {
    let (drives, fake) = with(Vec::new());
    fake.set(Some(vec![drive("A", None)]));
    assert_eq!(drives.refresh(None).await, Changes::default(), "the first read: nothing came");
    assert_eq!(drives.known().len(), 1);

    fake.set(Some(vec![drive("B", None)]));
    let changes = drives.refresh(None).await;
    assert_eq!(changes.plugged.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["B"]);
    assert_eq!(changes.unplugged.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["A"]);
}

#[tokio::test]
async fn another_kind_of_adapter_finds_what_is_there_as_a_first_read_does() {
    let (drives, fake) = with(Vec::new());
    fake.set(Some(vec![drive("A", None)]));
    drives.refresh(None).await;
    drives.use_adapter(fake.clone());
    let folders = MountFolders { parents: Vec::new(), roots: vec![std::env::temp_dir()] };
    drives.use_adapter(Arc::new(folders));
    assert_eq!(drives.refresh(None).await, Changes::default(), "not A gone and a folder come");
    assert_eq!(drives.known().len(), 1);
}

#[tokio::test]
async fn a_read_that_fails_changes_nothing() {
    let (drives, fake) = with(Vec::new());
    fake.set(Some(vec![drive("A", None)]));
    drives.refresh(None).await;
    fake.set(None);
    assert_eq!(drives.refresh(None).await, Changes::default(), "not every drive unplugged");
    assert_eq!(drives.known().len(), 1);
    fake.set(Some(vec![drive("A", None)]));
    assert_eq!(drives.refresh(None).await, Changes::default(), "A was there all along");
}

#[tokio::test]
async fn mounting_goes_through_the_adapter_and_the_list_is_read_again() {
    let (drives, fake) = with(Vec::new());
    fake.set(Some(vec![drive("A", None)]));
    drives.refresh(None).await;
    assert_eq!(drives.mount("A", job()).await.unwrap(), Path::new("/media/A"));
    assert_eq!(drives.known()[0].mount.as_deref(), Some(Path::new("/media/A")));
    assert_eq!(drives.mount("A", job()).await.unwrap(), Path::new("/media/A"), "mounted already: no second mount");
    drives.eject("A", job()).await.unwrap();
    assert_eq!(*fake.acted.lock().unwrap(), ["mount A", "eject A"]);
    assert!(drives.mount("Z", job()).await.unwrap_err().to_string().contains("no drive Z"));
}

#[tokio::test]
async fn a_drive_that_comes_during_an_action_is_still_found_by_the_next_read() {
    let (drives, fake) = with(Vec::new());
    fake.set(Some(vec![drive("A", None)]));
    drives.refresh(None).await;
    fake.set(Some(vec![drive("A", None), drive("B", None)]));
    drives.mount("A", job()).await.unwrap();
    assert_eq!(drives.known().len(), 1, "B waits for the next read");
    let changes = drives.refresh(None).await;
    assert_eq!(changes.plugged.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["B"]);
}

#[tokio::test]
async fn the_mount_folders_list_each_drive_mounted_in_them_and_can_t_mount() {
    let media = tempfile::tempdir().unwrap();
    let (stick, disk) = (media.path().join("STICK"), media.path().join("Games disk"));
    std::fs::create_dir_all(&stick).unwrap();
    std::fs::create_dir_all(&disk).unwrap();
    std::fs::write(media.path().join("a file"), "").unwrap();
    let folders =
        MountFolders { parents: vec![media.path().to_owned(), media.path().join("not there")], roots: Vec::new() };
    let list = folders.list().await.unwrap();
    let seen: Vec<_> = list.iter().map(|d| (d.label.as_str(), d.mount.as_deref())).collect();
    assert_eq!(seen, [("Games disk", Some(disk.as_path())), ("STICK", Some(stick.as_path()))]);
    let refused = folders.mount(&list[0], job()).await.unwrap_err().to_string();
    assert!(refused.contains("Mounting needs UDisks"), "{refused}");
}

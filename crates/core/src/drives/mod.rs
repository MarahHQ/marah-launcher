//! Drives: the one place for drive questions. Which drives there are, mounted or not, the free
//! space and the size of the drive that holds a folder, whether two folders share a drive, and
//! mounting, unmounting and ejecting.
//!
//! Two parts answer them:
//! - The [`DriveSpace`] port: quick system calls on a path (the app's `disk.rs`). Every system
//!   has them, and they answer at once, so the room checks call them on the spot.
//! - An [`Adapter`]: the list of drives and the actions. The base adapter, [`MountFolders`],
//!   lists the folders the system mounts drives in, on every system, and can't mount anything.
//!   A system plugin with the `drives` capability brings a better one.
//!
//! [`Drives::refresh`] reads the list again and says which drives came and went since the last
//! read. The flows over the core (the commands, a plugged drive, the game folders' drives at the
//! start) are in [`flows`].

mod flows;
#[cfg(test)]
mod tests;
mod udisks;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use launcher_api::SystemJob;

pub use flows::DriveCmd;
pub(crate) use flows::{away_games, handle, mount, mount_game_drives, refresh, remember_folders};

use crate::{CoreError, CoreResult, DriveSpace};

/// One drive: a USB stick, a disk, or a part of a disk with a file system on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drive {
    /// What stays the same from one plug-in to the next: the file system's UUID, else the
    /// device or the mount path.
    pub id: String,
    /// The system's device, such as `/dev/sdb1`. Empty when the adapter doesn't name one.
    pub device: String,
    /// The whole disk it is on, such as `/dev/sdb`, which an eject powers off. Empty when the
    /// adapter doesn't name one.
    pub disk: String,
    /// The name the player knows it by: its label, else its mount folder's name, else its model.
    pub label: String,
    /// Its size in bytes; `None` when the system doesn't say.
    pub size: Option<u64>,
    /// Its free bytes, while it is mounted; `None` when the system doesn't say.
    pub free: Option<u64>,
    /// Its file system, such as `ext4` or `exfat`. Empty when the system doesn't say.
    pub filesystem: String,
    /// It is plugged in by USB, or into another port the player may take it out of.
    pub usb: bool,
    /// Where it is mounted; `None` while it isn't.
    pub mount: Option<PathBuf>,
}

/// The drives that came and went between two reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub plugged: Vec<Drive>,
    pub unplugged: Vec<Drive>,
}

/// Where the list of drives and the actions come from.
#[async_trait]
pub trait Adapter: Send + Sync {
    /// What kind of adapter it is. The drives' ids are the adapter's own, so another kind starts
    /// the list again.
    fn kind(&self) -> &'static str;
    /// Every drive there is now, mounted or not.
    async fn list(&self) -> CoreResult<Vec<Drive>>;
    /// Mounts the drive, and returns where. `job` shows the player how it goes; the action ends
    /// it, or the caller does with the action's error.
    async fn mount(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<PathBuf>;
    async fn unmount(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<()>;
    /// Unmounts every part of the drive's disk, then powers the disk off, so it can be taken out.
    async fn eject(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<()>;
}

/// The free bytes on the drive that holds `folder`, which may not be there yet: the drive of the
/// nearest folder above it. `None` when the system doesn't say.
pub(crate) fn free(space: &dyn DriveSpace, folder: &Path) -> Option<u64> {
    space.free_bytes(nearest(folder)?)
}

/// The size in bytes of the drive that holds `folder`, which may not be there yet. `None` when
/// the system doesn't say.
pub(crate) fn size(space: &dyn DriveSpace, folder: &Path) -> Option<u64> {
    space.size_bytes(nearest(folder)?)
}

/// `a` and `b`, which may not be there yet, are on one drive and share its free space. `false`
/// when the system doesn't say.
pub(crate) fn same_drive(space: &dyn DriveSpace, a: &Path, b: &Path) -> bool {
    let id = |folder: &Path| space.drive_id(nearest(folder)?);
    id(a).is_some_and(|a| id(b) == Some(a))
}

/// The nearest folder at or above `folder` that is there.
fn nearest(folder: &Path) -> Option<&Path> {
    folder.ancestors().find(|p| p.is_dir())
}

/// The drives, through the adapter in use, and the list as it was last read.
pub(crate) struct Drives {
    space: Arc<dyn DriveSpace>,
    adapter: Mutex<Arc<dyn Adapter>>,
    /// The list at the last read; `None` before the first one.
    last: Mutex<Option<Vec<Drive>>>,
}

impl Drives {
    pub(crate) fn new(space: Arc<dyn DriveSpace>, adapter: Arc<dyn Adapter>) -> Self {
        Self { space, adapter: Mutex::new(adapter), last: Mutex::new(None) }
    }

    fn adapter(&self) -> Arc<dyn Adapter> {
        lock(&self.adapter).clone()
    }

    /// The adapter names drives by their file system (the system plugin's), so a drive it
    /// doesn't list isn't plugged in. The mount folders' adapter lists mount folders only.
    pub(crate) fn names_by_file_system(&self) -> bool {
        self.adapter().kind() != "mount folders"
    }

    /// Uses `adapter` from now on. With another kind of adapter, the next read finds what is
    /// there, as the first one does: the ids changed, not the drives.
    pub(crate) fn use_adapter(&self, adapter: Arc<dyn Adapter>) {
        let was = std::mem::replace(&mut *lock(&self.adapter), adapter.clone());
        if was.kind() != adapter.kind() {
            *lock(&self.last) = None;
        }
    }

    /// Every drive there is now, each mounted one with its free space, and its size when the
    /// adapter didn't say.
    pub(crate) async fn list(&self) -> CoreResult<Vec<Drive>> {
        let mut drives = self.adapter().list().await?;
        for drive in &mut drives {
            if let Some(mount) = &drive.mount {
                drive.free = drive.free.or_else(|| free(self.space.as_ref(), mount));
                drive.size = drive.size.or_else(|| size(self.space.as_ref(), mount));
            }
        }
        Ok(drives)
    }

    /// The list as it was last read; empty before the first read.
    pub(crate) fn known(&self) -> Vec<Drive> {
        lock(&self.last).clone().unwrap_or_default()
    }

    /// Reads the list again, and says which drives came and went since the last read. The first
    /// read finds what is there: nothing came. A read that fails changes nothing.
    pub(crate) async fn refresh(&self) -> Changes {
        let now = match self.list().await {
            Ok(now) => now,
            Err(e) => {
                tracing::debug!(error = %e, "the drives weren't read: they stay as they were");
                return Changes::default();
            }
        };
        let before = lock(&self.last).replace(now.clone());
        let Some(before) = before else { return Changes::default() };
        let had = |list: &[Drive], drive: &Drive| list.iter().any(|d| d.id == drive.id);
        Changes {
            plugged: now.iter().filter(|d| !had(&before, d)).cloned().collect(),
            unplugged: before.iter().filter(|d| !had(&now, d)).cloned().collect(),
        }
    }

    /// Reads the drives again after an action, for where each is mounted now. The drives that
    /// came and went stay for the next [`Drives::refresh`] to find: a drive taken out stays
    /// listed, and one plugged in isn't listed yet.
    async fn reread(&self) {
        let Ok(now) = self.list().await else { return };
        let mut last = lock(&self.last);
        let Some(last) = last.as_mut() else { return };
        for drive in last.iter_mut() {
            if let Some(read) = now.iter().find(|d| d.id == drive.id) {
                *drive = read.clone();
            }
        }
    }

    /// The drive with this id in the last list.
    pub(crate) fn find(&self, id: &str) -> CoreResult<Drive> {
        self.known()
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| CoreError::Invalid(format!("no drive {id} here: it was taken out")))
    }

    /// Mounts the drive with this id, and returns where. One that is mounted already isn't
    /// mounted again.
    pub(crate) async fn mount(&self, id: &str, job: Arc<dyn SystemJob>) -> CoreResult<PathBuf> {
        let drive = self.find(id)?;
        if let Some(mount) = drive.mount {
            job.done(&mount.display().to_string());
            return Ok(mount);
        }
        let at = self.adapter().mount(&drive, job).await;
        self.reread().await;
        at
    }

    pub(crate) async fn unmount(&self, id: &str, job: Arc<dyn SystemJob>) -> CoreResult<()> {
        let drive = self.find(id)?;
        let done = self.adapter().unmount(&drive, job).await;
        self.reread().await;
        done
    }

    pub(crate) async fn eject(&self, id: &str, job: Arc<dyn SystemJob>) -> CoreResult<()> {
        let drive = self.find(id)?;
        let done = self.adapter().eject(&drive, job).await;
        self.reread().await;
        done
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The base adapter: the folders the system mounts drives in, on every system. It lists the
/// drives that are mounted there, and can't mount, unmount or eject one.
pub(crate) struct MountFolders {
    /// Folders whose subfolders are drives: `/run/media/<user>`, `/media/<user>`, `/mnt`, and
    /// `/Volumes` on macOS.
    pub parents: Vec<PathBuf>,
    /// Folders that are drives themselves: `C:\` to `Z:\` on Windows.
    pub roots: Vec<PathBuf>,
}

impl MountFolders {
    /// This system's mount folders. The user's name is the home folder's.
    pub(crate) fn here(home: Option<&Path>) -> Self {
        if cfg!(target_os = "windows") {
            let roots = ('C'..='Z').map(|d| PathBuf::from(format!("{d}:\\"))).collect();
            return Self { parents: Vec::new(), roots };
        }
        if cfg!(target_os = "macos") {
            return Self { parents: vec![PathBuf::from("/Volumes")], roots: Vec::new() };
        }
        let mut parents = Vec::new();
        if let Some(user) = home.and_then(|h| h.file_name()) {
            parents.push(Path::new("/run/media").join(user));
            parents.push(Path::new("/media").join(user));
        }
        parents.push(PathBuf::from("/mnt"));
        Self { parents, roots: Vec::new() }
    }
}

/// What the base adapter says when asked to act.
pub(crate) const CANT_ACT: &str =
    "Mounting needs UDisks. Install udisks2, or mount the drive with the system's own tools";

#[async_trait]
impl Adapter for MountFolders {
    fn kind(&self) -> &'static str {
        "mount folders"
    }

    async fn list(&self) -> CoreResult<Vec<Drive>> {
        let (parents, roots) = (self.parents.clone(), self.roots.clone());
        let found = tokio::task::spawn_blocking(move || {
            let mut found: Vec<PathBuf> = Vec::new();
            for parent in parents {
                let Ok(read) = std::fs::read_dir(&parent) else { continue };
                let mut children: Vec<PathBuf> =
                    read.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect();
                children.sort();
                found.extend(children);
            }
            found.extend(roots.into_iter().filter(|p| p.is_dir()));
            found
        })
        .await
        .map_err(|e| CoreError::Invalid(e.to_string()))?;
        Ok(found
            .into_iter()
            .map(|path| Drive {
                id: path.display().to_string(),
                label: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                mount: Some(path),
                ..Drive::default()
            })
            .collect())
    }

    async fn mount(&self, _: &Drive, _: Arc<dyn SystemJob>) -> CoreResult<PathBuf> {
        Err(CoreError::Invalid(CANT_ACT.into()))
    }

    async fn unmount(&self, _: &Drive, _: Arc<dyn SystemJob>) -> CoreResult<()> {
        Err(CoreError::Invalid(CANT_ACT.into()))
    }

    async fn eject(&self, _: &Drive, _: Arc<dyn SystemJob>) -> CoreResult<()> {
        Err(CoreError::Invalid(CANT_ACT.into()))
    }
}

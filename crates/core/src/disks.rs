//! The disks the catalogs' downloads go to: each console's download folder and the room on the
//! disk that holds it. A Game Hub says when a release doesn't fit. Also, for each download's
//! sheet, the room where its files go and where its game installs.
//!
//! The view reads these on every render, so they are read ahead: [`refresh`] runs when a Game
//! Hub opens, the catalogs are read, a folder changes, a download starts or ends, and on the
//! system's 5 s tick, and keeps the answer in the core's state. So does the free space of each
//! host folder's folders and of the backup folder: a drive that hangs can't hold a render. Each answer is a [`Room`]: the view asks it the same question
//! the command asks its own reading on the click.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use launcher_api::PluginId;

use crate::Core;
use crate::changes::Area;
use crate::folders::Purpose;
use crate::room::Room;

/// Where a console's downloads go, and the room there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadFolder {
    /// The console's folder in the emulator's packages folder. It may not be there yet.
    pub path: PathBuf,
    /// The room on the disk that holds it.
    pub room: Room,
}

/// Each console's download folder, by the emulator and the console a catalog's offers name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Disks {
    folders: Arc<BTreeMap<(PluginId, String), DownloadFolder>>,
    /// The room where each download's files go and in its emulator's game folder, the one the
    /// install sheet offers, by the download's id.
    downloads: Arc<BTreeMap<String, Room>>,
    /// The free bytes on the drive of each host folder's folder that is there; `None` when the
    /// system doesn't say. Settings' folder lists show it.
    pub roots_free: Arc<BTreeMap<PathBuf, Option<u64>>>,
    /// The free bytes on the backup folder's drive, while the folder is there.
    pub backup_free: Option<u64>,
}

impl Disks {
    /// Where `emulator`'s downloads of `console`'s games go; `None` when it has no packages
    /// folder, or the catalogs offered none of its games when the disks were last read.
    pub fn download_folder(&self, emulator: &PluginId, console: &str) -> Option<&DownloadFolder> {
        self.folders.get(&(emulator.clone(), console.to_owned()))
    }

    /// The room for download `id`: where its files go and where its game installs. A download
    /// the disks weren't read for has room.
    pub fn room(&self, id: &str) -> Room {
        self.downloads.get(id).copied().unwrap_or_default()
    }
}

/// Reads the download folder of every emulator and console the catalogs offer, with the free
/// space there, and keeps them. Says so when they changed. The reads (folders, free space) run
/// on a blocking thread, not on the async workers.
pub(crate) async fn refresh(core: &Core) {
    let _turn = core.disk_turn().await;
    let reader = core.clone();
    // A drive that stopped answering holds that thread, not the command that waits here.
    let read =
        tokio::time::timeout(crate::drives::SPACE_READ_LIMIT, tokio::task::spawn_blocking(move || read(&reader)));
    let Ok(Ok(disks)) = read.await else {
        tracing::warn!("the folders' free space wasn't read in time: it stays as it was");
        return;
    };
    let changed = {
        let mut state = core.state();
        let changed = state.disks != disks;
        state.disks = disks;
        changed
    };
    if changed {
        core.changed(Area::Hardware);
    }
}

/// [`refresh`], from code that can't wait for it: it ends soon after.
pub(crate) fn refresh_soon(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move { refresh(&core).await });
}

fn read(core: &Core) -> Disks {
    let drives = core.ports().drives.as_ref();
    let library = core.library().snapshot();
    let mut folders = BTreeMap::new();
    for offer in library.offers.iter() {
        let console = &offer.releases[0].platform;
        let at = (offer.emulator.clone(), console.clone());
        if folders.contains_key(&at) {
            continue;
        }
        let kernel = core.kernel();
        let Some(host) = kernel.host_folders(&offer.emulator) else { continue };
        let console_name = crate::release_downloads::console(kernel, &offer.emulator, console);
        let Ok((_, path)) = host.place(Purpose::Download, &console_name, None) else { continue };
        let room = Room::read(drives, &path, None);
        folders.insert(at, DownloadFolder { path, room });
    }
    let mut downloads = BTreeMap::new();
    // Each emulator's game folder, found once.
    let mut games: BTreeMap<(PluginId, String), Option<PathBuf>> = BTreeMap::new();
    for job in core.torrents().snapshot().jobs.values() {
        let kernel = core.kernel();
        let console = crate::release_downloads::console_of_job(kernel, job);
        let game_folder = games.entry((job.emulator.clone(), console.clone())).or_insert_with(|| {
            let place = kernel.host_folders(&job.emulator).and_then(|h| h.place(Purpose::Install, &console, None).ok());
            place.map(|(_, path)| path)
        });
        downloads.insert(job.id.clone(), Room::read(drives, &job.folder, game_folder.as_deref()));
    }
    // Each host folder's folders, as the first plugin that asks for each sees them.
    let kernel = core.kernel();
    let host = kernel.host();
    let mut roots_free = BTreeMap::new();
    let mut named = std::collections::BTreeSet::new();
    for manifest in kernel.order().iter().filter_map(|id| kernel.manifest(id).ok()) {
        for grant in &manifest.permissions.filesystem {
            if !named.insert(grant.folder.clone()) {
                continue;
            }
            for root in crate::folders::roots(kernel.config(), host.home.as_deref(), host.platform, grant) {
                if root.is_dir() {
                    let free = crate::drives::free(drives, &root);
                    roots_free.insert(root, free);
                }
            }
        }
    }
    let backup_free =
        crate::backups::folder(core).ok().filter(|f| f.is_dir()).and_then(|f| crate::drives::free(drives, &f));
    Disks { folders: Arc::new(folders), downloads: Arc::new(downloads), roots_free: Arc::new(roots_free), backup_free }
}

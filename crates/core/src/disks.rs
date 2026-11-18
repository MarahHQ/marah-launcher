//! The disks the catalogs' downloads go to: each console's download folder and the free space
//! on the disk that holds it. A Game Hub says when a release doesn't fit. Also, for each
//! download's sheet, the free space where its files go and where its game installs.
//!
//! The view reads these on every render, so they are read ahead: [`refresh`] runs when a Game
//! Hub opens, the catalogs are read, a folder changes and a download starts or ends, and keeps
//! the answer in the core's state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use launcher_api::PluginId;

use crate::Core;
use crate::changes::Area;
use crate::folders::Purpose;

/// Where a console's downloads go, and the free space there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadFolder {
    /// The console's folder in the emulator's packages folder. It may not be there yet.
    pub path: PathBuf,
    /// The free bytes on the disk that holds it; `None` when the system doesn't say.
    pub free: Option<u64>,
}

/// Each console's download folder, by the emulator and the console a catalog's offers name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Disks {
    folders: Arc<BTreeMap<(PluginId, String), DownloadFolder>>,
    /// The free bytes where each download's files go, by its id.
    downloads: Arc<BTreeMap<String, u64>>,
    /// The free bytes in each emulator's game folder, the one the install sheet offers.
    games: Arc<BTreeMap<PluginId, u64>>,
    /// The downloads whose folder is on the same drive as their emulator's game folder.
    one_drive: Arc<BTreeSet<String>>,
}

impl Disks {
    /// Where `emulator`'s downloads of `console`'s games go; `None` when it has no packages
    /// folder, or the catalogs offered none of its games when the disks were last read.
    pub fn download_folder(&self, emulator: &PluginId, console: &str) -> Option<&DownloadFolder> {
        self.folders.get(&(emulator.clone(), console.to_owned()))
    }

    /// The free bytes on the disk of download `id`'s folder; `None` when the system doesn't
    /// say.
    pub fn download_free(&self, id: &str) -> Option<u64> {
        self.downloads.get(id).copied()
    }

    /// The free bytes on the disk of `emulator`'s game folder; `None` when it has none or the
    /// system doesn't say.
    pub fn games_free(&self, emulator: &PluginId) -> Option<u64> {
        self.games.get(emulator).copied()
    }

    /// Download `id`'s folder and its emulator's game folder are on one drive: the download
    /// and the install share its free space.
    pub fn one_drive(&self, id: &str) -> bool {
        self.one_drive.contains(id)
    }
}

/// Reads the download folder of every emulator and console the catalogs offer, with the free
/// space there, and keeps them. Says so when they changed.
pub(crate) fn refresh(core: &Core) {
    let disks = read(core);
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

fn read(core: &Core) -> Disks {
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
        // The console's folder may not be there yet: the disk that holds it is the one the
        // nearest folder above is on.
        let free = path.ancestors().find(|p| p.is_dir()).and_then(|there| core.ports().hardware.free_bytes(there));
        folders.insert(at, DownloadFolder { path, free });
    }
    let hardware = &core.ports().hardware;
    let free_at = |path: &std::path::Path| crate::release_downloads::free_at(hardware.as_ref(), path);
    let disk_at = |path: &std::path::Path| crate::release_downloads::disk_at(hardware.as_ref(), path);
    let mut downloads = BTreeMap::new();
    let mut games = BTreeMap::new();
    // Each emulator's game folder, read once.
    let mut places: BTreeMap<PluginId, Option<PathBuf>> = BTreeMap::new();
    let mut one_drive = BTreeSet::new();
    for job in core.torrents().snapshot().jobs.values() {
        if let Some(free) = free_at(&job.folder) {
            downloads.insert(job.id.clone(), free);
        }
        let place = places.entry(job.emulator.clone()).or_insert_with(|| {
            let kernel = core.kernel();
            let console = crate::release_downloads::console_of(kernel, &job.emulator);
            let place = kernel.host_folders(&job.emulator).and_then(|h| h.place(Purpose::Install, &console, None).ok());
            place.map(|(_, path)| path)
        });
        let Some(place) = place.as_deref() else { continue };
        if !games.contains_key(&job.emulator)
            && let Some(free) = free_at(place)
        {
            games.insert(job.emulator.clone(), free);
        }
        if disk_at(place).is_some_and(|d| disk_at(&job.folder) == Some(d)) {
            one_drive.insert(job.id.clone());
        }
    }
    Disks {
        folders: Arc::new(folders),
        downloads: Arc::new(downloads),
        games: Arc::new(games),
        one_drive: Arc::new(one_drive),
    }
}

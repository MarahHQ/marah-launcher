//! The disks the catalogs' downloads go to: each console's download folder and the room on the
//! disk that holds it. A Game Hub says when a release doesn't fit. Also, for each download's
//! sheet, the room where its files go and where its game installs.
//!
//! The view reads these on every render, so they are read ahead: [`refresh`] runs when a Game
//! Hub opens, the catalogs are read, a folder changes and a download starts or ends, and keeps
//! the answer in the core's state. Each answer is a [`Room`]: the view asks it the same question
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
    let mut games: BTreeMap<PluginId, Option<PathBuf>> = BTreeMap::new();
    for job in core.torrents().snapshot().jobs.values() {
        let game_folder = games.entry(job.emulator.clone()).or_insert_with(|| {
            let kernel = core.kernel();
            let console = crate::release_downloads::console_of(kernel, &job.emulator);
            let place = kernel.host_folders(&job.emulator).and_then(|h| h.place(Purpose::Install, &console, None).ok());
            place.map(|(_, path)| path)
        });
        downloads.insert(job.id.clone(), Room::read(drives, &job.folder, game_folder.as_deref()));
    }
    Disks { folders: Arc::new(folders), downloads: Arc::new(downloads) }
}

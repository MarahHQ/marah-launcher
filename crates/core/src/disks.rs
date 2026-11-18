//! The disks the catalogs' downloads go to: each console's download folder and the free space
//! on the disk that holds it. A Game Hub says when a release doesn't fit. Also, for each
//! download's sheet, the free space where its files go and where its game installs.
//!
//! The view reads these on every render, so they are read ahead: [`refresh`] runs when a Game
//! Hub opens, the catalogs are read, a folder changes and a download starts or ends, and keeps
//! the answer in the core's state.
//!
//! The rule that says whether a download and its install fit lives here too
//! ([`download_short`], [`install_short`] and [`Room::install_after_short`]). The view asks it
//! with the numbers read ahead. The commands ask it with numbers read at the time
//! ([`free_at`], [`Room::at`]), for the folder the player picked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use launcher_api::PluginId;

use crate::Core;
use crate::changes::Area;
use crate::folders::Purpose;
use crate::torrents::gigabytes;

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

    /// The room for download `id` and for its install into `emulator`'s game folder, as read
    /// ahead.
    pub fn room(&self, id: &str, emulator: &PluginId) -> Room {
        Room {
            download: self.downloads.get(id).copied(),
            install: self.games.get(emulator).copied(),
            one_drive: self.one_drive.contains(id),
        }
    }
}

/// The free space for a download and for the install of its game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Room {
    /// The free bytes where the download's files go; `None` when the system doesn't say.
    pub download: Option<u64>,
    /// The free bytes where its game installs; `None` when there is no game folder or the
    /// system doesn't say.
    pub install: Option<u64>,
    /// The two folders are on one drive: the download and the install share its free space.
    pub one_drive: bool,
}

impl Room {
    /// The room for a download into `download` and its install into `install`, read now. Both
    /// folders may not be there yet.
    pub fn at(hardware: &dyn crate::Hardware, download: &Path, install: &Path) -> Self {
        Room {
            download: free_at(hardware, download),
            install: free_at(hardware, install),
            one_drive: disk_at(hardware, install).is_some_and(|d| disk_at(hardware, download) == Some(d)),
        }
    }

    /// Why the install after the download, of `checked` bytes, doesn't fit; `None` when it
    /// fits or the system doesn't say. On one drive, the download and the install need room
    /// there together: twice the checked bytes.
    pub fn install_after_short(&self, checked: u64) -> Option<String> {
        let free = self.install?;
        if self.one_drive {
            let needed = checked.saturating_mul(2);
            (free < needed).then(|| {
                format!(
                    "Not enough free space on the drive for the download and the install: {} needed, {} free",
                    gigabytes(needed),
                    gigabytes(free)
                )
            })
        } else {
            install_short(checked, Some(free))
        }
    }
}

/// Why a download of `needed` bytes doesn't fit in the `free` bytes of its folder; `None` when
/// it fits or the system doesn't say. The Game Hub says the same before the download starts.
pub(crate) fn download_short(needed: u64, free: Option<u64>) -> Option<String> {
    short(needed, free?, "download folder")
}

/// Why the install of `needed` downloaded bytes doesn't fit in the `free` bytes of the game
/// folder; `None` when it fits or the system doesn't say.
pub(crate) fn install_short(needed: u64, free: Option<u64>) -> Option<String> {
    short(needed, free?, "game folder")
}

fn short(needed: u64, free: u64, place: &str) -> Option<String> {
    (free < needed).then(|| {
        format!("Not enough free space in the {place}: {} needed, {} free", gigabytes(needed), gigabytes(free))
    })
}

/// The free bytes on the disk that holds `folder`, which may not be there yet: the disk of the
/// nearest folder above it.
pub(crate) fn free_at(hardware: &dyn crate::Hardware, folder: &Path) -> Option<u64> {
    let there = folder.ancestors().find(|p| p.is_dir())?;
    hardware.free_bytes(there)
}

/// Which disk holds `folder`, which may not be there yet: the disk of the nearest folder above
/// it. `None` when the system doesn't say.
fn disk_at(hardware: &dyn crate::Hardware, folder: &Path) -> Option<u64> {
    let there = folder.ancestors().find(|p| p.is_dir())?;
    hardware.disk_id(there)
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
        let free = free_at(core.ports().hardware.as_ref(), &path);
        folders.insert(at, DownloadFolder { path, free });
    }
    let hardware = &core.ports().hardware;
    let free_at = |path: &Path| free_at(hardware.as_ref(), path);
    let disk_at = |path: &Path| disk_at(hardware.as_ref(), path);
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

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    #[test]
    fn a_download_fits_in_the_free_space_of_its_folder() {
        assert_eq!(download_short(3 * GB, Some(5 * GB)), None);
        assert_eq!(download_short(5 * GB, Some(5 * GB)), None, "just enough fits");
        assert_eq!(download_short(5 * GB, None), None, "the system doesn't say: no warning");
        assert_eq!(
            download_short(5 * GB, Some(3 * GB)).as_deref(),
            Some("Not enough free space in the download folder: 5.0 GB needed, 3.0 GB free")
        );
    }

    #[test]
    fn an_install_from_downloaded_files_fits_in_the_free_space_of_the_game_folder() {
        assert_eq!(install_short(3 * GB, Some(5 * GB)), None);
        assert_eq!(install_short(5 * GB, Some(5 * GB)), None, "just enough fits");
        assert_eq!(install_short(5 * GB, None), None, "the system doesn't say: no warning");
        assert_eq!(
            install_short(5 * GB, Some(3 * GB)).as_deref(),
            Some("Not enough free space in the game folder: 5.0 GB needed, 3.0 GB free")
        );
    }

    #[test]
    fn on_two_drives_the_install_after_the_download_needs_room_in_the_game_folder_only() {
        let room = |install| Room { download: Some(GB), install, one_drive: false };
        assert_eq!(room(Some(5 * GB)).install_after_short(3 * GB), None, "the download's drive doesn't count");
        assert_eq!(room(Some(5 * GB)).install_after_short(5 * GB), None, "just enough fits");
        assert_eq!(room(None).install_after_short(5 * GB), None, "the system doesn't say: no warning");
        assert_eq!(
            room(Some(5 * GB)).install_after_short(6 * GB).as_deref(),
            Some("Not enough free space in the game folder: 6.0 GB needed, 5.0 GB free")
        );
    }

    #[test]
    fn on_one_drive_the_download_and_the_install_need_room_together() {
        let room = |free| Room { download: free, install: free, one_drive: true };
        assert_eq!(room(Some(5 * GB)).install_after_short(2 * GB), None);
        assert_eq!(room(Some(6 * GB)).install_after_short(3 * GB), None, "just enough for both fits");
        assert_eq!(room(None).install_after_short(3 * GB), None, "the system doesn't say: no warning");
        assert_eq!(
            room(Some(5 * GB)).install_after_short(3 * GB).as_deref(),
            Some("Not enough free space on the drive for the download and the install: 6.0 GB needed, 5.0 GB free"),
            "twice the checked bytes, though each alone fits"
        );
    }
}

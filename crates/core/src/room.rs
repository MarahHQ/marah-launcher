//! Room: whether a download or an install fits on its drive, and the words the player sees when
//! it doesn't.
//!
//! This module owns the free-space rule. A [`Room`] is one reading of the drives where a
//! download's files go and where its game installs. The commands read the drives when the
//! player clicks; [`crate::disks`] reads them ahead and keeps the reading for the view, which
//! asks the same question before the click. One rule and one text serve both, so the warning
//! and the refusal can't say different things for the same reading.
//!
//! The callers still choose the bytes: the catalog's size before a download exists, the checked
//! files on the download sheet, what is left of a download when it starts.

use std::path::Path;

use crate::DriveSpace;
use crate::drives::{free, same_drive};

/// `1.5 GB`, for the free space.
pub(crate) fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// What needs room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Need {
    /// A download's files, in the download folder.
    Download,
    /// The install of a download that is already on the disk, in the game folder.
    Install,
    /// "Download and install", checked before anything downloads: the install needs room in the
    /// game folder. On one drive the download and the install need room there together. On two
    /// drives the download's own room is checked when it starts.
    DownloadAndInstall,
}

/// The free space where a download's files go and where its game installs. A drive the system
/// says nothing about has room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Room {
    download: Option<u64>,
    games: Option<u64>,
    one_drive: bool,
}

impl Room {
    /// Reads the drives of `download` and of `games`, the game folder when there is one. Either
    /// may not be there yet.
    pub fn read(drives: &dyn DriveSpace, download: &Path, games: Option<&Path>) -> Room {
        Room {
            download: free(drives, download),
            games: games.and_then(|games| free(drives, games)),
            one_drive: games.is_some_and(|games| same_drive(drives, games, download)),
        }
    }

    /// The free bytes where the download's files go.
    pub fn download_free(&self) -> Option<u64> {
        self.download
    }

    /// The free bytes in the game folder.
    pub fn games_free(&self) -> Option<u64> {
        self.games
    }

    /// Why `bytes` don't fit for `need`; `None` when they fit.
    pub fn short(&self, need: Need, bytes: u64) -> Option<String> {
        match need {
            Need::Download => short_in(bytes, self.download?, "download folder"),
            Need::Install => short_in(bytes, self.games?, "game folder"),
            Need::DownloadAndInstall if self.one_drive => {
                let (needed, free) = (bytes.saturating_mul(2), self.games?);
                (free < needed).then(|| {
                    format!(
                        "Not enough free space on the drive for the download and the install: {} needed, {} free",
                        gigabytes(needed),
                        gigabytes(free)
                    )
                })
            }
            Need::DownloadAndInstall => short_in(bytes, self.games?, "game folder"),
        }
    }
}

/// Why `needed` bytes don't fit in `free` in the `place`; `None` when they fit.
fn short_in(needed: u64, free: u64, place: &str) -> Option<String> {
    (free < needed).then(|| {
        format!("Not enough free space in the {place}: {} needed, {} free", gigabytes(needed), gigabytes(free))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Drives by the folder they hold, each with its free bytes; a drive's id is its place in the
    /// list.
    struct Drives(Vec<(PathBuf, u64)>);

    impl DriveSpace for Drives {
        fn free_bytes(&self, path: &Path) -> Option<u64> {
            self.0.iter().find(|(disk, _)| path.starts_with(disk)).map(|(_, free)| *free)
        }

        fn drive_id(&self, path: &Path) -> Option<u64> {
            self.0.iter().position(|(disk, _)| path.starts_with(disk)).map(|at| at as u64)
        }
    }

    const GB: u64 = 1_000_000_000;

    fn folders() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let (downloads, games) = (home.path().join("Downloads"), home.path().join("Games"));
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&games).unwrap();
        (home, downloads, games)
    }

    #[test]
    fn on_two_drives_the_install_needs_room_for_the_checked_bytes_once() {
        let (_home, downloads, games) = folders();
        let drives = Drives(vec![(downloads.clone(), 10 * GB), (games.clone(), 3 * GB)]);
        let room = Room::read(&drives, &downloads, Some(&games));
        assert_eq!(room.short(Need::DownloadAndInstall, 3 * GB), None);
        assert_eq!(
            room.short(Need::DownloadAndInstall, 4 * GB).as_deref(),
            Some("Not enough free space in the game folder: 4.0 GB needed, 3.0 GB free")
        );
        assert_eq!(room.short(Need::Download, 4 * GB), None, "the download's drive has room");
    }

    #[test]
    fn on_one_drive_the_download_and_the_install_need_room_together() {
        let (home, downloads, games) = folders();
        let drives = Drives(vec![(home.path().to_owned(), 5 * GB)]);
        let room = Room::read(&drives, &downloads, Some(&games));
        assert_eq!(room.short(Need::DownloadAndInstall, 2 * GB), None, "4 GB of 5");
        assert_eq!(
            room.short(Need::DownloadAndInstall, 3 * GB).as_deref(),
            Some("Not enough free space on the drive for the download and the install: 6.0 GB needed, 5.0 GB free")
        );
        assert_eq!(room.short(Need::Download, 3 * GB), None, "the download alone fits");
        assert_eq!(room.short(Need::Install, 3 * GB), None, "a finished download already takes its room");
    }

    #[test]
    fn a_folder_that_is_not_there_yet_is_on_the_drive_of_the_folder_above() {
        let (home, downloads, _) = folders();
        let drives = Drives(vec![(downloads.clone(), GB), (home.path().to_owned(), 8 * GB)]);
        let not_yet = downloads.join("PS5/torrent-1");
        assert_eq!(free(&drives, &not_yet), Some(GB));
        let room = Room::read(&drives, &not_yet, Some(&home.path().join("Games/PS5")));
        assert_eq!(room.download_free(), Some(GB));
        assert_eq!(room.games_free(), Some(8 * GB));
        assert_eq!(
            room.short(Need::Download, 2 * GB).as_deref(),
            Some("Not enough free space in the download folder: 2.0 GB needed, 1.0 GB free")
        );
    }

    #[test]
    fn a_drive_the_system_says_nothing_about_has_room() {
        let (_home, downloads, games) = folders();
        let room = Room::read(&crate::NoDriveSpace, &downloads, Some(&games));
        assert_eq!(room, Room::default());
        for need in [Need::Download, Need::Install, Need::DownloadAndInstall] {
            assert_eq!(room.short(need, u64::MAX), None, "{need:?}");
        }
    }

    #[test]
    fn drives_without_an_id_are_not_one_drive() {
        struct NoIds;
        impl DriveSpace for NoIds {
            fn free_bytes(&self, _: &Path) -> Option<u64> {
                Some(5 * GB)
            }
        }
        let (_home, downloads, games) = folders();
        let room = Room::read(&NoIds, &downloads, Some(&games));
        assert_eq!(room.short(Need::DownloadAndInstall, 3 * GB), None, "3 GB once, not twice");
    }
}

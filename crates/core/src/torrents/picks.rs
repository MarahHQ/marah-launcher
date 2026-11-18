//! Which files of a download the player picks on the download sheet. Pure: it reads paths and
//! returns values.
//!
//! The rule for what a file is, by its path in lower case:
//!
//! - **Update**: a word in it is `update`, `patch` or `upd`.
//! - **DLC**: a word in it is `dlc`, `addon`, `add-on` or `addons`.
//! - **Extra**: its extension is one of [`EXTRA_EXTENSIONS`] (text, pictures, music, checksums),
//!   or a word in it is one of [`EXTRA_WORDS`] (`soundtrack`, `manual` and the like).
//! - **Game**: anything else. The game itself is always downloaded: the player can't uncheck it.
//!
//! When no file is the game (a torrent of updates only), every file is optional. The parts of
//! one multi-part archive go on and off together: half an archive can't unpack.

use crate::TorrentFile;

/// What a file of a download is, for the download sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileKind {
    /// The game itself: always downloaded.
    Game,
    Update,
    Dlc,
    /// Text, pictures, music and other files the game doesn't need.
    Extra,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Update => "update",
            Self::Dlc => "dlc",
            Self::Extra => "extra",
        }
    }
}

/// The extensions of files a game doesn't need.
const EXTRA_EXTENSIONS: [&str; 18] = [
    "txt", "nfo", "diz", "sfv", "md5", "sha1", "sha256", "url", "htm", "html", "jpg", "jpeg", "png", "gif", "bmp",
    "pdf", "mp3", "flac",
];

/// The words that name files a game doesn't need.
const EXTRA_WORDS: [&str; 9] =
    ["soundtrack", "ost", "artbook", "manual", "extras", "bonus", "readme", "scans", "wallpapers"];

/// What the file at `path` is.
pub(crate) fn kind_of(path: &str) -> FileKind {
    let lower = path.to_lowercase().replace("add-on", "addon");
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    if has(&["update", "patch", "upd"]) {
        return FileKind::Update;
    }
    if has(&["dlc", "addon", "addons"]) {
        return FileKind::Dlc;
    }
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
    if EXTRA_EXTENSIONS.contains(&extension) || has(&EXTRA_WORDS) {
        return FileKind::Extra;
    }
    FileKind::Game
}

/// Each file of a download, as the sheet shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pick {
    pub kind: FileKind,
    /// It is downloaded.
    pub on: bool,
    /// The player can't uncheck it: the game itself.
    pub locked: bool,
}

/// The files the sheet shows (`files`, the first of the torrent's), with `skip` unchecked.
pub(crate) fn picks(files: &[TorrentFile], skip: &[usize]) -> Vec<Pick> {
    let kinds: Vec<FileKind> = files.iter().map(|f| kind_of(&f.path)).collect();
    let any_game = kinds.contains(&FileKind::Game);
    kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| {
            let locked = any_game && kind == FileKind::Game;
            Pick { kind, on: locked || !skip.contains(&i), locked }
        })
        .collect()
}

/// The files left out after the player checks (`on`) or unchecks file `file`, or every file
/// for `None`. A locked file stays on, and the parts of an archive follow each other.
pub(crate) fn pick(files: &[TorrentFile], skip: &[usize], file: Option<usize>, on: bool) -> Vec<usize> {
    let shown = picks(files, skip);
    let mut chosen: Vec<usize> = match file {
        None => (0..files.len()).collect(),
        Some(i) if i < files.len() => together(files, i),
        Some(_) => Vec::new(),
    };
    chosen.retain(|i| !shown[*i].locked);
    let mut skip: Vec<usize> = skip.iter().copied().filter(|i| *i < files.len()).collect();
    if on {
        skip.retain(|i| !chosen.contains(i));
    } else {
        skip.extend(chosen);
    }
    skip.sort_unstable();
    skip.dedup();
    skip
}

/// File `i` and the other parts of its archive, if it is one.
fn together(files: &[TorrentFile], i: usize) -> Vec<usize> {
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let set =
        crate::archives::sets_of(paths.iter().copied()).into_iter().find(|s| s.parts.iter().any(|p| p == paths[i]));
    match set {
        Some(set) => {
            paths.iter().enumerate().filter(|(_, p)| set.parts.iter().any(|q| q == *p)).map(|(n, _)| n).collect()
        }
        None => vec![i],
    }
}

/// The files to download, by their place in the torrent of `count` files: `None` for all of
/// them. A file past the ones the sheet shows is always downloaded.
pub(crate) fn only(count: usize, skip: &[usize]) -> Option<Vec<usize>> {
    (!skip.is_empty()).then(|| (0..count).filter(|i| !skip.contains(i)).collect())
}

/// The bytes the checked files hold: `total` of the whole torrent less the skipped ones.
pub(crate) fn checked_bytes(files: &[TorrentFile], total: u64, skip: &[usize]) -> u64 {
    let skipped: u64 = skip.iter().filter_map(|i| files.get(*i)).map(|f| f.size).sum();
    total.saturating_sub(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> TorrentFile {
        TorrentFile { path: path.into(), size }
    }

    #[test]
    fn a_file_s_name_says_what_it_is() {
        let kinds: Vec<FileKind> = [
            "Astro/Astro.pkg",
            "Astro/Update/Astro-v1.05.pkg",
            "Astro/astro_patch_1.02.pkg",
            "Astro/UPD-1.10.pkg",
            "Astro/DLC/Costume.pkg",
            "Astro/astro-addon.pkg",
            "Astro/readme.txt",
            "Astro/cover.JPG",
            "Astro/Soundtrack/01.flac",
            "Astro/game.part1.rar",
            "Astro/Updater.exe",
        ]
        .into_iter()
        .map(kind_of)
        .collect();
        use FileKind::*;
        assert_eq!(kinds, [Game, Update, Update, Update, Dlc, Dlc, Extra, Extra, Extra, Game, Game]);
    }

    #[test]
    fn the_game_is_always_on_and_the_rest_can_go() {
        let files = [file("A/game.pkg", 10), file("A/Update/u.pkg", 5), file("A/nfo.txt", 1)];
        let skip = pick(&files, &[], None, false);
        assert_eq!(skip, [1, 2], "none leaves the game on");
        let shown = picks(&files, &skip);
        assert_eq!(
            shown.iter().map(|p| (p.on, p.locked)).collect::<Vec<_>>(),
            [(true, true), (false, false), (false, false)]
        );
        assert_eq!(pick(&files, &skip, Some(0), false), [1, 2], "the game can't be unchecked");
        assert_eq!(pick(&files, &skip, Some(1), true), [2]);
        assert_eq!(pick(&files, &[2], None, true), Vec::<usize>::new(), "all");
        assert_eq!(checked_bytes(&files, 16, &[1, 2]), 10);
        assert_eq!(only(3, &[1, 2]), Some(vec![0]));
        assert_eq!(only(3, &[]), None, "every file");
    }

    #[test]
    fn with_no_game_file_every_file_is_optional() {
        let files = [file("Update/u1.pkg", 5), file("Update/u2.pkg", 5)];
        assert!(picks(&files, &[]).iter().all(|p| !p.locked));
        assert_eq!(pick(&files, &[], Some(0), false), [0]);
    }

    #[test]
    fn the_parts_of_an_archive_go_on_and_off_together() {
        let files = [file("game.7z", 9), file("extras.part1.rar", 4), file("extras.part2.rar", 4), file("a.nfo", 1)];
        assert_eq!(pick(&files, &[], Some(2), false), [1, 2]);
        assert_eq!(pick(&files, &[1, 2, 3], Some(1), true), [3]);
    }
}

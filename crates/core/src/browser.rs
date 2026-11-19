//! The folder browser: a sheet over everything that goes through the folders of this computer and
//! its drives. It has a purpose:
//!
//! - [`Purpose::Restore`]: "Restore from another folder". It lists folders and backups, and a
//!   backup restores ([`crate::backups`]).
//! - [`Purpose::Pick`]: pick a folder for a named host folder, such as the game folders. It lists
//!   folders only, and "Use this folder" adds the folder shown to the named folder's list.
//!
//! Its places are Home and its usual folders, each drive (a drive that isn't mounted mounts from
//! here, then the browser opens it), and for a restore the backup folder and the folders restores
//! came from. Its changes are [`Area::Browser`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::backup::BackupFile;
use crate::changes::{Area, Notifier};
use crate::{Core, CoreError, CoreResult};

/// Why the browser's commands do nothing.
const CLOSED: &str = "the folder browser isn't open";

/// What the browser is open for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Purpose {
    /// Restore a backup from a folder other than the backup folder.
    Restore,
    /// Pick a folder for the named host folder (`games`, `packages`): it is added to the folders
    /// that name means.
    Pick(String),
}

/// The folder browser's commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowseCmd {
    /// Opens the browser for `purpose` at `at`. With no folder, a restore opens where the last
    /// restore came from, else at the backup folder; a pick opens at the named folder's first
    /// folder that is there, else at Home.
    Open {
        #[serde(default)]
        at: Option<PathBuf>,
        purpose: Purpose,
    },
    /// Moves the open browser to this folder.
    Go(PathBuf),
    Close,
    /// Lists the hidden folders, or not.
    ShowHidden(bool),
    /// Shows only the entries whose name holds this text, in any case; all for empty text.
    Filter(String),
    /// Mounts the drive with this id, then opens it.
    Mount(String),
    /// A pick: adds the folder shown to the named folder, and closes the browser.
    Use,
}

/// What a place the browser goes to is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Downloads,
    Desktop,
    Documents,
    /// The backup folder.
    Backups,
    /// A drive, such as a USB stick, mounted or not.
    Drive,
    /// A folder a restore came from.
    Recent,
}

impl PlaceKind {
    /// Its name for the theme: `home`, `downloads`, … `recent`.
    pub fn as_str(self) -> &'static str {
        match self {
            PlaceKind::Home => "home",
            PlaceKind::Downloads => "downloads",
            PlaceKind::Desktop => "desktop",
            PlaceKind::Documents => "documents",
            PlaceKind::Backups => "backups",
            PlaceKind::Drive => "drive",
            PlaceKind::Recent => "recent",
        }
    }
}

/// One place the browser goes to: Home, the backup folder, a drive, a recent folder.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowsePlace {
    pub name: String,
    /// Where it is; empty for a drive that isn't mounted.
    pub path: PathBuf,
    pub kind: PlaceKind,
    /// A drive's size, such as "32.0 GB"; a recent folder's path. Empty for the others.
    pub detail: String,
    /// A drive's id, which mounts it ([`BrowseCmd::Mount`]); `None` for the other places.
    pub drive: Option<String>,
}

/// A folder or a backup in the browser's folder.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowseEntry {
    pub path: PathBuf,
    /// A backup's details; `None` for a folder.
    pub backup: Option<BackupFile>,
    /// A folder's: what is in it ("2 backups", "empty", "can't be read"). Empty for a backup,
    /// and for a folder with other things but no backups.
    pub detail: String,
}

/// The folder the browser shows, and where it can go.
#[derive(Clone, Debug, PartialEq)]
pub struct Browser {
    pub purpose: Purpose,
    pub at: PathBuf,
    pub places: Vec<BrowsePlace>,
    /// Its folders (for a restore, those with backups first), then for a restore its backups
    /// newest first. Hidden folders only with `hidden`.
    pub entries: Vec<BrowseEntry>,
    /// Hidden folders are listed.
    pub hidden: bool,
    /// The text the player typed: the view shows only the entries whose name holds it, in any
    /// case. Empty for all.
    pub filter: String,
}

impl Browser {
    /// The entries the filter lets through: by the name, or by the date a backup's row shows.
    pub fn shown(&self) -> impl Iterator<Item = &BrowseEntry> {
        let filter = self.filter.to_lowercase();
        self.entries.iter().filter(move |e| {
            let holds = |text: &str| text.to_lowercase().contains(&filter);
            filter.is_empty()
                || e.path.file_name().is_some_and(|n| holds(&n.to_string_lossy()))
                || e.backup.as_ref().is_some_and(|b| holds(&crate::display::date_time_text(b.made)))
        })
    }
}

/// The browser, while it is open.
pub(crate) struct Browsing {
    state: Mutex<Option<Arc<Browser>>>,
    notifier: Notifier,
}

impl Browsing {
    pub(crate) fn new(notifier: Notifier) -> Self {
        Self { state: Mutex::new(None), notifier }
    }

    fn state(&self) -> MutexGuard<'_, Option<Arc<Browser>>> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The browser, while it is open.
    pub(crate) fn snapshot(&self) -> Option<Arc<Browser>> {
        self.state().clone()
    }

    fn set(&self, browser: Option<Browser>) {
        *self.state() = browser.map(Arc::new);
        self.notifier.changed(Area::Browser);
    }

    pub(crate) fn close(&self) {
        self.set(None);
    }

    /// Opens the browser at `at` for `purpose`, with `places` to go to. Hidden folders stay listed
    /// or not as they were; the filter is cleared.
    async fn open(&self, purpose: Purpose, at: PathBuf, places: Vec<BrowsePlace>) -> CoreResult<()> {
        let hidden = self.snapshot().is_some_and(|b| b.hidden);
        let (dir, restore) = (at.clone(), purpose == Purpose::Restore);
        let entries = blocking(move || list(&dir, hidden, restore)).await?;
        self.set(Some(Browser { purpose, at, places, entries, hidden, filter: String::new() }));
        Ok(())
    }

    async fn show_hidden(&self, hidden: bool) -> CoreResult<()> {
        let open = self.snapshot().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?;
        let (dir, restore) = (open.at.clone(), open.purpose == Purpose::Restore);
        let entries = blocking(move || list(&dir, hidden, restore)).await?;
        let mut state = self.state();
        // Moved or closed in the meantime: that newer state stays.
        if state.as_ref().is_some_and(|b| b.at == open.at) {
            let mut browser = (*open).clone();
            (browser.entries, browser.hidden) = (entries, hidden);
            *state = Some(Arc::new(browser));
            drop(state);
            self.notifier.changed(Area::Browser);
        }
        Ok(())
    }

    fn filter(&self, text: String) -> CoreResult<()> {
        let mut browser = (*self.snapshot().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?).clone();
        browser.filter = text;
        self.set(Some(browser));
        Ok(())
    }
}

pub(crate) async fn handle(core: &Core, command: BrowseCmd) -> CoreResult<()> {
    match command {
        BrowseCmd::Open { at, purpose } => open(core, at, purpose).await,
        BrowseCmd::Go(at) => {
            let open = core.browsing().snapshot().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?;
            go(core, at, open.purpose.clone()).await
        }
        BrowseCmd::Close => {
            core.browsing().close();
            Ok(())
        }
        BrowseCmd::ShowHidden(hidden) => core.browsing().show_hidden(hidden).await,
        BrowseCmd::Filter(text) => core.browsing().filter(text),
        BrowseCmd::Mount(drive) => {
            let open = core.browsing().snapshot().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?;
            let at = crate::drives::mount(core, &drive).await?;
            go(core, at, open.purpose.clone()).await
        }
        BrowseCmd::Use => {
            let open = core.browsing().snapshot().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?;
            let Purpose::Pick(name) = &open.purpose else {
                return Err(CoreError::Invalid("the browser is open for a restore, not to pick a folder".into()));
            };
            crate::plugin_settings::add_folder(core, name, &open.at)?;
            core.browsing().close();
            Ok(())
        }
    }
}

/// Opens the browser for `purpose`, at `at` or where that purpose starts.
pub(crate) async fn open(core: &Core, at: Option<PathBuf>, purpose: Purpose) -> CoreResult<()> {
    let start = match (at, &purpose) {
        (Some(at), _) => at,
        (None, Purpose::Restore) => crate::backups::recent(core)
            .into_iter()
            .next()
            .or_else(|| crate::backups::folder(core).ok().filter(|f| f.is_dir()))
            .or_else(|| core.kernel().host().home.clone())
            .ok_or_else(|| CoreError::Invalid("no folder to start from".into()))?,
        // With no home folder, the top of the system's disk: `/`, or `C:\` on Windows.
        (None, Purpose::Pick(name)) => crate::plugin_settings::folder_roots(core, name)
            .into_iter()
            .find(|p| p.is_dir())
            .or_else(|| core.kernel().host().home.clone())
            .or_else(|| std::env::temp_dir().ancestors().last().map(Path::to_path_buf))
            .ok_or_else(|| CoreError::Invalid("no folder to start from".into()))?,
    };
    go(core, start, purpose).await
}

/// Opens the browser at `at` for `purpose`, with the places read again.
async fn go(core: &Core, at: PathBuf, purpose: Purpose) -> CoreResult<()> {
    if !at.is_absolute() || !at.is_dir() {
        return Err(CoreError::Invalid(format!("{} isn't a folder", at.display())));
    }
    let places = places(core, &purpose).await;
    core.browsing().open(purpose, at, places).await
}

/// Where the browser can go: Home, the Downloads, Desktop and Documents folders that are there,
/// for a restore the backup folder, each drive, then for a restore the folders restores came from
/// that no other place is.
async fn places(core: &Core, purpose: &Purpose) -> Vec<BrowsePlace> {
    let mut out = Vec::new();
    let mut add = |name: String, path: PathBuf, kind: PlaceKind| {
        out.push(BrowsePlace { name, path, kind, detail: String::new(), drive: None });
    };
    let home = core.kernel().host().home.clone();
    if let Some(home) = &home {
        add("Home".into(), home.clone(), PlaceKind::Home);
        for (name, kind) in
            [("Downloads", PlaceKind::Downloads), ("Desktop", PlaceKind::Desktop), ("Documents", PlaceKind::Documents)]
        {
            let path = home.join(name);
            if path.is_dir() {
                add(name.into(), path, kind);
            }
        }
    }
    let restore = *purpose == Purpose::Restore;
    if restore
        && let Ok(folder) = crate::backups::folder(core)
        && folder.is_dir()
    {
        add("Backup folder".into(), folder, PlaceKind::Backups);
    }
    let drives = core.drives().list().await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "the drives weren't read: the browser lists none");
        Vec::new()
    });
    for drive in drives {
        out.push(BrowsePlace {
            name: drive.label,
            path: drive.mount.unwrap_or_default(),
            kind: PlaceKind::Drive,
            detail: drive.size.map(drive_size_text).unwrap_or_default(),
            drive: Some(drive.id),
        });
    }
    if restore {
        for path in crate::backups::recent(core) {
            if out.iter().any(|p| p.path == path) {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let shown =
                crate::display::short_home(&path.display().to_string(), home.as_deref().and_then(|h| h.to_str()));
            out.push(BrowsePlace { name, path, kind: PlaceKind::Recent, detail: shown, drive: None });
        }
    }
    out
}

/// A drive's size: `1.6 TB` from 1000 GB up, else as other sizes (`32.0 GB`).
pub(crate) fn drive_size_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 { format!("{:.1} TB", bytes as f64 / 1e12) } else { crate::display::size_text(bytes) }
}

/// Runs `work` off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> std::io::Result<T> + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::Invalid(e.to_string()))?
        .map_err(|e| CoreError::Invalid(e.to_string()))
}

/// The folders of `dir`, each by name, then with `restore` its backups newest first; for a
/// restore the folders that hold backups come first. Hidden folders only with `hidden`; other
/// files and links are left out.
fn list(dir: &Path, hidden: bool, restore: bool) -> std::io::Result<Vec<BrowseEntry>> {
    let mut folders = Vec::new();
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(dir)?.filter_map(Result::ok) {
        let Ok(kind) = entry.file_type() else { continue };
        let name = entry.file_name();
        if kind.is_dir() && (hidden || !name.to_string_lossy().starts_with('.')) {
            let inside = inside(&entry.path(), restore);
            folders.push((entry.path(), inside));
        } else if restore
            && kind.is_file()
            && let Some(backup) = crate::backup::backup_file(&entry.path())
        {
            backups.push((entry.path(), backup));
        }
    }
    folders.sort_by_key(|(p, inside)| {
        (!matches!(inside, Inside::Backups(_)), p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
    });
    backups.sort_by_key(|b| std::cmp::Reverse(b.1.made));
    let mut out: Vec<BrowseEntry> =
        folders.into_iter().map(|(path, inside)| BrowseEntry { path, backup: None, detail: inside.text() }).collect();
    out.extend(backups.into_iter().map(|(path, backup)| BrowseEntry {
        path,
        backup: Some(backup),
        detail: String::new(),
    }));
    Ok(out)
}

/// What is in a folder, as its row says.
enum Inside {
    Backups(usize),
    Empty,
    Other,
    Unreadable,
}

impl Inside {
    fn text(&self) -> String {
        match self {
            Inside::Backups(1) => "1 backup".into(),
            Inside::Backups(n) => format!("{n} backups"),
            Inside::Empty => "empty".into(),
            Inside::Other => String::new(),
            Inside::Unreadable => "can't be read".into(),
        }
    }
}

/// What is in `dir`, by the names of its files: no backup is opened. Backups count only for a
/// restore.
fn inside(dir: &Path, restore: bool) -> Inside {
    let Ok(read) = std::fs::read_dir(dir) else { return Inside::Unreadable };
    let (mut any, mut backups) = (false, 0);
    for entry in read.filter_map(Result::ok) {
        any = true;
        if restore
            && entry.file_type().is_ok_and(|t| t.is_file())
            && entry.file_name().to_str().and_then(crate::backup::archive::kind_of).is_some()
        {
            backups += 1;
        }
    }
    match (backups, any) {
        (0, false) => Inside::Empty,
        (0, true) => Inside::Other,
        (n, _) => Inside::Backups(n),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_drive_of_a_thousand_gb_or_more_says_tb() {
        assert_eq!(super::drive_size_text(1_645_500_000_000), "1.6 TB");
        assert_eq!(super::drive_size_text(32_000_000_000), "32.0 GB");
    }
}

//! The drives' flows over the core: the player's commands, a drive that was plugged in, the game
//! folders' drives at the start, and the games of a drive that is away.
//!
//! - The adapter is the system plugin's when one provides `drives` and can use it, else the
//!   mount folders ([`adopt`], at each read).
//! - The drives are read again on the system's refresh tick. When the launcher is the whole
//!   screen, a drive that came is mounted, and a notice says so; a desktop does that itself.
//! - Unmount and eject wait while a game runs from the drive, or a download or an install writes
//!   there.
//! - A game folder on a drive keeps the drive's id (`app.folder_drives`). At the start, that drive
//!   is mounted before the games are read, for at most [`BOOT_MOUNT_LIMIT`].
//! - While a game folder's drive is away, the library keeps the games it had there.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use launcher_api::SystemJob;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::udisks::PluginDrives;
use super::{Adapter, Drive, MountFolders};
use crate::changes::Area;
use crate::library::Entry;
use crate::system::{JobKind, provider, start_job};
use crate::{Core, CoreError, CoreResult};

/// The longest the start waits for the game folders' drives to mount: a drive that doesn't
/// answer must not hold the boot.
pub(crate) const BOOT_MOUNT_LIMIT: Duration = Duration::from_secs(10);

/// The key under `app` that holds each game folder on a drive: its path, and the drive's id.
const FOLDER_DRIVES: &str = "folder_drives";

/// What the player asks of a drive, by its id (`Drive::id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriveCmd {
    /// Reads the drives again now.
    Refresh,
    Mount(String),
    /// Unmounts the drive. It waits while a game runs from it, or a download or an install
    /// writes to it.
    Unmount(String),
    /// Unmounts the drive's disk and powers it off, so it can be taken out. Only a drive the
    /// player can take out; it waits as an unmount does.
    Eject(String),
}

pub(crate) async fn handle(core: &Core, command: DriveCmd) -> CoreResult<()> {
    match command {
        DriveCmd::Refresh => {}
        DriveCmd::Mount(id) => {
            mount(core, &id).await?;
        }
        DriveCmd::Unmount(id) => {
            let drive = core.drives().find(&id)?;
            refuse_when_busy(core, &[&drive])?;
            let job = start_job(core, JobKind::Unmount, &drive.label).1;
            ended(&job, core.drives().unmount(&id, job.clone())).await?;
        }
        DriveCmd::Eject(id) => {
            let drive = core.drives().find(&id)?;
            if !drive.usb {
                return Err(CoreError::Invalid(format!("{} can't be taken out: unmount it instead", drive.label)));
            }
            let known = core.drives().known();
            let parts: Vec<&Drive> =
                known.iter().filter(|d| d.id == id || (!d.disk.is_empty() && d.disk == drive.disk)).collect();
            refuse_when_busy(core, &parts)?;
            let job = start_job(core, JobKind::Eject, &drive.label).1;
            ended(&job, core.drives().eject(&id, job.clone())).await?;
        }
    }
    // A drive ejected is gone at once, not at the next tick.
    refresh(core).await;
    Ok(())
}

/// Mounts the drive with this id, as a job the player sees, and returns where.
pub(crate) async fn mount(core: &Core, id: &str) -> CoreResult<PathBuf> {
    let drive = core.drives().find(id)?;
    let job = start_job(core, JobKind::Mount, &drive.label).1;
    let at = ended(&job, core.drives().mount(id, job.clone())).await;
    changed(core);
    at
}

/// `work`'s outcome; a failure ends `job` with its reason, if `work` didn't end it.
async fn ended<T>(job: &Arc<dyn SystemJob>, work: impl Future<Output = CoreResult<T>>) -> CoreResult<T> {
    let outcome = work.await;
    if let Err(e) = &outcome {
        job.failed(&e.to_string());
    }
    outcome
}

/// The view hears that the drives changed.
fn changed(core: &Core) {
    core.changed(Area::System);
}

/// Why `drives` can't be unmounted now: a game runs from one, or a download or an install writes
/// to one.
fn refuse_when_busy(core: &Core, drives: &[&Drive]) -> CoreResult<()> {
    let mounts: Vec<&Path> = drives.iter().filter_map(|d| d.mount.as_deref()).collect();
    let on = |path: &Path| mounts.iter().any(|m| path.starts_with(m));
    let kernel = core.kernel();
    let folders = core.library().game_folders();
    for session in core.sessions().snapshot().sessions.values() {
        let folder = folders.get(&session.game).and_then(|f| crate::app::resolve_game_folder(kernel, &session.game, f));
        if folder.is_some_and(|f| on(&f)) {
            return Err(CoreError::Invalid(format!("Close {} first: it runs from this drive", session.title)));
        }
    }
    for job in core.torrents().snapshot().jobs.values() {
        if (job.state.active() || job.seeding || job.verifying) && on(&job.folder) {
            return Err(CoreError::Invalid(format!("Pause the download of {} first: it uses this drive", job.title)));
        }
    }
    for job in core.installs().snapshot().jobs.values() {
        if job.phase != crate::game_installs::GamePhase::Installing {
            continue;
        }
        let Some(host) = kernel.host_folders(&job.plugin) else { continue };
        let paths = std::iter::once(&job.source).chain(job.work.as_ref()).filter_map(|p| host.resolve(p).ok());
        if paths.into_iter().any(|p| on(&p)) {
            return Err(CoreError::Invalid(format!(
                "Wait for the install of {} to finish: it uses this drive",
                job.label
            )));
        }
    }
    Ok(())
}

/// Uses the system plugin's drives when one provides them and can use them here; else the mount
/// folders.
async fn adopt(core: &Core) {
    let mut adapter: Option<Arc<dyn Adapter>> = None;
    if let Some((plugin, settings)) = provider(core, |p| p.drives.clone()).await {
        let why = core.kernel().guard(&plugin, "drives.unavailable", async { Ok(settings.unavailable().await) }).await;
        match why {
            Ok(None) => adapter = Some(Arc::new(PluginDrives { kernel: core.kernel().clone(), plugin, settings })),
            Ok(Some(why)) => tracing::debug!(%why, "the drives come from the mount folders"),
            Err(e) => tracing::debug!(error = %e, "the drives come from the mount folders"),
        }
    }
    let home = core.kernel().host().home.clone();
    core.drives().use_adapter(adapter.unwrap_or_else(|| Arc::new(MountFolders::here(home.as_deref()))));
}

/// Reads the drives again. A drive that came is mounted, with a notice; the view hears of any
/// change.
pub(crate) async fn refresh(core: &Core) {
    adopt(core).await;
    let before = core.drives().known();
    let changes = core.drives().refresh().await;
    if core.drives().known() != before {
        changed(core);
    }
    // A desktop mounts a drive and says so itself: only the whole screen does it here.
    if !core.kernel().host().place.whole_screen {
        return;
    }
    for drive in changes.plugged {
        let core = core.clone();
        tokio::spawn(async move { plugged(&core, drive).await });
    }
}

/// A drive came: it is mounted, and a notice says so.
async fn plugged(core: &Core, drive: Drive) {
    let mounted = match drive.mount {
        Some(_) => Ok(()),
        None => mount(core, &drive.id).await.map(|_| ()),
    };
    match mounted {
        Ok(()) => core.notices().drive_found(&drive.id, format!("Drive {} found", drive.label)),
        Err(e) => core.notices().problem(format!("Drive {} found, but it didn't mount: {e}", drive.label)),
    }
    core.changed(Area::Notices);
}

/// Every folder the player mapped (`app.folders.<name>`), as saved.
fn mapped_folders(core: &Core) -> BTreeSet<String> {
    let folders = core.kernel().config().get(&["app", "folders"]);
    let lists = folders.as_object().into_iter().flat_map(|names| names.values());
    lists.flat_map(|list| list.as_array().into_iter().flatten()).filter_map(Value::as_str).map(str::to_owned).collect()
}

/// Each mapped folder on a drive, with the drive's id, as saved.
fn folder_drives(core: &Core) -> BTreeMap<String, String> {
    let saved = core.kernel().config().get(&["app", FOLDER_DRIVES]);
    let pairs = saved.as_object().into_iter().flatten();
    pairs.filter_map(|(path, id)| Some((path.clone(), id.as_str()?.to_owned()))).collect()
}

/// Keeps the drive of each of `paths` that is on a drive the system mounted for the player, and
/// forgets the folders the player doesn't map any more.
pub(crate) fn remember_folders(core: &Core, paths: &[PathBuf]) {
    let mapped = mapped_folders(core);
    let mut saved = folder_drives(core);
    saved.retain(|path, _| mapped.contains(path));
    let known = core.drives().known();
    for path in paths {
        // A drive the plugin named by its device: the mount folders' drives have no id to keep.
        let on = known.iter().find(|d| !d.device.is_empty() && d.mount.as_deref().is_some_and(|m| path.starts_with(m)));
        if let Some(drive) = on {
            saved.insert(path.display().to_string(), drive.id.clone());
        }
    }
    let value = serde_json::to_value(saved).unwrap_or_default();
    if let Err(e) = core.kernel().config().set(&["app", FOLDER_DRIVES], value) {
        tracing::warn!(error = %e, "the game folders' drives weren't kept: they don't mount at the start");
    }
}

/// At the start, before the games are read: mounts the drive of each mapped folder that isn't
/// there, for at most [`BOOT_MOUNT_LIMIT`].
pub(crate) async fn mount_game_drives(core: &Core) {
    let mapped = mapped_folders(core);
    let wanted: BTreeSet<String> = folder_drives(core)
        .into_iter()
        .filter(|(path, _)| mapped.contains(path) && !Path::new(path).is_dir())
        .map(|(_, id)| id)
        .collect();
    if wanted.is_empty() {
        return;
    }
    let work = async {
        refresh(core).await;
        for id in &wanted {
            match core.drives().find(id) {
                Ok(drive) if drive.mount.is_none() => {
                    if let Err(e) = mount(core, id).await {
                        tracing::warn!(drive = %drive.label, error = %e, "a game folder's drive didn't mount");
                    }
                }
                Ok(_) => {}
                Err(_) => tracing::info!(drive = %id, "a game folder's drive isn't plugged in"),
            }
        }
    };
    if core.ports().clock.timeout(BOOT_MOUNT_LIMIT, work).await.is_none() {
        tracing::warn!("the game folders' drives are slow to mount: the games are read without them");
    }
}

/// The games `listed` lacks that the library had, in a folder whose drive is away: the root it
/// is in isn't there. A plugin lists nothing from a root that isn't there, and a drive taken
/// out, or not mounted yet, must not drop its games.
pub(crate) fn away_games(core: &Core, listed: &[Entry]) -> Vec<Entry> {
    let folders = core.library().game_folders();
    let kernel = core.kernel();
    let root_away = |entry: &Entry| {
        let Some(folder) = folders.get(&entry.key) else { return false };
        let Some(host) = kernel.host_folders(&entry.key.plugin) else { return false };
        let roots = host.roots_of(&folder.folder).map(|(roots, _)| roots).unwrap_or_default();
        roots.get(folder.root as usize).is_some_and(|root| !root.is_dir())
    };
    let library = core.library().snapshot();
    library.library().filter(|e| !listed.iter().any(|l| l.key == e.key) && root_away(e)).cloned().collect()
}

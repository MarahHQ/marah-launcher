//! The core's side of the catalogs' downloads and of game installs: what the torrents and the
//! game installs subsystems can't do alone.
//!
//! [`Command::Torrent`](crate::Command::Torrent) comes here first. A download needs the
//! catalog's release and a folder inside one of the emulator's packages folders: the core
//! resolves them and adds the download to the subsystem. A release its store installs never
//! reaches the subsystem. The rest goes to the subsystem as it is. The subsystem's events come
//! back here: a finished download unpacks and says so, or, after "Download and install",
//! installs by itself.
//!
//! [`Command::GameInstall`](crate::Command::GameInstall) comes here too. An install needs the
//! plugin's [`launcher_api::GameInstaller`], its folders and a game folder: the core resolves them
//! and hands them to the game installs subsystem, which takes the files from the download's
//! folder. Install checks the free space first. The subsystem's events come back here: a new
//! game refreshes the library, and a failure is a notice.

use std::path::Path;

use launcher_api::{GameKey, HostPath, PluginId};

use crate::app::Core;
use crate::changes::Area;
use crate::disks::{Room, download_short, free_at, install_short};
use crate::folders::{HostFolders, Purpose};
use crate::game_installs::{GameInstallCmd, InstallEvent, NewInstall};
use crate::torrents::{InstallAfter, NewDownload, TorrentAction, TorrentCmd, TorrentEvent, TorrentState};
use crate::{CoreError, CoreResult, Kernel};

/// The console an emulator plays, which names its folders: the plugin's id when its manifest
/// doesn't say.
pub(crate) fn console_of(kernel: &Kernel, emulator: &PluginId) -> String {
    kernel
        .manifest(emulator)
        .ok()
        .and_then(|m| m.console.as_ref().map(|c| c.as_str().to_owned()))
        .unwrap_or_else(|| emulator.to_string())
}

/// The player's folders as `emulator` may use them.
pub(crate) fn folders_of(kernel: &Kernel, emulator: &PluginId) -> CoreResult<HostFolders> {
    kernel.host_folders(emulator).ok_or_else(|| CoreError::UnknownPlugin(emulator.clone()))
}

/// The console a release is for, which names its folder: the emulator's when the catalog
/// doesn't say.
pub(crate) fn console(kernel: &Kernel, emulator: &PluginId, platform: &str) -> String {
    if platform.is_empty() { console_of(kernel, emulator) } else { platform.to_owned() }
}

/// Runs the player's command on the catalogs' downloads.
pub(crate) async fn torrent(core: &Core, command: TorrentCmd) -> CoreResult<()> {
    match command {
        TorrentCmd::Download { offer, release, folder } => {
            core.torrents().set_download_error(None);
            let started = download_release(core, &offer, &release, folder.as_deref()).await;
            crate::disks::refresh(core);
            // The sheet that asked says why nothing downloads, and so does a notice, for
            // when the player closed the sheet.
            if let Err(e) = &started {
                core.torrents().set_download_error(Some((offer, e.to_string())));
                core.notices().problem(e.to_string());
            }
            started
        }
        TorrentCmd::Act { id, action } => {
            if let Some(store) = id.strip_prefix("store:") {
                return store_install_action(core, store, action).await;
            }
            if !core.torrents().has(&id) {
                return Err(CoreError::Invalid(format!("no download {id}")));
            }
            match action {
                // Not while the download's install reads its files.
                TorrentAction::DeleteFiles if core.installs().running_from(&id).is_some() => Ok(()),
                action => core.torrents().handle(TorrentCmd::Act { id, action }).await,
            }
        }
        TorrentCmd::Begin { id, install, folder } => {
            if !core.torrents().has(&id) {
                return Err(CoreError::Invalid(format!("no download {id}")));
            }
            core.installs().set_install_error(&id, String::new());
            let begun = begin(core, &id, install, folder).await;
            // The download sheet stays open and says why.
            if let Err(e) = &begun {
                core.installs().set_install_error(&id, e.to_string());
            }
            crate::disks::refresh(core);
            begun
        }
        command @ (TorrentCmd::SeedDownloads(_) | TorrentCmd::Pick { .. }) => core.torrents().handle(command).await,
    }
}

/// The download sheet's Download, or Download and install (`install`, into `folder`). The game
/// folder is checked now, before anything downloads: inside a game folder, and room there for
/// the checked files.
async fn begin(core: &Core, id: &str, install: bool, folder: Option<std::path::PathBuf>) -> CoreResult<()> {
    let job = core.torrents().job(id).ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
    let then = if install {
        let kernel = core.kernel();
        let folders = folders_of(kernel, &job.emulator)?;
        let console = console_of(kernel, &job.emulator);
        let (_, path) = folders.place(Purpose::Install, &console, folder.as_deref())?;
        let checked = crate::torrents::picks::checked_bytes(&job.files, job.total, &job.skip);
        // Fresh numbers for the folder the player typed, not the sheet's read-ahead.
        let room = Room::at(core.ports().hardware.as_ref(), &job.folder, &path);
        if let Some(error) = room.install_after_short(checked) {
            return Err(CoreError::Invalid(error));
        }
        if folder.is_some() {
            folders.remember(Purpose::Install, &console, &path);
        }
        Some(InstallAfter { folder, started: false })
    } else {
        None
    };
    core.torrents().begin(id, then).await
}

/// The start of a run: a download that finished, with "Download and install", before the last
/// run started its install, installs now. Once: [`crate::torrents::TorrentsSubsystem::claim_install_after`]
/// marks it before this returns.
pub(crate) fn resume_installs_after(core: &Core) {
    let jobs = core.torrents().snapshot().jobs.clone();
    for job in jobs.values().filter(|j| j.state == TorrentState::Complete && j.install_after.is_some()) {
        if let Some(after) = core.torrents().claim_install_after(&job.id) {
            start_install_after(core, job.id.clone(), job.title.clone(), after);
        }
    }
}

/// Says the install starts, and starts it in the background.
fn start_install_after(core: &Core, id: String, title: String, after: InstallAfter) {
    core.notices().opening(
        launcher_ui_api::NoticeKind::Success,
        crate::Route::Downloads,
        format!("Download complete: {title}. Installing it now."),
    );
    let core = core.clone();
    tokio::spawn(async move { install_after(&core, &id, &title, after).await });
}

/// "Download and install": the download finished, and installs as Install on its card does.
/// Why it doesn't start shows on its card, and in a notice.
async fn install_after(core: &Core, id: &str, title: &str, after: InstallAfter) {
    let command = GameInstallCmd::Download { id: id.to_owned(), folder: after.folder };
    if let Err(e) = Box::pin(game_install(core, command)).await {
        core.installs().set_install_error(id, e.to_string());
        core.notices().opening(
            launcher_ui_api::NoticeKind::Error,
            crate::Route::Downloads,
            format!("{title} downloaded, but the install didn't start: {e}"),
        );
    }
}

/// What the torrents subsystem tells the rest of the core.
pub(crate) fn torrent_event(core: &Core, event: TorrentEvent) {
    match event {
        TorrentEvent::Finished { id, title } => {
            let job = core.torrents().job(&id);
            crate::disks::refresh(core);
            if let Some(after) = core.torrents().claim_install_after(&id) {
                start_install_after(core, id, title, after);
                return;
            }
            let waiting = job.is_some_and(|j| j.hand_off.unpack.phase == crate::archives::UnpackPhase::Waiting);
            if waiting {
                core.installs().unpack(&id, false);
            }
            core.notices().opening(
                launcher_ui_api::NoticeKind::Success,
                crate::Route::Downloads,
                format!("Download complete: {title}. Install it from Downloads."),
            );
        }
        TorrentEvent::Stopped { title } => {
            crate::disks::refresh(core);
            core.notices().opening(
                launcher_ui_api::NoticeKind::Error,
                crate::Route::Downloads,
                format!("Download stopped: {title}. Open Downloads to see why"),
            )
        }
        TorrentEvent::Problem(message) => core.notices().problem(message),
        TorrentEvent::VpnsRead(vpns) => core.vpn().keep(vpns),
        // Not in the VPN check: the catalogs' fetches would hold up the kill switch.
        TorrentEvent::VpnBack => {
            let core = core.clone();
            tokio::spawn(async move { crate::catalog_reads::refresh_releases(&core).await });
        }
    }
}

/// An action on the store's install `store` (`<plugin>:<release>`): only opening the store's
/// page; the store pauses and cancels its own.
async fn store_install_action(core: &Core, store: &str, action: TorrentAction) -> CoreResult<()> {
    let install = core
        .state()
        .store_installs
        .iter()
        .find(|(plugin, i)| format!("{plugin}:{}", i.release) == store)
        .cloned()
        .ok_or_else(|| CoreError::Invalid(format!("no install {store}")))?;
    let (plugin, install) = install;
    if action != TorrentAction::OpenStore {
        return Err(CoreError::Invalid(format!("{} does that in its own Downloads", install.title)));
    }
    if !core.kernel().manifest(&plugin)?.permissions.allows_link(&install.open) {
        return Err(CoreError::PermissionDenied { plugin, action: format!("open {}", install.open) });
    }
    let link = install.open.clone();
    crate::running::open_link(core, link).await.map_err(CoreError::Invalid)
}

/// A release its store installs: the store's link opens (`steam://install/440`), if the
/// catalog plugin's manifest allows it, and the store takes it from there. Nothing downloads
/// here; the game shows in the Library once the store installed it.
async fn install_in_store(core: &Core, offer: &GameKey, release: &launcher_api::Release) -> CoreResult<()> {
    let plugin = &offer.plugin;
    let allowed = core.kernel().manifest(plugin)?.permissions.allows_link(&release.store);
    if !allowed {
        return Err(CoreError::PermissionDenied { plugin: plugin.clone(), action: format!("open {}", release.store) });
    }
    let link = release.store.clone();
    crate::running::open_link(core, link).await.map_err(CoreError::Invalid)?;
    let store = crate::snapshot::Plugins::new(core.kernel()).system(&release.platform).name;
    tracing::info!(%offer, link = %release.store, "asked the store to install");
    // The store writes its install down a moment later: its progress is read often for now.
    core.state().store_watch_until = Some(core.ports().clock.now() + std::time::Duration::from_secs(60));
    core.notices().info(format!("{store} is installing {}: it shows in your Library when it's done", release.title));
    core.changed(Area::Downloads);
    Ok(())
}

/// The player asked to download `release` of the offered game `offer`: the subsystem reads
/// the torrent's files, and waits for the player to start it.
async fn download_release(core: &Core, offer: &GameKey, release: &str, folder: Option<&Path>) -> CoreResult<()> {
    let found = core
        .library()
        .snapshot()
        .offer(offer)
        .cloned()
        .ok_or_else(|| CoreError::Invalid(format!("no game {offer} in a catalog")))?;
    let release = found
        .releases
        .iter()
        .find(|r| r.id == release)
        .cloned()
        .ok_or_else(|| CoreError::Invalid(format!("no release {release} of {offer}")))?;
    if !release.store.is_empty() {
        return install_in_store(core, offer, &release).await;
    }
    let torrents = core.torrents();
    if torrents.has(&release.id) {
        return Ok(());
    }
    let chosen = folder.is_some();
    if torrents.full() {
        return Err(CoreError::Invalid(format!(
            "Remove an old download first (at most {})",
            crate::torrents::MAX_DOWNLOADS
        )));
    }
    let folders = folders_of(core.kernel(), &found.emulator)?;
    let for_console = console(core.kernel(), &found.emulator, &release.platform);
    let (at, console_folder) = folders.place(Purpose::Download, &for_console, folder)?;
    let name = format!("torrent-{}", release.id);
    let source = at.join(&name).map_err(|e| CoreError::Invalid(e.to_string()))?;
    let folder = console_folder.join(name);
    // A picked folder may be on another disk than the one the Game Hub checked.
    if chosen
        && let Some(size) = release.size_bytes
        && let Some(error) = download_short(size, free_at(core.ports().hardware.as_ref(), &folder))
    {
        return Err(CoreError::Invalid(error));
    }
    if chosen {
        folders.remember(Purpose::Download, &for_console, &console_folder);
    }
    torrents
        .add(NewDownload {
            id: release.id.clone(),
            offer: offer.clone(),
            emulator: found.emulator.clone(),
            title: release.title.clone(),
            version: release.version.clone(),
            region: release.region.clone(),
            magnet: release.torrent.clone(),
            source,
            folder,
            total: release.size_bytes.unwrap_or(0),
        })
        .await
}

/// Runs the player's command on game installs.
pub(crate) async fn game_install(core: &Core, command: GameInstallCmd) -> CoreResult<()> {
    let starts = matches!(command, GameInstallCmd::Download { .. } | GameInstallCmd::Package { .. });
    if starts && let Some(why) = crate::backups::busy(core) {
        return Err(CoreError::Invalid(why.into()));
    }
    match command {
        GameInstallCmd::Download { id, folder } => {
            if !core.torrents().has(&id) {
                return Err(CoreError::Invalid(format!("no download {id}")));
            }
            install_download(core, &id, folder.as_deref()).await
        }
        GameInstallCmd::Package { plugin, source } => install_game(core, &plugin, source, None).await,
        command => core.installs().handle(command).await,
    }
}

/// Starts installing `source` with the plugin's game installer, into the game folder `into`
/// (`None`: the plugin's choice).
pub(crate) async fn install_game(
    core: &Core,
    plugin: &PluginId,
    source: HostPath,
    into: Option<HostPath>,
) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let installer = crate::error::require(plugin, launcher_api::Capability::GameInstaller, provided.game_installer)?;
    let folders = kernel.host_folders(plugin).ok_or_else(|| CoreError::Invalid(format!("{plugin} has no manifest")))?;
    let id = core.inner.downloads.next_id();
    core.installs().begin(NewInstall { id, plugin: plugin.clone(), source, into, installer, folders }).await
}

/// What the game installs subsystem tells the rest of the core.
pub(crate) async fn install_event(core: &Core, event: InstallEvent) {
    match event {
        InstallEvent::Installed => {
            if let Err(e) = Box::pin(core.dispatch(crate::Command::Library(crate::LibraryCmd::Refresh))).await {
                tracing::warn!(error = %e, "library refresh after an install failed");
            }
        }
        InstallEvent::Failed(error) => core.notices().opening(
            launcher_ui_api::NoticeKind::Error,
            crate::Route::Downloads,
            format!("The install failed: {error}"),
        ),
        InstallEvent::Unpacked { plugin, source, into, title } => {
            if let Err(e) = install_game(core, &plugin, source, into).await {
                core.notices().opening(
                    launcher_ui_api::NoticeKind::Error,
                    crate::Route::Downloads,
                    format!("{title} couldn't install: {e}"),
                );
            }
        }
    }
}

/// The player pressed Install on the install sheet, with the game folder they picked
/// (`None` for the one the sheet shows). The archives unpack first. Why it doesn't start
/// stays on the download, for the sheet.
async fn install_download(core: &Core, id: &str, folder: Option<&Path>) -> CoreResult<()> {
    core.installs().set_install_error(id, String::new());
    let started = start_install(core, id, folder).await;
    if let Err(e) = &started {
        core.installs().set_install_error(id, e.to_string());
    }
    started
}

async fn start_install(core: &Core, id: &str, folder: Option<&Path>) -> CoreResult<()> {
    let job = core.torrents().job(id).ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
    if job.state != TorrentState::Complete {
        return Ok(());
    }
    let kernel = core.kernel();
    let folders = folders_of(kernel, &job.emulator)?;
    let console = console_of(kernel, &job.emulator);
    let (into, path) = folders.place(Purpose::Install, &console, folder)?;
    if let Some(error) = install_short(job.total, free_at(core.ports().hardware.as_ref(), &path)) {
        return Err(CoreError::Invalid(error));
    }
    if folder.is_some() {
        folders.remember(Purpose::Install, &console, &path);
    }
    if let Some((plugin, source)) = core.installs().install_download(id, into.clone()) {
        install_game(core, &plugin, source, Some(into)).await?;
    }
    Ok(())
}

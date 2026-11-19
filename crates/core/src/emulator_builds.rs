//! Each emulator's builds: read what is installed and available, install, update and uninstall
//! a build, and look for updates every few hours. The plugin calls run here; what they read and
//! the installs under way belong to the builds subsystem ([`crate::builds`]). Each install claims
//! its emulator first, and its record is saved while it runs, so a crash resumes it. An update
//! for an emulator whose game is running waits until the game closes. A minute after the start,
//! each turned-on emulator with no build gets its newest build ("Install emulators
//! automatically"). With that off, a first play of a game whose emulator isn't installed asks
//! the player first.

use std::sync::Arc;

use launcher_api::{
    Capability, DownloadRequest, GameKey, InstallPhase, PluginId, PluginKind, Provided, SlotId, SlotRequest,
};
use launcher_ui_api::NoticeKind;

use crate::app::{Core, Route};
use crate::builds::{InstallClaim, InstallPrompt};
use crate::changes::Area;
use crate::error::require;
use crate::{CoreError, CoreResult};

/// How often the core looks for emulator updates by itself, after the start's round.
pub const UPDATE_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// Installed and available versions, and panels, for one emulator.
pub(crate) async fn refresh_emulator(core: &Core, plugin: &PluginId) {
    let builds = core.builds();
    let _reading = builds.reading().await;
    let kernel = core.kernel();
    let Ok(provided) = kernel.provided(plugin).await else { return };
    let mut emu = builds.emulator(plugin);
    if let Some(installer) = &provided.installer {
        match kernel.guard(plugin, "installer.installed", installer.installed()).await {
            Ok(list) => {
                emu.own = list.iter().any(|v| v.own) || uses_own_build(core, plugin);
                emu.installed = list.into_iter().map(|v| v.version).collect();
            }
            Err(e) => core.notices().problem(e.to_string()),
        }
        match kernel.guard(plugin, "installer.available", installer.available()).await {
            Ok(list) => emu.available = list.into_iter().map(|v| v.version).collect(),
            Err(e) => core.notices().problem(e.to_string()),
        }
    }
    if let Some(slots) = &provided.slots {
        let request = SlotRequest { slot: SlotId::EmulatorPanel, game: None };
        if let Ok(items) = kernel.guard(plugin, "slots.items", slots.items(&request)).await {
            emu.panels = items;
        }
    }
    builds.read(plugin, emu);
}

pub(crate) async fn refresh_emulators(core: &Core) {
    for plugin in core.kernel().of_kind(PluginKind::Emulator) {
        refresh_emulator(core, &plugin).await;
    }
}

/// The install workflow: plan, download, check, install. Each phase is saved, so a crash
/// resumes it at the next start. It is refused while another install of the emulator runs.
pub(crate) async fn install(
    core: &Core,
    plugin: &PluginId,
    version: Option<String>,
    from: Option<String>,
) -> CoreResult<()> {
    let claim = claim(core, plugin)?;
    install_claimed(core, claim, plugin, version, from).await
}

/// The player's own install, update or uninstall claims the emulator, or is refused. The
/// refusal names the emulator and the state as the launch refusal does ([`crate::launch`]).
fn claim(core: &Core, plugin: &PluginId) -> CoreResult<InstallClaim> {
    core.builds().claim(plugin).ok_or_else(|| {
        let name = core.kernel().manifest(plugin).map(|m| m.name.clone()).unwrap_or_else(|_| plugin.to_string());
        CoreError::Invalid(format!("{name} is updating: wait for it"))
    })
}

/// An automatic install or update claims the emulator. While another install holds it, the
/// automatic one is skipped: that is no failure, and the player sees nothing.
fn claim_if_free(core: &Core, plugin: &PluginId) -> Option<InstallClaim> {
    let claim = core.builds().claim(plugin);
    if claim.is_none() {
        tracing::info!(plugin_id = %plugin, "another install of the emulator runs: this one is skipped");
    }
    claim
}

/// [`install`], once it holds the claim.
async fn install_claimed(
    core: &Core,
    claim: InstallClaim,
    plugin: &PluginId,
    version: Option<String>,
    from: Option<String>,
) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let installer = require(plugin, Capability::Installer, provided.installer.clone())?;
    let downloader = require(plugin, Capability::Downloader, provided.downloader.clone())?;
    let version = match version {
        Some(v) => v,
        None => kernel
            .guard(plugin, "installer.available", installer.available())
            .await?
            .into_iter()
            .next()
            .map(|v| v.version)
            .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no version to install")))?,
    };
    claim.step(&version, from.as_deref(), InstallPhase::Downloading);

    let result: CoreResult<()> = async {
        let request = DownloadRequest::EmulatorBuild { version: Some(version.clone()) };
        let plan = kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await?;
        let files = core.inner.downloads.run(plugin, plan).await?;
        claim.step(&version, from.as_deref(), InstallPhase::Installing);
        match &from {
            Some(from) => kernel.guard(plugin, "installer.update", installer.update(from, &version, &files)).await?,
            None => kernel.guard(plugin, "installer.install", installer.install(&version, &files)).await?,
        };
        Ok(())
    }
    .await;

    match &result {
        Ok(()) => claim.end(&version, InstallPhase::Installed),
        Err(e) => {
            claim.end(&version, InstallPhase::Failed);
            core.notices().opening(
                NoticeKind::Error,
                Route::Plugin(plugin.clone()),
                format!("Could not install {plugin} {version}: {e}"),
            );
        }
    }
    refresh_emulator(core, plugin).await;
    core.changed(Area::Downloads);
    result
}

/// The player's update of `plugin` to its newest build.
pub(crate) async fn update(core: &Core, plugin: &PluginId) -> CoreResult<()> {
    let (from, to) = found_update(core, plugin).await?;
    install(core, plugin, Some(to), Some(from)).await
}

/// An automatic update of `plugin`, skipped while another install of it runs.
async fn update_if_free(core: &Core, plugin: &PluginId) -> CoreResult<()> {
    let (from, to) = found_update(core, plugin).await?;
    let Some(claim) = claim_if_free(core, plugin) else { return Ok(()) };
    install_claimed(core, claim, plugin, Some(to), Some(from)).await
}

/// Reads `plugin`'s builds: its newest installed build, and the newer one it updates to.
async fn found_update(core: &Core, plugin: &PluginId) -> CoreResult<(String, String)> {
    refresh_emulator(core, plugin).await;
    let emu = core.builds().emulator(plugin);
    let from = emu.installed.last().cloned().ok_or_else(|| CoreError::Invalid(format!("{plugin} is not installed")))?;
    let to = emu.update().ok_or_else(|| CoreError::Invalid(format!("{plugin} is up to date")))?;
    Ok((from, to))
}

pub(crate) async fn uninstall(core: &Core, plugin: &PluginId, version: &str) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let installer = require(plugin, Capability::Installer, provided.installer)?;
    let claim = claim(core, plugin)?;
    kernel.guard(plugin, "installer.uninstall", installer.uninstall(version)).await?;
    claim.end(version, InstallPhase::Uninstalled);
    refresh_emulator(core, plugin).await;
    Ok(())
}

/// Re-runs installs that were in progress when the app last stopped.
pub(crate) async fn resume_installs(core: &Core) {
    for (plugin, record) in core.builds().saved_installs() {
        tracing::info!(plugin_id = %plugin, version = %record.version, "resuming an interrupted install");
        let core = core.clone();
        tokio::spawn(async move {
            if let Err(e) = install(&core, &plugin, Some(record.version), record.from).await {
                tracing::warn!(error = %e, plugin_id = %plugin, "could not resume an interrupted install");
            }
        });
    }
}

/// Runs [`Command::CheckUpdates`] every few hours while the app runs, the first one
/// [`UPDATE_CHECK_EVERY`] after the start. The start's own round is [`start_pass`], which the
/// boot runs.
pub(crate) fn schedule_update_checks(core: &Core) {
    let weak = Arc::downgrade(&core.inner);
    let clock = core.ports().clock.clone();
    tokio::spawn(async move {
        loop {
            clock.sleep(UPDATE_CHECK_EVERY).await;
            let Some(inner) = weak.upgrade() else { return };
            let core = Core { inner };
            if core.prefs().auto_update {
                check_updates(&core).await;
            }
        }
    });
}

/// The start's round, right after the boot read each emulator's builds: it installs the
/// missing builds, then the updates. Only this round installs, so a build the player removes
/// stays removed until the next start. The boot screen waits for it ([`crate::startup`]).
pub(crate) async fn start_pass(core: &Core) {
    let prefs = core.prefs();
    if prefs.auto_install {
        install_missing(core).await;
    }
    install_updates(core).await;
}

/// Looks for updates of every installed emulator, and installs them when automatic
/// updates are on. One whose game is running waits for it to close.
pub(crate) async fn check_updates(core: &Core) {
    refresh_builds(core).await;
    install_updates(core).await;
}

/// Reads the builds of each emulator that has official builds.
async fn refresh_builds(core: &Core) {
    for plugin in core.kernel().of_kind(PluginKind::Emulator) {
        if has_builds(core, &plugin) {
            refresh_emulator(core, &plugin).await;
        }
    }
}

/// Installs the updates the last read found, when automatic updates are on. One whose game is
/// running waits for it to close.
async fn install_updates(core: &Core) {
    let mut acted = false;
    // The updates download side by side, each emulator's on its own.
    let mut updates = Vec::new();
    for plugin in core.kernel().of_kind(PluginKind::Emulator) {
        if !wants_update(core, &plugin) {
            continue;
        }
        acted = true;
        if plays(core, &plugin) {
            tracing::info!(plugin_id = %plugin, "an update waits for the game to close");
            core.builds().wait_for_game(&plugin);
            continue;
        }
        updates.push(async move {
            if let Err(e) = update_if_free(core, &plugin).await {
                core.state().startup.failed.insert(plugin);
                core.notices().problem(e.to_string());
            }
        });
    }
    futures::future::join_all(updates).await;
    if acted {
        core.changed(Area::Downloads);
    }
}

/// Installs the newest build of each turned-on emulator that has official builds and none
/// installed: see [`wants_build`]. It reads the builds the boot read. A failed install is a
/// notice from [`install`]; the next start tries again.
async fn install_missing(core: &Core) {
    // The emulators install side by side: one's download doesn't wait for another's.
    let installs = core.kernel().of_kind(PluginKind::Emulator).into_iter().filter(|p| wants_build(core, p)).map(
        |plugin| async move {
            let Some(claim) = claim_if_free(core, &plugin) else { return };
            tracing::info!(plugin_id = %plugin, "installing the emulator's newest build");
            if let Err(e) = install_claimed(core, claim, &plugin, None, None).await {
                core.state().startup.failed.insert(plugin.clone());
                tracing::warn!(error = %e, plugin_id = %plugin, "could not install the emulator's build");
            }
        },
    );
    futures::future::join_all(installs).await;
}

/// The emulators [`start_pass`] installs or updates now: a missing build, or an update whose
/// game isn't running.
pub(crate) fn start_pass_installs(core: &Core) -> std::collections::BTreeSet<PluginId> {
    let auto_install = core.prefs().auto_install;
    core.kernel()
        .of_kind(PluginKind::Emulator)
        .into_iter()
        .filter(|plugin| {
            (auto_install && wants_build(core, plugin)) || (wants_update(core, plugin) && !plays(core, plugin))
        })
        .collect()
}

/// The emulator gets an update: automatic updates are on, and the last read found one for its
/// official build. Its install isn't under way. A saved record alone doesn't stop the update: a
/// resume that ended before its first step leaves its record, and the update's own claim keeps
/// it from running beside a resume.
fn wants_update(core: &Core, plugin: &PluginId) -> bool {
    let builds = core.builds();
    if builds.busy(plugin) {
        return false;
    }
    let emu = builds.emulator(plugin);
    has_builds(core, plugin)
        && !emu.installed.is_empty()
        && !emu.own
        && emu.update().is_some()
        && core.prefs().auto_update
}

/// The emulator has official builds and none installed. It isn't one the player runs with their
/// own build, nor one whose install is under way or waits to resume.
fn wants_build(core: &Core, plugin: &PluginId) -> bool {
    let builds = core.builds();
    if !has_builds(core, plugin) || uses_own_build(core, plugin) || builds.busy(plugin) || builds.resuming(plugin) {
        return false;
    }
    let emu = builds.emulator(plugin);
    emu.installed.is_empty() && !emu.own && !emu.available.is_empty()
}

/// The emulator installs official builds: it isn't one that only starts games, as PC.
fn has_builds(core: &Core, plugin: &PluginId) -> bool {
    core.kernel().manifest(plugin).is_ok_and(|m| m.capabilities.contains(&Capability::Installer))
}

/// "Use your own build" is on in the emulator's settings. The plugin lists the build only
/// once the player picks its program, so this reads the setting itself.
fn uses_own_build(core: &Core, plugin: &PluginId) -> bool {
    core.kernel().config().get(&["plugins", plugin.as_str(), "emulator", "own_build"]).as_bool() == Some(true)
}

/// One of the emulator's games is running.
fn plays(core: &Core, plugin: &PluginId) -> bool {
    core.sessions().plays(plugin)
}

/// A game closed: an update that waited for it installs now. One that finds an install under
/// way is dropped: the next look for updates finds it again.
pub(crate) fn game_closed(core: &Core, plugin: &PluginId) {
    if plays(core, plugin) || !core.builds().take_waiting(plugin) || core.builds().busy(plugin) {
        return;
    }
    let (core, plugin) = (core.clone(), plugin.clone());
    tokio::spawn(async move {
        if let Err(e) = update_if_free(&core, &plugin).await {
            core.notices().problem(e.to_string());
        }
        core.changed(Area::Downloads);
    });
}

/// The game's emulator isn't installed: asks the player first. Returns whether it asked.
pub(crate) async fn ask_to_install(core: &Core, game: &GameKey, provided: &Provided) -> CoreResult<bool> {
    let kernel = core.kernel();
    let (Some(installer), Some(downloader)) = (&provided.installer, &provided.downloader) else { return Ok(false) };
    let plugin = &game.plugin;
    if !kernel.guard(plugin, "installer.installed", installer.installed()).await?.is_empty() {
        return Ok(false);
    }
    let version = kernel
        .guard(plugin, "installer.available", installer.available())
        .await?
        .into_iter()
        .next()
        .map(|v| v.version)
        .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no version to install")))?;
    let request = DownloadRequest::EmulatorBuild { version: Some(version.clone()) };
    let size_bytes = match kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await {
        Ok(plan) => plan.artifacts.iter().map(|a| a.size).sum::<Option<u64>>(),
        Err(_) => None,
    };
    let emulator = kernel.manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_else(|_| plugin.to_string());
    let title = core.library().snapshot().title(game);
    core.builds().ask(InstallPrompt { game: game.clone(), emulator, version, title, size_bytes });
    Ok(true)
}

/// The player agreed to install the emulator a game needs: install it, then play.
pub(crate) async fn confirm_install(core: &Core) -> CoreResult<()> {
    let Some(prompt) = core.builds().answered() else { return Ok(()) };
    install(core, &prompt.game.plugin, Some(prompt.version), None).await?;
    crate::launch::launch(core, prompt.game).await
}

/// The player said "not now" to installing the emulator.
pub(crate) fn cancel_install(core: &Core) -> CoreResult<()> {
    core.builds().answered();
    Ok(())
}

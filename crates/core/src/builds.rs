//! The builds subsystem: each emulator's builds and panels as its plugin last gave them, the
//! installs under way, their saved records (`app.installs.<plugin>`), the updates that wait for
//! a game to close, and the install prompt of a first play.
//!
//! One install of an emulator runs at a time, and none of its games starts while it does: an
//! install, an update or an uninstall claims the emulator ([`BuildsSubsystem::claim`]) before
//! its first plugin call. The check and the claim happen under one lock, so two installs can't
//! both start. The flow [`crate::emulator_builds`] makes the plugin calls and hands the
//! subsystem what they read.
//!
//! It follows the subsystem pattern of [`crate::sessions`]: its own lock, the config and the
//! event bus only, [`BuildsSubsystem::snapshot`], and [`Area::Downloads`] (the builds) and
//! [`Area::Sessions`] (the install prompt) on the core's change signal. It never calls out while
//! it holds its lock.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use launcher_api::{Event, GameKey, InstallPhase, PluginId};

use crate::EventBus;
use crate::changes::{Area, Notifier};
use crate::config::ConfigService;

/// Where the installs under way are saved.
const INSTALLS: [&str; 2] = ["app", "installs"];

/// An install in progress, saved under `app.installs.<plugin>` so a crash resumes it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct InstallRecord {
    pub version: String,
    pub from: Option<String>,
    pub phase: InstallPhase,
}

/// A first play waiting for the player to agree to install its emulator.
#[derive(Clone, Debug)]
pub(crate) struct InstallPrompt {
    pub game: GameKey,
    pub emulator: String,
    pub version: String,
    pub title: String,
    pub size_bytes: Option<u64>,
}

/// An emulator's builds and blocks, as its plugin last gave them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EmulatorState {
    pub installed: Vec<String>,
    /// "Use your own build" is on: the installed build, if the player picked one, is theirs,
    /// and it never updates by itself.
    pub own: bool,
    pub available: Vec<String>,
    /// Its `emulator.panel` slot items.
    pub panels: Vec<launcher_api::SlotItem>,
}

impl EmulatorState {
    /// The newest available version, if it is newer than every installed one. Versions
    /// compare as semver when they parse, as text otherwise.
    pub fn update(&self) -> Option<String> {
        let newest = self.available.first()?;
        let installed = self.installed.last()?;
        let newer = match (semver::Version::parse(newest), semver::Version::parse(installed)) {
            (Ok(a), Ok(b)) => a > b,
            _ => newest != installed && !self.installed.contains(newest),
        };
        newer.then(|| newest.clone())
    }
}

/// The subsystem's state, read whole.
#[derive(Clone, Debug, Default)]
pub(crate) struct BuildsSnapshot {
    /// Each emulator's builds and blocks, and a service plugin's blocks once its page opens.
    pub emulators: BTreeMap<PluginId, EmulatorState>,
    /// The emulators whose install, update or uninstall is under way.
    pub installing: BTreeSet<PluginId>,
    pub install_prompt: Option<InstallPrompt>,
    /// Each plugin's system files, for its page, by plugin and console: see
    /// [`crate::system_files`].
    pub system_files: BTreeMap<crate::system_files::Owner, crate::system_files::Status>,
}

impl BuildsSnapshot {
    /// An install, update or uninstall of `plugin` is under way.
    pub fn busy(&self, plugin: &PluginId) -> bool {
        self.installing.contains(plugin)
    }
}

#[derive(Default)]
struct State {
    emulators: BTreeMap<PluginId, EmulatorState>,
    /// The claimed emulators: see [`BuildsSubsystem::claim`].
    installing: BTreeSet<PluginId>,
    install_prompt: Option<InstallPrompt>,
    /// Emulators with an update that waits for their game to close.
    waiting: BTreeSet<PluginId>,
    system_files: BTreeMap<crate::system_files::Owner, crate::system_files::Status>,
}

struct Shared {
    config: Arc<ConfigService>,
    /// For [`Event::InstallChanged`], which plugins hear.
    bus: EventBus,
    notifier: Notifier,
    state: Mutex<State>,
    /// One read of an emulator's builds and panel at a time. Two at once could end in the
    /// wrong order: a read that began before the plugin changed would put the old panel over
    /// the new one.
    reading: tokio::sync::Mutex<()>,
}

/// The emulators' builds. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct BuildsSubsystem {
    inner: Arc<Shared>,
}

/// An install, update or uninstall of one emulator in progress. While it lives, the emulator is
/// busy: no other install of it starts, and none of its games. It lets go when dropped, and
/// leaves the saved record as it was, so an install the app quits in resumes at the next start.
pub(crate) struct InstallClaim {
    builds: BuildsSubsystem,
    plugin: PluginId,
    /// [`InstallClaim::end`] let go already, and named the change.
    ended: bool,
}

impl Drop for InstallClaim {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        self.builds.state().installing.remove(&self.plugin);
        self.builds.inner.notifier.changed(Area::Downloads);
    }
}

impl InstallClaim {
    /// The install reached `phase`: it is saved, so a crash resumes it from there.
    pub fn step(&self, version: &str, from: Option<&str>, phase: InstallPhase) {
        let record = InstallRecord { version: version.into(), from: from.map(str::to_owned), phase };
        let key = [INSTALLS[0], INSTALLS[1], self.plugin.as_str()];
        if let Err(e) = self.builds.inner.config.set(&key, serde_json::to_value(record).expect("records serialize")) {
            tracing::warn!(error = %e, "could not save install state");
        }
        self.builds.published(&self.plugin, version, phase);
    }

    /// The install ended in `phase`: its record goes, and the emulator is free again.
    pub fn end(mut self, version: &str, phase: InstallPhase) {
        if let Err(e) = self.builds.inner.config.remove(&[INSTALLS[0], INSTALLS[1], self.plugin.as_str()]) {
            tracing::warn!(error = %e, "could not save install state");
        }
        self.builds.state().installing.remove(&self.plugin);
        self.ended = true;
        self.builds.published(&self.plugin, version, phase);
    }
}

impl BuildsSubsystem {
    pub fn new(config: Arc<ConfigService>, bus: EventBus, notifier: Notifier) -> Self {
        let shared =
            Shared { config, bus, notifier, state: Mutex::new(State::default()), reading: tokio::sync::Mutex::new(()) };
        Self { inner: Arc::new(shared) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Tells the plugins the install of `plugin` reached `phase`, and the core that the builds
    /// changed.
    fn published(&self, plugin: &PluginId, version: &str, phase: InstallPhase) {
        self.inner.bus.publish(Event::InstallChanged { plugin: plugin.clone(), version: version.into(), state: phase });
        self.inner.notifier.changed(Area::Downloads);
    }

    /// The builds, the installs under way and the install prompt, as they are now.
    pub fn snapshot(&self) -> BuildsSnapshot {
        let state = self.state();
        BuildsSnapshot {
            emulators: state.emulators.clone(),
            installing: state.installing.clone(),
            install_prompt: state.install_prompt.clone(),
            system_files: state.system_files.clone(),
        }
    }

    /// What the plugin's page says of the system files of `owner`.
    pub fn system_files(&self, owner: &crate::system_files::Owner) -> crate::system_files::Status {
        self.state().system_files.get(owner).cloned().unwrap_or_default()
    }

    /// Marks an import of the system files of `owner` as running: `false` while any import
    /// runs, as plugins and consoles may share the folders. The check and the mark happen under
    /// one lock.
    pub fn start_import(&self, owner: &crate::system_files::Owner) -> bool {
        let mut state = self.state();
        if state.system_files.values().any(|s| s.importing) {
            return false;
        }
        let status = state.system_files.entry(owner.clone()).or_default();
        (status.importing, status.progress) = (true, 0.0);
        drop(state);
        self.inner.notifier.changed(Area::Plugins);
        true
    }

    pub fn set_system_files(&self, owner: &crate::system_files::Owner, status: crate::system_files::Status) {
        let before = self.state().system_files.insert(owner.clone(), status.clone());
        if before.as_ref() != Some(&status) {
            self.inner.notifier.changed(Area::Plugins);
        }
    }

    /// `plugin`'s builds and blocks as last read; empty before the first read.
    pub fn emulator(&self, plugin: &PluginId) -> EmulatorState {
        self.state().emulators.get(plugin).cloned().unwrap_or_default()
    }

    /// The emulator build `plugin` runs games on: its newest installed one.
    pub fn build_of(&self, plugin: &PluginId) -> Option<String> {
        self.state().emulators.get(plugin).and_then(|e| e.installed.last().cloned())
    }

    /// Held while the flow reads an emulator's builds from its plugin, so the reads end in the
    /// order they began.
    pub async fn reading(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.reading.lock().await
    }

    /// What the flow read of `plugin`'s builds and blocks. A read that finds them as they were
    /// names no change, so the screen doesn't redraw for nothing.
    pub fn read(&self, plugin: &PluginId, emulator: EmulatorState) {
        let before = self.state().emulators.insert(plugin.clone(), emulator.clone());
        if before.as_ref() != Some(&emulator) {
            self.inner.notifier.changed(Area::Downloads);
        }
    }

    /// One install of an emulator at a time: `None` while another install of `plugin` holds
    /// the claim. The check and the claim happen under one lock, so two installs can't both
    /// start. The flow says what a taken claim means: a refusal for the player's own install, a
    /// skip for an automatic one.
    ///
    /// It is refused too while one of `sharing`, the plugins that share the emulator's data
    /// folder, installs: they install into the same folder.
    pub fn claim(&self, plugin: &PluginId, sharing: &[PluginId]) -> Option<InstallClaim> {
        {
            let mut state = self.state();
            if sharing.iter().any(|p| state.installing.contains(p)) || !state.installing.insert(plugin.clone()) {
                return None;
            }
            // The emulator shows in the lists and on the boot screen before its first read.
            state.emulators.entry(plugin.clone()).or_default();
        }
        self.inner.notifier.changed(Area::Downloads);
        Some(InstallClaim { builds: self.clone(), plugin: plugin.clone(), ended: false })
    }

    /// An install, update or uninstall of `plugin` is under way: it moves the emulator's files
    /// and the games' saves, so no game starts and the emulator isn't turned off until it is
    /// done.
    pub fn busy(&self, plugin: &PluginId) -> bool {
        self.state().installing.contains(plugin)
    }

    /// The emulators whose install, update or uninstall is under way.
    pub fn installing(&self) -> Vec<PluginId> {
        self.state().installing.iter().cloned().collect()
    }

    /// The installs the app last stopped in, each with its plugin. A record that doesn't read is
    /// left out.
    pub fn saved_installs(&self) -> Vec<(PluginId, InstallRecord)> {
        let saved = self.inner.config.get(&INSTALLS);
        let Some(saved) = saved.as_object() else { return Vec::new() };
        saved
            .iter()
            .filter_map(|(plugin, record)| {
                Some((PluginId::new(plugin.clone()).ok()?, serde_json::from_value(record.clone()).ok()?))
            })
            .collect()
    }

    /// An install of `plugin` stopped with the app and resumes at the start.
    pub fn resuming(&self, plugin: &PluginId) -> bool {
        !self.inner.config.get(&[INSTALLS[0], INSTALLS[1], plugin.as_str()]).is_null()
    }

    /// `plugin`'s update waits for its game to close.
    pub fn wait_for_game(&self, plugin: &PluginId) {
        self.state().waiting.insert(plugin.clone());
    }

    /// The update of `plugin` that waited for its game, if one did. It no longer waits.
    pub fn take_waiting(&self, plugin: &PluginId) -> bool {
        self.state().waiting.remove(plugin)
    }

    /// Asks the player to agree to install the emulator a first play needs.
    pub fn ask(&self, prompt: InstallPrompt) {
        self.state().install_prompt = Some(prompt);
        self.inner.notifier.changed(Area::Sessions);
    }

    /// The player answered the install prompt: it goes. What it asked, if it was open.
    pub fn answered(&self) -> Option<InstallPrompt> {
        let prompt = self.state().install_prompt.take();
        self.inner.notifier.changed(Area::Sessions);
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{Change, Changes};

    fn builds() -> (BuildsSubsystem, tokio::sync::broadcast::Receiver<Change>) {
        let changes = Changes::new();
        let heard = changes.subscribe();
        let notifier = Notifier::new(changes, EventBus::new(16));
        (BuildsSubsystem::new(Arc::new(ConfigService::in_memory()), EventBus::new(16), notifier), heard)
    }

    /// The changes to the builds heard since the last call.
    fn downloads(heard: &mut tokio::sync::broadcast::Receiver<Change>) -> usize {
        std::iter::from_fn(|| heard.try_recv().ok()).filter(|c| c.area == Area::Downloads).count()
    }

    #[test]
    fn an_install_s_end_sends_one_change() {
        let (builds, mut heard) = builds();
        let plugin = PluginId::new("emu-a".to_owned()).unwrap();
        let claim = builds.claim(&plugin, &[]).expect("the emulator is free");
        downloads(&mut heard);

        claim.end("1.0", InstallPhase::Installed);
        assert_eq!(downloads(&mut heard), 1);
        assert!(!builds.busy(&plugin));
    }

    /// Plugins that share a data folder install into the same folder: one at a time.
    #[test]
    fn an_install_waits_for_one_of_a_plugin_that_shares_its_data_folder() {
        let (builds, _) = builds();
        let (nes, ps1) = (PluginId::new("emu-nes".to_owned()).unwrap(), PluginId::new("emu-ps1".to_owned()).unwrap());
        let claim = builds.claim(&nes, std::slice::from_ref(&ps1)).expect("both are free");
        assert!(builds.claim(&ps1, std::slice::from_ref(&nes)).is_none(), "the other installs into the same folder");
        assert!(builds.claim(&ps1, &[]).is_some(), "a plugin of its own folder doesn't wait");
        claim.end("1.0", InstallPhase::Installed);
    }

    #[test]
    fn a_read_of_an_emulator_s_builds_names_its_change() {
        let (builds, mut heard) = builds();
        let plugin = PluginId::new("emu-a".to_owned()).unwrap();

        builds.read(&plugin, EmulatorState { installed: vec!["1.0".into()], ..EmulatorState::default() });
        assert_eq!(downloads(&mut heard), 1);
        assert_eq!(builds.build_of(&plugin).as_deref(), Some("1.0"));
    }

    #[test]
    fn a_read_that_finds_the_builds_as_they_were_sends_no_change() {
        let (builds, mut heard) = builds();
        let plugin = PluginId::new("emu-a".to_owned()).unwrap();
        let read = || EmulatorState { installed: vec!["1.0".into()], ..EmulatorState::default() };
        builds.read(&plugin, read());
        downloads(&mut heard);

        builds.read(&plugin, read());
        assert_eq!(downloads(&mut heard), 0);
    }
}

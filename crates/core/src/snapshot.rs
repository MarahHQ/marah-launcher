//! The app's state, read at once for a render: one read of each subsystem, a copy of the
//! core's own state, and what the view needs from the plugins, the config and the disk.
//! [`Core::snapshot`] does every read; [`crate::view::build`] then works on the [`Snapshot`] alone, with
//! no lock and no I/O.
//!
//! What the view needs from the disk is read ahead, off the UI thread ([`refresh_files`]): a
//! render reads none of it, so a slow or sleeping drive can't hold the screen.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use launcher_api::{Capability, GameKey, PluginId, PluginKind, PluginManifest};
use launcher_ui_api::GameSystemVm;
use serde_json::Value;

use crate::Core;
use crate::app::AppState;
use crate::changes::Area;
use crate::folders::Purpose;
use crate::game_installs::InstallsSnapshot;
use crate::grid_query::GridData;
use crate::launcher_update::LauncherUpdateSnapshot;
use crate::library::LibrarySnapshot;
use crate::notices::Notice;
use crate::ownership::Ownership;
use crate::sessions::SessionsSnapshot;
use crate::torrents::TorrentsSnapshot;
use crate::vpn::VpnSnapshot;
use crate::{HostConfig, Job, Kernel, PluginStatus, PreferencesSnapshot};

/// Everything a render shows.
pub(crate) struct Snapshot<'a> {
    pub plugins: Plugins<'a>,
    pub library: LibrarySnapshot,
    pub torrents: TorrentsSnapshot,
    pub installs: InstallsSnapshot,
    pub sessions: SessionsSnapshot,
    /// The emulators' builds, the installs under way and the install prompt.
    pub builds: crate::builds::BuildsSnapshot,
    pub prefs: PreferencesSnapshot,
    pub notices: Arc<Vec<Notice>>,
    pub launcher_update: LauncherUpdateSnapshot,
    pub backup: crate::backup::BackupSnapshot,
    /// The folder browser, while it is open.
    pub browser: Option<std::sync::Arc<crate::browser::Browser>>,
    /// The drives, as last read.
    pub drives: Vec<crate::drives::Drive>,
    /// A read of the drives ended: before, their list is still loading.
    pub drives_listed: bool,
    /// Settings → Backup's folder; `None` with no home folder and none chosen.
    pub backup_folder: Option<std::path::PathBuf>,
    pub vpn: VpnSnapshot,
    /// The emulator downloads.
    pub downloads: Vec<Job>,
    /// The core's own state: the screens, the selection, the grids, the settings panels and the
    /// rest no subsystem holds yet.
    pub state: AppState,
    /// The game whose Game Hub the view shows (see [`detail_game`]).
    pub detail_game: Option<GameKey>,
    /// What the view reads from the disk.
    pub files: Files,
    /// The player's option values for the current theme.
    pub theme_options: Value,
    /// The Catalog shows its "Hidden" filter: no catalog plugin turned it off.
    pub hidden_filter: bool,
    /// The system can start Marah Launcher at login.
    pub can_start_at_login: bool,
    /// The player's choices of what the top bar shows.
    pub tray: crate::tray::Choices,
    /// Now, by the core's clock.
    pub now: Instant,
    /// Now, in seconds since 1970: for "new" releases and the days since a game was played.
    pub now_secs: u64,
}

impl Snapshot<'_> {
    /// The links between the offers and the installed games.
    pub fn ownership(&self) -> Ownership<'_> {
        Ownership::new(&self.library, &self.torrents.jobs, &self.installs)
    }

    /// What the grids read.
    pub fn grid_data<'a>(&'a self, ownership: &'a Ownership<'a>) -> GridData<'a> {
        GridData {
            library: &self.library,
            ownership,
            plays: &self.sessions.plays,
            hidden_filter: self.hidden_filter,
            pc: self.this_pc(),
        }
    }

    /// Home's emulators, in its order: the turned-on emulator plugins that install or start
    /// games, one entry for each of their consoles that is on, by the console's system's full
    /// name ("Nintendo Switch", "PlayStation 5"). An emulator that names no system has one
    /// entry, with no system.
    pub fn home_emulators(&self) -> Vec<(PluginId, Option<&launcher_api::SystemLook>)> {
        let plugins = &self.plugins;
        let mut named: Vec<(String, (PluginId, Option<&launcher_api::SystemLook>))> = Vec::new();
        for id in plugins.of_kind(PluginKind::Emulator) {
            let Some(manifest) = plugins.manifest(&id) else { continue };
            let caps = &manifest.capabilities;
            if !(caps.contains(&Capability::Installer) || caps.contains(&Capability::Launcher)) {
                continue;
            }
            let looks: Vec<_> = plugins.consoles_on(&id).iter().filter_map(|p| p.system).collect();
            if looks.is_empty() && manifest.consoles.is_empty() {
                named.push((String::new(), (id.clone(), None)));
            }
            for look in looks {
                named.push((look.name.to_lowercase(), (id.clone(), Some(look))));
            }
        }
        named.sort_by(|a, b| a.0.cmp(&b.0));
        named.into_iter().map(|(_, entry)| entry).collect()
    }

    /// This PC's system and how it meets each emulator's needs, for each game's best version.
    pub fn this_pc(&self) -> crate::game_groups::ThisPc {
        let emulators = self.plugins.order().iter().filter_map(|id| {
            let manifest = self.plugins.manifest(id).filter(|m| m.kind == PluginKind::Emulator)?;
            let verdict = self.state.hardware.as_ref().map(|specs| crate::hardware::verdict(specs, &manifest.requires));
            Some((id.clone(), crate::game_groups::EmulatorFit::new(&manifest.requires, verdict.as_ref())))
        });
        crate::game_groups::ThisPc { system: self.plugins.host().platform, emulators: emulators.collect() }
    }

    /// The games beside `game` in its row or grid, at most `most` (see
    /// [`crate::grid_query::neighbours`]).
    pub fn neighbours(&self, game: &GameKey, most: usize) -> Vec<GameKey> {
        let ownership = self.ownership();
        let data = self.grid_data(&ownership);
        let state = &self.state;
        crate::grid_query::neighbours(&data, &state.library_view, &state.catalog_view, &state.grid_columns, game, most)
    }

    /// The games on the Catalog's shown pages, in order.
    pub fn catalog_page(&self) -> Vec<GameKey> {
        let ownership = self.ownership();
        self.state.catalog_view.page(&self.grid_data(&ownership))
    }

    /// Days since 1970, today.
    pub fn today(&self) -> i64 {
        (self.now_secs / 86_400) as i64
    }
}

/// What the view reads from the disk, read ahead: see [`refresh_files`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Files {
    /// The games whose folder is there, of every game with its details in.
    pub game_folders: Arc<HashSet<GameKey>>,
    /// The games with a log of their last run.
    pub logs: Arc<HashSet<GameKey>>,
    /// Where each emulator installs the games of each console it plays that is on, for the
    /// downloads' cards.
    pub install_folders: Arc<HashMap<(PluginId, String), PathBuf>>,
    /// The host folders' paths, by the folder's name, as the first plugin that asks for each
    /// sees them.
    pub roots: Arc<HashMap<String, Vec<PathBuf>>>,
}

/// The loaded plugins: their manifests, which are on, and how each is.
pub(crate) struct Plugins<'a> {
    host: &'a HostConfig,
    order: &'a [PluginId],
    manifests: BTreeMap<&'a PluginId, &'a PluginManifest>,
    enabled: BTreeSet<PluginId>,
    statuses: BTreeMap<PluginId, PluginStatus>,
    /// Each plugin's `app.consoles` object: which of its `[[consoles]]` the player turned on or
    /// off.
    consoles: BTreeMap<PluginId, serde_json::Value>,
}

impl<'a> Plugins<'a> {
    pub fn new(kernel: &'a Kernel) -> Self {
        let order = kernel.order();
        Self {
            host: kernel.host(),
            order,
            manifests: order.iter().filter_map(|id| Some((id, kernel.manifest(id).ok()?))).collect(),
            enabled: order.iter().filter(|id| kernel.enabled(id)).cloned().collect(),
            statuses: kernel.statuses(),
            consoles: order
                .iter()
                .map(|id| (id.clone(), kernel.config().get(&["app", "consoles", id.as_str()])))
                .collect(),
        }
    }

    /// The consoles the plugin plays that are on: see [`Kernel::consoles_on`].
    pub fn consoles_on(&self, plugin: &PluginId) -> Vec<launcher_api::Played<'a>> {
        let Some(manifest) = self.manifest(plugin) else { return Vec::new() };
        let saved = self.consoles.get(plugin).unwrap_or(&serde_json::Value::Null);
        manifest.played().into_iter().filter(|p| crate::kernel::console_on(saved, p)).collect()
    }

    /// The console a download of the plugin's is for, which names its folders: `saved`, else
    /// the plugin's first console that is on, else the plugin's id. See
    /// [`crate::release_downloads::console_of_job`].
    pub fn console_of(&self, plugin: &PluginId, saved: &str) -> String {
        if !saved.is_empty() {
            return saved.to_owned();
        }
        let first = self.consoles_on(plugin).iter().find_map(|p| p.console.map(|c| c.as_str().to_owned()));
        first.unwrap_or_else(|| plugin.to_string())
    }

    /// Whether the player has the console `code` of the plugin's `[[consoles]]` on.
    pub fn console_on(&self, plugin: &PluginId, code: &str) -> bool {
        self.consoles_on(plugin).iter().any(|p| p.console.map(launcher_api::Console::as_str) == Some(code))
    }

    pub fn host(&self) -> &'a HostConfig {
        self.host
    }

    /// Loaded plugins, dependencies first.
    pub fn order(&self) -> &'a [PluginId] {
        self.order
    }

    pub fn manifest(&self, plugin: &PluginId) -> Option<&'a PluginManifest> {
        self.manifests.get(plugin).copied()
    }

    /// The plugin's name; its id when it isn't loaded.
    pub fn name(&self, plugin: &PluginId) -> String {
        self.manifest(plugin).map_or_else(|| plugin.to_string(), |m| m.name.clone())
    }

    /// Whether the player has the plugin on.
    pub fn enabled(&self, plugin: &PluginId) -> bool {
        self.enabled.contains(plugin)
    }

    /// The plugin is made for other systems than this one.
    pub fn elsewhere(&self, plugin: &PluginId) -> bool {
        let platform = self.host.platform;
        self.manifest(plugin).is_some_and(|m| platform.is_some_and(|p| !m.supports(p)))
    }

    pub fn status(&self, plugin: &PluginId) -> Option<&PluginStatus> {
        self.statuses.get(plugin)
    }

    /// Turned-on plugins of one kind, in load order.
    pub fn of_kind(&self, kind: PluginKind) -> Vec<PluginId> {
        self.on(|m| m.kind == kind)
    }

    /// Turned-on plugins whose manifest declares `capability`, in load order.
    pub fn providers(&self, capability: Capability) -> Vec<PluginId> {
        self.on(|m| m.capabilities.contains(&capability))
    }

    fn on(&self, pick: impl Fn(&PluginManifest) -> bool) -> Vec<PluginId> {
        let picked = |id: &&PluginId| self.enabled(id) && self.manifest(id).is_some_and(&pick);
        self.order.iter().filter(picked).cloned().collect()
    }

    /// A turned-on plugin gives release catalogs, so there is a Catalog.
    pub fn catalog_available(&self) -> bool {
        !self.providers(Capability::Releases).is_empty()
    }

    /// The system `platform` (`PS5`), as the plugin whose `[system]` it is describes it: its
    /// full name and color. Without one, the code is its name and it has no color.
    pub fn system(&self, platform: &str) -> GameSystemVm {
        let look = self
            .order
            .iter()
            .filter_map(|id| self.manifest(id))
            .flat_map(|m| m.played().into_iter().filter_map(|p| p.system))
            .find(|look| look.id == platform);
        match look {
            Some(look) => GameSystemVm { name: look.name.clone(), color: look.rgb() },
            None => GameSystemVm { name: platform.to_owned(), color: None },
        }
    }
}

/// The game whose Game Hub the view shows (see [`crate::stack::Stack::detail_game`]).
pub(crate) fn detail_game(state: &AppState) -> Option<GameKey> {
    state.stack.detail_game().cloned()
}

/// What the view reads from the disk: each game's folder and log, each emulator's install
/// folder and the host folders.
pub(crate) fn read_files(kernel: &Kernel, library: &LibrarySnapshot) -> Files {
    disk_read();
    let host = kernel.host();
    let mut game_folders = HashSet::new();
    let mut logs = HashSet::new();
    for (key, detail) in library.records() {
        let folder = detail.and_then(|d| d.details.folder.as_ref());
        if folder.is_some_and(|f| crate::app::resolve_game_folder(kernel, key, f).is_some()) {
            game_folders.insert(key.clone());
        }
        if crate::sessions::log_path(&host.data_dir, key).is_file() {
            logs.insert(key.clone());
        }
    }
    let mut roots = HashMap::new();
    let mut install_folders = HashMap::new();
    for (id, manifest) in kernel.order().iter().filter_map(|id| Some((id, kernel.manifest(id).ok()?))) {
        for grant in &manifest.permissions.filesystem {
            roots
                .entry(grant.folder.clone())
                .or_insert_with(|| crate::folders::roots(kernel.config(), host.home.as_deref(), host.platform, grant));
        }
        if manifest.kind == PluginKind::Emulator
            && let Some(folders) = kernel.host_folders(id)
        {
            let mut consoles: Vec<String> =
                kernel.consoles_on(id).iter().filter_map(|p| p.console.map(|c| c.as_str().to_owned())).collect();
            if consoles.is_empty() {
                consoles.push(crate::release_downloads::console_of(kernel, id));
            }
            for console in consoles {
                if let Ok((_, path)) = folders.place(Purpose::Install, &console, None) {
                    install_folders.insert((id.clone(), console), path);
                }
            }
        }
    }
    Files {
        game_folders: Arc::new(game_folders),
        logs: Arc::new(logs),
        install_folders: Arc::new(install_folders),
        roots: Arc::new(roots),
    }
}

/// Reads what the view needs from the disk again, on a blocking thread, and keeps it. Says so,
/// as a change in [`Area::Files`], when it changed. It runs when the library or the folders
/// change, a drive comes or goes, a game ends, and on the system's 5 s tick
/// ([`watch_files`]); a command that changes a game's folder awaits it before its own notice.
pub(crate) async fn refresh_files(core: &Core) {
    let _turn = core.disk_turn().await;
    let reader = core.clone();
    // A drive that stopped answering holds that thread, not the caller; the read stores what it
    // read whenever it ends, unless a later read stored first.
    let read = tokio::task::spawn_blocking(move || {
        let read = reader.inner.files_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let folders_read = reader.inner.folder_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let files = reader.inner.timings.files.time(|| read_files(reader.kernel(), &reader.library().snapshot()));
        let changed = {
            let mut state = reader.state();
            let mut changed = false;
            // The host folders and the install folders: only a read of every game reads them, so
            // the newest of those stands, whatever a read of one game did meanwhile.
            if folders_read > state.folders_read {
                state.folders_read = folders_read;
                let stored = &mut state.files;
                changed |= stored.roots != files.roots || stored.install_folders != files.install_folders;
                (stored.roots, stored.install_folders) = (files.roots, files.install_folders);
            }
            // Each game's folder and log: a read of one game that began since has newer ones.
            if read > state.files_read {
                state.files_read = read;
                let stored = &mut state.files;
                changed |= stored.game_folders != files.game_folders || stored.logs != files.logs;
                (stored.game_folders, stored.logs) = (files.game_folders, files.logs);
            }
            changed
        };
        if changed {
            reader.changed(Area::Files);
        }
    });
    if tokio::time::timeout(crate::drives::SPACE_READ_LIMIT, read).await.is_err() {
        tracing::warn!("the game folders weren't read in time: the view shows them as they were");
    }
}

/// Reads one game's folder and log ahead, before its details go in, so the view shows them with
/// the details. The games' folders and logs of a read of every game that began before this one
/// are dropped when it ends; the caller calls [`drop_older_reads`] once the details are in, for
/// the reads that began meanwhile.
pub(crate) async fn read_game_files(core: &Core, game: &GameKey, folder: Option<launcher_api::HostPath>) {
    let (reader, game) = (core.clone(), game.clone());
    let read = tokio::task::spawn_blocking(move || {
        let kernel = reader.kernel();
        let there = folder.is_some_and(|f| crate::app::resolve_game_folder(kernel, &game, &f).is_some());
        let log = crate::sessions::log_path(&kernel.host().data_dir, &game).is_file();
        let read = reader.inner.files_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let mut state = reader.state();
        state.files_read = state.files_read.max(read);
        let files = &mut state.files;
        for (set, on) in [(&mut files.game_folders, there), (&mut files.logs, log)] {
            if set.contains(&game) != on {
                let set = Arc::make_mut(set);
                if on {
                    set.insert(game.clone())
                } else {
                    set.remove(&game)
                };
            }
        }
    });
    if tokio::time::timeout(crate::drives::SPACE_READ_LIMIT, read).await.is_err() {
        tracing::warn!("a game's folder wasn't read in time");
    }
}

/// The games' folders and logs of a read of every game that began before now are dropped when
/// it ends: it read the library before a change the files already have.
pub(crate) fn drop_older_reads(core: &Core) {
    let read = core.inner.files_reads.fetch_add(1, Ordering::SeqCst) + 1;
    let mut state = core.state();
    state.files_read = state.files_read.max(read);
}

/// Reads the files again ([`refresh_files`]) after each change that can move them: one read at
/// a time, and one more for every change that came meanwhile, however many.
pub(crate) fn watch_files(core: &Core) {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    let weak = Arc::downgrade(&core.inner);
    let mut changes = core.changes();
    let moves =
        |area: Area| matches!(area, Area::Library | Area::Sessions | Area::Settings | Area::Plugins | Area::System);
    tokio::spawn(async move {
        loop {
            match changes.recv().await {
                Ok(change) if !moves(change.area) => continue,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
            // The rest of the burst: this read covers it.
            loop {
                match changes.try_recv() {
                    Ok(_) | Err(TryRecvError::Lagged(_)) => continue,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Closed) => return,
                }
            }
            let Some(inner) = weak.upgrade() else { return };
            refresh_files(&Core { inner }).await;
        }
    });
}

thread_local! {
    /// The disk may not be read on this thread now: see [`no_disk_reads`].
    static NO_DISK_READS: Cell<bool> = const { Cell::new(false) };
}

/// Runs `work`, and panics if it reads what the view reads from the disk on this thread
/// meanwhile: the games' folders and logs, and the host folders. For a test of what must read
/// only what was read ahead, such as a build of the view. It guards the core's reads that
/// call `disk_read`, not every file call.
pub fn no_disk_reads<T>(work: impl FnOnce() -> T) -> T {
    struct Allow;
    impl Drop for Allow {
        fn drop(&mut self) {
            NO_DISK_READS.set(false);
        }
    }
    NO_DISK_READS.set(true);
    let _allow = Allow;
    work()
}

/// Each read of the disk the view's data needs calls this first.
pub(crate) fn disk_read() {
    assert!(!NO_DISK_READS.get(), "the disk was read where only what was read ahead may be read");
}

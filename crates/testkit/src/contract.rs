//! Capability contract suites: the behaviour the host relies on, checked against any emulator
//! plugin or port adapter. Each check panics with `contract <capability>: ...` when the plugin
//! or adapter breaks a rule.
//!
//! - Run [`emulator`] on a started core with the plugin loaded.
//! - Run [`game_install`] on a started core with the plugin loaded, its folders mapped, and a
//!   game for it to install.
//! - Run [`archives`] on an [`Archives`] adapter, with a fixture that makes its archives.
//! - Run [`process_tree`] on a [`ProcessTree`] adapter, with a fixture that starts its processes.
//! - Run [`windows`] on a [`Windows`] adapter, with a fixture that starts a process with a window
//!   and one without.
//! - Run [`torrents`] on a [`Torrents`] adapter, with a fixture that serves a torrent and makes
//!   the adapter, again after a restart.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use launcher_api::*;
use launcher_core::{
    ArchiveError, ArchiveKind, Archives, Command, Escape, ProcessTree, SessionCmd, TorrentStats, Torrents,
    TransferState, UNPACK_SPARE, Windows,
};

use crate::TestCore;

/// Runs every contract for the capabilities `plugin` declares.
pub async fn emulator(test: &TestCore, plugin: &PluginId) {
    assert!(test.report().rejection(plugin).is_none(), "contract boot: {plugin} was rejected: {:?}", test.report());
    let kernel = test.core.kernel();
    let manifest = kernel.manifest(plugin).expect("loaded").clone();
    let provided = kernel.provided(plugin).await.unwrap_or_else(|e| panic!("contract init: {e}"));
    let caps = &manifest.capabilities;

    let mut first_game = None;
    if let Some(catalog) = &provided.catalog {
        first_game = Some(catalog_contract(catalog.as_ref()).await);
    }
    if let Some(downloader) = &provided.downloader {
        downloader_contract(downloader.as_ref()).await;
    }
    if let Some(configurator) = &provided.configurator {
        configurator_contract(configurator.as_ref(), first_game.as_deref()).await;
    }
    if let Some(compatibility) = &provided.compatibility {
        let game = first_game.as_deref().expect("contract compatibility: needs a catalog game");
        compatibility.status(game).await.unwrap_or_else(|e| panic!("contract compatibility: status({game}): {e}"));
        assert!(
            compatibility.status("__no_such_game__").await.is_err(),
            "contract compatibility: unknown game must fail"
        );
    }
    for played in manifest.played() {
        let buttons: BTreeSet<&str> = played.keymap.iter().map(|k| k.button.as_str()).collect();
        assert_eq!(buttons.len(), played.keymap.len(), "contract keymap: each button once");
        assert!(
            played.keymap.iter().all(|k| !k.button.is_empty() && !k.keys.is_empty()),
            "contract keymap: empty button or keys"
        );
    }
    if let Some(slots) = &provided.slots {
        slots_contract(slots.as_ref(), first_game.as_deref()).await;
    }
    if caps.contains(&Capability::Installer) {
        install_contract(test, plugin, provided.installer.as_ref().expect("declared")).await;
    }
    if caps.contains(&Capability::Launcher) {
        launch_contract(test, plugin, first_game.as_deref().expect("contract launcher: needs a catalog game")).await;
    }
}

/// Returns the first game's id.
async fn catalog_contract(catalog: &dyn Catalog) -> String {
    let all = catalog.list(PageRequest::ALL).await.unwrap_or_else(|e| panic!("contract catalog: list: {e}")).items;
    assert!(!all.is_empty(), "contract catalog: the catalog lists no games");
    let ids: BTreeSet<&str> = all.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids.len(), all.len(), "contract catalog: game ids must be unique");
    assert!(all.iter().all(|g| !g.id.is_empty() && !g.title.is_empty()), "contract catalog: empty id or title");

    if all.len() > 1 {
        let page = catalog.list(PageRequest { offset: 1, limit: 1 }).await.expect("paged list").items;
        assert_eq!(page, all[1..2], "contract catalog: offset and limit page through the same list");
    }
    let first = &all[0];
    let details = catalog.details(&first.id).await.unwrap_or_else(|e| panic!("contract catalog: details: {e}"));
    assert_eq!(details.summary.id, first.id, "contract catalog: details describe the asked-for game");
    assert!(
        matches!(catalog.details("__no_such_game__").await, Err(PluginError::NotFound(_))),
        "contract catalog: an unknown game is NotFound"
    );
    let found = catalog.search(&SearchQuery { text: first.title.clone(), limit: 50 }).await.expect("search");
    assert!(found.iter().any(|g| g.id == first.id), "contract catalog: searching a title finds it");
    let one = catalog.search(&SearchQuery { text: String::new(), limit: 1 }).await.expect("search");
    assert!(one.len() <= 1, "contract catalog: search honours its limit");
    first.id.clone()
}

async fn downloader_contract(downloader: &dyn Downloader) {
    let plan = downloader
        .plan(&DownloadRequest::EmulatorBuild { version: None })
        .await
        .unwrap_or_else(|e| panic!("contract downloader: plan for the newest build: {e}"));
    assert!(!plan.label.is_empty(), "contract downloader: a plan has a label");
    assert!(!plan.artifacts.is_empty(), "contract downloader: a plan has files");
    for artifact in &plan.artifacts {
        let hex = artifact.sha256.len() == 64 && artifact.sha256.chars().all(|c| c.is_ascii_hexdigit());
        assert!(hex, "contract downloader: {} needs a hex SHA-256", artifact.dest);
        assert!(!artifact.dest.is_root(), "contract downloader: a file needs a destination");
    }
}

async fn configurator_contract(configurator: &dyn Configurator, game: Option<&str>) {
    let mut scopes = vec![ConfigScope::Emulator];
    scopes.extend(game.map(|g| ConfigScope::Game(g.into())));
    for scope in scopes {
        let schema = configurator.schema(&scope).await.unwrap_or_else(|e| panic!("contract configurator: schema: {e}"));
        assert!(schema.is_object(), "contract configurator: the {scope:?} schema is a JSON object");
        let defaults = configurator.defaults(&scope).await.expect("defaults");
        configurator
            .validate(&scope, &defaults)
            .await
            .unwrap_or_else(|e| panic!("contract configurator: the {scope:?} defaults must validate: {e}"));
        assert!(
            matches!(
                configurator.validate(&scope, &serde_json::json!("not settings")).await,
                Err(PluginError::InvalidInput(_))
            ),
            "contract configurator: garbage is InvalidInput"
        );
    }
}

async fn slots_contract(slots: &dyn Slots, game: Option<&str>) {
    for slot in SlotId::ALL {
        let request = SlotRequest { slot, game: slot.is_per_game().then(|| game.map(str::to_owned)).flatten() };
        let items = slots.items(&request).await.unwrap_or_else(|e| panic!("contract slots: {}: {e}", slot.as_str()));
        let ids: BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), items.len(), "contract slots: ids are unique within {}", slot.as_str());
    }
}

async fn install_contract(test: &TestCore, plugin: &PluginId, installer: &Arc<dyn Installer>) {
    let available = installer.available().await.unwrap_or_else(|e| panic!("contract installer: available: {e}"));
    let newest = available.first().expect("contract installer: at least one version is available").version.clone();
    test.core
        .dispatch(Command::InstallEmulator { plugin: plugin.clone(), version: None })
        .await
        .unwrap_or_else(|e| panic!("contract installer: install through the host: {e}"));
    let installed = installer.installed().await.expect("installed");
    assert!(installed.iter().any(|v| v.version == newest), "contract installer: {newest} shows as installed");
    for v in &installed {
        assert!(!v.path.is_root(), "contract installer: an install has its own folder");
    }
    test.core
        .dispatch(Command::UninstallEmulator { plugin: plugin.clone(), version: newest.clone() })
        .await
        .unwrap_or_else(|e| panic!("contract installer: uninstall: {e}"));
    assert!(
        !installer.installed().await.expect("installed").iter().any(|v| v.version == newest),
        "contract installer: uninstall removes {newest}"
    );
    // Leave it installed for the launch contract.
    test.core.dispatch(Command::InstallEmulator { plugin: plugin.clone(), version: None }).await.expect("reinstall");
}

async fn launch_contract(test: &TestCore, plugin: &PluginId, game: &str) {
    let mut events = test.core.events();
    let key = GameKey::new(plugin.clone(), game);
    // Installs may have run programs (an AppImage's extract); the game is the next one.
    let before = test.spawner.spawned().len();
    test.core
        .dispatch(Command::Session(SessionCmd::Launch(key.clone())))
        .await
        .unwrap_or_else(|e| panic!("contract launcher: launch through the host: {e}"));
    let process =
        tokio::time::timeout(Duration::from_secs(5), test.spawner.wait_for(before + 1)).await.expect("spawned");
    // The program is in the plugin's data folder, in a host folder it was granted, or a
    // system program by name.
    let program = &process.command.program;
    let data = test.core.kernel().host().plugin_dir(plugin);
    let grants = &test.core.kernel().manifest(plugin).expect("loaded").permissions.filesystem;
    let folders = test.core.view().settings.folders;
    let in_granted_folder = folders
        .iter()
        .filter(|f| grants.iter().any(|g| g.folder == f.name))
        .flat_map(|f| f.paths.iter())
        .any(|root| program.starts_with(root));
    let system = program.components().count() == 1;
    assert!(
        program.starts_with(&data) || in_granted_folder || system,
        "contract launcher: {} is not in a place the plugin may run programs from",
        program.display()
    );
    process.exit(0);
    let exited = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if let Event::ProcessExited { game, .. } = event {
                return game;
            }
        }
        panic!("the event bus closed")
    })
    .await
    .expect("contract launcher: the host reports the exit");
    assert_eq!(exited, key);
}

/// The game install contract, for a plugin whose game installer installs from `source`: the game
/// installs, and installing it again is refused before the plugin's `begin` returns, as
/// [`PluginError::Precondition`]. Neither the install nor the refusal leaves a work folder
/// (`<name>.part`) in any host folder.
pub async fn game_install(test: &TestCore, plugin: &PluginId, source: &HostPath) {
    let start = || Command::RunAction {
        plugin: plugin.clone(),
        action: HOST_INSTALL_GAME.into(),
        game: None,
        args: serde_json::to_value(source).expect("a host path is JSON"),
    };
    // A fixture may hold a work folder of its own: only a new one counts.
    let work_before = work_folders(test);
    let before = test.core.view().downloads.jobs.len();
    test.core.dispatch(start()).await.unwrap_or_else(|e| panic!("contract game install: install {}: {e}", source.path));
    test.until(launcher_core::Area::Installs, |core| {
        let jobs = core.view().downloads.jobs;
        // The newest job is first.
        jobs.len() > before && jobs.first().is_some_and(|j| j.state != "installing")
    })
    .await;
    let job = test.core.view().downloads.jobs.into_iter().next().expect("the install's job");
    assert_eq!(job.state, "done", "contract game install: install {}: {}", source.path, job.error);
    assert_eq!(work_folders(test), work_before, "contract game install: a finished install leaves no work folder");

    // An error from the dispatch is an error from `begin`: once `begin` returns, the install runs.
    let again = test.core.dispatch(start()).await;
    assert!(
        matches!(&again, Err(launcher_core::CoreError::Plugin { error: PluginError::Precondition(_), .. })),
        "contract game install: installing an installed game is refused in begin as Precondition: {again:?}"
    );
    assert_eq!(work_folders(test), work_before, "contract game install: a refused install leaves no work folder");
}

/// Every file and folder named `<name>.part` in the host folders.
fn work_folders(test: &TestCore) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    let mut dirs: Vec<PathBuf> =
        test.core.view().settings.folders.iter().flat_map(|f| f.paths.iter().map(PathBuf::from)).collect();
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "part") {
                found.insert(path.clone());
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(path);
            }
        }
    }
    found
}

/// One entry of an archive the [`archives`] contract asks a fixture for.
#[derive(Clone, Debug)]
pub enum Sample {
    /// A file: its path in the archive and its bytes.
    File(String, Vec<u8>),
    /// A link: its path in the archive and its target.
    Link(String, String),
}

/// Makes archives for an [`Archives`] adapter under the [`archives`] contract, in the
/// adapter's own terms: real archive files for the app's adapter, declared contents for a fake.
pub trait ArchiveFixture {
    /// An archive in `dir` that holds `entries`, in order: its kind and its parts.
    fn archive(&self, dir: &Path, entries: &[Sample]) -> (ArchiveKind, Vec<PathBuf>);
    /// An archive in `dir` whose entries need a password, or `None` when the adapter can't
    /// read one on this computer. That check is then skipped, and says so.
    fn locked(&self, dir: &Path) -> Option<(ArchiveKind, Vec<PathBuf>)>;
}

/// The refusals every [`Archives`] adapter makes before it writes anything: a path that climbs
/// out with `..`, an absolute path, a link that points out on its own or through other links,
/// an entry under a link, an entry that needs a password, and an archive bigger than the free
/// space. A plain archive unpacks, so an adapter that refuses everything fails too.
pub async fn archives(adapter: &dyn Archives, fixture: &dyn ArchiveFixture) {
    let file = |path: &str, bytes: &[u8]| Sample::File(path.into(), bytes.to_vec());
    let plain = || vec![file("Game/eboot.bin", b"elf"), file("readme.txt", b"hi")];

    let (dir, into) = folders();
    let (kind, parts) = fixture.archive(dir.path(), &plain());
    let last = unpack(adapter, kind, &parts, &into, None).await.unwrap_or_else(|e| panic!("contract archives: {e}"));
    assert_eq!(std::fs::read(into.join("Game/eboot.bin")).ok(), Some(b"elf".to_vec()), "contract archives: unpacks");
    assert_eq!(std::fs::read(into.join("readme.txt")).ok(), Some(b"hi".to_vec()), "contract archives: unpacks");
    assert_eq!(last, (5, 5), "contract archives: progress ends at the files' bytes");

    let leaves = |entry: &str| ArchiveError::Escapes { entry: entry.into(), how: Escape::Leaves };
    let (dir, into) = folders();
    let (kind, parts) = fixture.archive(dir.path(), &[file("good.txt", b"ok"), file("../evil.txt", b"no")]);
    let refused = unpack(adapter, kind, &parts, &into, None).await;
    assert_eq!(refused, Err(leaves("../evil.txt")), "contract archives: `..` leaves the folder");
    refused_before_writing(&into, "`..`");
    assert!(!dir.path().join("evil.txt").exists(), "contract archives: nothing written beside the folder");

    let (dir, into) = folders();
    let absolute = dir.path().join("absolute.txt").to_string_lossy().replace('\\', "/");
    let (kind, parts) = fixture.archive(dir.path(), &[file("good.txt", b"ok"), file(&absolute, b"no")]);
    let refused = unpack(adapter, kind, &parts, &into, None).await;
    assert_eq!(refused, Err(leaves(&absolute)), "contract archives: an absolute path leaves the folder");
    refused_before_writing(&into, "an absolute path");
    assert!(!dir.path().join("absolute.txt").exists(), "contract archives: nothing written at the absolute path");

    for target in ["../../../etc/passwd", "/etc/passwd"] {
        let (dir, into) = folders();
        let link = Sample::Link("Game/passwd".into(), target.into());
        let (kind, parts) = fixture.archive(dir.path(), &[file("good.txt", b"ok"), link]);
        let refused = unpack(adapter, kind, &parts, &into, None).await;
        let out = ArchiveError::Escapes { entry: "Game/passwd".into(), how: Escape::LinksOut };
        assert_eq!(refused, Err(out), "contract archives: a link to {target} points out");
        refused_before_writing(&into, "a link out");
    }

    // `d/l` ends at the top, so `x` ends one folder above it.
    let (dir, into) = folders();
    let link = |path: &str, target: &str| Sample::Link(path.into(), target.into());
    let chained = [file("good.txt", b"ok"), link("d/l", ".."), link("x", "d/l/.."), file("x/evil.txt", b"no")];
    let (kind, parts) = fixture.archive(dir.path(), &chained);
    let refused = unpack(adapter, kind, &parts, &into, None).await;
    let out = ArchiveError::Escapes { entry: "x".into(), how: Escape::LinksOut };
    assert_eq!(refused, Err(out), "contract archives: links that chain out point out");
    refused_before_writing(&into, "links that chain out");
    assert!(!dir.path().join("evil.txt").exists(), "contract archives: nothing written beside the folder");

    let (dir, into) = folders();
    let under = [link("a", "b"), file("b/keep", b"ok"), file("a/f", b"no")];
    let (kind, parts) = fixture.archive(dir.path(), &under);
    let refused = unpack(adapter, kind, &parts, &into, None).await;
    let through = ArchiveError::Escapes { entry: "a/f".into(), how: Escape::ThroughLink };
    assert_eq!(refused, Err(through), "contract archives: a file under a link would write through it");
    refused_before_writing(&into, "a file under a link");

    let (dir, into) = folders();
    match fixture.locked(dir.path()) {
        Some((kind, parts)) => {
            let refused = unpack(adapter, kind, &parts, &into, None).await;
            assert_eq!(refused, Err(ArchiveError::Password), "contract archives: a password is refused");
            refused_before_writing(&into, "a password");
        }
        None => eprintln!("SKIPPED: contract archives: this adapter can't read a locked archive here"),
    }

    let (dir, into) = folders();
    let (kind, parts) = fixture.archive(dir.path(), &plain());
    let refused = unpack(adapter, kind, &parts, &into, Some(UNPACK_SPARE)).await;
    let no_room = ArchiveError::NoRoom { needed: 5 + UNPACK_SPARE, free: UNPACK_SPARE };
    assert_eq!(refused, Err(no_room), "contract archives: an archive bigger than the free space is refused");
    refused_before_writing(&into, "no room");
}

/// A new folder for one check's archive, and the empty folder it unpacks into.
fn folders() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let into = dir.path().join("out");
    std::fs::create_dir_all(&into).expect("the unpack folder");
    (dir, into)
}

/// Unpacks, and returns the last progress the adapter reported.
async fn unpack(
    adapter: &dyn Archives,
    kind: ArchiveKind,
    parts: &[PathBuf],
    into: &Path,
    free: Option<u64>,
) -> Result<(u64, u64), ArchiveError> {
    let last = Arc::new(Mutex::new((0, 0)));
    let progress: launcher_core::UnpackProgress = {
        let last = last.clone();
        Arc::new(move |done, total| *last.lock().unwrap() = (done, total))
    };
    adapter.unpack(kind, parts, into, free, progress, Arc::new(AtomicBool::new(false)), &[]).await?;
    let last = *last.lock().unwrap();
    Ok(last)
}

fn refused_before_writing(into: &Path, case: &str) {
    let left: Vec<_> = std::fs::read_dir(into).expect("the unpack folder").flatten().map(|e| e.path()).collect();
    assert!(left.is_empty(), "contract archives: {case}: nothing written, found {left:?}");
}

/// Starts the processes the [`process_tree`] contract looks for. They run until the contract
/// ends them.
pub trait ProcessFixture {
    /// A process whose command line has `text`, and one it started whose command line has
    /// `text` too: `(parent, child)`.
    fn parent_and_child(&mut self, text: &str) -> (u32, u32);
    /// A process whose environment has `key` set to `value`, and one it started without it:
    /// `(parent, child)`.
    fn env_parent_and_child(&mut self, key: &str, value: &str) -> (u32, u32);
}

/// What the sessions rely on from a [`ProcessTree`] adapter: `find` gives each match not below
/// another match, `with_env` gives the matches and every process below them, and `end` with
/// `force` ends them. The real adapter reads the system's process list, so each check waits a
/// little for the list to catch up.
pub fn process_tree(adapter: &dyn ProcessTree, fixture: &mut dyn ProcessFixture) {
    // Words and a value of this run only: what an earlier run left can't match.
    let text = format!("marah-launcher-contract-{}", std::process::id());
    let (key, value) = ("MARAH_CONTRACT", text.as_str());

    let (parent, child) = fixture.parent_and_child(&text);
    let found = within(|| adapter.find(&text) == [parent]);
    let (env_parent, env_child) = fixture.env_parent_and_child(key, value);
    let with_env = within(|| {
        let ids = adapter.with_env(key, value);
        ids.contains(&env_parent) && ids.contains(&env_child)
    });
    let (last_find, last_env) = (adapter.find(&text), adapter.with_env(key, value));

    // The processes end whatever the checks said; the checks are asserted after.
    adapter.end(&[parent, child, env_parent, env_child], true);
    let ended = within(|| adapter.find(&text).is_empty() && adapter.with_env(key, value).is_empty());

    assert!(found, "contract process tree: find gives {parent} only, not {child} below it: {last_find:?}");
    assert!(with_env, "contract process tree: with_env gives {env_parent} and {env_child} below it: {last_env:?}");
    assert!(ended, "contract process tree: end with force ends them");
}

/// Starts the processes the [`windows`] contract looks at. They run until the fixture ends.
pub trait WindowFixture {
    /// A process with a window on screen.
    fn windowed(&mut self) -> u32;
    /// A process with no window.
    fn windowless(&mut self) -> u32;
}

/// What the sessions rely on from a [`Windows`] adapter: `has_window` answers for the processes
/// it is given, not for any process, and `show_game` brings nothing to the front for processes
/// without a window. A desktop may not say whether a process has a window (`None`); it must
/// never say the wrong thing.
pub fn windows(adapter: &dyn Windows, fixture: &mut dyn WindowFixture) {
    let windowed = fixture.windowed();
    let windowless = fixture.windowless();
    // A new window takes a moment to show.
    let shown = within_secs(20, || adapter.has_window(&[windowed]) != Some(false));
    assert!(shown, "contract windows: has_window for {windowed}, which has a window, stayed Some(false) for 20 s");
    let answer = adapter.has_window(&[windowless]);
    assert_ne!(answer, Some(true), "contract windows: has_window for {windowless}, which has no window");
    assert_ne!(adapter.has_window(&[]), Some(true), "contract windows: has_window for no process");
    assert!(!adapter.show_game(&[windowless]), "contract windows: show_game for {windowless}, which has no window");
    assert!(!adapter.show_game(&[]), "contract windows: show_game for no process");
}

/// The size of a piece of the torrent the [`torrents`] contract downloads.
pub const TORRENT_PIECE: u64 = 16 * 1024;

/// Serves the torrent the [`torrents`] contract downloads, and makes the adapter it checks.
#[async_trait]
pub trait TorrentFixture: Send {
    /// Puts a torrent named `name` into the swarm, with these files in this order, `/` between
    /// the parts of a path, and pieces of [`TORRENT_PIECE`] bytes: its info hash in lowercase
    /// hex, and its magnet link.
    async fn torrent(&mut self, name: &str, files: &[(&str, Vec<u8>)]) -> (String, String);
    /// A new adapter over the same data folder and swarm, as after the launcher restarts. The
    /// contract stops every torrent of the adapter before it asks for the next one.
    fn adapter(&mut self) -> Arc<dyn Torrents>;
    /// The swarm sends the running torrent `id` the pieces it lacks. A real swarm does so by
    /// itself, so its fixture does nothing.
    fn send(&mut self, id: &str);
}

/// A torrent file of `len` bytes, whose pieces of [`TORRENT_PIECE`] bytes differ from each other.
fn torrent_bytes(len: u32, step: u32) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(step) % 251) as u8).collect()
}

/// What the torrents subsystem relies on from a [`Torrents`] adapter, as the port's docs say:
///
/// - Reading the files of a torrent starts nothing. A torrent that isn't running has no stats,
///   and stopping, unpausing or forgetting it is no error.
/// - A start checks the pieces in the folder first. A live start downloads what is missing and
///   ends finished and live, with its files whole.
/// - A paused start stays paused after its check: finished when the files are whole, not
///   finished when a piece is missing, and it downloads nothing. Started again live, a torrent
///   the adapter has paused goes on. Unpausing a live torrent is no error.
/// - A changed file costs at most its own bytes: the check counts every other piece.
/// - After a restart, the check counts the pieces the folder has, and an unpause downloads the
///   rest.
/// - With files left out, it downloads only the chosen files, and makes no file for the rest,
///   after a check too. `total` counts the chosen files, and at most the two pieces they share
///   with the rest.
/// - Stopping a torrent, or every torrent, leaves its files.
///
/// Rules of one engine stay in its own tests: which pieces it trusts after a restart without
/// reading them, how many bytes it fetches, and where it keeps the bytes of a file left out.
pub async fn torrents(fixture: &mut dyn TorrentFixture) {
    // No file starts or ends on the edge of a piece, so pieces span two files.
    let files = [
        ("game.bin", torrent_bytes(100_001, 7)),
        ("update.bin", torrent_bytes(60_003, 13)),
        ("dlc.bin", torrent_bytes(50_007, 17)),
    ];
    let sum =
        |names: &[&str]| -> u64 { files.iter().filter(|(n, _)| names.contains(n)).map(|(_, b)| b.len() as u64).sum() };
    let every = ["game.bin", "update.bin", "dlc.bin"];
    let (id, magnet) = fixture.torrent("Game", &files.iter().map(|(n, b)| (*n, b.clone())).collect::<Vec<_>>()).await;
    let id = id.as_str();
    let adapter = fixture.adapter();
    let scratch = tempfile::tempdir().expect("a temporary folder");
    let whole = scratch.path().join("whole");
    let some = scratch.path().join("some");

    assert!(adapter.stats(id).is_none(), "contract torrents: no stats for a torrent that never started");
    adapter.unpause(id).await.unwrap_or_else(|e| panic!("contract torrents: unpause one not running: {e}"));
    adapter.stop(id).await.unwrap_or_else(|e| panic!("contract torrents: stop one not running: {e}"));
    adapter.forget(id).await.unwrap_or_else(|e| panic!("contract torrents: forget one that kept nothing: {e}"));
    adapter.route(None).await.unwrap_or_else(|e| panic!("contract torrents: route(None): {e}"));

    assert!(adapter.resolve("not a magnet link").await.is_err(), "contract torrents: a bad magnet link fails");
    let info = adapter.resolve(&magnet).await.unwrap_or_else(|e| panic!("contract torrents: resolve: {e}"));
    let mut listed: Vec<(&str, u64)> =
        info.files.iter().map(|f| (f.path.rsplit('/').next().unwrap_or_default(), f.size)).collect();
    listed.sort();
    let mut served: Vec<(&str, u64)> = files.iter().map(|(n, b)| (*n, b.len() as u64)).collect();
    served.sort();
    assert_eq!(listed, served, "contract torrents: resolve lists every file with its size: {info:?}");
    assert!(adapter.stats(id).is_none(), "contract torrents: resolve starts nothing");
    let at = |name: &str| info.files.iter().position(|f| f.path.rsplit('/').next() == Some(name)).expect("listed");
    let path = |folder: &Path, name: &str| folder.join(&info.files[at(name)].path);
    let bytes = |name: &str| &files.iter().find(|(n, _)| *n == name).expect("a file").1;
    let read = |folder: &Path, name: &str| std::fs::read(path(folder, name)).ok();
    let all_whole = |folder: &Path, names: &[&str], when: &str| {
        for name in names {
            assert!(read(folder, name).as_ref() == Some(bytes(name)), "contract torrents: {when}: {name} is whole");
        }
    };

    // A live start into an empty folder downloads every file.
    adapter.start(id, &magnet, &whole, false, None).await.unwrap_or_else(|e| panic!("contract torrents: start: {e}"));
    fixture.send(id);
    let done = until(adapter.as_ref(), id, "finished and live", |s| s.finished && s.state == TransferState::Live).await;
    assert_eq!((done.done, done.total), (sum(&every), sum(&every)), "contract torrents: done and total, finished");
    all_whole(&whole, &every, "downloaded");
    adapter.stop(id).await.unwrap_or_else(|e| panic!("contract torrents: stop: {e}"));
    assert!(adapter.stats(id).is_none(), "contract torrents: no stats once stopped");
    all_whole(&whole, &every, "stopped, its files stay");

    // Paused, with its files whole: its check finishes it, and it stays paused.
    adapter.start(id, &magnet, &whole, true, None).await.unwrap_or_else(|e| panic!("contract torrents: start: {e}"));
    let checked = until(adapter.as_ref(), id, "checked", |s| s.state != TransferState::Checking).await;
    assert_eq!((checked.state, checked.finished), (TransferState::Paused, true), "contract torrents: whole, paused");
    assert_eq!(checked.done, checked.total, "contract torrents: whole, done is the total");
    adapter.start(id, &magnet, &whole, false, None).await.unwrap_or_else(|e| panic!("contract torrents: start: {e}"));
    until(adapter.as_ref(), id, "live again", |s| s.state == TransferState::Live && s.finished).await;
    adapter.unpause(id).await.unwrap_or_else(|e| panic!("contract torrents: unpause one already live: {e}"));
    adapter.stop(id).await.unwrap_or_else(|e| panic!("contract torrents: stop: {e}"));

    // A byte changed in one file: the check finds a piece missing, and nothing downloads.
    let mut broken = bytes("game.bin").clone();
    broken[50_000] ^= 0xff;
    std::fs::write(path(&whole, "game.bin"), &broken).expect("change a file");
    let at_least = sum(&every) - sum(&["game.bin"]);
    adapter.start(id, &magnet, &whole, true, None).await.unwrap_or_else(|e| panic!("contract torrents: start: {e}"));
    let missing = until(adapter.as_ref(), id, "checked", |s| s.state != TransferState::Checking).await;
    assert_eq!((missing.state, missing.finished), (TransferState::Paused, false), "contract torrents: a piece missing");
    assert!(
        (at_least..missing.total).contains(&missing.done),
        "contract torrents: the check counts the pieces of the other files: {missing:?}"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let still = adapter.stats(id).expect("running");
    assert_eq!((still.state, still.finished), (TransferState::Paused, false), "contract torrents: paused stays paused");
    assert_eq!(read(&whole, "game.bin"), Some(broken), "contract torrents: paused, nothing downloads");

    // The launcher closes with the download still running, and starts again: the check counts
    // the pieces the folder has, and the rest downloads.
    adapter.stop_all().await.unwrap_or_else(|e| panic!("contract torrents: stop_all: {e}"));
    drop(adapter);
    let adapter = fixture.adapter();
    assert!(adapter.stats(id).is_none(), "contract torrents: after a restart, nothing runs");
    adapter.start(id, &magnet, &whole, true, None).await.unwrap_or_else(|e| panic!("contract torrents: start: {e}"));
    let resumed = until(adapter.as_ref(), id, "checked", |s| s.state != TransferState::Checking).await;
    assert_eq!((resumed.state, resumed.finished), (TransferState::Paused, false), "contract torrents: after a restart");
    assert!(
        (at_least..resumed.total).contains(&resumed.done),
        "contract torrents: after a restart, the check counts the pieces it has: {resumed:?}"
    );
    adapter.unpause(id).await.unwrap_or_else(|e| panic!("contract torrents: unpause: {e}"));
    fixture.send(id);
    until(adapter.as_ref(), id, "finished after a restart", |s| s.finished && s.state == TransferState::Live).await;
    all_whole(&whole, &every, "finished after a restart");
    adapter.stop(id).await.unwrap_or_else(|e| panic!("contract torrents: stop: {e}"));

    // A file left out: only the chosen files download, and the other gets no file.
    let mut only = vec![at("game.bin"), at("dlc.bin")];
    only.sort();
    let chosen = ["game.bin", "dlc.bin"];
    adapter
        .start(id, &magnet, &some, false, Some(&only))
        .await
        .unwrap_or_else(|e| panic!("contract torrents: start with a file left out: {e}"));
    fixture.send(id);
    let left_out =
        until(adapter.as_ref(), id, "finished with a file left out", |s| s.finished && s.state == TransferState::Live)
            .await;
    // An engine counts whole pieces, so the total may have the bytes the chosen files share a
    // piece with: the file left out shares one piece with the file before it and one with the
    // file after it.
    assert_eq!(left_out.done, left_out.total, "contract torrents: finished, done is the total");
    assert!(
        (sum(&chosen)..=sum(&chosen) + 2 * TORRENT_PIECE).contains(&left_out.total),
        "contract torrents: the total counts the chosen files, not the one left out: {left_out:?}"
    );
    all_whole(&some, &chosen, "a file left out");
    assert!(!path(&some, "update.bin").exists(), "contract torrents: no file for one left out");
    adapter.stop(id).await.unwrap_or_else(|e| panic!("contract torrents: stop: {e}"));
    adapter
        .start(id, &magnet, &some, true, Some(&only))
        .await
        .unwrap_or_else(|e| panic!("contract torrents: start with a file left out: {e}"));
    let checked = until(adapter.as_ref(), id, "checked", |s| s.state != TransferState::Checking).await;
    assert_eq!(
        (checked.state, checked.finished),
        (TransferState::Paused, true),
        "contract torrents: chosen ones whole"
    );
    assert!(!path(&some, "update.bin").exists(), "contract torrents: no file for one left out, after a check");
    adapter.stop(id).await.unwrap_or_else(|e| panic!("contract torrents: stop: {e}"));
    adapter.forget(id).await.unwrap_or_else(|e| panic!("contract torrents: forget: {e}"));
    adapter.forget(id).await.unwrap_or_else(|e| panic!("contract torrents: forget again: {e}"));

    // Stopping every torrent stops this one, and its files stay.
    adapter.start(id, &magnet, &whole, true, None).await.unwrap_or_else(|e| panic!("contract torrents: start: {e}"));
    until(adapter.as_ref(), id, "checked", |s| s.state != TransferState::Checking).await;
    adapter.stop_all().await.unwrap_or_else(|e| panic!("contract torrents: stop_all: {e}"));
    assert!(adapter.stats(id).is_none(), "contract torrents: stop_all stops it");
    all_whole(&whole, &every, "stop_all, its files stay");
}

/// The torrent's stats once `done` holds for them. Panics after a minute.
async fn until(adapter: &dyn Torrents, id: &str, what: &str, done: impl Fn(&TorrentStats) -> bool) -> TorrentStats {
    for _ in 0..1200 {
        if let Some(stats) = adapter.stats(id).filter(|s| done(s)) {
            return stats;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("contract torrents: never {what}: {:?}", adapter.stats(id))
}

/// Whether `done` holds within 5 seconds.
fn within(done: impl Fn() -> bool) -> bool {
    within_secs(5, done)
}

/// Whether `done` holds within `secs` seconds.
fn within_secs(secs: u64, done: impl Fn() -> bool) -> bool {
    let start = std::time::Instant::now();
    while !done() {
        if start.elapsed() > Duration::from_secs(secs) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

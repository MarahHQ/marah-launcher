//! Capability contract suites: the behaviour the host relies on, checked against any emulator
//! plugin or port adapter. Each check panics with `contract <capability>: ...` when the plugin
//! or adapter breaks a rule.
//!
//! - Run [`emulator`] on a started core with the plugin loaded.
//! - Run [`archives`] on an [`Archives`] adapter, with a fixture that makes its archives.
//! - Run [`process_tree`] on a [`ProcessTree`] adapter, with a fixture that starts its processes.
//! - Run [`windows`] on a [`Windows`] adapter, with a fixture that starts a process with a window
//!   and one without.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use launcher_api::*;
use launcher_core::{
    ArchiveError, ArchiveKind, Archives, Command, Escape, ProcessTree, SessionCmd, UNPACK_SPARE, Windows,
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
    let buttons: BTreeSet<&str> = manifest.keymap.iter().map(|k| k.button.as_str()).collect();
    assert_eq!(buttons.len(), manifest.keymap.len(), "contract keymap: each button once");
    assert!(
        manifest.keymap.iter().all(|k| !k.button.is_empty() && !k.keys.is_empty()),
        "contract keymap: empty button or keys"
    );
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
    let data = test.data_dir().join("plugins").join(plugin.as_str());
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
    adapter.unpack(kind, parts, into, free, progress, Arc::new(AtomicBool::new(false))).await?;
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

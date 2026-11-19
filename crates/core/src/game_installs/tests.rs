//! The subsystem on its own, over fake ports: no kernel, no plugins, no core.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use launcher_api::{GameInstaller, HostPath, InstallJob, Installable, InstalledGame, PluginError, PluginId, RelPath};

use super::*;
use crate::archives::{ArchiveSet, Unpack};
use crate::changes::{Change, Changes};
use crate::config::ConfigService;
use crate::{ArchiveError, ArchiveKind, EventBus, NoHardware, TokioClock, UnpackProgress};

fn plugin() -> PluginId {
    PluginId::new("emu-g").unwrap()
}

fn at(folder: &str, path: &str) -> HostPath {
    HostPath::new(folder, 0, RelPath::new(path).unwrap())
}

/// The downloads, kept in memory.
#[derive(Default)]
struct FakeDownloads(Mutex<BTreeMap<String, TorrentJob>>);

impl TorrentDownloads for FakeDownloads {
    fn job(&self, id: &str) -> Option<TorrentJob> {
        self.0.lock().unwrap().get(id).cloned()
    }

    fn jobs(&self) -> Vec<TorrentJob> {
        self.0.lock().unwrap().values().cloned().collect()
    }

    fn hand_off(&self, id: &str, change: &mut dyn FnMut(&TorrentState, &mut HandOff)) -> bool {
        let mut jobs = self.0.lock().unwrap();
        let Some(job) = jobs.get_mut(id) else { return false };
        change(&job.state, &mut job.hand_off);
        true
    }
}

/// Writes one file for each set, named after its first part.
struct FakeArchives;

#[async_trait]
impl Archives for FakeArchives {
    async fn unpack(
        &self,
        _kind: ArchiveKind,
        parts: &[PathBuf],
        into: &Path,
        _free: Option<u64>,
        progress: UnpackProgress,
        _cancel: Arc<AtomicBool>,
    ) -> Result<(), ArchiveError> {
        let name = parts[0].file_name().unwrap();
        std::fs::write(into.join(name), b"game").unwrap();
        progress(1, 1);
        Ok(())
    }
}

/// Installs a package into `games/<name>`, through the work folder `games/<name>.part`. A
/// package called `broken` fails and leaves its work folder behind.
struct FakeInstaller {
    games: PathBuf,
}

#[async_trait]
impl GameInstaller for FakeInstaller {
    async fn candidates(&self) -> launcher_api::PluginResult<Vec<Installable>> {
        Ok(Vec::new())
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> launcher_api::PluginResult<HostPath> {
        let name = source.path.as_str().rsplit('/').next().unwrap().to_owned();
        let work = self.games.join(format!("{name}.part"));
        std::fs::create_dir_all(&work).unwrap();
        let games = self.games.clone();
        let title = name.clone();
        tokio::spawn(async move {
            job.progress(1, 2);
            if title == "broken" {
                job.finished(Err(PluginError::Other("the package is broken".into())));
                return;
            }
            std::fs::rename(work, games.join(&title)).unwrap();
            job.finished(Ok(InstalledGame { dest: at("games", &title), title }));
        });
        Ok(at("games", &format!("{name}.part")))
    }
}

/// Lists a package in the background and never answers `begin`, as a plugin's slow listing of a
/// large package. Its work stops once the job is cancelled, and says so on `stopped`.
struct HangingInstaller {
    stopped: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[async_trait]
impl GameInstaller for HangingInstaller {
    async fn candidates(&self) -> launcher_api::PluginResult<Vec<Installable>> {
        Ok(Vec::new())
    }

    async fn begin(&self, _source: &HostPath, job: Arc<dyn InstallJob>) -> launcher_api::PluginResult<HostPath> {
        let stopped = self.stopped.lock().unwrap().take().unwrap();
        tokio::spawn(async move {
            while !job.cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let _ = stopped.send(());
        });
        std::future::pending().await
    }
}

/// Never answers `begin`, but its work goes on: once `go` fires, it finishes `Ok` and doesn't
/// look at `cancelled`, as a plugin whose `begin` hangs after its work started.
struct LateInstaller {
    go: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

#[async_trait]
impl GameInstaller for LateInstaller {
    async fn candidates(&self) -> launcher_api::PluginResult<Vec<Installable>> {
        Ok(Vec::new())
    }

    async fn begin(&self, _source: &HostPath, job: Arc<dyn InstallJob>) -> launcher_api::PluginResult<HostPath> {
        let go = self.go.lock().unwrap().take().unwrap();
        tokio::spawn(async move {
            let _ = go.await;
            job.finished(Ok(InstalledGame { dest: at("games", "late"), title: "late".into() }));
        });
        std::future::pending().await
    }
}

/// What the core heard, with the job's phase at that moment.
type Heard = Arc<Mutex<Vec<(String, Option<GamePhase>)>>>;

struct Setup {
    installs: GameInstallsSubsystem,
    downloads: Arc<FakeDownloads>,
    config: Arc<ConfigService>,
    changes: Changes,
    heard: Heard,
    /// Told each time the core hears something.
    told: Arc<tokio::sync::Notify>,
    games: tempfile::TempDir,
    packages: tempfile::TempDir,
}

fn setup() -> Setup {
    setup_with(Duration::from_secs(5))
}

/// [`setup`], with `call_timeout` the longest a plugin's `begin` may take.
fn setup_with(call_timeout: Duration) -> Setup {
    let config = Arc::new(ConfigService::in_memory());
    let (games, packages) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let folders = serde_json::json!({
        "games": [games.path().display().to_string()],
        "packages": [packages.path().display().to_string()],
    });
    config.set(&["app", "folders"], folders).unwrap();
    let downloads = Arc::new(FakeDownloads::default());
    let changes = Changes::new();
    let heard = Heard::default();
    let told = Arc::new(tokio::sync::Notify::new());
    // The installs are read through this, once the subsystem is made.
    let reader: Arc<Mutex<Option<GameInstallsSubsystem>>> = Arc::default();
    let events: EventSink = {
        let (heard, told, reader) = (heard.clone(), told.clone(), reader.clone());
        Arc::new(move |event| {
            let installs = reader.lock().unwrap().clone().unwrap();
            let phase = installs.snapshot().jobs.values().next().map(|j| j.phase);
            let name = match event {
                InstallEvent::Installed => "installed".to_owned(),
                InstallEvent::Failed(error) => format!("failed: {error}"),
                InstallEvent::Unpacked { source, into, title, .. } => {
                    format!("unpacked {title} from {} into {:?}", source.path, into.map(|i| i.path.to_string()))
                }
            };
            heard.lock().unwrap().push((name, phase));
            told.notify_one();
            Box::pin(async {})
        })
    };
    let installs = GameInstallsSubsystem::new(
        InstallPorts {
            archives: Arc::new(FakeArchives),
            hardware: Arc::new(NoHardware),
            clock: Arc::new(TokioClock),
            downloads: downloads.clone(),
            call_timeout,
        },
        Records::new(config.clone()),
        Notifier::new(changes.clone(), EventBus::new(16)),
        events,
    );
    *reader.lock().unwrap() = Some(installs.clone());
    Setup { installs, downloads, config, changes, heard, told, games, packages }
}

impl Setup {
    fn folders(&self) -> HostFolders {
        let permissions = serde_json::from_value(serde_json::json!({
            "filesystem": [{ "folder": "games", "write": true }, { "folder": "packages", "write": true }],
        }))
        .unwrap();
        HostFolders {
            plugin: plugin(),
            name: "Emu".into(),
            permissions,
            config: self.config.clone(),
            home: None,
            platform: None,
        }
    }

    fn new_install(&self, id: u64, source: HostPath) -> NewInstall {
        NewInstall {
            id: JobId(id),
            plugin: plugin(),
            source,
            into: None,
            installer: Arc::new(FakeInstaller { games: self.games.path().to_owned() }),
            folders: self.folders(),
        }
    }

    fn phase(&self, id: u64) -> Option<GamePhase> {
        self.installs.snapshot().jobs.get(&JobId(id)).map(|j| j.phase)
    }

    fn heard(&self) -> Vec<(String, Option<GamePhase>)> {
        self.heard.lock().unwrap().clone()
    }

    /// A finished download of one archive, `game.zip`, in `packages/torrent-abc`.
    fn finished_download(&self) -> PathBuf {
        let folder = self.packages.path().join("torrent-abc");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("game.zip"), b"zip").unwrap();
        let set =
            ArchiveSet { name: "game".into(), kind: ArchiveKind::Zip, parts: vec!["game.zip".into()], problem: None };
        let job = TorrentJob {
            id: "abc".into(),
            offer: launcher_api::GameKey::new(PluginId::new("catalog").unwrap(), "astro"),
            emulator: plugin(),
            title: "Astro".into(),
            version: String::new(),
            region: String::new(),
            magnet: String::new(),
            source: at("packages", "torrent-abc"),
            folder: folder.clone(),
            state: TorrentState::Complete,
            done: 3,
            total: 3,
            files: Vec::new(),
            file_count: 1,
            skip: Vec::new(),
            parts_kept: false,
            install_after: None,
            error: String::new(),
            download_rate: 0,
            upload_rate: 0,
            peers: 0,
            seeding: false,
            verifying: false,
            waiting_vpn: false,
            start: None,
            hand_off: HandOff {
                unpack: Unpack { sets: vec![set], phase: UnpackPhase::Waiting, ..Unpack::default() },
                ..HandOff::default()
            },
        };
        self.downloads.0.lock().unwrap().insert(job.id.clone(), job);
        folder
    }
}

/// Waits until `done` holds, looking after each change of the installs.
async fn until(changes: &Changes, mut done: impl FnMut() -> bool) {
    let mut rx = changes.subscribe();
    while !done() {
        match rx.recv().await {
            Ok(Change { .. }) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(e) => panic!("the changes stopped: {e}"),
        }
    }
}

#[test]
fn install_starts_at_once_unpacks_first_or_waits_by_the_unpack_phase() {
    use UnpackPhase as U;
    let table = [
        (U::None, Step::Install),
        (U::Unpacked, Step::Install),
        (U::Waiting, Step::Unpack),
        (U::Failed, Step::Unpack),
        (U::Cancelled, Step::Unpack),
        (U::Unpacking, Step::Nothing),
    ];
    for (phase, step) in table {
        assert_eq!(first_step(phase), step, "{phase:?}");
    }
}

#[tokio::test]
async fn an_install_shows_as_done_only_after_the_core_heard_of_the_game() {
    let s = setup();
    s.installs.begin(s.new_install(1, at("packages", "astro"))).await.unwrap();
    until(&s.changes, || s.phase(1) == Some(GamePhase::Done)).await;

    assert_eq!(s.heard(), [("installed".to_owned(), Some(GamePhase::Installing))]);
    assert!(s.games.path().join("astro").is_dir());
    let records = s.config.get(&["app", "game_installs"]);
    assert!(records.as_object().is_none_or(|r| r.is_empty()), "no work folder to clean up: {records}");
    let job = s.installs.snapshot().jobs[&JobId(1)].clone();
    assert_eq!((job.done, job.total), (2, 2), "a finished job is full");
}

#[tokio::test]
async fn a_failed_install_tells_the_player_first_and_removes_its_work_folder() {
    let s = setup();
    s.installs.begin(s.new_install(1, at("packages", "broken"))).await.unwrap();
    until(&s.changes, || s.phase(1) == Some(GamePhase::Failed)).await;

    let heard = s.heard();
    assert_eq!(heard, [("failed: the package is broken".to_owned(), Some(GamePhase::Installing))]);
    assert!(!s.games.path().join("broken.part").exists(), "the host removes what the plugin left");
}

#[tokio::test]
async fn the_same_source_doesn_t_install_twice_at_once() {
    let s = setup();
    s.installs.begin(s.new_install(1, at("packages", "astro"))).await.unwrap();
    // The plugin's work runs once this test waits.
    let again = s.installs.begin(s.new_install(2, at("packages", "astro"))).await;
    assert!(again.unwrap_err().to_string().contains("already installing"));
    assert_eq!(s.phase(2), None, "the refused install isn't listed");
}

#[tokio::test]
async fn a_begin_that_times_out_cancels_the_plugin_s_work() {
    let s = setup_with(Duration::from_millis(50));
    let (stopped, heard) = tokio::sync::oneshot::channel();
    let mut install = s.new_install(1, at("packages", "huge"));
    install.installer = Arc::new(HangingInstaller { stopped: Mutex::new(Some(stopped)) });

    let begun = s.installs.begin(install).await;

    assert!(begun.unwrap_err().to_string().contains("timed out"), "the plugin didn't answer in time");
    assert_eq!(s.phase(1), None, "the install that didn't start isn't listed");
    let stops = tokio::time::timeout(Duration::from_secs(5), heard).await;
    assert!(stops.is_ok_and(|told| told.is_ok()), "the plugin's work saw the job cancelled");
    // The work dropped the job as it stopped: the player heard of the failure from `begin`.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(s.heard(), [], "no second failure");
}

#[tokio::test]
async fn a_begin_that_times_out_still_lists_the_game_its_work_finished() {
    let s = setup_with(Duration::from_millis(50));
    let (go, wait) = tokio::sync::oneshot::channel();
    let mut install = s.new_install(1, at("packages", "late"));
    install.installer = Arc::new(LateInstaller { go: Mutex::new(Some(wait)) });

    let begun = s.installs.begin(install).await;
    assert!(begun.unwrap_err().to_string().contains("timed out"), "the plugin didn't answer in time");
    let told = s.told.notified();
    go.send(()).unwrap();

    let heard = tokio::time::timeout(Duration::from_secs(5), told).await;
    assert!(heard.is_ok(), "the core heard of the game");
    assert_eq!(s.heard(), [("installed".to_owned(), None)], "the library lists it, and no failure follows");
    assert_eq!(s.phase(1), None, "the install that didn't start stays off the list");
}

#[tokio::test]
async fn install_unpacks_a_download_s_archives_first_then_asks_the_core_to_install() {
    let s = setup();
    let folder = s.finished_download();

    assert_eq!(s.installs.install_download("abc", at("games", "PS5")), None, "it unpacks first");
    // The hand-off changes publish as the downloads' own, which the fake doesn't: the event
    // says the unpack ended.
    s.told.notified().await;

    assert_eq!(s.downloads.job("abc").unwrap().hand_off.unpack.phase, UnpackPhase::Unpacked);
    assert_eq!(std::fs::read(folder.join("unpacked/game/game.zip")).unwrap(), b"game");
    assert_eq!(s.heard(), [("unpacked Astro from torrent-abc into Some(\"PS5\")".to_owned(), None)]);
    let job = s.downloads.job("abc").unwrap();
    assert_eq!(job.hand_off.unpack.progress, 1.0);
    assert_eq!(job.hand_off.install_into, Some(at("games", "PS5")));

    // Unpacked: Install starts the install at once.
    let now = s.installs.install_download("abc", at("games", "PS5"));
    assert_eq!(now, Some((plugin(), at("packages", "torrent-abc"))));
}

#[test]
fn the_work_folder_a_crash_left_goes_at_the_next_start() {
    let s = setup();
    std::fs::create_dir_all(s.games.path().join("astro.part")).unwrap();
    let records = serde_json::json!({ "7": { "plugin": "emu-g", "work": at("games", "astro.part") } });
    s.config.set(&["app", "game_installs"], records).unwrap();

    s.installs.clean_up(|p| (p == &plugin()).then(|| s.folders()));

    assert!(!s.games.path().join("astro.part").exists());
    assert_eq!(s.config.get(&["app", "game_installs"]), serde_json::json!({}));
}

#[test]
fn a_work_folder_out_of_reach_at_the_start_goes_at_a_later_start() {
    let s = setup();
    // The games folder is on a drive that isn't plugged in.
    let drive = s.games.path().join("drive");
    s.config.set(&["app", "folders", "games"], serde_json::json!([drive.display().to_string()])).unwrap();
    let left = serde_json::json!({ "plugin": "emu-g", "work": at("games", "astro.part") });
    s.config.set(&["app", "game_installs"], serde_json::json!({ "1": left })).unwrap();

    s.installs.clean_up(|p| (p == &plugin()).then(|| s.folders()));

    let kept = s.config.get(&["app", "game_installs"]);
    // The kept record names its games folder by path too.
    let named = serde_json::json!({ "plugin": "emu-g", "work": at("games", "astro.part"), "games": drive });
    assert_eq!(kept.as_object().map(|r| r.values().collect::<Vec<_>>()), Some(vec![&named]), "kept: {kept}");
    // This run's install 1 comes and goes, and leaves the last run's record as it was.
    s.installs.inner.records.save(JobId(1), &plugin(), &at("games", "quest.part"));
    s.installs.inner.records.forget(JobId(1));
    assert_eq!(s.config.get(&["app", "game_installs"]), kept);

    // At the next start, the drive is back.
    std::fs::create_dir_all(drive.join("astro.part")).unwrap();
    s.installs.clean_up(|p| (p == &plugin()).then(|| s.folders()));

    assert!(!drive.join("astro.part").exists());
    assert_eq!(s.config.get(&["app", "game_installs"]), serde_json::json!({}));
}

/// Sets the player's games folders.
fn games_folders(s: &Setup, folders: &[&Path]) {
    let list: Vec<String> = folders.iter().map(|f| f.display().to_string()).collect();
    s.config.set(&["app", "folders", "games"], serde_json::json!(list)).unwrap();
}

/// Saves what the last run left: an install of `astro` at games root `root`.
fn left_at(s: &Setup, root: u32) {
    let work = HostPath::new("games", root, RelPath::new("astro.part").unwrap());
    let left = serde_json::json!({ "plugin": "emu-g", "work": work });
    s.config.set(&["app", "game_installs"], serde_json::json!({ "1": left })).unwrap();
}

fn clean_up(s: &Setup) {
    s.installs.clean_up(|p| (p == &plugin()).then(|| s.folders()));
}

fn records(s: &Setup) -> serde_json::Value {
    s.config.get(&["app", "game_installs"])
}

#[test]
fn a_kept_record_finds_its_games_folder_by_path_when_the_list_changes() {
    let s = setup();
    let (a, b) = (s.games.path().join("a"), s.games.path().join("b"));
    std::fs::create_dir_all(b.join("other")).unwrap();
    // Drive `a` isn't plugged in.
    games_folders(&s, &[&a, &b]);
    left_at(&s, 0);
    clean_up(&s);

    // The player puts `b` first, and drive `a` is back.
    games_folders(&s, &[&b, &a]);
    std::fs::create_dir_all(a.join("astro.part")).unwrap();
    std::fs::create_dir_all(b.join("astro.part")).unwrap();
    clean_up(&s);

    assert!(!a.join("astro.part").exists(), "the work folder on `a` goes");
    assert!(b.join("astro.part").is_dir(), "the folder of the same name on `b` stays");
    assert_eq!(records(&s), serde_json::json!({}));
}

#[test]
fn a_kept_record_whose_games_folder_left_the_settings_is_dropped() {
    let s = setup();
    let (a, b) = (s.games.path().join("a"), s.games.path().join("b"));
    std::fs::create_dir_all(b.join("astro.part")).unwrap();
    games_folders(&s, &[&a, &b]);
    left_at(&s, 0);
    clean_up(&s);

    // The player removes `a` from Settings: index 0 is `b` now.
    games_folders(&s, &[&b]);
    clean_up(&s);

    assert!(b.join("astro.part").is_dir(), "nothing on `b` goes");
    assert_eq!(records(&s), serde_json::json!({}), "the record is dropped");
}

#[test]
fn a_record_whose_games_root_is_gone_from_the_list_is_dropped() {
    let s = setup();
    games_folders(&s, &[s.games.path()]);
    left_at(&s, 3);
    clean_up(&s);

    assert_eq!(records(&s), serde_json::json!({}));
}

#[test]
fn an_empty_games_folder_is_a_drive_not_mounted_and_its_record_waits() {
    let s = setup();
    // A mount point stays as an empty folder while its drive is away.
    let mount = s.games.path().join("mnt");
    std::fs::create_dir_all(&mount).unwrap();
    games_folders(&s, &[&mount]);
    left_at(&s, 0);
    clean_up(&s);

    assert_eq!(records(&s).as_object().map(serde_json::Map::len), Some(1), "kept: {}", records(&s));

    // The drive is mounted again.
    std::fs::create_dir_all(mount.join("astro.part")).unwrap();
    clean_up(&s);

    assert!(!mount.join("astro.part").exists());
    assert_eq!(records(&s), serde_json::json!({}));
}

#[test]
fn a_games_folder_with_files_but_no_work_folder_drops_the_record() {
    let s = setup();
    std::fs::create_dir_all(s.games.path().join("quest")).unwrap();
    left_at(&s, 0);
    clean_up(&s);

    assert_eq!(records(&s), serde_json::json!({}));
}

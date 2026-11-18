//! Seam 1, core app API: the download sheet of a release a catalog offers. The player picks the
//! files (the game itself always on), sees the free space where it downloads and where it
//! installs, and with "Download and install" the game installs by itself once the download
//! finishes. Through the fake swarm, the fake disks and a stub emulator's game installer.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use launcher_api::*;
use launcher_core::{Area, Command, GameInstallCmd, TorrentAction, TorrentCmd};
use launcher_testkit::{FakeHost, StubPlugin, TestCore, until as until_in};

use crate::common::{catalogs_fetched, id};

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
const URL: &str = "https://xyz.dev/c/ps5.json";
const GAME: &[u8] = b"the game's package, the biggest file";
const UPDATE: &[u8] = b"an update package";
const DLC: &[u8] = b"a costume";
const README: &[u8] = b"read me";

/// The torrent's files, in its order.
fn files() -> [(&'static str, &'static [u8]); 4] {
    [
        ("Astro/Astro.pkg", GAME),
        ("Astro/Update/Astro-v1.05.pkg", UPDATE),
        ("Astro/DLC/Costume.pkg", DLC),
        ("Astro/readme.txt", README),
    ]
}

/// What the stub emulator's game installer saw.
#[derive(Clone, Default)]
struct Seen {
    /// Each install's source.
    sources: Arc<Mutex<Vec<HostPath>>>,
    /// The files in the download's folder when each install began, relative to it.
    files: Arc<Mutex<Vec<Vec<String>>>>,
    /// The next install fails.
    fail: Arc<AtomicBool>,
}

/// Every file under `folder`, relative to it, sorted.
fn listing(folder: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push(path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, folder, &mut out);
    out.sort();
    out
}

/// An emulator with no games of its own, whose game installer says what it got, and fails
/// while `seen.fail` is set.
fn emulator(seen: Seen, folder: PathBuf) -> StubPlugin {
    struct Emu(Seen, PathBuf);
    #[async_trait]
    impl Catalog for Emu {
        async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
            Ok(Page { items: vec![], total: None })
        }
        async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
            Ok(vec![])
        }
        async fn details(&self, game: &str) -> PluginResult<GameDetails> {
            Err(PluginError::NotFound(game.into()))
        }
        async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
            Ok(None)
        }
    }
    #[async_trait]
    impl GameInstaller for Emu {
        async fn candidates(&self) -> PluginResult<Vec<Installable>> {
            Ok(vec![])
        }
        async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
            self.0.sources.lock().unwrap().push(source.clone());
            self.0.files.lock().unwrap().push(listing(&self.1));
            let result = if self.0.fail.swap(false, Ordering::SeqCst) {
                Err(PluginError::Other("the package is damaged".into()))
            } else {
                Ok(InstalledGame { dest: source.clone(), title: "Astro".into() })
            };
            job.finished(result);
            Ok(HostPath::new("games", 0, RelPath::new("PS5/astro.part").unwrap()))
        }
    }
    StubPlugin::from_toml(
        "id = \"emu-kyty\"\nkind = \"emulator\"\nname = \"KytyPS5\"\nversion = \"1.0.0\"\nhost_api = \"^0.3\"\n\
         console = \"PS5\"\n\
         capabilities = [\"catalog\", \"game_installer\"]\n\
         [[permissions.filesystem]]\nfolder = \"packages\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"\nwrite = true\n",
    )
    .provides(move |_| {
        let emu = Arc::new(Emu(seen.clone(), folder.clone()));
        Ok(Provided { catalog: Some(emu.clone()), game_installer: Some(emu), ..Provided::default() })
    })
}

struct Setup {
    t: TestCore,
    home: tempfile::TempDir,
    seen: Seen,
}

impl Setup {
    /// The download's folder.
    fn folder(&self) -> PathBuf {
        self.home.path().join(format!("Downloads/PS5/torrent-{HASH}"))
    }
}

async fn setup() -> Setup {
    setup_on(|h, _| h).await
}

/// The catalog offers one release, of torrent `HASH` with [`files`], on a host that `build`
/// changes first (it gets the home folder). Its sheet is open: the download is Ready.
async fn setup_on(build: impl FnOnce(FakeHost, &Path) -> FakeHost) -> Setup {
    setup_with(&files(), build).await
}

/// [`setup_on`] with the torrent's `files`.
async fn setup_with(files: &[(&str, &[u8])], build: impl FnOnce(FakeHost, &Path) -> FakeHost) -> Setup {
    let home = tempfile::tempdir().unwrap();
    for folder in ["Downloads", "Games"] {
        std::fs::create_dir_all(home.path().join(folder)).unwrap();
    }
    let seen = Seen::default();
    let folder = home.path().join(format!("Downloads/PS5/torrent-{HASH}"));
    let t = build(FakeHost::new(), home.path())
        .home(home.path())
        .plugin(emulator(seen.clone(), folder))
        .built_in(PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05" } }] });
    t.http.serve(URL, catalog.to_string());
    t.torrents.has(HASH, "Astro", files);
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c", "hide_without_images": false }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    let offer = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
    t.core
        .dispatch(Command::Torrent(TorrentCmd::Download { offer, release: HASH.into(), folder: None }))
        .await
        .unwrap();
    until(&t, |r| r.state == "ready").await;
    // Downloads opens, and reads the free space on its drives.
    t.core.dispatch(Command::Navigate(launcher_core::Route::Downloads)).await.unwrap();
    Setup { t, home, seen }
}

fn row(t: &TestCore) -> launcher_ui_api::TorrentVm {
    t.core.view().downloads.torrents.into_iter().next().expect("a download")
}

/// Waits until the download's row passes `done`, after a change of the downloads.
async fn until(t: &TestCore, done: impl Fn(&launcher_ui_api::TorrentVm) -> bool) -> launcher_ui_api::TorrentVm {
    t.until(Area::Torrents, |core| core.view().downloads.torrents.first().is_some_and(&done)).await;
    row(t)
}

/// Waits until the download's install is `state`.
async fn until_install(t: &TestCore, state: &str) -> launcher_ui_api::TorrentVm {
    t.until(Area::Installs, |core| core.view().downloads.torrents.first().is_some_and(|r| r.install == state)).await;
    row(t)
}

async fn pick(t: &TestCore, file: Option<usize>, on: bool) {
    t.core.dispatch(Command::Torrent(TorrentCmd::Pick { id: HASH.into(), file, on })).await.unwrap();
}

async fn begin(t: &TestCore, install: bool, folder: Option<&str>) -> launcher_core::CoreResult<()> {
    let folder = folder.map(PathBuf::from);
    t.core.dispatch(Command::Torrent(TorrentCmd::Begin { id: HASH.into(), install, folder })).await
}

/// Each file on the sheet: its path, kind, checked and locked.
fn picks(r: &launcher_ui_api::TorrentVm) -> Vec<(String, String, bool, bool)> {
    r.picks.iter().map(|p| (p.path.clone(), p.kind.clone(), p.on, p.locked)).collect()
}

fn bytes(of: &[&[u8]]) -> u64 {
    of.iter().map(|b| b.len() as u64).sum()
}

#[tokio::test]
async fn the_sheet_lists_each_file_with_its_kind_and_size_and_the_game_stays_on() {
    let s = setup().await;
    let t = &s.t;
    let ready = row(t);
    let p = |path: &str, kind: &str, on: bool, locked: bool| (path.to_owned(), kind.to_owned(), on, locked);
    assert_eq!(
        picks(&ready),
        [
            p("Astro/Astro.pkg", "game", true, true),
            p("Astro/Update/Astro-v1.05.pkg", "update", true, false),
            p("Astro/DLC/Costume.pkg", "dlc", true, false),
            p("Astro/readme.txt", "extra", true, false),
        ]
    );
    assert_eq!(ready.picks[0].size, "0 MB");
    assert_eq!(ready.picked, "4 of 4 files · 0 MB");
    assert_eq!(ready.total, bytes(&[GAME, UPDATE, DLC, README]));

    // None leaves the game on; the game can't be unchecked; one file comes back.
    pick(t, None, false).await;
    assert_eq!(row(t).picks.iter().map(|p| p.on).collect::<Vec<_>>(), [true, false, false, false]);
    assert_eq!(row(t).picked, "1 of 4 files · 0 MB");
    pick(t, Some(0), false).await;
    assert!(row(t).picks[0].on, "the game itself always downloads");
    pick(t, Some(1), true).await;
    assert_eq!(row(t).picks.iter().map(|p| p.on).collect::<Vec<_>>(), [true, true, false, false]);
    pick(t, None, true).await;
    assert!(row(t).picks.iter().all(|p| p.on), "all");
    assert!(t.torrents.started().is_empty(), "picking downloads nothing");
}

#[tokio::test]
async fn unchecked_files_get_skip_priority_and_their_stubs_go_when_it_finishes() {
    let s = setup().await;
    let t = &s.t;
    pick(t, Some(2), false).await;
    pick(t, Some(3), false).await;
    begin(t, false, None).await.unwrap();

    assert_eq!(t.torrents.only(HASH), Some(vec![0, 1]), "the DLC and the readme get skip priority");
    assert_eq!(row(t).total, bytes(&[GAME, UPDATE]), "only what is checked counts");
    assert!(s.folder().join("Astro/DLC/Costume.pkg").exists(), "the engine makes a stub");

    t.torrents.complete(HASH);
    let done = until(t, |r| r.state == "complete").await;
    assert_eq!(listing(&s.folder()), ["Astro/Astro.pkg", "Astro/Update/Astro-v1.05.pkg"], "the stubs went");
    assert!(!s.folder().join("Astro/DLC").exists(), "and the folder they left empty");
    assert!(!done.seeding, "a download with files left out doesn't share");
    assert!(t.torrents.running().is_empty());
    assert_eq!(done.actions, ["install", "remove"], "it installs as a separate step");
    assert!(s.seen.sources.lock().unwrap().is_empty(), "Download alone installs nothing");
    assert!(t.core.view().notices.iter().any(|n| n.contains("Install it from Downloads")));
}

#[tokio::test]
async fn the_sheet_says_each_drive_s_free_space_and_warns_when_it_is_short() {
    // The game folder's drive has room for the game and its update, not for the rest.
    let room = bytes(&[GAME, UPDATE]);
    let s = setup_on(|h, home| h.disk(home.join("Downloads"), 5_000_000_000).disk(home.join("Games"), room)).await;
    let t = &s.t;
    let sheet = row(t);
    assert_eq!(sheet.download_free, "5.0 GB free");
    assert_eq!(sheet.install_free, "0.0 GB free");
    assert_eq!(sheet.download_room, "");
    assert_eq!(sheet.install_room, "Not enough free space in the game folder: 0.0 GB needed, 0.0 GB free");

    // Download and install is refused before anything downloads; Download alone isn't.
    let refused = begin(t, true, None).await.unwrap_err();
    assert_eq!(refused.to_string(), sheet.install_room);
    assert_eq!(row(t).install_error, sheet.install_room, "the sheet says why");
    assert!(t.torrents.started().is_empty());

    // With the DLC and the readme left out, it fits.
    pick(t, Some(2), false).await;
    pick(t, Some(3), false).await;
    assert_eq!(row(t).install_room, "");
    begin(t, true, None).await.unwrap();
    assert_eq!(t.torrents.started().len(), 1);
    assert_eq!(row(t).install_error, "");
    assert!(row(t).install_after);
}

#[tokio::test]
async fn the_sheet_warns_when_the_download_drive_is_short() {
    let s = setup_on(|h, home| h.disk(home.join("Downloads"), 4).disk(home.join("Games"), 5_000_000_000)).await;
    let sheet = row(&s.t);
    assert_eq!(sheet.download_free, "0.0 GB free");
    assert_eq!(sheet.install_free, "5.0 GB free");
    assert_eq!(sheet.download_room, "Not enough free space in the download folder: 0.0 GB needed, 0.0 GB free");
    assert_eq!(sheet.install_room, "");
}

#[tokio::test]
async fn download_and_install_installs_by_itself_once_the_download_finishes() {
    let s = setup().await;
    let t = &s.t;
    pick(t, Some(1), false).await;
    pick(t, Some(2), false).await;
    begin(t, true, None).await.unwrap();
    assert!(row(t).install_after);
    t.torrents.progress(HASH, 4);
    until(t, |r| r.state == "downloading").await;
    assert!(s.seen.sources.lock().unwrap().is_empty(), "nothing installs before the download ends");

    t.torrents.complete(HASH);
    let installed = until_install(t, "done").await;
    let sources = s.seen.sources.lock().unwrap().clone();
    assert_eq!(sources, [HostPath::new("packages", 0, RelPath::new(format!("PS5/torrent-{HASH}")).unwrap())]);
    // The installer saw the checked files only: no stub of the update or the DLC.
    assert_eq!(s.seen.files.lock().unwrap()[0], ["Astro/Astro.pkg", "Astro/readme.txt"]);
    assert_eq!(installed.error, "");
    assert!(t.core.view().notices.iter().any(|n| n.contains("Download complete: Astro. Installing it now.")));
}

#[tokio::test]
async fn a_failed_install_keeps_the_download_and_install_can_be_tried_again() {
    let s = setup().await;
    let t = &s.t;
    s.seen.fail.store(true, Ordering::SeqCst);
    begin(t, true, None).await.unwrap();
    t.torrents.complete(HASH);

    let failed = until_install(t, "failed").await;
    assert_eq!(failed.state, "complete", "the download stage is done");
    assert_eq!(failed.error, "the package is damaged", "the card says the install failed, and why");
    assert!(t.core.view().notices.iter().any(|n| n.contains("The install failed: the package is damaged")));
    assert_eq!(std::fs::read(s.folder().join("Astro/Astro.pkg")).unwrap(), GAME, "the downloaded files stay");
    assert!(failed.actions.contains(&"install".to_owned()), "{:?}", failed.actions);

    t.core.dispatch(Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: None })).await.unwrap();
    assert_eq!(until_install(t, "done").await.error, "");
    assert_eq!(s.seen.sources.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_game_folder_outside_the_game_folders_is_refused_before_anything_downloads() {
    let s = setup().await;
    let t = &s.t;
    let refused = begin(t, true, Some("/elsewhere")).await.unwrap_err();
    assert!(refused.to_string().contains("Pick a folder inside a game folder"), "{refused}");
    assert_eq!(row(t).install_error, refused.to_string());
    assert_eq!(row(t).state, "ready", "the sheet stays open");
    assert!(t.torrents.started().is_empty());
}

#[tokio::test]
async fn download_and_install_into_a_folder_the_player_typed_installs_there() {
    let s = setup().await;
    let t = &s.t;
    begin(t, true, Some("~/Games/Mine")).await.unwrap();
    t.torrents.complete(HASH);
    until_install(t, "done").await;
    assert_eq!(row(t).install_folder, "~/Games/Mine", "the next install offers it");
}

#[tokio::test]
async fn the_separate_steps_still_download_every_file_and_install_only_when_asked() {
    let s = setup().await;
    let t = &s.t;
    t.core.dispatch(Command::Torrent(TorrentCmd::Act { id: HASH.into(), action: TorrentAction::Start })).await.unwrap();
    assert_eq!(t.torrents.only(HASH), None, "every file");
    assert!(!row(t).install_after);
    t.torrents.complete(HASH);
    let done = until(t, |r| r.state == "complete").await;
    assert!(done.seeding, "a whole download shares");
    assert_eq!(listing(&s.folder()).len(), 4);
    assert!(s.seen.sources.lock().unwrap().is_empty(), "nothing installs by itself");

    t.core.dispatch(Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: None })).await.unwrap();
    until_install(t, "done").await;
}

#[tokio::test]
async fn a_download_with_files_left_out_resumes_with_the_same_files_after_a_restart() {
    let s = setup().await;
    let t = &s.t;
    pick(t, Some(3), false).await;
    begin(t, true, None).await.unwrap();
    t.torrents.progress(HASH, 4);
    until(t, |r| r.state == "downloading").await;

    let again = launcher_core::Core::start(t.kernel.clone()).await;
    let restored = again.view().downloads.torrents.into_iter().next().unwrap();
    assert_eq!(restored.state, "paused");
    assert!(restored.install_after, "it still installs once it finishes");
    again.dispatch(Command::Torrent(TorrentCmd::Act { id: HASH.into(), action: TorrentAction::Start })).await.unwrap();
    assert_eq!(t.torrents.only(HASH), Some(vec![0, 1, 2]), "the readme stays out");
    t.torrents.complete(HASH);
    until_in(&again, Area::Installs, |core| {
        core.view().downloads.torrents.first().is_some_and(|r| r.install == "done")
    })
    .await;
}

#[tokio::test]
async fn with_no_game_file_every_file_is_optional_and_one_must_stay_checked() {
    let s = setup_with(&[("Astro/Update/a.pkg", UPDATE), ("Astro/Update/b.pkg", DLC)], |h, _| h).await;
    let t = &s.t;
    assert!(row(t).picks.iter().all(|p| p.on && !p.locked));
    pick(t, None, false).await;
    let refused = begin(t, false, None).await.unwrap_err();
    assert_eq!(refused.to_string(), "Choose at least one file");
    assert!(t.torrents.started().is_empty());
}

#[tokio::test]
async fn a_skipped_archive_s_stub_doesn_t_unpack_and_the_game_s_archive_installs() {
    let files: &[(&str, &[u8])] = &[("Astro/game.zip", b"zip of the game"), ("Astro/update.zip", b"zip of the update")];
    let s = setup_with(files, |h, _| h).await;
    let t = &s.t;
    t.archives.holds("game.zip", &[("Astro.pkg", GAME)]);
    t.archives.fails("update.zip", "a stub is no archive");
    assert_eq!(row(t).picks.iter().map(|p| p.kind.as_str()).collect::<Vec<_>>(), ["game", "update"]);
    pick(t, Some(1), false).await;
    begin(t, true, None).await.unwrap();
    t.torrents.complete(HASH);

    let installed = until_install(t, "done").await;
    assert_eq!(installed.unpack, "unpacked");
    assert_eq!(installed.error, "");
    assert_eq!(s.seen.files.lock().unwrap()[0], ["Astro/game.zip", "unpacked/Astro/game/Astro.pkg"]);
}

#[tokio::test]
async fn an_install_that_can_t_start_after_the_download_says_why_and_keeps_the_files() {
    let s = setup().await;
    let t = &s.t;
    std::fs::create_dir_all(s.home.path().join("Other")).unwrap();
    begin(t, true, Some("~/Games/Mine")).await.unwrap();
    // The game folder goes while it downloads.
    let paths = vec![s.home.path().join("Other")];
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths }).await.unwrap();
    t.torrents.complete(HASH);

    let refused = until(t, |r| !r.install_error.is_empty()).await;
    assert!(refused.install_error.contains("Pick a folder inside a game folder"), "{refused:?}");
    assert_eq!(refused.error, refused.install_error, "the card says why");
    assert_eq!(refused.install, "", "nothing installs");
    let notice = t.core.view().notices.into_iter().find(|n| n.contains("downloaded, but the install didn't start"));
    assert!(notice.is_some(), "{:?}", t.core.view().notices);
    assert_eq!(std::fs::read(s.folder().join("Astro/Astro.pkg")).unwrap(), GAME, "the downloaded files stay");
    assert!(refused.actions.contains(&"install".to_owned()));
}

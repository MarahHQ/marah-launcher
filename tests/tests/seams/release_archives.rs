//! Seam 1, core app API: a finished download's archives are unpacked through the host's
//! archives port (a fake) before the emulator's game installer takes the download.

use launcher_api::*;
use launcher_core::{ArchiveKind, Area, Command, GameInstallCmd, TorrentAction};
use launcher_testkit::{FakeCatalog, FakeHost, TestCore};

use crate::common::serve_catalog;

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
struct Setup {
    t: TestCore,
    home: tempfile::TempDir,
    emulator: FakeCatalog,
}

/// The catalog offers one release, whose torrent holds `files`, to KytyPS5 with no games of its
/// own, whose game installer installs Astro.
async fn setup(files: &[(&str, &[u8])]) -> Setup {
    let emulator = FakeCatalog::kyty().installs("Astro");
    setup_with(|h| h.plugin(emulator.clone().build()), files, emulator.clone()).await
}

/// [`setup`] with the PS5 emulator `add` puts in; `emulator` is the one whose installs the
/// test reads. A test that adds a real plugin reads no installs, and its `emulator` goes
/// unused.
async fn setup_with(add: impl FnOnce(FakeHost) -> FakeHost, files: &[(&str, &[u8])], emulator: FakeCatalog) -> Setup {
    let home = tempfile::tempdir().unwrap();
    let t = add(FakeHost::new().home(home.path()))
        .built_in(PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    t.torrents.has(HASH, "Astro", files);
    let astro = serde_json::json!([{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
        "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
        "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05" } }]);
    serve_catalog(&t, &[("ps5", astro)]).await;
    Setup { t, home, emulator }
}

impl Setup {
    fn folder(&self) -> std::path::PathBuf {
        self.home.path().join(format!("Downloads/PS5/torrent-{HASH}"))
    }

    /// Downloads the release to the end.
    async fn finish(&self) {
        let offer: GameKey = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
        self.t
            .core
            .dispatch(Command::Torrent(launcher_core::TorrentCmd::Download {
                offer,
                release: HASH.into(),
                folder: None,
            }))
            .await
            .unwrap();
        until(&self.t.core, Area::Torrents, |r| r.state == "ready").await;
        act(&self.t.core, TorrentAction::Start).await;
        self.t.torrents.complete(HASH);
        until(&self.t.core, Area::Torrents, |r| r.state == "complete").await;
    }
}

fn row(core: &launcher_core::Core) -> launcher_ui_api::TorrentVm {
    core.view().downloads.torrents.into_iter().next().expect("a download")
}

/// Waits until the download's row passes `test`, looking after each change in `area`.
async fn until(
    core: &launcher_core::Core,
    area: Area,
    test: impl Fn(&launcher_ui_api::TorrentVm) -> bool,
) -> launcher_ui_api::TorrentVm {
    launcher_testkit::until(core, area, |core| core.view().downloads.torrents.first().is_some_and(&test)).await;
    row(core)
}

async fn act(core: &launcher_core::Core, action: TorrentAction) {
    core.dispatch(Command::Torrent(launcher_core::TorrentCmd::Act { id: HASH.into(), action })).await.unwrap();
}

async fn installs(core: &launcher_core::Core, command: GameInstallCmd) {
    core.dispatch(Command::GameInstall(command)).await.unwrap();
}

/// Installs the download into the game folder the install sheet shows.
async fn install(core: &launcher_core::Core) {
    installs(core, GameInstallCmd::Download { id: HASH.into(), folder: None }).await;
}

#[tokio::test]
async fn a_finished_download_unpacks_its_archive_and_install_takes_the_unpacked_files() {
    let s =
        setup(&[("Astro/game.part1.rar", b"one"), ("Astro/game.part2.rar", b"two"), ("Astro/readme.txt", b"hi")]).await;
    s.t.archives.holds("game.part1.rar", &[("Astro-app0/eboot.bin", b"elf")]);
    s.finish().await;

    let done = until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;
    assert_eq!(s.t.archives.unpacked(), [(ArchiveKind::Rar, vec!["game.part1.rar".into(), "game.part2.rar".into()])]);
    let file = s.folder().join("unpacked/Astro/game/Astro-app0/eboot.bin");
    assert_eq!(std::fs::read(file).unwrap(), b"elf");
    assert_eq!(done.actions, ["install", "stop-seeding", "remove-unpacked"]);
    assert_eq!(done.unpack_progress, 1.0);

    install(&s.t.core).await;
    until(&s.t.core, Area::Installs, |r| r.install == "done").await;
    let sources = s.emulator.installed_from();
    assert_eq!(sources, [HostPath::new("packages", 0, RelPath::new(format!("PS5/torrent-{HASH}")).unwrap())]);
}

#[tokio::test]
async fn several_archives_each_unpack_into_their_own_folder() {
    let s = setup(&[("Astro/game.rar", b"g"), ("Astro/update.rar", b"u"), ("dlc/game.7z", b"d")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    s.t.archives.holds("update.rar", &[("patch.pkg", b"update")]);
    s.t.archives.holds("game.7z", &[("addon.pkg", b"dlc")]);
    s.finish().await;

    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;
    let unpacked = s.folder().join("unpacked");
    assert_eq!(std::fs::read(unpacked.join("Astro/game/game.pkg")).unwrap(), b"game");
    assert_eq!(std::fs::read(unpacked.join("Astro/update/patch.pkg")).unwrap(), b"update");
    assert_eq!(std::fs::read(unpacked.join("dlc/game/addon.pkg")).unwrap(), b"dlc");
}

#[tokio::test]
async fn a_set_with_a_missing_part_fails_before_anything_is_unpacked() {
    let s = setup(&[("Astro/game.part1.rar", b"one"), ("Astro/game.part3.rar", b"three")]).await;
    s.finish().await;

    let failed = until(&s.t.core, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(failed.error.contains("part 2 of Astro/game is missing"), "{}", failed.error);
    assert!(s.t.archives.unpacked().is_empty(), "nothing unpacked");
    assert!(!s.folder().join("unpacked").exists());
}

#[tokio::test]
async fn an_unpack_that_fails_says_why_and_leaves_no_half_folder() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.fails("game.rar", "This archive needs a password");
    s.finish().await;

    let failed = until(&s.t.core, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(failed.error.contains("needs a password"), "{}", failed.error);
    assert_eq!(failed.actions, ["install", "stop-seeding"], "Install tries again");
    let left: Vec<_> = walk(&s.folder().join("unpacked"));
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn a_failed_unpack_removes_the_sets_it_already_unpacked() {
    let s = setup(&[("Astro/a.rar", b"a"), ("Astro/b.rar", b"b")]).await;
    s.t.archives.holds("a.rar", &[("a.pkg", b"a")]);
    s.t.archives.fails("b.rar", "b is damaged");
    s.finish().await;

    let failed = until(&s.t.core, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(failed.error.contains("b is damaged"), "{}", failed.error);
    assert!(walk(&s.folder().join("unpacked")).is_empty(), "{:?}", walk(&s.folder().join("unpacked")));
}

#[tokio::test]
async fn cancel_stops_the_unpacking_and_install_starts_it_again() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    let held = s.t.archives.hold();
    s.finish().await;

    let unpacking = until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacking").await;
    assert_eq!(unpacking.actions, ["stop-seeding", "cancel"], "Cancel stops the unpacking");
    act(&s.t.core, TorrentAction::Cancel).await;
    let cancelled = until(&s.t.core, Area::Torrents, |r| r.unpack == "cancelled").await;
    assert_eq!(cancelled.state, "complete", "the download stays");
    assert!(walk(&s.folder().join("unpacked")).is_empty());

    held.release();
    install(&s.t.core).await;
    until(&s.t.core, Area::Installs, |r| r.install == "done").await;
    assert_eq!(std::fs::read(s.folder().join("unpacked/Astro/game/game.pkg")).unwrap(), b"game");
    assert_eq!(s.emulator.installed_from().len(), 1, "installed once unpacked");
}

#[tokio::test]
async fn an_unpack_cut_by_a_restart_comes_back_failed_and_install_unpacks_first() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    let _held = s.t.archives.hold();
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacking").await;
    // The row says it unpacks before the unpacker is asked: let it get there.
    s.t.clock.run_until_idle().await;
    assert_eq!(s.t.archives.unpacked().len(), 1, "the first app's unpack runs");

    let again = launcher_core::Core::start(s.t.kernel.clone()).await;
    let restored = until(&again, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(restored.error.contains("stopped when Marah Launcher closed"), "{}", restored.error);
    // The download checks its files first, and seeds once they check out.
    until(&again, Area::Torrents, |r| r.seeding).await;
    // The first app closed: its unpack never ends, so only the new one writes the folder.
    s.t.archives.cut();
    install(&again).await;
    until(&again, Area::Installs, |r| r.install == "done").await;
    assert_eq!(std::fs::read(s.folder().join("unpacked/Astro/game/game.pkg")).unwrap(), b"game");
}

/// Install pressed at once after a restart, before the download's first poll and before its
/// check of the files ends, unpacks and installs: nothing in the restart holds the folder.
#[tokio::test]
async fn install_right_after_a_restart_cut_an_unpack_unpacks_and_installs() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    let _held = s.t.archives.hold();
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacking").await;
    s.t.clock.run_until_idle().await;
    // The first app closed: its unpack never ends.
    s.t.archives.cut();

    let again = launcher_core::Core::start(s.t.kernel.clone()).await;
    assert!(!row(&again).seeding, "the check of its files hasn't ended: {:?}", row(&again));
    install(&again).await;
    until(&again, Area::Installs, |r| r.install == "done").await;
    assert_eq!(std::fs::read(s.folder().join("unpacked/Astro/game/game.pkg")).unwrap(), b"game");
    assert_eq!(s.emulator.installed_from().len(), 1, "installed once unpacked");
}

#[tokio::test]
async fn remove_unpacked_files_keeps_the_download_and_its_seeding() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;

    installs(&s.t.core, GameInstallCmd::RemoveUnpacked { id: HASH.into() }).await;
    let after = row(&s.t.core);
    assert_eq!(after.unpack, "waiting");
    assert!(!s.folder().join("unpacked").exists());
    assert_eq!(std::fs::read(s.folder().join("Astro/game.rar")).unwrap(), b"g", "the torrent's files stay");
    assert!(after.seeding);
}

#[tokio::test]
async fn a_download_with_no_archives_installs_as_before() {
    let s = setup(&[("Astro/astro.pkg", b"package")]).await;
    s.finish().await;
    let done = row(&s.t.core);
    assert_eq!(done.unpack, "");
    assert_eq!(done.actions, ["install", "stop-seeding"]);
}

/// Every file under `dir`.
fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

// Unix only: "moved, not copied" compares inode numbers.
#[cfg(unix)]
#[tokio::test]
async fn installing_the_unpacked_game_moves_it_into_the_games_folder_and_the_archive_stays() {
    use std::os::unix::fs::MetadataExt;
    let s = setup_with(
        |h| h.built_in(PluginBox::code(emu_kyty::KytyPlugin)),
        &[("Astro/game.7z", b"seven")],
        FakeCatalog::kyty(),
    )
    .await;
    let param = br#"{"titleId":"PPSA01234","localizedParameters":{"en-US":{"titleName":"Astro"}}}"#;
    s.t.archives
        .holds("game.7z", &[("PPSA01234-app0/sce_sys/param.json", param), ("PPSA01234-app0/eboot.bin", b"elf")]);
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;
    let unpacked = s.folder().join("unpacked/Astro/game/PPSA01234-app0");
    let inode = std::fs::metadata(unpacked.join("eboot.bin")).unwrap().ino();

    install(&s.t.core).await;
    let done = until(&s.t.core, Area::Installs, |r| r.install == "done").await;
    let installed = s.home.path().join("Games/PS5/PPSA01234-app0/eboot.bin");
    assert_eq!(std::fs::read(&installed).unwrap(), b"elf");
    assert_eq!(std::fs::metadata(&installed).unwrap().ino(), inode, "moved, not copied");
    assert!(!unpacked.exists(), "the unpacked copy moved");
    assert_eq!(std::fs::read(s.folder().join("Astro/game.7z")).unwrap(), b"seven", "the download stays, for sharing");
    assert!(!done.actions.contains(&"remove-unpacked".to_owned()), "nothing unpacked is left: {:?}", done.actions);
}

//! The PS4 system files: the start asks before it installs the missing ones into shadPS4's data
//! folder, and the System files block on the PS4 page shows them and installs them at once.
//! The tests use a short list of fake files, served by the fake network: never Sony's files.

use std::path::Path;

use emu_shadps4::ShadWithFiles;
use emu_shadps4::system_files::{SystemFile, url};
use launcher_api::*;
use launcher_core::{Area, Command, LibraryCmd, Route};
use launcher_testkit::{FakeHost, FakeHttp, TestCore};

/// The fake list: a module and a font in each font folder.
const FILES: [(&str, &[u8]); 3] = [
    ("sys_modules/libSceFont.sprx", b"a fake module"),
    ("fonts/font/SST-Roman.otf", b"a fake font"),
    ("fonts/font2/PS4Icon.ttf", b"a fake second font"),
];

fn shad() -> PluginId {
    PluginId::new("emu-shadps4").unwrap()
}

/// The SHA-256 of `bytes`, as the fake network gives it for a file it serves.
fn sha(bytes: &[u8]) -> String {
    FakeHttp::default().serve("https://example.com/", bytes.to_vec())
}

/// The fake list, as the plugin reads it.
fn table() -> Vec<SystemFile> {
    FILES.iter().map(|(path, bytes)| SystemFile::new(path, bytes.len() as u64, &sha(bytes))).collect()
}

/// A host with the PS4 plugin on the fake list, and the fake files served.
fn host(plugin_files: Vec<SystemFile>) -> FakeHost {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    for (path, bytes) in FILES {
        host.http().serve(&url(path), bytes.to_vec());
    }
    host.built_in(PluginBox::code(ShadWithFiles(plugin_files)))
}

/// Starts the app with shadPS4's data folder in a temporary folder, after `prepare` put files
/// in it, and runs the boot.
async fn start(host: FakeHost, prepare: impl FnOnce(&Path)) -> (TestCore, tempfile::TempDir) {
    start_saved(host, None, prepare).await
}

/// [`start`], with the System files settings an earlier run saved.
async fn start_saved(
    host: FakeHost,
    settings: Option<serde_json::Value>,
    prepare: impl FnOnce(&Path),
) -> (TestCore, tempfile::TempDir) {
    let test = host.boot_core().await;
    if let Some(settings) = settings {
        let storage = test.host.data_dir().join("plugins/emu-shadps4");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(storage.join("system-files-settings.json"), settings.to_string()).unwrap();
    }
    let data = tempfile::tempdir().unwrap();
    prepare(data.path());
    test.core
        .dispatch(Command::SetFolders { name: "shadps4-data".into(), paths: vec![data.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();
    (test, data)
}

fn put(data: &Path, path: &str, bytes: &[u8]) {
    let file = data.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, bytes).unwrap();
}

/// The downloads of system files the plugin asked for.
fn downloads(test: &TestCore) -> Vec<String> {
    test.http.sent().into_iter().filter(|u| u.contains("Abdess/retrobios")).collect()
}

/// Every file under `dir`, relative, sorted.
fn tree(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                found.push(path.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    found
}

/// The question the start asks.
async fn asked(test: &TestCore) -> launcher_ui_api::QuestionVm {
    test.until(Area::Library, |core| core.view().question.is_some()).await;
    test.core.view().question.unwrap()
}

/// The rows of the System files block on the PS4 page.
fn rows(core: &launcher_core::Core) -> Option<Vec<launcher_ui_api::RowVm>> {
    let view = core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == shad())?.clone();
    emu.panels.into_iter().find(|p| p.id == "system-files").map(|p| p.rows)
}

fn row<'a>(rows: &'a [launcher_ui_api::RowVm], label: &str) -> Option<&'a launcher_ui_api::RowVm> {
    rows.iter().find(|r| r.label == label)
}

/// The block's Status row's value, such as "Missing 3 of 3".
fn status(test: &TestCore) -> String {
    row(&rows(&test.core).expect("a System files block"), "Status").expect("a Status row").value.clone()
}

/// Waits until an install ended: the block has its result row, and no install runs. Returns
/// the result's hint, with "failed: " before it when it failed.
async fn settled(test: &TestCore) -> String {
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let ended = |rows: &[launcher_ui_api::RowVm]| {
        row(rows, "Installing").is_none()
            && (row(rows, "Last install").is_some() || row(rows, "The last install failed").is_some())
    };
    test.until(Area::Plugins, |core| rows(core).is_some_and(|r| ended(&r))).await;
    let rows = rows(&test.core).unwrap();
    match row(&rows, "The last install failed") {
        Some(failed) => format!("failed: {}", failed.hint),
        None => row(&rows, "Last install").unwrap().hint.clone(),
    }
}

/// The boot screen's PS4 system files step, once the boot is over.
async fn boot_step(test: &TestCore) -> launcher_ui_api::BootStepVm {
    test.until(Area::Library, |core| core.view().boot.ready).await;
    test.core.view().boot.steps.into_iter().find(|s| s.label == "PS4 system files").expect("a system files step")
}

#[tokio::test]
async fn the_start_asks_before_it_installs_the_missing_system_files() {
    let (test, data) = start(host(table()), |data| {
        // shadPS4 makes these folders, empty, at its first run.
        for folder in ["sys_modules", "fonts/font", "fonts/font2"] {
            std::fs::create_dir_all(data.join(folder)).unwrap();
        }
    })
    .await;

    let question = asked(&test).await;

    assert_eq!(question.title, "Install PS4 system files?");
    assert_eq!((question.yes.as_str(), question.no.as_str()), ("Install", "Not now"));
    assert!(question.text.contains("system modules and fonts"), "{}", question.text);
    assert!(question.text.contains("the 3 files, about 0.0 MB"), "{}", question.text);
    assert!(question.text.contains("retrobios collection on GitHub"), "{}", question.text);
    let boot = test.core.view().boot;
    assert!(!boot.ready && boot.installing, "the boot screen waits for the answer");
    assert!(downloads(&test).is_empty(), "nothing downloads before the answer: {:?}", downloads(&test));
    assert!(tree(data.path()).is_empty());
}

/// Install: each file downloads, is checked and moves into place; the boot screen shows it and
/// waits for it.
#[tokio::test]
async fn install_puts_each_checked_file_in_place_and_the_boot_screen_waits_for_it() {
    let host = host(table());
    let gate = host.http().hold_next(&url(FILES[1].0));
    let (test, data) = start(host, |_| {}).await;
    asked(&test).await;

    test.core.dispatch(Command::AnswerQuestion(true)).await.unwrap();

    let step = |boot: &launcher_ui_api::BootVm| boot.steps.iter().find(|s| s.label == "PS4 system files").cloned();
    test.until(Area::Library, |core| step(&core.view().boot).is_some_and(|s| s.state == "working")).await;
    let boot = test.core.view().boot;
    assert!(!boot.ready && boot.installing, "it waits for the system files");
    assert!(step(&boot).unwrap().detail.starts_with("Downloading"), "{:?}", step(&boot));
    gate.release();
    let done = boot_step(&test).await;
    assert_eq!((done.detail.as_str(), done.state.as_str()), ("Installed", "done"));

    assert_eq!(settled(&test).await, "Installed 3 of the 3 files");
    for (path, bytes) in FILES {
        assert_eq!(std::fs::read(data.path().join(path)).unwrap(), bytes, "{path}");
    }
    assert_eq!(tree(data.path()).len(), 3, "no staging left behind: {:?}", tree(data.path()));
    assert_eq!(status(&test), "Installed");
    let rows = rows(&test.core).unwrap();
    assert!(row(&rows, "Install now").is_none(), "nothing left to install");
    // The marker names the commit and the files the plugin installed.
    let marker = std::fs::read(test.host.data_dir().join("plugins/emu-shadps4/system-files.json")).unwrap();
    let marker: serde_json::Value = serde_json::from_slice(&marker).unwrap();
    assert_eq!(marker["commit"], emu_shadps4::system_files::COMMIT);
    assert_eq!(marker["files"].as_array().map(Vec::len), Some(3));
}

/// Not now: nothing downloads, and the question doesn't come back in this run.
#[tokio::test]
async fn not_now_downloads_nothing_and_the_block_says_what_is_missing() {
    let (test, data) = start(host(table()), |_| {}).await;
    asked(&test).await;

    test.core.dispatch(Command::AnswerQuestion(false)).await.unwrap();
    test.until(Area::Library, |core| core.view().boot.ready).await;
    test.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();

    assert!(test.core.view().question.is_none(), "asked once a run");
    assert!(downloads(&test).is_empty(), "{:?}", downloads(&test));
    assert!(tree(data.path()).is_empty());
    assert!(!test.core.view().boot.steps.iter().any(|s| s.label == "PS4 system files"));
    assert_eq!(status(&test), "Missing 3 of 3");
    assert!(row(&rows(&test.core).unwrap(), "Install now").is_some());
}

/// Install now on the PS4 page installs at once, also after Not now.
#[tokio::test]
async fn install_now_on_the_ps4_page_installs_at_once() {
    let (test, data) = start(host(table()), |_| {}).await;
    asked(&test).await;
    test.core.dispatch(Command::AnswerQuestion(false)).await.unwrap();
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();

    let action = "install-system-files".to_owned();
    test.core
        .dispatch(Command::RunAction { plugin: shad(), action, game: None, args: serde_json::Value::Null })
        .await
        .unwrap();

    assert_eq!(settled(&test).await, "Installed 3 of the 3 files");
    for (path, bytes) in FILES {
        assert_eq!(std::fs::read(data.path().join(path)).unwrap(), bytes, "{path}");
    }
    assert_eq!(status(&test), "Installed");
}

/// A listed file the player put there stays as it is, even when it differs from the list's;
/// only the missing files are offered and installed. Files not on the list stay too.
#[tokio::test]
async fn the_player_s_own_files_stay_and_only_the_missing_ones_install() {
    let (test, data) = start(host(table()), |data| {
        put(data, FILES[0].0, b"the player's own module");
        put(data, "sys_modules/libSceOther.sprx", b"not on the list");
        put(data, "fonts/font/notes.txt", b"not on the list");
    })
    .await;

    let question = asked(&test).await;
    assert!(question.text.contains("the 2 missing files"), "{}", question.text);
    test.core.dispatch(Command::AnswerQuestion(true)).await.unwrap();

    assert_eq!(settled(&test).await, "Installed 2 of the 3 files");
    assert_eq!(std::fs::read(data.path().join(FILES[0].0)).unwrap(), b"the player's own module");
    assert_eq!(std::fs::read(data.path().join("sys_modules/libSceOther.sprx")).unwrap(), b"not on the list");
    assert_eq!(std::fs::read(data.path().join("fonts/font/notes.txt")).unwrap(), b"not on the list");
    for (path, bytes) in &FILES[1..] {
        assert_eq!(std::fs::read(data.path().join(path)).unwrap(), *bytes, "{path}");
    }
    assert!(!downloads(&test).contains(&url(FILES[0].0)), "the player's file isn't downloaded");
    assert_eq!(status(&test), "Your own files");
}

/// With every listed file there, the start asks nothing.
#[tokio::test]
async fn with_every_file_there_the_start_asks_nothing() {
    let (test, _data) = start(host(table()), |data| {
        for (path, bytes) in FILES {
            put(data, path, bytes);
        }
    })
    .await;

    test.until(Area::Library, |core| core.view().boot.ready).await;
    assert!(test.core.view().question.is_none());
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(status(&test), "Your own files");
    assert!(downloads(&test).is_empty());
}

/// A download whose SHA-256 isn't the list's ends the install: the boot screen and the block
/// say so, the boot goes on, and the file isn't in place, not even in part.
#[tokio::test]
async fn a_download_with_the_wrong_sha_256_is_a_failure_and_nothing_half_written_stays() {
    let mut files = table();
    files[1].sha256 = "0".repeat(64);
    let (test, data) = start(host(files), |_| {}).await;
    asked(&test).await;

    test.core.dispatch(Command::AnswerQuestion(true)).await.unwrap();

    let step = boot_step(&test).await;
    assert_eq!((step.detail.as_str(), step.state.as_str()), ("Could not install", "failed"));
    let failed = settled(&test).await;
    assert!(failed.starts_with("failed: ") && failed.contains("SHA-256"), "{failed}");
    assert_eq!(tree(data.path()), [FILES[0].0], "the first file, checked, stays; nothing else");
    assert_eq!(status(&test), "Missing 2 of 3");
    assert!(row(&rows(&test.core).unwrap(), "Install now").is_some());
}

/// A file that isn't on GitHub (404) or a wrong size ends the install the same way.
#[tokio::test]
async fn a_missing_file_or_a_wrong_size_is_a_failure_and_nothing_half_written_stays() {
    let mut not_served = table();
    not_served.push(SystemFile::new("fonts/font2/Gone.otf", 4, &"0".repeat(64)));
    let mut wrong_size = table();
    wrong_size[0].size += 1;
    for (files, why) in [(not_served, "404"), (wrong_size, "bytes")] {
        let (test, data) = start(host(files), |_| {}).await;
        asked(&test).await;
        test.core.dispatch(Command::AnswerQuestion(true)).await.unwrap();

        assert_eq!(boot_step(&test).await.state, "failed", "{why}");
        let failed = settled(&test).await;
        assert!(failed.starts_with("failed: ") && failed.contains(why), "{failed}");
        let left = tree(data.path());
        assert!(!left.iter().any(|f| f.contains(".part") || f.contains("Gone")), "{why}: {left:?}");
    }
}

/// The Switch firmware and the PS4 system files both ask in one start: one after the other.
#[tokio::test]
async fn the_switch_firmware_and_the_ps4_system_files_are_both_asked_one_after_the_other() {
    let host = host(table()).built_in(PluginBox::code(emu_lichii::LichiiPlugin));
    // The firmware release the Lichii check finds: it asks to install it.
    let release = serde_json::json!({ "tag_name": "22.5.0", "published_at": "2026-11-10T01:55:37Z", "assets": [
        { "name": "Firmware.22.5.0.zip", "browser_download_url": "https://github.com/THZoria/NX_Firmware/releases/download/22.5.0/Firmware.22.5.0.zip",
          "size": 3, "digest": format!("sha256:{}", "0".repeat(64)) },
    ]});
    host.http().serve(&emu_lichii::release_url("22.5.0"), serde_json::to_vec(&release).unwrap());
    let test = host.boot_core().await;
    let (ps4, switch) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for (name, dir) in [("shadps4-data", &ps4), ("lichii-data", &switch)] {
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![dir.path().into()] }).await.unwrap();
    }
    test.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();

    let first = asked(&test).await;
    test.core.dispatch(Command::AnswerQuestion(false)).await.unwrap();
    test.until(Area::Library, |core| core.view().question.as_ref().is_some_and(|q| q.title != first.title)).await;
    let second = test.core.view().question.unwrap();
    assert!(!test.core.view().boot.ready, "the boot screen waits for the second answer");
    test.core.dispatch(Command::AnswerQuestion(false)).await.unwrap();
    test.until(Area::Library, |core| core.view().boot.ready).await;

    let mut titles = [first.title, second.title];
    titles.sort();
    assert_eq!(titles, ["Install PS4 system files?", "Install Switch firmware 22.5.0?"]);
    assert!(test.core.view().question.is_none());
}

/// With "Ask at start" off, the start asks nothing and downloads nothing. The block still says
/// what is missing, and its switch turns asking back on.
#[tokio::test]
async fn with_ask_at_start_off_the_start_asks_nothing() {
    let (test, data) = start_saved(host(table()), Some(serde_json::json!({ "ask": false })), |_| {}).await;
    test.until(Area::Library, |core| core.view().boot.ready).await;

    assert!(test.core.view().question.is_none(), "no question with Ask at start off");
    assert!(downloads(&test).is_empty(), "{:?}", downloads(&test));
    assert!(tree(data.path()).is_empty());
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    test.until(Area::Plugins, |core| rows(core).is_some()).await;
    assert_eq!(status(&test), "Missing 3 of 3");
    let block = rows(&test.core).unwrap();
    assert_eq!(row(&block, "Ask at start").expect("an Ask at start switch").switch, "off");
    assert!(row(&block, "Install now").is_some(), "Install now still works");

    let action = "toggle-system-files-ask".to_owned();
    test.core
        .dispatch(Command::RunAction { plugin: shad(), action, game: None, args: serde_json::Value::Null })
        .await
        .unwrap();
    test.until(Area::Plugins, |core| {
        rows(core).is_some_and(|r| row(&r, "Ask at start").is_some_and(|s| s.switch == "on"))
    })
    .await;
    let saved = std::fs::read(test.host.data_dir().join("plugins/emu-shadps4/system-files-settings.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(saved["ask"], true);
}

/// By default the block's switch is on: the start asks.
#[tokio::test]
async fn ask_at_start_is_on_by_default() {
    let (test, _data) = start(host(table()), |_| {}).await;
    asked(&test).await;
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    test.until(Area::Plugins, |core| rows(core).is_some()).await;
    let rows = rows(&test.core).unwrap();
    assert_eq!(row(&rows, "Ask at start").expect("an Ask at start switch").switch, "on");
}

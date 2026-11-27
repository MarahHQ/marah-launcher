//! Seam 1, core app API: an emulator's system files (`[system_files]`), which the player imports
//! from one archive on the emulator's page. The page's block says whether they are in place,
//! its Import… row opens the folder browser on archives, and the pick imports: each part of the
//! archive goes where the manifest says, or nothing changes and the block says why.

use std::path::Path;

use launcher_api::*;
use launcher_core::{Command, Route};
use launcher_testkit::{FakeHost, StubPlugin, TestCore};

/// An emulator whose firmware replaces its folder in `emu-data`, and whose keys and BIOS files
/// join the plugin's data folder and `emu-data`.
fn emu(optional: bool) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"emu\"\nkind = \"emulator\"\nname = \"Emu\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         [[permissions.filesystem]]\nfolder = \"emu-data\"\nwrite = true\n\
         [system_files]\nname = \"Emu firmware and keys\"\narchive = \"emu-files.zip\"\noptional = {optional}\n\
         [[system_files.part]]\nfrom = \"firmware\"\nfolder = \"emu-data\"\ninto = \"nand/registered\"\n\
         files = [\"*.nca\"]\nreplace = true\n\
         [[system_files.part]]\nfrom = \"keys\"\nfolder = \"emu-data\"\ninto = \"keys\"\nrequired = [\"prod.keys\"]\n\
         [[system_files.part]]\nfrom = \"system\"\ninto = \"system\"\n"
    ))
}

fn emu_id() -> PluginId {
    PluginId::new("emu").unwrap()
}

/// A core with [`emu`], its `emu-data` folder in a temporary folder, and a Downloads folder in
/// the home folder with the archive `emu-files.zip`, a 7z, and a file that is no archive.
async fn boot(host: FakeHost, optional: bool) -> (TestCore, tempfile::TempDir, tempfile::TempDir) {
    let (home, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let downloads = home.path().join("Downloads");
    std::fs::create_dir_all(downloads.join("Old")).unwrap();
    for name in ["emu-files.zip", "other.7z", "notes.txt"] {
        std::fs::write(downloads.join(name), b"archive").unwrap();
    }
    let t = host.home(home.path()).plugin(emu(optional)).boot_core().await;
    let folders = Command::SetFolders { name: "emu-data".into(), paths: vec![data.path().into()] };
    t.core.dispatch(folders).await.unwrap();
    (t, home, data)
}

/// The archive's files: firmware, keys, BIOS, and others the parts don't take.
const GOOD: &[(&str, &[u8])] = &[
    ("firmware/a.nca", b"new a"),
    ("firmware/VERSION", b"22.5.0"),
    ("keys/prod.keys", b"prod"),
    ("keys/title.keys", b"title"),
    ("system/scph5501.bin", b"bios"),
    ("README.md", b"how to"),
];

async fn open_page(t: &TestCore) {
    t.core.dispatch(Command::Navigate(Route::Plugin(emu_id()))).await.unwrap();
}

/// The System files block on the page: its rows, as label and value, then the last import.
fn block(t: &TestCore) -> launcher_ui_api::SlotVm {
    let view = t.core.view();
    let page = view.plugin_page.expect("the page");
    page.blocks.into_iter().find(|b| b.id == "system-files").expect("the System files block")
}

fn row(block: &launcher_ui_api::SlotVm, label: &str) -> launcher_ui_api::RowVm {
    block.rows.iter().find(|r| r.label == label).cloned().unwrap_or_else(|| panic!("no {label} row"))
}

async fn import(t: &TestCore, file: &Path) -> launcher_core::CoreResult<()> {
    t.core.dispatch(Command::ImportSystemFiles { plugin: emu_id(), file: file.to_path_buf() }).await
}

fn read(dir: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(rel)).ok()
}

#[tokio::test]
async fn import_opens_the_browser_on_archives_and_the_pick_puts_each_part_in_its_place() {
    let host = FakeHost::new();
    host.archives().holds("emu-files.zip", GOOD);
    let (t, home, data) = boot(host, false).await;
    std::fs::create_dir_all(data.path().join("nand/registered")).unwrap();
    std::fs::write(data.path().join("nand/registered/old.nca"), b"old").unwrap();
    std::fs::create_dir_all(data.path().join("keys")).unwrap();
    std::fs::write(data.path().join("keys/mine.txt"), b"the player's").unwrap();
    open_page(&t).await;
    let before = block(&t);
    assert_eq!(before.text, "Emu firmware and keys");
    assert_eq!((row(&before, "Status").value.as_str(), row(&before, "Status").tone), ("Missing", Tone::Warning));
    let pick = row(&before, "Import…");
    assert_eq!(pick.action, HOST_IMPORT_SYSTEM_FILES);
    assert!(pick.hint.contains("emu-files.zip"), "{}", pick.hint);

    let run = Command::RunAction { plugin: emu_id(), action: pick.action, game: None, args: serde_json::Value::Null };
    t.core.dispatch(run).await.unwrap();
    let browser = t.core.view().browser;
    assert_eq!((browser.purpose.as_str(), browser.folder.as_str()), ("system-files", "emu"));
    assert_eq!(browser.title, "Import Emu firmware and keys");
    let downloads = home.path().join("Downloads");
    assert_eq!(browser.shown, "~/Downloads", "it opens at the Downloads folder");
    let entries: Vec<(String, bool)> = browser.entries.iter().map(|e| (e.name.clone(), e.folder)).collect();
    assert_eq!(
        entries,
        [("Old".to_owned(), true), ("emu-files.zip".to_owned(), false), ("other.7z".to_owned(), false)],
        "folders, then archives; no other file"
    );
    assert_eq!(browser.entries[1].size, "0 MB", "an archive shows its size");

    import(&t, &downloads.join("emu-files.zip")).await.unwrap();

    assert!(!t.core.view().browser.open, "the pick closes the browser");
    assert_eq!(read(data.path(), "nand/registered/a.nca").as_deref(), Some("new a"));
    assert_eq!(read(data.path(), "nand/registered/old.nca"), None, "the firmware is replaced whole");
    assert_eq!(read(data.path(), "nand/registered/VERSION"), None, "only the files the part takes");
    assert_eq!(read(data.path(), "keys/prod.keys").as_deref(), Some("prod"));
    assert_eq!(read(data.path(), "keys/mine.txt").as_deref(), Some("the player's"), "the player's own files stay");
    let plugin_dir = t.data_dir().join("plugins/emu");
    assert_eq!(read(&plugin_dir, "system/scph5501.bin").as_deref(), Some("bios"), "a part without folder");
    assert!(!t.data_dir().join("tmp/system-files/emu").exists(), "the work folder goes");
    let after = block(&t);
    assert_eq!((row(&after, "Status").value.as_str(), row(&after, "Status").tone), ("Installed", Tone::Success));
    assert_eq!(row(&after, "Last import").hint, "4 files imported");
    assert!(t.core.view().notices.iter().any(|n| n == "Emu firmware and keys: 4 files imported"));
}

#[tokio::test]
async fn a_wrong_archive_changes_nothing_and_the_block_says_what_it_should_hold() {
    let host = FakeHost::new();
    host.archives().holds("other.7z", &[("firmware/a.nca", b"a"), ("system/bios.bin", b"b")]);
    host.archives().fails("emu-files.zip", "the archive is cut short");
    let (t, home, data) = boot(host, false).await;
    std::fs::create_dir_all(data.path().join("nand/registered")).unwrap();
    std::fs::write(data.path().join("nand/registered/old.nca"), b"old").unwrap();
    open_page(&t).await;

    let refused = import(&t, &home.path().join("Downloads/other.7z")).await.unwrap_err().to_string();
    assert!(refused.contains("This isn't emu-files.zip: it has no keys/ folder"), "{refused}");
    assert_eq!(read(data.path(), "nand/registered/old.nca").as_deref(), Some("old"), "nothing changed");
    let last = row(&block(&t), "Last import");
    assert_eq!(last.tone, Tone::Danger);
    assert!(last.hint.contains("It should hold firmware/, keys/, system/"), "{}", last.hint);

    let damaged = import(&t, &home.path().join("Downloads/emu-files.zip")).await.unwrap_err().to_string();
    assert!(damaged.contains("the archive is cut short"), "{damaged}");
    assert_eq!(read(data.path(), "nand/registered/old.nca").as_deref(), Some("old"), "nothing changed");
    let notes = import(&t, &home.path().join("Downloads/notes.txt")).await.unwrap_err().to_string();
    assert!(notes.contains("isn't an archive: pick emu-files.zip"), "{notes}");
}

#[tokio::test]
async fn optional_system_files_that_are_missing_bring_no_warning() {
    let (t, _home, _data) = boot(FakeHost::new(), true).await;
    open_page(&t).await;
    let status = row(&block(&t), "Status");
    assert_eq!((status.value.as_str(), status.tone), ("Not installed", Tone::Neutral));
    assert_eq!(status.hint, "Most games play without them; some need them");
}

#[tokio::test]
async fn a_part_that_writes_where_the_plugin_may_only_read_is_refused() {
    let plugin = StubPlugin::from_toml(
        "id = \"sneaky\"\nkind = \"emulator\"\nname = \"S\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"\n\
         [system_files]\nname = \"S files\"\narchive = \"s.zip\"\n\
         [[system_files.part]]\nfrom = \"x\"\nfolder = \"games\"\ninto = \"x\"\n",
    );
    let t = FakeHost::new().plugin(plugin).boot_core().await;
    let refused = t.report().rejection(&PluginId::new("sneaky").unwrap()).expect("refused").to_string();
    assert!(refused.contains("folder \"games\" must be a [[permissions.filesystem]] folder with write"), "{refused}");
}

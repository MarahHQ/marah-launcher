//! Seam 1, core app API: an emulator that plays several consoles (`[[consoles]]`). Each console
//! that is on has its Home entry, its games, its catalog and its system files; the player turns
//! each on or off with the switches of the plugin page's Consoles block, and the change shows at
//! once.

use std::sync::{Arc, Mutex};

use launcher_api::*;
use launcher_core::{Command, Route};
use launcher_testkit::{FakeHost, StubPlugin, TestCore};

/// Three consoles: the NES, with keys and BIOS files; the Genesis, with BIOS files; and the
/// Atari 2600, off until the player turns it on.
const RETRO: &str = "id = \"retro\"\nkind = \"emulator\"\nname = \"Retro\"\nemulator = \"RetroArch\"\n\
    version = \"1.0.0\"\nhost_api = \"^0.7\"\ncapabilities = [\"catalog\", \"launcher\"]\n\
    [[consoles]]\nconsole = \"NES\"\ninstall_note = \"Installs an NES game.\"\n\
    [consoles.system]\nid = \"NES\"\nname = \"Nintendo Entertainment System\"\n\
    [[consoles.keymap]]\nbutton = \"A\"\nkeys = \"X\"\n\
    [consoles.system_files]\nname = \"NES BIOS files\"\narchive = \"nes-bios.zip\"\noptional = true\n\
    [[consoles.system_files.part]]\nfrom = \"system\"\ninto = \"system\"\nfiles = [\"disksys.rom\"]\n\
    [[consoles]]\nconsole = \"GENESIS\"\n\
    [consoles.system]\nid = \"GENESIS\"\nname = \"Sega Genesis\"\n\
    [consoles.system_files]\nname = \"Genesis BIOS files\"\narchive = \"genesis-bios.zip\"\noptional = true\n\
    [[consoles.system_files.part]]\nfrom = \"system\"\ninto = \"system\"\nfiles = [\"bios_MD.bin\"]\n\
    [[consoles]]\nconsole = \"ATARI2600\"\non = false\n\
    [consoles.system]\nid = \"ATARI2600\"\nname = \"Atari 2600\"\n";

/// One game of each console.
struct Games;

#[async_trait]
impl Catalog for Games {
    async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
        let game = |id: &str, title: &str, platform: &str| GameSummary {
            id: id.into(),
            title: title.into(),
            platform: platform.into(),
            cover: None,
            tags: vec![],
            release_year: None,
            title_id: None,
            content_id: None,
        };
        let items = vec![
            game("NES/Contra", "Contra", "NES"),
            game("GENESIS/Sonic", "Sonic", "GENESIS"),
            game("ATARI2600/Pitfall", "Pitfall", "ATARI2600"),
        ];
        Ok(Page { items, total: Some(3) })
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
impl Launcher for Games {
    async fn prepare(&self, game: &str, _: &serde_json::Value) -> PluginResult<LaunchSpec> {
        Err(PluginError::NotFound(game.into()))
    }
}

fn retro() -> StubPlugin {
    StubPlugin::from_toml(RETRO).provides(|_| {
        Ok(Provided { catalog: Some(Arc::new(Games)), launcher: Some(Arc::new(Games)), ..Provided::default() })
    })
}

fn retro_id() -> PluginId {
    PluginId::new("retro").unwrap()
}

/// A catalog service that keeps the consoles the host asks it for.
fn recording_catalog(asked: Arc<Mutex<Vec<String>>>) -> StubPlugin {
    struct Asked(Arc<Mutex<Vec<String>>>);
    #[async_trait]
    impl Releases for Asked {
        async fn releases(&self, console: &Console) -> PluginResult<Option<ReleaseList>> {
            self.0.lock().unwrap().push(console.to_string());
            Ok(None)
        }
    }
    StubPlugin::from_toml(
        "id = \"catalogs\"\nkind = \"service\"\nname = \"Catalogs\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         capabilities = [\"releases\"]",
    )
    .provides(move |_| Ok(Provided { releases: Some(Arc::new(Asked(asked.clone()))), ..Provided::default() }))
}

fn home(t: &TestCore) -> Vec<String> {
    t.core.view().emulators.emulators.iter().map(|e| e.system.clone()).collect()
}

fn titles(t: &TestCore) -> Vec<String> {
    let mut titles: Vec<String> = t.core.view().home.library.games.iter().map(|g| g.title.clone()).collect();
    titles.sort();
    titles
}

async fn open_page(t: &TestCore) {
    t.core.dispatch(Command::Navigate(Route::Plugin(retro_id()))).await.unwrap();
}

fn block(t: &TestCore, id: &str) -> Option<launcher_ui_api::SlotVm> {
    t.core.view().plugin_page?.blocks.into_iter().find(|b| b.id == id)
}

/// The Consoles block's switches: each console's name and whether it is on.
fn switches(t: &TestCore) -> Vec<(String, String)> {
    let block = block(t, "consoles").expect("the Consoles block");
    block.rows.iter().map(|r| (r.label.clone(), r.switch.clone())).collect()
}

/// Flips the Consoles block's switch of `name`, as the player does.
async fn flip(t: &TestCore, name: &str) {
    let block = block(t, "consoles").expect("the Consoles block");
    let row = block.rows.iter().find(|r| r.label == name).unwrap_or_else(|| panic!("no switch {name}"));
    assert_eq!(row.action, HOST_SET_CONSOLE);
    let run = Command::RunAction { plugin: retro_id(), action: row.action.clone(), game: None, args: row.args.clone() };
    t.core.dispatch(run).await.unwrap();
}

fn on(name: &str, on: bool) -> (String, String) {
    (name.into(), if on { "on" } else { "off" }.into())
}

#[tokio::test]
async fn each_console_that_is_on_has_its_home_entry_and_its_games_and_a_switch_turns_one_off_at_once() {
    let t = FakeHost::new().plugin(retro()).boot_core().await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    assert_eq!(home(&t), ["NES", "GENESIS"], "by the systems' full names; the Atari 2600 is off");
    assert_eq!(titles(&t), ["Contra", "Sonic"]);
    open_page(&t).await;
    assert_eq!(
        switches(&t),
        [on("Nintendo Entertainment System", true), on("Sega Genesis", true), on("Atari 2600", false)]
    );

    flip(&t, "Atari 2600").await;
    assert_eq!(home(&t), ["ATARI2600", "NES", "GENESIS"]);
    assert_eq!(titles(&t), ["Contra", "Pitfall", "Sonic"]);
    flip(&t, "Nintendo Entertainment System").await;
    assert_eq!(home(&t), ["ATARI2600", "GENESIS"]);
    assert_eq!(titles(&t), ["Pitfall", "Sonic"], "its games hide");
    assert_eq!(switches(&t)[0], on("Nintendo Entertainment System", false));

    let saved = t.core.kernel().config().get(&["app", "consoles", "retro"]);
    assert_eq!(saved, serde_json::json!({ "ATARI2600": true, "NES": false }), "only what the player changed");
}

#[tokio::test]
async fn the_catalog_is_asked_for_each_console_that_is_on() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let t = FakeHost::new().plugin(recording_catalog(asked.clone())).plugin(retro()).boot_core().await;
    t.core.dispatch(Command::Navigate(Route::Catalog)).await.unwrap();
    assert_eq!(*asked.lock().unwrap(), ["NES", "GENESIS"]);

    asked.lock().unwrap().clear();
    open_page(&t).await;
    flip(&t, "Sega Genesis").await;
    flip(&t, "Atari 2600").await;
    let mut last: Vec<String> = asked.lock().unwrap().clone();
    last.dedup();
    assert_eq!(last.iter().rev().take(2).rev().collect::<Vec<_>>(), ["NES", "ATARI2600"], "{last:?}");
}

#[tokio::test]
async fn each_console_that_is_on_imports_its_own_system_files_from_its_own_block() {
    let (home_dir, host) = (tempfile::tempdir().unwrap(), FakeHost::new());
    let downloads = home_dir.path().join("Downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("genesis-bios.zip"), b"archive").unwrap();
    host.archives().holds("genesis-bios.zip", &[("system/bios_MD.bin", b"md"), ("system/disksys.rom", b"fds")]);
    let t = host.home(home_dir.path()).plugin(retro()).boot_core().await;
    open_page(&t).await;
    let genesis = block(&t, "system-files:GENESIS").expect("the Genesis block");
    assert_eq!(genesis.text, "Genesis BIOS files");
    assert!(block(&t, "system-files:NES").is_some());
    assert!(block(&t, "system-files:ATARI2600").is_none(), "a console that is off, and has none");

    let pick = genesis.rows.iter().find(|r| r.label == "Import…").expect("Import…");
    let run =
        Command::RunAction { plugin: retro_id(), action: pick.action.clone(), game: None, args: pick.args.clone() };
    t.core.dispatch(run).await.unwrap();
    assert_eq!(t.core.view().browser.title, "Import Genesis BIOS files");
    let file = downloads.join("genesis-bios.zip");
    t.core.dispatch(Command::ImportSystemFiles { plugin: retro_id(), file }).await.unwrap();

    let system = t.data_dir().join("plugins/retro/system");
    assert_eq!(std::fs::read(system.join("bios_MD.bin")).ok().as_deref(), Some(&b"md"[..]));
    assert!(!system.join("disksys.rom").exists(), "only the Genesis part's files");
    let status = |id: &str| block(&t, id).unwrap().rows[0].value.clone();
    assert_eq!(status("system-files:GENESIS"), "Installed");
    assert_eq!(status("system-files:NES"), "Not installed");
    assert!(!t.data_dir().join("tmp/system-files/retro/GENESIS").exists(), "the work folder goes");
}

#[tokio::test]
async fn a_game_s_keys_and_its_install_note_are_its_console_s() {
    let t = FakeHost::new().plugin(retro()).boot_core().await;
    let manifest = t.core.kernel().manifest(&retro_id()).unwrap();
    assert_eq!(manifest.played_for("NES").map(|p| p.keymap.len()), Some(1));
    assert_eq!(manifest.played_for("GENESIS").map(|p| p.keymap.len()), Some(0));
    assert_eq!(manifest.played_for("nes").map(|p| p.install_note), Some("Installs an NES game."), "in any case");
}

fn refusal(manifest: &str) -> impl std::future::Future<Output = String> {
    let plugin = StubPlugin::from_toml(manifest);
    async move {
        let t = FakeHost::new().plugin(plugin).boot_core().await;
        t.report().rejection(&retro_id()).map(|e| e.to_string()).unwrap_or_default()
    }
}

#[tokio::test]
async fn a_manifest_of_several_consoles_is_refused_with_a_single_console_too_a_console_twice_or_a_wrong_id() {
    let both = format!("console = \"PS1\"\n{RETRO}");
    assert!(refusal(&both).await.contains("leave the single ones out"));
    let twice = RETRO.replace("console = \"GENESIS\"", "console = \"NES\"");
    assert!(refusal(&twice).await.contains("lists NES twice"));
    let wrong = RETRO.replace("id = \"GENESIS\"", "id = \"MD\"");
    assert!(refusal(&wrong).await.contains("make it \"GENESIS\""));
    let service = RETRO.replace("kind = \"emulator\"", "kind = \"service\"");
    assert!(refusal(&service).await.contains("only an emulator lists [[consoles]]"));
    assert_eq!(refusal(RETRO).await, "", "the manifest itself loads");
}

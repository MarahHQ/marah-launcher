//! One Catalog card per game: in "All", a game on two consoles is one card with a chip for each
//! console, standing for the version this PC runs best; a console's chip shows that console's
//! cards as before; the Game Hub lists the versions, the best first, and says why.

use super::*;
use launcher_core::{Gpu, Specs};

/// KytyPS5's needs, as its manifest has them.
const PS5_NEEDS: &str = "vulkan = \"1.3\"\n[requires.recommended]\ncpu_threads = 12\nmemory_gb = 14.5\ngpu_memory_gb = 7.5\ndiscrete_gpu = true";
/// shadPS4's needs, as its manifest has them.
const PS4_NEEDS: &str = "vulkan = \"1.3\"\navx2 = true\n[requires.recommended]\ncpu_threads = 8\nmemory_gb = 7.0\ngpu_memory_gb = 3.5\ndiscrete_gpu = true";

/// A PC with 32 threads, 64 GB and a 24 GB Vulkan 1.3 card; `avx2` as the processor says.
fn strong_pc(avx2: bool) -> Specs {
    let gpu =
        Gpu { name: "Test GPU".into(), memory_gb: Some(24.0), discrete: true, unified: false, vulkan: Some((1, 3)) };
    Specs { cpu: "Test CPU".into(), cores: 16, threads: 32, memory_gb: 64.0, avx2: Some(avx2), gpus: Some(vec![gpu]) }
}

/// A topic of `title` for `console`, released in 2024 by Sony.
fn on(console: &str, id: u32, title_id: &str, title: &str, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[{console}] {title} [EUR / v1.00]"),
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": "2026-10-01",
        "game_info": { "name": title, "title_id": title_id, "region": "EUR", "version": "1.00", "genre": "Action",
            "release_year": "2024", "publisher": "Sony", "release_summary": format!("{title} on {console}.") } })
}

/// Astro Bot on PS5 and PS4, and Quest on PS4 only, on the Catalog, on `pc`.
async fn astro_on_two_consoles(pc: Specs) -> TestCore {
    let t = FakeHost::new()
        .hardware(pc)
        .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-ps5").system("PS5").console("PS5").requires(PS5_NEEDS).games([]).build())
        .plugin(FakeCatalog::new("emu-ps4").system("PS4").console("PS4").requires(PS4_NEEDS).games([]).build())
        .boot_core()
        .await;
    let ps5 = serde_json::json!([on("PS5", 1, "PPSA01234", "Astro Bot", 'a')]);
    let ps4 = serde_json::json!([on("PS4", 2, "CUSA01234", "Astro Bot", 'b'), on("PS4", 3, "CUSA00002", "Quest", 'c')]);
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", ps5));
    t.http.serve("https://xyz.dev/catalog/ps4.json", catalog_of("2026-10-18T00:00:00Z", ps4));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(launcher_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    t.until(Area::Hardware, |core| core.view().settings.hardware.known).await;
    t.core.dispatch(Command::CatalogSort(launcher_core::Sort::Title)).await.unwrap();
    t
}

const ASTRO_PS5: &str = "release-catalog:emu-ps5/PPSA01234";
const ASTRO_PS4: &str = "release-catalog:emu-ps4/CUSA01234";

/// Each card of the Catalog's grid as (key, its consoles' chips).
fn cards(t: &TestCore) -> Vec<(String, Vec<String>)> {
    let games = t.core.view().catalog.games.games;
    games.into_iter().map(|c| (c.key, c.consoles.into_iter().map(|c| c.platform).collect())).collect()
}

/// The Catalog's system chips as (id, count).
fn chips(t: &TestCore) -> Vec<(String, usize)> {
    t.core.view().catalog.games.systems.into_iter().map(|c| (c.id, c.count)).collect()
}

#[tokio::test]
async fn all_shows_a_game_on_two_consoles_as_one_card_with_a_chip_for_each() {
    let t = astro_on_two_consoles(strong_pc(true)).await;
    let quest = "release-catalog:emu-ps4/CUSA00002".to_owned();
    // Both run well: shadPS4 asks less of this PC than KytyPS5.
    assert_eq!(cards(&t), [(ASTRO_PS4.to_owned(), vec!["PS4".to_owned(), "PS5".to_owned()]), (quest, Vec::new())]);
    let chip = |id: &str, count| (id.to_owned(), count);
    assert_eq!(chips(&t), [chip("", 2), chip("PS5", 1), chip("PS4", 2)], "All counts cards; a console its own");
}

#[tokio::test]
async fn a_console_chip_shows_that_console_s_cards_as_before() {
    let t = astro_on_two_consoles(strong_pc(true)).await;
    let system = |id: &str| Command::CatalogSystem(Some(launcher_core::SystemChoice::System(id.into())));
    t.core.dispatch(system("PS5")).await.unwrap();
    assert_eq!(cards(&t), [(ASTRO_PS5.to_owned(), Vec::new())]);
    t.core.dispatch(system("PS4")).await.unwrap();
    let keys: Vec<String> = cards(&t).into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, [ASTRO_PS4, "release-catalog:emu-ps4/CUSA00002"]);
}

#[tokio::test]
async fn the_game_hub_lists_the_versions_the_best_first_and_says_why() {
    let t = astro_on_two_consoles(strong_pc(true)).await;
    let detail = open_first(&t).await;
    let versions: Vec<(String, bool, String, String, bool)> =
        detail.versions.iter().map(|v| (v.platform.clone(), v.best, v.why.clone(), v.how.clone(), v.current)).collect();
    assert_eq!(
        versions,
        [
            (
                "PS4".into(),
                true,
                "Its plugin recommends the least of this PC".into(),
                "Not tested · Should run well".into(),
                true
            ),
            ("PS5".into(), false, String::new(), "Not tested · Should run well".into(), false),
        ]
    );
    // The other version opens on its own Game Hub, with the same versions.
    t.core.dispatch(Command::OpenGame(ASTRO_PS5.parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.platform, "PS5");
    let current: Vec<bool> = detail.versions.iter().map(|v| v.current).collect();
    assert_eq!(current, [false, true]);
}

#[tokio::test]
async fn a_version_this_pc_can_t_start_is_never_the_best() {
    // No AVX2: shadPS4 won't start.
    let t = astro_on_two_consoles(strong_pc(false)).await;
    assert_eq!(cards(&t)[0], (ASTRO_PS5.to_owned(), vec!["PS5".to_owned(), "PS4".to_owned()]));
    let detail = open_first(&t).await;
    let best = &detail.versions[0];
    assert_eq!((best.platform.as_str(), best.why.as_str()), ("PS5", "The other versions can't start on this PC"));
    assert_eq!(detail.versions[1].how, "Won't start on this PC");
}

#[tokio::test]
async fn hiding_a_game_s_card_hides_each_of_its_versions() {
    let t = astro_on_two_consoles(strong_pc(true)).await;
    let hide = |hidden| Command::Library(launcher_core::LibraryCmd::Hide { game: ASTRO_PS4.parse().unwrap(), hidden });
    t.core.dispatch(hide(true)).await.unwrap();
    let keys: Vec<String> = cards(&t).into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["release-catalog:emu-ps4/CUSA00002"], "the PS5 version doesn't take the card's place");
    let filter = Command::CatalogSystem(Some(launcher_core::SystemChoice::Filter(launcher_core::GridFilter::Hidden)));
    t.core.dispatch(filter).await.unwrap();
    assert_eq!(cards(&t).len(), 1, "under Hidden: the one card of both");
    t.core.dispatch(Command::CatalogSystem(None)).await.unwrap();
    t.core.dispatch(hide(false)).await.unwrap();
    assert_eq!(cards(&t).len(), 2, "shown again, both versions");
}

#[tokio::test]
async fn the_card_is_selected_when_any_of_its_versions_is() {
    let t = astro_on_two_consoles(strong_pc(true)).await;
    // The player came back from the PS5 version's Game Hub: the card of both is selected.
    t.core.dispatch(Command::Select(ASTRO_PS5.parse().unwrap())).await.unwrap();
    let grid = t.core.view().catalog.games;
    assert_eq!(grid.selected, Some(0));
    assert_eq!(grid.games[0].key, ASTRO_PS4);
}

//! One Catalog card per game: in "All", a game on two consoles is one card with a chip for each
//! console, standing for the version this PC runs best; a console's chip shows that console's
//! cards as before; the Game Hub lists the versions, the best first, and says why.

use super::*;
use launcher_api::CompatLevel;
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

/// A topic of `title` for `console`, released in 2024 by Sony, added to the catalog long ago.
fn on(console: &str, id: u32, title_id: &str, title: &str, hash: char) -> serde_json::Value {
    added_on(console, id, title_id, title, hash, "2026-10-01")
}

/// [`on`], added to the catalog on `day`.
fn added_on(console: &str, id: u32, title_id: &str, title: &str, hash: char, day: &str) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[{console}] {title} [EUR / v1.00]"),
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": day,
        "game_info": { "name": title, "title_id": title_id, "region": "EUR", "version": "1.00", "genre": "Action",
            "release_year": "2024", "publisher": "Sony", "release_summary": format!("{title} on {console}.") } })
}

/// Astro Bot on PS5 and PS4, and Quest on PS4 only, on the Catalog, on `pc`.
async fn astro_on_two_consoles(pc: Specs) -> TestCore {
    astro_added(pc, "2026-10-01").await
}

/// The PS5 emulator, with KytyPS5's needs and no games.
fn emu_ps5() -> FakeCatalog {
    FakeCatalog::new("emu-ps5").system("PS5").console("PS5").requires(PS5_NEEDS).games([])
}

/// The PS4 emulator, with shadPS4's needs and no games.
fn emu_ps4() -> FakeCatalog {
    FakeCatalog::new("emu-ps4").system("PS4").console("PS4").requires(PS4_NEEDS).games([])
}

/// [`astro_on_two_consoles`], with Astro Bot's PS5 version added to the catalog on `ps5_day`.
async fn astro_added(pc: Specs, ps5_day: &str) -> TestCore {
    astro_with(pc, ps5_day, emu_ps5(), emu_ps4()).await
}

/// [`astro_added`], with these emulators.
async fn astro_with(pc: Specs, ps5_day: &str, ps5: FakeCatalog, ps4: FakeCatalog) -> TestCore {
    let t = FakeHost::new()
        .hardware(pc)
        .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(ps5.build())
        .plugin(ps4.build())
        .boot_core()
        .await;
    let ps5 = serde_json::json!([added_on("PS5", 1, "PPSA01234", "Astro Bot", 'a', ps5_day)]);
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
    assert!(chips(&t).contains(&("hidden".to_owned(), 1)), "one card hidden, not two offers: {:?}", chips(&t));
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

#[tokio::test]
async fn hiding_one_version_leaves_the_game_s_other_versions() {
    let t = astro_on_two_consoles(strong_pc(true)).await;
    // The card is the PS4 version: the player hides it alone.
    let hide =
        Command::Library(launcher_core::LibraryCmd::HideVersion { game: ASTRO_PS4.parse().unwrap(), hidden: true });
    t.core.dispatch(hide).await.unwrap();
    let quest = "release-catalog:emu-ps4/CUSA00002".to_owned();
    assert_eq!(
        cards(&t),
        [(ASTRO_PS5.to_owned(), Vec::new()), (quest.clone(), Vec::new())],
        "the PS5's, with no chips"
    );
    let chip = |id: &str, count| (id.to_owned(), count);
    assert!(chips(&t).contains(&chip("hidden", 1)), "{:?}", chips(&t));
    let system = |id: &str| Command::CatalogSystem(Some(launcher_core::SystemChoice::System(id.into())));
    t.core.dispatch(system("PS5")).await.unwrap();
    assert_eq!(cards(&t), [(ASTRO_PS5.to_owned(), Vec::new())], "the PS5 version is not hidden");
    t.core.dispatch(system("PS4")).await.unwrap();
    assert_eq!(cards(&t), [(quest, Vec::new())], "the PS4 version is");
    // The Game Hub of the PS5 version is not hidden; Show on the PS4 version's shows it again.
    t.core.dispatch(Command::OpenGame(ASTRO_PS5.parse().unwrap())).await.unwrap();
    assert!(!t.core.view().detail.unwrap().hidden);
    let show = Command::Library(launcher_core::LibraryCmd::Hide { game: ASTRO_PS4.parse().unwrap(), hidden: false });
    t.core.dispatch(show).await.unwrap();
    t.core.dispatch(Command::Navigate(launcher_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::CatalogSystem(None)).await.unwrap();
    assert_eq!(cards(&t)[0], (ASTRO_PS4.to_owned(), vec!["PS4".to_owned(), "PS5".to_owned()]), "both versions again");
}

#[tokio::test]
async fn a_card_is_new_when_another_of_its_versions_is() {
    // The card is the PS4 version, added long ago; the PS5 version came this week.
    let t = astro_added(strong_pc(true), &days_ago(2)).await;
    let new = |t: &TestCore| -> Vec<(String, bool)> {
        let games = t.core.view().catalog.games.games;
        games.into_iter().map(|c| (c.key, c.badges.iter().any(|b| b.id == "new"))).collect()
    };
    let quest = "release-catalog:emu-ps4/CUSA00002".to_owned();
    assert_eq!(new(&t), [(ASTRO_PS4.to_owned(), true), (quest.clone(), false)], "All: New from the PS5 version");
    let system = |id: &str| Command::CatalogSystem(Some(launcher_core::SystemChoice::System(id.into())));
    t.core.dispatch(system("PS4")).await.unwrap();
    assert_eq!(new(&t), [(ASTRO_PS4.to_owned(), false), (quest, false)], "PS4: its own release is old");
    t.core.dispatch(system("PS5")).await.unwrap();
    assert_eq!(new(&t), [(ASTRO_PS5.to_owned(), true)]);
}

#[tokio::test]
async fn the_card_stands_for_the_versions_the_grid_picked_it_from() {
    // No AVX2: the PS5 version is the best, and it came this week.
    let t = astro_added(strong_pc(false), &days_ago(2)).await;
    // A search that finds the PS4 version alone: the grid picks it, and the card has both chips.
    t.core.dispatch(Command::CatalogSearch("CUSA01234".into())).await.unwrap();
    assert_eq!(cards(&t), [(ASTRO_PS4.to_owned(), vec!["PS4".to_owned(), "PS5".to_owned()])]);
    t.core.dispatch(Command::CatalogSearch(String::new())).await.unwrap();
    assert_eq!(cards(&t)[0], (ASTRO_PS5.to_owned(), vec!["PS5".to_owned(), "PS4".to_owned()]), "the best again");
    // The player hides the PS5 version alone.
    let hide =
        Command::Library(launcher_core::LibraryCmd::HideVersion { game: ASTRO_PS5.parse().unwrap(), hidden: true });
    t.core.dispatch(hide).await.unwrap();
    let new = |t: &TestCore| -> Vec<bool> {
        t.core.view().catalog.games.games.iter().map(|c| c.badges.iter().any(|b| b.id == "new")).collect()
    };
    let quest = "release-catalog:emu-ps4/CUSA00002".to_owned();
    // All: the grid picks the PS4 version, and its card has no chip and no New from the hidden one.
    assert_eq!(cards(&t), [(ASTRO_PS4.to_owned(), Vec::new()), (quest, Vec::new())]);
    assert_eq!(new(&t), [false, false], "the hidden PS5 version doesn't make the card New");
    let chip = |id: &str, count| (id.to_owned(), count);
    assert!(chips(&t).contains(&chip("", 2)) && chips(&t).contains(&chip("hidden", 1)), "{:?}", chips(&t));
    // Hidden: the PS5 version alone, with no chip for the PS4 version the player didn't hide.
    let filter = Command::CatalogSystem(Some(launcher_core::SystemChoice::Filter(launcher_core::GridFilter::Hidden)));
    t.core.dispatch(filter).await.unwrap();
    assert_eq!(cards(&t), [(ASTRO_PS5.to_owned(), Vec::new())]);
    assert_eq!(new(&t), [true], "New from its own release");
}

#[tokio::test]
async fn a_version_takes_the_community_result_of_the_installed_game_it_is() {
    // The player has Astro Bot on PS4 (title id CUSA00000): its list says Playable.
    let t = FakeHost::new()
        .hardware(strong_pc(true))
        .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-ps5").system("PS5").console("PS5").requires(PS5_NEEDS).games([]).build())
        .plugin(
            FakeCatalog::new("emu-ps4")
                .system("PS4")
                .console("PS4")
                .requires(PS4_NEEDS)
                .games(["Astro Bot"])
                .tested()
                .build(),
        )
        .boot_core()
        .await;
    t.http.serve(
        URL,
        catalog_of("2026-10-18T00:00:00Z", serde_json::json!([on("PS5", 1, "PPSA01234", "Astro Bot", 'a')])),
    );
    let ps4 = serde_json::json!([on("PS4", 2, "CUSA00000", "Astro Bot", 'b')]);
    t.http.serve("https://xyz.dev/catalog/ps4.json", catalog_of("2026-10-18T00:00:00Z", ps4));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(launcher_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    t.until(Area::Hardware, |core| core.view().settings.hardware.known).await;
    // The library reads its games again with the catalogs in; the installed game's details, with
    // its result, come in the background.
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    t.until(Area::Library, |core| core.view().catalog.games.systems.iter().any(|c| c.id == "ingame")).await;
    let ingame = chips(&t).into_iter().find(|(id, _)| id == "ingame");
    assert_eq!(ingame, Some(("ingame".to_owned(), 1)), "the In-game filter reads it too");
    t.core.dispatch(Command::OpenGame("release-catalog:emu-ps4/CUSA00000".parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.compat.map(|c| c.label), Some("Playable".into()), "the Compatibility fact says the same");
    let versions: Vec<(String, String, String)> =
        detail.versions.iter().map(|v| (v.platform.clone(), v.how.clone(), v.why.clone())).collect();
    assert_eq!(
        versions,
        [
            (
                "PS4".into(),
                "Playable · Should run well".into(),
                "Gets the furthest: Playable, as players report".into()
            ),
            ("PS5".into(), "Not tested · Should run well".into(), String::new()),
        ]
    );
}

/// Each version on the open Game Hub as (console, how it runs, why it is the best).
fn versions_of(detail: &launcher_ui_api::GameDetailVm) -> Vec<(String, String, String)> {
    detail.versions.iter().map(|v| (v.platform.clone(), v.how.clone(), v.why.clone())).collect()
}

fn version(platform: &str, how: &str, why: &str) -> (String, String, String) {
    (platform.into(), how.into(), why.into())
}

/// [`astro_on_two_consoles`] with these emulators, once the emulators' lists answered for the
/// offers: the PS5 version is the first card.
async fn astro_listed(ps5: FakeCatalog, ps4: FakeCatalog) -> TestCore {
    let t = astro_with(strong_pc(true), "2026-10-01", ps5, ps4).await;
    t.until(Area::Catalogs, |core| core.view().catalog.games.games.first().is_some_and(|c| c.key == ASTRO_PS5)).await;
    t
}

#[tokio::test]
async fn an_offer_the_player_doesn_t_have_takes_its_list_s_result_and_wins_the_best_pick() {
    // Nobody has Astro Bot. KytyPS5's list says its PS5 version gets in game; the PS4
    // emulator has no list. Without the list, the PS4 version is the best (it asks less).
    let t = astro_listed(emu_ps5().lists(&[("PPSA01234", CompatLevel::Ingame)]), emu_ps4()).await;
    let quest = "release-catalog:emu-ps4/CUSA00002".to_owned();
    assert_eq!(cards(&t), [(ASTRO_PS5.to_owned(), vec!["PS5".to_owned(), "PS4".to_owned()]), (quest, Vec::new())]);
    assert!(chips(&t).contains(&("ingame".to_owned(), 1)), "the In-game filter reads it: {:?}", chips(&t));
    // The Compat sort puts it first, before Quest.
    t.core.dispatch(Command::CatalogSort(launcher_core::Sort::Compat)).await.unwrap();
    assert_eq!(cards(&t)[0].0, ASTRO_PS5);
    let detail = open_first(&t).await;
    assert_eq!(
        versions_of(&detail),
        [
            version("PS5", "In-game · Should run well", "Gets the furthest: In-game, as players report"),
            version("PS4", "Not tested · Should run well", ""),
        ]
    );
    assert_eq!(detail.compat.map(|c| c.label), Some("In-game".into()), "the Hub's Compatibility fact");
    assert_eq!(detail.compat_page, "https://list.example/compatibility", "with the list's page");
}

#[tokio::test]
async fn an_offer_its_list_says_doesn_t_start_loses_the_best_pick() {
    // shadPS4's list says the PS4 version doesn't start: the PS5 version is the best.
    let t = astro_listed(emu_ps5(), emu_ps4().lists(&[("cusa01234", CompatLevel::Nothing)])).await;
    assert_eq!(cards(&t)[0], (ASTRO_PS5.to_owned(), vec!["PS5".to_owned(), "PS4".to_owned()]));
    let detail = open_first(&t).await;
    assert_eq!(
        versions_of(&detail),
        [
            version("PS5", "Not tested · Should run well", "The other versions can't start on this PC"),
            version("PS4", "Doesn't start", ""),
        ]
    );
}

#[tokio::test]
async fn a_list_without_the_title_look_up_leaves_the_offers_as_before() {
    // Both emulators have a list that says Playable for any game id, but neither looks up a
    // title id. The core asks only the look-up for an offer, so a game nobody has is not
    // tested, and the PS4 version is the best, as with no list.
    let t =
        astro_with(strong_pc(true), "2026-10-01", emu_ps5().tested_for_any_id(), emu_ps4().tested_for_any_id()).await;
    let quest = "release-catalog:emu-ps4/CUSA00002".to_owned();
    assert_eq!(cards(&t), [(ASTRO_PS4.to_owned(), vec!["PS4".to_owned(), "PS5".to_owned()]), (quest, Vec::new())]);
    assert!(!chips(&t).iter().any(|(id, _)| id == "ingame"), "no In-game chip: {:?}", chips(&t));
    let detail = open_first(&t).await;
    assert_eq!(
        versions_of(&detail),
        [
            version("PS4", "Not tested · Should run well", "Its plugin recommends the least of this PC"),
            version("PS5", "Not tested · Should run well", ""),
        ]
    );
    assert_eq!(detail.compat, None);
}

/// The player has Astro Bot on PS4 (CUSA00000), and its list says Playable for it. The Catalog
/// has its PS4 version and a PS5 version. It returns once the library knows the PS4 version is
/// the installed game.
async fn astro_installed_on_ps4() -> TestCore {
    let t = FakeHost::new()
        .hardware(strong_pc(true))
        .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(emu_ps5().build())
        .plugin(emu_ps4().games(["Astro Bot"]).lists(&[("CUSA00000", CompatLevel::Playable)]).build())
        .boot_core()
        .await;
    t.http.serve(
        URL,
        catalog_of("2026-10-18T00:00:00Z", serde_json::json!([on("PS5", 1, "PPSA01234", "Astro Bot", 'a')])),
    );
    let ps4 = serde_json::json!([on("PS4", 2, "CUSA00000", "Astro Bot", 'b')]);
    t.http.serve("https://xyz.dev/catalog/ps4.json", catalog_of("2026-10-18T00:00:00Z", ps4));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(launcher_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    t.until(Area::Hardware, |core| core.view().settings.hardware.known).await;
    // The library reads its games again with the catalogs in, so it knows the offer is the
    // installed game.
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    t.until(Area::Catalogs, |core| core.view().catalog.games.systems.iter().any(|c| c.id == "ingame")).await;
    t
}

#[tokio::test]
async fn the_player_s_own_result_for_the_installed_game_wins_over_the_list() {
    // The PS4 version would be the best. The player played it: it doesn't start.
    let t = astro_installed_on_ps4().await;
    t.core.dispatch(Command::AskResult("emu-ps4:G0".parse().unwrap())).await.unwrap();
    t.core.dispatch(Command::ReportResult(Some(CompatLevel::Nothing))).await.unwrap();
    assert!(!chips(&t).iter().any(|(id, _)| id == "ingame"), "the In-game filter reads the player's: {:?}", chips(&t));
    t.core.dispatch(Command::OpenGame("release-catalog:emu-ps4/CUSA00000".parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(
        versions_of(&detail),
        [
            version("PS5", "Not tested · Should run well", "The other versions can't start on this PC"),
            version("PS4", "Doesn't start", ""),
        ]
    );
}

const ASTRO_INSTALLED: &str = "release-catalog:emu-ps4/CUSA00000";

/// Rates `game` as reaching `level`, as after playing it.
async fn rate(t: &TestCore, game: &str, level: CompatLevel) {
    t.core.dispatch(Command::AskResult(game.parse().unwrap())).await.unwrap();
    t.core.dispatch(Command::ReportResult(Some(level))).await.unwrap();
}

/// The open Game Hub's "your result" for `offer`.
async fn your_result_on(t: &TestCore, offer: &str) -> String {
    t.core.dispatch(Command::OpenGame(offer.parse().unwrap())).await.unwrap();
    t.core.view().detail.unwrap().your_result
}

#[tokio::test]
async fn the_hub_of_a_version_shows_the_player_s_result_for_the_installed_game_it_is() {
    let t = astro_installed_on_ps4().await;
    rate(&t, "emu-ps4:G0", CompatLevel::Menus).await;
    assert_eq!(your_result_on(&t, ASTRO_INSTALLED).await, "menus");
}

#[tokio::test]
async fn the_hub_shows_the_player_s_result_for_the_version_over_the_installed_game_s() {
    let t = astro_installed_on_ps4().await;
    rate(&t, "emu-ps4:G0", CompatLevel::Menus).await;
    rate(&t, ASTRO_INSTALLED, CompatLevel::Ingame).await;
    assert_eq!(your_result_on(&t, ASTRO_INSTALLED).await, "ingame");
}

#[tokio::test]
async fn the_hub_of_a_version_nobody_rated_shows_no_result_of_the_player_s() {
    let t = astro_installed_on_ps4().await;
    assert_eq!(your_result_on(&t, ASTRO_INSTALLED).await, "");
    assert_eq!(your_result_on(&t, ASTRO_PS5).await, "", "nor a version that isn't installed");
}

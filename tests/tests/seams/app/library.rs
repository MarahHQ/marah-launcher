//! The library: catalogs merged, a broken one, search, and the first selection.

use super::*;

#[tokio::test]
async fn the_library_merges_every_catalog_sorted_by_title_with_badges() {
    let a = FakeEmulator::new(vec!["Zeta", "Alpha"]);
    let b = FakeEmulator::new(vec!["Mu"]);
    let t = FakeHost::new().plugin(a.plugin("emu-a")).plugin(b.plugin("emu-b")).boot_core().await;

    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let home = t.core.view().home;
    let titles: Vec<_> = home.games.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["Alpha", "Mu", "Zeta"]);
    assert_eq!(home.games[0].key, "emu-a:G1");
    assert_eq!(home.games[1].key, "emu-b:G0");
    assert_eq!(home.games[0].badges[0].label, "emu-a:region");
    assert_eq!(home.games[0].badges[0].text, "EU");
    // No plugin here describes PS4: its code is its name, in the theme's neutral look.
    assert_eq!(home.games[0].system, launcher_ui_api::GameSystemVm { name: "PS4".into(), color: None });
}

#[tokio::test]
async fn a_broken_catalog_leaves_a_notice_and_the_other_catalogs_still_show() {
    let mut broken = FakeEmulator::new(vec!["Hidden"]);
    broken.broken_catalog = true;
    let ok = FakeEmulator::new(vec!["Shown"]);
    let t = FakeHost::new().plugin(broken.plugin("emu-a")).plugin(ok.plugin("emu-b")).boot_core().await;

    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.home.games.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Shown"]);
    assert!(view.notices.iter().any(|n| n.contains("catalog server is down")), "{:?}", view.notices);
}

#[tokio::test]
async fn the_catalog_s_search_is_its_own() {
    let a = FakeEmulator::new(vec!["The Last Road", "Road Rage"]);
    let t = FakeHost::new().plugin(a.plugin("emu-a")).boot_core().await;

    t.core.dispatch(Command::CatalogSearch("road".into())).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.catalog.games.query, "road");
    assert_eq!(view.home.library.query, "", "the Library keeps its own search");
}

#[tokio::test]
async fn the_first_game_is_selected_with_its_details_once_the_library_loads() {
    let (t, _) = core_with(FakeEmulator::new(vec!["Beta", "Alpha"])).await;

    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.home.selected, Some(0), "the first game by title");
    assert_eq!(view.detail.map(|d| d.title), Some("Alpha".to_owned()), "its details are in, for the hero");
}

#[tokio::test]
async fn navigation_and_selection_live_in_the_core() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A", "B"])).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    t.core.dispatch(Command::Select(key("emu-a:G1"))).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G1"))).await.unwrap();
    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::GameDetail);
    assert_eq!(view.detail.as_ref().unwrap().title, "B");
    assert!(view.can_go_back);

    t.core.dispatch(Command::Navigate(Route::Downloads)).await.unwrap();
    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::GameDetail);
    t.core.dispatch(Command::Back).await.unwrap();
    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::Home);
    assert_eq!(view.home.selected, Some(1));
    assert!(view.home.games[1].selected);
}

#[tokio::test]
async fn the_library_s_search_forgives_and_ranks_the_best_matches_first() {
    let games =
        vec!["Godfall", "God of War III", "The Last of Us Part I", "Final Fantasy VII", "Ōkami HD", "Bloodborne"];
    let t = FakeHost::new().plugin(FakeEmulator::new(games).plugin("emu-a")).boot_core().await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let search = |text: &'static str| {
        let t = &t;
        async move {
            t.core.dispatch(Command::LibrarySearch(text.into())).await.unwrap();
            let library = t.core.view().home.library;
            let titles: Vec<String> = library.games.iter().map(|g| g.title.clone()).collect();
            let best: Vec<String> = titles.iter().take(launcher_core::BEST_MATCHES).cloned().collect();
            assert_eq!(library.matches, if text.is_empty() { Vec::new() } else { best }, "the best matches");
            titles
        }
    };
    assert_eq!(search("god").await, ["God of War III", "Godfall"], "a whole word before the start of one");
    assert_eq!(search("gow 3").await, ["God of War III"], "initials, and a number for its numeral");
    assert_eq!(search("tlou").await, ["The Last of Us Part I"]);
    assert_eq!(search("ff7").await, ["Final Fantasy VII"]);
    assert_eq!(search("okami").await, ["Ōkami HD"], "without its accent");
    assert_eq!(search("bloodbrone").await, ["Bloodborne"], "a typo");
    assert_eq!(search("").await.len(), 6, "no search, no best matches");
}

#[tokio::test]
async fn the_last_searches_are_kept_across_a_restart_and_can_be_forgotten() {
    let first = FakeHost::new().plugin(FakeEmulator::new(vec!["Alpha"]).plugin("emu-a")).boot_core().await;
    for text in ["alpha", "beta", " Alpha ", ""] {
        first.core.dispatch(Command::RememberSearch(text.into())).await.unwrap();
    }
    assert_eq!(first.core.view().home.library.recent, ["Alpha", "beta"], "newest first, once, trimmed");
    assert_eq!(first.core.view().catalog.games.recent, ["Alpha", "beta"], "the Catalog's too");
    for i in 0..10 {
        first.core.dispatch(Command::RememberSearch(format!("search {i}"))).await.unwrap();
    }
    assert_eq!(first.core.view().home.library.recent.len(), launcher_core::RECENT_SEARCHES, "the oldest go");
    let saved = std::fs::read(first.host.data_dir().join("config.json")).unwrap();

    // The next start reads what the first one saved.
    let host = FakeHost::new().plugin(FakeEmulator::new(vec!["Alpha"]).plugin("emu-a"));
    std::fs::write(host.data_dir().join("config.json"), saved).unwrap();
    let second = host.boot_core().await;
    let recent = second.core.view().home.library.recent;
    assert_eq!((recent.len(), recent[0].as_str()), (launcher_core::RECENT_SEARCHES, "search 9"));

    second.core.dispatch(Command::ForgetSearches).await.unwrap();
    assert!(second.core.view().home.library.recent.is_empty());
}

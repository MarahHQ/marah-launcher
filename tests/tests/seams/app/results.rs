//! After a run: how far the game got, asked once for each build.

use super::*;

#[tokio::test]
async fn after_playing_the_player_says_how_far_it_got_and_it_is_kept() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    process.exit(0);
    closed(&mut events).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"));

    t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Menus))).await.unwrap();

    assert_eq!(t.core.view().result_prompt, None);
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "menus");
}

#[tokio::test]
async fn not_now_keeps_no_result() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    process.exit(0);
    closed(&mut events).await;

    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    assert_eq!(t.core.view().result_prompt, None);
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "");
}

/// Plays `emu-a:G0` once more, as the `n`th run, and waits until it closes.
async fn play_run(t: &TestCore, n: usize) {
    let mut events = t.core.events();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.spawner.wait_for(n).await.exit(0);
    closed(&mut events).await;
}

#[tokio::test]
async fn how_far_it_got_is_asked_once_for_each_build() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    install(&t.core).await;
    play_run(&t, 1).await;
    t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Menus))).await.unwrap();

    play_run(&t, 2).await;
    assert_eq!(t.core.view().result_prompt, None, "this build has a result");

    emu.release("2.0");
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    play_run(&t, 3).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "a new build asks again");
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "menus", "the old result stays until then");
}

#[tokio::test]
async fn not_now_is_kept_for_the_build() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    install(&t.core).await;
    play_run(&t, 1).await;
    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    play_run(&t, 2).await;
    assert_eq!(t.core.view().result_prompt, None, "not again for this build");
    t.core.dispatch(Command::AskResult(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "Rate how it runs still asks");
    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    emu.release("2.0");
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    play_run(&t, 3).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "a new build asks again");
}

/// "Send ratings": the player's results go to the community lists that take reports, each
/// after the player saw it all and said yes.
mod send_ratings {
    use super::*;
    use launcher_core::RatingsCmd;
    use launcher_testkit::FAKE_REPORT_FORM;

    /// A core whose `emu-a` has a community list with a report form, on Alice's computer.
    async fn listed(titles: Vec<&'static str>) -> TestCore {
        let mut emu = FakeEmulator::new(titles);
        emu.list = true;
        let host = FakeHost::new().home(std::path::Path::new("/home/alice")).computer("alice-desktop");
        emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
        let t = host.plugin(emu.plugin("emu-a")).boot_core().await;
        t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
        t
    }

    async fn ratings(t: &TestCore, command: RatingsCmd) {
        t.core.dispatch(Command::Ratings(command)).await.unwrap();
    }

    /// Plays `emu-a:G0`, whose run writes `log`, and rates it Main menu.
    async fn play_and_rate(t: &TestCore, log: &str) {
        let mut events = t.core.events();
        let (_, process) = play(t).await;
        let logs = t.host.data_dir().join("logs").join("emu-a");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("G0.log"), log).unwrap();
        process.exit(0);
        closed(&mut events).await;
        t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Menus))).await.unwrap();
    }

    const LOG: &str = "[Loader] open /home/alice/Games/PS4/CUSA1/eboot.bin\n\
                       [Net] alice@alice-desktop connected from 192.168.1.20, mail alice.smith@example.com\n\
                       [Render] the error\n";

    #[tokio::test]
    async fn a_rating_waits_and_its_report_shows_whole_without_personal_data() {
        let t = listed(vec!["A"]).await;
        play_and_rate(&t, LOG).await;

        ratings(&t, RatingsCmd::Open).await;
        let view = t.core.view().ratings;
        assert!(view.open && !view.previewing);
        let item = launcher_ui_api::RatingItemVm {
            game: "emu-a:G0".into(),
            title: "A".into(),
            result: "Main menu".into(),
            list: "Fake list".into(),
            picked: true,
        };
        assert_eq!(view.items, vec![item]);
        assert!(t.host.opener.links().is_empty(), "opening sends nothing");

        ratings(&t, RatingsCmd::Next).await;
        let preview = t.core.view().ratings.preview;
        assert_eq!(preview.title, "G0 - A");
        assert_eq!((preview.list.as_str(), preview.destination.as_str()), ("Fake list", "lists.example/new"));
        assert_eq!((preview.step.as_str(), preview.note.as_str()), ("1 of 1", "Tick the checklist."));
        assert_eq!(
            preview.body,
            "Game: A\n\nStatus: menus\n\nBuild: 1.0\n\nOS: Linux\n\nLog:\n\
             [Loader] open ~/Games/PS4/CUSA1/eboot.bin\n\
             [Net] <user>@<computer> connected from <ip>, mail <email>\n\
             [Render] the error"
        );
        assert!(t.host.opener.links().is_empty(), "the preview sends nothing");
    }

    #[tokio::test]
    async fn yes_opens_the_form_with_what_the_player_saw_and_marks_it_sent() {
        let t = listed(vec!["A"]).await;
        play_and_rate(&t, LOG).await;
        ratings(&t, RatingsCmd::Open).await;
        ratings(&t, RatingsCmd::Next).await;
        let preview = t.core.view().ratings.preview;

        ratings(&t, RatingsCmd::Send).await;

        let links = t.host.opener.links();
        assert_eq!(links.len(), 1);
        assert!(links[0].starts_with(FAKE_REPORT_FORM), "{}", links[0]);
        let url = url::Url::parse(&links[0]).unwrap();
        let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(query["title"], preview.title);
        let log = preview.body.split_once("Log:\n").unwrap().1;
        assert_eq!(query["log"], log, "the link holds the log the player saw");
        assert!(!links[0].contains("alice") && !links[0].contains("192.168"), "{}", links[0]);
        assert!(!t.core.view().ratings.open, "the last report closes it");

        ratings(&t, RatingsCmd::Open).await;
        assert!(t.core.view().ratings.items.is_empty(), "a sent rating doesn't wait");
    }

    #[tokio::test]
    async fn no_sends_nothing_and_the_rating_still_waits() {
        let t = listed(vec!["A"]).await;
        play_and_rate(&t, LOG).await;
        ratings(&t, RatingsCmd::Open).await;
        ratings(&t, RatingsCmd::Next).await;

        ratings(&t, RatingsCmd::Skip).await;
        assert!(!t.core.view().ratings.open);
        ratings(&t, RatingsCmd::Open).await;
        ratings(&t, RatingsCmd::Close).await;

        assert!(t.host.opener.links().is_empty());
        ratings(&t, RatingsCmd::Open).await;
        assert_eq!(t.core.view().ratings.items.len(), 1, "it waits for the next time");
    }

    #[tokio::test]
    async fn the_player_picks_which_ratings_go() {
        let t = listed(vec!["A", "B"]).await;
        for game in ["emu-a:G0", "emu-a:G1"] {
            t.core.dispatch(Command::AskResult(key(game))).await.unwrap();
            t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Ingame))).await.unwrap();
        }
        ratings(&t, RatingsCmd::Open).await;
        assert_eq!(t.core.view().ratings.items.len(), 2);

        ratings(&t, RatingsCmd::Pick(key("emu-a:G0"))).await;
        assert!(!t.core.view().ratings.items[0].picked);
        ratings(&t, RatingsCmd::Next).await;
        let preview = t.core.view().ratings.preview;
        assert_eq!((preview.game.as_str(), preview.step.as_str()), ("emu-a:G1", "1 of 1"));
        ratings(&t, RatingsCmd::Send).await;

        assert_eq!(t.host.opener.links().len(), 1);
        assert!(t.host.opener.links()[0].contains("G1"));
        ratings(&t, RatingsCmd::Open).await;
        let left: Vec<String> = t.core.view().ratings.items.into_iter().map(|i| i.game).collect();
        assert_eq!(left, ["emu-a:G0"], "the one left out still waits");
    }

    #[tokio::test]
    async fn sent_ratings_stay_sent_after_a_restart() {
        let t = listed(vec!["A", "B"]).await;
        for game in ["emu-a:G0", "emu-a:G1"] {
            t.core.dispatch(Command::AskResult(key(game))).await.unwrap();
            t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Boots))).await.unwrap();
        }
        ratings(&t, RatingsCmd::Open).await;
        ratings(&t, RatingsCmd::Next).await;
        ratings(&t, RatingsCmd::Send).await;

        let again = Core::start(t.host.kernel.clone()).await;
        again.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
        again.dispatch(Command::Ratings(RatingsCmd::Open)).await.unwrap();
        let left: Vec<String> = again.view().ratings.items.into_iter().map(|i| i.game).collect();
        assert_eq!(left, ["emu-a:G1"], "the sent one stays sent; the other still waits");
    }

    #[tokio::test]
    async fn on_windows_the_ratings_are_listed_and_the_whole_form_link_opens() {
        let mut emu = FakeEmulator::new(vec!["A"]);
        emu.list = true;
        let t = FakeHost::new().platform(Some(Platform::Windows)).plugin(emu.plugin("emu-a")).boot_core().await;
        t.core.dispatch(Command::AskResult(key("emu-a:G0"))).await.unwrap();
        t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Ingame))).await.unwrap();

        ratings(&t, RatingsCmd::Open).await;
        let games: Vec<String> = t.core.view().ratings.items.into_iter().map(|i| i.game).collect();
        assert_eq!(games, ["emu-a:G0"]);
        ratings(&t, RatingsCmd::Next).await;
        assert!(t.core.view().ratings.preview.body.contains("OS: Windows"), "{:?}", t.core.view().ratings.preview);
        ratings(&t, RatingsCmd::Send).await;

        let links = t.host.opener.links();
        assert_eq!(links.len(), 1);
        let url = url::Url::parse(&links[0]).unwrap();
        let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(query["title"], "G0 - A", "every field after the first `&` is there: {}", links[0]);
        assert_eq!(query["template"], "report.yml");
    }

    #[tokio::test]
    async fn a_list_that_takes_no_reports_is_left_out() {
        let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
        t.core.dispatch(Command::AskResult(key("emu-a:G0"))).await.unwrap();
        t.core.dispatch(Command::ReportResult(Some(launcher_api::CompatLevel::Ingame))).await.unwrap();

        ratings(&t, RatingsCmd::Open).await;
        let view = t.core.view().ratings;
        assert!(view.open && view.items.is_empty());
        ratings(&t, RatingsCmd::Next).await;
        assert!(!t.core.view().ratings.previewing);
        ratings(&t, RatingsCmd::Send).await;
        assert!(t.host.opener.links().is_empty());
    }
}

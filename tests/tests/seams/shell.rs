//! The app shell the app ships, on a fake platform and the core's fake clock: render
//! coalescing, the tick, and the requests only the app carries out.

use std::time::Duration;

use launcher_core::{Area, Command, LibraryCmd, Pref, SessionCmd};
use launcher_testkit::PlatformCall;
use launcher_ui_host::{RENDER_GAP, TICK, TrailerPoll, TrailerRequest, TrailerStatus, UiRequest};

use crate::common::*;

/// A shell showing the Classic theme's Home, booted, with one render behind it.
fn shown() -> Composed {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(LibraryCmd::Refresh));
    host.render().unwrap();
    assert!(!host.ui.waiting_for_art(), "Home has no art to wait for");
    host.platform.clear_wakes();
    host
}

#[test]
fn a_burst_of_core_events_renders_once_when_its_time_comes() {
    let mut host = shown();
    let before = host.renders();
    for _ in 0..20 {
        host.on_core_changed();
    }
    assert_eq!(host.platform.wakes(), [RENDER_GAP], "one wake, a frame after the last render");
    host.poll();
    assert_eq!(host.renders(), before, "the last render was just now");

    host.advance(RENDER_GAP);
    host.poll();
    assert_eq!(host.renders(), before + 1);
    host.poll();
    assert_eq!(host.renders(), before + 1, "once for the whole burst");

    host.on_core_changed();
    assert_eq!(host.platform.wakes().last(), Some(&RENDER_GAP), "the next change waits for its frame");
}

#[test]
fn a_change_long_after_the_last_render_renders_on_the_next_poll() {
    let mut host = shown();
    host.advance(Duration::from_secs(1));
    let before = host.renders();
    host.on_core_changed();
    assert_eq!(host.platform.wakes(), [Duration::ZERO]);
    host.poll();
    assert_eq!(host.renders(), before + 1);
}

#[test]
fn the_tick_re_renders_every_15_seconds_while_a_game_runs() {
    let mut host = shown();
    host.platform.set_local_time("9:38 PM");
    host.on_tick();
    host.on_tick();
    let idle = host.renders();
    host.on_tick();
    assert_eq!(host.renders(), idle, "nothing changed, no game: no render");

    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.on_tick();
    let playing = host.renders();
    for _ in 0..59 {
        host.advance(TICK);
        host.on_tick();
    }
    assert_eq!(host.renders(), playing, "not before 15 s");
    host.advance(TICK);
    host.on_tick();
    assert_eq!(host.renders(), playing + 1, "the time played moves on");
}

#[test]
fn the_clock_s_minute_renders_once() {
    let mut host = shown();
    host.platform.set_local_time("9:38 PM");
    host.on_tick();
    let before = host.renders();
    host.on_tick();
    assert_eq!(host.renders(), before);
    host.platform.set_local_time("9:39 PM");
    host.on_tick();
    assert_eq!(host.renders(), before + 1);
    assert_eq!(host.ui.clock(), "9:39 PM");
}

#[test]
fn the_window_shown_line_goes_out_once() {
    let mut host = shown();
    host.on_tick();
    host.on_tick();
    let lines: Vec<_> = host.platform.calls().into_iter().filter(|c| matches!(c, PlatformCall::Announce(_))).collect();
    assert_eq!(lines, [PlatformCall::Announce("window shown".into())]);
}

#[test]
fn the_window_is_claimed_after_the_announcement_and_on_each_tick_until_the_platform_is_done() {
    let mut host = shown();
    host.platform.refuse_claims(2);
    for _ in 0..4 {
        host.on_tick();
    }
    let session_calls: Vec<_> = host
        .platform
        .calls()
        .into_iter()
        .filter(|c| matches!(c, PlatformCall::Announce(_) | PlatformCall::ClaimWindow))
        .collect();
    assert_eq!(
        session_calls,
        [
            PlatformCall::Announce("window shown".into()),
            PlatformCall::ClaimWindow,
            PlatformCall::ClaimWindow,
            PlatformCall::ClaimWindow,
        ],
        "the line first; two refusals ask again on the next tick; the third claim is the last"
    );
}

#[test]
fn log_out_closes_what_runs_then_tells_the_session_and_quits() {
    let mut host = shown();
    // What Log out asks of the app; the first tick's own calls (the window shown) aside.
    let leaving = |host: &Composed| {
        let calls = host.platform.calls();
        calls
            .into_iter()
            .filter(|c| matches!(c, PlatformCall::Quit) || *c == PlatformCall::Announce("log out".into()))
            .collect::<Vec<_>>()
    };
    host.on_request(UiRequest::LogOut).unwrap();
    host.on_tick();
    assert!(leaving(&host).is_empty(), "not before the core closed what runs: {:?}", host.platform.calls());

    // The core's Log out runs; with nothing running it is ready at once.
    host.run_work();
    host.on_tick();
    assert_eq!(leaving(&host), [PlatformCall::Announce("log out".into()), PlatformCall::Quit]);
    host.on_tick();
    assert_eq!(leaving(&host).len(), 2, "once: {:?}", host.platform.calls());
}

#[test]
fn the_theme_s_requests_wait_for_the_next_poll() {
    let mut host = shown();
    host.requests.borrow_mut().push(UiRequest::OpenUrl("https://art.example/".into()));
    host.requests.borrow_mut().push(UiRequest::Restart);
    assert!(host.platform.calls().is_empty());
    host.poll();
    assert_eq!(host.platform.calls(), [PlatformCall::OpenUrl("https://art.example/".into()), PlatformCall::Restart]);
    assert!(host.requests.borrow().is_empty());
}

#[test]
fn a_theme_command_runs_on_the_core_off_the_ui_thread() {
    let mut host = shown();
    host.on_request(UiRequest::Core(Command::Navigate(launcher_core::Route::Settings))).unwrap();
    assert_eq!(host.platform.waiting_work(), 1, "started, not run");
    assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Home);
    host.run_work();
    assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::Settings);
}

#[test]
fn a_trailer_that_can_t_play_in_the_window_opens_in_the_browser_and_says_why() {
    let mut host = shown();
    host.platform.refuse_trailers("Trailers need libmpv");
    host.on_request(UiRequest::Trailer(TrailerRequest::Play("https://art.example/a.mp4".into()))).unwrap();
    assert_eq!(
        host.platform.calls(),
        [
            PlatformCall::PlayTrailer("https://art.example/a.mp4".into()),
            PlatformCall::OpenUrl("https://art.example/a.mp4".into())
        ]
    );
    let notices = host.core().view().notices;
    assert!(notices.iter().any(|n| n == "The trailer opened in your browser: Trailers need libmpv"), "{notices:?}");
}

#[test]
fn the_tick_follows_the_trailer_and_closes_it_at_its_end() {
    let mut host = shown();
    host.on_request(UiRequest::Trailer(TrailerRequest::Play("https://art.example/a.mp4".into()))).unwrap();
    assert_eq!(host.ui.trailer().map(|t| t.loading), Some(true), "opening");
    let playing = TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 };
    host.platform.set_trailer(Some(TrailerPoll::Playing(playing)));
    host.on_tick();
    assert_eq!(host.ui.trailer().map(|t| t.position), Some(42.0));

    host.platform.set_trailer(Some(TrailerPoll::Ended));
    host.on_tick();
    assert_eq!(host.platform.calls().last(), Some(&PlatformCall::CloseTrailer));
    assert_eq!(host.ui.trailer(), None);
}

#[test]
fn the_window_moves_once_to_the_monitor_picked_in_settings() {
    let mut host = shown();
    host.platform.set_monitors(&["DP-1", "DP-2"]);
    host.on_tick();
    assert!(!host.platform.calls().iter().any(|c| matches!(c, PlatformCall::MoveToMonitor { .. })), "none picked");
    host.dispatch(Command::Pref(Pref::Monitor, "DP-2".into()));
    host.on_tick();
    host.on_tick();
    let moves: Vec<_> =
        host.platform.calls().into_iter().filter(|c| matches!(c, PlatformCall::MoveToMonitor { .. })).collect();
    assert_eq!(moves, [PlatformCall::MoveToMonitor { monitor: "DP-2".into(), fullscreen: true }]);
}

/// A shell showing the Classic theme's Home over a library and a system plugin, booted, with
/// the machine read and one render behind it, and the plugin's machine.
fn shown_with_system() -> (Composed, launcher_testkit::FakeSystem) {
    let system = launcher_testkit::FakeSystem::new();
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").playable().games(["Alpha", "Beta"]).build()).plugin(system.plugin())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(LibraryCmd::Refresh));
    host.until(Area::System, |core| core.view().system.audio_available);
    // The work of the start ends: the details, the files, the machine's reads.
    host.run_until_idle();
    host.run_work();
    host.render().unwrap();
    host.platform.clear_wakes();
    (host, system)
}

/// The volume the theme shows, as bound.
fn shown_volume(host: &Composed) -> Option<f64> {
    use slint_interpreter::Value;
    let Ok(Value::Struct(system)) = host.instance().get_global_property("Vm", "system") else { return None };
    match system.get_field("volume") {
        Some(Value::Number(volume)) => Some(*volume),
        _ => None,
    }
}

/// The sound's level changes on the machine, and the core reads it again.
fn change_the_volume(host: &mut Composed, system: &launcher_testkit::FakeSystem, volume: u8) {
    system.with(|m| m.level.volume = volume);
    host.rt.block_on(host.core().dispatch(Command::System(launcher_core::system::SystemCommand::Refresh))).unwrap();
    host.run_work();
}

#[test]
fn fifty_changes_in_one_frame_render_once() {
    let (mut host, _system) = shown_with_system();
    let before = host.renders();
    for n in 0..50 {
        host.core().report(format!("change {n}"));
    }
    host.run_work();
    assert_eq!(host.platform.wakes(), [RENDER_GAP], "one wake for the whole burst");
    host.poll();
    assert_eq!(host.renders(), before, "not before its frame");
    host.advance(RENDER_GAP);
    host.poll();
    host.poll();
    assert_eq!(host.renders(), before + 1, "one render for fifty changes");
}

#[test]
fn a_system_change_doesn_t_build_the_game_lists_again() {
    let (mut host, system) = shown_with_system();
    let (renders, lists) = (host.renders(), host.core().timings().game_lists);
    change_the_volume(&mut host, &system, 31);
    host.advance(RENDER_GAP);
    host.poll();
    assert_eq!(host.renders(), renders + 1, "the change renders");
    assert_eq!(host.core().timings().game_lists, lists, "the game lists stand as they were");
    assert_eq!(shown_volume(&host), Some(31.0), "the new volume shows");

    // A change in the library builds them again.
    host.core().report("and a notice".into());
    host.dispatch(Command::Library(LibraryCmd::Refresh));
    assert!(host.core().timings().game_lists > lists, "the library changed: the lists are built again");
}

/// The areas that changed since `changes` began, each once.
fn areas(changes: &mut tokio::sync::broadcast::Receiver<launcher_core::Change>) -> Vec<Area> {
    let mut areas = Vec::new();
    while let Ok(change) = changes.try_recv() {
        if !areas.contains(&change.area) {
            areas.push(change.area);
        }
    }
    areas
}

/// A view built after a change in an area that doesn't move the game lists, over the view
/// before it, is the view built whole.
#[test]
fn a_view_built_after_a_change_that_keeps_the_game_lists_is_the_whole_view() {
    let (mut host, system) = shown_with_system();
    let core = host.core().clone();
    let open = launcher_core::BrowseCmd::Open { at: None, purpose: launcher_core::BrowsePurpose::Pick("games".into()) };
    type Change<'a> = Box<dyn Fn(&mut Composed) + 'a>;
    let changes: [(Area, Change); 4] = [
        (Area::System, Box::new(|host| change_the_volume(host, &system, 12))),
        (Area::Notices, Box::new(|host| host.core().report("a notice".into()))),
        (
            Area::Browser,
            Box::new(|host| host.rt.block_on(host.core().dispatch(Command::Browse(open.clone()))).unwrap()),
        ),
        (
            Area::Backup,
            Box::new(|host| {
                host.rt.block_on(host.core().dispatch(Command::Backup(launcher_core::BackupCmd::Refresh))).unwrap()
            }),
        ),
    ];
    for (area, change) in changes {
        host.run_until_idle();
        let mut changed = core.changes();
        let before = core.view();
        change(&mut host);
        host.run_until_idle();
        assert_eq!(areas(&mut changed), [area], "only {area:?} changed");
        assert_eq!(core.view_after(before, &[area]), core.view(), "{area:?}");
    }
}

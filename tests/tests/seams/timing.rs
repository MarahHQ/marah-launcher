//! Opt-in, `MARAH_TIMING=1`: how long the Catalog's view takes to build and bind with the
//! bundled catalogs, which are as big as the project's `assets/catalog/` makes them, and how the
//! renders go in the first 10 s of a start with a big library. Prints the times; checks only
//! that the Catalog and the Library have their games.

use std::time::{Duration, Instant};

use crate::common::*;
use launcher_core::{Command, Route, Timed};
use launcher_testkit::FakeSystem;

#[test]
fn the_catalog_s_view_builds_and_binds_in_time() {
    if std::env::var_os("MARAH_TIMING").is_none() {
        return;
    }
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS5").games(["Alpha"]).build().console("PS5"))
            .plugin(FakeCatalog::new("emu-shadps4").system("PS4").games(["Beta"]).build().console("PS4"))
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    let started = Instant::now();
    host.dispatch(Command::Navigate(Route::Catalog));
    eprintln!("first Catalog open, with reading the catalogs: {:?}", started.elapsed());
    let started = Instant::now();
    host.dispatch(Command::Navigate(Route::Home));
    host.dispatch(Command::Navigate(Route::Catalog));
    eprintln!("Catalog open again: {:?}", started.elapsed());

    let total = host.core().view().catalog.games.total;
    assert!(total > 0, "the bundled catalogs have games");
    let runs = 20;
    let started = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(host.core().view());
    }
    eprintln!("{total} games: core.view() {:?} each", started.elapsed() / runs);
    let started = Instant::now();
    for _ in 0..runs {
        host.ui.render().unwrap();
    }
    eprintln!("ui.render() (view and bind) {:?} each", started.elapsed() / runs);
    let started = Instant::now();
    host.dispatch(Command::CatalogSort(launcher_core::Sort::Rating));
    host.dispatch(Command::CatalogSearch("the".into()));
    eprintln!("a sort and a search: {:?}", started.elapsed());
}

/// How many games the start's library has, and over how many game folders.
const GAMES: usize = 500;
const FOLDERS: u32 = 3;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn timed(what: &str, t: &Timed) {
    eprintln!(
        "{what}: {} runs, mean {:.2} ms, worst {:.2} ms, total {:.1} ms",
        t.count,
        ms(t.mean()),
        ms(t.worst),
        ms(t.total)
    );
}

/// The first 10 s of a start, as the app runs it: the theme, the first render, then a frame
/// each 16 ms that runs the work that came back, polls the shell, and ticks it each 250 ms. The
/// library has 500 games over 3 game folders on disk, and a system plugin reads the machine.
#[test]
fn the_first_10_seconds_of_a_start_render_in_time() {
    if std::env::var_os("MARAH_TIMING").is_none() {
        return;
    }
    let folders: Vec<tempfile::TempDir> = (0..FOLDERS).map(|_| tempfile::tempdir().unwrap()).collect();
    let titles: Vec<String> = (0..GAMES).map(|i| format!("Game {i:03}")).collect();
    for (i, title) in titles.iter().enumerate() {
        std::fs::create_dir_all(folders[i % FOLDERS as usize].path().join(title)).unwrap();
    }
    let system = FakeSystem::new();
    // As the app runs: the Default theme, the Game Catalog with its bundled catalogs.
    let mut host = composed_first_run(|h| {
        h.plugin(
            FakeCatalog::new("emu-kyty")
                .system("PS5")
                .in_folders_over(FOLDERS)
                .games(titles.iter().map(String::as_str))
                .build()
                .console("PS5"),
        )
        .plugin(FakeCatalog::new("emu-shadps4").system("PS4").games(["Beta"]).build().console("PS4"))
        .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(system.plugin())
    });
    let paths = folders.iter().map(|f| f.path().to_path_buf()).collect();
    host.rt.block_on(host.core().dispatch(Command::SetFolders { name: "games".into(), paths })).unwrap();

    let started = Instant::now();
    host.ui.start().unwrap();
    let theme = started.elapsed();
    {
        let _inside = host.rt.enter();
        host.core().scan_soon();
    }
    host.render().unwrap();
    let first = started.elapsed();
    let frame = Duration::from_millis(16);
    let mut since_tick = Duration::ZERO;
    let mut waited = Duration::ZERO;
    while waited < Duration::from_secs(10) {
        host.run_work();
        host.advance(frame);
        host.poll();
        since_tick += frame;
        if since_tick >= launcher_ui_host::TICK {
            since_tick = Duration::ZERO;
            host.on_tick();
        }
        waited += frame;
    }
    let all = host.core().view().home.library.all;
    assert_eq!(all, GAMES + 1, "the library has its games, and Beta");

    let (ui, core) = (host.shell.timings(), host.core().timings());
    eprintln!("the theme shown in {:.1} ms; the first render done {:.1} ms after the start", ms(theme), ms(first));
    timed("renders", &ui.renders);
    timed("core views", &core.views);
    timed("snapshots", &core.snapshots);
    timed("snapshot files", &core.files);
    timed("polls", &ui.polls);
    timed("ticks", &ui.ticks);
    timed("UI lock waits", &core.ui_lock_waits);
    eprintln!(
        "the share of the renders in files: {:.0} %",
        100.0 * core.files.total.as_secs_f64() / ui.renders.total.as_secs_f64()
    );

    // With the whole library in: what each render costs from now on.
    let runs = 20;
    for _ in 0..runs {
        host.render().unwrap();
    }
    let (after, core_after) = (host.shell.timings().renders.since(&ui.renders), host.core().timings());
    eprintln!("with the library in, a render: mean {:.2} ms", ms(after.mean()));
    let files = core_after.files.since(&core.files);
    let views = core_after.views.since(&core.views);
    eprintln!("  of it, the view {:.2} ms, the files {:.2} ms", ms(views.mean()), ms(files.mean()));
}

//! The launcher.
//!
//! The core runs on a tokio runtime. The UI runs on the main thread in Slint's event loop,
//! where the app shell (`launcher_ui_host::AppShell`) owns the theme host. This file wires the
//! two: core events, controller presses and the tick go to the shell on the UI thread, and the
//! shell reaches the system through `platform.rs`.

mod archives;
mod args;
mod audio;
mod clock;
mod controller_probe;
mod desktop;
mod disk;
mod gamepad;
mod gpu_probe;
mod hardware;
mod http;
mod libarchive;
mod platform;
mod processes;
#[cfg(test)]
mod real_releases;
mod torrent_storage;
mod torrents;
mod trailer;
mod update;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use launcher_core::{Command, Core, HostConfig, Kernel, TokioClock};
use launcher_ui_host::{AppShell, ShellOptions};

// Linked for their compile-time plugin factories.
use emu_kyty as _;
use emu_lichii as _;
use emu_pc as _;
use emu_shadps4 as _;
use emu_steam as _;
use playstation_store as _;
use release_catalog as _;
use system_host as _;
use theme_classic as _;
use theme_default as _;

fn main() {
    let launched = Instant::now();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    // The graphics probe the "This PC" check runs in a child process: print the cards, exit.
    let raw: Vec<String> = std::env::args().collect();
    if let Some(at) = raw.iter().position(|a| a == "--supports") {
        std::process::exit(if raw.get(at + 1).is_some_and(|f| args::supports(f)) { 0 } else { 1 });
    }
    if raw.iter().any(|a| a == "--probe-controllers") {
        std::process::exit(if controller_probe::print() { 0 } else { 1 });
    }
    if raw.iter().any(|a| a == "--probe-gpu") {
        let data_dir = raw.iter().position(|a| a == "--data-dir").and_then(|i| raw.get(i + 1)).map(PathBuf::from);
        let found = gpu_probe::print(&data_dir.unwrap_or_else(args::default_data_dir));
        std::process::exit(if found { 0 } else { 1 });
    }
    let args = match args::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    update::apply_launcher_update(&args.data_dir);
    if let Err(message) = run(args, launched) {
        tracing::error!("{message}");
        eprintln!("launcher: {message}");
        std::process::exit(1);
    }
}

/// The longest the app waits for the core to close down when it quits.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(10);
/// How long after the shutdown's limit the app ends anyway, whatever still runs.
const EXIT_MARGIN: Duration = Duration::from_secs(5);
/// How long after the start the app logs how its renders went.
const STARTUP_WINDOW: Duration = Duration::from_secs(10);

/// What the UI thread owns.
struct App {
    shell: AppShell,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the UI thread's app, once it is there and until it closes.
fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            f(app);
        }
    });
}

fn run(args: args::Args, launched: Instant) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    let mut host = HostConfig::new(&args.data_dir);
    host.home = args::home_dir();
    host.safe_mode = args.safe_mode;
    let extensions = args.extensions.clone().unwrap_or_else(|| args.data_dir.join("extensions"));

    let core = runtime.block_on(async {
        let (kernel, report) = Kernel::builder(host)
            .http(Arc::new(http::ReqwestTransport::new()?))
            .processes(Arc::new(processes::SystemProcesses))
            .windows(Arc::new(desktop::SystemWindows { raise: Box::new(platform::raise_launcher) }))
            .login_session(Arc::new(desktop::SystemLoginSession { login: update::login_program() }))
            .opener(Arc::new(desktop::SystemOpener))
            .hardware(Arc::new(hardware::SystemHardware { data_dir: args.data_dir.clone() }))
            .drives(Arc::new(disk::SystemDrives))
            .torrents(Arc::new(torrents::RqbitTorrents::new(args.data_dir.join("torrents"))))
            .archives(Arc::new(archives::HostArchives))
            .clock(Arc::new(TokioClock))
            .built_in_plugins()
            .discover(&extensions)
            .discover(&args::bundled_extensions())
            .boot()
            .await;
        for rejected in &report.rejected {
            tracing::warn!(source = %rejected.source, error = %rejected.error, "plugin not loaded");
        }
        tracing::info!(plugins = ?report.order, "booted");
        let booted = launched.elapsed();
        let core = Core::start(kernel).await;
        tracing::info!(
            boot_ms = booted.as_millis(),
            core_ms = (launched.elapsed() - booted).as_millis(),
            "core started"
        );
        for (name, paths) in &args.folders {
            core.dispatch(Command::SetFolders { name: name.clone(), paths: paths.clone() })
                .await
                .map_err(|e| format!("--folder {name}: {e}"))?;
        }
        Ok::<_, String>(core)
    })?;
    let handle = runtime.handle().clone();
    handle.spawn(platform::quit_on_signal());

    let restart = Arc::new(AtomicBool::new(false));
    let system =
        platform::SystemPlatform { runtime: handle.clone(), player: audio::Player::start(), restart: restart.clone() };
    core.mark_ui_thread();
    let thumbs = args.data_dir.join("cache/thumbs");
    let options = ShellOptions { dev: args.dev, monitor: args.monitor.clone(), thumbs: thumbs.clone() };
    let mut shell = runtime.block_on(AppShell::new(core.clone(), Rc::new(system), Arc::new(TokioClock), options));
    shell.ui.set_trailers_available(trailer::available());
    shell.ui.set_windowed(args.windowed);
    if let Some(size) = args::size_override(std::env::var(launcher_api::brand::env("SCALE")).ok().as_deref()) {
        shell.ui.set_size(size);
    }
    #[cfg(target_os = "macos")]
    shell.ui.set_fullscreen_with(Box::new(platform::simple_fullscreen));
    let theme_started = Instant::now();
    shell
        .ui
        .start_with(args.theme.clone())
        .map_err(|e| format!("no theme started, not even the fallback theme: {e}"))?;
    tracing::info!(theme_ms = theme_started.elapsed().as_millis(), "theme shown");
    on_first_frame(&shell, launched);
    handle.spawn_blocking(move || launcher_ui_host::sweep_thumbs(&thumbs));
    APP.with(|app| *app.borrow_mut() = Some(App { shell }));

    // Controllers: each press, and the controller's name for the launch splash. The event loop
    // refuses work only once the app closes, so a refusal drops the press.
    gamepad::spawn(
        |button, repeated| {
            let _ =
                slint_interpreter::invoke_from_event_loop(move || with_app(|app| app.shell.on_pad(button, repeated)));
        },
        |name| {
            let _ = slint_interpreter::invoke_from_event_loop(move || with_app(|app| app.shell.on_pad_name(name)));
        },
    );

    // Core events: at most one waiting on the UI thread; the shell coalesces the renders.
    let pending = Arc::new(AtomicBool::new(false));
    let mut events = core.events();
    handle.spawn(async move {
        while events.next().await.is_some() {
            if !pending.swap(true, Ordering::SeqCst) {
                let pending = pending.clone();
                // Refused only once the app closes: no render is due then.
                let _ = slint_interpreter::invoke_from_event_loop(move || {
                    pending.store(false, Ordering::SeqCst);
                    with_app(|app| app.shell.on_core_changed());
                });
            }
        }
    });

    let timer = slint::Timer::default();
    let mut loaded = false;
    let ticked = core.clone();
    timer.start(slint::TimerMode::Repeated, launcher_ui_host::TICK, move || {
        with_app(|app| app.shell.on_tick());
        if !loaded && ticked.booted() {
            loaded = true;
            tracing::info!(after_ms = launched.elapsed().as_millis(), "the first library load is done");
        }
    });

    // The first scan of the game folders, in the background: the first frame doesn't wait.
    {
        let _inside = handle.enter();
        core.scan_soon();
    }
    with_app(|app| {
        let rendered = Instant::now();
        if let Err(error) = app.shell.render() {
            tracing::error!(%error, "render failed");
        }
        tracing::info!(render_ms = rendered.elapsed().as_millis(), "the first render bound the view");
    });
    let window = slint::Timer::default();
    let timed = core.clone();
    window.start(slint::TimerMode::SingleShot, STARTUP_WINDOW, move || {
        with_app(|app| log_timings(&timed, &app.shell, launched));
    });

    slint_interpreter::run_event_loop().map_err(|e| e.to_string())?;
    tracing::info!("the window closed: quitting");
    // Whatever hangs from here on doesn't keep the app open: a library's thread, or a call on an
    // X connection the Marah Launcher session is taking away. Log out in the session waits for the app
    // to end, so a hang there froze the TV.
    std::thread::spawn(|| {
        std::thread::sleep(SHUTDOWN_LIMIT + EXIT_MARGIN);
        tracing::warn!("quitting took more than {} s: ending now", (SHUTDOWN_LIMIT + EXIT_MARGIN).as_secs());
        std::process::exit(0);
    });
    drop(timer);
    drop(window);
    APP.with(|app| app.borrow_mut().take());
    tracing::info!("the UI is closed");
    // While the runtime runs: the torrents write down their pieces on its tasks. A shutdown
    // that hangs doesn't keep the app open.
    if runtime.block_on(async { tokio::time::timeout(SHUTDOWN_LIMIT, core.shutdown()).await }).is_err() {
        tracing::warn!("shutting down took more than {} s; quitting anyway", SHUTDOWN_LIMIT.as_secs());
    }
    tracing::info!("shut down");
    // Restart to update: the next start puts the kept update in.
    if restart.load(Ordering::SeqCst) {
        update::restart();
    }
    Ok(())
}

/// Logs, once, how long after the start the window drew its first frame.
fn on_first_frame(shell: &AppShell, launched: Instant) {
    use slint_interpreter::ComponentHandle;
    let Some(instance) = shell.ui.instance() else { return };
    let mut drawn = false;
    let notifier = instance.window().set_rendering_notifier(move |state, _| {
        if !drawn && matches!(state, slint::RenderingState::AfterRendering) {
            drawn = true;
            tracing::info!(after_ms = launched.elapsed().as_millis(), "the first frame is drawn");
        }
    });
    if let Err(error) = notifier {
        tracing::debug!(%error, "the renderer doesn't say when it draws");
    }
}

/// Logs how the renders went since the start: how many, how long, the part in the disk reads,
/// and the UI thread's waits for the core's state lock.
fn log_timings(core: &Core, shell: &AppShell, launched: Instant) {
    let core = core.timings();
    let ui = shell.timings();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    tracing::info!(
        after_s = launched.elapsed().as_secs(),
        renders = ui.renders.count,
        render_mean_ms = ms(ui.renders.mean()),
        render_worst_ms = ms(ui.renders.worst),
        render_total_ms = ms(ui.renders.total),
        image_renders = ui.images.count,
        image_total_ms = ms(ui.images.total),
        views = core.views.count,
        view_mean_ms = ms(core.views.mean()),
        snapshot_mean_ms = ms(core.snapshots.mean()),
        snapshot_worst_ms = ms(core.snapshots.worst),
        files_mean_ms = ms(core.files.mean()),
        files_share = core.files.total.as_secs_f64() / ui.renders.total.as_secs_f64().max(f64::EPSILON),
        poll_worst_ms = ms(ui.polls.worst),
        tick_worst_ms = ms(ui.ticks.worst),
        lock_waits = core.ui_lock_waits.count,
        lock_wait_total_ms = ms(core.ui_lock_waits.total),
        lock_wait_worst_ms = ms(core.ui_lock_waits.worst),
        "the renders since the start"
    );
}

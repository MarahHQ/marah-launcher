use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use launcher_api::PluginBox;
use launcher_core::{Area, BootReport, Core, HostConfig, Kernel, KernelBuilder, Loader};
use tempfile::TempDir;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    FakeArchives, FakeClock, FakeHttp, FakeLoginSession, FakeNetwork, FakeOpener, FakeProcessTree, FakeSpawner,
    FakeTorrents, FakeWindows, StubPlugin,
};

/// How long [`TestCore::until`] waits on the fake clock. Its time moves only while every task
/// waits for a timer, so a slow machine doesn't use it up: only a wait for something that never
/// comes does.
const UNTIL_LIMIT: Duration = Duration::from_secs(60);

/// Builds a real core over a temporary data folder, with fake HTTP, fake processes and a
/// [`FakeClock`].
pub struct FakeHost {
    host: HostConfig,
    dir: TempDir,
    plugins: Vec<(PluginBox, bool)>,
    discover: Vec<std::path::PathBuf>,
    loaders: Vec<Arc<dyn Loader>>,
    http: FakeHttp,
    spawner: FakeSpawner,
    processes: FakeProcessTree,
    windows: FakeWindows,
    login_session: FakeLoginSession,
    opener: FakeOpener,
    torrents: FakeTorrents,
    network: FakeNetwork,
    archives: FakeArchives,
    hardware: Option<launcher_core::Specs>,
    controllers: Vec<launcher_api::Controller>,
    free_space: Option<u64>,
    /// Folders on a disk of their own, and its free bytes.
    disks: Vec<(PathBuf, u64)>,
    /// The computer's name, for the backups it makes.
    computer: String,
    first_run: bool,
    clock: FakeClock,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHost {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let mut host = HostConfig::new(dir.path());
        // Not the computer the tests run on: a host that runs everything, unless a test says.
        host.runs_32bit = true;
        // An installed launcher, not the test binary's own folder.
        host.install = launcher_core::launcher_update::Install::Folder;
        // Not an AppImage the tests might run from.
        host.appimage = None;
        // A desktop, not Marah OS, whatever the computer running the tests is.
        host.place = launcher_core::system::Place::default();
        // No display or sound of the computer running the tests in a sandboxed game.
        host.desktop = launcher_core::sandbox::Desktop::default();
        host.call_timeout = Duration::from_millis(500);
        host.shutdown_timeout = Duration::from_millis(200);
        host.stop_grace = Duration::from_millis(200);
        host.show_timeout = Duration::from_millis(500);
        host.hand_over_timeout = Duration::from_millis(500);
        host.watch_every = Duration::from_millis(20);
        host.torrent_poll = Duration::from_millis(10);
        host.store_poll = Duration::from_millis(10);
        host.vpn_poll = Duration::from_millis(10);
        // The test catalogs' topics carry no pictures: the Game Catalog's "Hide games without
        // images", on for a player, is off here, so they list. A test of the setting turns it on.
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"plugins": {"release-catalog": {"emulator": {"hide_without_images": false}}}}"#,
        )
        .expect("the test config");
        Self {
            host,
            dir,
            plugins: Vec::new(),
            discover: Vec::new(),
            loaders: Vec::new(),
            http: FakeHttp::default(),
            spawner: FakeSpawner::default(),
            processes: FakeProcessTree::default(),
            windows: FakeWindows::default(),
            login_session: FakeLoginSession::default(),
            opener: FakeOpener::default(),
            torrents: FakeTorrents::default(),
            network: FakeNetwork::default(),
            archives: FakeArchives::default(),
            hardware: None,
            controllers: Vec::new(),
            free_space: None,
            disks: Vec::new(),
            computer: String::new(),
            first_run: false,
            clock: FakeClock::default(),
        }
    }

    /// The archives the host unpacks, to say what each holds before the core starts: a
    /// download unpacks as soon as it finishes.
    pub fn archives(&self) -> FakeArchives {
        self.archives.clone()
    }

    /// The computer the "This PC" check sees. Without it the check finds nothing.
    pub fn hardware(mut self, specs: launcher_core::Specs) -> Self {
        self.hardware = Some(specs);
        self
    }

    /// The game controllers the computer has. Without it there are none.
    pub fn controllers(mut self, controllers: Vec<launcher_api::Controller>) -> Self {
        self.controllers = controllers;
        self
    }

    /// The free bytes every disk has. Without it the disks don't say.
    pub fn free_space(mut self, bytes: u64) -> Self {
        self.free_space = Some(bytes);
        self
    }

    /// `folder` and every folder in it are on a disk of their own, with `bytes` free. The
    /// first disk that holds a folder is its disk.
    pub fn disk(mut self, folder: impl Into<PathBuf>, bytes: u64) -> Self {
        self.disks.push((folder.into(), bytes));
        self
    }

    /// The computer's name, which its backups keep. Without it the system doesn't say.
    pub fn computer(mut self, name: &str) -> Self {
        self.computer = name.to_owned();
        self
    }

    pub fn plugin(mut self, plugin: StubPlugin) -> Self {
        self.plugins.push((plugin.boxed(), false));
        self
    }

    /// Any built-in plugin, such as a real emulator: `PluginBox::code(KytyPlugin)`.
    pub fn built_in(mut self, plugin: PluginBox) -> Self {
        self.plugins.push((plugin, false));
        self
    }

    pub fn third_party(mut self, plugin: PluginBox) -> Self {
        self.plugins.push((plugin, true));
        self
    }

    pub fn discover(mut self, dir: &Path) -> Self {
        self.discover.push(dir.to_path_buf());
        self
    }

    pub fn loader(mut self, loader: Arc<dyn Loader>) -> Self {
        self.loaders.push(loader);
        self
    }

    /// A first run: the player hasn't been welcomed yet. Otherwise the test host starts as a
    /// player who has been.
    pub fn first_run(mut self) -> Self {
        self.first_run = true;
        self
    }

    pub fn safe_mode(mut self) -> Self {
        self.host.safe_mode = true;
        self
    }

    /// The home folder for `~/` folder defaults.
    pub fn home(mut self, home: &Path) -> Self {
        self.host.home = Some(home.to_path_buf());
        self
    }

    /// The platform the host says it runs on (`None`: any).
    pub fn platform(mut self, platform: Option<launcher_api::Platform>) -> Self {
        self.host.platform = platform;
        self
    }

    /// Whether the host runs 32-bit Linux programs.
    pub fn runs_32bit(mut self, runs: bool) -> Self {
        self.host.runs_32bit = runs;
        self
    }

    /// How the launcher was installed, which decides how its update goes in.
    pub fn install(mut self, install: launcher_core::launcher_update::Install) -> Self {
        self.host.install = install;
        self
    }

    /// The running AppImage file, whose blocks a launcher update reuses.
    pub fn appimage(mut self, file: impl Into<std::path::PathBuf>) -> Self {
        self.host.appimage = Some(file.into());
        self
    }

    /// Where Marah Launcher runs: the whole screen, Marah OS, its USB stick.
    pub fn place(mut self, place: launcher_core::system::Place) -> Self {
        self.host.place = place;
        self
    }

    /// How often the core checks the VPN while downloads go only through it.
    pub fn vpn_poll(mut self, every: Duration) -> Self {
        self.host.vpn_poll = every;
        self
    }

    /// The computer's VPNs, before boot.
    pub fn network(&self) -> &FakeNetwork {
        &self.network
    }

    pub fn max_downloads(mut self, max: usize) -> Self {
        self.host.max_downloads = max;
        self
    }

    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.host.call_timeout = timeout;
        self
    }

    pub fn http(&self) -> &FakeHttp {
        &self.http
    }

    /// The clock the core's timers and its plugin calls' timeouts run on.
    pub fn clock(&self) -> FakeClock {
        self.clock.clone()
    }

    /// The data folder, before boot: for tests that set up files a previous run left.
    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    /// The desktop's windows the core sees.
    pub fn windows(&self) -> &FakeWindows {
        &self.windows
    }

    /// The login session the core starts the launcher at login and installs packages through.
    pub fn login_session(&self) -> &FakeLoginSession {
        &self.login_session
    }

    /// The opener the core opens plugins' links with.
    pub fn opener(&self) -> &FakeOpener {
        &self.opener
    }

    /// The kernel builder this host would boot, for tests that wire more on top, with the
    /// windows it plugs in.
    pub fn into_builder(self) -> (KernelBuilder, TempDir, FakeHttp, FakeSpawner, FakeWindows) {
        let mut builder = Kernel::builder(self.host)
            .http(Arc::new(self.http.clone()))
            .spawner(Arc::new(self.spawner.clone()))
            .processes(Arc::new(self.processes.clone()))
            .windows(Arc::new(self.windows.clone()))
            .login_session(Arc::new(self.login_session.clone()))
            .opener(Arc::new(self.opener.clone()))
            .torrents(Arc::new(self.torrents.clone()))
            .network(Arc::new(self.network.clone()))
            .archives(Arc::new(self.archives.clone()))
            .clock(Arc::new(self.clock.clone()));
        if self.hardware.is_some() || !self.computer.is_empty() || !self.controllers.is_empty() {
            let hardware = FixedHardware {
                specs: self.hardware.unwrap_or_default(),
                controllers: self.controllers,
                computer: self.computer,
            };
            builder = builder.hardware(Arc::new(hardware));
        }
        if self.free_space.is_some() || !self.disks.is_empty() {
            builder = builder.drives(Arc::new(FixedDrives { free_space: self.free_space, disks: self.disks }));
        }
        for (plugin, third_party) in self.plugins {
            builder = if third_party { builder.third_party(plugin) } else { builder.plugin(plugin) };
        }
        for dir in &self.discover {
            builder = builder.discover(dir);
        }
        for loader in self.loaders {
            builder = builder.loader(loader);
        }
        (builder, self.dir, self.http, self.spawner, self.windows)
    }

    /// Boots the kernel. From here on the runtime's time is the fake clock's.
    pub async fn boot(self) -> TestHost {
        self.clock.pause();
        let clock = self.clock.clone();
        let processes = self.processes.clone();
        let torrents = self.torrents.clone();
        let network = self.network.clone();
        let archives = self.archives.clone();
        let (login_session, opener) = (self.login_session.clone(), self.opener.clone());
        let (builder, dir, http, spawner, windows) = self.into_builder();
        let (kernel, report) = builder.boot().await;
        TestHost {
            kernel,
            report,
            http,
            spawner,
            processes,
            windows,
            login_session,
            opener,
            torrents,
            network,
            archives,
            clock,
            _dir: dir,
        }
    }

    /// Boots the kernel and starts the app on it.
    pub async fn boot_core(self) -> TestCore {
        let first_run = self.first_run;
        let host = self.boot().await;
        if !first_run {
            host.kernel.config().set(&["app", "welcomed"], serde_json::Value::Bool(true)).expect("config");
        }
        let core = Core::start(host.kernel.clone()).await;
        TestCore { core, host }
    }
}

/// A started app on a fake host.
pub struct TestCore {
    pub core: Core,
    pub host: TestHost,
}

impl TestCore {
    /// Returns once `done` holds: see [`until`].
    pub async fn until(&self, area: Area, done: impl FnMut(&Core) -> bool) {
        until(&self.core, area, done).await;
    }

    /// Moves the fake clock `step` at a time until `done`, at most `steps` times: for what no
    /// core change announces, such as a watcher's rounds or a plugin's files. Whether it is done.
    pub async fn until_clock(&self, step: std::time::Duration, steps: usize, mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..steps {
            if done() {
                return true;
            }
            self.clock.advance(step).await;
        }
        done()
    }
}

/// Returns once `done` holds for `core`. It asks at once, then after each change in `area`: no
/// sleeps. It panics when the fake clock passes [`UNTIL_LIMIT`] first.
pub async fn until(core: &Core, area: Area, mut done: impl FnMut(&Core) -> bool) {
    let mut changes = core.changes();
    let wait = async {
        loop {
            if done(core) {
                return;
            }
            loop {
                match changes.recv().await {
                    Ok(change) if change.area != area => continue,
                    Ok(_) | Err(RecvError::Lagged(_)) => break,
                    Err(RecvError::Closed) => unreachable!("the core holds its changes' sender"),
                }
            }
        }
    };
    if tokio::time::timeout(UNTIL_LIMIT, wait).await.is_err() {
        panic!("no change in {area:?} made it so in {UNTIL_LIMIT:?} of the fake clock");
    }
}

impl std::ops::Deref for TestCore {
    type Target = TestHost;
    fn deref(&self) -> &TestHost {
        &self.host
    }
}

/// A booted fake host. The data folder is deleted when it drops.
pub struct TestHost {
    pub kernel: Kernel,
    pub report: BootReport,
    pub http: FakeHttp,
    pub spawner: FakeSpawner,
    /// The running games' processes, beyond the one each started.
    pub processes: FakeProcessTree,
    pub windows: FakeWindows,
    pub login_session: FakeLoginSession,
    pub opener: FakeOpener,
    pub torrents: FakeTorrents,
    pub network: FakeNetwork,
    pub archives: FakeArchives,
    pub clock: FakeClock,
    _dir: TempDir,
}

impl TestHost {
    pub fn report(&self) -> &BootReport {
        &self.report
    }

    pub fn data_dir(&self) -> &Path {
        self._dir.path()
    }
}

/// A computer with these specs, these controllers, and this name.
struct FixedHardware {
    specs: launcher_core::Specs,
    controllers: Vec<launcher_api::Controller>,
    computer: String,
}

/// Drives with this much free space each, and the drives of their own.
struct FixedDrives {
    free_space: Option<u64>,
    disks: Vec<(PathBuf, u64)>,
}

thread_local! {
    /// The drives may not be asked on this thread now: see [`no_drive_calls`].
    static NO_DRIVE_CALLS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `work`, and panics if the drives of a [`FakeHost::free_space`] or [`FakeHost::disk`]
/// are asked anything on this thread meanwhile: for what must only read what was read ahead,
/// such as a build of the view. Reads on other threads (the core's blocking reads) go on.
pub fn no_drive_calls<T>(work: impl FnOnce() -> T) -> T {
    struct Allow;
    impl Drop for Allow {
        fn drop(&mut self) {
            NO_DRIVE_CALLS.set(false);
        }
    }
    NO_DRIVE_CALLS.set(true);
    let _allow = Allow;
    work()
}

impl FixedDrives {
    fn asked(&self) {
        assert!(!NO_DRIVE_CALLS.get(), "the drives were asked where only what was read ahead may be read");
    }
}

impl launcher_core::DriveSpace for FixedDrives {
    fn free_bytes(&self, path: &std::path::Path) -> Option<u64> {
        self.asked();
        self.disks.iter().find(|(disk, _)| path.starts_with(disk)).map(|(_, free)| *free).or(self.free_space)
    }

    /// A drive of its own is its place in the list; every other drive doesn't say.
    fn drive_id(&self, path: &std::path::Path) -> Option<u64> {
        self.asked();
        self.disks.iter().position(|(disk, _)| path.starts_with(disk)).map(|at| at as u64)
    }
}

impl launcher_core::Hardware for FixedHardware {
    fn specs(&self) -> launcher_core::Specs {
        self.specs.clone()
    }

    fn controllers(&self) -> Vec<launcher_api::Controller> {
        self.controllers.clone()
    }

    fn computer_name(&self) -> String {
        self.computer.clone()
    }
}

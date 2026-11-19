//! The machine's own settings, from the system plugins: network and Wi-Fi, Bluetooth, sound,
//! power and battery, and on Marah OS its updates and its installer. For each capability the
//! core uses the first active system plugin that provides it; without one, the capability is
//! unavailable. The state is read again every few seconds and after each action, and long work
//! (joining a network, scanning, pairing, installing) runs as a job the view model lists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use launcher_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Capability, InstallDisk, NetworkStatus, OsUpdate, PluginId,
    PluginKind, Provided, SystemJob, WifiNetwork,
};

use crate::changes::Area;
use crate::{Core, CoreError, CoreResult};

/// How often the state is read again while nothing happens.
pub(crate) const REFRESH_EVERY: Duration = Duration::from_secs(5);
/// How long a Bluetooth scan looks for devices.
const BLUETOOTH_SCAN_SECONDS: u32 = 10;
/// A finished job stays listed this long, so the theme can say how it ended.
const JOB_KEPT: Duration = Duration::from_secs(8);

/// Where Marah Launcher runs, which decides what the theme shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// Marah Launcher is the whole screen, with no desktop behind it: the Marah Launcher session or Marah OS.
    /// The network, sound and Bluetooth controls show then.
    pub whole_screen: bool,
    /// Marah OS (its marker, `/usr/lib/marah-os/release`).
    pub own_os: bool,
    /// Marah OS started from its USB stick, which can install it.
    pub stick: bool,
}

impl Place {
    /// This run's place: `MARAH_SESSION=1` (set by the Marah Launcher session), Marah OS's marker, and
    /// the stick's live kernel argument with its installer. `MARAH_OS=1` and `MARAH_USB=1`
    /// pretend, for screenshots.
    pub fn detect() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
        let own_os = set(&launcher_api::brand::env("OS")) || Path::new(launcher_api::brand::OS_RELEASE_MARKER).exists();
        let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
        let live = cmdline.split_whitespace().any(|a| a == "rd.live.image");
        let stick = set(&launcher_api::brand::env("USB")) || (own_os && live && Path::new(INSTALLER).exists());
        Place {
            whole_screen: set(&launcher_api::brand::env("SESSION")) || own_os || stick,
            own_os: own_os || stick,
            stick,
        }
    }
}

/// The installer on Marah OS's USB stick.
pub const INSTALLER: &str = launcher_api::brand::OS_INSTALLER;

/// What long work a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    JoinWifi,
    ScanBluetooth,
    Pair,
    InstallOs,
    Mount,
    Unmount,
    Eject,
}

impl JobKind {
    pub fn name(self) -> &'static str {
        match self {
            JobKind::JoinWifi => "join",
            JobKind::ScanBluetooth => "scan",
            JobKind::Pair => "pair",
            JobKind::InstallOs => "install",
            JobKind::Mount => "mount",
            JobKind::Unmount => "unmount",
            JobKind::Eject => "eject",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPhase {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug)]
pub struct JobState {
    pub id: u64,
    pub kind: JobKind,
    /// What it works on: the network's name, the device's address, the disk.
    pub target: String,
    /// The drive it works on, by its id, for a mount, an unmount or an eject; empty otherwise.
    pub drive: String,
    pub phase: JobPhase,
    pub percent: Option<u8>,
    pub text: String,
    pub(crate) cancel: Arc<AtomicBool>,
}

/// What the core knows of the machine.
#[derive(Clone, Debug, Default)]
pub struct SystemState {
    pub network: Option<NetworkStatus>,
    pub wifi: Vec<WifiNetwork>,
    pub bluetooth_on: bool,
    pub devices: Vec<BluetoothDevice>,
    pub outputs: Vec<AudioOutput>,
    pub level: Option<AudioLevel>,
    pub battery: Option<Battery>,
    pub os_update: Option<OsUpdate>,
    pub disks: Vec<InstallDisk>,
    /// Each capability a system plugin provides: `None` when it works, else why not.
    pub capabilities: BTreeMap<Capability, Option<String>>,
    /// Each capability whose first read ended, provided or not: the others are still loading.
    pub read: BTreeSet<Capability>,
    pub jobs: Vec<JobState>,
    pub(crate) next_job: u64,
    /// The lowest level a low battery notice warned of since the PC last charged.
    pub(crate) battery_warned: Option<u8>,
}

impl SystemState {
    /// Why `capability` can't be used: nobody provides it, or its plugin says why.
    pub fn unavailable(&self, capability: Capability) -> Option<String> {
        match self.capabilities.get(&capability) {
            Some(reason) => reason.clone(),
            None => Some(format!("{} can't change this on this system", launcher_api::brand::PRODUCT_NAME)),
        }
    }
}

/// The battery levels, highest first, at which a PC on battery tells the player, and what it says.
const LOW_BATTERY: [(u8, &str); 2] =
    [(20, "Battery at 20%. Plug in soon."), (10, "Battery at 10%. Save your game and plug in.")];

/// The low battery rule: after a read of `battery`, the lowest level warned of since the PC
/// last charged, and the notice to show now. Only charging warns again; a PC without a battery,
/// or a read that failed, changes nothing. A fall past both levels at once shows one notice.
fn low_battery(warned: Option<u8>, battery: Option<Battery>) -> (Option<u8>, Option<&'static str>) {
    let Some(battery) = battery else { return (warned, None) };
    if battery.charging {
        return (None, None);
    }
    match LOW_BATTERY.iter().rev().find(|(level, _)| battery.percent <= *level) {
        Some(&(level, text)) if warned.is_none_or(|w| level < w) => (Some(level), Some(text)),
        _ => (warned, None),
    }
}

/// The first active system plugin that provides a capability, and that capability.
pub(crate) async fn provider<T: ?Sized>(
    core: &Core,
    pick: impl Fn(&Provided) -> Option<Arc<T>>,
) -> Option<(PluginId, Arc<T>)> {
    let kernel = core.kernel();
    for plugin in kernel.of_kind(PluginKind::System) {
        if let Ok(provided) = kernel.provided(&plugin).await
            && let Some(capability) = pick(&provided)
        {
            return Some((plugin, capability));
        }
    }
    None
}

/// The job handle a plugin reports through: it changes the job in the state, and reads the
/// state again when the job ends.
struct HostJob {
    core: std::sync::Weak<crate::app::Inner>,
    id: u64,
    runtime: tokio::runtime::Handle,
    cancel: Arc<AtomicBool>,
}

impl HostJob {
    fn change(&self, change: impl FnOnce(&mut JobState)) {
        let Some(inner) = self.core.upgrade() else { return };
        let core = Core { inner };
        let ended = {
            let mut state = core.state();
            let Some(job) = state.system.jobs.iter_mut().find(|j| j.id == self.id) else { return };
            if job.phase != JobPhase::Running {
                return;
            }
            change(job);
            job.phase != JobPhase::Running
        };
        core.changed(Area::System);
        if ended {
            let id = self.id;
            self.runtime.spawn(async move {
                refresh(&core).await;
                core.ports().clock.sleep(JOB_KEPT).await;
                core.state().system.jobs.retain(|j| j.id != id || j.phase == JobPhase::Running);
                core.changed(Area::System);
            });
        }
    }
}

impl SystemJob for HostJob {
    fn progress(&self, percent: Option<u8>, text: &str) {
        self.change(|job| {
            job.percent = percent.map(|p| p.min(100));
            job.text = text.to_owned();
        });
    }

    fn done(&self, text: &str) {
        self.change(|job| {
            job.phase = JobPhase::Done;
            job.percent = Some(100);
            job.text = text.to_owned();
        });
    }

    fn failed(&self, reason: &str) {
        self.change(|job| {
            job.phase = JobPhase::Failed;
            job.text = reason.to_owned();
        });
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// A new running job, and the handle its plugin reports through.
pub(crate) fn start_job(core: &Core, kind: JobKind, target: &str) -> (u64, Arc<dyn SystemJob>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let id = {
        let mut state = core.state();
        state.system.next_job += 1;
        let id = state.system.next_job;
        state.system.jobs.push(JobState {
            id,
            kind,
            target: target.to_owned(),
            drive: String::new(),
            phase: JobPhase::Running,
            percent: None,
            text: String::new(),
            cancel: cancel.clone(),
        });
        id
    };
    core.changed(Area::System);
    let job = HostJob { core: Arc::downgrade(&core.inner), id, runtime: tokio::runtime::Handle::current(), cancel };
    (id, Arc::new(job))
}

/// A job that couldn't start fails with the reason.
fn failed_to_start(job: &Arc<dyn SystemJob>, error: &CoreError) {
    job.failed(&error.to_string());
}

/// The VPNs come from the system plugin that provides them, if one does; the app's port stays
/// otherwise. Called at boot, before downloads resume, and at each read.
pub(crate) async fn adopt_vpn(core: &Core) {
    if let Some((_, vpn)) = provider(core, |p| p.vpn.clone()).await {
        core.vpn().use_network(Arc::new(crate::ports::PluginVpn(vpn)));
    }
}

/// Reads every system capability again, all at once. Each part tells the view as soon as it is
/// read, so a slow one (a network tool that hangs) holds none of the others.
pub(crate) async fn refresh(core: &Core) {
    futures::join!(
        adopt_vpn(core),
        read_network(core),
        read_bluetooth(core),
        read_audio(core),
        read_power(core),
        read_os_updates(core),
        read_os_install(core),
        read_drives(core),
    );
}

/// Keeps what one part read. `reason` is `None` when no plugin provides `capability`, else why it
/// can't be used (`Some(None)` when it can). `write` puts the part's state in and says whether
/// it changed. Says whether the view must hear of it: the part changed, or this was its first
/// read.
fn publish(
    core: &Core,
    capability: Capability,
    reason: Option<Option<String>>,
    write: impl FnOnce(&mut SystemState) -> bool,
) -> bool {
    let mut state = core.state();
    let system = &mut state.system;
    let first = system.read.insert(capability);
    let before = system.capabilities.get(&capability).cloned();
    match reason {
        Some(why) => system.capabilities.insert(capability, why),
        None => system.capabilities.remove(&capability),
    };
    let wrote = write(system);
    first || wrote || system.capabilities.get(&capability) != before.as_ref()
}

/// Sets `field` to `value`, and says whether it changed.
fn set<T: PartialEq>(field: &mut T, value: T) -> bool {
    let changed = *field != value;
    *field = value;
    changed
}

/// Tells the view of a part that changed.
fn tell(core: &Core, changed: bool) {
    if changed {
        core.changed(Area::System);
    }
}

async fn read_network(core: &Core) {
    let (reason, status, wifi) = match provider(core, |p| p.network.clone()).await {
        None => (None, None, Vec::new()),
        Some((plugin, network)) => {
            let kernel = core.kernel();
            let reason = kernel.guard(&plugin, "network.unavailable", async { Ok(network.unavailable().await) }).await;
            let reason = reason.unwrap_or(None);
            if reason.is_none() {
                let (status, wifi) = futures::join!(
                    kernel.guard(&plugin, "network.status", network.status()),
                    kernel.guard(&plugin, "network.wifi", network.wifi(false)),
                );
                (Some(reason), status.ok(), wifi.unwrap_or_default())
            } else {
                (Some(reason), None, Vec::new())
            }
        }
    };
    let changed = publish(core, Capability::Network, reason, |system| {
        // A scan found more networks than the plain read: keep them until the next scan.
        let more = !wifi.is_empty() && set(&mut system.wifi, wifi);
        set(&mut system.network, status) | more
    });
    tell(core, changed);
}

async fn read_bluetooth(core: &Core) {
    let (reason, on, devices) = match provider(core, |p| p.bluetooth.clone()).await {
        None => (None, false, Vec::new()),
        Some((plugin, bluetooth)) => {
            let kernel = core.kernel();
            let reason =
                kernel.guard(&plugin, "bluetooth.unavailable", async { Ok(bluetooth.unavailable().await) }).await;
            let reason = reason.unwrap_or(None);
            if reason.is_none() {
                let (on, devices) = futures::join!(
                    kernel.guard(&plugin, "bluetooth.powered", bluetooth.powered()),
                    kernel.guard(&plugin, "bluetooth.devices", bluetooth.devices()),
                );
                (Some(reason), on.unwrap_or(false), devices.unwrap_or_default())
            } else {
                (Some(reason), false, Vec::new())
            }
        }
    };
    let changed = publish(core, Capability::Bluetooth, reason, |system| {
        set(&mut system.bluetooth_on, on) | set(&mut system.devices, devices)
    });
    tell(core, changed);
}

async fn read_audio(core: &Core) {
    let (reason, outputs, level) = match provider(core, |p| p.audio.clone()).await {
        None => (None, Vec::new(), None),
        Some((plugin, audio)) => {
            let kernel = core.kernel();
            let reason = kernel.guard(&plugin, "audio.unavailable", async { Ok(audio.unavailable().await) }).await;
            let reason = reason.unwrap_or(None);
            if reason.is_none() {
                let (outputs, level) = futures::join!(
                    kernel.guard(&plugin, "audio.outputs", audio.outputs()),
                    kernel.guard(&plugin, "audio.level", audio.level()),
                );
                (Some(reason), outputs.unwrap_or_default(), level.ok())
            } else {
                (Some(reason), Vec::new(), None)
            }
        }
    };
    let changed = publish(core, Capability::Audio, reason, |system| {
        set(&mut system.outputs, outputs) | set(&mut system.level, level)
    });
    tell(core, changed);
}

async fn read_power(core: &Core) {
    let (reason, battery) = match provider(core, |p| p.power.clone()).await {
        None => (None, None),
        Some((plugin, power)) => {
            let kernel = core.kernel();
            let (reason, battery) = futures::join!(
                kernel.guard(&plugin, "power.unavailable", async { Ok(power.unavailable().await) }),
                kernel.guard(&plugin, "power.battery", power.battery()),
            );
            (Some(reason.unwrap_or(None)), battery.ok().flatten())
        }
    };
    let mut notice = None;
    let changed = publish(core, Capability::Power, reason, |system| {
        (system.battery_warned, notice) = low_battery(system.battery_warned, battery);
        set(&mut system.battery, battery)
    });
    // Before the change: a notice tells only the core's readers, and the change tells the app.
    if let Some(text) = notice {
        core.notices().info(text.to_owned());
    }
    tell(core, changed);
}

async fn read_os_updates(core: &Core) {
    let (reason, status) = match provider(core, |p| p.os_updates.clone()).await {
        None => (None, None),
        Some((plugin, updates)) => {
            (Some(None), core.kernel().guard(&plugin, "os_updates.status", updates.status()).await.ok())
        }
    };
    let changed = publish(core, Capability::OsUpdates, reason, |system| set(&mut system.os_update, status));
    tell(core, changed);
}

async fn read_os_install(core: &Core) {
    let found = match core.kernel().host().place.stick {
        true => provider(core, |p| p.os_install.clone()).await,
        false => None,
    };
    let (reason, disks) = match found {
        None => (None, Vec::new()),
        Some((plugin, install)) => {
            (Some(None), core.kernel().guard(&plugin, "os_install.disks", install.disks()).await.unwrap_or_default())
        }
    };
    let changed = publish(core, Capability::OsInstall, reason, |system| set(&mut system.disks, disks));
    tell(core, changed);
}

/// Only whether drives can be managed here: the drives themselves come from
/// [`crate::drives::refresh`], which takes longer.
async fn read_drives(core: &Core) {
    let reason = match provider(core, |p| p.drives.clone()).await {
        None => None,
        Some((plugin, drives)) => {
            let reason = core.kernel().guard(&plugin, "drives.unavailable", async { Ok(drives.unavailable().await) });
            Some(reason.await.unwrap_or(None))
        }
    };
    let changed = publish(core, Capability::Drives, reason, |_| false);
    tell(core, changed);
}

/// Reads the state again every [`REFRESH_EVERY`]: each part on its own, so a slow part makes
/// none of the others wait, now or at the next tick.
pub(crate) fn schedule_refresh(core: &Core) {
    every_tick(core, |core| async move { adopt_vpn(&core).await });
    every_tick(core, |core| async move { read_network(&core).await });
    every_tick(core, |core| async move { read_bluetooth(&core).await });
    every_tick(core, |core| async move { read_audio(&core).await });
    every_tick(core, |core| async move { read_power(&core).await });
    every_tick(core, |core| async move { read_os_updates(&core).await });
    every_tick(core, |core| async move { read_os_install(&core).await });
    every_tick(core, |core| async move { read_drives(&core).await });
    every_tick(core, |core| async move { crate::drives::refresh(&core).await });
    every_tick(core, |core| async move { crate::disks::refresh(&core).await });
}

/// Runs `read` now, then again [`REFRESH_EVERY`] after each one ends, while the core lives.
fn every_tick<F, R>(core: &Core, read: F)
where
    F: Fn(Core) -> R + Send + 'static,
    R: std::future::Future<Output = ()> + Send,
{
    let weak = Arc::downgrade(&core.inner);
    tokio::spawn(async move {
        loop {
            let Some(inner) = weak.upgrade() else { return };
            let core = Core { inner };
            read(core.clone()).await;
            let wait = core.ports().clock.sleep(REFRESH_EVERY);
            drop(core);
            wait.await;
        }
    });
}

/// What the player asks of the machine.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SystemCommand {
    /// Reads everything again now.
    Refresh,
    /// Looks for Wi-Fi networks again (a few seconds).
    ScanWifi,
    /// Joins a Wi-Fi network, with its password when it's secured and not saved.
    JoinWifi {
        ssid: String,
        password: Option<String>,
    },
    ForgetWifi(String),
    SetBluetooth(bool),
    /// Looks for devices to pair.
    ScanBluetooth,
    Pair(String),
    ConnectDevice(String),
    DisconnectDevice(String),
    RemoveDevice(String),
    SetOutput(String),
    /// 0 to 100.
    SetVolume(u8),
    SetMuted(bool),
    Restart,
    ShutDown,
    /// Closes the running games, then lets the launcher go ([`crate::Core::log_out_ready`]): the
    /// app quits, and the Marah Launcher session ends with it.
    LogOut,
    /// Asks Marah OS to look for an update now.
    CheckOsUpdate,
    /// Erases the disk and installs Marah OS on it (the USB stick only).
    InstallOs(PathBuf),
    /// Starts the installer that lets the player choose partitions.
    ChoosePartitions,
    /// Cancels a running job, by its id.
    CancelJob(u64),
}

/// Restart or Shut down: the games close first, only once there is a way to do it and the
/// system doesn't refuse it here. When the system then fails, the closing screen goes.
async fn power_off(core: &Core, purpose: crate::closing::Purpose) -> CoreResult<()> {
    let (plugin, power) = provider(core, |p| p.power.clone()).await.ok_or(missing(Capability::Power))?;
    if let Some(why) =
        core.kernel().guard(&plugin, "power.unavailable", async { Ok(power.unavailable().await) }).await?
    {
        return Err(CoreError::Invalid(why));
    }
    crate::closing::close_everything(core, purpose).await;
    let done = match purpose {
        crate::closing::Purpose::ShutDown => core.kernel().guard(&plugin, "power.shut_down", power.shut_down()).await,
        _ => core.kernel().guard(&plugin, "power.restart", power.restart()).await,
    };
    if done.is_err() {
        crate::closing::cancelled(core);
    }
    done
}

fn missing(capability: Capability) -> CoreError {
    CoreError::Invalid(format!("no system plugin provides {capability:?} here"))
}

pub(crate) async fn run(core: &Core, command: SystemCommand) -> CoreResult<()> {
    use SystemCommand as C;
    match command {
        C::Refresh => {}
        C::ScanWifi => {
            let (plugin, network) = provider(core, |p| p.network.clone()).await.ok_or(missing(Capability::Network))?;
            let wifi = core.kernel().guard(&plugin, "network.wifi", network.wifi(true)).await?;
            core.state().system.wifi = wifi;
            core.changed(Area::System);
            return Ok(());
        }
        C::JoinWifi { ssid, password } => {
            let (plugin, network) = provider(core, |p| p.network.clone()).await.ok_or(missing(Capability::Network))?;
            let (_, job) = start_job(core, JobKind::JoinWifi, &ssid);
            if let Err(e) = core
                .kernel()
                .guard(&plugin, "network.join", network.join(&ssid, password.as_deref(), job.clone()))
                .await
            {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::ForgetWifi(ssid) => {
            let (plugin, network) = provider(core, |p| p.network.clone()).await.ok_or(missing(Capability::Network))?;
            core.kernel().guard(&plugin, "network.forget", network.forget(&ssid)).await?;
        }
        C::SetBluetooth(on) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            core.kernel().guard(&plugin, "bluetooth.set_powered", bt.set_powered(on)).await?;
        }
        C::ScanBluetooth => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            let (_, job) = start_job(core, JobKind::ScanBluetooth, "");
            if let Err(e) =
                core.kernel().guard(&plugin, "bluetooth.scan", bt.scan(BLUETOOTH_SCAN_SECONDS, job.clone())).await
            {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::Pair(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            let (_, job) = start_job(core, JobKind::Pair, &address);
            if let Err(e) = core.kernel().guard(&plugin, "bluetooth.pair", bt.pair(&address, job.clone())).await {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::ConnectDevice(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            core.kernel().guard(&plugin, "bluetooth.connect", bt.connect(&address)).await?;
        }
        C::DisconnectDevice(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            core.kernel().guard(&plugin, "bluetooth.disconnect", bt.disconnect(&address)).await?;
        }
        C::RemoveDevice(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            core.kernel().guard(&plugin, "bluetooth.remove", bt.remove(&address)).await?;
        }
        C::SetOutput(name) => {
            let (plugin, audio) = provider(core, |p| p.audio.clone()).await.ok_or(missing(Capability::Audio))?;
            core.kernel().guard(&plugin, "audio.set_output", audio.set_output(&name)).await?;
        }
        C::SetVolume(volume) => {
            let (plugin, audio) = provider(core, |p| p.audio.clone()).await.ok_or(missing(Capability::Audio))?;
            let volume = volume.min(100);
            core.kernel().guard(&plugin, "audio.set_volume", audio.set_volume(volume)).await?;
            // Shown at once: the next read confirms it.
            if let Some(level) = core.state().system.level.as_mut() {
                level.volume = volume;
                level.muted = false;
            }
            core.changed(Area::System);
        }
        C::SetMuted(muted) => {
            let (plugin, audio) = provider(core, |p| p.audio.clone()).await.ok_or(missing(Capability::Audio))?;
            core.kernel().guard(&plugin, "audio.set_muted", audio.set_muted(muted)).await?;
            if let Some(level) = core.state().system.level.as_mut() {
                level.muted = muted;
            }
            core.changed(Area::System);
        }
        C::Restart => return power_off(core, crate::closing::Purpose::Restart).await,
        C::ShutDown => return power_off(core, crate::closing::Purpose::ShutDown).await,
        C::LogOut => {
            crate::closing::log_out(core).await;
            return Ok(());
        }
        C::CheckOsUpdate => {
            let (plugin, updates) =
                provider(core, |p| p.os_updates.clone()).await.ok_or(missing(Capability::OsUpdates))?;
            core.kernel().guard(&plugin, "os_updates.check_now", updates.check_now()).await?;
        }
        C::InstallOs(disk) => {
            if !core.kernel().host().place.stick {
                return Err(CoreError::Invalid(format!(
                    "{} installs from its USB stick only",
                    launcher_api::brand::PRODUCT_OS_NAME
                )));
            }
            let (plugin, install) =
                provider(core, |p| p.os_install.clone()).await.ok_or(missing(Capability::OsInstall))?;
            let (_, job) = start_job(core, JobKind::InstallOs, &disk.display().to_string());
            if let Err(e) =
                core.kernel().guard(&plugin, "os_install.install", install.install(&disk, job.clone())).await
            {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::ChoosePartitions => {
            let (plugin, install) =
                provider(core, |p| p.os_install.clone()).await.ok_or(missing(Capability::OsInstall))?;
            return core.kernel().guard(&plugin, "os_install.choose_partitions", install.choose_partitions()).await;
        }
        C::CancelJob(id) => {
            let state = core.state();
            let job = state.system.jobs.iter().find(|j| j.id == id && j.phase == JobPhase::Running);
            let job = job.ok_or_else(|| CoreError::Invalid(format!("no running job {id}")))?;
            job.cancel.store(true, Ordering::SeqCst);
            return Ok(());
        }
    }
    refresh(core).await;
    Ok(())
}

/// Looks for the launcher's update. Marah OS brings the new launcher with its next update:
/// then it looks for that too.
pub(crate) async fn check_launcher_update(core: &Core) -> CoreResult<()> {
    let found = core.launcher_update().check().await;
    if matches!(found, crate::launcher_update::UpdateState::WithSystem { .. }) {
        let core = core.clone();
        tokio::spawn(async move {
            if let Err(e) = run(&core, SystemCommand::CheckOsUpdate).await {
                tracing::warn!(error = %e, "could not look for the operating system's update");
            }
        });
    }
    Ok(())
}

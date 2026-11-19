//! The machine's own settings, from the system plugins: network and Wi-Fi, VPNs, Bluetooth,
//! sound, power and battery, drives, and on Marah OS its updates and its installer. Each is a
//! system area ([`SystemArea`]) behind the same interface. For each area the core uses the first
//! active system plugin that provides it; without one, the area is absent. One reader
//! ([`read`]) reads every area again every few seconds and after each action, and keeps what
//! it read; an area is loading until its first read ends. Long work (joining a network,
//! scanning, pairing, installing, mounting) runs as a job the view model lists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, Battery, PluginId, PluginKind, SystemArea, SystemJob,
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
    /// The area the work runs in.
    pub fn area(self) -> SystemArea {
        match self {
            JobKind::JoinWifi => SystemArea::Network,
            JobKind::ScanBluetooth | JobKind::Pair => SystemArea::Bluetooth,
            JobKind::InstallOs => SystemArea::OsInstall,
            JobKind::Mount | JobKind::Unmount | JobKind::Eject => SystemArea::Drives,
        }
    }

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
    /// Each area as its last read left it. An area that isn't here is loading: its first read
    /// hasn't ended. Loading lives here, in the core's reader, and no plugin reports it.
    pub areas: BTreeMap<SystemArea, AreaState>,
    pub jobs: Vec<JobState>,
    pub(crate) next_job: u64,
    /// The lowest level a low battery notice warned of since the PC last charged.
    pub(crate) battery_warned: Option<u8>,
}

impl SystemState {
    /// The area as its last read left it; `None` while it is loading.
    pub fn area(&self, area: SystemArea) -> Option<&AreaState> {
        self.areas.get(&area)
    }

    /// The area's data, while it is ready.
    pub fn data(&self, area: SystemArea) -> Option<&AreaData> {
        match self.areas.get(&area) {
            Some(AreaState::Ready(data)) => Some(data),
            _ => None,
        }
    }
}

/// What an absent area says: nobody provides it here. On Linux the drives need UDisks, which
/// the system plugin uses.
pub(crate) fn absent(area: SystemArea) -> String {
    match area {
        SystemArea::Drives if cfg!(target_os = "linux") => crate::drives::CANT_ACT.into(),
        _ => format!("{} can't change this on this system", launcher_api::brand::PRODUCT_NAME),
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

/// The first active system plugin that provides `area`, and its adapter for it.
pub(crate) async fn provider(core: &Core, area: SystemArea) -> Option<(PluginId, Arc<dyn AreaSettings>)> {
    let kernel = core.kernel();
    for plugin in kernel.of_kind(PluginKind::System) {
        if let Ok(provided) = kernel.provided(&plugin).await
            && let Some(settings) = provided.areas.get(&area)
        {
            return Some((plugin, settings.clone()));
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

/// A job for a quick action, which ends before the action returns: nobody follows it.
pub(crate) struct QuietJob;

impl SystemJob for QuietJob {
    fn progress(&self, _: Option<u8>, _: &str) {}
    fn done(&self, _: &str) {}
    fn failed(&self, _: &str) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// The kernel's names for the calls on an area, for its spans and its errors.
fn method(area: SystemArea, read: bool) -> &'static str {
    match (area, read) {
        (SystemArea::Network, true) => "network.state",
        (SystemArea::Network, false) => "network.act",
        (SystemArea::Vpn, true) => "vpn.state",
        (SystemArea::Vpn, false) => "vpn.act",
        (SystemArea::Bluetooth, true) => "bluetooth.state",
        (SystemArea::Bluetooth, false) => "bluetooth.act",
        (SystemArea::Audio, true) => "audio.state",
        (SystemArea::Audio, false) => "audio.act",
        (SystemArea::Power, true) => "power.state",
        (SystemArea::Power, false) => "power.act",
        (SystemArea::OsUpdates, true) => "os_updates.state",
        (SystemArea::OsUpdates, false) => "os_updates.act",
        (SystemArea::OsInstall, true) => "os_install.state",
        (SystemArea::OsInstall, false) => "os_install.act",
        (SystemArea::Drives, true) => "drives.state",
        (SystemArea::Drives, false) => "drives.act",
    }
}

/// The VPNs come from the system plugin that provides them, if one does; the app's port stays
/// otherwise. At boot, before downloads resume, it reads nothing of the machine: the VPN's
/// facts come with its first read ([`crate::ports::PluginVpn`]).
pub(crate) async fn adopt_vpn(core: &Core) {
    if let Some((_, settings)) = provider(core, SystemArea::Vpn).await {
        core.vpn().use_network(Arc::new(crate::ports::PluginVpn::new(settings, None)));
    }
}

/// Reads every area again, all at once, but the drives: their read waits for a turn of the disk,
/// and a drive command or the drives' own tick reads them ([`crate::drives::refresh`]). Each
/// area tells the view as soon as it is read, so a slow one (a network tool that hangs) holds
/// none of the others.
pub(crate) async fn refresh(core: &Core) {
    let areas = SystemArea::ALL.into_iter().filter(|a| *a != SystemArea::Drives);
    futures::future::join_all(areas.map(|area| read(core, area))).await;
}

/// Asks the area's plugin what the area is now. An area nobody provides is absent, and so is
/// Marah OS's installer anywhere but on its USB stick. `Err` when the read failed.
async fn ask(core: &Core, area: SystemArea) -> CoreResult<(AreaState, Option<Arc<dyn AreaSettings>>)> {
    if area == SystemArea::OsInstall && !core.kernel().host().place.stick {
        return Ok((AreaState::Absent, None));
    }
    let Some((plugin, settings)) = provider(core, area).await else { return Ok((AreaState::Absent, None)) };
    let state = core.kernel().guard(&plugin, method(area, true), settings.state()).await?;
    Ok((state, Some(settings)))
}

/// The one reader of the machine: reads `area` and keeps what it read. A read that fails
/// changes nothing; when it was the area's first, the area is unsupported, with the error, so
/// it doesn't stay loading. Then each area's own rule runs: the low battery notice, the VPNs for
/// the downloads, and the drives' list.
pub(crate) async fn read(core: &Core, area: SystemArea) {
    let rereads = core.drives().rereads();
    let asked = ask(core, area).await;
    let mut notice = None;
    let (changed, now) = {
        let mut state = core.state();
        let system = &mut state.system;
        let before = system.areas.get(&area).cloned();
        let now = match (&asked, &before) {
            (Ok((now, _)), _) => keep_wifi(before.as_ref(), now.clone()),
            (Err(e), Some(before)) => {
                tracing::debug!(area = area.name(), error = %e, "the area wasn't read: it stays as it was");
                before.clone()
            }
            (Err(e), None) => AreaState::Unsupported(format!("Couldn't read it: {e}")),
        };
        if area == SystemArea::Power && asked.is_ok() {
            let battery = match &now {
                AreaState::Ready(AreaData::Power { battery, .. }) => *battery,
                _ => None,
            };
            (system.battery_warned, notice) = low_battery(system.battery_warned, battery);
        }
        system.areas.insert(area, now.clone());
        (before.as_ref() != Some(&now), now)
    };
    // Before the change: a notice tells only the core's readers, and the change tells the app.
    if let Some(text) = notice {
        core.notices().info(text.to_owned());
    }
    if changed {
        core.changed(Area::System);
    }
    match area {
        SystemArea::Vpn => {
            if let Ok((_, Some(settings))) = asked {
                core.vpn().use_network(Arc::new(crate::ports::PluginVpn::new(settings, Some(&now))));
            }
            if changed {
                core.changed(Area::Vpn);
            }
        }
        SystemArea::Drives => crate::drives::listed(core, rereads).await,
        _ => {}
    }
}

/// An empty Wi-Fi list, read while the radio is busy, keeps the networks of the last read.
fn keep_wifi(before: Option<&AreaState>, mut now: AreaState) -> AreaState {
    if let (
        Some(AreaState::Ready(AreaData::Network { wifi: kept, .. })),
        AreaState::Ready(AreaData::Network { wifi, .. }),
    ) = (before, &mut now)
        && wifi.is_empty()
    {
        wifi.clone_from(kept);
    }
    now
}

/// Reads each area again every [`REFRESH_EVERY`]: each on its own, so a slow area makes none of
/// the others wait, now or at the next tick.
pub(crate) fn schedule_refresh(core: &Core) {
    for area in SystemArea::ALL {
        every_tick(core, move |core| async move { read(&core, area).await });
    }
    every_tick(core, |core| async move { crate::disks::refresh(&core).await });
    every_tick(core, |core| async move { crate::snapshot::refresh_files(&core).await });
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
    /// Reads every area again now but the drives, which `DriveCmd::Refresh` reads.
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
    let area = SystemArea::Power;
    let (plugin, power) = provider(core, area).await.ok_or(missing(area))?;
    match core.kernel().guard(&plugin, method(area, true), power.state()).await? {
        AreaState::Ready(AreaData::Power { refused: None, .. }) => {}
        AreaState::Ready(AreaData::Power { refused: Some(why), .. }) | AreaState::Unsupported(why) => {
            return Err(CoreError::Invalid(why));
        }
        _ => return Err(missing(area)),
    }
    crate::closing::close_everything(core, purpose).await;
    let action = match purpose {
        crate::closing::Purpose::ShutDown => AreaAction::ShutDown,
        _ => AreaAction::Restart,
    };
    let done = core.kernel().guard(&plugin, method(area, false), power.act(action, Arc::new(QuietJob))).await;
    if done.is_err() {
        crate::closing::cancelled(core);
    }
    done
}

fn missing(area: SystemArea) -> CoreError {
    CoreError::Invalid(format!("no system plugin provides {} here", area.name()))
}

/// Runs a quick action on its area, through the first plugin that provides the area.
pub(crate) async fn act(core: &Core, action: AreaAction) -> CoreResult<()> {
    let area = action.area();
    let (plugin, settings) = provider(core, area).await.ok_or(missing(area))?;
    core.kernel().guard(&plugin, method(area, false), settings.act(action, Arc::new(QuietJob))).await
}

/// Starts a long action on its area as a job the view lists. A job that can't start fails
/// with the reason.
async fn start(core: &Core, kind: JobKind, target: &str, action: AreaAction) -> CoreResult<()> {
    let area = action.area();
    let (plugin, settings) = provider(core, area).await.ok_or(missing(area))?;
    let (_, job) = start_job(core, kind, target);
    if let Err(e) = core.kernel().guard(&plugin, method(area, false), settings.act(action, job.clone())).await {
        failed_to_start(&job, &e);
    }
    Ok(())
}

/// Changes the area's data at once, as an action just did: the next read confirms it.
fn shown_at_once(core: &Core, area: SystemArea, change: impl FnOnce(&mut AreaData)) {
    if let Some(AreaState::Ready(data)) = core.state().system.areas.get_mut(&area) {
        change(data);
    }
    core.changed(Area::System);
}

pub(crate) async fn run(core: &Core, command: SystemCommand) -> CoreResult<()> {
    use AreaAction as A;
    use SystemCommand as C;
    match command {
        C::Refresh => {}
        C::ScanWifi => act(core, A::ScanWifi).await?,
        C::JoinWifi { ssid, password } => {
            return start(core, JobKind::JoinWifi, &ssid.clone(), A::JoinWifi { ssid, password }).await;
        }
        C::ForgetWifi(ssid) => act(core, A::ForgetWifi { ssid }).await?,
        C::SetBluetooth(on) => act(core, A::SetBluetooth { on }).await?,
        C::ScanBluetooth => {
            return start(core, JobKind::ScanBluetooth, "", A::ScanBluetooth { seconds: BLUETOOTH_SCAN_SECONDS }).await;
        }
        C::Pair(address) => return start(core, JobKind::Pair, &address.clone(), A::Pair { address }).await,
        C::ConnectDevice(address) => act(core, A::ConnectDevice { address }).await?,
        C::DisconnectDevice(address) => act(core, A::DisconnectDevice { address }).await?,
        C::RemoveDevice(address) => act(core, A::RemoveDevice { address }).await?,
        C::SetOutput(name) => act(core, A::SetOutput { name }).await?,
        C::SetVolume(volume) => {
            let volume = volume.min(100);
            act(core, A::SetVolume { volume }).await?;
            shown_at_once(core, SystemArea::Audio, |data| {
                if let AreaData::Audio { level: Some(level), .. } = data {
                    level.volume = volume;
                    level.muted = false;
                }
            });
        }
        C::SetMuted(muted) => {
            act(core, A::SetMuted { muted }).await?;
            shown_at_once(core, SystemArea::Audio, |data| {
                if let AreaData::Audio { level: Some(level), .. } = data {
                    level.muted = muted;
                }
            });
        }
        C::Restart => return power_off(core, crate::closing::Purpose::Restart).await,
        C::ShutDown => return power_off(core, crate::closing::Purpose::ShutDown).await,
        C::LogOut => {
            crate::closing::log_out(core).await;
            return Ok(());
        }
        C::CheckOsUpdate => act(core, A::CheckOsUpdate).await?,
        C::InstallOs(disk) => {
            if !core.kernel().host().place.stick {
                return Err(CoreError::Invalid(format!(
                    "{} installs from its USB stick only",
                    launcher_api::brand::PRODUCT_OS_NAME
                )));
            }
            return start(core, JobKind::InstallOs, &disk.display().to_string(), A::InstallOs { disk }).await;
        }
        C::ChoosePartitions => return act(core, A::ChoosePartitions).await,
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

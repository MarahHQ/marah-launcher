//! Linux: NetworkManager (`nmcli`), BlueZ (`bluetoothctl`), PipeWire's PulseAudio server
//! (`pactl`), systemd (`systemctl`), the kernel's power supply folder, UDisks (`udisksctl`, with
//! `lsblk` for the list), and on Marah OS
//! `rpm-ostree` and the USB stick's installer. Every program runs through the host's
//! [`Programs`], so the manifest's `permissions.process` decides what may run.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection,
    InstallDisk, Location, NetworkStatus, OsUpdate, PluginError, PluginResult, Programs, SystemArea, SystemDrive,
    SystemJob, WifiNetwork,
};

use crate::parse;

/// How long a quick read may take: the host's own limit.
const QUICK: Duration = launcher_api::MAX_PROGRAM_TIME;
/// Marah OS's marker, and its USB stick's installer.
const MARAH_OS: &str = launcher_api::brand::OS_RELEASE_MARKER;
const INSTALLER: &str = launcher_api::brand::OS_INSTALLER;
/// The service that looks for a Marah OS update (a polkit rule lets the player start it).
const UPDATE_SERVICE: &str = launcher_api::brand::OS_UPDATE_SERVICE;

#[derive(Clone)]
pub struct Linux {
    programs: Arc<dyn Programs>,
    /// Where the power supplies are: `/sys/class/power_supply`, or a test's folder.
    power_supply: std::path::PathBuf,
    /// Where Bluetooth adapters are: `/sys/class/bluetooth`.
    bluetooth_class: std::path::PathBuf,
}

impl Linux {
    pub fn new(programs: Arc<dyn Programs>) -> Self {
        Self {
            programs,
            power_supply: "/sys/class/power_supply".into(),
            bluetooth_class: "/sys/class/bluetooth".into(),
        }
    }

    /// Runs a system tool briefly, and returns what it printed.
    async fn run(&self, program: &str, args: &[&str]) -> PluginResult<String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = self.programs.output(&Location::System(program.into()), &args, QUICK).await?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Whether a system tool is installed.
    async fn has(&self, program: &str) -> bool {
        self.programs.find(&Location::System(program.into())).await.ok().flatten().is_some()
    }

    /// Runs a tool line by line, with no time limit, until it ends or `cancelled` says so; its
    /// exit code and its last line.
    async fn exec(
        &self,
        program: &str,
        args: &[&str],
        elevated: bool,
        cancelled: &(dyn Fn() -> bool + Send + Sync),
        each: impl Fn(&str) + Send + Sync,
    ) -> PluginResult<(i32, String)> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let last = std::sync::Mutex::new(String::new());
        let line = |text: &str| {
            if !text.trim().is_empty() {
                *last.lock().expect("not poisoned") = text.trim().to_owned();
            }
            each(text);
        };
        let program = Location::System(program.into());
        let code = self.programs.stream(&program, &args, elevated, &line, cancelled).await?;
        let last = last.into_inner().expect("not poisoned");
        Ok((code, last))
    }

    /// [`Linux::exec`] for a job, which the player can cancel.
    async fn stream(
        &self,
        program: &str,
        args: &[&str],
        elevated: bool,
        job: &Arc<dyn SystemJob>,
        each: impl Fn(&str) + Send + Sync,
    ) -> PluginResult<(i32, String)> {
        self.exec(program, args, elevated, &|| job.cancelled(), each).await
    }

    /// Runs a tool that changes something, and fails with its error when it says no.
    async fn act(&self, program: &str, args: &[&str]) -> PluginResult<()> {
        let (code, last) = self.exec(program, args, false, &|| false, |_| {}).await?;
        if code == 0 { Ok(()) } else { Err(PluginError::Other(parse::error_line(&last))) }
    }

    /// Runs `work` in the background, ending `job` with its outcome.
    fn background(job: Arc<dyn SystemJob>, work: impl Future<Output = Result<String, String>> + Send + 'static) {
        tokio::spawn(async move {
            match work.await {
                Ok(text) => job.done(&text),
                Err(_) if job.cancelled() => job.failed("Cancelled"),
                Err(reason) => job.failed(&reason),
            }
        });
    }

    async fn missing(&self, program: &str, what: &str) -> Option<String> {
        (!self.has(program).await).then(|| format!("{what} ({program}) isn't installed"))
    }
}

fn failed(error: PluginError) -> String {
    match error {
        PluginError::NotFound(_) => "the system's tool isn't installed".into(),
        other => other.to_string(),
    }
}

/// Network and Wi-Fi.
impl Linux {
    async fn network_unavailable(&self) -> Option<String> {
        self.missing("nmcli", "NetworkManager").await
    }

    async fn network_status(&self) -> PluginResult<NetworkStatus> {
        let mut status = parse::network(&self.run("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]).await?);
        if status.connection == Connection::Wifi {
            let list =
                self.run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list"]).await?;
            status.signal = parse::wifi(&list).into_iter().find(|w| w.in_use).map(|w| w.signal);
        }
        Ok(status)
    }

    async fn wifi(&self, rescan: bool) -> PluginResult<Vec<WifiNetwork>> {
        let rescan = if rescan { "yes" } else { "no" };
        let args = ["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", rescan];
        // The list and the saved networks don't need each other: both are read at once.
        let (list, saved) =
            tokio::join!(self.run("nmcli", &args), self.run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]));
        let (list, saved) = (parse::wifi(&list?), parse::saved_wifi(&saved?));
        Ok(list
            .into_iter()
            .map(|w| WifiNetwork {
                saved: saved.contains(&w.ssid),
                ssid: w.ssid,
                signal: w.signal,
                secured: w.secured,
                active: w.in_use,
            })
            .collect())
    }

    async fn join(&self, ssid: &str, password: Option<&str>, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, &format!("Joining {ssid}"));
        let me = self.clone();
        let (ssid, password) = (ssid.to_owned(), password.filter(|p| !p.is_empty()).map(str::to_owned));
        let handle = job.clone();
        Linux::background(job, async move {
            let mut args = vec!["--wait", "45", "device", "wifi", "connect", ssid.as_str()];
            if let Some(password) = &password {
                args.extend(["password", password.as_str()]);
            }
            let (code, last) = me.stream("nmcli", &args, false, &handle, |_| {}).await.map_err(failed)?;
            if code == 0 {
                Ok("Connected".into())
            } else if last.contains("Secrets were required") || last.contains("802-11-wireless-security") {
                Err("The password is wrong".into())
            } else {
                Err(parse::error_line(&last))
            }
        });
        Ok(())
    }

    async fn forget(&self, ssid: &str) -> PluginResult<()> {
        self.act("nmcli", &["connection", "delete", "id", ssid]).await
    }
}

/// Bluetooth.
impl Linux {
    async fn bluetooth_unavailable(&self) -> Option<String> {
        if let Some(missing) = self.missing("bluetoothctl", "BlueZ").await {
            return Some(missing);
        }
        let class = self.bluetooth_class.clone();
        // A read of /sys: on a blocking thread, not on the async workers.
        let adapter = tokio::task::spawn_blocking(move || {
            std::fs::read_dir(&class)
                .is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("hci")))
        });
        let adapter = adapter.await.unwrap_or(false);
        (!adapter).then(|| "This PC has no Bluetooth adapter".into())
    }

    async fn powered(&self) -> PluginResult<bool> {
        Ok(parse::powered(&self.run("bluetoothctl", &["show"]).await?))
    }

    async fn set_powered(&self, on: bool) -> PluginResult<()> {
        self.act("bluetoothctl", &["power", if on { "on" } else { "off" }]).await
    }

    async fn devices(&self) -> PluginResult<Vec<BluetoothDevice>> {
        let (all, paired, connected) = tokio::join!(
            self.run("bluetoothctl", &["devices"]),
            self.run("bluetoothctl", &["devices", "Paired"]),
            self.run("bluetoothctl", &["devices", "Connected"]),
        );
        Ok(parse::bluetooth(&all?, &paired?, &connected?))
    }

    async fn scan_bluetooth(&self, seconds: u32, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Looking for devices: put yours in pairing mode");
        let me = self.clone();
        let handle = job.clone();
        Linux::background(job, async move {
            let _ = me.run("bluetoothctl", &["power", "on"]).await;
            let seconds = seconds.to_string();
            me.stream("bluetoothctl", &["--timeout", &seconds, "scan", "on"], false, &handle, |_| {})
                .await
                .map_err(failed)?;
            Ok("Done looking".into())
        });
        Ok(())
    }

    async fn pair(&self, address: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Pairing");
        let me = self.clone();
        let address = address.to_owned();
        let handle = job.clone();
        Linux::background(job, async move {
            let steps: [(&str, &[&str]); 3] = [
                ("Pairing", &["--timeout", "25", "pair", &address]),
                // Trusted, it connects again by itself.
                ("Trusting", &["trust", &address]),
                ("Connecting", &["--timeout", "15", "connect", &address]),
            ];
            for (what, args) in steps {
                handle.progress(None, what);
                let (code, last) = me.stream("bluetoothctl", args, false, &handle, |_| {}).await.map_err(failed)?;
                let refused = ["Failed", "not available", "org.bluez.Error"].iter().any(|w| last.contains(w));
                if code != 0 || refused {
                    return Err(format!("{what} didn't work: {}", parse::error_line(&last)));
                }
            }
            Ok("Connected".into())
        });
        Ok(())
    }

    async fn connect_device(&self, address: &str) -> PluginResult<()> {
        self.act("bluetoothctl", &["--timeout", "8", "connect", address]).await
    }

    async fn disconnect_device(&self, address: &str) -> PluginResult<()> {
        self.act("bluetoothctl", &["disconnect", address]).await
    }

    async fn remove_device(&self, address: &str) -> PluginResult<()> {
        self.act("bluetoothctl", &["remove", address]).await
    }
}

/// Sound.
impl Linux {
    async fn audio_unavailable(&self) -> Option<String> {
        self.missing("pactl", "PipeWire's sound tools").await
    }

    async fn outputs(&self) -> PluginResult<Vec<AudioOutput>> {
        let (sinks, default) =
            tokio::join!(self.run("pactl", &["-f", "json", "list", "sinks"]), self.run("pactl", &["get-default-sink"]));
        Ok(parse::outputs(&sinks?, &default?))
    }

    async fn set_output(&self, name: &str) -> PluginResult<()> {
        self.act("pactl", &["set-default-sink", name]).await
    }

    async fn level(&self) -> PluginResult<AudioLevel> {
        let (volume, mute) = tokio::join!(
            self.run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]),
            self.run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]),
        );
        Ok(parse::level(&volume?, &mute?))
    }

    async fn set_volume(&self, volume: u8) -> PluginResult<()> {
        let volume = format!("{}%", volume.min(100));
        self.act("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "0"]).await?;
        self.act("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &volume]).await
    }

    async fn set_muted(&self, muted: bool) -> PluginResult<()> {
        self.act("pactl", &["set-sink-mute", "@DEFAULT_SINK@", if muted { "1" } else { "0" }]).await
    }
}

/// Restart, shut down and the battery.
impl Linux {
    async fn power_unavailable(&self) -> Option<String> {
        self.missing("systemctl", "systemd").await
    }

    async fn restart(&self) -> PluginResult<()> {
        self.act("systemctl", &["reboot"]).await
    }

    async fn shut_down(&self) -> PluginResult<()> {
        self.act("systemctl", &["poweroff"]).await
    }

    async fn battery(&self) -> PluginResult<Option<Battery>> {
        // Reads of /sys: on a blocking thread, not on the async workers.
        let supply = self.power_supply.clone();
        let battery = tokio::task::spawn_blocking(move || parse::battery(&supply)).await;
        battery.map_err(|e| PluginError::Storage(format!("the battery wasn't read: {e}")))
    }
}

/// Marah OS's updates.
impl Linux {
    async fn os_update(&self) -> PluginResult<OsUpdate> {
        if !tokio::fs::try_exists(MARAH_OS).await.unwrap_or(false) {
            return Err(PluginError::NotFound(format!("this isn't {}", launcher_api::brand::PRODUCT_OS_NAME)));
        }
        let json = self.run("rpm-ostree", &["status", "--json"]).await?;
        match parse::staged(&json) {
            Some(Some(version)) => Ok(OsUpdate::Staged { version }),
            Some(None) => Ok(OsUpdate::Current),
            None => Ok(OsUpdate::Failed { reason: "rpm-ostree's status couldn't be read".into() }),
        }
    }

    async fn check_now(&self) -> PluginResult<()> {
        self.act("systemctl", &["start", "--no-block", UPDATE_SERVICE]).await
    }
}

/// Marah OS's installer, on its USB stick.
impl Linux {
    async fn disks(&self) -> PluginResult<Vec<InstallDisk>> {
        if !tokio::fs::try_exists(INSTALLER).await.unwrap_or(false) {
            return Ok(Vec::new());
        }
        // The stick has the installer on its PATH as marah-os-install (docs/contributors/os-contract.md).
        Ok(parse::disks(&self.run(launcher_api::brand::OS_INSTALL_COMMAND, &["--disks"]).await?))
    }

    async fn install(&self, disk: &Path, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(Some(0), "Getting ready");
        let me = self.clone();
        let disk = disk.display().to_string();
        let handle = job.clone();
        Linux::background(job, async move {
            let end = std::sync::Mutex::new(None);
            let each = |line: &str| match parse::install_line(line) {
                Some(parse::InstallLine::Progress(percent, text)) => handle.progress(Some(percent), &text),
                Some(other) => *end.lock().expect("not poisoned") = Some(other),
                None => {}
            };
            let (code, _) = me
                .stream(launcher_api::brand::OS_INSTALL_COMMAND, &[&disk], true, &handle, each)
                .await
                .map_err(failed)?;
            match end.into_inner().expect("not poisoned") {
                Some(parse::InstallLine::Done(installed)) => {
                    Ok(launcher_api::serde_json::to_string(&installed).expect("Installed is JSON"))
                }
                Some(parse::InstallLine::Failed(reason)) => Err(reason),
                _ if code == 126 || code == 127 => Err("The installer wasn't allowed to run".into()),
                _ => Err("The installer stopped without finishing".into()),
            }
        });
        Ok(())
    }

    async fn choose_partitions(&self) -> PluginResult<()> {
        // The installer knows how this stick reaches Anaconda (see docs/contributors/os-contract.md).
        let (code, last) = self
            .exec(launcher_api::brand::OS_INSTALL_COMMAND, &["--choose-partitions"], true, &|| false, |_| {})
            .await?;
        if code == 0 { Ok(()) } else { Err(PluginError::Other(parse::error_line(&last))) }
    }
}

/// The columns `lsblk` lists, as [`parse::drives`] reads them.
const LSBLK_COLUMNS: &str =
    "NAME,PKNAME,PATH,TYPE,SIZE,FSTYPE,PARTTYPE,LABEL,UUID,MOUNTPOINTS,FSAVAIL,TRAN,HOTPLUG,RM,MODEL";

impl Linux {
    /// Runs `udisksctl <action> -b <device>` for a job, without asking for a password: UDisks asks
    /// polkit itself, and the session has no one to type it. Every line it printed, or why it
    /// failed.
    async fn udisks(&self, action: &str, device: &str, job: &Arc<dyn SystemJob>) -> Result<String, String> {
        let printed = std::sync::Mutex::new(String::new());
        let each = |line: &str| {
            let mut printed = printed.lock().expect("not poisoned");
            printed.push_str(line);
            printed.push('\n');
        };
        let args = [action, "-b", device, "--no-user-interaction"];
        let (code, last) = self.stream("udisksctl", &args, false, job, each).await.map_err(failed)?;
        if code == 0 { Ok(printed.into_inner().expect("not poisoned")) } else { Err(parse::udisks_error(&last)) }
    }
}

/// Drives.
impl Linux {
    async fn drives_unavailable(&self) -> Option<String> {
        match self.missing("udisksctl", "UDisks").await {
            Some(missing) => Some(missing),
            None => self.missing("lsblk", "util-linux").await,
        }
    }

    async fn drives(&self) -> PluginResult<Vec<SystemDrive>> {
        Ok(parse::drives(&self.run("lsblk", &["--json", "--list", "-b", "-o", LSBLK_COLUMNS]).await?))
    }

    async fn mount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Mounting");
        let (me, device, handle) = (self.clone(), device.to_owned(), job.clone());
        Linux::background(job, async move {
            let reply = me.udisks("mount", &device, &handle).await?;
            // lsblk says where first: the reply's words may change from one UDisks to the next.
            let listed = me.drives().await.ok().and_then(|list| list.into_iter().find(|d| d.device == device)?.mount);
            let at = listed.or_else(|| parse::mounted_at(&reply));
            at.map(|at| at.display().to_string()).ok_or_else(|| "UDisks mounted the drive but didn't say where".into())
        });
        Ok(())
    }

    async fn unmount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Writing what is left to the drive");
        let (me, device, handle) = (self.clone(), device.to_owned(), job.clone());
        Linux::background(job, async move {
            me.udisks("unmount", &device, &handle).await?;
            Ok("Unmounted".into())
        });
        Ok(())
    }

    async fn eject(&self, disk: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Writing what is left to the drive");
        let parts: Vec<SystemDrive> = self.drives().await?.into_iter().filter(|d| d.disk == disk).collect();
        if parts.iter().any(|d| d.system) {
            return Err(PluginError::InvalidInput("the system runs from this drive".into()));
        }
        let (me, disk, handle) = (self.clone(), disk.to_owned(), job.clone());
        Linux::background(job, async move {
            for part in parts.iter().filter(|d| d.mount.is_some()) {
                me.udisks("unmount", &part.device, &handle).await?;
            }
            me.udisks("power-off", &disk, &handle).await?;
            Ok("You can take it out now".into())
        });
        Ok(())
    }
}

/// One of the areas Linux manages, through the same interface as every other.
pub struct LinuxArea {
    pub area: SystemArea,
    pub linux: Arc<Linux>,
}

/// An area that can't be used here, and why, else its data.
fn ready_unless(missing: Option<String>, data: impl FnOnce() -> AreaData) -> AreaState {
    match missing {
        Some(why) => AreaState::Unsupported(why),
        None => AreaState::Ready(data()),
    }
}

#[launcher_api::async_trait]
impl AreaSettings for LinuxArea {
    /// Each area's reads that don't need each other run at once. A read that fails leaves its
    /// part of the data empty, and the rest still comes.
    async fn state(&self) -> PluginResult<AreaState> {
        let linux = &self.linux;
        Ok(match self.area {
            SystemArea::Network => {
                let missing = linux.network_unavailable().await;
                let (status, wifi) = match missing {
                    Some(_) => (Err(PluginError::Other(String::new())), Ok(Vec::new())),
                    None => tokio::join!(linux.network_status(), linux.wifi(false)),
                };
                ready_unless(missing, || AreaData::Network { status: status.ok(), wifi: wifi.unwrap_or_default() })
            }
            SystemArea::Bluetooth => {
                let missing = linux.bluetooth_unavailable().await;
                let (powered, devices) = match missing {
                    Some(_) => (Ok(false), Ok(Vec::new())),
                    None => tokio::join!(linux.powered(), linux.devices()),
                };
                ready_unless(missing, || AreaData::Bluetooth {
                    powered: powered.unwrap_or(false),
                    devices: devices.unwrap_or_default(),
                })
            }
            SystemArea::Audio => {
                let missing = linux.audio_unavailable().await;
                let (outputs, level) = match missing {
                    Some(_) => (Ok(Vec::new()), Err(PluginError::Other(String::new()))),
                    None => tokio::join!(linux.outputs(), linux.level()),
                };
                ready_unless(missing, || AreaData::Audio { outputs: outputs.unwrap_or_default(), level: level.ok() })
            }
            SystemArea::Power => {
                let (refused, battery) = tokio::join!(linux.power_unavailable(), linux.battery());
                AreaState::Ready(AreaData::Power { battery: battery.ok().flatten(), refused })
            }
            SystemArea::OsUpdates => match linux.os_update().await {
                Err(PluginError::NotFound(_)) => AreaState::Absent,
                update => AreaState::Ready(AreaData::OsUpdates { update: update.ok() }),
            },
            SystemArea::OsInstall => match tokio::fs::try_exists(INSTALLER).await.unwrap_or(false) {
                false => AreaState::Absent,
                true => AreaState::Ready(AreaData::OsInstall { disks: linux.disks().await.unwrap_or_default() }),
            },
            SystemArea::Drives => match linux.drives_unavailable().await {
                Some(why) => AreaState::Unsupported(why),
                None => AreaState::Ready(AreaData::Drives { drives: linux.drives().await? }),
            },
            SystemArea::Vpn => AreaState::Absent,
        })
    }

    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        use AreaAction as A;
        let linux = &self.linux;
        match action {
            A::ScanWifi => linux.wifi(true).await.map(|_| ()),
            A::JoinWifi { ssid, password } => linux.join(&ssid, password.as_deref(), job).await,
            A::ForgetWifi { ssid } => linux.forget(&ssid).await,
            A::SetBluetooth { on } => linux.set_powered(on).await,
            A::ScanBluetooth { seconds } => linux.scan_bluetooth(seconds, job).await,
            A::Pair { address } => linux.pair(&address, job).await,
            A::ConnectDevice { address } => linux.connect_device(&address).await,
            A::DisconnectDevice { address } => linux.disconnect_device(&address).await,
            A::RemoveDevice { address } => linux.remove_device(&address).await,
            A::SetOutput { name } => linux.set_output(&name).await,
            A::SetVolume { volume } => linux.set_volume(volume).await,
            A::SetMuted { muted } => linux.set_muted(muted).await,
            A::Restart => linux.restart().await,
            A::ShutDown => linux.shut_down().await,
            A::CheckOsUpdate => linux.check_now().await,
            A::InstallOs { disk } => linux.install(&disk, job).await,
            A::ChoosePartitions => linux.choose_partitions().await,
            A::Mount { device } => linux.mount(&device, job).await,
            A::Unmount { device } => linux.unmount(&device, job).await,
            A::Eject { disk } => linux.eject(&disk, job).await,
            A::ImportVpn { .. } | A::ConnectVpn { .. } | A::DisconnectVpn { .. } => {
                Err(PluginError::Unsupported("the VPNs are their own area".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Programs that are all missing: what a system without the tools looks like.
    struct Nothing;

    #[launcher_api::async_trait]
    impl Programs for Nothing {
        async fn find(&self, _program: &Location) -> PluginResult<Option<String>> {
            Ok(None)
        }

        async fn output(&self, program: &Location, _args: &[String], _timeout: Duration) -> PluginResult<Vec<u8>> {
            Err(PluginError::NotFound(format!("{program} isn't there")))
        }
    }

    async fn state(area: SystemArea) -> AreaState {
        let linux = Arc::new(Linux::new(Arc::new(Nothing)));
        LinuxArea { area, linux }.state().await.unwrap()
    }

    #[tokio::test]
    async fn without_the_tools_each_setting_says_which_one_is_missing() {
        let unsupported = |why: &str| AreaState::Unsupported(why.into());
        assert_eq!(state(SystemArea::Network).await, unsupported("NetworkManager (nmcli) isn't installed"));
        assert_eq!(state(SystemArea::Bluetooth).await, unsupported("BlueZ (bluetoothctl) isn't installed"));
        assert_eq!(state(SystemArea::Audio).await, unsupported("PipeWire's sound tools (pactl) isn't installed"));
        assert_eq!(state(SystemArea::Drives).await, unsupported("UDisks (udisksctl) isn't installed"));
        assert!(Linux::new(Arc::new(Nothing)).network_status().await.is_err());
    }

    #[tokio::test]
    async fn without_systemd_restart_is_refused_and_the_battery_still_reads() {
        let AreaState::Ready(AreaData::Power { refused, .. }) = state(SystemArea::Power).await else {
            panic!("power is ready: the battery reads without the tools");
        };
        assert_eq!(refused.as_deref(), Some("systemd (systemctl) isn't installed"));
    }
}

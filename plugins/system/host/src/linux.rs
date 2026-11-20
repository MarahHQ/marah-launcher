//! Linux: NetworkManager (`nmcli`), BlueZ (`bluetoothctl`), PipeWire's PulseAudio server
//! (`pactl`), systemd (`systemctl`), the kernel's power supply folder, UDisks (`udisksctl`, with
//! `lsblk` for the list), the VPN interfaces in `/sys/class/net`, and on Marah OS `rpm-ostree`
//! and the USB stick's installer. Every tool runs through the [`Runner`], so the manifest's
//! `permissions.process` decides what may run, and every call has a time limit.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection,
    NetworkStatus, OsUpdate, PluginError, PluginResult, SystemArea, SystemDrive, SystemJob, Vpn, WifiNetwork,
};

use crate::parse;
use crate::tools::{self, Call, Limit, Runner, Tool};
use crate::vpn;

const NMCLI: Tool = Tool::new("nmcli", "NetworkManager");
const BLUETOOTHCTL: Tool = Tool::new("bluetoothctl", "BlueZ");
const PACTL: Tool = Tool::new("pactl", "PipeWire's sound tools");
const SYSTEMCTL: Tool = Tool::new("systemctl", "systemd");
const RPM_OSTREE: Tool = Tool::new("rpm-ostree", "rpm-ostree");
/// The stick has the installer on its PATH (docs/contributors/os-contract.md).
const OS_INSTALL: Tool = Tool::new(launcher_api::brand::OS_INSTALL_COMMAND, "the USB stick's installer");
const UDISKSCTL: Tool = Tool::new("udisksctl", "UDisks");
const LSBLK: Tool = Tool::new("lsblk", "util-linux");

/// Each area Linux manages, and the tools it runs. An area that can't work without its tools
/// says which one is missing, in this order.
pub const AREAS: &[(SystemArea, &[Tool])] = &[
    (SystemArea::Network, &[NMCLI]),
    // The VPNs are listed without NetworkManager; it imports and switches them.
    (SystemArea::Vpn, &[NMCLI]),
    (SystemArea::Bluetooth, &[BLUETOOTHCTL]),
    (SystemArea::Audio, &[PACTL]),
    // Without systemd, restart and shut down are refused, and the battery still shows.
    (SystemArea::Power, &[SYSTEMCTL]),
    (SystemArea::OsUpdates, &[RPM_OSTREE, SYSTEMCTL]),
    (SystemArea::OsInstall, &[OS_INSTALL]),
    (SystemArea::Drives, &[UDISKSCTL, LSBLK]),
];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    match area {
        SystemArea::Vpn => Arc::new(Vpns::new(tools)),
        area => Arc::new(LinuxArea { area, linux: Linux::new(tools) }),
    }
}

/// The tools of `area` in [`AREAS`].
fn tools_of(area: SystemArea) -> &'static [Tool] {
    AREAS.iter().find(|(a, _)| *a == area).map_or(&[], |(_, tools)| tools)
}

/// Marah OS's marker, and its USB stick's installer.
const MARAH_OS: &str = launcher_api::brand::OS_RELEASE_MARKER;
const INSTALLER: &str = launcher_api::brand::OS_INSTALLER;
/// The service that looks for a Marah OS update (a polkit rule lets the player start it).
const UPDATE_SERVICE: &str = launcher_api::brand::OS_UPDATE_SERVICE;

/// `nmcli --wait 45` joins a network: the time it gives itself, and some more.
const JOIN_TIME: Duration = Duration::from_secs(60);
/// `bluetoothctl --timeout 25 pair`, and `--timeout 15 connect`.
const PAIR_TIME: Duration = Duration::from_secs(40);
const CONNECT_TIME: Duration = Duration::from_secs(30);
/// UDisks mounts a drive, or writes what is left to it before it unmounts or powers it off: a
/// slow stick can take minutes.
const MOUNT_TIME: Duration = Duration::from_secs(120);
const UNMOUNT_TIME: Duration = Duration::from_secs(600);
/// A VPN's switch runs inside the core's 30-second limit on a plugin call: this one ends first,
/// so the player reads which tool took too long.
const VPN_TIME: Duration = Duration::from_secs(25);

#[derive(Clone)]
struct Linux {
    tools: Arc<Runner>,
    /// Where the power supplies are: `/sys/class/power_supply`, or a test's folder.
    power_supply: std::path::PathBuf,
    /// Where Bluetooth adapters are: `/sys/class/bluetooth`.
    bluetooth_class: std::path::PathBuf,
}

impl Linux {
    fn new(tools: Arc<Runner>) -> Self {
        Self { tools, power_supply: "/sys/class/power_supply".into(), bluetooth_class: "/sys/class/bluetooth".into() }
    }

    /// Why `area` can't be used: the first of its tools that isn't installed.
    async fn missing(&self, area: SystemArea) -> Option<String> {
        self.tools.missing(tools_of(area)).await
    }
}

/// Network and Wi-Fi.
impl Linux {
    async fn network_status(&self) -> PluginResult<NetworkStatus> {
        let mut status =
            parse::network(&self.tools.read(NMCLI, &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]).await?);
        if status.connection == Connection::Wifi {
            let list =
                self.tools.read(NMCLI, &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list"]).await?;
            status.signal = parse::wifi(&list).into_iter().find(|w| w.in_use).map(|w| w.signal);
        }
        Ok(status)
    }

    async fn wifi(&self, rescan: bool) -> PluginResult<Vec<WifiNetwork>> {
        let rescan = if rescan { "yes" } else { "no" };
        let args = ["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", rescan];
        // The list and the saved networks don't need each other: both are read at once.
        let (list, saved) = tokio::join!(
            self.tools.read(NMCLI, &args),
            self.tools.read(NMCLI, &["-t", "-f", "NAME,TYPE", "connection", "show"])
        );
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
        tools::job(job, async move {
            let mut args = vec!["--wait", "45", "device", "wifi", "connect", ssid.as_str()];
            if let Some(password) = &password {
                args.extend(["password", password.as_str()]);
            }
            let call = Call::job(&handle, Limit::After(JOIN_TIME));
            let ran = me.tools.run(NMCLI, &args, call, |_| {}).await.map_err(tools::reason)?;
            let (code, last) = (ran.code, ran.last);
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
        self.tools.act(NMCLI, &["connection", "delete", "id", ssid]).await.map(drop)
    }
}

/// Bluetooth.
impl Linux {
    async fn bluetooth_unavailable(&self) -> Option<String> {
        if let Some(missing) = self.missing(SystemArea::Bluetooth).await {
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
        Ok(parse::powered(&self.tools.read(BLUETOOTHCTL, &["show"]).await?))
    }

    async fn set_powered(&self, on: bool) -> PluginResult<()> {
        self.tools.act(BLUETOOTHCTL, &["power", if on { "on" } else { "off" }]).await.map(drop)
    }

    async fn devices(&self) -> PluginResult<Vec<BluetoothDevice>> {
        let (all, paired, connected) = tokio::join!(
            self.tools.read(BLUETOOTHCTL, &["devices"]),
            self.tools.read(BLUETOOTHCTL, &["devices", "Paired"]),
            self.tools.read(BLUETOOTHCTL, &["devices", "Connected"]),
        );
        Ok(parse::bluetooth(&all?, &paired?, &connected?))
    }

    async fn scan_bluetooth(&self, seconds: u32, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Looking for devices: put yours in pairing mode");
        let me = self.clone();
        let handle = job.clone();
        tools::job(job, async move {
            let _ = me.tools.read(BLUETOOTHCTL, &["power", "on"]).await;
            // `--timeout` ends the scan: the limit is for a bluetoothctl that doesn't.
            let call = Call::job(&handle, Limit::After(Duration::from_secs(u64::from(seconds) + 15)));
            let seconds = seconds.to_string();
            me.tools
                .run(BLUETOOTHCTL, &["--timeout", &seconds, "scan", "on"], call, |_| {})
                .await
                .map_err(tools::reason)?;
            Ok("Done looking".into())
        });
        Ok(())
    }

    async fn pair(&self, address: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Pairing");
        let me = self.clone();
        let address = address.to_owned();
        let handle = job.clone();
        tools::job(job, async move {
            let steps: [(&str, &[&str], Limit); 3] = [
                ("Pairing", &["--timeout", "25", "pair", &address], Limit::After(PAIR_TIME)),
                // Trusted, it connects again by itself.
                ("Trusting", &["trust", &address], Limit::Quick),
                ("Connecting", &["--timeout", "15", "connect", &address], Limit::After(CONNECT_TIME)),
            ];
            for (what, args, limit) in steps {
                handle.progress(None, what);
                let ran = me.tools.run(BLUETOOTHCTL, args, Call::job(&handle, limit), |_| {});
                let tools::Ran { code, last, .. } = ran.await.map_err(tools::reason)?;
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
        self.tools.act(BLUETOOTHCTL, &["--timeout", "8", "connect", address]).await.map(drop)
    }

    async fn disconnect_device(&self, address: &str) -> PluginResult<()> {
        self.tools.act(BLUETOOTHCTL, &["disconnect", address]).await.map(drop)
    }

    async fn remove_device(&self, address: &str) -> PluginResult<()> {
        self.tools.act(BLUETOOTHCTL, &["remove", address]).await.map(drop)
    }
}

/// Sound.
impl Linux {
    async fn outputs(&self) -> PluginResult<Vec<AudioOutput>> {
        let (sinks, default) = tokio::join!(
            self.tools.read(PACTL, &["-f", "json", "list", "sinks"]),
            self.tools.read(PACTL, &["get-default-sink"])
        );
        Ok(parse::outputs(&sinks?, &default?))
    }

    async fn set_output(&self, name: &str) -> PluginResult<()> {
        self.tools.act(PACTL, &["set-default-sink", name]).await.map(drop)
    }

    async fn level(&self) -> PluginResult<AudioLevel> {
        let (volume, mute) = tokio::join!(
            self.tools.read(PACTL, &["get-sink-volume", "@DEFAULT_SINK@"]),
            self.tools.read(PACTL, &["get-sink-mute", "@DEFAULT_SINK@"]),
        );
        Ok(parse::level(&volume?, &mute?))
    }

    async fn set_volume(&self, volume: u8) -> PluginResult<()> {
        let volume = format!("{}%", volume.min(100));
        self.tools.act(PACTL, &["set-sink-mute", "@DEFAULT_SINK@", "0"]).await?;
        self.tools.act(PACTL, &["set-sink-volume", "@DEFAULT_SINK@", &volume]).await.map(drop)
    }

    async fn set_muted(&self, muted: bool) -> PluginResult<()> {
        self.tools.act(PACTL, &["set-sink-mute", "@DEFAULT_SINK@", if muted { "1" } else { "0" }]).await.map(drop)
    }
}

/// Restart, shut down and the battery.
impl Linux {
    async fn restart(&self) -> PluginResult<()> {
        self.tools.act(SYSTEMCTL, &["reboot"]).await.map(drop)
    }

    async fn shut_down(&self) -> PluginResult<()> {
        self.tools.act(SYSTEMCTL, &["poweroff"]).await.map(drop)
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
        let json = self.tools.read(RPM_OSTREE, &["status", "--json"]).await?;
        match parse::staged(&json) {
            Some(Some(version)) => Ok(OsUpdate::Staged { version }),
            Some(None) => Ok(OsUpdate::Current),
            None => Ok(OsUpdate::Failed { reason: "rpm-ostree's status couldn't be read".into() }),
        }
    }

    async fn check_now(&self) -> PluginResult<()> {
        self.tools.act(SYSTEMCTL, &["start", "--no-block", UPDATE_SERVICE]).await.map(drop)
    }
}

/// Marah OS's installer, on its USB stick.
impl Linux {
    /// The disks the installer offers. When it fails, it offers none and says why on its errors
    /// (docs/contributors/os-contract.md): the area is unsupported with that reason, not a read
    /// that failed, which would keep the last list on screen. A run that didn't end, such as one
    /// that took too long, offers no disk.
    async fn os_install(&self) -> AreaState {
        let ran = self.tools.run(OS_INSTALL, &["--disks"], Call::quick(), |_| {}).await;
        match ran {
            Ok(ran) if ran.code != 0 => AreaState::Unsupported(
                parse::disks_error(&ran.text).unwrap_or_else(|| "The installer couldn't list the disks".into()),
            ),
            ran => AreaState::Ready(AreaData::OsInstall {
                disks: ran.map(|ran| parse::disks(&ran.text)).unwrap_or_default(),
            }),
        }
    }

    async fn install(&self, disk: &Path, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(Some(0), "Getting ready");
        let me = self.clone();
        let disk = disk.display().to_string();
        let handle = job.clone();
        tools::job(job, async move {
            let end = std::sync::Mutex::new(None);
            let each = |line: &str| match parse::install_line(line) {
                Some(parse::InstallLine::Progress(percent, text)) => handle.progress(Some(percent), &text),
                Some(other) => *end.lock().expect("not poisoned") = Some(other),
                None => {}
            };
            // It takes as long as the disk does: the player can cancel it.
            let call = Call::job(&handle, Limit::Never).elevated();
            let code = me.tools.run(OS_INSTALL, &[&disk], call, each).await.map_err(tools::reason)?.code;
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
        // pkexec waits for the player's password first.
        let call = Call { limit: Limit::Never, ..Call::quick() }.elevated();
        let tools::Ran { code, last, .. } = self.tools.run(OS_INSTALL, &["--choose-partitions"], call, |_| {}).await?;
        if code == 0 { Ok(()) } else { Err(PluginError::Other(parse::error_line(&last))) }
    }
}

/// The columns `lsblk` lists, as [`parse::drives`] reads them.
const LSBLK_COLUMNS: &str =
    "NAME,PKNAME,PATH,TYPE,SIZE,FSTYPE,PARTTYPE,LABEL,UUID,MOUNTPOINTS,FSAVAIL,TRAN,HOTPLUG,RM,MODEL";

impl Linux {
    /// Runs `udisksctl <action> -b <device>` for a job, for at most `limit`, without asking for a
    /// password: UDisks asks polkit itself, and the session has no one to type it. Every line it
    /// printed, or why it failed.
    async fn udisks(
        &self,
        action: &str,
        device: &str,
        limit: Duration,
        job: &Arc<dyn SystemJob>,
    ) -> Result<String, String> {
        let args = [action, "-b", device, "--no-user-interaction"];
        let call = Call::job(job, Limit::After(limit));
        let ran = self.tools.run(UDISKSCTL, &args, call, |_| {}).await.map_err(tools::reason)?;
        if ran.code == 0 { Ok(ran.text) } else { Err(parse::udisks_error(&ran.last)) }
    }
}

/// Drives.
impl Linux {
    async fn drives(&self) -> PluginResult<Vec<SystemDrive>> {
        Ok(parse::drives(&self.tools.read(LSBLK, &["--json", "--list", "-b", "-o", LSBLK_COLUMNS]).await?))
    }

    async fn mount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Mounting");
        let (me, device, handle) = (self.clone(), device.to_owned(), job.clone());
        tools::job(job, async move {
            let reply = me.udisks("mount", &device, MOUNT_TIME, &handle).await?;
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
        tools::job(job, async move {
            me.udisks("unmount", &device, UNMOUNT_TIME, &handle).await?;
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
        tools::job(job, async move {
            for part in parts.iter().filter(|d| d.mount.is_some()) {
                me.udisks("unmount", &part.device, UNMOUNT_TIME, &handle).await?;
            }
            me.udisks("power-off", &disk, UNMOUNT_TIME, &handle).await?;
            Ok("You can take it out now".into())
        });
        Ok(())
    }
}

/// One of the areas Linux manages, through the same interface as every other.
struct LinuxArea {
    area: SystemArea,
    linux: Linux,
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
                let missing = linux.missing(SystemArea::Network).await;
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
                let missing = linux.missing(SystemArea::Audio).await;
                let (outputs, level) = match missing {
                    Some(_) => (Ok(Vec::new()), Err(PluginError::Other(String::new()))),
                    None => tokio::join!(linux.outputs(), linux.level()),
                };
                ready_unless(missing, || AreaData::Audio { outputs: outputs.unwrap_or_default(), level: level.ok() })
            }
            SystemArea::Power => {
                let (refused, battery) = tokio::join!(linux.missing(SystemArea::Power), linux.battery());
                AreaState::Ready(AreaData::Power { battery: battery.ok().flatten(), refused })
            }
            SystemArea::OsUpdates => match linux.os_update().await {
                Err(PluginError::NotFound(_)) => AreaState::Absent,
                update => AreaState::Ready(AreaData::OsUpdates { update: update.ok() }),
            },
            SystemArea::OsInstall => match tokio::fs::try_exists(INSTALLER).await.unwrap_or(false) {
                false => AreaState::Absent,
                true => linux.os_install().await,
            },
            SystemArea::Drives => match linux.missing(SystemArea::Drives).await {
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

/// The VPNs: the interfaces in `/sys/class/net` ([`vpn::list_in`]), and the connections
/// NetworkManager knows, which it imports and switches.
struct Vpns {
    tools: Arc<Runner>,
    /// NetworkManager is there, as the last read of the VPNs found.
    nmcli: std::sync::OnceLock<bool>,
}

impl Vpns {
    fn new(tools: Arc<Runner>) -> Self {
        Self { tools, nmcli: std::sync::OnceLock::new() }
    }

    /// Runs `nmcli` with `args`, for at most `limit`. The error is its first line of output,
    /// without "Error: ".
    async fn nmcli(&self, args: &[&str], limit: Limit) -> PluginResult<String> {
        let ran = self.tools.run(NMCLI, args, Call { limit, ..Call::quick() }, |_| {}).await?;
        if ran.code == 0 { Ok(ran.text) } else { Err(PluginError::Other(parse::error_line(&ran.text))) }
    }

    async fn vpns(&self) -> Vec<Vpn> {
        let found = vpn::list_in(Path::new("/sys/class/net"), &crate::unix::addressed());
        let saved = self.nmcli(&["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"], Limit::Quick).await;
        let _ = self.nmcli.set(saved.is_ok());
        vpn::merge(found, saved.map(|text| vpn::parse_saved(&text)).unwrap_or_default())
    }

    fn can_import(&self) -> bool {
        self.nmcli.get().copied().unwrap_or(false)
    }

    async fn import(&self, file: &Path) -> PluginResult<()> {
        let kind = match file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("conf") => "wireguard",
            Some("ovpn") => "openvpn",
            _ => return Err(PluginError::Other("Choose a WireGuard .conf or OpenVPN .ovpn file".into())),
        };
        let args = ["connection", "import", "type", kind, "file", &file.to_string_lossy()];
        let out = self.nmcli(&args, Limit::Quick).await?;
        // "Connection 'wg-se' (<uuid>) successfully added."
        let name = out
            .split('\'')
            .nth(1)
            .map(str::to_owned)
            .ok_or_else(|| PluginError::Other("NetworkManager didn't name the connection".into()))?;
        self.connect(&name).await
    }

    async fn connect(&self, name: &str) -> PluginResult<()> {
        self.nmcli(&["connection", "up", "id", name], Limit::After(VPN_TIME)).await.map(drop)
    }

    async fn disconnect(&self, name: &str) -> PluginResult<()> {
        self.nmcli(&["connection", "down", "id", name], Limit::After(VPN_TIME)).await.map(drop)
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Vpns {
    async fn state(&self) -> PluginResult<AreaState> {
        let vpns = self.vpns().await;
        Ok(AreaState::Ready(AreaData::Vpn { finds_vpns: true, can_import: self.can_import(), vpns }))
    }

    async fn act(&self, action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        match action {
            AreaAction::ImportVpn { file } => self.import(&file).await,
            AreaAction::ConnectVpn { name } => self.connect(&name).await,
            AreaAction::DisconnectVpn { name } => self.disconnect(&name).await,
            _ => Err(PluginError::Unsupported("not a VPN action".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use launcher_api::{Location, Programs};

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

    fn nothing() -> Arc<Runner> {
        Arc::new(Runner::new(Arc::new(Nothing)))
    }

    async fn state(area: SystemArea) -> AreaState {
        adapter(area, nothing()).state().await.unwrap()
    }

    #[tokio::test]
    async fn without_the_tools_each_setting_says_which_one_is_missing() {
        let unsupported = |why: &str| AreaState::Unsupported(why.into());
        assert_eq!(state(SystemArea::Network).await, unsupported("NetworkManager (nmcli) isn't installed"));
        assert_eq!(state(SystemArea::Bluetooth).await, unsupported("BlueZ (bluetoothctl) isn't installed"));
        assert_eq!(state(SystemArea::Audio).await, unsupported("PipeWire's sound tools (pactl) isn't installed"));
        assert_eq!(state(SystemArea::Drives).await, unsupported("UDisks (udisksctl) isn't installed"));
        assert!(Linux::new(nothing()).network_status().await.is_err());
    }

    #[tokio::test]
    async fn without_systemd_restart_is_refused_and_the_battery_still_reads() {
        let AreaState::Ready(AreaData::Power { refused, .. }) = state(SystemArea::Power).await else {
            panic!("power is ready: the battery reads without the tools");
        };
        assert_eq!(refused.as_deref(), Some("systemd (systemctl) isn't installed"));
    }

    /// The USB stick's installer as a test sets it: `--disks` prints `out`, output and errors
    /// mixed as the host streams them, and ends with `code`.
    struct Installer {
        out: &'static str,
        code: i32,
    }

    #[launcher_api::async_trait]
    impl Programs for Installer {
        async fn find(&self, program: &Location) -> PluginResult<Option<String>> {
            Ok(Some(format!("/usr/bin/{program}")))
        }

        async fn output(&self, _program: &Location, _args: &[String], _timeout: Duration) -> PluginResult<Vec<u8>> {
            Ok(self.out.as_bytes().to_vec())
        }

        async fn stream(
            &self,
            _program: &Location,
            _args: &[String],
            _elevated: bool,
            line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
            _cancelled: &(dyn Fn() -> bool + Send + Sync),
        ) -> PluginResult<i32> {
            self.out.lines().for_each(line);
            Ok(self.code)
        }
    }

    async fn os_install(out: &'static str, code: i32) -> AreaState {
        Linux::new(Arc::new(Runner::new(Arc::new(Installer { out, code })))).os_install().await
    }

    #[tokio::test]
    async fn the_installer_s_disks_are_the_area_s_data() {
        let state = os_install("disk /dev/nvme0n1 512000000000 1 0 Samsung_970\n", 0).await;
        let AreaState::Ready(AreaData::OsInstall { disks }) = state else { panic!("ready: {state:?}") };
        assert_eq!(disks.iter().map(|d| d.path.display().to_string()).collect::<Vec<_>>(), ["/dev/nvme0n1"]);
        assert_eq!(disks[0].model, "Samsung 970");
    }

    #[tokio::test]
    async fn an_installer_that_lists_no_disk_leaves_the_area_ready_and_empty() {
        assert_eq!(os_install("", 0).await, AreaState::Ready(AreaData::OsInstall { disks: Vec::new() }));
    }

    #[tokio::test]
    async fn an_installer_that_fails_says_why_with_its_own_last_line() {
        let why = "The USB stick Marah OS started from wasn't found, so no disk is offered.";
        let out = "disk /dev/sda 64000000000 0 1 Stick\nLooking for the USB stick\n\
                   The USB stick Marah OS started from wasn't found, so no disk is offered.\n\
                   disk /dev/sdb 64000000000 0 1 Other\n\n";
        assert_eq!(os_install(out, 1).await, AreaState::Unsupported(why.into()));
    }

    #[tokio::test]
    async fn an_installer_that_fails_without_a_word_still_says_why() {
        let why = "The installer couldn't list the disks";
        assert_eq!(os_install("", 2).await, AreaState::Unsupported(why.into()));
        assert_eq!(os_install("disk /dev/sda 64000000000 0 1 Stick\n", 2).await, AreaState::Unsupported(why.into()));
    }

    /// Opt-in, `MARAH_VPN_TESTS=1`: reads this computer's interfaces. It connects nothing.
    #[test]
    fn lists_this_computer_s_networks() {
        if std::env::var_os("MARAH_VPN_TESTS").is_none() {
            return;
        }
        let found = vpn::list_in(Path::new("/sys/class/net"), &crate::unix::addressed());
        assert!(found.iter().all(|v| !v.name.is_empty()), "{found:?}");
        println!("{found:?}");
    }
}

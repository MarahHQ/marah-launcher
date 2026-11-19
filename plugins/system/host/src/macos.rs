//! macOS: only the tools macOS ships, each through the [`Runner`]. `diskutil` for the external
//! drives, `networksetup` and `scutil` for the network and Wi-Fi, `scutil` for the VPNs,
//! `system_profiler` for Bluetooth, `osascript` for the sound and for restart and shut down,
//! and `pmset` for the battery. What they print becomes the areas' data in `macmap.rs`, which
//! the tests run on every OS.
//!
//! macOS has no built-in tool for some of the work: a Wi-Fi scan, choosing the sound output,
//! and turning Bluetooth on or off, pairing and connecting. Those actions say so, and where in
//! macOS the player does them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use launcher_api::serde_json::Value;
use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, AudioLevel, Battery, Connection, NetworkStatus, PluginError,
    PluginResult, SystemArea, SystemDrive, SystemJob, Vpn, WifiNetwork,
};

use crate::macmap;
use crate::tools::{self, Call, Limit, Runner, Tool};

const PMSET: Tool = Tool::new("pmset", "pmset");
const DISKUTIL: Tool = Tool::new("diskutil", "Disk Utility's tool");
const NETWORKSETUP: Tool = Tool::new("networksetup", "networksetup");
const SCUTIL: Tool = Tool::new("scutil", "scutil");
const SYSTEM_PROFILER: Tool = Tool::new("system_profiler", "System Information's tool");
const OSASCRIPT: Tool = Tool::new("osascript", "AppleScript");

/// Each area macOS manages, and the tools it runs. An area that can't work without its tools
/// says which one is missing, in this order.
pub const AREAS: &[(SystemArea, &[Tool])] = &[
    (SystemArea::Network, &[NETWORKSETUP, SCUTIL]),
    // The networks with an address are listed without scutil; it lists and switches the VPNs.
    (SystemArea::Vpn, &[SCUTIL]),
    (SystemArea::Bluetooth, &[SYSTEM_PROFILER]),
    (SystemArea::Audio, &[OSASCRIPT]),
    // Without osascript, restart and shut down are refused, and the battery still shows.
    (SystemArea::Power, &[OSASCRIPT, PMSET]),
    (SystemArea::Drives, &[DISKUTIL]),
];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    Arc::new(MacArea { area, mac: Mac { tools } })
}

/// The tools of `area` in [`AREAS`].
fn tools_of(area: SystemArea) -> &'static [Tool] {
    AREAS.iter().find(|(a, _)| *a == area).map_or(&[], |(_, tools)| tools)
}

/// `networksetup -setairportnetwork` joins a network: the time it takes, and some more.
const JOIN_TIME: Duration = Duration::from_secs(60);
/// `diskutil mount`. `diskutil unmount` gives up after a minute for each volume, and eject
/// unmounts each volume of the disk first.
const MOUNT_TIME: Duration = Duration::from_secs(120);
const UNMOUNT_TIME: Duration = Duration::from_secs(120);
const EJECT_TIME: Duration = Duration::from_secs(600);
/// A call that may wait for the player to answer macOS's question about Automation: inside the
/// core's 30-second limit on a plugin call, so the player reads which tool took too long.
const ANSWER_TIME: Duration = Duration::from_secs(25);
/// How long a VPN may take to connect, and how often its state is read meanwhile. A VPN that
/// still reads as disconnected after the grace didn't start.
const VPN_TIME: Duration = Duration::from_secs(20);
const VPN_GRACE: Duration = Duration::from_secs(5);
const VPN_POLL: Duration = Duration::from_millis(500);

#[derive(Clone)]
struct Mac {
    tools: Arc<Runner>,
}

impl Mac {
    /// Why `area` can't be used: the first of its tools that isn't installed.
    async fn missing(&self, area: SystemArea) -> Option<String> {
        self.tools.missing(tools_of(area)).await
    }

    /// Runs `osascript` with one line of AppleScript, for at most `limit`. What it printed, or
    /// why it failed, for the player.
    async fn osascript(&self, script: &str, limit: Limit) -> PluginResult<String> {
        let ran = self.tools.run(OSASCRIPT, &["-e", script], Call { limit, ..Call::quick() }, |_| {}).await?;
        if ran.code == 0 { Ok(ran.text) } else { Err(PluginError::Other(macmap::apple_event_error(&ran.text))) }
    }
}

/// Network and Wi-Fi. macOS has no built-in tool that scans for networks any more (`airport`
/// does nothing since macOS 14.4), so the list is the saved networks. Since macOS 14, it hides
/// the joined network's name from a program without Location Services: the network then shows
/// as "Wi-Fi".
impl Mac {
    async fn ports(&self) -> PluginResult<Vec<(String, String)>> {
        Ok(macmap::ports(&self.tools.read(NETWORKSETUP, &["-listallhardwareports"]).await?))
    }

    async fn wifi_device(&self) -> PluginResult<String> {
        macmap::wifi_device(&self.ports().await?)
            .ok_or_else(|| PluginError::Unsupported("this Mac has no Wi-Fi".into()))
    }

    async fn network(&self) -> (Option<NetworkStatus>, Vec<WifiNetwork>) {
        let (ports, nwi) = tokio::join!(self.ports(), self.tools.read(SCUTIL, &["--nwi"]));
        let Ok(ports) = ports else { return (None, Vec::new()) };
        let primary = nwi.ok().and_then(|text| macmap::primary(&text));
        let Some(wifi) = macmap::wifi_device(&ports) else {
            return (Some(macmap::network(primary.as_deref(), &ports, None)), Vec::new());
        };
        let (current, saved) = (["-getairportnetwork", &wifi], ["-listpreferredwirelessnetworks", &wifi]);
        let (current, saved) =
            tokio::join!(self.tools.read(NETWORKSETUP, &current), self.tools.read(NETWORKSETUP, &saved));
        let ssid = current.ok().and_then(|text| macmap::current_ssid(&text));
        let status = macmap::network(primary.as_deref(), &ports, ssid.clone());
        // The Wi-Fi's network is the joined one only while the Mac is online through it.
        let joined = ssid.filter(|_| status.connection == Connection::Wifi);
        let saved = saved.map(|text| macmap::preferred(&text)).unwrap_or_default();
        (Some(status), macmap::wifi(&saved, joined.as_deref()))
    }

    async fn join(&self, ssid: &str, password: Option<&str>, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        let device = self.wifi_device().await?;
        job.progress(None, "Joining");
        let (me, ssid, password, handle) = (self.clone(), ssid.to_owned(), password.map(str::to_owned), job.clone());
        tools::job(job, async move {
            let mut args = vec!["-setairportnetwork", &device, &ssid];
            args.extend(password.as_deref());
            let call = Call::job(&handle, Limit::After(JOIN_TIME));
            let ran = me.tools.run(NETWORKSETUP, &args, call, |_| {}).await.map_err(tools::reason)?;
            // It ends with 0 even when it failed: what it printed says.
            match macmap::join_error(&ran.text) {
                Some(why) => Err(why),
                None if ran.code != 0 => Err(crate::parse::error_line(&ran.last)),
                None => Ok("Connected".into()),
            }
        });
        Ok(())
    }
}

/// Bluetooth: `system_profiler` lists the paired devices and whether Bluetooth is on. macOS
/// has no built-in tool that changes any of it.
impl Mac {
    async fn bluetooth(&self) -> PluginResult<AreaState> {
        let json = self.tools.read(SYSTEM_PROFILER, &["SPBluetoothDataType", "-json"]).await?;
        Ok(match macmap::bluetooth(&json) {
            Some((powered, devices)) => AreaState::Ready(AreaData::Bluetooth { powered, devices }),
            None => AreaState::Unsupported("No Bluetooth adapter".into()),
        })
    }
}

/// Sound: the output's volume and mute through AppleScript, which needs no permission for them.
/// macOS has no built-in tool that lists or chooses the outputs.
impl Mac {
    async fn level(&self) -> PluginResult<Option<AudioLevel>> {
        Ok(macmap::level(&self.tools.read(OSASCRIPT, &["-e", "get volume settings"]).await?))
    }

    async fn set_volume(&self, volume: u8) -> PluginResult<()> {
        self.osascript(&format!("set volume output volume {}", volume.min(100)), Limit::Quick).await.map(drop)
    }

    async fn set_muted(&self, muted: bool) -> PluginResult<()> {
        self.osascript(&format!("set volume output muted {muted}"), Limit::Quick).await.map(drop)
    }
}

/// Restart, shut down and the battery. Restart and shut down ask System Events, as the Apple
/// menu does: `shutdown` needs root. macOS asks the player once whether the launcher may
/// control System Events.
impl Mac {
    async fn battery(&self) -> PluginResult<Option<Battery>> {
        Ok(crate::parse::pmset(&self.tools.read(PMSET, &["-g", "batt"]).await?))
    }

    async fn power(&self, verb: &str) -> PluginResult<()> {
        let script = format!("tell application \"System Events\" to {verb}");
        self.osascript(&script, Limit::After(ANSWER_TIME)).await.map(drop)
    }
}

/// Drives: the volumes on the external disks (`diskutil list external`), each read with
/// `diskutil info`. The Mac's own disks aren't listed.
impl Mac {
    async fn plist(&self, args: &[&str]) -> PluginResult<Value> {
        let xml = self.tools.read(DISKUTIL, args).await?;
        macmap::plist(&xml).ok_or_else(|| PluginError::Other("diskutil's answer couldn't be read".into()))
    }

    async fn drives(&self) -> PluginResult<Vec<SystemDrive>> {
        let list = self.plist(&["list", "-plist", "external"]).await?;
        let mut drives = Vec::new();
        for volume in macmap::volumes(&list) {
            // A volume that went away since the list is left out.
            if let Ok(info) = self.plist(&["info", "-plist", &volume.id]).await {
                drives.extend(macmap::drive(&info, &volume.disk));
            }
        }
        Ok(drives)
    }

    /// The drive at `device`, from a fresh list. Never one the system runs from.
    async fn drive(&self, device: &str) -> PluginResult<SystemDrive> {
        let drive = self.drives().await?.into_iter().find(|d| d.device == device);
        let drive = drive.ok_or_else(|| PluginError::InvalidInput(format!("{device} isn't an external drive")))?;
        if drive.system {
            return Err(PluginError::InvalidInput("the system runs from this drive".into()));
        }
        Ok(drive)
    }

    /// Runs `diskutil` for a job, for at most `limit`: what it printed, or why it failed.
    async fn diskutil(&self, args: &[&str], limit: Duration, job: &Arc<dyn SystemJob>) -> Result<String, String> {
        let ran = self.tools.run(DISKUTIL, args, Call::job(job, Limit::After(limit)), |_| {}).await;
        let ran = ran.map_err(tools::reason)?;
        if ran.code == 0 { Ok(ran.text) } else { Err(macmap::diskutil_error(&ran.text)) }
    }

    async fn mount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.drive(device).await?;
        job.progress(None, "Mounting");
        let (me, device, handle) = (self.clone(), device.to_owned(), job.clone());
        tools::job(job, async move {
            me.diskutil(&["mount", &device], MOUNT_TIME, &handle).await?;
            // diskutil's reply doesn't name the folder: its info does.
            let info = me.plist(&["info", "-plist", &device]).await.map_err(tools::reason)?;
            let at = info["MountPoint"].as_str().unwrap_or_default();
            if at.is_empty() {
                Err("diskutil mounted the drive but didn't say where".into())
            } else {
                Ok(at.to_owned())
            }
        });
        Ok(())
    }

    async fn unmount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.drive(device).await?;
        job.progress(None, "Writing what is left to the drive");
        let (me, device, handle) = (self.clone(), device.to_owned(), job.clone());
        tools::job(job, async move {
            me.diskutil(&["unmount", &device], UNMOUNT_TIME, &handle).await?;
            Ok("Unmounted".into())
        });
        Ok(())
    }

    /// `diskutil eject` unmounts each volume of the disk first, then ejects it. For an APFS
    /// volume the disk is the physical one that holds its container.
    async fn eject(&self, disk: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        let parts: Vec<SystemDrive> = self.drives().await?.into_iter().filter(|d| d.disk == disk).collect();
        if parts.is_empty() {
            return Err(PluginError::InvalidInput(format!("{disk} isn't an external drive")));
        }
        if parts.iter().any(|d| d.system) {
            return Err(PluginError::InvalidInput("the system runs from this drive".into()));
        }
        job.progress(None, "Writing what is left to the drive");
        let (me, disk, handle) = (self.clone(), disk.to_owned(), job.clone());
        tools::job(job, async move {
            me.diskutil(&["eject", &disk], EJECT_TIME, &handle).await?;
            Ok("You can take it out now".into())
        });
        Ok(())
    }
}

/// VPNs: macOS can't tell which network a download should go through, so the list has every
/// network with an address, and the player picks the one the VPN uses. Next to them are the
/// VPNs System Settings knows (`scutil --nc list`), which scutil starts and stops. Those have no
/// interface, so a download never goes through one by its name.
impl Mac {
    async fn services(&self) -> Vec<Vpn> {
        let text = self.tools.read(SCUTIL, &["--nc", "list"]).await;
        text.map(|text| macmap::services(&text)).unwrap_or_default()
    }

    async fn vpns(&self) -> Vec<Vpn> {
        let addressed = crate::unix::addressed().into_iter().map(|name| Vpn {
            kind: "Network".into(),
            connected: true,
            interface: name.clone(),
            switchable: false,
            name,
        });
        addressed.chain(self.services().await).collect()
    }

    /// Fails unless `name` is a VPN that scutil can switch.
    async fn service(&self, name: &str) -> PluginResult<()> {
        if self.services().await.iter().any(|v| v.name == name && v.switchable) {
            Ok(())
        } else {
            Err(PluginError::Unsupported("this system can't change that network: use the VPN's own app".into()))
        }
    }

    async fn status(&self, name: &str) -> String {
        let text = self.tools.read(SCUTIL, &["--nc", "status", name]).await.unwrap_or_default();
        macmap::nc_status(&text).to_owned()
    }

    /// scutil only asks for the VPN to start: its state then says whether it did.
    async fn connect(&self, name: &str) -> PluginResult<()> {
        self.service(name).await?;
        self.tools.act(SCUTIL, &["--nc", "start", name]).await?;
        let started = Instant::now();
        while started.elapsed() < VPN_TIME {
            match self.status(name).await.as_str() {
                "Connected" => return Ok(()),
                "Disconnected" | "Invalid" if started.elapsed() >= VPN_GRACE => {
                    return Err(PluginError::Other("The VPN didn't connect: open its app to see why".into()));
                }
                _ => tokio::time::sleep(VPN_POLL).await,
            }
        }
        // Still connecting: the next state shows how it ends.
        Ok(())
    }

    async fn disconnect(&self, name: &str) -> PluginResult<()> {
        self.service(name).await?;
        self.tools.act(SCUTIL, &["--nc", "stop", name]).await.map(drop)
    }
}

/// One of the areas macOS manages, through the same interface as every other.
struct MacArea {
    area: SystemArea,
    mac: Mac,
}

fn refuse(why: &str) -> PluginResult<()> {
    Err(PluginError::Unsupported(why.into()))
}

#[launcher_api::async_trait]
impl AreaSettings for MacArea {
    async fn state(&self) -> PluginResult<AreaState> {
        let mac = &self.mac;
        let missing = match self.area {
            // The networks with an address list without scutil, and the battery without osascript.
            SystemArea::Vpn | SystemArea::Power => None,
            area => mac.missing(area).await,
        };
        if let Some(why) = missing {
            return Ok(AreaState::Unsupported(why));
        }
        Ok(match self.area {
            SystemArea::Network => {
                let (status, wifi) = mac.network().await;
                AreaState::Ready(AreaData::Network { status, wifi })
            }
            SystemArea::Vpn => {
                AreaState::Ready(AreaData::Vpn { finds_vpns: false, can_import: false, vpns: mac.vpns().await })
            }
            SystemArea::Bluetooth => mac.bluetooth().await?,
            SystemArea::Audio => AreaState::Ready(AreaData::Audio { outputs: Vec::new(), level: mac.level().await? }),
            SystemArea::Power => {
                let (refused, battery) = tokio::join!(mac.tools.missing(&[OSASCRIPT]), mac.battery());
                AreaState::Ready(AreaData::Power { battery: battery.ok().flatten(), refused })
            }
            SystemArea::Drives => AreaState::Ready(AreaData::Drives { drives: mac.drives().await? }),
            SystemArea::OsUpdates | SystemArea::OsInstall => AreaState::Absent,
        })
    }

    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        use AreaAction as A;
        let mac = &self.mac;
        match action {
            A::JoinWifi { ssid, password } => mac.join(&ssid, password.as_deref(), job).await,
            A::ScanWifi => {
                refuse("macOS has no built-in tool that looks for Wi-Fi networks: the list shows the saved ones")
            }
            A::ForgetWifi { .. } => {
                refuse("macOS asks for an administrator to forget a network: forget it in System Settings, Wi-Fi")
            }
            A::ConnectVpn { name } => mac.connect(&name).await,
            A::DisconnectVpn { name } => mac.disconnect(&name).await,
            A::ImportVpn { .. } => refuse("macOS adds a VPN in System Settings, VPN, or in the VPN's own app"),
            A::SetBluetooth { .. } => {
                refuse("macOS has no built-in tool that turns Bluetooth on or off: use the Bluetooth menu")
            }
            A::ScanBluetooth { .. } | A::Pair { .. } => {
                refuse("macOS has no built-in tool that pairs a device: pair it in System Settings, Bluetooth")
            }
            A::ConnectDevice { .. } | A::DisconnectDevice { .. } | A::RemoveDevice { .. } => {
                refuse("macOS has no built-in tool that connects or forgets a device: use System Settings, Bluetooth")
            }
            A::SetOutput { .. } => {
                refuse("macOS has no built-in tool that chooses the sound output: choose it in the Sound menu")
            }
            A::SetVolume { volume } => mac.set_volume(volume).await,
            A::SetMuted { muted } => mac.set_muted(muted).await,
            A::Restart => mac.power("restart").await,
            A::ShutDown => mac.power("shut down").await,
            A::Mount { device } => mac.mount(&device, job).await,
            A::Unmount { device } => mac.unmount(&device, job).await,
            A::Eject { disk } => mac.eject(&disk, job).await,
            A::CheckOsUpdate | A::InstallOs { .. } | A::ChoosePartitions => {
                refuse("macOS updates itself in System Settings, General, Software Update")
            }
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

    async fn state(area: SystemArea) -> AreaState {
        adapter(area, Arc::new(Runner::new(Arc::new(Nothing)))).state().await.unwrap()
    }

    #[tokio::test]
    async fn without_the_tools_each_area_says_which_one_is_missing() {
        let unsupported = |why: &str| AreaState::Unsupported(why.into());
        assert_eq!(state(SystemArea::Network).await, unsupported("networksetup (networksetup) isn't installed"));
        assert_eq!(
            state(SystemArea::Bluetooth).await,
            unsupported("System Information's tool (system_profiler) isn't installed")
        );
        assert_eq!(state(SystemArea::Audio).await, unsupported("AppleScript (osascript) isn't installed"));
        assert_eq!(state(SystemArea::Drives).await, unsupported("Disk Utility's tool (diskutil) isn't installed"));
    }

    #[tokio::test]
    async fn without_osascript_restart_is_refused_and_the_vpns_still_list() {
        let AreaState::Ready(AreaData::Power { refused, .. }) = state(SystemArea::Power).await else {
            panic!("power is ready: the battery reads without the tools");
        };
        assert_eq!(refused.as_deref(), Some("AppleScript (osascript) isn't installed"));
        assert!(matches!(state(SystemArea::Vpn).await, AreaState::Ready(AreaData::Vpn { finds_vpns: false, .. })));
    }

    #[test]
    fn the_os_s_own_updates_and_installer_aren_t_managed() {
        assert!(AREAS.iter().all(|(area, _)| !matches!(area, SystemArea::OsUpdates | SystemArea::OsInstall)));
    }
}

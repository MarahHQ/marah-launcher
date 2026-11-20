//! What a system plugin provides: the machine's own settings and devices. Network and Wi-Fi,
//! VPNs, Bluetooth, sound, power and battery, drives, and on Marah OS its updates and its
//! installer.
//!
//! Each of them is a [`SystemArea`], and every area has the same interface, [`AreaSettings`]:
//! [`AreaSettings::state`] reads the area, and [`AreaSettings::act`] runs one [`AreaAction`] on
//! it. A state is [`AreaState::Absent`], [`AreaState::Unsupported`] with the reason, or
//! [`AreaState::Ready`] with the area's data. The core uses the first active system plugin that
//! provides an area, and shows an area nobody provides as absent.
//!
//! Quick calls answer within the host's call timeout. Long work (joining a network, a
//! Bluetooth scan, pairing, installing the OS, mounting) starts in the background and returns
//! at once; it reports through the [`SystemJob`] the host passes in.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::PluginResult;

/// A long piece of work, as the host follows it. Each method may be called from any thread.
pub trait SystemJob: Send + Sync {
    /// How far it is: a percent when the plugin knows one, and what it's doing.
    fn progress(&self, percent: Option<u8>, text: &str);
    /// It ended well; `text` says how ("Connected").
    fn done(&self, text: &str);
    /// It ended badly; `reason` says why, for the player.
    fn failed(&self, reason: &str);
    /// The player cancelled it: stop at the next chance, then call [`SystemJob::failed`].
    fn cancelled(&self) -> bool;
}

/// How the machine is online.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Connection {
    #[default]
    None,
    Wired,
    Wifi,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub connection: Connection,
    /// The Wi-Fi network's name, or the wired connection's.
    pub name: String,
    /// The Wi-Fi signal, 0 to 100.
    pub signal: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0 to 100.
    pub signal: u8,
    /// It needs a password.
    pub secured: bool,
    /// The machine has joined it before and joins it again by itself.
    pub saved: bool,
    /// The machine is on it now.
    pub active: bool,
}

/// A VPN, or on a system that can't tell VPNs from other networks, any network with an
/// address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vpn {
    /// The name the player knows it by. Connect and disconnect take it.
    pub name: String,
    /// "WireGuard", "OpenVPN", "VPN", or "Network" when the system can't tell.
    pub kind: String,
    /// Up and with an address: traffic can go through it.
    pub connected: bool,
    /// The network interface that downloads go through while it is connected. Empty when the
    /// system doesn't name one.
    pub interface: String,
    /// The system can connect and disconnect it.
    pub switchable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutput {
    /// The system's name for it, which [`AreaAction::SetOutput`] takes.
    pub name: String,
    /// What the player sees: "HDMI / DisplayPort", "Headset".
    pub description: String,
    pub current: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLevel {
    /// 0 to 100.
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    /// 0 to 100.
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum OsUpdate {
    #[default]
    Current,
    Checking,
    /// Downloaded: a restart puts it in.
    Staged {
        version: String,
    },
    Failed {
        reason: String,
    },
}

/// A disk Marah OS can be installed on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallDisk {
    pub path: PathBuf,
    pub bytes: u64,
    /// Windows is on it.
    pub windows: bool,
    pub usb: bool,
    pub model: String,
}

/// How an install ended well.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    /// The edition installed: `minimal` or `minimal-nvidia`.
    pub edition: String,
    /// Secure Boot's key screen comes at the restart.
    pub mok: bool,
    /// The PC has an NVIDIA card and gets its driver on the installed system.
    pub nvidia_later: bool,
}

/// A drive as the system sees it: a USB stick, a disk, or a part of a disk with a file system.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemDrive {
    /// The device that mounts, such as `/dev/sdb1`.
    pub device: String,
    /// The whole disk it is on, such as `/dev/sdb`: what [`AreaAction::Eject`] powers off.
    pub disk: String,
    /// The file system's UUID: it stays the same from one plug-in to the next. Empty when the
    /// file system has none.
    pub uuid: String,
    pub label: String,
    /// The disk's model, such as "SanDisk Ultra". Empty when the system doesn't say.
    pub model: String,
    pub size: u64,
    /// Such as `ext4`, `exfat` or `ntfs`.
    pub filesystem: String,
    /// The player can take it out: it is plugged in by USB, or into a port like a card reader.
    pub usb: bool,
    /// Where it is mounted; `None` while it isn't.
    pub mount: Option<PathBuf>,
    /// Its free bytes, while it is mounted and the system says.
    pub free: Option<u64>,
    /// The system mounted it for itself (`/`, `/home`, `/boot`, swap): the launcher never
    /// unmounts it.
    pub system: bool,
}

/// One part of the machine that a system plugin manages. Each is one capability
/// ([`SystemArea::capability`]) and one entry in [`crate::Provided::areas`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemArea {
    /// Network and Wi-Fi.
    Network,
    /// VPNs, for "Download only through a VPN".
    Vpn,
    /// Bluetooth: controllers and headsets.
    Bluetooth,
    /// Sound: where it goes and how loud.
    Audio,
    /// Restart, shut down, and a laptop's battery.
    Power,
    /// Marah OS's updates: the system downloads them by itself, and a restart puts one in.
    OsUpdates,
    /// Installing Marah OS from its USB stick.
    OsInstall,
    /// Drives: listing, mounting, unmounting and ejecting them.
    Drives,
}

impl SystemArea {
    /// Every area, in the order the view model lists them.
    pub const ALL: [SystemArea; 8] = [
        SystemArea::Network,
        SystemArea::Vpn,
        SystemArea::Bluetooth,
        SystemArea::Audio,
        SystemArea::Power,
        SystemArea::OsUpdates,
        SystemArea::OsInstall,
        SystemArea::Drives,
    ];

    /// The capability a manifest declares for the area.
    pub fn capability(self) -> crate::Capability {
        use crate::Capability as C;
        match self {
            SystemArea::Network => C::Network,
            SystemArea::Vpn => C::Vpn,
            SystemArea::Bluetooth => C::Bluetooth,
            SystemArea::Audio => C::Audio,
            SystemArea::Power => C::Power,
            SystemArea::OsUpdates => C::OsUpdates,
            SystemArea::OsInstall => C::OsInstall,
            SystemArea::Drives => C::Drives,
        }
    }

    /// The area's name, as the manifest's capability and the view model name it: `network`,
    /// `os_updates`.
    pub fn name(self) -> &'static str {
        match self {
            SystemArea::Network => "network",
            SystemArea::Vpn => "vpn",
            SystemArea::Bluetooth => "bluetooth",
            SystemArea::Audio => "audio",
            SystemArea::Power => "power",
            SystemArea::OsUpdates => "os_updates",
            SystemArea::OsInstall => "os_install",
            SystemArea::Drives => "drives",
        }
    }
}

/// What an area is on this machine now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "data", rename_all = "snake_case")]
pub enum AreaState {
    /// The machine has no such thing to manage. The core also shows an area that no plugin
    /// provides as absent.
    Absent,
    /// The area can't be used here, and why, for the player: "No Bluetooth adapter",
    /// "NetworkManager isn't installed".
    Unsupported(String),
    /// The area works here, with what it holds now.
    Ready(AreaData),
}

/// What a ready area holds. Each area has its own kind of data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "area", rename_all = "snake_case")]
pub enum AreaData {
    Network {
        /// `None` when the system didn't say.
        status: Option<NetworkStatus>,
        /// The Wi-Fi networks in reach, strongest first, as the last scan found them.
        wifi: Vec<WifiNetwork>,
    },
    Vpn {
        /// The system tells VPNs from other networks. When it can't (macOS), `vpns` lists every
        /// network with an address, and the player picks the one the VPN uses.
        finds_vpns: bool,
        /// The system can add a VPN from a provider's file ([`AreaAction::ImportVpn`]).
        can_import: bool,
        vpns: Vec<Vpn>,
    },
    Bluetooth {
        powered: bool,
        /// Paired devices, and the ones the last scan found.
        devices: Vec<BluetoothDevice>,
    },
    Audio {
        outputs: Vec<AudioOutput>,
        /// The current output's level; `None` when the system didn't say.
        level: Option<AudioLevel>,
    },
    Power {
        /// `None` on a PC without a battery.
        battery: Option<Battery>,
        /// Why restart and shut down are refused here ("Restart from the system's own menu");
        /// `None` when they work. The battery still shows.
        refused: Option<String>,
    },
    OsUpdates {
        /// `None` when the system didn't say.
        update: Option<OsUpdate>,
    },
    OsInstall {
        /// The disks Marah OS can be installed on.
        disks: Vec<InstallDisk>,
    },
    Drives {
        /// Every drive with a file system, mounted or not.
        drives: Vec<SystemDrive>,
    },
}

/// One thing the player asks of an area. Each action belongs to one area
/// ([`AreaAction::area`]). An adapter that can't run an action here returns
/// [`crate::PluginError::Unsupported`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AreaAction {
    /// Looks for Wi-Fi networks again (a few seconds). The next state lists what it found.
    ScanWifi,
    /// Starts joining `ssid`, with its password when it's secured and not saved. Long.
    JoinWifi {
        ssid: String,
        password: Option<String>,
    },
    /// Forgets a saved network: it isn't joined by itself any more.
    ForgetWifi {
        ssid: String,
    },
    /// Adds a VPN from a provider's file (WireGuard `.conf`, OpenVPN `.ovpn`), wherever the
    /// player keeps it, and connects it.
    ImportVpn {
        file: PathBuf,
    },
    ConnectVpn {
        name: String,
    },
    DisconnectVpn {
        name: String,
    },
    SetBluetooth {
        on: bool,
    },
    /// Starts looking for devices to pair for `seconds`. Long.
    ScanBluetooth {
        seconds: u32,
    },
    /// Starts pairing, trusting and connecting a device the scan found. Long.
    Pair {
        address: String,
    },
    ConnectDevice {
        address: String,
    },
    DisconnectDevice {
        address: String,
    },
    /// Forgets a paired device.
    RemoveDevice {
        address: String,
    },
    SetOutput {
        name: String,
    },
    /// 0 to 100.
    SetVolume {
        volume: u8,
    },
    SetMuted {
        muted: bool,
    },
    Restart,
    ShutDown,
    /// Looks for an update now rather than at the next daily check.
    CheckOsUpdate,
    /// Starts erasing `disk` and installing Marah OS on it. Long: the job's `done` text is
    /// [`Installed`] as JSON.
    InstallOs {
        disk: PathBuf,
    },
    /// Starts the installer that lets the player choose partitions (Anaconda). It may restart
    /// the PC into it.
    ChoosePartitions,
    /// Starts mounting `device`. Long: the job's `done` text is the folder it is mounted at.
    Mount {
        device: String,
    },
    /// Starts unmounting `device`. Long: an unmount writes what is left to the drive first.
    Unmount {
        device: String,
    },
    /// Starts unmounting every part of `disk` that is mounted, then powers `disk` off, so the
    /// player can take it out. Long.
    Eject {
        disk: String,
    },
}

impl AreaAction {
    /// The area the action belongs to.
    pub fn area(&self) -> SystemArea {
        use AreaAction as A;
        match self {
            A::ScanWifi | A::JoinWifi { .. } | A::ForgetWifi { .. } => SystemArea::Network,
            A::ImportVpn { .. } | A::ConnectVpn { .. } | A::DisconnectVpn { .. } => SystemArea::Vpn,
            A::SetBluetooth { .. }
            | A::ScanBluetooth { .. }
            | A::Pair { .. }
            | A::ConnectDevice { .. }
            | A::DisconnectDevice { .. }
            | A::RemoveDevice { .. } => SystemArea::Bluetooth,
            A::SetOutput { .. } | A::SetVolume { .. } | A::SetMuted { .. } => SystemArea::Audio,
            A::Restart | A::ShutDown => SystemArea::Power,
            A::CheckOsUpdate => SystemArea::OsUpdates,
            A::InstallOs { .. } | A::ChoosePartitions => SystemArea::OsInstall,
            A::Mount { .. } | A::Unmount { .. } | A::Eject { .. } => SystemArea::Drives,
        }
    }

    /// The action is long work: [`AreaSettings::act`] starts it, returns at once, and the work
    /// ends through its job. A quick action ends before `act` returns and leaves its job alone.
    pub fn long(&self) -> bool {
        use AreaAction as A;
        matches!(
            self,
            A::JoinWifi { .. }
                | A::ScanBluetooth { .. }
                | A::Pair { .. }
                | A::InstallOs { .. }
                | A::Mount { .. }
                | A::Unmount { .. }
                | A::Eject { .. }
        )
    }
}

/// One system area, as a system plugin manages it. The same interface serves every area: an
/// adapter for one OS answers [`AreaSettings::state`] and runs the actions it can.
#[async_trait]
pub trait AreaSettings: Send + Sync {
    /// What the area is now. Reading it may run the system's tools; the core reads it on its
    /// tick and after each action.
    async fn state(&self) -> PluginResult<AreaState>;
    /// Runs `action`, which belongs to this area. A long action reports through `job`
    /// ([`AreaAction::long`]); a quick one leaves `job` alone.
    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_area_names_its_own_capability() {
        let capabilities: Vec<_> = SystemArea::ALL.iter().map(|a| a.capability()).collect();
        assert_eq!(capabilities, crate::Capability::SYSTEM);
        for area in SystemArea::ALL {
            let named: crate::Capability = serde_json::from_value(serde_json::json!(area.name())).unwrap();
            assert_eq!(named, area.capability(), "{area:?}");
        }
    }

    #[test]
    fn only_the_actions_that_start_work_are_long() {
        assert!(AreaAction::Mount { device: "/dev/sdb1".into() }.long());
        assert!(!AreaAction::SetVolume { volume: 3 }.long());
        assert_eq!(AreaAction::Eject { disk: "/dev/sdb".into() }.area(), SystemArea::Drives);
        assert_eq!(AreaAction::ImportVpn { file: "a.conf".into() }.area(), SystemArea::Vpn);
    }
}

//! [`FakeSystem`]: a system plugin the test controls. It has a machine's network, Wi-Fi, Bluetooth,
//! sound, power and battery, drives, and Marah OS's updates and installer, as plain state the test sets
//! and reads. Its long work (joining, scanning, pairing, installing) ends when the test says, or
//! at once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, AudioLevel, AudioOutput, Battery, BluetoothDevice, InstallDisk,
    Installed, NetworkStatus, OsUpdate, PluginError, PluginResult, Provided, SystemArea, SystemDrive, SystemJob, Vpn,
    WifiNetwork,
};

use crate::StubPlugin;

/// What the fake machine is, and what was asked of it.
#[derive(Default)]
pub struct FakeMachine {
    pub network: NetworkStatus,
    /// What a plain read lists.
    pub wifi: Vec<WifiNetwork>,
    /// What a rescan finds.
    pub wifi_scanned: Vec<WifiNetwork>,
    /// The password each secured network takes.
    pub passwords: Vec<(String, String)>,
    pub forgotten: Vec<String>,
    pub bluetooth_on: bool,
    pub devices: Vec<BluetoothDevice>,
    /// What a Bluetooth scan finds.
    pub found: Vec<BluetoothDevice>,
    /// Pairing these fails with the reason.
    pub pair_refusals: Vec<(String, String)>,
    pub outputs: Vec<AudioOutput>,
    pub level: AudioLevel,
    pub battery: Option<Battery>,
    pub restarts: usize,
    pub shutdowns: usize,
    pub os_update: OsUpdate,
    pub os_checks: usize,
    pub disks: Vec<InstallDisk>,
    /// The progress an install reports, then how it ends.
    pub install_steps: Vec<(u8, String)>,
    pub install_result: Option<Result<Installed, String>>,
    pub installed_on: Vec<PathBuf>,
    pub partition_choices: usize,
    pub vpns: Vec<Vpn>,
    /// The drives, as the system lists them.
    pub drives: Vec<SystemDrive>,
    /// Where a mount puts a drive: a folder named by its label in here, which the mount makes.
    /// Without it, `/media/fake/<label>`, which isn't made.
    pub mount_at: Option<PathBuf>,
    /// Mounting these devices fails with the reason.
    pub mount_refusals: Vec<(String, String)>,
    /// What was asked of the drives: `mount /dev/sda1`, `unmount …`, `eject /dev/sda`.
    pub drive_actions: Vec<String>,
    /// How many times the core asked why drives can't be managed here.
    pub drive_reason_reads: usize,
    /// Why an area is unavailable, by its name ("network", "vpn", "bluetooth", "audio", "power",
    /// "drives").
    pub unavailable: Vec<(&'static str, String)>,
    /// Calls that never answer, by method name ("status", "devices", …).
    pub hangs: Vec<&'static str>,
    /// Calls that panic, by method name.
    pub panics: Vec<&'static str>,
    /// Calls that take this long on the fake clock, by method name ("status",
    /// "drives.unavailable", …).
    pub slow: Vec<(&'static str, std::time::Duration)>,
    /// Each read the core made, by method name, with when it began and ended on the fake clock.
    pub calls: Vec<(&'static str, tokio::time::Instant, tokio::time::Instant)>,
    /// Long work held until the test calls [`FakeSystem::finish_jobs`].
    pub hold_jobs: bool,
    held: Vec<(Arc<dyn SystemJob>, Result<String, String>)>,
    /// A rescan ran: a plain read lists what it found, as NetworkManager's list does.
    scanned: bool,
}

/// A system plugin over a [`FakeMachine`]. Clones share the machine.
#[derive(Clone, Default)]
pub struct FakeSystem {
    machine: Arc<Mutex<FakeMachine>>,
}

impl FakeSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes the machine.
    pub fn with(&self, change: impl FnOnce(&mut FakeMachine)) -> &Self {
        change(&mut self.machine.lock().unwrap());
        self
    }

    /// Reads the machine.
    pub fn read<T>(&self, read: impl FnOnce(&FakeMachine) -> T) -> T {
        read(&self.machine.lock().unwrap())
    }

    /// Ends the held long work, each as it would have.
    pub fn finish_jobs(&self) {
        let held = std::mem::take(&mut self.machine.lock().unwrap().held);
        for (job, result) in held {
            match result {
                Ok(text) => job.done(&text),
                Err(reason) => job.failed(&reason),
            }
        }
    }

    /// The plugin, `fake-system`, providing every system capability.
    pub fn plugin(&self) -> StubPlugin {
        self.plugin_with(&["network", "vpn", "bluetooth", "audio", "power", "os_updates", "os_install", "drives"])
    }

    /// The plugin, providing only `capabilities` (as the manifest names them).
    pub fn plugin_with(&self, capabilities: &[&str]) -> StubPlugin {
        let list = capabilities.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
        let manifest = format!(
            "id = \"fake-system\"\nname = \"Fake system\"\nversion = \"1.0.0\"\nkind = \"system\"\n\
             host_api = \"*\"\ncapabilities = [{list}]\n"
        );
        let names: Vec<String> = capabilities.iter().map(|c| c.to_string()).collect();
        let system = self.clone();
        StubPlugin::from_toml(&manifest).provides(move |_| {
            let has = |name: &str| names.iter().any(|n| n == name);
            let me = system.clone();
            let areas = SystemArea::ALL.into_iter().filter(|area| has(area.name()));
            let areas =
                areas.map(|area| (area, Arc::new(FakeArea { area, system: me.clone() }) as Arc<dyn AreaSettings>));
            Ok(Provided { areas: areas.collect(), ..Provided::default() })
        })
    }

    /// Hangs or panics when the test said so for `method`.
    /// Hangs, panics or takes long when the test said so for `method`, and keeps when it began
    /// and ended.
    async fn call(&self, method: &'static str) {
        let began = tokio::time::Instant::now();
        let (hang, panic) = self.read(|m| (m.hangs.contains(&method), m.panics.contains(&method)));
        let slow = self.read(|m| m.slow.iter().find(|(name, _)| *name == method).map(|(_, time)| *time));
        if panic {
            panic!("the fake system's {method} panicked on purpose");
        }
        if hang {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
        if let Some(time) = slow {
            tokio::time::sleep(time).await;
        }
        self.with(|m| m.calls.push((method, began, tokio::time::Instant::now())));
    }

    fn unavailable(&self, capability: &str) -> Option<String> {
        self.read(|m| m.unavailable.iter().find(|(c, _)| *c == capability).map(|(_, why)| why.clone()))
    }

    /// Ends long work now, or holds it for [`FakeSystem::finish_jobs`].
    fn end(&self, job: Arc<dyn SystemJob>, result: Result<String, String>) {
        let mut machine = self.machine.lock().unwrap();
        if machine.hold_jobs {
            machine.held.push((job, result));
            return;
        }
        drop(machine);
        match result {
            Ok(text) => job.done(&text),
            Err(reason) => job.failed(&reason),
        }
    }
}

impl FakeSystem {
    async fn network_unavailable(&self) -> Option<String> {
        self.call("network.unavailable").await;
        self.unavailable("network")
    }

    async fn network_status(&self) -> PluginResult<NetworkStatus> {
        self.call("status").await;
        Ok(self.read(|m| m.network.clone()))
    }

    async fn wifi(&self, rescan: bool) -> PluginResult<Vec<WifiNetwork>> {
        self.call("wifi").await;
        if rescan {
            self.with(|m| m.scanned = true);
        }
        Ok(self.read(|m| if m.scanned { m.wifi_scanned.clone() } else { m.wifi.clone() }))
    }

    async fn join(&self, ssid: &str, password: Option<&str>, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.call("join").await;
        job.progress(None, &format!("Joining {ssid}"));
        let wanted = self.read(|m| m.passwords.iter().find(|(s, _)| s == ssid).map(|(_, p)| p.clone()));
        let result = match (wanted, password) {
            (Some(wanted), Some(given)) if wanted == given => Ok(()),
            (Some(_), _) => Err("The password is wrong".to_owned()),
            (None, _) => Ok(()),
        };
        if result.is_ok() {
            self.with(|m| {
                let connection = launcher_api::Connection::Wifi;
                m.network = NetworkStatus { connection, name: ssid.into(), signal: Some(80) };
                for w in m.wifi.iter_mut().chain(m.wifi_scanned.iter_mut()) {
                    w.active = w.ssid == ssid;
                    w.saved |= w.ssid == ssid;
                }
            });
        }
        self.end(job, result.map(|()| "Connected".to_owned()));
        Ok(())
    }

    async fn forget(&self, ssid: &str) -> PluginResult<()> {
        self.with(|m| m.forgotten.push(ssid.into()));
        Ok(())
    }
}

impl FakeSystem {
    async fn vpns(&self) -> Vec<Vpn> {
        self.read(|m| m.vpns.clone())
    }
}

impl FakeSystem {
    async fn bluetooth_unavailable(&self) -> Option<String> {
        self.call("bluetooth.unavailable").await;
        self.unavailable("bluetooth")
    }

    async fn powered(&self) -> PluginResult<bool> {
        self.call("powered").await;
        Ok(self.read(|m| m.bluetooth_on))
    }

    async fn set_powered(&self, on: bool) -> PluginResult<()> {
        self.with(|m| m.bluetooth_on = on);
        Ok(())
    }

    async fn devices(&self) -> PluginResult<Vec<BluetoothDevice>> {
        self.call("devices").await;
        Ok(self.read(|m| m.devices.clone()))
    }

    async fn scan_bluetooth(&self, _seconds: u32, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Looking for devices");
        self.with(|m| {
            let found = m.found.clone();
            for device in found {
                if !m.devices.iter().any(|d| d.address == device.address) {
                    m.devices.push(device);
                }
            }
        });
        self.end(job, Ok("Done looking".into()));
        Ok(())
    }

    async fn pair(&self, address: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Pairing");
        let refusal = self.read(|m| m.pair_refusals.iter().find(|(a, _)| a == address).map(|(_, why)| why.clone()));
        if refusal.is_none() {
            self.with(|m| {
                for d in m.devices.iter_mut().filter(|d| d.address == address) {
                    d.paired = true;
                    d.connected = true;
                }
            });
        }
        self.end(job, refusal.map_or(Ok("Connected".into()), Err));
        Ok(())
    }

    async fn connect_device(&self, address: &str) -> PluginResult<()> {
        self.with(|m| m.devices.iter_mut().filter(|d| d.address == address).for_each(|d| d.connected = true));
        Ok(())
    }

    async fn disconnect_device(&self, address: &str) -> PluginResult<()> {
        self.with(|m| m.devices.iter_mut().filter(|d| d.address == address).for_each(|d| d.connected = false));
        Ok(())
    }

    async fn remove_device(&self, address: &str) -> PluginResult<()> {
        self.with(|m| m.devices.retain(|d| d.address != address));
        Ok(())
    }
}

impl FakeSystem {
    async fn audio_unavailable(&self) -> Option<String> {
        self.call("audio.unavailable").await;
        self.unavailable("audio")
    }

    async fn outputs(&self) -> PluginResult<Vec<AudioOutput>> {
        self.call("outputs").await;
        Ok(self.read(|m| m.outputs.clone()))
    }

    async fn set_output(&self, name: &str) -> PluginResult<()> {
        if !self.read(|m| m.outputs.iter().any(|o| o.name == name)) {
            return Err(PluginError::NotFound(format!("no output {name}")));
        }
        self.with(|m| m.outputs.iter_mut().for_each(|o| o.current = o.name == name));
        Ok(())
    }

    async fn level(&self) -> PluginResult<AudioLevel> {
        self.call("level").await;
        Ok(self.read(|m| m.level))
    }

    async fn set_volume(&self, volume: u8) -> PluginResult<()> {
        self.with(|m| m.level = AudioLevel { volume, muted: false });
        Ok(())
    }

    async fn set_muted(&self, muted: bool) -> PluginResult<()> {
        self.with(|m| m.level.muted = muted);
        Ok(())
    }
}

impl FakeSystem {
    async fn power_unavailable(&self) -> Option<String> {
        self.call("power.unavailable").await;
        self.unavailable("power")
    }

    async fn restart(&self) -> PluginResult<()> {
        self.call("restart").await;
        self.with(|m| m.restarts += 1);
        Ok(())
    }

    async fn shut_down(&self) -> PluginResult<()> {
        self.call("shut_down").await;
        self.with(|m| m.shutdowns += 1);
        Ok(())
    }

    async fn battery(&self) -> PluginResult<Option<Battery>> {
        self.call("battery").await;
        Ok(self.read(|m| m.battery))
    }
}

impl FakeSystem {
    async fn os_update(&self) -> PluginResult<OsUpdate> {
        Ok(self.read(|m| m.os_update.clone()))
    }

    async fn check_now(&self) -> PluginResult<()> {
        self.with(|m| m.os_checks += 1);
        Ok(())
    }
}

impl FakeSystem {
    async fn disks(&self) -> PluginResult<Vec<InstallDisk>> {
        Ok(self.read(|m| m.disks.clone()))
    }

    async fn install(&self, disk: &Path, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        let (steps, result) = self.read(|m| (m.install_steps.clone(), m.install_result.clone()));
        for (percent, text) in steps {
            job.progress(Some(percent), &text);
        }
        self.with(|m| m.installed_on.push(disk.to_path_buf()));
        let result = result.unwrap_or(Ok(Installed { edition: "minimal".into(), mok: false, nvidia_later: false }));
        let result = result.map(|done| launcher_api::serde_json::to_string(&done).expect("Installed is JSON"));
        self.end(job, result);
        Ok(())
    }

    async fn choose_partitions(&self) -> PluginResult<()> {
        self.with(|m| m.partition_choices += 1);
        Ok(())
    }
}

impl FakeSystem {
    async fn drives_unavailable(&self) -> Option<String> {
        self.with(|m| m.drive_reason_reads += 1);
        self.call("drives.unavailable").await;
        self.unavailable("drives")
    }

    async fn drives(&self) -> PluginResult<Vec<SystemDrive>> {
        self.call("drives").await;
        Ok(self.read(|m| m.drives.clone()))
    }

    async fn mount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.call("mount").await;
        let refused = self.read(|m| m.mount_refusals.iter().find(|(d, _)| d == device).map(|(_, why)| why.clone()));
        self.with(|m| m.drive_actions.push(format!("mount {device}")));
        if let Some(why) = refused {
            self.end(job, Err(why));
            return Ok(());
        }
        let label = self.read(|m| m.drives.iter().find(|d| d.device == device).map(|d| d.label.clone()));
        let label = label.ok_or_else(|| PluginError::NotFound(format!("no drive {device}")))?;
        let at = match self.read(|m| m.mount_at.clone()) {
            Some(root) => {
                let at = root.join(&label);
                std::fs::create_dir_all(&at).map_err(|e| PluginError::Storage(e.to_string()))?;
                at
            }
            None => PathBuf::from("/media/fake").join(&label),
        };
        self.with(|m| {
            for drive in m.drives.iter_mut().filter(|d| d.device == device) {
                drive.mount = Some(at.clone());
            }
        });
        self.end(job, Ok(at.display().to_string()));
        Ok(())
    }

    async fn unmount(&self, device: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.with(|m| {
            m.drive_actions.push(format!("unmount {device}"));
            for drive in m.drives.iter_mut().filter(|d| d.device == device) {
                drive.mount = None;
            }
        });
        self.end(job, Ok("Unmounted".into()));
        Ok(())
    }

    async fn eject(&self, disk: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.with(|m| {
            m.drive_actions.push(format!("eject {disk}"));
            m.drives.retain(|d| d.disk != disk);
        });
        self.end(job, Ok("You can take it out now".into()));
        Ok(())
    }
}

/// One area of the fake machine, through the interface every system plugin gives. Its reads are
/// the fake's named reads ("status", "wifi", "battery", "drives.unavailable"…), each that doesn't
/// need another begun at once, so a test can hang, slow or panic one of them.
struct FakeArea {
    area: SystemArea,
    system: FakeSystem,
}

#[async_trait]
impl AreaSettings for FakeArea {
    async fn state(&self) -> PluginResult<AreaState> {
        let system = &self.system;
        let unsupported = AreaState::Unsupported;
        Ok(match self.area {
            SystemArea::Network => match system.network_unavailable().await {
                Some(why) => unsupported(why),
                None => {
                    let (status, wifi) = futures::join!(system.network_status(), system.wifi(false));
                    AreaState::Ready(AreaData::Network { status: status.ok(), wifi: wifi.unwrap_or_default() })
                }
            },
            SystemArea::Vpn => match system.unavailable("vpn") {
                Some(why) => unsupported(why),
                None => {
                    let vpns = system.vpns().await;
                    AreaState::Ready(AreaData::Vpn { finds_vpns: true, can_import: false, vpns })
                }
            },
            SystemArea::Bluetooth => match system.bluetooth_unavailable().await {
                Some(why) => unsupported(why),
                None => {
                    let (powered, devices) = futures::join!(system.powered(), system.devices());
                    AreaState::Ready(AreaData::Bluetooth {
                        powered: powered.unwrap_or(false),
                        devices: devices.unwrap_or_default(),
                    })
                }
            },
            SystemArea::Audio => match system.audio_unavailable().await {
                Some(why) => unsupported(why),
                None => {
                    let (outputs, level) = futures::join!(system.outputs(), system.level());
                    AreaState::Ready(AreaData::Audio { outputs: outputs.unwrap_or_default(), level: level.ok() })
                }
            },
            SystemArea::Power => {
                let (refused, battery) = futures::join!(system.power_unavailable(), system.battery());
                AreaState::Ready(AreaData::Power { battery: battery.ok().flatten(), refused })
            }
            SystemArea::OsUpdates => AreaState::Ready(AreaData::OsUpdates { update: system.os_update().await.ok() }),
            SystemArea::OsInstall => AreaState::Ready(AreaData::OsInstall { disks: system.disks().await? }),
            SystemArea::Drives => match system.drives_unavailable().await {
                Some(why) => unsupported(why),
                None => AreaState::Ready(AreaData::Drives { drives: system.drives().await? }),
            },
        })
    }

    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        use AreaAction as A;
        let system = &self.system;
        match action {
            A::ScanWifi => system.wifi(true).await.map(|_| ()),
            A::JoinWifi { ssid, password } => system.join(&ssid, password.as_deref(), job).await,
            A::ForgetWifi { ssid } => system.forget(&ssid).await,
            A::ImportVpn { .. } | A::ConnectVpn { .. } | A::DisconnectVpn { .. } => {
                Err(PluginError::Unsupported("the fake can't change a VPN".into()))
            }
            A::SetBluetooth { on } => system.set_powered(on).await,
            A::ScanBluetooth { seconds } => system.scan_bluetooth(seconds, job).await,
            A::Pair { address } => system.pair(&address, job).await,
            A::ConnectDevice { address } => system.connect_device(&address).await,
            A::DisconnectDevice { address } => system.disconnect_device(&address).await,
            A::RemoveDevice { address } => system.remove_device(&address).await,
            A::SetOutput { name } => system.set_output(&name).await,
            A::SetVolume { volume } => system.set_volume(volume).await,
            A::SetMuted { muted } => system.set_muted(muted).await,
            A::Restart => system.restart().await,
            A::ShutDown => system.shut_down().await,
            A::CheckOsUpdate => system.check_now().await,
            A::InstallOs { disk } => system.install(&disk, job).await,
            A::ChoosePartitions => system.choose_partitions().await,
            A::Mount { device } => system.mount(&device, job).await,
            A::Unmount { device } => system.unmount(&device, job).await,
            A::Eject { disk } => system.eject(&disk, job).await,
        }
    }
}

//! Windows: the battery (`GetSystemPowerStatus`), restart and shut down (`shutdown`), and the
//! drives (the volumes with a letter, and Configuration Manager's eject), the sound (Core
//! Audio), the network (WinRT's connection profile, and the WLAN API for Wi-Fi), and
//! Bluetooth (WinRT's radios and devices). VPNs say why they can't work there.
//!
//! Wi-Fi goes through the WLAN API, not `netsh`: `netsh` prints in the language Windows is set
//! to, so its output can't be read the same way on every PC.
//!
//! Windows' own calls block: each runs on a blocking thread with COM set up for it, for at
//! most a time limit ([`blocking`]). What Windows' calls return becomes the areas' data in
//! `winmap.rs`, which the tests run on every OS.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, AudioLevel, AudioOutput, BluetoothDevice, Connection, NetworkStatus,
    PluginError, PluginResult, SystemArea, SystemDrive, SystemJob, WifiNetwork,
};
use windows::Devices::Bluetooth::{
    BluetoothConnectionStatus, BluetoothDevice as WinBluetoothDevice, BluetoothLEDevice,
};
use windows::Devices::Enumeration::{
    DeviceInformation, DeviceInformationCustomPairing, DeviceInformationUpdate, DevicePairingKinds,
    DevicePairingRequestedEventArgs, DeviceUnpairingResultStatus, DeviceWatcher,
};
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Foundation::TypedEventHandler;
use windows::Networking::Connectivity::NetworkInformation;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVCAP_REMOVABLE, CM_DRP_CAPABILITIES, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_DevNode_Registry_PropertyW,
    CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_PropertyW,
    CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CM_Request_Device_EjectW, CR_SUCCESS,
    PNP_VetoTypeUnknown,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::NetworkManagement::WiFi::{
    DOT11_SSID, WLAN_AVAILABLE_NETWORK, WLAN_AVAILABLE_NETWORK_CONNECTED, WLAN_AVAILABLE_NETWORK_HAS_PROFILE,
    WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST,
    WlanCloseHandle, WlanConnect, WlanDeleteProfile, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
    WlanOpenHandle, WlanQueryInterface, WlanScan, WlanSetProfile, dot11_BSS_type_infrastructure,
    wlan_connection_mode_profile, wlan_interface_state_connected, wlan_intf_opcode_current_connection,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetDriveTypeW,
    GetLogicalDriveStringsW, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SetThreadErrorMode, THREAD_ERROR_MODE};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::System::WindowsProgramming::{
    DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
};
use windows::core::{GUID, HRESULT, HSTRING, PCWSTR, Ref};

use crate::tools::{self, Runner, Tool};
use crate::winmap;

/// Windows' own restart and shut down.
const SHUTDOWN: Tool = Tool::new("shutdown", "Windows' shutdown");

/// Each area Windows manages, and the tools it runs.
pub const AREAS: &[(SystemArea, &[Tool])] =
    &[(SystemArea::Vpn, &[]), (SystemArea::Audio, &[]), (SystemArea::Power, &[SHUTDOWN]), (SystemArea::Drives, &[])];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    match area {
        SystemArea::Vpn => Arc::new(Vpns),
        SystemArea::Drives => Arc::new(Drives),
        SystemArea::Audio => Arc::new(Audio),
        SystemArea::Network => Arc::new(Network),
        SystemArea::Bluetooth => Arc::new(Bluetooth::default()),
        _ => Arc::new(Power(tools)),
    }
}

/// How long a read or a quick change through Windows' own calls may take.
const QUICK: Duration = crate::tools::QUICK;

/// COM, set up on this thread for as long as the guard lives: the multithreaded kind, so a wait on a
/// WinRT call doesn't block a window's thread.
struct Com {
    /// COM was set up here, and is closed again on drop. Not when the thread had another kind.
    owned: bool,
}

impl Com {
    fn init() -> Self {
        #[allow(unsafe_code)] // COM's own setup, closed in drop when it worked.
        // SAFETY: no reserved pointer; the guard closes it on this same thread.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Com { owned: result.is_ok() }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            #[allow(unsafe_code)] // Closes the setup `init` made on this thread.
            // SAFETY: `CoInitializeEx` worked on this thread, and each such call is closed once.
            unsafe {
                CoUninitialize()
            };
        }
    }
}

/// Runs `work` on a blocking thread with COM, for at most `limit`. At the limit the area stops
/// waiting, but the thread runs on until Windows answers: a Windows call can't be stopped.
async fn blocking<T, F>(limit: Duration, work: F) -> PluginResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> PluginResult<T> + Send + 'static,
{
    let task = tokio::task::spawn_blocking(move || {
        let _com = Com::init();
        work()
    });
    match tokio::time::timeout(limit, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(PluginError::Other(format!("Windows' call stopped: {e}"))),
        Err(_) => Err(PluginError::Other("Windows took too long to answer".into())),
    }
}

/// A failed Windows call, in plain words: what failed, and Windows' own message.
fn failed(what: &'static str) -> impl Fn(windows::core::Error) -> PluginError {
    move |e| PluginError::Other(format!("{what}: {}", e.message()))
}

struct Power(Arc<Runner>);

impl Power {
    async fn battery(&self) -> PluginResult<Option<launcher_api::Battery>> {
        blocking(QUICK, || {
            let mut status = SYSTEM_POWER_STATUS::default();
            #[allow(unsafe_code)] // One call that fills a struct this closure owns.
            // SAFETY: `status` is a writable SYSTEM_POWER_STATUS.
            unsafe { GetSystemPowerStatus(&mut status) }.map_err(failed("The battery wasn't read"))?;
            Ok(winmap::battery(status.BatteryFlag, status.BatteryLifePercent))
        })
        .await
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Power {
    async fn state(&self) -> PluginResult<AreaState> {
        let (refused, battery) = tokio::join!(self.0.missing(&[SHUTDOWN]), self.battery());
        Ok(AreaState::Ready(AreaData::Power { battery: battery.ok().flatten(), refused }))
    }

    async fn act(&self, action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        let how = match action {
            AreaAction::Restart => "/r",
            AreaAction::ShutDown => "/s",
            other => return Err(PluginError::Unsupported(format!("the power area can't run {other:?}"))),
        };
        self.0.act(SHUTDOWN, &[how, "/t", "0"]).await.map(|_| ())
    }
}

/// `text` as a wide string with its nul, for Windows' calls.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The wide string in `buffer`, up to its nul.
fn unwide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// A device Windows opened, closed on drop.
struct Device(HANDLE);

impl Device {
    /// The device at `path` (`\\.\E:`, or a disk's interface), opened only to ask it things:
    /// no read or write, so the player needn't be the administrator. `None` when it isn't there.
    fn open(path: &str) -> Option<Device> {
        let path = wide(path);
        #[allow(unsafe_code)] // Opens a handle that `Device` closes.
        // SAFETY: `path` ends with a nul and lives through the call.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };
        handle.ok().map(Device)
    }

    /// Sends `code` with `input`, and fills `output`: how many bytes it wrote.
    fn ask<I>(&self, code: u32, input: Option<&I>, output: &mut [u8]) -> Option<usize> {
        let mut wrote = 0u32;
        let (input, size) = match input {
            Some(input) => (Some(std::ptr::from_ref(input).cast()), std::mem::size_of::<I>() as u32),
            None => (None, 0),
        };
        #[allow(unsafe_code)] // One control call on a handle this owns.
        // SAFETY: the input is `size` bytes long and the output `output.len()`; both live
        // through the call, which isn't overlapped.
        let asked = unsafe {
            DeviceIoControl(
                self.0,
                code,
                input,
                size,
                Some(output.as_mut_ptr().cast()),
                output.len() as u32,
                Some(&mut wrote),
                None,
            )
        };
        asked.ok().map(|()| wrote as usize)
    }

    /// The number of the disk the device is on.
    fn disk(&self) -> Option<u32> {
        let mut out = [0u8; std::mem::size_of::<STORAGE_DEVICE_NUMBER>()];
        self.ask::<()>(IOCTL_STORAGE_GET_DEVICE_NUMBER, None, &mut out)?;
        // DeviceType, then DeviceNumber: the second u32.
        Some(u32::from_le_bytes(out[4..8].try_into().ok()?))
    }

    /// Whether the disk is on USB, and its model.
    fn storage(&self) -> Option<winmap::StorageDevice> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut out = vec![0u8; 1024];
        let wrote = self.ask(IOCTL_STORAGE_QUERY_PROPERTY, Some(&query), &mut out)?;
        out.truncate(wrote);
        winmap::storage_device(&out)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        #[allow(unsafe_code)] // Closes the handle `open` made.
        // SAFETY: the handle came from CreateFileW and is closed once, here.
        let _closed = unsafe { CloseHandle(self.0) };
    }
}

/// How long an eject may take: Windows asks each program that holds the drive to let it go, and
/// writes what is left to it.
const EJECT_TIME: Duration = Duration::from_secs(55);

/// The volumes with a letter. Windows mounts each by itself when it is plugged in.
struct Drives;

impl Drives {
    /// Each volume with a letter that has a file system: a disk, a USB stick, a card, a disc,
    /// and a network drive.
    fn list() -> Vec<SystemDrive> {
        // A card reader or a disc drive with nothing in it would have Windows show a dialog.
        let mut was = THREAD_ERROR_MODE(0);
        #[allow(unsafe_code)] // Changes this thread's error mode; set back below.
        // SAFETY: `was` is writable.
        let quiet = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut was)) }.is_ok();
        let drives = Self::volumes();
        if quiet {
            #[allow(unsafe_code)] // Sets this thread's error mode back.
            // SAFETY: `was` is the mode the thread had.
            let _set = unsafe { SetThreadErrorMode(was, None) };
        }
        drives
    }

    fn volumes() -> Vec<SystemDrive> {
        #[allow(unsafe_code)] // Asks how long the list of roots is.
        // SAFETY: no buffer: the call only says the length it needs.
        let needed = unsafe { GetLogicalDriveStringsW(None) };
        let mut roots = vec![0u16; needed as usize + 1];
        #[allow(unsafe_code)] // Fills `roots`, whose length the call is given.
        // SAFETY: `roots` is a writable buffer of its own length.
        let wrote = unsafe { GetLogicalDriveStringsW(Some(&mut roots)) };
        roots.truncate(wrote as usize);
        winmap::multi_sz(&roots).into_iter().filter_map(|root| Self::volume(&root)).collect()
    }

    /// The volume at `root` (`E:\`); `None` when it isn't one to list, or holds no medium.
    fn volume(root: &str) -> Option<SystemDrive> {
        let at = wide(root);
        #[allow(unsafe_code)] // One call on a nul-ended path.
        // SAFETY: `at` ends with a nul and lives through the call.
        let kind = unsafe { GetDriveTypeW(PCWSTR(at.as_ptr())) };
        if kind == DRIVE_REMOTE {
            // Only its letter: a share's name and space can hold the list for as long as the
            // network takes. The core reads its space with its own time limit.
            let label = winmap::drive_label("", root);
            return Some(SystemDrive {
                device: root.to_owned(),
                label,
                mount: Some(root.into()),
                ..SystemDrive::default()
            });
        }
        if ![DRIVE_FIXED, DRIVE_REMOVABLE, DRIVE_CDROM, DRIVE_RAMDISK].contains(&kind) {
            return None;
        }
        let (mut label, mut filesystem) = ([0u16; 261], [0u16; 261]);
        #[allow(unsafe_code)] // Fills two buffers whose lengths the call is given.
        // SAFETY: `at` ends with a nul; both buffers are writable and live through the call.
        let read = unsafe {
            GetVolumeInformationW(PCWSTR(at.as_ptr()), Some(&mut label), None, None, None, Some(&mut filesystem))
        };
        // A card reader or a disc drive with nothing in it.
        read.ok()?;
        let (mut free, mut size) = (0u64, 0u64);
        #[allow(unsafe_code)] // Fills two numbers this function owns.
        // SAFETY: `at` ends with a nul; `free` and `size` are writable.
        let space = unsafe { GetDiskFreeSpaceExW(PCWSTR(at.as_ptr()), Some(&mut free), Some(&mut size), None) };
        let mut name = [0u16; 64];
        #[allow(unsafe_code)] // Fills a buffer whose length the call is given.
        // SAFETY: `at` ends with a nul (and a backslash, as the call needs); `name` is writable.
        let named = unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(at.as_ptr()), &mut name) };
        let device = Device::open(&format!(r"\\.\{}", root.trim_end_matches('\\')));
        let disk = device.as_ref().and_then(Device::disk);
        let storage = device.as_ref().and_then(Device::storage).unwrap_or_default();
        Some(SystemDrive {
            device: root.to_owned(),
            disk: disk.map(winmap::disk_name).unwrap_or_default(),
            uuid: named.ok().map(|()| winmap::volume_id(&unwide(&name))).unwrap_or_default(),
            label: winmap::drive_label(&unwide(&label), root),
            model: storage.model,
            size: if space.is_ok() { size } else { 0 },
            filesystem: unwide(&filesystem).to_ascii_lowercase(),
            // A disc comes out by its own button.
            usb: kind != DRIVE_CDROM && (kind == DRIVE_REMOVABLE || storage.usb),
            mount: Some(root.into()),
            free: space.is_ok().then_some(free),
            // Windows keeps every volume mounted: none is the launcher's to unmount.
            system: false,
        })
    }

    /// The number of the disk Windows runs from.
    fn windows_disk() -> Option<u32> {
        let mut folder = [0u16; 261];
        #[allow(unsafe_code)] // Fills a buffer whose length the call is given.
        // SAFETY: `folder` is writable.
        let wrote = unsafe { GetWindowsDirectoryW(Some(&mut folder)) } as usize;
        let folder = String::from_utf16_lossy(folder.get(..wrote)?);
        let letter = folder.get(..2)?;
        Device::open(&format!(r"\\.\{letter}"))?.disk()
    }

    /// Ejects disk `number`: Windows asks every program that holds it to let it go, writes what
    /// is left to it, and powers it off, as "Safely remove" does.
    fn eject(number: u32) -> PluginResult<()> {
        if Self::windows_disk() == Some(number) {
            return Err(PluginError::InvalidInput("the system runs from this drive".into()));
        }
        let instance =
            Self::disk_instance(number).ok_or_else(|| PluginError::NotFound("the drive isn't plugged in".into()))?;
        let id = wide(&instance);
        let mut node = 0u32;
        #[allow(unsafe_code)] // Finds the device by the id Windows gave.
        // SAFETY: `id` ends with a nul; `node` is writable.
        let found = unsafe { CM_Locate_DevNodeW(&mut node, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) };
        if found != CR_SUCCESS {
            return Err(PluginError::NotFound("the drive isn't plugged in".into()));
        }
        let node =
            Self::removable(node).ok_or_else(|| PluginError::Unsupported(winmap::eject_refused(8).to_owned()))?;
        let (mut veto, mut vetoed_by) = (PNP_VetoTypeUnknown, [0u16; 261]);
        #[allow(unsafe_code)] // Asks Windows to eject the device it found.
        // SAFETY: `veto` and `vetoed_by` are writable and live through the call.
        let ejected = unsafe { CM_Request_Device_EjectW(node, Some(&mut veto), Some(&mut vetoed_by), 0) };
        if ejected != CR_SUCCESS || veto != PNP_VetoTypeUnknown {
            return Err(PluginError::Other(winmap::eject_refused(veto.0).into()));
        }
        Ok(())
    }

    /// The device that comes out with the disk at `node`: the disk itself, or the USB device it
    /// is on, a few steps up. The first that Windows marks removable; `None` when none is, as
    /// an eject of anything else could stop a controller inside the PC.
    fn removable(mut node: u32) -> Option<u32> {
        for _ in 0..4 {
            let (mut capabilities, mut size) = (0u32, std::mem::size_of::<u32>() as u32);
            #[allow(unsafe_code)] // Reads one number about a device Windows found.
            // SAFETY: `capabilities` is `size` bytes long and writable.
            let read = unsafe {
                CM_Get_DevNode_Registry_PropertyW(
                    node,
                    CM_DRP_CAPABILITIES,
                    None,
                    Some(std::ptr::from_mut(&mut capabilities).cast()),
                    &mut size,
                    0,
                )
            };
            if read == CR_SUCCESS && capabilities & CM_DEVCAP_REMOVABLE.0 != 0 {
                return Some(node);
            }
            let mut parent = 0u32;
            #[allow(unsafe_code)] // Finds the device this one is on.
            // SAFETY: `parent` is writable.
            if unsafe { CM_Get_Parent(&mut parent, node, 0) } != CR_SUCCESS {
                return None;
            }
            node = parent;
        }
        None
    }

    /// The device instance of disk `number`, from the disks Windows has now.
    fn disk_instance(number: u32) -> Option<String> {
        let mut length = 0u32;
        #[allow(unsafe_code)] // Asks how long the list of disks is.
        // SAFETY: `length` is writable; the GUID is a constant.
        let sized = unsafe {
            CM_Get_Device_Interface_List_SizeW(
                &mut length,
                &GUID_DEVINTERFACE_DISK,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if sized != CR_SUCCESS {
            return None;
        }
        let mut list = vec![0u16; length as usize];
        #[allow(unsafe_code)] // Fills `list`, whose length the call is given.
        // SAFETY: `list` is writable and as long as the call said.
        let listed = unsafe {
            CM_Get_Device_Interface_ListW(
                &GUID_DEVINTERFACE_DISK,
                PCWSTR::null(),
                &mut list,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if listed != CR_SUCCESS {
            return None;
        }
        let path =
            winmap::multi_sz(&list).into_iter().find(|p| Device::open(p).and_then(|d| d.disk()) == Some(number))?;
        let path = wide(&path);
        let (mut kind, mut id, mut size) = (DEVPROPTYPE(0), [0u16; 512], 1024u32);
        #[allow(unsafe_code)] // Reads one property into a buffer whose size the call is given.
        // SAFETY: `path` ends with a nul; `id` is `size` bytes long and writable.
        let read = unsafe {
            CM_Get_Device_Interface_PropertyW(
                PCWSTR(path.as_ptr()),
                &DEVPKEY_Device_InstanceId,
                &mut kind,
                Some(id.as_mut_ptr().cast()),
                &mut size,
                0,
            )
        };
        (read == CR_SUCCESS).then(|| unwide(&id))
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Drives {
    async fn state(&self) -> PluginResult<AreaState> {
        let drives = blocking(QUICK, || Ok(Drives::list())).await?;
        Ok(AreaState::Ready(AreaData::Drives { drives }))
    }

    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        match action {
            AreaAction::Mount { .. } => {
                Err(PluginError::Unsupported("Windows mounts a drive by itself when it is plugged in".into()))
            }
            AreaAction::Unmount { .. } => {
                Err(PluginError::Unsupported("Windows keeps its drives mounted: eject the drive instead".into()))
            }
            AreaAction::Eject { disk } => {
                let number = winmap::disk_number(&disk)
                    .ok_or_else(|| PluginError::InvalidInput(format!("{disk} isn't a disk Windows named")))?;
                job.progress(None, "Writing what is left to the drive");
                tools::job(job, async move {
                    blocking(EJECT_TIME, move || Drives::eject(number)).await.map_err(tools::reason)?;
                    Ok("You can take it out now".into())
                });
                Ok(())
            }
            other => Err(PluginError::Unsupported(format!("the drives area can't run {other:?}"))),
        }
    }
}

/// The sound, through Core Audio: the outputs that are plugged in, and the default one's level.
/// Windows has no public call to change the default output.
struct Audio;

impl Audio {
    fn devices() -> PluginResult<IMMDeviceEnumerator> {
        #[allow(unsafe_code)] // Makes Core Audio's own object, on a thread with COM.
        // SAFETY: `blocking` set up COM on this thread.
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(failed("Windows' sound isn't there"))
    }

    /// The output Windows plays to; `None` when none is plugged in.
    fn default_output(devices: &IMMDeviceEnumerator) -> Option<IMMDevice> {
        #[allow(unsafe_code)] // One call on a Core Audio object.
        // SAFETY: `devices` is a live Core Audio object on this COM thread.
        unsafe { devices.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()
    }

    /// Windows' id for `device`.
    fn id(device: &IMMDevice) -> Option<String> {
        #[allow(unsafe_code)] // Reads the id, then frees the string Windows made for it.
        // SAFETY: `device` is live; the string Windows returns is read once, then freed once.
        unsafe {
            let id = device.GetId().ok()?;
            let text = id.to_string().ok();
            CoTaskMemFree(Some(id.0.cast_const().cast()));
            text
        }
    }

    /// The name Windows shows for `device`, such as "Speakers (Realtek(R) Audio)".
    fn name(device: &IMMDevice) -> Option<String> {
        #[allow(unsafe_code)] // Reads one property of a live Core Audio object.
        // SAFETY: `device` is live; the key is a constant.
        let value = unsafe { device.OpenPropertyStore(STGM_READ).and_then(|s| s.GetValue(&PKEY_Device_FriendlyName)) };
        value.ok().map(|v| v.to_string()).filter(|n| !n.is_empty())
    }

    fn outputs(devices: &IMMDeviceEnumerator, current: Option<&str>) -> PluginResult<Vec<AudioOutput>> {
        #[allow(unsafe_code)] // Lists the outputs that are plugged in.
        // SAFETY: `devices` is a live Core Audio object on this COM thread.
        let list = unsafe { devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(failed("The sound outputs weren't read"))?;
        #[allow(unsafe_code)] // Reads the length of the list it made.
        // SAFETY: `list` is live.
        let count = unsafe { list.GetCount() }.map_err(failed("The sound outputs weren't read"))?;
        let mut outputs = Vec::new();
        for index in 0..count {
            #[allow(unsafe_code)] // Reads one item of the list, below its length.
            // SAFETY: `list` is live and `index` is below its count.
            let Ok(device) = (unsafe { list.Item(index) }) else { continue };
            let Some(id) = Self::id(&device) else { continue };
            let description = Self::name(&device).unwrap_or_else(|| "Sound output".into());
            outputs.push(AudioOutput { current: current == Some(id.as_str()), name: id, description });
        }
        Ok(outputs)
    }

    /// The volume control of `device`.
    fn volume(device: &IMMDevice) -> PluginResult<IAudioEndpointVolume> {
        #[allow(unsafe_code)] // Gets the volume control of a live Core Audio object.
        // SAFETY: `device` is live, on this COM thread; no activation values.
        unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(failed("The volume wasn't read"))
    }

    fn level(device: &IMMDevice) -> PluginResult<AudioLevel> {
        let volume = Self::volume(device)?;
        #[allow(unsafe_code)] // Two reads of a live volume control.
        // SAFETY: `volume` is live, on this COM thread.
        let (scalar, muted) = unsafe { (volume.GetMasterVolumeLevelScalar(), volume.GetMute()) };
        let read = failed("The volume wasn't read");
        Ok(AudioLevel {
            volume: winmap::volume_percent(scalar.map_err(&read)?),
            muted: muted.map_err(&read)?.as_bool(),
        })
    }

    /// Changes the default output's volume control with `change`.
    async fn change(
        change: impl FnOnce(&IAudioEndpointVolume) -> windows::core::Result<()> + Send + 'static,
    ) -> PluginResult<()> {
        blocking(QUICK, move || {
            let devices = Self::devices()?;
            let device = Self::default_output(&devices)
                .ok_or_else(|| PluginError::NotFound("No sound output is plugged in".into()))?;
            change(&Self::volume(&device)?).map_err(failed("The volume didn't change"))
        })
        .await
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Audio {
    async fn state(&self) -> PluginResult<AreaState> {
        let (outputs, level) = blocking(QUICK, || {
            let devices = Self::devices()?;
            let current = Self::default_output(&devices);
            let id = current.as_ref().and_then(Self::id);
            let outputs = Self::outputs(&devices, id.as_deref())?;
            Ok((outputs, current.as_ref().and_then(|d| Self::level(d).ok())))
        })
        .await?;
        Ok(AreaState::Ready(AreaData::Audio { outputs, level }))
    }

    async fn act(&self, action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        match action {
            AreaAction::SetVolume { volume } => {
                let scalar = winmap::volume_scalar(volume);
                #[allow(unsafe_code)] // One change of a live volume control.
                // SAFETY: the control is live, on the COM thread `change` runs it on; no event id.
                Self::change(move |control| unsafe { control.SetMasterVolumeLevelScalar(scalar, std::ptr::null()) })
                    .await
            }
            AreaAction::SetMuted { muted } => {
                #[allow(unsafe_code)] // One change of a live volume control.
                // SAFETY: the control is live, on the COM thread `change` runs it on; no event id.
                Self::change(move |control| unsafe { control.SetMute(muted, std::ptr::null()) }).await
            }
            AreaAction::SetOutput { .. } => Err(PluginError::Unsupported(
                "Windows lets no program change the sound output: change it in Windows' sound settings".into(),
            )),
            other => Err(PluginError::Unsupported(format!("the sound area can't run {other:?}"))),
        }
    }
}

/// Why Windows didn't list the Wi-Fi networks when it refused: since its fall 2024 release,
/// Windows 11 shows them to a program only with the player's consent to location.
const WIFI_REFUSED: &str =
    "Windows didn't let the launcher see the Wi-Fi networks: allow location in Settings, Privacy & security, Location";

/// How long joining a network may take: the WLAN API's wait, and some more.
const JOIN_TIME: Duration = Duration::from_secs(60);
const JOIN_WAIT: Duration = Duration::from_secs(45);
/// How long a Wi-Fi scan takes: Windows asks drivers to end one in 4 seconds.
const SCAN_TIME: Duration = Duration::from_secs(4);

/// A WLAN API error code as a plain error: `what` failed, and why.
fn wlan(code: u32, what: &str) -> PluginResult<()> {
    const ACCESS_DENIED: u32 = 5;
    match code {
        0 => Ok(()),
        ACCESS_DENIED => Err(PluginError::PermissionDenied(WIFI_REFUSED.into())),
        code => Err(PluginError::Other(format!("{what}: {}", HRESULT::from_win32(code).message()))),
    }
}

/// Memory the WLAN API made, freed on drop.
struct WlanMemory<T>(*mut T);

impl<T> Drop for WlanMemory<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            #[allow(unsafe_code)] // Frees memory the WLAN API made.
            // SAFETY: the pointer came from a WLAN API call and is freed once, here.
            unsafe {
                WlanFreeMemory(self.0.cast_const().cast())
            };
        }
    }
}

/// The SSID's bytes.
fn ssid(ssid: &DOT11_SSID) -> &[u8] {
    &ssid.ucSSID[..(ssid.uSSIDLength as usize).min(ssid.ucSSID.len())]
}

/// The WLAN API, open on its first Wi-Fi adapter. Closed on drop.
struct Wlan {
    handle: HANDLE,
    adapter: GUID,
}

impl Wlan {
    /// `None` when the PC has no Wi-Fi adapter, or the WLAN service doesn't run.
    fn open() -> Option<Wlan> {
        let (mut version, mut handle) = (0u32, HANDLE::default());
        #[allow(unsafe_code)] // Opens a WLAN handle that `Wlan` closes.
        // SAFETY: `version` and `handle` are writable.
        let code = unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) };
        if code != 0 {
            return None;
        }
        let mut list = WlanMemory::<WLAN_INTERFACE_INFO_LIST>(std::ptr::null_mut());
        #[allow(unsafe_code)] // Lists the adapters into memory `list` frees.
        // SAFETY: `handle` is open; `list.0` is writable.
        let code = unsafe { WlanEnumInterfaces(handle, None, &mut list.0) };
        #[allow(unsafe_code)] // Reads the list the call made.
        // SAFETY: on success the list holds `dwNumberOfItems` adapters, read here before it is freed.
        let adapter = (code == 0 && !list.0.is_null())
            .then(|| unsafe { ((*list.0).dwNumberOfItems > 0).then(|| (*list.0).InterfaceInfo[0].InterfaceGuid) })
            .flatten();
        let wlan = Wlan { handle, adapter: adapter.unwrap_or_default() };
        adapter.map(|_| wlan)
    }

    /// The networks in reach, as the adapter's last scan found them.
    fn networks(&self) -> PluginResult<Vec<WLAN_AVAILABLE_NETWORK>> {
        let mut list = WlanMemory::<WLAN_AVAILABLE_NETWORK_LIST>(std::ptr::null_mut());
        #[allow(unsafe_code)] // Lists the networks into memory `list` frees.
        // SAFETY: the handle is open; `list.0` is writable.
        let code = unsafe { WlanGetAvailableNetworkList(self.handle, &self.adapter, 0, None, &mut list.0) };
        wlan(code, "The Wi-Fi networks weren't read")?;
        if list.0.is_null() {
            return Ok(Vec::new());
        }
        #[allow(unsafe_code)] // Copies the list the call made before it is freed.
        // SAFETY: the list holds `dwNumberOfItems` networks after its header.
        let networks = unsafe {
            std::slice::from_raw_parts((*list.0).Network.as_ptr(), (*list.0).dwNumberOfItems as usize).to_vec()
        };
        Ok(networks)
    }

    /// The network the adapter is on now: its SSID and signal. `None` while it is on none.
    fn current(&self) -> Option<(Vec<u8>, u8)> {
        let (mut size, mut data) = (0u32, WlanMemory::<WLAN_CONNECTION_ATTRIBUTES>(std::ptr::null_mut()));
        #[allow(unsafe_code)] // Reads the connection into memory `data` frees.
        // SAFETY: the handle is open; `size` and `data.0` are writable.
        let code = unsafe {
            WlanQueryInterface(
                self.handle,
                &self.adapter,
                wlan_intf_opcode_current_connection,
                None,
                &mut size,
                std::ptr::from_mut(&mut data.0).cast(),
                None,
            )
        };
        if code != 0 || data.0.is_null() {
            return None;
        }
        #[allow(unsafe_code)] // Reads the connection the call made before it is freed.
        // SAFETY: on success the call wrote a WLAN_CONNECTION_ATTRIBUTES there.
        let connection = unsafe { &*data.0 };
        let on = connection.isState == wlan_interface_state_connected;
        let association = &connection.wlanAssociationAttributes;
        on.then(|| (ssid(&association.dot11Ssid).to_vec(), association.wlanSignalQuality.min(100) as u8))
    }

    fn scan(&self) -> PluginResult<()> {
        #[allow(unsafe_code)] // Asks the adapter to look for networks again.
        // SAFETY: the handle is open; no SSID or extra data.
        let code = unsafe { WlanScan(self.handle, &self.adapter, None, None, None) };
        wlan(code, "Windows didn't look for Wi-Fi networks")
    }

    /// The saved profile for `wanted`, as Windows names it; `None` when it has none.
    fn profile(&self, wanted: &str) -> PluginResult<Option<String>> {
        Ok(self.networks()?.iter().find_map(|n| {
            let saved = n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0;
            (saved && String::from_utf8_lossy(ssid(&n.dot11Ssid)) == wanted).then(|| unwide(&n.strProfileName))
        }))
    }

    fn set_profile(&self, xml: &str) -> PluginResult<()> {
        let (xml, mut reason) = (wide(xml), 0u32);
        #[allow(unsafe_code)] // Saves a profile from a nul-ended XML text.
        // SAFETY: the handle is open; `xml` ends with a nul; `reason` is writable.
        let code = unsafe {
            WlanSetProfile(self.handle, &self.adapter, 0, PCWSTR(xml.as_ptr()), PCWSTR::null(), true, None, &mut reason)
        };
        wlan(code, "Windows didn't keep the network's settings")
    }

    fn delete_profile(&self, name: &str) -> PluginResult<()> {
        let name = wide(name);
        #[allow(unsafe_code)] // Deletes a profile by its nul-ended name.
        // SAFETY: the handle is open; `name` ends with a nul.
        let code = unsafe { WlanDeleteProfile(self.handle, &self.adapter, PCWSTR(name.as_ptr()), None) };
        wlan(code, "Windows didn't forget the network")
    }

    fn connect(&self, profile: &str) -> PluginResult<()> {
        let profile = wide(profile);
        let parameters = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(profile.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: 0,
        };
        #[allow(unsafe_code)] // Starts joining with a saved profile.
        // SAFETY: the handle is open; `parameters` and the name it points to live through the call.
        let code = unsafe { WlanConnect(self.handle, &self.adapter, &parameters, None) };
        wlan(code, "Windows didn't start joining the network")
    }

    /// Joins `wanted`: with its saved profile, or with a new one made from `password`. Waits
    /// until the adapter is on it, `cancelled` says so, or [`JOIN_WAIT`] passes. A new profile
    /// that didn't join is deleted, so a wrong password isn't kept.
    fn join(&self, wanted: &str, password: Option<&str>, cancelled: &dyn Fn() -> bool) -> PluginResult<()> {
        let networks = self.networks()?;
        let named = |n: &&WLAN_AVAILABLE_NETWORK| String::from_utf8_lossy(ssid(&n.dot11Ssid)) == wanted;
        let network =
            networks.iter().find(named).ok_or_else(|| PluginError::NotFound(format!("{wanted} isn't in reach")))?;
        let saved = self.profile(wanted)?;
        let (profile, made) = match (&saved, password.filter(|p| !p.is_empty())) {
            (Some(saved), None) => (saved.clone(), false),
            (_, password) => {
                let security = winmap::profile_security(
                    network.dot11DefaultAuthAlgorithm.0,
                    network.dot11DefaultCipherAlgorithm.0,
                )
                .ok_or_else(|| {
                    PluginError::Unsupported(format!(
                        "{wanted} signs in with an account: join it in Windows' own settings"
                    ))
                })?;
                if winmap::needs_password(security) && password.is_none() {
                    return Err(PluginError::InvalidInput(format!("{wanted} needs its password")));
                }
                self.set_profile(&winmap::wifi_profile(wanted, security, password))?;
                (wanted.to_owned(), saved.is_none())
            }
        };
        let joined = self.connect(&profile).and_then(|()| self.wait_on(wanted, cancelled));
        if joined.is_err() && made {
            let _kept = self.delete_profile(&profile);
        }
        joined
    }

    fn wait_on(&self, wanted: &str, cancelled: &dyn Fn() -> bool) -> PluginResult<()> {
        let started = Instant::now();
        while started.elapsed() < JOIN_WAIT {
            if cancelled() {
                return Err(PluginError::Other("Cancelled".into()));
            }
            if self.current().is_some_and(|(on, _)| String::from_utf8_lossy(&on) == wanted) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(PluginError::Other(format!("{wanted} didn't connect: check the password")))
    }
}

impl Drop for Wlan {
    fn drop(&mut self) {
        #[allow(unsafe_code)] // Closes the handle `open` made.
        // SAFETY: the handle came from WlanOpenHandle and is closed once, here.
        unsafe {
            WlanCloseHandle(self.handle, None)
        };
    }
}

/// The network: how the PC is online (WinRT's internet connection profile), and Wi-Fi.
struct Network;

impl Network {
    /// How the PC is online. The Wi-Fi network's name and signal come from `wifi`.
    fn status(wifi: Option<&Wlan>) -> NetworkStatus {
        let profile = NetworkInformation::GetInternetConnectionProfile().ok();
        let Some(profile) = profile else { return NetworkStatus::default() };
        if profile.IsWlanConnectionProfile().unwrap_or(false) {
            let current = wifi.and_then(Wlan::current);
            let name = current.as_ref().map(|(ssid, _)| String::from_utf8_lossy(ssid).into_owned());
            return NetworkStatus {
                connection: Connection::Wifi,
                name: name.or_else(|| profile.ProfileName().ok().map(|n| n.to_string())).unwrap_or_default(),
                signal: current.map(|(_, signal)| signal),
            };
        }
        let name = profile.ProfileName().map(|n| n.to_string()).unwrap_or_default();
        NetworkStatus { connection: Connection::Wired, name, signal: None }
    }

    /// The networks in reach, from what `wifi` lists.
    fn wifi(wifi: &Wlan) -> PluginResult<Vec<WifiNetwork>> {
        let networks = wifi.networks()?;
        let seen = networks.iter().map(|n| winmap::Seen {
            ssid: ssid(&n.dot11Ssid).to_vec(),
            signal: n.wlanSignalQuality.min(100) as u8,
            secured: n.bSecurityEnabled.as_bool(),
            saved: n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0,
            active: n.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
        });
        Ok(winmap::wifi(seen.collect()))
    }

    /// Runs `work` with the Wi-Fi adapter, on a blocking thread.
    async fn with_wifi<T: Send + 'static>(
        limit: Duration,
        work: impl FnOnce(&Wlan) -> PluginResult<T> + Send + 'static,
    ) -> PluginResult<T> {
        blocking(limit, move || {
            work(&Wlan::open().ok_or_else(|| PluginError::NotFound("This PC has no Wi-Fi".into()))?)
        })
        .await
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Network {
    /// The status, and the Wi-Fi networks when the PC has Wi-Fi. A list Windows refuses is
    /// empty: Scan says why.
    async fn state(&self) -> PluginResult<AreaState> {
        let (status, wifi) = blocking(QUICK, || {
            let wlan = Wlan::open();
            let wifi = wlan.as_ref().map(Self::wifi).transpose().unwrap_or_default().unwrap_or_default();
            Ok((Self::status(wlan.as_ref()), wifi))
        })
        .await?;
        Ok(AreaState::Ready(AreaData::Network { status: Some(status), wifi }))
    }

    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        match action {
            AreaAction::ScanWifi => {
                Self::with_wifi(QUICK, Wlan::scan).await?;
                tokio::time::sleep(SCAN_TIME).await;
                // Reads the list once, so a refusal shows here.
                Self::with_wifi(QUICK, Self::wifi).await.map(|_| ())
            }
            AreaAction::JoinWifi { ssid, password } => {
                job.progress(None, &format!("Joining {ssid}"));
                let handle = job.clone();
                tools::job(job, async move {
                    let cancelled = move || handle.cancelled();
                    let joined = ssid.clone();
                    let join = move |wifi: &Wlan| wifi.join(&ssid, password.as_deref(), &cancelled);
                    Self::with_wifi(JOIN_TIME, join).await.map_err(tools::reason)?;
                    Ok(format!("Connected to {joined}"))
                });
                Ok(())
            }
            AreaAction::ForgetWifi { ssid } => {
                Self::with_wifi(QUICK, move |wifi| {
                    let profile = wifi.profile(&ssid)?.unwrap_or_else(|| ssid.clone());
                    wifi.delete_profile(&profile)
                })
                .await
            }
            other => Err(PluginError::Unsupported(format!("the network area can't run {other:?}"))),
        }
    }
}

/// How long pairing may take: the device's answer, and the player putting it in pairing mode.
const PAIR_TIME: Duration = Duration::from_secs(40);

/// A device the last scan found, not paired yet.
#[derive(Clone, Debug)]
struct Found {
    /// WinRT's id for it, which pairing takes.
    id: String,
    /// It is a Bluetooth LE device, not a classic one.
    low_energy: bool,
    address: String,
    name: String,
}

/// Bluetooth, through WinRT: the radio, the paired devices, and a scan for new ones. WinRT has
/// no call to connect or disconnect a paired device: Windows connects one by itself when it is
/// turned on.
#[derive(Default)]
struct Bluetooth {
    /// What the last scan found, which the state lists and pairing takes.
    found: Arc<Mutex<Vec<Found>>>,
}

/// WinRT's selectors for Bluetooth devices, paired or not: the classic kind (most headsets),
/// then Bluetooth LE (newer controllers).
fn selectors(paired: bool) -> PluginResult<[HSTRING; 2]> {
    let read = failed("Bluetooth wasn't read");
    Ok([
        WinBluetoothDevice::GetDeviceSelectorFromPairingState(paired).map_err(&read)?,
        BluetoothLEDevice::GetDeviceSelectorFromPairingState(paired).map_err(&read)?,
    ])
}

impl Bluetooth {
    /// The Bluetooth radio; `None` when the PC has none.
    fn radio() -> PluginResult<Option<Radio>> {
        let radios = Radio::GetRadiosAsync().and_then(|r| r.join()).map_err(failed("Bluetooth wasn't read"))?;
        Ok(radios.into_iter().find(|r| r.Kind().is_ok_and(|k| k == RadioKind::Bluetooth)))
    }

    /// The paired devices, each with what WinRT knows of it.
    fn paired() -> PluginResult<Vec<(DeviceInformation, BluetoothDevice)>> {
        let mut devices = Vec::new();
        for (low_energy, selector) in selectors(true)?.iter().enumerate() {
            let list = DeviceInformation::FindAllAsyncAqsFilter(selector)
                .and_then(|l| l.join())
                .map_err(failed("The Bluetooth devices weren't read"))?;
            for index in 0..list.Size().unwrap_or(0) {
                let Ok(info) = list.GetAt(index) else { continue };
                let Ok(id) = info.Id() else { continue };
                let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
                // A device out of reach still opens: it isn't connected.
                let (address, connected) = if low_energy == 1 {
                    let device = BluetoothLEDevice::FromIdAsync(&id).and_then(|d| d.join()).ok();
                    let address = device.as_ref().and_then(|d| d.BluetoothAddress().ok());
                    (address, device.and_then(|d| d.ConnectionStatus().ok()))
                } else {
                    let device = WinBluetoothDevice::FromIdAsync(&id).and_then(|d| d.join()).ok();
                    let address = device.as_ref().and_then(|d| d.BluetoothAddress().ok());
                    (address, device.and_then(|d| d.ConnectionStatus().ok()))
                };
                let id = id.to_string();
                let Some(address) = address.map(winmap::bluetooth_address).or_else(|| winmap::address_in_id(&id))
                else {
                    continue;
                };
                let connected = connected == Some(BluetoothConnectionStatus::Connected);
                devices.push((info, BluetoothDevice { address, name, paired: true, connected }));
            }
        }
        Ok(devices)
    }

    /// Looks for devices that aren't paired for `seconds`, or until `cancelled`: what it found,
    /// named, one per address.
    fn scan(seconds: u32, cancelled: &dyn Fn() -> bool) -> PluginResult<Vec<Found>> {
        let found: Arc<Mutex<Vec<Found>>> = Arc::default();
        let start = failed("Windows didn't look for Bluetooth devices");
        let mut watchers = Vec::new();
        for (low_energy, selector) in selectors(false)?.into_iter().enumerate() {
            let low_energy = low_energy == 1;
            let watcher = DeviceInformation::CreateWatcherAqsFilter(&selector).map_err(&start)?;
            let seen = found.clone();
            let added = TypedEventHandler::new(move |_: Ref<DeviceWatcher>, info: Ref<DeviceInformation>| {
                let Some(info) = info.as_ref() else { return Ok(()) };
                let (id, name) = (info.Id()?.to_string(), info.Name()?.to_string());
                // A device with no name is a beacon or a phone nearby, not one to pair.
                if let Some(address) = winmap::address_in_id(&id).filter(|_| !name.is_empty()) {
                    let mut seen = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !seen.iter().any(|f| f.address == address) {
                        seen.push(Found { id, low_energy, address, name });
                    }
                }
                Ok(())
            });
            watcher.Added(&added).map_err(&start)?;
            // WinRT sends the updates only to a watcher that takes them.
            let updated = TypedEventHandler::new(|_: Ref<DeviceWatcher>, _: Ref<DeviceInformationUpdate>| Ok(()));
            watcher.Updated(&updated).map_err(&start)?;
            watcher.Start().map_err(&start)?;
            watchers.push(watcher);
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(u64::from(seconds)) && !cancelled() {
            std::thread::sleep(Duration::from_millis(250));
        }
        for watcher in watchers {
            let _stopped = watcher.Stop();
        }
        if cancelled() {
            return Err(PluginError::Other("Cancelled".into()));
        }
        let found = found.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        Ok(found)
    }

    /// Pairs the device WinRT knows by `id`. It accepts only a pairing that asks nothing of the
    /// player: one that asks for a PIN or a code fails, and says to use Windows' own settings.
    fn pair(device: &Found) -> PluginResult<()> {
        let id = HSTRING::from(device.id.as_str());
        // The typed device's own information: its pairing is the device's, whatever kind of id
        // the scan gave.
        let info = if device.low_energy {
            BluetoothLEDevice::FromIdAsync(&id).and_then(|d| d.join()).and_then(|d| d.DeviceInformation())
        } else {
            WinBluetoothDevice::FromIdAsync(&id).and_then(|d| d.join()).and_then(|d| d.DeviceInformation())
        };
        let info = info.map_err(failed("The device isn't in reach"))?;
        let custom = info.Pairing().and_then(|p| p.Custom()).map_err(failed("Windows didn't pair the device"))?;
        let accept = TypedEventHandler::new(
            |_: Ref<DeviceInformationCustomPairing>, asked: Ref<DevicePairingRequestedEventArgs>| {
                if let Some(asked) = asked.as_ref()
                    && asked.PairingKind()? == DevicePairingKinds::ConfirmOnly
                {
                    asked.Accept()?;
                }
                Ok(())
            },
        );
        let token = custom.PairingRequested(&accept).map_err(failed("Windows didn't pair the device"))?;
        let result = custom.PairAsync(DevicePairingKinds::ConfirmOnly).and_then(|r| r.join());
        let _removed = custom.RemovePairingRequested(token);
        let status = result.and_then(|r| r.Status()).map_err(failed("Windows didn't pair the device"))?;
        winmap::paired(status.0).map_err(|why| PluginError::Other(why.into()))
    }

    fn remove(address: &str) -> PluginResult<()> {
        let paired = Self::paired()?;
        let (info, _) = paired
            .iter()
            .find(|(_, d)| d.address.eq_ignore_ascii_case(address))
            .ok_or_else(|| PluginError::NotFound(format!("{address} isn't paired")))?;
        let forget = failed("Windows didn't forget the device");
        let status = info.Pairing().and_then(|p| p.UnpairAsync()).and_then(|u| u.join()).and_then(|r| r.Status());
        match status.map_err(&forget)? {
            DeviceUnpairingResultStatus::Unpaired | DeviceUnpairingResultStatus::AlreadyUnpaired => Ok(()),
            _ => Err(PluginError::Other("Windows didn't forget the device".into())),
        }
    }

    fn set_powered(on: bool) -> PluginResult<()> {
        let access = Radio::RequestAccessAsync().and_then(|a| a.join()).map_err(failed("Bluetooth didn't change"))?;
        if access != RadioAccessStatus::Allowed {
            return Err(PluginError::PermissionDenied(
                "Windows didn't let the launcher turn Bluetooth on or off: allow it in Windows' privacy settings"
                    .into(),
            ));
        }
        let radio = Self::radio()?.ok_or_else(|| PluginError::NotFound("No Bluetooth adapter".into()))?;
        let state = if on { RadioState::On } else { RadioState::Off };
        let changed = radio.SetStateAsync(state).and_then(|s| s.join()).map_err(failed("Bluetooth didn't change"))?;
        if changed != RadioAccessStatus::Allowed {
            return Err(PluginError::Other("Windows didn't turn Bluetooth on or off: turn airplane mode off".into()));
        }
        Ok(())
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Bluetooth {
    /// The radio, the paired devices, and what the last scan found that isn't paired.
    async fn state(&self) -> PluginResult<AreaState> {
        let found = self.found.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        blocking(QUICK, move || {
            let Some(radio) = Bluetooth::radio()? else {
                return Ok(AreaState::Unsupported("No Bluetooth adapter".into()));
            };
            let powered = radio.State().is_ok_and(|s| s == RadioState::On);
            let mut devices: Vec<BluetoothDevice> = Bluetooth::paired()?.into_iter().map(|(_, d)| d).collect();
            for new in found {
                if !devices.iter().any(|d| d.address == new.address) {
                    devices.push(BluetoothDevice {
                        address: new.address,
                        name: new.name,
                        paired: false,
                        connected: false,
                    });
                }
            }
            Ok(AreaState::Ready(AreaData::Bluetooth { powered, devices }))
        })
        .await
    }

    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        match action {
            AreaAction::SetBluetooth { on } => blocking(QUICK, move || Bluetooth::set_powered(on)).await,
            AreaAction::ScanBluetooth { seconds } => {
                job.progress(None, "Looking for devices: put yours in pairing mode");
                let (keep, handle) = (self.found.clone(), job.clone());
                tools::job(job, async move {
                    let limit = Duration::from_secs(u64::from(seconds) + 15);
                    let scan = move || Bluetooth::scan(seconds, &move || handle.cancelled());
                    let found = blocking(limit, scan).await.map_err(tools::reason)?;
                    *keep.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = found;
                    Ok("Done looking".into())
                });
                Ok(())
            }
            AreaAction::Pair { address } => {
                let found = self.found.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                let device = found
                    .into_iter()
                    .find(|f| f.address.eq_ignore_ascii_case(&address))
                    .ok_or_else(|| PluginError::NotFound(format!("{address} wasn't found: look for devices again")))?;
                job.progress(None, "Pairing");
                tools::job(job, async move {
                    blocking(PAIR_TIME, move || Bluetooth::pair(&device)).await.map_err(tools::reason)?;
                    Ok("Paired".into())
                });
                Ok(())
            }
            AreaAction::RemoveDevice { address } => blocking(QUICK, move || Bluetooth::remove(&address)).await,
            AreaAction::ConnectDevice { .. } | AreaAction::DisconnectDevice { .. } => Err(PluginError::Unsupported(
                "Windows connects a paired device by itself when it is on: turn the device on or off".into(),
            )),
            other => Err(PluginError::Unsupported(format!("the Bluetooth area can't run {other:?}"))),
        }
    }
}

/// librqbit 9.0.1 can't bind its connections to an interface on Windows.
struct Vpns;

#[launcher_api::async_trait]
impl AreaSettings for Vpns {
    async fn state(&self) -> PluginResult<AreaState> {
        Ok(AreaState::Unsupported("Not available on Windows".into()))
    }

    async fn act(&self, _action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        Err(PluginError::Unsupported("this system can't change a VPN: use the VPN's own app".into()))
    }
}

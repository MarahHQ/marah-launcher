//! Windows: the battery (`GetSystemPowerStatus`), restart and shut down (`shutdown`), and the
//! drives (the volumes with a letter, and Configuration Manager's eject). VPNs say why they
//! can't work there.
//!
//! Windows' own calls block: each runs on a blocking thread with COM set up for it, for at
//! most a time limit ([`blocking`]). What Windows' calls return becomes the areas' data in
//! `winmap.rs`, which the tests run on every OS.

use std::sync::Arc;
use std::time::Duration;

use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, PluginError, PluginResult, SystemArea, SystemDrive, SystemJob,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW,
    CM_Get_Device_Interface_PropertyW, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CM_Request_Device_EjectW,
    CR_SUCCESS, PNP_VETO_TYPE, PNP_VetoTypeUnknown,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetDriveTypeW,
    GetLogicalDriveStringsW, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, OPEN_EXISTING,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::System::WindowsProgramming::{DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOVABLE};
use windows::core::PCWSTR;

use crate::tools::{self, Runner, Tool};
use crate::winmap;

/// Windows' own restart and shut down.
const SHUTDOWN: Tool = Tool::new("shutdown", "Windows' shutdown");

/// Each area Windows manages, and the tools it runs.
pub const AREAS: &[(SystemArea, &[Tool])] =
    &[(SystemArea::Vpn, &[]), (SystemArea::Power, &[SHUTDOWN]), (SystemArea::Drives, &[])];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    match area {
        SystemArea::Vpn => Arc::new(Vpns),
        SystemArea::Drives => Arc::new(Drives),
        _ => Arc::new(Power(tools)),
    }
}

/// How long a read or a quick change through Windows' own calls may take.
const QUICK: Duration = crate::tools::QUICK;

/// COM, set up on this thread for as long as it lives: the multithreaded kind, so a wait on a
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
    /// Each volume with a letter that has a file system: a disk, a USB stick, a card, a disc.
    /// Not a network share, which can hold the list for as long as the network takes.
    fn list() -> Vec<SystemDrive> {
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
            label: unwide(&label),
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
        let (mut veto, mut vetoed_by) = (PNP_VetoTypeUnknown, [0u16; 261]);
        #[allow(unsafe_code)] // Asks Windows to eject the device it found.
        // SAFETY: `veto` and `vetoed_by` are writable and live through the call.
        let ejected = unsafe { CM_Request_Device_EjectW(node, Some(&mut veto), Some(&mut vetoed_by), 0) };
        if ejected != CR_SUCCESS || veto != PNP_VetoTypeUnknown {
            return Err(PluginError::Other(eject_refused(veto)));
        }
        Ok(())
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

/// Why Windows didn't eject a drive, for the player.
fn eject_refused(veto: PNP_VETO_TYPE) -> String {
    if veto == PNP_VetoTypeUnknown {
        "Windows didn't eject the drive".into()
    } else {
        "A program still uses the drive: close it, then eject again".into()
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

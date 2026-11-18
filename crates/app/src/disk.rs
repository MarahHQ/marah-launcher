//! The disks: how much free space the disk that holds a folder has, and its size.

use std::path::Path;

/// The free bytes on the disk that holds `path`; `None` when the system doesn't say.
#[allow(unsafe_code)] // One system call that fills a struct this function owns.
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid C string and `stat` a writable statvfs.
        (unsafe { libc::statvfs(c.as_ptr(), &mut stat) } == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` ends with a nul and `free` is a writable u64.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// The size in bytes of the disk that holds `path`; `None` when the system doesn't say.
#[allow(unsafe_code)] // One system call that fills a struct this function owns.
pub fn disk_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid C string and `stat` a writable statvfs.
        (unsafe { libc::statvfs(c.as_ptr(), &mut stat) } == 0).then(|| stat.f_blocks as u64 * stat.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut total = 0u64;
        // SAFETY: `wide` ends with a nul and `total` is a writable u64.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &mut total,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(total)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Which disk holds `path`: two paths with the same id share their free space. `None` when
/// the system doesn't say. The device id on Unix; on Windows, the serial number of the volume
/// the path is on.
#[allow(unsafe_code)] // Two system calls that fill buffers this function owns.
pub fn disk_id(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).ok().map(|m| m.dev())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut root = [0u16; 1024];
        // SAFETY: `wide` ends with a nul, and `root` is writable for the length given.
        if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return None;
        }
        let mut serial = 0u32;
        // SAFETY: `root` holds the nul-ended path the call above wrote, and `serial` is a
        // writable u32; the other buffers are null with a size of 0.
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        (ok != 0).then_some(u64::from(serial))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_and_a_folder_in_it_are_on_the_same_disk() {
        let dir = std::env::temp_dir().join(format!("disk-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (outer, inner) = (disk_id(&std::env::temp_dir()), disk_id(&dir));
        std::fs::remove_dir(&dir).unwrap();
        assert!(outer.is_some());
        assert_eq!(inner, outer);
        assert_eq!(disk_id(&dir), None, "a folder that isn't there");
    }

    #[test]
    fn the_free_space_of_a_folder_s_disk_is_known() {
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|f| f > 0));
    }

    #[test]
    fn a_folder_s_disk_is_at_least_as_big_as_its_free_space() {
        let dir = std::env::temp_dir();
        assert!(disk_bytes(&dir).unwrap() >= free_bytes(&dir).unwrap());
    }
}

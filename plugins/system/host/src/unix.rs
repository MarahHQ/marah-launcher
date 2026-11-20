//! Unix's own calls that more than one OS module needs: Linux and macOS read the interfaces
//! with an address.

use std::collections::BTreeSet;

/// The interfaces that are up, aren't the loopback, and have an IPv4 or a global IPv6
/// address.
#[allow(unsafe_code)] // getifaddrs: the list it makes is read, then freed, in this function.
pub fn addressed() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `list` is a writable pointer; on success it holds a list freed below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return names;
    }
    let mut cursor = list;
    while !cursor.is_null() {
        // SAFETY: `cursor` is a node of the list getifaddrs made, not freed yet.
        let entry = unsafe { &*cursor };
        let up = entry.ifa_flags & libc::IFF_UP as libc::c_uint != 0;
        let loopback = entry.ifa_flags & libc::IFF_LOOPBACK as libc::c_uint != 0;
        if up && !loopback && !entry.ifa_addr.is_null() {
            // SAFETY: `ifa_addr` is not null, and points to a sockaddr of the family it names.
            let family = i32::from(unsafe { (*entry.ifa_addr).sa_family });
            let usable = match family {
                libc::AF_INET => true,
                libc::AF_INET6 => {
                    // SAFETY: the family is AF_INET6, so the address is a sockaddr_in6.
                    let v6 = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                    let bytes = v6.sin6_addr.s6_addr;
                    // Not link-local (fe80::/10).
                    !(bytes[0] == 0xfe && bytes[1] & 0xc0 == 0x80)
                }
                _ => false,
            };
            if usable {
                // SAFETY: `ifa_name` is a nul-terminated string in the list.
                let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }.to_string_lossy().into_owned();
                names.insert(name);
            }
        }
        cursor = entry.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and nothing reads it after this.
    unsafe { libc::freeifaddrs(list) };
    names
}

//! What Windows' own calls return, as the areas' data: plain functions with no Windows type, so
//! the tests run on every OS. `windows.rs` makes the calls and hands their values here.

use launcher_api::{Battery, WifiNetwork};

/// The battery from `GetSystemPowerStatus`: its `BatteryFlag` and `BatteryLifePercent`. `None`
/// on a PC without a battery, or when Windows doesn't know.
pub fn battery(flag: u8, percent: u8) -> Option<Battery> {
    // 128: no battery; 255: Windows doesn't know. A percent of 255 is unknown too.
    const NO_BATTERY: u8 = 128;
    const UNKNOWN: u8 = 255;
    const CHARGING: u8 = 8;
    if flag == UNKNOWN || flag & NO_BATTERY != 0 || percent > 100 {
        return None;
    }
    Some(Battery { percent, charging: flag & CHARGING != 0 })
}

/// A volume from Core Audio's 0.0 to 1.0, as 0 to 100.
pub fn volume_percent(scalar: f32) -> u8 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// A volume of 0 to 100, as Core Audio's 0.0 to 1.0.
pub fn volume_scalar(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

/// A Bluetooth address as WinRT gives it (the low 48 bits of a number), as the player reads it:
/// `AA:BB:CC:DD:EE:FF`.
pub fn bluetooth_address(address: u64) -> String {
    (0..6).rev().map(|i| format!("{:02X}", (address >> (i * 8)) & 0xff)).collect::<Vec<_>>().join(":")
}

/// `AA:BB:CC:DD:EE:FF` (any case) as WinRT's number. `None` when it isn't one.
pub fn parse_bluetooth_address(text: &str) -> Option<u64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() != 6 || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    parts.iter().try_fold(0u64, |address, part| Some(address << 8 | u64::from(u8::from_str_radix(part, 16).ok()?)))
}

/// One entry of the WLAN API's list of networks in reach. Windows lists a network once for each
/// saved profile it has, and once more without one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The SSID's bytes: most are UTF-8, but any byte may be in one.
    pub ssid: Vec<u8>,
    /// 0 to 100.
    pub signal: u8,
    pub secured: bool,
    /// Windows has a profile for it.
    pub saved: bool,
    pub active: bool,
}

/// The networks in reach, one per SSID, strongest first. Hidden networks (no SSID) aren't
/// listed: the player can't pick them by name.
pub fn wifi(seen: Vec<Seen>) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for entry in seen.into_iter().filter(|s| !s.ssid.is_empty()) {
        let ssid = String::from_utf8_lossy(&entry.ssid).into_owned();
        match networks.iter_mut().find(|n| n.ssid == ssid) {
            Some(known) => {
                known.signal = known.signal.max(entry.signal);
                known.secured |= entry.secured;
                known.saved |= entry.saved;
                known.active |= entry.active;
            }
            None => networks.push(WifiNetwork {
                ssid,
                signal: entry.signal.min(100),
                secured: entry.secured,
                saved: entry.saved,
                active: entry.active,
            }),
        }
    }
    networks.sort_by_key(|n| std::cmp::Reverse(n.signal));
    networks
}

/// How a network signs in, as a WLAN profile names it: its `authentication` and `encryption`.
/// From the WLAN API's `DOT11_AUTH_ALGORITHM` and `DOT11_CIPHER_ALGORITHM` numbers. `None` for a
/// network that signs in with an account (802.1X), which needs more than a password.
pub fn profile_security(auth: i32, cipher: i32) -> Option<(&'static str, &'static str)> {
    let authentication = match auth {
        1 => "open",
        2 => "shared",
        4 => "WPAPSK",
        7 => "WPA2PSK",
        9 => "WPA3SAE",
        10 => "OWE",
        _ => return None,
    };
    let encryption = match cipher {
        0 => "none",
        1 | 5 | 257 => "WEP",
        2 => "TKIP",
        4 | 8 | 9 | 10 => "AES",
        _ => return None,
    };
    Some((authentication, encryption))
}

/// The WLAN profile that joins `ssid` with a password, for `WlanSetProfile`. `security` is
/// [`profile_security`]'s. The SSID goes in as hex too, so any byte in it survives.
pub fn wifi_profile(ssid: &str, security: (&str, &str), password: Option<&str>) -> String {
    let (authentication, encryption) = security;
    let name = escape(ssid);
    let hex: String = ssid.bytes().map(|b| format!("{b:02X}")).collect();
    let key = match password.filter(|p| !p.is_empty()) {
        Some(password) => {
            let kind = if encryption == "WEP" { "networkKey" } else { "passPhrase" };
            format!(
                "<sharedKey><keyType>{kind}</keyType><protected>false</protected>\
                 <keyMaterial>{}</keyMaterial></sharedKey>",
                escape(password)
            )
        }
        None => String::new(),
    };
    format!(
        "<?xml version=\"1.0\"?>\
         <WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
         <name>{name}</name>\
         <SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig>\
         <connectionType>ESS</connectionType><connectionMode>auto</connectionMode>\
         <MSM><security><authEncryption><authentication>{authentication}</authentication>\
         <encryption>{encryption}</encryption><useOneX>false</useOneX></authEncryption>{key}\
         </security></MSM></WLANProfile>"
    )
}

/// `text` as XML text.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// What `IOCTL_STORAGE_QUERY_PROPERTY` says of a disk: its `STORAGE_DEVICE_DESCRIPTOR`, with the
/// strings it points to after it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StorageDevice {
    /// It is plugged in by USB.
    pub usb: bool,
    /// Its vendor and product, such as "SanDisk Ultra". Empty when it doesn't say.
    pub model: String,
}

/// The descriptor in `bytes`, as the call wrote it. `None` when it is too short to be one.
pub fn storage_device(bytes: &[u8]) -> Option<StorageDevice> {
    // The descriptor's fields: offsets of the vendor (12) and product (16) strings, and the bus
    // type (28). 7 is BusTypeUsb.
    const BUS_USB: u32 = 7;
    let field = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let bus = field(28)?;
    let text = |offset: u32| -> String {
        let offset = offset as usize;
        // An offset of 0 means the disk doesn't say.
        if offset == 0 || offset >= bytes.len() {
            return String::new();
        }
        let end = bytes[offset..].iter().position(|b| *b == 0).map_or(bytes.len(), |n| offset + n);
        String::from_utf8_lossy(&bytes[offset..end]).trim().to_owned()
    };
    let parts = [text(field(12)?), text(field(16)?)];
    let model = parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    Some(StorageDevice { usb: bus == BUS_USB, model })
}

/// The strings of a list Windows ends each with a nul, and the list with one more.
pub fn multi_sz(wide: &[u16]) -> Vec<String> {
    wide.split(|c| *c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// A disk's name, as the drives give it to an eject: `\\.\PhysicalDrive<number>`.
pub fn disk_name(number: u32) -> String {
    format!(r"\\.\PhysicalDrive{number}")
}

/// The number in a disk's name ([`disk_name`]).
pub fn disk_number(name: &str) -> Option<u32> {
    name.strip_prefix(r"\\.\PhysicalDrive")?.parse().ok()
}

/// The GUID in a volume's name, `\\?\Volume{GUID}\`, without its braces and in lower case: it
/// stays the same from one plug-in to the next. Empty when the name has none.
pub fn volume_id(name: &str) -> String {
    let guid = name.split_once('{').and_then(|(_, rest)| rest.split_once('}')).map(|(guid, _)| guid);
    guid.unwrap_or_default().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pc_without_a_battery_has_none_and_a_laptop_s_reads() {
        assert_eq!(battery(128, 255), None);
        assert_eq!(battery(255, 255), None);
        assert_eq!(battery(1, 255), None);
        assert_eq!(battery(1, 87), Some(Battery { percent: 87, charging: false }));
        assert_eq!(battery(8 | 2, 12), Some(Battery { percent: 12, charging: true }));
    }

    #[test]
    fn a_volume_goes_both_ways_without_drift() {
        assert_eq!(volume_percent(0.0), 0);
        assert_eq!(volume_percent(0.355), 36);
        assert_eq!(volume_percent(1.2), 100);
        for percent in [0, 1, 33, 50, 99, 100] {
            assert_eq!(volume_percent(volume_scalar(percent)), percent);
        }
        assert_eq!(volume_scalar(150), 1.0);
    }

    #[test]
    fn a_bluetooth_address_reads_as_the_player_knows_it_and_back() {
        let address = 0x00_1A_7D_DA_71_13;
        assert_eq!(bluetooth_address(address), "00:1A:7D:DA:71:13");
        assert_eq!(parse_bluetooth_address("00:1a:7d:da:71:13"), Some(address));
        assert_eq!(parse_bluetooth_address("00:1A:7D:DA:71"), None);
        assert_eq!(parse_bluetooth_address("00:1A:7D:DA:71:1"), None);
        assert_eq!(parse_bluetooth_address("zz:1A:7D:DA:71:13"), None);
    }

    #[test]
    fn each_network_shows_once_with_what_its_entries_say_strongest_first() {
        let seen =
            |ssid: &[u8], signal, saved, active| Seen { ssid: ssid.to_vec(), signal, secured: true, saved, active };
        let list = wifi(vec![
            seen(b"Cafe", 40, false, false),
            seen(b"Home", 70, true, true),
            seen(b"Home", 72, false, false),
            seen(b"", 90, false, false),
            seen(b"Caf\xe9", 10, false, false),
        ]);

        let names: Vec<_> = list.iter().map(|n| (n.ssid.as_str(), n.signal, n.saved, n.active)).collect();
        assert_eq!(names, [("Home", 72, true, true), ("Cafe", 40, false, false), ("Caf\u{fffd}", 10, false, false)]);
    }

    #[test]
    fn a_profile_names_how_the_network_signs_in_and_escapes_the_player_s_text() {
        assert_eq!(profile_security(7, 4), Some(("WPA2PSK", "AES")));
        assert_eq!(profile_security(1, 0), Some(("open", "none")));
        assert_eq!(profile_security(1, 257), Some(("open", "WEP")));
        assert_eq!(profile_security(9, 4), Some(("WPA3SAE", "AES")));
        // 802.1X needs an account, not a password.
        assert_eq!(profile_security(6, 4), None);
        assert_eq!(profile_security(11, 4), None);

        let xml = wifi_profile("Tom & <Jo>", ("WPA2PSK", "AES"), Some("p\"a'ss<"));
        assert!(xml.contains("<name>Tom &amp; &lt;Jo&gt;</name>"), "{xml}");
        assert!(xml.contains("<hex>546F6D2026203C4A6F3E</hex>"), "{xml}");
        assert!(xml.contains("<keyType>passPhrase</keyType>"), "{xml}");
        assert!(xml.contains("<keyMaterial>p&quot;a&apos;ss&lt;</keyMaterial>"), "{xml}");
        assert!(xml.contains("<authentication>WPA2PSK</authentication><encryption>AES</encryption>"), "{xml}");

        let open = wifi_profile("Cafe", ("open", "none"), None);
        assert!(!open.contains("sharedKey"), "{open}");
        assert!(wifi_profile("Old", ("open", "WEP"), Some("abcde")).contains("<keyType>networkKey</keyType>"));
    }

    /// A descriptor as the call writes it: the fixed part, then the strings it points to.
    fn descriptor(bus: u32, vendor: &str, product: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; 40];
        let mut strings = Vec::new();
        let mut put = |at: usize, text: &str, bytes: &mut Vec<u8>| {
            if text.is_empty() {
                return;
            }
            let offset = 40 + strings.len() as u32;
            bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
            strings.extend_from_slice(text.as_bytes());
            strings.push(0);
        };
        put(12, vendor, &mut bytes);
        put(16, product, &mut bytes);
        bytes[28..32].copy_from_slice(&bus.to_le_bytes());
        bytes.extend(strings);
        bytes
    }

    #[test]
    fn a_disk_says_whether_it_is_on_usb_and_its_model() {
        assert_eq!(
            storage_device(&descriptor(7, "SanDisk ", "Ultra  ")),
            Some(StorageDevice { usb: true, model: "SanDisk Ultra".into() })
        );
        assert_eq!(
            storage_device(&descriptor(17, "", "Samsung SSD 980")),
            Some(StorageDevice { usb: false, model: "Samsung SSD 980".into() })
        );
        assert_eq!(storage_device(&[0; 20]), None);
        // An offset past the end reads as nothing, not a panic.
        let mut broken = descriptor(7, "", "");
        broken[16..20].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(storage_device(&broken), Some(StorageDevice { usb: true, model: String::new() }));
    }

    #[test]
    fn names_of_disks_volumes_and_lists_read_back() {
        let wide: Vec<u16> = "C:\\\0E:\\\0\0".encode_utf16().collect();
        assert_eq!(multi_sz(&wide), ["C:\\", "E:\\"]);
        assert_eq!(disk_name(2), r"\\.\PhysicalDrive2");
        assert_eq!(disk_number(&disk_name(12)), Some(12));
        assert_eq!(disk_number("E:\\"), None);
        assert_eq!(
            volume_id(r"\\?\Volume{6B29FC40-CA47-1067-B31D-00DD010662DA}\"),
            "6b29fc40-ca47-1067-b31d-00dd010662da"
        );
        assert_eq!(volume_id("E:\\"), "");
    }
}

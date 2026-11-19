//! What macOS's own tools print, as the areas' data: plain functions with no macOS call, so the
//! tests run on every OS. `macos.rs` runs the tools and hands their output here.
//!
//! `diskutil` prints its lists as an XML property list (`-plist`), the form its man page tells
//! scripts to read. No plist crate is in the workspace, so [`plist`] reads the part of the
//! format that `diskutil` writes (`dict`, `array`, `key`, `string`, `integer`, `real`, `true`,
//! `false`, `date`, `data`) into JSON values, and the drive functions read those.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use launcher_api::serde_json::{Map, Number, Value};
use launcher_api::{AudioLevel, BluetoothDevice, Connection, NetworkStatus, SystemDrive, Vpn, WifiNetwork};

/// An XML property list as a JSON value: a `dict` is an object, an `array` an array, a
/// `string`, `date` or `data` a string, `true` and `false` booleans. `None` when it isn't one.
pub fn plist(xml: &str) -> Option<Value> {
    let mut reader = Reader { rest: &xml[xml.find("<plist")?..] };
    let root = reader.tag()?;
    if root.name != "plist" || root.closing || root.empty {
        return None;
    }
    let value = reader.value()?;
    let end = reader.tag()?;
    (end.closing && end.name == "plist").then_some(value)
}

struct Tag<'a> {
    name: &'a str,
    /// `</name>`.
    closing: bool,
    /// `<name/>`.
    empty: bool,
}

struct Reader<'a> {
    rest: &'a str,
}

impl<'a> Reader<'a> {
    /// The next tag, past blanks, comments and declarations.
    fn tag(&mut self) -> Option<Tag<'a>> {
        loop {
            let rest = self.rest.trim_start();
            if let Some(after) = rest.strip_prefix("<!--") {
                self.rest = &after[after.find("-->")? + 3..];
            } else if rest.starts_with("<?") || rest.starts_with("<!") {
                self.rest = &rest[rest.find('>')? + 1..];
            } else {
                let inner = rest.strip_prefix('<')?;
                let end = inner.find('>')?;
                self.rest = &inner[end + 1..];
                let body = &inner[..end];
                let (body, closing) = body.strip_prefix('/').map_or((body, false), |b| (b, true));
                let (body, empty) = body.strip_suffix('/').map_or((body, false), |b| (b, true));
                return Some(Tag { name: body.split_whitespace().next()?, closing, empty });
            }
        }
    }

    /// The next tag, left to read.
    fn peek(&self) -> Option<Tag<'a>> {
        Reader { rest: self.rest }.tag()
    }

    /// The text of an element that `name` opened, and its closing tag.
    fn text(&mut self, name: &str) -> Option<String> {
        let end = self.rest.find('<')?;
        let text = unescape(&self.rest[..end])?;
        self.rest = &self.rest[end..];
        let close = self.tag()?;
        (close.closing && close.name == name).then_some(text)
    }

    fn value(&mut self) -> Option<Value> {
        let tag = self.tag()?;
        if tag.closing {
            return None;
        }
        let text = |me: &mut Self| if tag.empty { Some(String::new()) } else { me.text(tag.name) };
        match tag.name {
            "dict" => {
                let mut map = Map::new();
                if tag.empty {
                    return Some(Value::Object(map));
                }
                loop {
                    let key = self.tag()?;
                    if key.closing {
                        if key.name != "dict" {
                            return None;
                        }
                        break;
                    }
                    if key.name != "key" {
                        return None;
                    }
                    let name = if key.empty { String::new() } else { self.text("key")? };
                    map.insert(name, self.value()?);
                }
                Some(Value::Object(map))
            }
            "array" => {
                let mut items = Vec::new();
                if tag.empty {
                    return Some(Value::Array(items));
                }
                loop {
                    if self.peek()?.closing {
                        if self.tag()?.name != "array" {
                            return None;
                        }
                        break;
                    }
                    items.push(self.value()?);
                }
                Some(Value::Array(items))
            }
            "string" | "date" => text(self).map(Value::String),
            // Base64 that may wrap: the lines joined.
            "data" => text(self).map(|t| Value::String(t.split_whitespace().collect())),
            "integer" => {
                let t = text(self)?;
                let t = t.trim();
                t.parse::<u64>()
                    .map(Number::from)
                    .or_else(|_| t.parse::<i64>().map(Number::from))
                    .ok()
                    .map(Value::Number)
            }
            "real" => text(self)?.trim().parse::<f64>().ok().and_then(Number::from_f64).map(Value::Number),
            "true" | "false" => {
                if !tag.empty {
                    text(self)?;
                }
                Some(Value::Bool(tag.name == "true"))
            }
            _ => None,
        }
    }
}

/// XML's five named characters, and numbered ones (`&#38;`, `&#x26;`).
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after.find(';')?;
        let named = match &after[..end] {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            number => {
                let number = number.strip_prefix('#')?;
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)?
            }
        };
        out.push(named);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// A volume that `diskutil list -plist external` lists, and the whole disk that ejects it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Such as `disk4s1`.
    pub id: String,
    /// The physical whole disk, such as `disk4`. For an APFS volume it is the disk that holds
    /// the container, not the container's own synthesized disk.
    pub disk: String,
}

/// Partitions that hold no volume a player uses: the boot partitions, and an APFS container's
/// store (its volumes are listed under the container).
const NOT_VOLUMES: &[&str] = &[
    "EFI",
    "Apple_APFS",
    "Apple_APFS_ISC",
    "Apple_APFS_Recovery",
    "Apple_Boot",
    "Apple_CoreStorage",
    "Apple_Free",
    "Apple_partition_map",
    "Microsoft Reserved",
];

/// The items of `value[key]`, an array; none when it isn't one.
fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The volumes in `diskutil list -plist`: each partition that can hold one, each APFS
/// container's volumes, and a disk with no partition map that is one volume itself.
pub fn volumes(list: &Value) -> Vec<Volume> {
    let disks = items(list, "AllDisksAndPartitions");
    let id = |v: &Value| v["DeviceIdentifier"].as_str().unwrap_or_default().to_owned();
    let whole_of: HashMap<String, String> =
        disks.iter().flat_map(|d| items(d, "Partitions").iter().map(move |p| (id(p), id(d)))).collect();
    let mut found = Vec::new();
    for disk in disks {
        let name = id(disk);
        if name.is_empty() {
            continue;
        }
        if let Some(stores) = disk["APFSPhysicalStores"].as_array() {
            // A container whose store isn't on a listed disk can't be ejected safely: skipped.
            let Some(physical) = stores.first().map(id).and_then(|store| whole_of.get(&store).cloned()) else {
                continue;
            };
            found.extend(items(disk, "APFSVolumes").iter().map(id).map(|id| Volume { id, disk: physical.clone() }));
        } else if disk.get("Partitions").is_none() {
            found.push(Volume { id: name.clone(), disk: name });
        } else {
            let usable = items(disk, "Partitions")
                .iter()
                .filter(|p| !NOT_VOLUMES.contains(&p["Content"].as_str().unwrap_or("")));
            found.extend(usable.map(id).map(|id| Volume { id, disk: name.clone() }));
        }
    }
    found.retain(|v| !v.id.is_empty());
    found
}

/// A volume from `diskutil info -plist <volume>`, on the physical whole disk `disk`. `None`
/// for a volume with no file system macOS reads, an internal one, or a disk image.
///
/// It comes from the external disks' list, so the player can take it out (`usb`). A volume the
/// system runs from (`/`, `/System/Volumes/...`) is marked `system`: the launcher never
/// unmounts it.
pub fn drive(info: &Value, disk: &str) -> Option<SystemDrive> {
    let text = |key: &str| info[key].as_str().unwrap_or_default().trim().to_owned();
    let number = |key: &str| info[key].as_u64();
    let filesystem = text("FilesystemType");
    let id = text("DeviceIdentifier");
    let internal = info["Internal"].as_bool().unwrap_or(false);
    if filesystem.is_empty() || id.is_empty() || internal || text("BusProtocol") == "Disk Image" {
        return None;
    }
    // `info` says "" for a volume that isn't mounted.
    let mount = Some(text("MountPoint")).filter(|m| !m.is_empty()).map(PathBuf::from);
    // APFS says 0 in FreeSpace: its free space is the container's.
    let free = mount.as_ref().and(number("APFSContainerFree").or_else(|| number("FreeSpace")));
    let device = Some(text("DeviceNode")).filter(|d| !d.is_empty()).unwrap_or_else(|| format!("/dev/{id}"));
    let uuid = [text("VolumeUUID"), text("DiskUUID")].into_iter().find(|u| !u.is_empty()).unwrap_or_default();
    Some(SystemDrive {
        device,
        disk: format!("/dev/{disk}"),
        uuid,
        label: text("VolumeName"),
        model: text("MediaName"),
        size: number("TotalSize").or_else(|| number("Size")).unwrap_or(0),
        filesystem,
        usb: true,
        system: mount.as_deref().is_some_and(system_mount),
        mount,
        free,
    })
}

fn system_mount(at: &Path) -> bool {
    at == Path::new("/") || at.starts_with("/System/Volumes")
}

/// Why `diskutil mount`, `unmount` or `eject` failed, from what it printed, for the player.
pub fn diskutil_error(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.iter().any(|l| l.contains("dissented") || l.contains("could not be unmounted")) {
        return "A program is using the drive: close it and try again".into();
    }
    if lines.iter().any(|l| l.contains("Unable to find disk") || l.contains("Could not find disk")) {
        return "The drive isn't plugged in any more".into();
    }
    lines.last().map_or_else(|| "diskutil failed".into(), |l| (*l).to_owned())
}

/// `networksetup -listallhardwareports`: each port's name and its device, such as
/// `("Wi-Fi", "en0")`.
pub fn ports(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut port = None;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("Hardware Port:") {
            port = Some(name.trim().to_owned());
        } else if let Some(device) = line.strip_prefix("Device:")
            && let Some(name) = port.take()
        {
            found.push((name, device.trim().to_owned()));
        }
    }
    found
}

/// A port's name says it is Wi-Fi ("Wi-Fi", or "AirPort" before macOS 10.7).
fn is_wifi(port: &str) -> bool {
    port == "Wi-Fi" || port == "AirPort"
}

/// The Wi-Fi device in [`ports`], such as `en0`.
pub fn wifi_device(ports: &[(String, String)]) -> Option<String> {
    ports.iter().find(|(port, _)| is_wifi(port)).map(|(_, device)| device.clone())
}

/// `scutil --nwi`: the interface traffic goes through first, past VPN tunnels. Its last line
/// lists them in order: `Network interfaces: utun3 en0`.
pub fn primary(nwi: &str) -> Option<String> {
    let list = nwi.lines().find_map(|l| l.trim().strip_prefix("Network interfaces:"))?;
    let tunnel = |name: &str| ["utun", "ipsec", "ppp", "gif", "stf"].iter().any(|t| name.starts_with(t));
    list.split_whitespace().find(|name| !tunnel(name)).map(str::to_owned)
}

/// `networksetup -getairportnetwork <device>`: the network's name. `None` when it doesn't say:
/// since macOS 14 it hides the name from a program without Location Services, as
/// `<redacted>` or as "You are not associated with an AirPort network." while it is joined.
pub fn current_ssid(text: &str) -> Option<String> {
    let name = text.lines().find_map(|l| l.trim().strip_prefix("Current Wi-Fi Network:"))?.trim();
    (!name.is_empty() && !hidden(name)).then(|| name.to_owned())
}

fn hidden(name: &str) -> bool {
    name == "<redacted>" || name == "<hidden>"
}

/// How the Mac is online: through its first interface ([`primary`]), named by its port. The
/// Wi-Fi network's name when macOS says it, else "Wi-Fi".
pub fn network(primary: Option<&str>, ports: &[(String, String)], ssid: Option<String>) -> NetworkStatus {
    let Some(device) = primary else { return NetworkStatus::default() };
    let port = ports.iter().find(|(_, d)| d == device).map(|(port, _)| port.as_str());
    match port {
        Some(port) if is_wifi(port) => {
            NetworkStatus { connection: Connection::Wifi, name: ssid.unwrap_or_else(|| port.to_owned()), signal: None }
        }
        port => NetworkStatus { connection: Connection::Wired, name: port.unwrap_or(device).to_owned(), signal: None },
    }
}

/// `networksetup -listpreferredwirelessnetworks <device>`: the saved networks, after the
/// `Preferred networks on en0:` line, one to a line.
pub fn preferred(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with(char::is_whitespace))
        .map(str::trim)
        .filter(|l| !l.is_empty() && !hidden(l))
        .map(str::to_owned)
        .collect()
}

/// The Wi-Fi networks macOS's tools can name without a scan: the saved ones, the joined one
/// first. macOS's tools give no signal, so it is 0.
pub fn wifi(saved: &[String], current: Option<&str>) -> Vec<WifiNetwork> {
    let network = |ssid: &str| WifiNetwork {
        ssid: ssid.to_owned(),
        signal: 0,
        secured: true,
        saved: true,
        active: current == Some(ssid),
    };
    let mut list: Vec<WifiNetwork> = current.into_iter().map(network).collect();
    list.extend(saved.iter().filter(|s| current != Some(s.as_str())).map(|s| network(s)));
    list
}

/// What `networksetup -setairportnetwork` printed, as its error: it ends with 0 even when it
/// failed, and prints nothing when it joined. `None` when it joined.
pub fn join_error(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(if text.contains("Could not find network") {
        "The network isn't in reach".into()
    } else if text.contains("-3912") || text.contains("-3900") {
        // -3912: kCWChallengeFailureErr, the network refused the password.
        "The password is wrong".into()
    } else {
        text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or(text).trim_start_matches("** Error: ").to_owned()
    })
}

/// `system_profiler SPBluetoothDataType -json` (macOS 12 and later): whether Bluetooth is on,
/// and the paired devices. `None` when it lists no adapter.
pub fn bluetooth(json: &str) -> Option<(bool, Vec<BluetoothDevice>)> {
    let value: Value = launcher_api::serde_json::from_str(json).ok()?;
    let adapter = &value["SPBluetoothDataType"][0];
    let controller = adapter.get("controller_properties")?;
    let powered = controller["controller_state"].as_str() == Some("attrib_on");
    let mut devices = Vec::new();
    for (key, connected) in [("device_connected", true), ("device_not_connected", false)] {
        // An array of one-key objects: `{"<name>": {"device_address": ...}}`.
        for entry in items(adapter, key) {
            for (name, fields) in entry.as_object().into_iter().flatten() {
                let Some(address) = fields["device_address"].as_str() else { continue };
                devices.push(BluetoothDevice {
                    address: address.to_owned(),
                    name: name.clone(),
                    paired: true,
                    connected,
                });
            }
        }
    }
    Some((powered, devices))
}

/// `osascript -e 'get volume settings'`: `output volume:58, input volume:85, alert
/// volume:72, output muted:false`. `None` when the output has no volume of its own (`missing
/// value`), such as some HDMI or USB outputs.
pub fn level(text: &str) -> Option<AudioLevel> {
    let field =
        |name: &str| text.split(',').find_map(|part| part.trim().strip_prefix(name)?.strip_prefix(':')).map(str::trim);
    let volume = field("output volume")?.parse::<u8>().ok()?.min(100);
    Some(AudioLevel { volume, muted: field("output muted") == Some("true") })
}

/// The VPNs in `scutil --nc list`, one to a line:
/// `* (Disconnected)   <id> VPN (com.wireguard.macos) "Home"   [VPN:com.wireguard.macos]`.
/// The `*` marks one that is on; macOS can start and stop those. macOS doesn't name their
/// interface here, so a download can't go through one by its name.
pub fn services(text: &str) -> Vec<Vpn> {
    text.lines()
        .filter_map(|line| {
            let enabled = line.starts_with('*');
            let status = line.split_once('(')?.1.split_once(')')?.0;
            let (start, end) = (line.find('"')?, line.rfind('"')?);
            let name = line.get(start + 1..end).filter(|n| !n.is_empty())?;
            let kind = line.rsplit_once('[').and_then(|(_, k)| k.strip_suffix(']')).unwrap_or("VPN");
            let kind = match kind.split_once(':') {
                Some(("PPP", sub)) => sub,
                Some((kind, _)) => kind,
                None => kind,
            };
            Some(Vpn {
                name: name.to_owned(),
                kind: kind.to_owned(),
                connected: status == "Connected",
                interface: String::new(),
                switchable: enabled,
            })
        })
        .collect()
}

/// `scutil --nc status <name>`: its first word, such as `Connected` or `Connecting`.
pub fn nc_status(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or_default().trim_end_matches(':')
}

/// Why an `osascript` call failed, for the player. macOS asks once whether the launcher may
/// control System Events; after a "Don't Allow" the call fails with -1743.
pub fn apple_event_error(text: &str) -> String {
    if text.contains("-1743") {
        return "Allow the launcher to control System Events in System Settings, Privacy & Security, Automation".into();
    }
    let last = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("osascript failed");
    // `execution error: <message> (-128)`
    last.split_once("execution error: ").map_or(last, |(_, why)| why).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = include_str!("fixtures/diskutil-list-external.plist");
    const STICK: &str = include_str!("fixtures/diskutil-info-stick.plist");
    const APFS: &str = include_str!("fixtures/diskutil-info-apfs.plist");
    const BLUETOOTH: &str = include_str!("fixtures/system-profiler-bluetooth.json");

    #[test]
    fn a_property_list_reads_as_json() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<!-- a comment -->
	<key>Name</key>
	<string>A &lt;b&gt; &amp; &#233;</string>
	<key>Empty</key>
	<string></string>
	<key>None</key>
	<string/>
	<key>Size</key>
	<integer>15682240512</integer>
	<key>Below</key>
	<integer>-3</integer>
	<key>Ratio</key>
	<real>0.5</real>
	<key>On</key>
	<true/>
	<key>Off</key>
	<false/>
	<key>List</key>
	<array>
		<array/>
		<dict/>
		<data>
		AAEC
		AwQ=
		</data>
	</array>
</dict>
</plist>
"#;
        let value = plist(xml).unwrap();
        assert_eq!(value["Name"], "A <b> & \u{e9}");
        assert_eq!(value["Empty"], "");
        assert_eq!(value["None"], "");
        assert_eq!(value["Size"].as_u64(), Some(15_682_240_512));
        assert_eq!(value["Below"].as_i64(), Some(-3));
        assert_eq!(value["Ratio"].as_f64(), Some(0.5));
        assert_eq!(value["On"], true);
        assert_eq!(value["Off"], false);
        assert_eq!(value["List"][0], Value::Array(Vec::new()));
        assert_eq!(value["List"][1], Value::Object(Map::new()));
        assert_eq!(value["List"][2], "AAECAwQ=");
    }

    #[test]
    fn a_broken_property_list_reads_as_nothing() {
        assert_eq!(plist("not a plist"), None);
        assert_eq!(plist("<plist><dict><key>A</key></dict></plist>"), None);
        assert_eq!(plist("<plist><dict><key>A</key><string>b</dict></plist>"), None);
        assert_eq!(plist("<plist><array><string>a</string></plist>"), None);
        assert_eq!(plist("<plist><string>&bad</string></plist>"), None);
    }

    #[test]
    fn the_external_list_names_each_volume_and_the_disk_that_ejects_it() {
        let list = plist(LIST).unwrap();
        let volume = |id: &str, disk: &str| Volume { id: id.into(), disk: disk.into() };
        assert_eq!(
            volumes(&list),
            [
                // A stick with no partition map is one volume.
                volume("disk4", "disk4"),
                // An NTFS partition that isn't mounted.
                volume("disk5s1", "disk5"),
                // Not EFI (disk6s1), nor the APFS store (disk6s2).
                volume("disk6s3", "disk6"),
                // The APFS volumes eject with the disk that holds their container.
                volume("disk7s1", "disk6"),
                volume("disk7s2", "disk6"),
            ]
        );
        assert!(volumes(&Value::Null).is_empty());
    }

    #[test]
    fn a_container_whose_store_isn_t_listed_is_skipped() {
        let list = plist(
            "<plist><dict><key>AllDisksAndPartitions</key><array><dict>\
             <key>APFSPhysicalStores</key><array><dict><key>DeviceIdentifier</key><string>disk0s2</string></dict></array>\
             <key>APFSVolumes</key><array><dict><key>DeviceIdentifier</key><string>disk3s1</string></dict></array>\
             <key>DeviceIdentifier</key><string>disk3</string><key>Partitions</key><array/>\
             </dict></array></dict></plist>",
        )
        .unwrap();
        assert!(volumes(&list).is_empty());
    }

    #[test]
    fn a_fat_stick_reads_with_its_label_size_and_free_space() {
        let drive = drive(&plist(STICK).unwrap(), "disk4").unwrap();
        assert_eq!(
            drive,
            SystemDrive {
                device: "/dev/disk4".into(),
                disk: "/dev/disk4".into(),
                uuid: "22222222-3333-3444-8555-666666666661".into(),
                label: "GAMES & SAVES".into(),
                model: "Example Flash".into(),
                size: 15_665_463_296,
                filesystem: "msdos".into(),
                usb: true,
                mount: Some("/Volumes/GAMES & SAVES".into()),
                free: Some(9_662_678_016),
                system: false,
            }
        );
    }

    #[test]
    fn an_apfs_volume_has_its_container_s_free_space_and_the_physical_disk() {
        let drive = drive(&plist(APFS).unwrap(), "disk6").unwrap();
        assert_eq!(drive.device, "/dev/disk7s1");
        assert_eq!(drive.disk, "/dev/disk6");
        assert_eq!(drive.free, Some(687_654_321_088));
        assert_eq!(drive.filesystem, "apfs");
        assert_eq!(drive.label, "Library");
    }

    #[test]
    fn a_volume_that_isn_t_mounted_has_no_folder_and_no_free_space() {
        let unmounted = STICK
            .replace("<string>/Volumes/GAMES &amp; SAVES</string>", "<string></string>")
            .replace("<integer>9662678016</integer>", "<integer>0</integer>");
        let drive = drive(&plist(&unmounted).unwrap(), "disk4").unwrap();
        assert_eq!(drive.mount, None);
        assert_eq!(drive.free, None);
    }

    #[test]
    fn the_system_s_own_volumes_are_marked_and_others_are_left_out() {
        let at = |mount: &str| {
            let info = APFS.replace("/Volumes/Library", mount);
            drive(&plist(&info).unwrap(), "disk6").unwrap().system
        };
        assert!(at("/"));
        assert!(at("/System/Volumes/Data"));
        assert!(!at("/Volumes/Library"));
        let info = |from: &str, to: &str| plist(&STICK.replace(from, to)).unwrap();
        // Internal, a disk image, or no file system macOS reads: not a drive.
        assert_eq!(drive(&info("<key>Internal</key>\n\t<false/>", "<key>Internal</key>\n\t<true/>"), "disk4"), None);
        assert_eq!(drive(&info("<string>USB</string>", "<string>Disk Image</string>"), "disk4"), None);
        assert_eq!(drive(&info("<string>msdos</string>", "<string></string>"), "disk4"), None);
    }

    #[test]
    fn diskutil_s_errors_read_plainly() {
        let busy = "Unmount of disk4 failed: at least one volume could not be unmounted\nUnmount was dissented by PID 412 (Finder)";
        assert_eq!(diskutil_error(busy), "A program is using the drive: close it and try again");
        assert_eq!(diskutil_error("Unable to find disk for /dev/disk9\n"), "The drive isn't plugged in any more");
        assert_eq!(diskutil_error("Volume on disk4s1 failed to mount\n"), "Volume on disk4s1 failed to mount");
        assert_eq!(diskutil_error(""), "diskutil failed");
    }

    const PORTS: &str = "\nHardware Port: Ethernet Adapter (en5)\nDevice: en5\nEthernet Address: aa:aa:aa:aa:aa:01\n\n\
        Hardware Port: Wi-Fi\nDevice: en0\nEthernet Address: aa:aa:aa:aa:aa:02\n\n\
        Hardware Port: Thunderbolt Bridge\nDevice: bridge0\nEthernet Address: aa:aa:aa:aa:aa:03\n\n\
        VLAN Configurations\n===================\n";

    const NWI: &str = "Network information\n\nIPv4 network interface information\n     utun4 : flags      : 0x5 (IPv4,DNS)\n\
        \x20          address    : 10.8.0.2\n     en0 : flags      : 0x5 (IPv4,DNS)\n           address    : 192.168.1.20\n\
        \x20          reach      : 0x00000002 (Reachable)\n\n   REACH : flags 0x00000002 (Reachable)\n\n\
        IPv6 network interface information\n   No IPv6 states found\n\n   REACH : flags 0x00000000 (Not Reachable)\n\n\
        Network interfaces: utun4 en0\n";

    #[test]
    fn the_ports_name_the_wi_fi_device() {
        let ports = ports(PORTS);
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[0], ("Ethernet Adapter (en5)".into(), "en5".into()));
        assert_eq!(wifi_device(&ports).as_deref(), Some("en0"));
        assert_eq!(wifi_device(&[]), None);
    }

    #[test]
    fn the_mac_is_online_through_its_first_interface_past_the_vpn() {
        let ports = ports(PORTS);
        assert_eq!(primary(NWI).as_deref(), Some("en0"));
        assert_eq!(primary("Network information\n\nNetwork interfaces: \n"), None);
        let wifi = network(Some("en0"), &ports, Some("Home".into()));
        assert_eq!(wifi, NetworkStatus { connection: Connection::Wifi, name: "Home".into(), signal: None });
        // macOS hid the name.
        assert_eq!(network(Some("en0"), &ports, None).name, "Wi-Fi");
        let wired = network(Some("en5"), &ports, None);
        assert_eq!(
            wired,
            NetworkStatus { connection: Connection::Wired, name: "Ethernet Adapter (en5)".into(), signal: None }
        );
        assert_eq!(network(None, &ports, None), NetworkStatus::default());
    }

    #[test]
    fn a_hidden_network_name_reads_as_unknown() {
        assert_eq!(current_ssid("Current Wi-Fi Network: Home Network\n").as_deref(), Some("Home Network"));
        assert_eq!(current_ssid("Current Wi-Fi Network: <redacted>\n"), None);
        assert_eq!(current_ssid("You are not associated with an AirPort network.\n"), None);
    }

    #[test]
    fn the_saved_networks_list_with_the_joined_one_first() {
        let saved = preferred("Preferred networks on en0:\n\tCafe\n\tHome Network\n\t<redacted>\n");
        assert_eq!(saved, ["Cafe", "Home Network"]);
        let list = wifi(&saved, Some("Home Network"));
        assert_eq!(list.iter().map(|w| w.ssid.as_str()).collect::<Vec<_>>(), ["Home Network", "Cafe"]);
        assert!(list[0].active && list[0].saved && !list[1].active);
        // Joined, and not in the list yet.
        assert_eq!(wifi(&saved, Some("Phone")).len(), 3);
        assert_eq!(wifi(&saved, None).len(), 2);
    }

    #[test]
    fn joining_fails_by_what_networksetup_printed() {
        assert_eq!(join_error(""), None);
        assert_eq!(join_error("Could not find network Cafe.\n").as_deref(), Some("The network isn't in reach"));
        let refused = "Failed to join network Cafe.\nError: -3912  The operation couldn\u{2019}t be completed. (com.apple.wifi.apple80211API.error error -3912.)";
        assert_eq!(join_error(refused).as_deref(), Some("The password is wrong"));
        assert_eq!(
            join_error("** Error: Error obtaining wireless information.").as_deref(),
            Some("Error obtaining wireless information.")
        );
    }

    #[test]
    fn bluetooth_lists_the_paired_devices_and_whether_it_is_on() {
        let (powered, devices) = bluetooth(BLUETOOTH).unwrap();
        assert!(powered);
        let named: Vec<_> = devices.iter().map(|d| (d.name.as_str(), d.address.as_str(), d.connected)).collect();
        assert_eq!(
            named,
            [
                ("Example Wireless Controller", "11:22:33:44:55:66", true),
                ("Example Headphones", "66:55:44:33:22:11", false),
                ("Example Keyboard", "AB:CD:EF:01:23:45", false),
            ]
        );
        assert!(devices.iter().all(|d| d.paired));
        let off = BLUETOOTH.replace("\"controller_state\" : \"attrib_on\"", "\"controller_state\" : \"attrib_off\"");
        assert!(!bluetooth(&off).unwrap().0);
        assert_eq!(bluetooth(r#"{"SPBluetoothDataType":[]}"#), None);
        assert_eq!(bluetooth("not json"), None);
    }

    #[test]
    fn the_volume_settings_read_as_a_level() {
        let level_of = |t| level(t);
        assert_eq!(
            level_of("output volume:58, input volume:85, alert volume:72, output muted:false\n"),
            Some(AudioLevel { volume: 58, muted: false })
        );
        assert_eq!(
            level_of("output volume:0, input volume:50, alert volume:100, output muted:true"),
            Some(AudioLevel { volume: 0, muted: true })
        );
        assert_eq!(
            level_of("output volume:missing value, input volume:50, alert volume:100, output muted:missing value"),
            None
        );
        assert_eq!(level_of(""), None);
    }

    #[test]
    fn scutil_lists_the_vpns_and_their_state() {
        let text = "Available network connection services in the current set (*=enabled):\n\
            * (Disconnected)   00000000-0000-4000-8000-000000000001 VPN (com.wireguard.macos) \"Home WG\"                      [VPN:com.wireguard.macos]\n\
            * (Connected)      00000000-0000-4000-8000-000000000002 PPP --> L2TP       \"Office\"                       [PPP:L2TP]\n\
            \x20 (Invalid)        00000000-0000-4000-8000-000000000003 IPSec              \"Old\"                          [IPSec]\n";
        let vpns = services(text);
        let named: Vec<_> =
            vpns.iter().map(|v| (v.name.as_str(), v.kind.as_str(), v.connected, v.switchable)).collect();
        assert_eq!(
            named,
            [("Home WG", "VPN", false, true), ("Office", "L2TP", true, true), ("Old", "IPSec", false, false)]
        );
        assert!(vpns.iter().all(|v| v.interface.is_empty()));
        assert!(services("Available network connection services in the current set (*=enabled):\n").is_empty());
    }

    #[test]
    fn scutil_s_status_is_its_first_word() {
        assert_eq!(nc_status("Connected\nExtended Status <dictionary> {\n}\n"), "Connected");
        assert_eq!(nc_status("Invalid: no service\n"), "Invalid");
        assert_eq!(nc_status(""), "");
    }

    #[test]
    fn a_refused_apple_event_says_where_to_allow_it() {
        let refused = "37:52: execution error: Not authorized to send Apple events to System Events. (-1743)";
        assert!(apple_event_error(refused).contains("Automation"));
        assert_eq!(apple_event_error("0:4: execution error: User canceled. (-128)"), "User canceled. (-128)");
        assert_eq!(apple_event_error(""), "osascript failed");
    }
}

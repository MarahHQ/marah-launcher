//! What the system's tools print, read into the system types. Pure functions over text, so the
//! tests run on recorded output anywhere.

use std::path::{Path, PathBuf};

use launcher_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection, InstallDisk, Installed, NetworkStatus, SystemDrive,
};

/// One line of `nmcli -t` output: fields are `:`-separated, with `\:` inside a field.
pub fn nm_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields.last_mut().expect("one field at least").extend(chars.next()),
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("one field at least").push(c),
        }
    }
    fields
}

/// `nmcli -t -f TYPE,STATE,CONNECTION device`: a cable is used before Wi-Fi.
pub fn network(text: &str) -> NetworkStatus {
    let rows: Vec<Vec<String>> = text.lines().map(nm_fields).collect();
    let connected = |kind: &str| rows.iter().find(|f| f.len() >= 3 && f[0] == kind && f[1] == "connected");
    if let Some(f) = connected("ethernet") {
        return NetworkStatus { connection: Connection::Wired, name: f[2].clone(), signal: None };
    }
    match connected("wifi") {
        Some(f) => NetworkStatus { connection: Connection::Wifi, name: f[2].clone(), signal: None },
        None => NetworkStatus::default(),
    }
}

/// A Wi-Fi network as `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list` prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wifi {
    pub ssid: String,
    pub signal: u8,
    pub secured: bool,
    pub in_use: bool,
}

/// Nearby Wi-Fi networks, the one in use first, then strongest first, one entry per name;
/// hidden networks (no name) are left out.
pub fn wifi(text: &str) -> Vec<Wifi> {
    let mut list: Vec<Wifi> = text
        .lines()
        .map(nm_fields)
        .filter(|f| f.len() >= 4 && !f[1].is_empty())
        .map(|f| Wifi {
            in_use: f[0] == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            secured: !f[3].is_empty() && f[3] != "--",
        })
        .collect();
    list.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    let mut seen = std::collections::HashSet::new();
    list.retain(|w| seen.insert(w.ssid.clone()));
    list
}

/// The names of the saved Wi-Fi connections, from `nmcli -t -f NAME,TYPE connection show`.
pub fn saved_wifi(text: &str) -> Vec<String> {
    text.lines().map(nm_fields).filter(|f| f.len() >= 2 && f[1] == "802-11-wireless").map(|f| f[0].clone()).collect()
}

/// The first line of a tool's error, without `Error: `.
pub fn error_line(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it failed");
    first.trim_start_matches("Error: ").to_owned()
}

/// `bluetoothctl devices` (and `devices Paired`, `devices Connected`): address and name.
/// Unnamed devices show their address again, with dashes; they aren't worth listing.
pub fn devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            (address.len() == 17 && !name.trim().is_empty() && name.replace('-', ":") != address)
                .then(|| (address.to_owned(), name.trim().to_owned()))
        })
        .collect()
}

/// Every device, from the three lists: all known, paired, connected. Paired first.
pub fn bluetooth(all: &str, paired: &str, connected: &str) -> Vec<BluetoothDevice> {
    let paired: Vec<String> = devices(paired).into_iter().map(|(a, _)| a).collect();
    let connected: Vec<String> = devices(connected).into_iter().map(|(a, _)| a).collect();
    let mut list: Vec<BluetoothDevice> = devices(all)
        .into_iter()
        .map(|(address, name)| BluetoothDevice {
            paired: paired.contains(&address),
            connected: connected.contains(&address),
            address,
            name,
        })
        .collect();
    list.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.name.cmp(&b.name)));
    list
}

/// `bluetoothctl show`: whether the adapter is on.
pub fn powered(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "Powered: yes")
}

/// `pactl -f json list sinks` and `pactl get-default-sink`: the outputs, named for the player.
pub fn outputs(json: &str, default: &str) -> Vec<AudioOutput> {
    let Ok(launcher_api::serde_json::Value::Array(sinks)) = launcher_api::serde_json::from_str(json) else {
        return Vec::new();
    };
    let default = default.trim();
    sinks
        .iter()
        .filter_map(|s| {
            let name = s["name"].as_str()?.to_owned();
            let description = output_name(s);
            let description = description.as_str();
            let port = s["active_port"].as_str().unwrap_or("");
            // "GA102 High Definition Audio Controller Digital Stereo (HDMI)" says less than this.
            let description = if port.starts_with("hdmi") || description.contains("HDMI") {
                "TV or monitor (HDMI)".to_owned()
            } else if port.contains("headphones") {
                "Headphones".to_owned()
            } else {
                description.replace(" Analog Stereo", "").replace(" Digital Stereo", "")
            };
            Some(AudioOutput { current: name == default, name, description })
        })
        .collect()
}

/// The name a sink's player sees, before the HDMI and headphones names: its description, else
/// its device's product name or nick, else its sink name cleaned ("raop_sink.Living-Room.local.…"
/// is "Living Room"), else "Unknown output". PipeWire writes "(null)" for a description it
/// doesn't have, as for an AirPlay speaker.
fn output_name(sink: &launcher_api::serde_json::Value) -> String {
    let readable = |v: &launcher_api::serde_json::Value| {
        v.as_str().map(str::trim).filter(|t| !t.is_empty() && *t != "(null)").map(str::to_owned)
    };
    let properties = &sink["properties"];
    let found = readable(&sink["description"])
        .or_else(|| readable(&properties["device.description"]))
        .or_else(|| readable(&properties["device.product.name"]))
        .or_else(|| readable(&properties["device.nick"]))
        .or_else(|| readable(&properties["node.nick"]));
    if let Some(found) = found {
        return found;
    }
    // "raop_sink.Living-Room.local.192.0.2.10.7000": the part after the kind, before the dots.
    let name = sink["name"].as_str().unwrap_or_default();
    let rest = name.split_once('.').map(|(_, rest)| rest).unwrap_or(name);
    let cleaned = rest.split('.').next().unwrap_or_default().replace(['-', '_'], " ");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().all(|c| c.is_ascii_digit()) {
        "Unknown output".into()
    } else {
        cleaned.to_owned()
    }
}

/// `pactl get-sink-volume @DEFAULT_SINK@` and `pactl get-sink-mute @DEFAULT_SINK@`.
pub fn level(volume: &str, mute: &str) -> AudioLevel {
    let percent = volume
        .split('/')
        .map(str::trim)
        .find_map(|part| part.strip_suffix('%').and_then(|n| n.trim().parse::<u32>().ok()))
        .unwrap_or(0);
    AudioLevel { volume: percent.min(100) as u8, muted: mute.trim() == "Mute: yes" }
}

/// The PC's own batteries under `root` (`/sys/class/power_supply`), as one: laptops with two
/// add up. A controller's or a mouse's battery (`scope=Device`) isn't the PC's.
pub fn battery(root: &Path) -> Option<Battery> {
    let read = |dir: &Path, file: &str| std::fs::read_to_string(dir.join(file)).ok().map(|s| s.trim().to_owned());
    let (mut now, mut full, mut percents) = (0u64, 0u64, Vec::new());
    let (mut charging, mut found) = (false, false);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for dir in entries {
        match read(&dir, "type").as_deref() {
            Some("Mains") | Some("USB") if read(&dir, "online").as_deref() == Some("1") => charging = true,
            Some("Battery") if read(&dir, "scope").as_deref() != Some("Device") => {
                // A removable battery taken out says present=0.
                if read(&dir, "present").as_deref() == Some("0") {
                    continue;
                }
                found = true;
                if matches!(read(&dir, "status").as_deref(), Some("Charging" | "Full" | "Not charging")) {
                    charging = true;
                }
                let num = |f: &str| read(&dir, f).and_then(|s| s.parse::<u64>().ok());
                match (num("energy_now").or(num("charge_now")), num("energy_full").or(num("charge_full"))) {
                    (Some(n), Some(f)) if f > 0 => {
                        now += n;
                        full += f;
                    }
                    _ => percents.extend(num("capacity")),
                }
            }
            _ => {}
        }
    }
    if !found {
        return None;
    }
    // From the energy left of the full charge, or else the average of the batteries' own figures.
    let percent = (now * 100)
        .checked_div(full)
        .or_else(|| percents.iter().sum::<u64>().checked_div(percents.len() as u64))?
        .min(100);
    Some(Battery { percent: percent as u8, charging })
}

#[cfg_attr(target_os = "linux", allow(dead_code))] // macOS reads it; the tests run everywhere.
/// macOS's `pmset -g batt`: `… -InternalBattery-0 (id=…) 76%; discharging; 4:12 remaining …`
/// (a tab before the percent).
pub fn pmset(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let percent = line.split(['\t', ' ', ';']).find_map(|w| w.strip_suffix('%')?.parse::<u8>().ok())?;
    let charging = line.contains("; charging") || line.contains("; charged") || line.contains("AC attached");
    Some(Battery { percent: percent.min(100), charging })
}

/// The installer's `--disks`: `disk PATH SIZE_BYTES WINDOWS(0/1) USB(0/1) MODEL…`.
pub fn disks(text: &str) -> Vec<InstallDisk> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(6, ' ');
            if f.next()? != "disk" {
                return None;
            }
            let path = PathBuf::from(f.next()?);
            let bytes = f.next()?.parse().ok()?;
            let windows = f.next()? == "1";
            let usb = f.next()? == "1";
            let model = f.next().unwrap_or("Disk").replace('_', " ").trim().to_owned();
            Some(InstallDisk { path, bytes, windows, usb, model })
        })
        .collect()
}

/// Why the installer's `--disks` failed: its last line that isn't blank and isn't a disk,
/// trimmed. Its errors and its disks come mixed, so the last line can be a disk.
pub fn disks_error(text: &str) -> Option<String> {
    let said = text.lines().map(str::trim).rfind(|l| !l.is_empty() && l.split(' ').next() != Some("disk"));
    said.map(str::to_owned)
}

/// One line the installer printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallLine {
    Progress(u8, String),
    Done(Installed),
    Failed(String),
}

/// `progress PERCENT TEXT`, `done EDITION [mok] [nvidia-later]` or `error TEXT`; anything else
/// is the installer's own chatter.
pub fn install_line(line: &str) -> Option<InstallLine> {
    let line = line.trim();
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    match word {
        "progress" => {
            let (percent, text) = rest.split_once(' ').unwrap_or((rest, ""));
            let percent = percent.parse::<f32>().ok()?.clamp(0.0, 100.0) as u8;
            Some(InstallLine::Progress(percent, text.to_owned()))
        }
        "done" => {
            let mut words = rest.split_whitespace();
            let edition = words.next().unwrap_or("minimal").to_owned();
            let flags: Vec<&str> = words.collect();
            Some(InstallLine::Done(Installed {
                edition,
                mok: flags.contains(&"mok"),
                nvidia_later: flags.contains(&"nvidia-later"),
            }))
        }
        "error" => Some(InstallLine::Failed(rest.to_owned())),
        _ => None,
    }
}

/// `rpm-ostree status --json`: the version a restart starts, when one is staged.
pub fn staged(json: &str) -> Option<Option<String>> {
    let v: launcher_api::serde_json::Value = launcher_api::serde_json::from_str(json).ok()?;
    let deployments = v["deployments"].as_array()?;
    let version = |d: &launcher_api::serde_json::Value| {
        d["version"].as_str().map(str::to_owned).unwrap_or_else(|| {
            let digest = d["container-image-reference-digest"].as_str().unwrap_or_default();
            digest.chars().rev().take(7).collect::<Vec<_>>().into_iter().rev().collect()
        })
    };
    deployments.iter().find(|d| d["booted"].as_bool() == Some(true))?;
    Some(deployments.iter().find(|d| d["staged"].as_bool() == Some(true)).map(version))
}

/// The folders a drive the launcher may unmount is mounted in: the ones UDisks mounts in
/// (`/run/media/<user>` on Fedora and Arch, `/media/<user>` on Debian and Ubuntu) and `/mnt`.
const DRIVE_FOLDERS: [&str; 3] = ["/run/media/", "/media/", "/mnt/"];

/// File systems that hold no files to mount: swap, and the parts of volumes, arrays and
/// encrypted drives.
const NOT_FILES: [&str; 6] = ["swap", "LVM2_member", "crypto_LUKS", "linux_raid_member", "zfs_member", "BitLocker"];

/// The partition types of the system's own parts, never a player's drive: the EFI system part,
/// Microsoft's reserved part, Windows' recovery part, and BIOS boot (GPT type GUIDs).
const SYSTEM_PARTS: [&str; 4] = [
    "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
    "e3c9e316-0b5c-4db8-817d-f92df00215ae",
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
    "21686148-6449-6e6f-744e-656564454649",
];

/// Transports of disks inside the PC: a removable flag on them doesn't make them a drive the
/// player takes out.
const INSIDE: [&str; 3] = ["sata", "nvme", "ata"];

/// `lsblk --json --list -b -o NAME,PKNAME,PATH,TYPE,SIZE,FSTYPE,PARTTYPE,LABEL,UUID,MOUNTPOINTS,FSAVAIL,TRAN,HOTPLUG,RM,MODEL`:
/// every drive with a file system to mount. A part of a disk takes its disk's model, and whether
/// the player can take it out, from the disk (`pkname` names it): its transport is USB, or it
/// is removable (RM) and not on a SATA, NVMe or ATA transport. A hot-plug SATA port alone doesn't
/// make a disk one to take out. Loop devices and RAM disks (zram) are left out.
///
/// `system` marks the system's own parts: a system part type (EFI, reserved, recovery, BIOS
/// boot), one mounted outside the drives' folders, and any part of the disk `/` or `/boot` is on
/// that the player didn't mount in a drives' folder.
pub fn drives(json: &str) -> Vec<SystemDrive> {
    use launcher_api::serde_json::Value;
    let Ok(v) = launcher_api::serde_json::from_str::<Value>(json) else { return Vec::new() };
    let rows: Vec<&Value> = v["blockdevices"].as_array().into_iter().flatten().collect();
    // Older lsblk prints numbers and flags as text.
    let number = |v: &Value| v.as_u64().or_else(|| v.as_str()?.parse().ok());
    let flag = |v: &Value| v.as_bool().unwrap_or_else(|| matches!(v.as_str(), Some("1" | "true")));
    let text = |v: &Value| v.as_str().unwrap_or_default().trim().to_owned();
    let mounts_of = |row: &Value| -> Vec<String> {
        row["mountpoints"].as_array().into_iter().flatten().filter_map(|m| m.as_str()).map(str::to_owned).collect()
    };
    // The disks `/` and `/boot` are on.
    let system_disks: Vec<String> = rows
        .iter()
        .filter(|r| mounts_of(r).iter().any(|m| m == "/" || m.starts_with("/boot")))
        .map(|r| text(&disk_of(&rows, r)["path"]))
        .collect();
    let mut out = Vec::new();
    for row in &rows {
        let path = text(&row["path"]);
        let disk = disk_of(&rows, row);
        if text(&row["type"]) == "loop" || text(&disk["type"]) == "loop" || path.starts_with("/dev/zram") {
            continue;
        }
        let filesystem = text(&row["fstype"]);
        if filesystem.is_empty() || NOT_FILES.contains(&filesystem.as_str()) {
            continue;
        }
        // A device may be mounted at more than one folder; an unmounted one may list `null`.
        let mounts: Vec<&str> =
            row["mountpoints"].as_array().into_iter().flatten().filter_map(|m| m.as_str()).collect();
        let ours = |m: &str| DRIVE_FOLDERS.iter().any(|f| m.starts_with(f));
        let shown = mounts.iter().find(|m| ours(m)).or(mounts.first());
        let transport = text(&disk["tran"]);
        let removable = flag(&disk["rm"]) && !INSIDE.contains(&transport.as_str());
        let disk_path = text(&disk["path"]);
        let on_system_disk = system_disks.contains(&disk_path);
        let mounted_by_player = !mounts.is_empty() && mounts.iter().all(|m| ours(m));
        let system = SYSTEM_PARTS.contains(&text(&row["parttype"]).to_lowercase().as_str())
            || !mounts.iter().all(|m| ours(m))
            || (on_system_disk && !mounted_by_player);
        out.push(SystemDrive {
            device: path,
            disk: disk_path,
            uuid: text(&row["uuid"]),
            label: text(&row["label"]),
            model: text(&disk["model"]),
            size: number(&row["size"]).unwrap_or(0),
            filesystem,
            usb: transport == "usb" || removable,
            mount: shown.map(PathBuf::from),
            free: number(&row["fsavail"]),
            system,
        });
    }
    out.sort_by(|a, b| a.device.cmp(&b.device));
    out
}

/// The whole disk an lsblk row is on: itself, or the row its `pkname` names, up to the top.
fn disk_of<'a>(
    rows: &[&'a launcher_api::serde_json::Value],
    row: &'a launcher_api::serde_json::Value,
) -> &'a launcher_api::serde_json::Value {
    let name = |r: &launcher_api::serde_json::Value| r["name"].as_str().map(str::to_owned);
    let mut row = row;
    // A part of a part (a volume on an encrypted part) is a few levels down; a loop in the rows
    // can't hold this up.
    for _ in 0..8 {
        let parent = row["pkname"].as_str();
        match rows.iter().find(|r| parent.is_some() && name(r).as_deref() == parent) {
            Some(up) => row = up,
            None => break,
        }
    }
    row
}

/// The folder in `udisksctl mount`'s reply: `Mounted /dev/sdb1 at /run/media/player/STICK`, with
/// or without a full stop at the end.
pub fn mounted_at(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("Mounted ")?;
        let (_, at) = rest.split_once(" at ")?;
        let at = at.strip_suffix('.').unwrap_or(at);
        Some(PathBuf::from(at)).filter(|p| p.is_absolute())
    })
}

/// What went wrong, from udisksctl's error line, for the player: `Error mounting /dev/sdb1:
/// GDBus.Error:org.freedesktop.UDisks2.Error.Failed: Wrong fs type` says `Wrong fs type`. A line
/// without a D-Bus error name stays as it is.
pub fn udisks_error(line: &str) -> String {
    let line = line.trim();
    match line.split_once("GDBus.Error:") {
        Some((_, rest)) => rest.split_once(": ").map(|(_, why)| why).unwrap_or(rest).trim().to_owned(),
        None if line.is_empty() => "it failed".into(),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_networkmanager_s_devices_a_cable_first() {
        let wifi = network("ethernet:unavailable:\nwifi:connected:Home\\: 5G\nloopback:connected (externally):lo\n");
        assert_eq!((wifi.connection, wifi.name.as_str()), (Connection::Wifi, "Home: 5G"));
        let cable = network("ethernet:connected:Wired connection 1\nwifi:connected:Home\n");
        assert_eq!((cable.connection, cable.name.as_str()), (Connection::Wired, "Wired connection 1"));
        assert_eq!(network("wifi:disconnected:\n"), NetworkStatus::default());
    }

    #[test]
    fn reads_wifi_networks_one_per_name_in_use_and_strongest_first() {
        let list = wifi(
            ":Cafe:40:\n*:Home\\: 5G:78:WPA2\n:Neighbour:90:WPA1 WPA2\n:Neighbour:30:WPA2\n::70:WPA2\n:Old:20:WEP\n",
        );
        let seen: Vec<_> = list.iter().map(|w| (w.ssid.as_str(), w.signal, w.secured, w.in_use)).collect();
        assert_eq!(
            seen,
            [
                ("Home: 5G", 78, true, true),
                ("Neighbour", 90, true, false),
                ("Cafe", 40, false, false),
                ("Old", 20, true, false)
            ]
        );
        assert_eq!(saved_wifi("Home\\: 5G:802-11-wireless\nWired connection 1:802-3-ethernet\n"), ["Home: 5G"]);
    }

    #[test]
    fn reads_bluetooth_devices_and_skips_unnamed_ones() {
        let all = "Device 70:5A:6F:6D:EF:70 EarFun Wave Pro\nDevice A0:5A:5C:11:22:33 DualSense Wireless Controller\n\
                   Device 4C:11:22:33:44:55 4C-11-22-33-44-55\n";
        let list = bluetooth(all, "Device A0:5A:5C:11:22:33 DualSense Wireless Controller\n", "");
        let seen: Vec<_> = list.iter().map(|d| (d.name.as_str(), d.paired, d.connected)).collect();
        assert_eq!(seen, [("DualSense Wireless Controller", true, false), ("EarFun Wave Pro", false, false)]);
        assert!(powered("Controller 00:1A:7D:DA:71:13 (public)\n\tPowered: yes\n\tDiscoverable: no\n"));
        assert!(!powered("\tPowered: no\n"));
    }

    #[test]
    fn reads_sound_outputs_named_for_the_player() {
        let json = r#"[{"name":"alsa_output.usb-Shure.analog-stereo","description":"Shure MV7 Analog Stereo","active_port":"analog-output"},
            {"name":"alsa_output.pci.hdmi-stereo","description":"GA102 High Definition Audio Controller Digital Stereo (HDMI)","active_port":"hdmi-output-0"},
            {"name":"bluez_output.70_5A.1","description":"EarFun Wave Pro","active_port":"headset-output"}]"#;
        let list = outputs(json, "alsa_output.pci.hdmi-stereo\n");
        let seen: Vec<_> = list.iter().map(|o| (o.description.as_str(), o.current)).collect();
        assert_eq!(seen, [("Shure MV7", false), ("TV or monitor (HDMI)", true), ("EarFun Wave Pro", false)]);
        let volume = "Volume: front-left: 26214 /  40% / -23.87 dB,   front-right: 26214 /  40% / -23.87 dB\n";
        assert_eq!(level(volume, "Mute: no\n"), AudioLevel { volume: 40, muted: false });
        assert_eq!(
            level("Volume: mono: 98304 / 150% / 10.57 dB", "Mute: yes"),
            AudioLevel { volume: 100, muted: true }
        );
    }

    fn supply(root: &Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
    }

    #[test]
    fn reads_the_pc_s_batteries_but_not_a_controller_s() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(battery(root.path()), None, "a desktop PC");
        supply(root.path(), "AC", &[("type", "Mains"), ("online", "0")]);
        supply(
            root.path(),
            "BAT0",
            &[("type", "Battery"), ("status", "Discharging"), ("energy_now", "30"), ("energy_full", "100")],
        );
        supply(
            root.path(),
            "BAT1",
            &[("type", "Battery"), ("status", "Discharging"), ("energy_now", "10"), ("energy_full", "100")],
        );
        supply(root.path(), "ps-controller-battery", &[("type", "Battery"), ("scope", "Device"), ("capacity", "5")]);
        assert_eq!(battery(root.path()), Some(Battery { percent: 20, charging: false }));
        supply(root.path(), "AC", &[("type", "Mains"), ("online", "1")]);
        assert_eq!(battery(root.path()), Some(Battery { percent: 20, charging: true }));
        let mac = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t76%; discharging; 4:12 remaining present: true\n";
        assert_eq!(pmset(mac), Some(Battery { percent: 76, charging: false }));
        assert_eq!(pmset("Now drawing from 'AC Power'\n"), None, "a Mac without a battery");
    }

    #[test]
    fn reads_the_installer_s_disks_and_lines() {
        let list = disks(
            "disk /dev/nvme0n1 512110190592 1 0 Samsung_SSD 970 EVO\ndisk /dev/sdb 2000398934016 0 1 CT2000\nnoise\n",
        );
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].model.as_str(), list[0].bytes, list[0].windows, list[0].usb),
            ("Samsung SSD 970 EVO", 512110190592, true, false)
        );
        assert!(list[1].usb);
        assert_eq!(
            install_line("progress 35 Copying Marah OS"),
            Some(InstallLine::Progress(35, "Copying Marah OS".into()))
        );
        let done = |edition: &str, mok, nvidia_later| {
            Some(InstallLine::Done(Installed { edition: edition.into(), mok, nvidia_later }))
        };
        assert_eq!(install_line("done minimal-nvidia mok"), done("minimal-nvidia", true, false));
        assert_eq!(install_line("done minimal nvidia-later"), done("minimal", false, true));
        assert_eq!(
            install_line("error Installing on /dev/sda failed."),
            Some(InstallLine::Failed("Installing on /dev/sda failed.".into()))
        );
        assert_eq!(install_line("Deploying container image"), None);
    }

    #[test]
    fn reads_a_staged_system_update() {
        let json = r#"{"deployments":[{"booted":false,"staged":true,"version":"44.20261008"},{"booted":true,"version":"44.20261007"}]}"#;
        assert_eq!(staged(json), Some(Some("44.20261008".into())));
        assert_eq!(staged(r#"{"deployments":[{"booted":true,"version":"44.1"}]}"#), Some(None));
        assert_eq!(staged("not json"), None);
    }

    /// Recorded on a Fedora 45 desktop (util-linux 2.42) with one NVMe disk: no USB drive.
    const LSBLK_NVME: &str = include_str!("fixtures/lsblk-nvme.json");
    /// Derived from the recording above, not recorded: a USB stick (`/dev/sda`, its exFAT part
    /// mounted where UDisks mounts on Fedora) and an internal NTFS part that isn't mounted,
    /// written as older lsblk writes values (text numbers, `null` mount folders).
    const LSBLK_USB: &str = include_str!("fixtures/lsblk-usb-derived.json");

    /// Recorded on a Fedora 45 desktop (PipeWire): an AirPlay speaker whose description is
    /// "(null)", and the PC's own sound. The speaker's host name and address are replaced.
    const PACTL_NULL: &str = include_str!("fixtures/pactl-sinks-null.json");

    #[test]
    fn an_output_without_a_description_has_a_readable_name_never_null() {
        let list = outputs(PACTL_NULL, "alsa_output.pci-0000_c4_00.6.analog-stereo");
        let names: Vec<(&str, &str, bool)> =
            list.iter().map(|o| (o.name.as_str(), o.description.as_str(), o.current)).collect();
        assert_eq!(
            names,
            [
                ("alsa_output.pci-0000_c4_00.6.analog-stereo", "Ryzen HD Audio Controller", true),
                ("raop_sink.Living-Room.local.192.0.2.10.7000", "Living Room", false),
            ]
        );
        let unnamed = r#"[{"name":"1234","description":"(null)","properties":{}}]"#;
        assert_eq!(outputs(unnamed, "")[0].description, "Unknown output");
    }

    #[test]
    fn reads_the_drives_of_a_desktop_with_one_disk() {
        let list = drives(LSBLK_NVME);
        let seen: Vec<_> = list
            .iter()
            .map(|d| (d.device.as_str(), d.filesystem.as_str(), d.mount.as_deref().and_then(|m| m.to_str()), d.system))
            .collect();
        assert_eq!(
            seen,
            [
                ("/dev/nvme0n1p1", "vfat", Some("/boot/efi"), true),
                ("/dev/nvme0n1p2", "ext4", Some("/"), true),
                ("/dev/nvme0n1p3", "ext4", Some("/mnt/Media"), false),
            ],
            "the swap on zram is left out"
        );
        let media = &list[2];
        assert_eq!(media.disk, "/dev/nvme0n1");
        assert_eq!(media.model, "CT2000P310SSD8", "the disk's model");
        assert_eq!(media.uuid, "5c5610e8-88ce-47e1-a610-ece27a9df67d");
        assert_eq!((media.size, media.free), (1_672_906_539_008, Some(766_699_134_976)), "as recorded");
        assert!(!media.usb);
    }

    #[test]
    fn a_usb_stick_s_part_is_usb_and_an_unmounted_part_has_no_folder() {
        let list = drives(LSBLK_USB);
        let stick = list.iter().find(|d| d.device == "/dev/sda1").unwrap();
        assert!(stick.usb && !stick.system);
        assert_eq!(
            (stick.disk.as_str(), stick.label.as_str(), stick.filesystem.as_str()),
            ("/dev/sda", "STICK", "exfat")
        );
        assert_eq!(stick.mount.as_deref(), Some(Path::new("/run/media/player/STICK")));
        assert_eq!((stick.size, stick.free), (62_008_590_336, Some(40_000_000_000)));
        let card = list.iter().find(|d| d.device == "/dev/mmcblk0p1").unwrap();
        assert!(card.usb && !card.system, "a card in a reader with no transport can be taken out");
        assert!(drives("not json").is_empty());
    }

    /// The part of an internal disk on a hot-plug SATA port is not one the player takes out:
    /// Eject would power the disk off.
    #[test]
    fn a_sata_disk_on_a_hot_plug_port_is_internal() {
        let list = drives(LSBLK_USB);
        let games = list.iter().find(|d| d.device == "/dev/sdb1").unwrap();
        assert_eq!((games.usb, games.system, games.mount.as_ref()), (false, false, None), "a drive to mount");
        let media = list.iter().find(|d| d.device == "/dev/nvme0n1p3").unwrap();
        assert!(!media.usb && !media.system, "the data part the player mounted under /mnt stays");
    }

    /// The system's own parts are never a player's drive: the EFI part (its real type GUID), a
    /// Windows recovery part, the reserved part (no file system), and any part of the system's
    /// disk the player didn't mount in a drives' folder.
    #[test]
    fn the_system_s_parts_are_system_drives() {
        let list = drives(LSBLK_USB);
        let system = |device: &str| list.iter().find(|d| d.device == device).map(|d| d.system);
        assert_eq!(system("/dev/nvme0n1p1"), Some(true), "EFI");
        assert_eq!(system("/dev/nvme0n1p2"), Some(true), "/");
        assert_eq!(system("/dev/nvme0n1p4"), Some(true), "unmounted, on the system's disk");
        assert_eq!(system("/dev/nvme0n1p5"), Some(true), "Windows recovery");
        assert_eq!(system("/dev/sdb2"), None, "the reserved part has no file system: not listed");
        let efi = r#"{"blockdevices":[{"name":"sdc","path":"/dev/sdc","type":"disk","tran":"sata"},
            {"name":"sdc1","pkname":"sdc","path":"/dev/sdc1","type":"part","fstype":"vfat",
             "parttype":"c12a7328-f81f-11d2-ba4b-00a0c93ec93b","mountpoints":[null]}]}"#;
        assert!(drives(efi)[0].system, "an EFI part on another disk, not mounted");
    }

    #[test]
    fn reads_where_udisks_mounted_a_drive() {
        // The reply's words are the format in udisksctl 2.11.2's binary ("Mounted %s at %s").
        assert_eq!(
            mounted_at("Mounted /dev/sda1 at /run/media/player/STICK\n"),
            Some(PathBuf::from("/run/media/player/STICK"))
        );
        assert_eq!(
            mounted_at("Mounted /dev/sda1 at /media/player/MY STICK.\n"),
            Some(PathBuf::from("/media/player/MY STICK"))
        );
        assert_eq!(mounted_at("Error mounting /dev/sda1: it failed"), None);
    }

    #[test]
    fn says_why_udisks_refused_in_the_player_s_words() {
        // Recorded: udisksctl 2.11.2 asked to mount a device that isn't there.
        let recorded = "Error looking up object for device /dev/marah-does-not-exist";
        assert_eq!(udisks_error(recorded), recorded);
        // The format of udisksctl 2.11.2's binary ("Error mounting %s: %s"), with a D-Bus error.
        assert_eq!(
            udisks_error("Error mounting /dev/sda1: GDBus.Error:org.freedesktop.UDisks2.Error.Failed: Wrong fs type"),
            "Wrong fs type"
        );
        assert_eq!(udisks_error(""), "it failed");
    }
}

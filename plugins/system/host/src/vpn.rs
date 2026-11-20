//! The VPN area's shared parts, for "Download only through a VPN": what each OS module reads
//! of the VPNs.
//!
//! - Linux: a VPN is a WireGuard or OpenVPN interface in `/sys/class/net`, or another layer-3
//!   tunnel. Tunnels that don't hide the player's address, such as Tailscale or Docker's, are
//!   skipped. "Connected" means up and with an address. NetworkManager (`nmcli`) adds a VPN
//!   from a provider's file, and connects and disconnects the VPNs it knows.
//! - macOS: the system keeps `utun` interfaces up for its own services with no VPN, so the
//!   adapter can't tell a VPN. It lists every interface that is up and has an address, and
//!   the player picks the one the VPN app made. There is no import: the VPN's own app
//!   connects it.
//! - Windows: nothing. librqbit 9.0.1 can't bind its connections to an interface there.

use std::collections::BTreeSet;
use std::path::Path;

use launcher_api::Vpn;

/// Interfaces that look like tunnels but don't hide the player's address from the internet.
const NOT_PRIVACY: [&str; 9] = ["tailscale", "zt", "docker", "virbr", "veth", "br-", "vnet", "lxc", "waydroid"];

/// A VPN interface found in `/sys/class/net`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub kind: &'static str,
    /// Up and with an address.
    pub connected: bool,
}

/// The VPN interfaces under `root` (`/sys/class/net`), connected or not. `addressed` holds the
/// interfaces with an address.
pub fn list_in(root: &Path, addressed: &BTreeSet<String>) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return out };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" || NOT_PRIVACY.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let dir = entry.path();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
        let kind = if read("uevent").lines().any(|l| l.trim() == "DEVTYPE=wireguard") {
            "WireGuard"
        } else if dir.join("tun_flags").exists() {
            "OpenVPN"
        } else if read("type").trim() == "65534" {
            // A layer-3 tunnel with no hardware address.
            "VPN"
        } else {
            continue;
        };
        let flags = u32::from_str_radix(read("flags").trim().trim_start_matches("0x"), 16).unwrap_or(0);
        let up = flags & 0x1 != 0;
        out.push(Found { connected: up && addressed.contains(&name), name, kind });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A VPN connection NetworkManager knows (its WireGuard or VPN plugin type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    name: String,
    wireguard: bool,
    active: bool,
}

/// `nmcli -t -f NAME,TYPE,ACTIVE connection show`, the VPNs only.
pub fn parse_saved(text: &str) -> Vec<Saved> {
    text.lines()
        .filter_map(|line| {
            // A NAME can hold escaped colons; TYPE and ACTIVE can't.
            let mut parts = line.rsplitn(3, ':');
            let active = parts.next()? == "yes";
            let kind = parts.next()?;
            let name = parts.next()?.replace("\\:", ":");
            matches!(kind, "wireguard" | "vpn").then_some(Saved { name, wireguard: kind == "wireguard", active })
        })
        .collect()
}

/// The interfaces, and the connections NetworkManager knows, as one list. A connection with
/// the name of an interface is that interface. One without has no interface to bind to.
pub fn merge(found: Vec<Found>, saved: Vec<Saved>) -> Vec<Vpn> {
    let mut vpns: Vec<Vpn> = found
        .into_iter()
        .map(|f| Vpn {
            interface: if f.connected { f.name.clone() } else { String::new() },
            switchable: saved.iter().any(|s| s.name == f.name),
            kind: f.kind.into(),
            connected: f.connected,
            name: f.name,
        })
        .collect();
    for s in saved {
        if !vpns.iter().any(|v| v.name == s.name) {
            let kind = if s.wireguard { "WireGuard" } else { "VPN" };
            vpns.push(Vpn {
                name: s.name,
                kind: kind.into(),
                connected: s.active,
                interface: String::new(),
                switchable: true,
            });
        }
    }
    vpns.sort_by(|a, b| a.name.cmp(&b.name));
    vpns
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh `/sys/class/net` stand-in for test `name`.
    fn fixtures(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marah-launcher-vpn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn iface(root: &Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (f, v) in files {
            std::fs::write(dir.join(f), v).unwrap();
        }
    }

    #[test]
    fn finds_vpn_tunnels_but_not_networks_that_don_t_hide_the_address() {
        let root = fixtures("tunnels");
        let r = root.as_path();
        iface(r, "lo", &[("type", "772"), ("flags", "0x9")]);
        iface(r, "enp3s0", &[("type", "1"), ("flags", "0x1003")]);
        iface(r, "wg-se", &[("type", "65534"), ("flags", "0x91"), ("uevent", "DEVTYPE=wireguard\nINTERFACE=wg-se\n")]);
        iface(r, "tun0", &[("type", "65534"), ("flags", "0x1091"), ("tun_flags", "0x1001")]);
        // Down.
        iface(r, "proton0", &[("type", "65534"), ("flags", "0x90")]);
        iface(r, "tailscale0", &[("type", "65534"), ("flags", "0x91"), ("tun_flags", "0x1001")]);
        iface(r, "docker0", &[("type", "1"), ("flags", "0x1003")]);
        let addressed: BTreeSet<String> = ["wg-se", "tun0", "enp3s0", "tailscale0"].map(String::from).into();

        let found = list_in(r, &addressed);
        let _ = std::fs::remove_dir_all(r);

        let summary: Vec<_> = found.iter().map(|v| (v.name.as_str(), v.kind, v.connected)).collect();
        assert_eq!(summary, [("proton0", "VPN", false), ("tun0", "OpenVPN", true), ("wg-se", "WireGuard", true)]);
    }

    #[test]
    fn up_but_with_no_address_is_not_connected() {
        let root = fixtures("no-address");
        iface(&root, "wg0", &[("type", "65534"), ("flags", "0x91"), ("uevent", "DEVTYPE=wireguard\n")]);
        let found = list_in(&root, &BTreeSet::new());
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(found, [Found { name: "wg0".into(), kind: "WireGuard", connected: false }]);
    }

    #[test]
    fn reads_networkmanager_s_vpn_connections() {
        let text = "Wired connection 1:802-3-ethernet:yes\nwg-se:wireguard:yes\nWork\\: office:vpn:no\nhome wifi:802-11-wireless:no\n";
        assert_eq!(
            parse_saved(text),
            [
                Saved { name: "wg-se".into(), wireguard: true, active: true },
                Saved { name: "Work: office".into(), wireguard: false, active: false },
            ]
        );
    }

    #[test]
    fn a_saved_connection_with_an_interface_s_name_is_that_interface() {
        let found = vec![
            Found { name: "wg-se".into(), kind: "WireGuard", connected: true },
            Found { name: "tun0".into(), kind: "OpenVPN", connected: true },
        ];
        let saved = vec![
            Saved { name: "wg-se".into(), wireguard: true, active: true },
            Saved { name: "Work".into(), wireguard: false, active: false },
        ];
        let vpns = merge(found, saved);
        let summary: Vec<_> =
            vpns.iter().map(|v| (v.name.as_str(), v.connected, v.interface.as_str(), v.switchable)).collect();
        assert_eq!(summary, [("Work", false, "", true), ("tun0", true, "tun0", false), ("wg-se", true, "wg-se", true)]);
    }
}

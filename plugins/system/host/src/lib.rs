//! The built-in system plugin: the machine's own settings through the system's own tools. Each
//! area it manages ([`SystemArea`]) is one adapter over the same interface ([`AreaSettings`]).
//!
//! Every tool runs through one runner ([`tools`]), with a time limit on each call. Each OS has
//! one module, chosen here and nowhere else, with no `#[cfg]` inside it. Its `AREAS` table lists
//! the areas it manages and the tools each runs: `init` and the manifest read it.
//!
//! - Linux (`linux.rs`): network and Wi-Fi, VPNs, Bluetooth, sound, restart and shut down, the
//!   battery, drives (UDisks), and on Marah OS its updates and its USB stick's installer.
//! - macOS (`macos.rs`): VPNs (every network with an address) and the battery (`pmset`).
//! - Windows (`windows.rs`): the battery isn't read yet; VPNs say why they can't work there.

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), path = "windows.rs")]
mod os;
#[cfg(unix)]
mod unix;

// Mostly the Linux tools' output; macOS reads only pmset's. The tests run everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod parse;
// macOS runs only pmset, and Windows no tool yet. The tests run everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod tools;
// Linux reads the VPN interfaces. The tests run everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod vpn;

use std::collections::BTreeMap;
use std::sync::Arc;

use launcher_api::{
    AreaSettings, CodePlugin, Plugin, PluginContext, PluginFactory, PluginManifest, PluginResult, Provided, SystemArea,
    inventory,
};

const MANIFEST: &str = include_str!("../plugin.toml");

pub struct HostSystem;

impl Plugin for HostSystem {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid");
        manifest.capabilities.retain(|c| os::AREAS.iter().any(|(area, _)| area.capability() == *c));
        manifest
    }
}

#[launcher_api::async_trait]
impl CodePlugin for HostSystem {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let runner = Arc::new(tools::Runner::new(ctx.programs().clone()));
        let areas: BTreeMap<SystemArea, Arc<dyn AreaSettings>> =
            os::AREAS.iter().map(|(area, _)| (*area, os::adapter(*area, runner.clone()))).collect();
        Ok(Provided { areas, ..Provided::default() })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(HostSystem)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_lets_every_tool_in_this_os_s_table_run() {
        let manifest = HostSystem.manifest();
        for (area, tools) in os::AREAS {
            assert!(manifest.capabilities.contains(&area.capability()), "{area:?} isn't a capability");
            for tool in *tools {
                let entry = format!("system:{}", tool.program);
                assert!(manifest.permissions.process.contains(&entry), "{entry} isn't in permissions.process");
            }
        }
        assert_eq!(manifest.capabilities.len(), os::AREAS.len());
    }
}

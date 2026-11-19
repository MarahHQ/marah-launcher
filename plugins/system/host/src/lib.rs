//! The built-in system plugin: the machine's own settings through the system's own tools. Each
//! area it manages ([`SystemArea`]) is one adapter over the same interface ([`AreaSettings`]).
//!
//! - Linux ([`linux`]): network and Wi-Fi, VPNs, Bluetooth, sound, restart and shut down, the
//!   battery, drives (UDisks), and on Marah OS its updates and its USB stick's installer.
//! - macOS: VPNs (every network with an address) and the battery (`pmset`).
//! - Windows: the battery isn't read yet; VPNs say why they can't work there.

#[cfg(target_os = "linux")]
mod linux;
// Mostly the Linux tools' output; macOS reads only pmset's. The tests run everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod parse;
mod vpn;

use std::collections::BTreeMap;
use std::sync::Arc;

use launcher_api::{
    AreaSettings, Capability, CodePlugin, Plugin, PluginContext, PluginFactory, PluginManifest, PluginResult, Provided,
    SystemArea, inventory,
};

const MANIFEST: &str = include_str!("../plugin.toml");

pub struct HostSystem;

impl Plugin for HostSystem {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid");
        if !cfg!(target_os = "linux") {
            manifest.capabilities.retain(|c| matches!(c, Capability::Vpn | Capability::Power));
        }
        manifest
    }
}

#[launcher_api::async_trait]
impl CodePlugin for HostSystem {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let mut areas: BTreeMap<SystemArea, Arc<dyn AreaSettings>> = BTreeMap::new();
        areas.insert(SystemArea::Vpn, Arc::new(vpn::Vpns::new(ctx.programs().clone())));
        #[cfg(target_os = "linux")]
        {
            let linux = Arc::new(linux::Linux::new(ctx.programs().clone()));
            for area in SystemArea::ALL.into_iter().filter(|a| *a != SystemArea::Vpn) {
                areas.insert(area, Arc::new(linux::LinuxArea { area, linux: linux.clone() }));
            }
        }
        #[cfg(not(target_os = "linux"))]
        areas.insert(SystemArea::Power, Arc::new(other::Power(ctx.programs().clone())));
        Ok(Provided { areas, ..Provided::default() })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(HostSystem)) }

/// macOS and Windows: the battery, and no restart or shut down (the system's own menu does
/// that, and Marah Launcher isn't the whole screen there).
#[cfg(not(target_os = "linux"))]
mod other {
    use std::sync::Arc;

    use launcher_api::{
        AreaAction, AreaData, AreaSettings, AreaState, Battery, Location, PluginError, PluginResult, Programs,
        SystemJob,
    };

    pub struct Power(pub Arc<dyn Programs>);

    impl Power {
        async fn battery(&self) -> PluginResult<Option<Battery>> {
            if !cfg!(target_os = "macos") {
                return Ok(None);
            }
            let args = ["-g".to_owned(), "batt".to_owned()];
            let out = self.0.output(&Location::System("pmset".into()), &args, launcher_api::MAX_PROGRAM_TIME).await?;
            Ok(crate::parse::pmset(&String::from_utf8_lossy(&out)))
        }
    }

    #[launcher_api::async_trait]
    impl AreaSettings for Power {
        async fn state(&self) -> PluginResult<AreaState> {
            let refused = Some("Restart or shut down from the system's own menu".into());
            // A battery that isn't read leaves the refusal standing.
            Ok(AreaState::Ready(AreaData::Power { battery: self.battery().await.ok().flatten(), refused }))
        }

        async fn act(&self, _action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
            Err(PluginError::Unsupported("restart or shut down from the system's own menu".into()))
        }
    }
}

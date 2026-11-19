//! macOS: the battery (`pmset`), and every network with an address for the VPNs. No restart or
//! shut down: the system's own menu does that, and the launcher isn't the whole screen there.

use std::sync::Arc;

use launcher_api::{
    AreaAction, AreaData, AreaSettings, AreaState, Battery, PluginError, PluginResult, SystemArea, SystemJob, Vpn,
};

use crate::tools::{Runner, Tool};

const PMSET: Tool = Tool::new("pmset", "pmset");

/// Each area macOS manages, and the tools it runs.
pub const AREAS: &[(SystemArea, &[Tool])] = &[(SystemArea::Vpn, &[]), (SystemArea::Power, &[PMSET])];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    match area {
        SystemArea::Vpn => Arc::new(Vpns),
        _ => Arc::new(Power(tools)),
    }
}

struct Power(Arc<Runner>);

impl Power {
    async fn battery(&self) -> PluginResult<Option<Battery>> {
        Ok(crate::parse::pmset(&self.0.read(PMSET, &["-g", "batt"]).await?))
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

/// macOS can't tell VPNs from other networks: every network with an address, and the player
/// picks the one the VPN uses.
struct Vpns;

#[launcher_api::async_trait]
impl AreaSettings for Vpns {
    async fn state(&self) -> PluginResult<AreaState> {
        let vpns = crate::unix::addressed()
            .into_iter()
            .map(|name| Vpn {
                kind: "Network".into(),
                connected: true,
                interface: name.clone(),
                switchable: false,
                name,
            })
            .collect();
        Ok(AreaState::Ready(AreaData::Vpn { finds_vpns: false, can_import: false, vpns }))
    }

    async fn act(&self, _action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        Err(PluginError::Unsupported("this system can't change a VPN: use the VPN's own app".into()))
    }
}

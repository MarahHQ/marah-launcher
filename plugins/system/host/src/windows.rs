//! Windows: the battery isn't read yet, and VPNs say why they can't work there. No restart or
//! shut down: the system's own menu does that, and the launcher isn't the whole screen there.

use std::sync::Arc;

use launcher_api::{AreaAction, AreaData, AreaSettings, AreaState, PluginError, PluginResult, SystemArea, SystemJob};

use crate::tools::{Runner, Tool};

/// Each area Windows manages, and the tools it runs.
pub const AREAS: &[(SystemArea, &[Tool])] = &[(SystemArea::Vpn, &[]), (SystemArea::Power, &[])];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, _tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    match area {
        SystemArea::Vpn => Arc::new(Vpns),
        _ => Arc::new(Power),
    }
}

struct Power;

#[launcher_api::async_trait]
impl AreaSettings for Power {
    async fn state(&self) -> PluginResult<AreaState> {
        let refused = Some("Restart or shut down from the system's own menu".into());
        Ok(AreaState::Ready(AreaData::Power { battery: None, refused }))
    }

    async fn act(&self, _action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        Err(PluginError::Unsupported("restart or shut down from the system's own menu".into()))
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

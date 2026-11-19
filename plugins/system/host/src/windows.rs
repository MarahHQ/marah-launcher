//! Windows: the battery (`GetSystemPowerStatus`), and restart and shut down (`shutdown`). VPNs
//! say why they can't work there.
//!
//! Windows' own calls block: each runs on a blocking thread with COM set up for it, for at
//! most a time limit ([`blocking`]). What Windows' calls return becomes the areas' data in
//! `winmap.rs`, which the tests run on every OS.

use std::sync::Arc;
use std::time::Duration;

use launcher_api::{AreaAction, AreaData, AreaSettings, AreaState, PluginError, PluginResult, SystemArea, SystemJob};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::tools::{Runner, Tool};
use crate::winmap;

/// Windows' own restart and shut down.
const SHUTDOWN: Tool = Tool::new("shutdown", "Windows' shutdown");

/// Each area Windows manages, and the tools it runs.
pub const AREAS: &[(SystemArea, &[Tool])] = &[(SystemArea::Vpn, &[]), (SystemArea::Power, &[SHUTDOWN])];

/// The adapter for `area`, one of [`AREAS`].
pub fn adapter(area: SystemArea, tools: Arc<Runner>) -> Arc<dyn AreaSettings> {
    match area {
        SystemArea::Vpn => Arc::new(Vpns),
        _ => Arc::new(Power(tools)),
    }
}

/// How long a read or a quick change through Windows' own calls may take.
const QUICK: Duration = crate::tools::QUICK;

/// COM, set up on this thread for as long as it lives: the multithreaded kind, so a wait on a
/// WinRT call doesn't block a window's thread.
struct Com {
    /// COM was set up here, and is closed again on drop. Not when the thread had another kind.
    owned: bool,
}

impl Com {
    fn init() -> Self {
        #[allow(unsafe_code)] // COM's own setup, closed in drop when it worked.
        // SAFETY: no reserved pointer; the guard closes it on this same thread.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Com { owned: result.is_ok() }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            #[allow(unsafe_code)] // Closes the setup `init` made on this thread.
            // SAFETY: `CoInitializeEx` worked on this thread, and each such call is closed once.
            unsafe {
                CoUninitialize()
            };
        }
    }
}

/// Runs `work` on a blocking thread with COM, for at most `limit`. At the limit the area stops
/// waiting, but the thread runs on until Windows answers: a Windows call can't be stopped.
async fn blocking<T, F>(limit: Duration, work: F) -> PluginResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> PluginResult<T> + Send + 'static,
{
    let task = tokio::task::spawn_blocking(move || {
        let _com = Com::init();
        work()
    });
    match tokio::time::timeout(limit, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(PluginError::Other(format!("Windows' call stopped: {e}"))),
        Err(_) => Err(PluginError::Other("Windows took too long to answer".into())),
    }
}

/// A failed Windows call, in plain words: what failed, and Windows' own message.
fn failed(what: &'static str) -> impl Fn(windows::core::Error) -> PluginError {
    move |e| PluginError::Other(format!("{what}: {}", e.message()))
}

struct Power(Arc<Runner>);

impl Power {
    async fn battery(&self) -> PluginResult<Option<launcher_api::Battery>> {
        blocking(QUICK, || {
            let mut status = SYSTEM_POWER_STATUS::default();
            #[allow(unsafe_code)] // One call that fills a struct this closure owns.
            // SAFETY: `status` is a writable SYSTEM_POWER_STATUS.
            unsafe { GetSystemPowerStatus(&mut status) }.map_err(failed("The battery wasn't read"))?;
            Ok(winmap::battery(status.BatteryFlag, status.BatteryLifePercent))
        })
        .await
    }
}

#[launcher_api::async_trait]
impl AreaSettings for Power {
    async fn state(&self) -> PluginResult<AreaState> {
        let (refused, battery) = tokio::join!(self.0.missing(&[SHUTDOWN]), self.battery());
        Ok(AreaState::Ready(AreaData::Power { battery: battery.ok().flatten(), refused }))
    }

    async fn act(&self, action: AreaAction, _job: Arc<dyn SystemJob>) -> PluginResult<()> {
        let how = match action {
            AreaAction::Restart => "/r",
            AreaAction::ShutDown => "/s",
            other => return Err(PluginError::Unsupported(format!("the power area can't run {other:?}"))),
        };
        self.0.act(SHUTDOWN, &[how, "/t", "0"]).await.map(|_| ())
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

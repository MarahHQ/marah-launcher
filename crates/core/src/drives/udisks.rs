//! The adapter over a system plugin's drives area ([`SystemArea::Drives`]): on Linux, UDisks.
//! Its actions run in the background and end through their job; this adapter waits for that end.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use launcher_api::{AreaAction, AreaData, AreaSettings, AreaState, PluginId, SystemDrive, SystemJob};

use super::{Adapter, Drive};
use crate::{CoreError, CoreResult, Kernel};

pub(crate) struct PluginDrives {
    pub kernel: Kernel,
    pub plugin: PluginId,
    pub settings: Arc<dyn AreaSettings>,
}

/// The longest a mount or an unmount may take, and an eject, which unmounts every part of a
/// disk first. A drive that doesn't answer must not hold its panel forever.
const MOUNT_LIMIT: Duration = Duration::from_secs(30);
const EJECT_LIMIT: Duration = Duration::from_secs(60);

impl PluginDrives {
    /// Starts `action`, which reports through a job over `job`, and waits for its end, for at
    /// most `limit`: then the job fails, with a plain reason.
    async fn act(&self, action: AreaAction, job: Arc<dyn SystemJob>, limit: Duration) -> CoreResult<String> {
        let watched = Watched::over(job);
        self.kernel.guard(&self.plugin, "drives.act", self.settings.act(action, watched.clone())).await?;
        match self.kernel.ports().clock.timeout(limit, watched.wait()).await {
            Some(ended) => ended.map_err(CoreError::Invalid),
            None => {
                let why = format!("The drive didn't answer in {} seconds", limit.as_secs());
                watched.failed(&why);
                Err(CoreError::Invalid(why))
            }
        }
    }
}

/// The drives the system didn't mount for itself, as the player sees them: those the player may
/// mount and unmount.
pub(crate) fn drives(found: Vec<SystemDrive>) -> Vec<Drive> {
    found.into_iter().filter(|d| !d.system).map(drive).collect()
}

/// The drive the player sees: by its file system's UUID, named by its label, else its model.
pub(crate) fn drive(found: SystemDrive) -> Drive {
    let id = if found.uuid.is_empty() { found.device.clone() } else { found.uuid };
    let label = [&found.label, &found.model]
        .into_iter()
        .find(|l| !l.is_empty())
        .cloned()
        .unwrap_or_else(|| found.device.trim_start_matches("/dev/").to_owned());
    Drive {
        id,
        device: found.device,
        disk: found.disk,
        label,
        size: Some(found.size).filter(|s| *s > 0),
        free: found.free,
        filesystem: found.filesystem,
        usb: found.usb,
        mount: found.mount,
    }
}

#[async_trait]
impl Adapter for PluginDrives {
    fn kind(&self) -> &'static str {
        "plugin"
    }

    async fn list(&self) -> CoreResult<Vec<Drive>> {
        // A list it doesn't give is a read that failed, not a list with no drive.
        match self.kernel.guard(&self.plugin, "drives.state", self.settings.state()).await? {
            AreaState::Ready(AreaData::Drives { drives: found }) => Ok(drives(found)),
            AreaState::Unsupported(why) => Err(CoreError::Invalid(why)),
            other => Err(CoreError::Invalid(format!("the system plugin gave no drives: {other:?}"))),
        }
    }

    async fn mount(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<PathBuf> {
        let action = AreaAction::Mount { device: drive.device.clone() };
        Ok(PathBuf::from(self.act(action, job, MOUNT_LIMIT).await?))
    }

    async fn unmount(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<()> {
        self.act(AreaAction::Unmount { device: drive.device.clone() }, job, MOUNT_LIMIT).await.map(|_| ())
    }

    async fn eject(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<()> {
        self.act(AreaAction::Eject { disk: drive.disk.clone() }, job, EJECT_LIMIT).await.map(|_| ())
    }
}

/// A job the player sees, which also keeps how it ended for the one waiting on it.
struct Watched {
    job: Arc<dyn SystemJob>,
    end: Mutex<Option<Result<String, String>>>,
    ended: tokio::sync::Notify,
}

impl Watched {
    fn over(job: Arc<dyn SystemJob>) -> Arc<Watched> {
        Arc::new(Watched { job, end: Mutex::new(None), ended: tokio::sync::Notify::new() })
    }

    fn end(&self, outcome: Result<String, String>) {
        self.end.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert(outcome);
        self.ended.notify_one();
    }

    async fn wait(&self) -> Result<String, String> {
        loop {
            if let Some(outcome) = self.end.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
                return outcome;
            }
            self.ended.notified().await;
        }
    }
}

impl SystemJob for Watched {
    fn progress(&self, percent: Option<u8>, text: &str) {
        self.job.progress(percent, text);
    }

    fn done(&self, text: &str) {
        self.job.done(text);
        self.end(Ok(text.to_owned()));
    }

    fn failed(&self, reason: &str) {
        self.job.failed(reason);
        self.end(Err(reason.to_owned()));
    }

    fn cancelled(&self) -> bool {
        self.job.cancelled()
    }
}

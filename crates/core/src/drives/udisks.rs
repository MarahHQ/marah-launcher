//! The adapter over a system plugin's `drives` capability ([`DriveSettings`]): on Linux, UDisks.
//! Its actions run in the background and end through their job; this adapter waits for that end.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use launcher_api::{DriveSettings, PluginId, SystemDrive, SystemJob};

use super::{Adapter, Drive};
use crate::{CoreError, CoreResult, Kernel};

pub(crate) struct PluginDrives {
    pub kernel: Kernel,
    pub plugin: PluginId,
    pub settings: Arc<dyn DriveSettings>,
}

impl PluginDrives {
    /// Starts an action, which reports through `watched`, and waits for its end.
    async fn act(
        &self,
        method: &'static str,
        watched: &Watched,
        start: impl Future<Output = launcher_api::PluginResult<()>>,
    ) -> CoreResult<String> {
        self.kernel.guard(&self.plugin, method, start).await?;
        watched.wait().await.map_err(CoreError::Invalid)
    }
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

    /// The drives the system didn't mount for itself: those the player may mount and unmount.
    async fn list(&self) -> CoreResult<Vec<Drive>> {
        let found = self.kernel.guard(&self.plugin, "drives.drives", self.settings.drives()).await?;
        Ok(found.into_iter().filter(|d| !d.system).map(drive).collect())
    }

    async fn mount(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<PathBuf> {
        let watched = Watched::over(job);
        let start = self.settings.mount(&drive.device, watched.clone());
        Ok(PathBuf::from(self.act("drives.mount", &watched, start).await?))
    }

    async fn unmount(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<()> {
        let watched = Watched::over(job);
        let start = self.settings.unmount(&drive.device, watched.clone());
        self.act("drives.unmount", &watched, start).await.map(|_| ())
    }

    async fn eject(&self, drive: &Drive, job: Arc<dyn SystemJob>) -> CoreResult<()> {
        let watched = Watched::over(job);
        let start = self.settings.eject(&drive.disk, watched.clone());
        self.act("drives.eject", &watched, start).await.map(|_| ())
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

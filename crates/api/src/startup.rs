//! The optional startup capability: work a plugin does on its own when the launcher starts,
//! such as a firmware update. The boot screen shows it as a step, and waits until it is over.
//!
//! The core asks for [`StartupWork::status`] after it reads the plugin's slots at the start,
//! then again every moment until the work is over. A plugin that has no work this run says
//! `None`, and the boot screen doesn't wait for it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Where the work is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupState {
    /// It runs: the boot screen waits.
    Working,
    /// It ended well.
    Done,
    /// It ended with an error, in `detail`. The boot screen doesn't wait for it.
    Failed,
}

/// The work as the boot screen shows it: a step of its checklist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartupStep {
    /// What it is, such as "Switch firmware".
    pub label: String,
    /// What it does now, or how it ended, such as "Downloading 120.0 MB of 400.0 MB".
    pub detail: String,
    /// From 0 to 1, when the plugin knows how far it is.
    pub progress: Option<f32>,
    pub state: StartupState,
}

#[async_trait]
pub trait StartupWork: Send + Sync {
    /// The work the plugin does at this start, as it is now; `None` when it has none.
    async fn status(&self) -> Option<StartupStep>;
}

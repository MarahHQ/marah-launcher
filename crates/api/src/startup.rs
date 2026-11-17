//! The optional startup capability: work a plugin does on its own when the launcher starts,
//! such as a firmware update. The boot screen shows it as a step, and waits until it is over.
//!
//! The core asks for [`StartupWork::status`] after it reads the plugin's slots at the start,
//! then again every moment until the work is over. A plugin that has no work this run says
//! `None`, and the boot screen doesn't wait for it.
//!
//! Work that downloads something the player didn't ask for asks first: the plugin offers a
//! [`StartupQuestion`], and acts on [`StartupWork::answer`].

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

/// A question for the player before the work runs, such as "Install Switch firmware 22.5.0?".
/// The launcher shows it over the boot screen, with two buttons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartupQuestion {
    pub title: String,
    /// A sentence or two: what the work does, and what it downloads.
    pub text: String,
    /// The button that says yes, such as "Install".
    pub yes: String,
    /// The button that says no, such as "Not now".
    pub no: String,
}

#[async_trait]
pub trait StartupWork: Send + Sync {
    /// The work the plugin does at this start, as it is now; `None` when it has none.
    async fn status(&self) -> Option<StartupStep>;

    /// A question the player answers before the work runs (API 0.3.2). While it is asked,
    /// `status` says `None`, and the boot screen waits for the answer.
    async fn question(&self) -> Option<StartupQuestion> {
        None
    }

    /// The player's answer to [`StartupWork::question`]: `true` for its yes button.
    async fn answer(&self, _yes: bool) {}
}

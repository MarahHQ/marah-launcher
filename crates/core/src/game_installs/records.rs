//! The installs' saved records: the work folder of each running install, under
//! `app.game_installs.<job id>`, so the next start removes one a crash left. A work folder out
//! of reach at the start stays saved, under `left-<n>`, for a later start, with the path of its
//! games folder: the player may change the list of games folders in the meantime.

use std::path::PathBuf;
use std::sync::Arc;

use launcher_api::{HostPath, JobId, PluginId};
use serde_json::Value;

use crate::config::ConfigService;

/// Where a running install's work folder is saved, by job id.
const RECORDS: [&str; 2] = ["app", "game_installs"];

/// An install the last run left.
#[derive(Clone, Debug)]
pub(crate) struct Left {
    pub plugin: PluginId,
    pub work: HostPath,
    /// The games folder `work` is in, as the player's list names it. Only a kept record has
    /// it; a record a crash left has only `work.root`, its place in the list.
    pub games: Option<PathBuf>,
}

/// The subsystem's part of the config.
pub(crate) struct Records {
    config: Arc<ConfigService>,
}

impl Records {
    pub fn new(config: Arc<ConfigService>) -> Self {
        Self { config }
    }

    /// Saves the work folder of install `id`.
    pub fn save(&self, id: JobId, plugin: &PluginId, work: &HostPath) {
        let record = serde_json::json!({ "plugin": plugin, "work": work });
        if let Err(e) = self.config.set(&[RECORDS[0], RECORDS[1], &id.0.to_string()], record) {
            tracing::warn!(error = %e, "could not save the install's record");
        }
    }

    pub fn forget(&self, id: JobId) {
        if let Err(e) = self.config.remove(&[RECORDS[0], RECORDS[1], &id.0.to_string()]) {
            tracing::warn!(error = %e, "could not forget the install's record");
        }
    }

    /// The work folders the last run left, and no record after. A record that doesn't read is
    /// left out.
    pub fn take_all(&self) -> Vec<Left> {
        let records = self.config.get(&RECORDS);
        let Some(map) = records.as_object() else { return Vec::new() };
        let left = map
            .values()
            .filter_map(|record| {
                let plugin = record.get("plugin").and_then(Value::as_str).and_then(|p| PluginId::new(p).ok())?;
                let work = serde_json::from_value::<HostPath>(record.get("work")?.clone()).ok()?;
                let games = record.get("games").and_then(Value::as_str).map(PathBuf::from);
                Some(Left { plugin, work, games })
            })
            .collect();
        if let Err(e) = self.config.set(&RECORDS, serde_json::json!({})) {
            tracing::warn!(error = %e, "could not clear the install records");
        }
        left
    }

    /// Saves again the work folders of the last run that [`Records::take_all`] took and the
    /// start couldn't reach. Their keys are no job id, so this run's installs leave them be.
    pub fn keep(&self, left: &[Left]) {
        for (n, Left { plugin, work, games }) in left.iter().enumerate() {
            let mut record = serde_json::json!({ "plugin": plugin, "work": work });
            if let Some(games) = games {
                record["games"] = Value::String(games.display().to_string());
            }
            if let Err(e) = self.config.set(&[RECORDS[0], RECORDS[1], &format!("left-{n}")], record) {
                tracing::warn!(error = %e, "could not keep the install's record");
            }
        }
    }
}

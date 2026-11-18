//! The optional compatibility capability: how far a game gets in the emulator.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::PluginResult;

/// How far a game gets in the emulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompatLevel {
    Unknown,
    Nothing,
    Boots,
    Menus,
    Ingame,
    Playable,
}

impl CompatLevel {
    /// The levels a player can give a game after playing it, best first.
    pub const RESULTS: [CompatLevel; 4] =
        [CompatLevel::Ingame, CompatLevel::Menus, CompatLevel::Boots, CompatLevel::Nothing];

    /// Its name, as it reads and writes with `serde`: `unknown`, `nothing`, `boots`, `menus`,
    /// `ingame` or `playable`.
    pub fn as_str(self) -> &'static str {
        match self {
            CompatLevel::Unknown => "unknown",
            CompatLevel::Nothing => "nothing",
            CompatLevel::Boots => "boots",
            CompatLevel::Menus => "menus",
            CompatLevel::Ingame => "ingame",
            CompatLevel::Playable => "playable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatInfo {
    pub level: CompatLevel,
    #[serde(default)]
    pub notes: Option<String>,
    /// The system `level` was tested on, for example `macOS`. `None` when no one says.
    #[serde(default)]
    pub source: Option<String>,
    /// The community list's page, for the player to read or report.
    #[serde(default)]
    pub page: Option<String>,
    /// When `level` was last tested, as `YYYY-MM-DD`. `None` when the list doesn't say.
    #[serde(default)]
    pub tested: Option<String>,
}

/// The player's own result for a game, for a community list's report form (API 0.3.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultReport {
    /// One of [`CompatLevel::RESULTS`].
    pub level: Option<CompatLevel>,
    /// The emulator build the game ran on; empty when the host doesn't know it.
    pub build: String,
    /// The system it ran on: `Linux`, `macOS` or `Windows`.
    pub system: String,
    /// The processor, the graphics and the memory, such as `AMD Ryzen 7 5800X`,
    /// `AMD Radeon RX 6800` and `32 GB`. Empty when the host doesn't know.
    pub cpu: String,
    pub gpu: String,
    pub memory: String,
    /// The end of the emulator's log of the run the player rated, with the home folder, the
    /// user and computer names, IP addresses and emails taken out. Empty when there is none.
    pub log: String,
}

/// A community list's form for one report, filled in (API 0.3.3). The host shows it to the
/// player, and on their yes opens it in the browser, where they submit it. The host adds the
/// title and the fields to `form` as query parameters, as GitHub's issue forms read them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportForm {
    /// The list's name, for the player, such as `shadPS4 compatibility list`.
    pub list: String,
    /// The form's link with its own query, such as
    /// `https://github.com/owner/repo/issues/new?template=report.yml`. The plugin's
    /// `permissions.links` must allow it.
    pub form: String,
    pub title: String,
    /// The form's fields, in the form's order.
    pub fields: Vec<ReportField>,
    /// What the player still does on the form, such as its checklist. Empty for nothing.
    #[serde(default)]
    pub note: String,
}

/// One field of a [`ReportForm`]: its id in the form, its label, and its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportField {
    pub id: String,
    pub label: String,
    pub value: String,
}

#[async_trait]
pub trait Compatibility: Send + Sync {
    async fn status(&self, game: &str) -> PluginResult<CompatInfo>;

    /// The list's report form for `game`, filled in with the player's `report`, or `None` when
    /// the list takes no reports this way (API 0.3.3).
    async fn report(&self, _game: &str, _report: &ResultReport) -> PluginResult<Option<ReportForm>> {
        Ok(None)
    }

    /// The list's result for the game of `title_id`, such as `CUSA03173`, whether the player has
    /// the game or not; `None` when the list has no result for it (API 0.3.4). The host asks it
    /// for each Catalog offer, so it must be cheap: a look-up in the list held in memory, not a
    /// network call each time.
    async fn result_for_title(&self, _title_id: &str) -> PluginResult<Option<CompatInfo>> {
        Ok(None)
    }
}

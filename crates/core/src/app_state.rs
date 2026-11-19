//! The state no subsystem owns: where the player is and what they look at (the navigation
//! stack with each screen's selection, the Library's and the Catalog's grids), the questions the player
//! answers ("How far did it get?"), the Settings screen's panels as the plugins last gave them,
//! the machine's state, the stores' installs, and the theme on screen.
//!
//! The core's flows write it under one lock, and the view reads it. The core takes it only
//! while it holds no subsystem's lock.

use std::collections::HashMap;

use launcher_api::{GameKey, PluginId};
use serde_json::Value;

use crate::app::Grid;
use crate::grid_query::{GridQuery, ShownGrid};
use crate::stack::Stack;

/// A plugin's settings panel on the Settings screen: its slot item, and the schema and the
/// values of the settings scope it names.
#[derive(Clone, Debug)]
pub(crate) struct SettingsPanel {
    pub plugin: PluginId,
    pub item: launcher_api::SlotItem,
    pub schema: Value,
    pub values: Value,
}

/// The state no subsystem owns (see the module's text). One lock holds it all; the view reads
/// a copy.
#[derive(Clone)]
pub(crate) struct AppState {
    /// The screens the player came through, each with the game selected on it.
    pub stack: Stack,
    pub settings: std::sync::Arc<Vec<SettingsPanel>>,
    pub theme: Option<PluginId>,
    /// The Library's search, filters and sort.
    pub library_view: GridQuery,
    /// The Catalog's.
    pub catalog_view: GridQuery,
    /// A game that just closed, to ask the player how far it got, and the emulator build it
    /// ran on.
    pub result_prompt: Option<(GameKey, Option<String>)>,
    /// "Send ratings", while it is open.
    pub ratings: Option<crate::ratings::SendRatings>,
    /// Log out, Restart or Shut down is closing what runs first: the step it is at.
    pub closing: Option<crate::closing::Step>,
    /// Log out closed what ran: the launcher can go.
    pub log_out_ready: bool,
    /// This PC, once the check ran.
    pub hardware: Option<crate::Specs>,
    /// The machine's settings, from the system plugins.
    pub system: crate::system::SystemState,
    /// The start's installs, which the boot screen waits for.
    pub startup: crate::startup::Startup,
    /// The installs stores are doing (Steam's), by the catalog plugin, as last read.
    pub store_installs: Vec<(PluginId, launcher_api::StoreInstall)>,
    /// Until when the stores' installs are read often: a minute after Marah Launcher asked a store to
    /// install, as the store writes its install down a moment later.
    pub store_watch_until: Option<std::time::Instant>,
    /// The Catalog's grid as kept when the Catalog left the top of the stack, or when the
    /// catalogs were read while it was off screen.
    pub catalog_grid: ShownGrid,
    /// How many catalog reads have ended: the count a kept Catalog grid is built over.
    pub catalog_reads: u64,
    /// Each grid's columns.
    pub grid_columns: HashMap<Grid, usize>,
    /// Where the catalogs' downloads go, and the free space there.
    pub disks: crate::disks::Disks,
}

impl AppState {
    /// Home, with nothing selected, and `theme` on screen.
    pub fn new(theme: Option<PluginId>) -> Self {
        Self {
            stack: Stack::default(),
            settings: Default::default(),
            theme,
            library_view: GridQuery::library(),
            catalog_view: GridQuery::catalog(),
            result_prompt: None,
            ratings: None,
            closing: None,
            log_out_ready: false,
            hardware: None,
            system: Default::default(),
            startup: Default::default(),
            store_installs: Vec::new(),
            store_watch_until: None,
            catalog_grid: ShownGrid::default(),
            catalog_reads: 0,
            grid_columns: HashMap::new(),
            disks: Default::default(),
        }
    }

    /// The search, filters and sort of `grid`.
    pub fn grid_view(&mut self, grid: Grid) -> &mut GridQuery {
        match grid {
            Grid::Library => &mut self.library_view,
            Grid::Catalog => &mut self.catalog_view,
        }
    }
}

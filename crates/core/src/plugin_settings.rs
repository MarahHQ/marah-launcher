//! The plugins' settings and actions: the Settings screen's panels, a settings scope set whole
//! or one field at a time, a plugin's slot actions, turning a plugin on or off, the player's
//! folders and the theme's options.

use launcher_api::{Capability, ConfigScope, GameKey, PluginId, PluginKind, SlotId, SlotRequest};
use serde_json::Value;

use crate::app::{Core, HOST_INSTALL_GAME, Route};
use crate::app_state::SettingsPanel;
use crate::changes::Area;
use crate::error::require;
use crate::{CoreError, CoreResult};

pub(crate) async fn refresh_settings(core: &Core) {
    let kernel = core.kernel();
    let mut panels = Vec::new();
    for (plugin, provided) in core.providers(Capability::Slots).await {
        let slots = provided.slots.clone().expect("declared and checked at init");
        let request = SlotRequest { slot: SlotId::SettingsPanel, game: None };
        let Ok(items) = kernel.guard(&plugin, "slots.items", slots.items(&request)).await else { continue };
        let items =
            crate::order::sort_by_order(items.into_iter().map(|i| (i.id.clone(), i.order.clone(), i)).collect());
        for item in items {
            let scope = match &item.content {
                launcher_api::SlotContent::SettingsPanel { scope, .. } => scope.clone(),
                _ => ConfigScope::Emulator,
            };
            let schema = match &provided.configurator {
                Some(c) => kernel.guard(&plugin, "configurator.schema", c.schema(&scope)).await.unwrap_or(Value::Null),
                None => Value::Null,
            };
            let values = core.config_value(&plugin, &scope).await.unwrap_or(Value::Null);
            panels.push(SettingsPanel { plugin: plugin.clone(), item, schema, values });
        }
    }
    core.state().settings = std::sync::Arc::new(panels);
}

pub(crate) async fn set_config(core: &Core, plugin: &PluginId, scope: ConfigScope, value: Value) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    if let Some(configurator) = &provided.configurator {
        kernel.guard(plugin, "configurator.validate", configurator.validate(&scope, &value)).await?;
    }
    kernel
        .config()
        .set(&["plugins", plugin.as_str(), &crate::ConfigService::scope_key(&scope)], value)
        .map_err(|e| CoreError::Io(e.to_string()))?;
    if !core.state().settings.is_empty() {
        refresh_settings(core).await;
    }
    // A new catalog address: the catalogs are read again.
    if provided.releases.is_some() {
        crate::catalog_reads::refresh_releases(core).await;
    }
    // A game's own settings: its panel shows the new value.
    if let ConfigScope::Game(local) = &scope {
        let game = GameKey::new(plugin.clone(), local.clone());
        let provided = kernel.provided(plugin).await?;
        let panel = crate::library_reads::game_settings(core, plugin, &provided, &game).await;
        core.library().set_game_settings(&game, panel);
    }
    core.changed(Area::Plugins);
    Ok(())
}

pub(crate) async fn run_action(
    core: &Core,
    plugin: &PluginId,
    action: &str,
    game: Option<GameKey>,
    args: Value,
) -> CoreResult<()> {
    // `host:` actions are the host's: a plugin's slot asks the host to do something.
    if action == launcher_api::HOST_OPEN_SETTINGS {
        return crate::navigation::navigate(core, Route::Settings).await;
    }
    if action == launcher_api::HOST_OPEN_EMULATOR {
        return crate::launch::open_emulator(core, plugin).await;
    }
    if action == HOST_INSTALL_GAME {
        let source = serde_json::from_value(args)
            .map_err(|e| CoreError::Invalid(format!("{HOST_INSTALL_GAME} needs a folder path: {e}")))?;
        return crate::release_downloads::install_game(core, plugin, source, None).await;
    }
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let slots = require(plugin, Capability::Slots, provided.slots)?;
    let local = game.as_ref().filter(|g| &g.plugin == plugin).map(|g| g.local.clone());
    kernel.guard(plugin, "slots.run_action", slots.run_action(action, local.as_deref(), &args)).await?;
    // The action may have changed what the plugin offers and shows.
    if provided.releases.is_some() {
        crate::catalog_reads::refresh_releases(core).await;
    }
    // Its panel shows on its page and in the Emulators screen.
    let route = core.state().stack.route().clone();
    if route == Route::Plugin(plugin.clone()) || route == Route::Emulators {
        crate::emulator_builds::refresh_emulator(core, plugin).await;
    }
    if let Some(game) = game.filter(|g| &g.plugin == plugin)
        && core.state().stack.route() == &Route::GameDetail(game.clone())
    {
        crate::navigation::open_game(core, game).await?;
    }
    core.changed(Area::Plugins);
    Ok(())
}

pub(crate) async fn set_config_field(
    core: &Core,
    plugin: &PluginId,
    scope: ConfigScope,
    key: String,
    text: String,
) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let schema = match &provided.configurator {
        Some(c) => kernel.guard(plugin, "configurator.schema", c.schema(&scope)).await?,
        None => Value::Null,
    };
    let value = crate::settings_schema::typed(&schema, &key, text)?;
    let mut current = core.config_value(plugin, &scope).await?;
    if !current.is_object() {
        current = Value::Object(serde_json::Map::new());
    }
    current.as_object_mut().expect("an object").insert(key, value);
    set_config(core, plugin, scope, current).await
}

pub(crate) async fn set_plugin_enabled(core: &Core, plugin: &PluginId, enabled: bool) -> CoreResult<()> {
    let kernel = core.kernel();
    let manifest = kernel.manifest(plugin)?;
    if manifest.kind == PluginKind::Theme {
        return Err(CoreError::Invalid(format!("{} is a theme: switch themes instead", manifest.name)));
    }
    if !enabled && core.builds().busy(plugin) {
        return Err(CoreError::Invalid(format!("{} is busy: turn it off when it is done", manifest.name)));
    }
    // The list of exceptions: plugins turned off, or for one made for other systems, on.
    let key = if kernel.elsewhere(plugin) { "enabled_plugins" } else { "disabled_plugins" };
    let mut listed: Vec<String> = kernel
        .config()
        .get(&["app", key])
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    listed.retain(|id| id != plugin.as_str());
    if enabled == kernel.elsewhere(plugin) {
        listed.push(plugin.to_string());
    }
    kernel.config().set(&["app", key], serde_json::json!(listed)).map_err(|e| CoreError::Io(e.to_string()))?;
    // The last catalog turned off: the Catalog goes, and Back passes where it was. Its
    // offers go too, so turning one on reads them again.
    if !crate::navigation::catalog_available(kernel) {
        core.library().clear_catalogs();
        let mut state = core.state();
        state.stack.close(&Route::Catalog);
        state.catalog_grid.clear();
    }
    if enabled && kernel.of_kind(PluginKind::Emulator).contains(plugin) {
        crate::emulator_builds::refresh_emulator(core, plugin).await;
    }
    // An emulator on or off: its console's catalog comes or goes, and with it the catalog's
    // settings for that console.
    if manifest.kind == PluginKind::Emulator && crate::navigation::catalog_available(kernel) {
        crate::catalog_reads::refresh_releases(core).await;
    }
    refresh_settings(core).await;
    crate::library_reads::refresh_library(core).await?;
    core.changed(Area::Plugins);
    Ok(())
}

/// The shared folder of the games: Settings → Games → Game folders.
pub(crate) const GAMES: &str = "games";

pub(crate) async fn set_folders(core: &Core, name: &str, paths: &[std::path::PathBuf]) -> CoreResult<()> {
    if !launcher_api::PathGrant::valid_name(name) {
        return Err(CoreError::Invalid(format!("{name:?} is not a folder name")));
    }
    // `~` and `~/…` are the player's home folder, as a shell reads them.
    let home = core.kernel().host().home.clone();
    let paths: Vec<std::path::PathBuf> = paths
        .iter()
        .map(|p| match (p.strip_prefix("~"), &home) {
            (Ok(rest), Some(home)) if rest.as_os_str().is_empty() => home.clone(),
            (Ok(rest), Some(home)) => home.join(rest),
            _ => p.clone(),
        })
        .collect();
    if let Some(relative) = paths.iter().find(|p| !p.is_absolute()) {
        return Err(CoreError::Invalid(format!("{} is not a full path", relative.display())));
    }
    let config = core.kernel().config();
    let before = folder_roots(core, name);
    let saved = if paths.is_empty() {
        config.remove(&["app", "folders", name])
    } else {
        let list: Vec<Value> = paths.iter().map(|p| Value::from(p.to_string_lossy().into_owned())).collect();
        config.set(&["app", "folders", name], Value::Array(list))
    };
    saved.map_err(|e| CoreError::Io(e.to_string()))?;
    core.library().move_folders(name, &before, &folder_roots(core, name));
    crate::drives::remember_folders(core, &paths);
    crate::disks::refresh(core).await;
    // The games come from a game folder that came, and go with one that went, with no Rescan.
    // Before the first read of the library, that read takes them.
    let came: Vec<std::path::PathBuf> = paths.iter().filter(|p| !before.contains(p)).cloned().collect();
    let changed = !came.is_empty() || before.iter().any(|p| !paths.contains(p));
    if name == GAMES && changed && core.library().booted() {
        crate::library_reads::scan(core, came).await;
    }
    // Settings shows the folders from the files read ahead: they are read before it hears.
    crate::snapshot::refresh_files(core).await;
    core.changed(Area::Settings);
    Ok(())
}

/// The folders the named host folder means now: the player's, or the default ones, as the first
/// plugin that asks for it sees them. Empty when no plugin asks for it.
pub(crate) fn folder_roots(core: &Core, name: &str) -> Vec<std::path::PathBuf> {
    let kernel = core.kernel();
    let host = kernel.host();
    kernel
        .order()
        .iter()
        .filter_map(|id| kernel.manifest(id).ok())
        .find_map(|m| m.permissions.folder(name).cloned())
        .map(|grant| crate::folders::roots(kernel.config(), host.home.as_deref(), host.platform, &grant))
        .unwrap_or_default()
}

/// Adds `path` to the folders `name` means. One that is there already isn't added again.
pub(crate) async fn add_folder(core: &Core, name: &str, path: &std::path::Path) -> CoreResult<()> {
    let mut paths = folder_roots(core, name);
    if !paths.iter().any(|p| p == path) {
        paths.push(path.to_path_buf());
    }
    set_folders(core, name, &paths).await
}

/// Takes `path` out of the folders `name` means. Its files stay.
pub(crate) async fn remove_folder(core: &Core, name: &str, path: &std::path::Path) -> CoreResult<()> {
    let mut paths = folder_roots(core, name);
    let before = paths.len();
    paths.retain(|p| p != path);
    if paths.len() == before {
        return Err(CoreError::Invalid(format!("{} isn't one of the folders", path.display())));
    }
    set_folders(core, name, &paths).await
}

pub(crate) fn set_theme_option(core: &Core, key: String, value: String) -> CoreResult<()> {
    let theme = core.state().theme.clone().ok_or_else(|| CoreError::Invalid("no theme is active".into()))?;
    let mut options = core.theme_options(&theme);
    if !options.is_object() {
        options = Value::Object(serde_json::Map::new());
    }
    options.as_object_mut().expect("an object").insert(key, Value::String(value));
    core.inner
        .kernel
        .config()
        .set(&["app", "theme-options", theme.as_str()], options)
        .map_err(|e| CoreError::Io(e.to_string()))?;
    core.changed(Area::Settings);
    Ok(())
}

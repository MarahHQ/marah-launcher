//! A plugin that took the place of others (`[[replaces]]`): at each start, the core moves what
//! it keeps of each replaced plugin to the new one. A game's key `emu-nes:Contra` becomes
//! `emu-retro:NES/Contra`, the replaced plugin's console in front of its id.
//!
//! It moves only what is still under a replaced plugin's id, so a second run does nothing, and
//! the settings of an old backup, put back at the start after its restore, move too. It moves:
//!
//! - the records of each game, by its key: play time, art, folders, results, reports and
//!   whether it is hidden;
//! - the plugin's settings (`plugins.<id>`): each game's own, by its new id, and the plugin's,
//!   when the new plugin has none yet: the first replaced plugin's win;
//! - a replaced plugin the player turned off: its console is off in the new plugin;
//! - the game installs and downloads it started, which go on in the new plugin.
//!
//! The install of a build a replaced plugin left unfinished is dropped: the shared folder's
//! builds are the new plugin's.

use launcher_api::{PluginId, PluginManifest};
use serde_json::{Map, Value};

use crate::config::ConfigService;

/// The records under `app` kept by game key: an object by key, or a list of keys.
const BY_GAME: [&str; 8] = [
    "playtime",
    "hidden_games",
    "game_art",
    "game_folders",
    "results",
    "result_builds",
    "result_skipped",
    "result_reports",
];

/// Moves what the config keeps of each plugin that one of `plugins` replaces.
pub(crate) fn move_replaced<'a>(
    config: &ConfigService,
    plugins: impl IntoIterator<Item = (&'a PluginId, &'a PluginManifest)>,
) {
    for (plugin, manifest) in plugins {
        for replaced in &manifest.replaces {
            let moved = move_one(config, plugin, &replaced.plugin, replaced.console.as_str());
            match moved {
                Ok(true) => tracing::info!(from = %replaced.plugin, to = %plugin, "moved a replaced plugin's settings"),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, from = %replaced.plugin, "a replaced plugin's settings didn't move")
                }
            }
        }
    }
}

/// Moves `old`'s records to `new`, its games under `console`. `true` when anything moved.
fn move_one(config: &ConfigService, new: &PluginId, old: &PluginId, console: &str) -> std::io::Result<bool> {
    let prefix = format!("{old}:");
    let rekey = |key: &str| key.strip_prefix(&prefix).map(|local| format!("{new}:{console}/{local}"));
    let mut moved = false;
    for name in BY_GAME {
        let value = config.get(&["app", name]);
        if let Some(changed) = rekeyed(&value, &rekey) {
            config.set(&["app", name], changed)?;
            moved = true;
        }
    }
    moved |= move_settings(config, new, old, console)?;
    for name in ["disabled_plugins", "enabled_plugins"] {
        let list = config.get(&["app", name]);
        let Some(ids) = list.as_array() else { continue };
        if !ids.iter().any(|id| id.as_str() == Some(old.as_str())) {
            continue;
        }
        if name == "disabled_plugins" {
            config.set(&["app", "consoles", new.as_str(), console], Value::Bool(false))?;
        }
        let kept: Vec<Value> = ids.iter().filter(|id| id.as_str() != Some(old.as_str())).cloned().collect();
        config.set(&["app", name], Value::Array(kept))?;
        moved = true;
    }
    if !config.get(&["app", "installs", old.as_str()]).is_null() {
        config.remove(&["app", "installs", old.as_str()])?;
        moved = true;
    }
    for name in ["game_installs", "torrents"] {
        let mut records = config.get(&["app", name]);
        let Some(map) = records.as_object_mut() else { continue };
        let mut changed = false;
        for record in map.values_mut().filter_map(Value::as_object_mut) {
            for field in ["plugin", "emulator"] {
                if record.get(field).and_then(Value::as_str) == Some(old.as_str()) {
                    record.insert(field.into(), Value::String(new.to_string()));
                    // A download's folders are its console's.
                    if field == "emulator"
                        && record.get("console").and_then(Value::as_str).unwrap_or_default().is_empty()
                    {
                        record.insert("console".into(), Value::String(console.to_owned()));
                    }
                    changed = true;
                }
            }
        }
        if changed {
            config.set(&["app", name], records)?;
            moved = true;
        }
    }
    Ok(moved)
}

/// `value` with each game key `rekey` changes, as an object's keys or a list's items. `None`
/// when no key changes.
fn rekeyed(value: &Value, rekey: &dyn Fn(&str) -> Option<String>) -> Option<Value> {
    match value {
        Value::Object(map) => {
            if !map.keys().any(|k| rekey(k).is_some()) {
                return None;
            }
            let mut out = Map::new();
            for (key, item) in map {
                match rekey(key) {
                    // The new plugin's own record of the game wins.
                    Some(new) => {
                        out.entry(new).or_insert_with(|| item.clone());
                    }
                    None => {
                        out.insert(key.clone(), item.clone());
                    }
                }
            }
            Some(Value::Object(out))
        }
        Value::Array(items) => {
            if !items.iter().any(|i| i.as_str().is_some_and(|k| rekey(k).is_some())) {
                return None;
            }
            let mut out: Vec<Value> = Vec::new();
            for item in items {
                let item = match item.as_str().and_then(rekey) {
                    Some(new) => Value::String(new),
                    None => item.clone(),
                };
                if !out.contains(&item) {
                    out.push(item);
                }
            }
            Some(Value::Array(out))
        }
        _ => None,
    }
}

/// Moves `plugins.<old>` into `plugins.<new>`: each game's settings (`game:<id>`) by its new
/// id, the others when `new` has none of its own.
fn move_settings(config: &ConfigService, new: &PluginId, old: &PluginId, console: &str) -> std::io::Result<bool> {
    let value = config.get(&["plugins", old.as_str()]);
    let Some(settings) = value.as_object() else { return Ok(false) };
    let mut target = config.get(&["plugins", new.as_str()]);
    if !target.is_object() {
        target = Value::Object(Map::new());
    }
    let into = target.as_object_mut().expect("an object");
    for (key, item) in settings {
        let key = match key.strip_prefix("game:") {
            Some(local) => format!("game:{console}/{local}"),
            None => key.clone(),
        };
        into.entry(key).or_insert_with(|| item.clone());
    }
    config.set(&["plugins", new.as_str()], target)?;
    config.remove(&["plugins", old.as_str()])?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> PluginId {
        PluginId::new(s).unwrap()
    }

    /// A config of the old shape, with every record a replaced plugin has.
    fn old_config(dir: &std::path::Path) -> ConfigService {
        let config = ConfigService::open(dir.join("config.json"));
        let app = serde_json::json!({
            "playtime": { "emu-nes:Contra": 600, "emu-genesis:Sonic": 60, "emu-ps1:Tekken": 5 },
            "hidden_games": ["emu-nes:Duck Hunt", "emu-ps1:Tekken"],
            "game_art": { "emu-nes:Contra": { "cover": "x" } },
            "game_folders": { "emu-genesis:Sonic": { "folder": "games" } },
            "results": { "emu-nes:Contra": "playable" },
            "result_builds": { "emu-nes:Contra": "1.22.2" },
            "result_skipped": ["emu-nes:Contra"],
            "result_reports": { "emu-genesis:Sonic": { "sent": true } },
            "disabled_plugins": ["emu-genesis", "pc"],
            "enabled_plugins": [],
            "installs": { "emu-nes": { "version": "1.22.2" } },
            "game_installs": { "4": { "plugin": "emu-nes", "work": "w" } },
            "torrents": { "abc": { "emulator": "emu-genesis", "title": "Sonic" } },
        });
        config.set(&["app"], app).unwrap();
        config
            .set(
                &["plugins", "emu-nes"],
                serde_json::json!({ "emulator": { "picture": "sharp" }, "game:Contra": { "fullscreen": false } }),
            )
            .unwrap();
        config
            .set(
                &["plugins", "emu-genesis"],
                serde_json::json!({ "emulator": { "picture": "crt" }, "game:Sonic": { "picture": "crt" } }),
            )
            .unwrap();
        config
    }

    fn retro() -> PluginManifest {
        PluginManifest::from_toml(
            "id = \"emu-retro\"\nkind = \"emulator\"\nname = \"Retro\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
             [[replaces]]\nplugin = \"emu-nes\"\nconsole = \"NES\"\n\
             [[replaces]]\nplugin = \"emu-genesis\"\nconsole = \"GENESIS\"\n",
        )
        .unwrap()
    }

    #[test]
    fn every_record_of_a_replaced_plugin_moves_once_with_the_console_in_front_of_its_games() {
        let dir = tempfile::tempdir().unwrap();
        let config = old_config(dir.path());
        let (plugin, manifest) = (id("emu-retro"), retro());
        move_replaced(&config, [(&plugin, &manifest)]);

        let app = |name: &str| config.get(&["app", name]);
        assert_eq!(
            app("playtime"),
            serde_json::json!({ "emu-retro:NES/Contra": 600, "emu-retro:GENESIS/Sonic": 60, "emu-ps1:Tekken": 5 })
        );
        assert_eq!(app("hidden_games"), serde_json::json!(["emu-retro:NES/Duck Hunt", "emu-ps1:Tekken"]));
        assert_eq!(app("game_art"), serde_json::json!({ "emu-retro:NES/Contra": { "cover": "x" } }));
        assert_eq!(app("game_folders"), serde_json::json!({ "emu-retro:GENESIS/Sonic": { "folder": "games" } }));
        assert_eq!(app("results"), serde_json::json!({ "emu-retro:NES/Contra": "playable" }));
        assert_eq!(app("result_builds"), serde_json::json!({ "emu-retro:NES/Contra": "1.22.2" }));
        assert_eq!(app("result_skipped"), serde_json::json!(["emu-retro:NES/Contra"]));
        assert_eq!(app("result_reports"), serde_json::json!({ "emu-retro:GENESIS/Sonic": { "sent": true } }));
        assert_eq!(app("disabled_plugins"), serde_json::json!(["pc"]));
        assert_eq!(config.get(&["app", "consoles", "emu-retro"]), serde_json::json!({ "GENESIS": false }));
        assert!(config.get(&["app", "installs", "emu-nes"]).is_null());
        assert_eq!(app("game_installs")["4"]["plugin"], "emu-retro");
        assert_eq!(app("torrents")["abc"]["emulator"], "emu-retro");
        assert_eq!(app("torrents")["abc"]["console"], "GENESIS");
        assert_eq!(
            config.get(&["plugins", "emu-retro"]),
            serde_json::json!({
                "emulator": { "picture": "sharp" },
                "game:NES/Contra": { "fullscreen": false },
                "game:GENESIS/Sonic": { "picture": "crt" },
            }),
            "the first replaced plugin's settings, and each game's own"
        );
        assert!(config.get(&["plugins", "emu-nes"]).is_null() && config.get(&["plugins", "emu-genesis"]).is_null());

        let before = std::fs::read(dir.path().join("config.json")).unwrap();
        move_replaced(&config, [(&plugin, &manifest)]);
        assert_eq!(std::fs::read(dir.path().join("config.json")).unwrap(), before, "a second run changes nothing");
    }
}

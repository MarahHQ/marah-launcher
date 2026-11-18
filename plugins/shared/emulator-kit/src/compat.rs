//! Community compatibility lists, by title id ([`title_key`]).
//!
//! - KytyPS5 (PS5): `https://kytyps5.github.io/data/compatibility.json`,
//!   `{ "PPSA…": { "status": "InGame", "platforms": { "linux": { "status": … } } } }`.
//! - shadPS4 (PS4): the `compatibility_data.json` release asset of
//!   `shadps4-compatibility/shadps4-game-compatibility`,
//!   `{ "CUSA…": { "os-linux": { "status": "status-ingame", "last_tested": … } } }`.
//!
//! Both take reports through a GitHub issue form: [`shad_report`] and [`kyty_report`] fill one
//! in with the player's result.
//!
//! The list's own top-level status is the best of any system, which would let a Windows
//! "in game" hide a Linux "doesn't boot", so the result for this system wins when there is one.

use std::collections::HashMap;

use launcher_api::serde_json::Value;
use launcher_api::{CompatInfo, CompatLevel, Platform, ReportField, ReportForm, ResultReport};

/// shadPS4's report form, which feeds its list.
pub const SHAD_FORM: &str =
    "https://github.com/shadps4-compatibility/shadps4-game-compatibility/issues/new?template=game_compatibility.yml";
/// KytyPS5's "Game Emulation Status Report" form, which feeds its list.
pub const KYTY_FORM: &str = "https://github.com/KytyPS5/KytyPS5/issues/new?template=kytyps5-game-emulation.yaml";

/// A game, as a report names it.
pub struct ReportedGame<'a> {
    pub title: &'a str,
    pub title_id: &'a str,
    pub version: &'a str,
}

fn field(id: &str, label: &str, value: impl Into<String>) -> ReportField {
    ReportField { id: id.into(), label: label.into(), value: value.into() }
}

/// What a result means, in the words of both forms' help.
fn meaning(level: CompatLevel) -> &'static str {
    match level {
        CompatLevel::Ingame | CompatLevel::Playable => "The game reaches gameplay",
        CompatLevel::Menus => "The game reaches its menus, but not gameplay",
        CompatLevel::Boots => "The game shows its first logos, but doesn't reach the menus",
        CompatLevel::Nothing | CompatLevel::Unknown => "The game doesn't start",
    }
}

/// shadPS4's form, filled in. Its checklist can't be: the player ticks it on the form.
pub fn shad_report(game: &ReportedGame, report: &ResultReport) -> Option<ReportForm> {
    let level = report.level?;
    let status = match level {
        CompatLevel::Playable => "Playable",
        CompatLevel::Ingame => "Ingame",
        CompatLevel::Menus => "Menus",
        CompatLevel::Boots => "Boots",
        CompatLevel::Nothing | CompatLevel::Unknown => "Nothing",
    };
    Some(ReportForm {
        list: "shadPS4 compatibility list".into(),
        form: SHAD_FORM.into(),
        title: format!("{} - {}", game.title_id, game.title),
        fields: vec![
            field("game-name", "Game name", game.title),
            field("game-serial", "Game serial", game.title_id),
            field("game-version", "Game version", game.version),
            field("emulator-version", "shadPS4 version", report.build.as_str()),
            field("emulation-status", "Current status", status),
            field("Operating-System", "Operating system", report.system.as_str()),
            field("processor", "Processor", report.cpu.as_str()),
            field("graphics-card", "Graphics card", report.gpu.as_str()),
            field("emulation-description", "Description", format!("{} on {}.", meaning(level), report.system)),
            field("log", "Log file", report.log.as_str()),
        ],
        note: "On the form, tick its checklist: shadPS4 takes reports only for a game dumped from a copy you own, \
               unmodified, played on a major release of shadPS4, not a nightly build."
            .into(),
    })
}

/// KytyPS5's form, filled in.
pub fn kyty_report(game: &ReportedGame, report: &ResultReport) -> Option<ReportForm> {
    let level = report.level?;
    let status = match level {
        CompatLevel::Ingame | CompatLevel::Playable => "In game",
        CompatLevel::Menus => "Main menu",
        CompatLevel::Boots => "Logo",
        CompatLevel::Nothing | CompatLevel::Unknown => "Doesn't boot",
    };
    Some(ReportForm {
        list: "KytyPS5 compatibility list".into(),
        form: KYTY_FORM.into(),
        title: format!("[GAME STATUS]: {} ({})", game.title, game.title_id),
        fields: vec![
            field("game-title", "Game title", game.title),
            field("game-id", "Game ID", game.title_id),
            field("kyty-version", "KytyPS5 version", report.build.as_str()),
            field("compatibility-status", "Compatibility status", status),
            field("what-happened", "Result details", format!("{} on {}.", meaning(level), report.system)),
            field("steps-to-reproduce", "Steps to reproduce", "Start the game in KytyPS5."),
            field("expected-behavior", "Expected behavior", "The game plays normally."),
            field("os", "OS", report.system.as_str()),
            field("cpu", "CPU", report.cpu.as_str()),
            field("gpu", "GPU", report.gpu.as_str()),
            field("ram-vram", "RAM / VRAM", report.memory.as_str()),
            field("log-file", "Log file", report.log.as_str()),
        ],
        note: "KytyPS5 asks for the whole log: on the form, attach it if you can (the game's menu, View emulator log)."
            .into(),
    })
}

fn kyty_level(s: &str) -> Option<CompatLevel> {
    Some(match s {
        "InGame" => CompatLevel::Ingame,
        "MainMenu" => CompatLevel::Menus,
        "Logo" => CompatLevel::Boots,
        "DoesntBoot" => CompatLevel::Nothing,
        _ => return None,
    })
}

fn shad_level(s: &str) -> Option<CompatLevel> {
    Some(match s {
        "status-playable" => CompatLevel::Playable,
        "status-ingame" => CompatLevel::Ingame,
        "status-menus" => CompatLevel::Menus,
        "status-boots" => CompatLevel::Boots,
        "status-nothing" => CompatLevel::Nothing,
        _ => return None,
    })
}

pub fn os_name(platform: Option<Platform>) -> &'static str {
    match platform {
        Some(Platform::Macos) => "macOS",
        Some(Platform::Windows) => "Windows",
        _ => "Linux",
    }
}

/// What the notes say about where a result comes from. `from` is the system the level was
/// tested on, when the list says; `version` the build it was tested with.
fn info(
    level: CompatLevel,
    from: Option<String>,
    platform: Option<Platform>,
    tested_on: &[String],
    version: &str,
    tested: Option<String>,
) -> CompatInfo {
    let here = from.as_deref() == Some(os_name(platform));
    let source = if here {
        format!("Tested on {}", os_name(platform))
    } else if tested_on.is_empty() {
        "Tested on another system".to_owned()
    } else {
        format!("Tested on {}, not yet on {}", tested_on.join(", "), os_name(platform))
    };
    let notes = if version.is_empty() { source } else { format!("{source} ({version})") };
    CompatInfo { level, notes: Some(notes), source: from, page: None, tested }
}

/// The `YYYY-MM-DD` at the start of `stamp`, such as `2026-10-01T22:30:38Z`.
fn day(stamp: &str) -> Option<String> {
    let day = stamp.get(..10)?;
    let digits = day.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    digits.then(|| day.to_owned())
}

/// A title id as the lists' maps key it: upper case, letters and digits only, and the four
/// letters and five digits of a PlayStation title id when they are in it, so `cusa-04118`,
/// `CUSA04118_00` and a content id such as `UP9000-CUSA04118_00-BLOODBORNE0000EU` are all
/// `CUSA04118`. `None` for an id with no letter or digit.
pub fn title_key(id: &str) -> Option<String> {
    let plain: Vec<u8> = id.bytes().filter(u8::is_ascii_alphanumeric).map(|b| b.to_ascii_uppercase()).collect();
    if plain.is_empty() {
        return None;
    }
    let playstation =
        plain.windows(9).find(|w| w[..4].iter().all(u8::is_ascii_alphabetic) && w[4..].iter().all(u8::is_ascii_digit));
    Some(String::from_utf8_lossy(playstation.unwrap_or(&plain)).into_owned())
}

/// KytyPS5's list for `platform`.
pub fn kyty(json: &Value, platform: Option<Platform>) -> HashMap<String, CompatInfo> {
    let key = match platform {
        Some(Platform::Macos) => "macos",
        Some(Platform::Windows) => "windows",
        _ => "linux",
    };
    let mut out = HashMap::new();
    let Some(games) = json.as_object() else { return out };
    for (tid, e) in games {
        let Some(best) = e["status"].as_str().and_then(kyty_level) else { continue };
        let per_os = e["platforms"].as_object();
        let here = per_os.and_then(|p| p.get(key)).and_then(|l| l["status"].as_str()).and_then(kyty_level);
        let mut tested_on: Vec<String> = per_os.map(|p| p.keys().map(|k| kyty_os(k)).collect()).unwrap_or_default();
        tested_on.sort();
        let version = per_os
            .and_then(|p| p.get(key).or_else(|| p.values().next()))
            .and_then(|l| l["version"].as_str())
            .unwrap_or_default();
        // Where the best result was tested, when it isn't this system's.
        let from = if here.is_some() {
            Some(os_name(platform).to_owned())
        } else {
            per_os.and_then(|p| {
                let mut best_on: Vec<_> = p
                    .iter()
                    .filter(|(_, l)| l["status"].as_str().and_then(kyty_level) == Some(best))
                    .map(|(k, _)| kyty_os(k))
                    .collect();
                best_on.sort();
                best_on.into_iter().next()
            })
        };
        let Some(tid) = title_key(tid) else { continue };
        out.insert(tid, info(here.unwrap_or(best), from, platform, &tested_on, version, None));
    }
    out
}

fn kyty_os(key: &str) -> String {
    match key {
        "linux" => "Linux".into(),
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        other => other.to_owned(),
    }
}

fn shad_os(key: &str) -> Option<String> {
    match key {
        "os-linux" => Some("Linux".to_owned()),
        "os-windows" => Some("Windows".to_owned()),
        "os-macOS" => Some("macOS".to_owned()),
        _ => None,
    }
}

/// shadPS4's list for `platform`.
pub fn shad(json: &Value, platform: Option<Platform>) -> HashMap<String, CompatInfo> {
    let key = match platform {
        Some(Platform::Macos) => "os-macOS",
        Some(Platform::Windows) => "os-windows",
        _ => "os-linux",
    };
    let mut out = HashMap::new();
    let Some(games) = json.as_object() else { return out };
    for (tid, per_os) in games {
        let Some(per_os) = per_os.as_object() else { continue };
        let level = |e: &Value| e["status"].as_str().and_then(shad_level);
        let Some(best) = per_os.values().filter_map(level).max() else { continue };
        let here = per_os.get(key).and_then(level);
        let mut tested_on: Vec<String> = per_os.keys().filter_map(|k| shad_os(k)).collect();
        tested_on.sort();
        let tested = per_os
            .get(key)
            .and_then(|e| e["last_tested"].as_str())
            .or_else(|| per_os.values().filter_map(|e| e["last_tested"].as_str()).max())
            .and_then(day);
        let from = if here.is_some() {
            Some(os_name(platform).to_owned())
        } else {
            let mut best_on: Vec<_> =
                per_os.iter().filter(|(_, e)| level(e) == Some(best)).filter_map(|(k, _)| shad_os(k)).collect();
            best_on.sort();
            best_on.into_iter().next()
        };
        let Some(tid) = title_key(tid) else { continue };
        out.insert(tid, info(here.unwrap_or(best), from, platform, &tested_on, "", tested));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_api::serde_json::json;

    #[test]
    fn this_system_s_result_wins_over_the_best_elsewhere() {
        let list = json!({
            "ppsa01": { "status": "InGame", "platforms": { "windows": { "status": "InGame" }, "linux": { "status": "DoesntBoot", "version": "KytyPS5-2026-10-01" } } },
            "PPSA02": { "status": "MainMenu", "platforms": { "windows": { "status": "MainMenu" } } },
            "PPSA03": { "status": "Unknown" },
        });
        let db = kyty(&list, Some(Platform::Linux));
        assert_eq!(db["PPSA01"].level, CompatLevel::Nothing);
        assert_eq!(db["PPSA01"].notes.as_deref(), Some("Tested on Linux (KytyPS5-2026-10-01)"));
        assert_eq!(db["PPSA02"].level, CompatLevel::Menus);
        assert_eq!(db["PPSA02"].notes.as_deref(), Some("Tested on Windows, not yet on Linux"));
        assert!(!db.contains_key("PPSA03"));

        let shad_list = json!({ "CUSA04118": {
            "os-windows": { "status": "status-playable", "last_tested": "2026-09-01" },
            "os-linux": { "status": "status-menus", "last_tested": "2026-08-01" } } });
        let db = shad(&shad_list, Some(Platform::Linux));
        assert_eq!(db["CUSA04118"].level, CompatLevel::Menus);
        assert_eq!(shad(&shad_list, Some(Platform::Macos))["CUSA04118"].level, CompatLevel::Playable);
        let stamped =
            json!({ "CUSA00001": { "os-linux": { "status": "status-boots", "last_tested": "2026-10-01T22:30:38Z" } } });
        assert_eq!(shad(&stamped, Some(Platform::Linux))["CUSA00001"].tested.as_deref(), Some("2026-10-01"));
    }

    #[test]
    fn a_title_id_is_the_same_in_any_case_and_with_any_separators() {
        for id in ["CUSA04118", "cusa04118", " CUSA-04118 ", "CUSA04118_00", "UP9000-CUSA04118_00-BLOODBORNE0000EU"] {
            assert_eq!(title_key(id).as_deref(), Some("CUSA04118"), "{id:?}");
        }
        assert_eq!(title_key("ppsa01325").as_deref(), Some("PPSA01325"));
        assert_eq!(title_key(""), None);
        assert_eq!(title_key(" - "), None);
        // A list's own keys read the same way.
        let shad_list = json!({ "cusa-04118": { "os-linux": { "status": "status-menus" } } });
        assert_eq!(shad(&shad_list, Some(Platform::Linux))["CUSA04118"].level, CompatLevel::Menus);
        let kyty_list = json!({ "ppsa01325": { "status": "InGame" } });
        assert_eq!(kyty(&kyty_list, Some(Platform::Linux))["PPSA01325"].level, CompatLevel::Ingame);
    }

    fn report() -> ResultReport {
        ResultReport {
            level: Some(CompatLevel::Menus),
            build: "0.21.0".into(),
            system: "Linux".into(),
            cpu: "AMD Ryzen 7 5800X".into(),
            gpu: "AMD Radeon RX 6800".into(),
            memory: "32 GB".into(),
            log: "[Render] the error".into(),
        }
    }

    fn values(form: &ReportForm) -> Vec<(&str, &str)> {
        form.fields.iter().map(|f| (f.id.as_str(), f.value.as_str())).collect()
    }

    #[test]
    fn shadps4_s_form_is_filled_in_with_its_own_fields() {
        let game = ReportedGame { title: "Bloodborne", title_id: "CUSA03173", version: "01.09" };
        let form = shad_report(&game, &report()).unwrap();
        assert_eq!(form.form, SHAD_FORM);
        assert_eq!(form.title, "CUSA03173 - Bloodborne", "the list's own titles");
        assert_eq!(
            values(&form),
            [
                ("game-name", "Bloodborne"),
                ("game-serial", "CUSA03173"),
                ("game-version", "01.09"),
                ("emulator-version", "0.21.0"),
                ("emulation-status", "Menus"),
                ("Operating-System", "Linux"),
                ("processor", "AMD Ryzen 7 5800X"),
                ("graphics-card", "AMD Radeon RX 6800"),
                ("emulation-description", "The game reaches its menus, but not gameplay on Linux."),
                ("log", "[Render] the error"),
            ]
        );
        assert!(form.note.contains("checklist"));
        assert!(shad_report(&game, &ResultReport::default()).is_none(), "no result, no report");
    }

    #[test]
    fn kytyps5_s_form_is_filled_in_with_its_own_fields() {
        let game = ReportedGame { title: "Astro Bot", title_id: "PPSA01325", version: "" };
        let form = kyty_report(&game, &ResultReport { level: Some(CompatLevel::Nothing), ..report() }).unwrap();
        assert_eq!(form.form, KYTY_FORM);
        assert_eq!(form.title, "[GAME STATUS]: Astro Bot (PPSA01325)");
        let values = values(&form);
        assert!(values.contains(&("compatibility-status", "Doesn't boot")));
        assert!(values.contains(&("game-id", "PPSA01325")));
        assert!(values.contains(&("kyty-version", "0.21.0")));
        assert!(values.contains(&("ram-vram", "32 GB")));
        assert!(values.contains(&("log-file", "[Render] the error")));
    }

    #[test]
    fn the_badge_names_the_system_the_result_was_tested_on() {
        let list = json!({
            "PPSA01": { "status": "InGame", "platforms": { "macos": { "status": "InGame" } } },
            "PPSA02": { "status": "InGame", "platforms": { "windows": { "status": "MainMenu" }, "linux": { "status": "InGame" } } },
        });
        let db = kyty(&list, Some(Platform::Macos));
        let badge = |tid: &str, platform| match crate::games::compat_badge(&db[tid], platform) {
            launcher_api::SlotContent::Badge { label, short, .. } => (label, short),
            _ => unreachable!("a badge"),
        };
        assert_eq!(badge("PPSA01", Some(Platform::Macos)), ("In-game on macOS".into(), "In-game".into()));
        assert_eq!(
            badge("PPSA02", Some(Platform::Macos)),
            ("In-game on Linux · untested on macOS".into(), "Linux · In-game".into())
        );

        let shad_list = json!({ "CUSA04118": { "os-windows": { "status": "status-playable" } } });
        let shad_db = shad(&shad_list, Some(Platform::Linux));
        let launcher_api::SlotContent::Badge { label, short, .. } =
            crate::games::compat_badge(&shad_db["CUSA04118"], Some(Platform::Linux))
        else {
            unreachable!("a badge")
        };
        assert_eq!((label.as_str(), short.as_str()), ("Playable on Windows · untested on Linux", "Win · Playable"));
    }
}

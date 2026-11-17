//! shadPS4's key map (`input_config/*.ini` in its data folder), set up so that the touchpad
//! button starts PS1 games on PS4.
//!
//! A PS1 game on PS4 reads a touchpad click on the right half as START and on the left half as
//! SELECT. shadPS4's own map clicks the centre (`touchpad_center`), which is neither, so such a
//! game waits at "Press START" for ever. A PS4 game takes a click anywhere as the touchpad
//! button, so the right half works for it too.
//!
//! Before each launch: with no `default.ini`, the plugin writes shadPS4's defaults
//! (`default-input.ini`, from shadPS4 v0.21.0's `GetDefaultInputConfig`) with the fix. In a map
//! that has them, only shadPS4's own two touchpad lines change. A touchpad the player mapped to
//! other keys stays, and `global.ini` isn't touched.

use emulator_kit::host::Host;
use launcher_api::*;

use crate::DATA;

/// shadPS4's default key map, with the touchpad button on the right half.
const DEFAULT: &str = include_str!("default-input.ini");
/// The folder of the key maps, in shadPS4's data folder.
const FOLDER: &str = "input_config";

/// `map` with shadPS4's own touchpad lines moved to the right half (and Backspace on the left
/// half, for SELECT), or `None` when it has none of them.
pub fn fix(map: &str) -> Option<String> {
    let mut changed = false;
    let mut out = Vec::new();
    for line in map.split('\n') {
        match line.trim_end_matches('\r') {
            "touchpad_center = space" => {
                out.extend(["touchpad_right = space", "touchpad_left = backspace"]);
                changed = true;
            }
            "touchpad_center = back" => {
                out.push("touchpad_right = back");
                changed = true;
            }
            _ => out.push(line),
        }
    }
    changed.then(|| out.join("\n"))
}

/// Sets up the key maps in shadPS4's first data folder. A failure is only logged: the game
/// starts with the maps as they are.
pub async fn set_up(host: &Host) {
    if let Err(e) = try_set_up(host).await {
        host.log.warn(&format!("shadPS4's key map wasn't set up: {e}"));
    }
}

async fn try_set_up(host: &Host) -> PluginResult<()> {
    let Some(root) = host.folders.roots(DATA).await?.first().map(|r| r.root) else { return Ok(()) };
    let folder = HostPath::new(DATA, root, RelPath::new(FOLDER)?);
    let default = folder.join("default.ini")?;
    if !host.folders.exists(&default).await? {
        return host.folders.write(&default, DEFAULT.as_bytes()).await;
    }
    for entry in host.folders.list(&folder).await? {
        if entry.is_dir || !entry.name.ends_with(".ini") || entry.name == "global.ini" {
            continue;
        }
        let path = folder.join(&entry.name)?;
        let bytes = host.folders.read_range(&path, 0, MAX_FOLDER_READ).await?;
        let Ok(map) = String::from_utf8(bytes) else { continue };
        if let Some(fixed) = fix(&map) {
            host.folders.write(&path, fixed.as_bytes()).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_map_is_shadps4_s_with_the_touchpad_on_the_right_half() {
        assert!(fix(DEFAULT).is_none(), "nothing left to fix");
        for line in
            ["touchpad_right = space", "touchpad_left = backspace", "touchpad_right = back", "options = options"]
        {
            assert!(DEFAULT.lines().any(|l| l == line), "{line}");
        }
    }

    #[test]
    fn windows_line_ends_are_fixed_too() {
        assert_eq!(fix("a\r\ntouchpad_center = back\r\n").as_deref(), Some("a\r\ntouchpad_right = back\n"));
    }
}

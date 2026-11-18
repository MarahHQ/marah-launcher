//! shadPS4's key map (`input_config/*.ini` in its data folder), set up so that the touchpad
//! button starts PS1 games on PS4.
//!
//! A PS1 game on PS4 reads a touchpad click on the right half as START and on the left half as
//! SELECT. shadPS4's own map clicks the centre (`touchpad_center`), which is neither, so such a
//! game waits at "Press START" for ever. A PS4 game takes a click anywhere as the touchpad
//! button, so the right half works for it too.
//!
//! In a PS1 game on PS4 the PS4's Options button does nothing, and a player presses the
//! controller's Start for START. So before a PS1 game ([`Pad::Ps1`]), the controller's Start
//! clicks the right half and Back the left half; before a PS4 game ([`Pad::Ps4`]), Start is
//! Options and Back the touchpad again. Only these two lines of shadPS4's map change.
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

/// What the controller's Start and Back do in the game about to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad {
    /// Start is Options, Back the touchpad: a PS4 game.
    Ps4,
    /// Start is START (the touchpad's right half), Back is SELECT (its left half): a PS1 game.
    Ps1,
}

impl Pad {
    /// The two controller lines, as `(Start's, Back's)`.
    fn lines(self) -> (&'static str, &'static str) {
        match self {
            Pad::Ps4 => ("options = options", "touchpad_right = back"),
            Pad::Ps1 => ("touchpad_right = options", "touchpad_left = back"),
        }
    }
}

/// `map` with the other kind's Start and Back lines turned into `pad`'s, or `None` when it has
/// none of them.
pub fn for_game(map: &str, pad: Pad) -> Option<String> {
    let other = if pad == Pad::Ps1 { Pad::Ps4 } else { Pad::Ps1 };
    let (from_start, from_back) = other.lines();
    let (to_start, to_back) = pad.lines();
    let mut changed = false;
    let lines: Vec<String> = map
        .split('\n')
        .map(|line| match line.trim_end_matches('\r') {
            l if l == from_start => {
                changed = true;
                to_start.to_owned()
            }
            l if l == from_back => {
                changed = true;
                to_back.to_owned()
            }
            _ => line.to_owned(),
        })
        .collect();
    changed.then(|| lines.join("\n"))
}

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
pub async fn set_up(host: &Host, pad: Pad, title_id: &str) {
    if let Err(e) = try_set_up(host, pad, title_id).await {
        host.log.warn(&format!("shadPS4's key map wasn't set up: {e}"));
    }
}

async fn try_set_up(host: &Host, pad: Pad, title_id: &str) -> PluginResult<()> {
    let Some(root) = host.folders.roots(DATA).await?.first().map(|r| r.root) else { return Ok(()) };
    let folder = HostPath::new(DATA, root, RelPath::new(FOLDER)?);
    let default = folder.join("default.ini")?;
    if !host.folders.exists(&default).await? {
        host.folders.write(&default, DEFAULT.as_bytes()).await?;
    }
    // shadPS4 reads `default.ini`, or the game's own map when its "one map for all games"
    // setting is off: both get the game's Start and Back.
    let game = format!("{title_id}.ini");
    for entry in host.folders.list(&folder).await? {
        if entry.is_dir || !entry.name.ends_with(".ini") || entry.name == "global.ini" {
            continue;
        }
        let path = folder.join(&entry.name)?;
        let bytes = host.folders.read_range(&path, 0, MAX_FOLDER_READ).await?;
        let Ok(map) = String::from_utf8(bytes) else { continue };
        let mut new = fix(&map);
        if entry.name == "default.ini" || entry.name == game {
            new = for_game(new.as_deref().unwrap_or(&map), pad).or(new);
        }
        if let Some(new) = new {
            host.folders.write(&path, new.as_bytes()).await?;
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
    fn start_and_back_switch_between_a_ps1_and_a_ps4_game_and_the_rest_stays() {
        let ps1 = for_game(DEFAULT, Pad::Ps1).expect("the default map is a PS4 game's");
        assert!(
            ps1.lines().any(|l| l == "touchpad_right = options") && ps1.lines().any(|l| l == "touchpad_left = back")
        );
        assert!(for_game(&ps1, Pad::Ps1).is_none(), "nothing left to switch");
        assert_eq!(for_game(&ps1, Pad::Ps4).as_deref(), Some(DEFAULT), "back to shadPS4's map with the fix");
        assert!(for_game("touchpad_right = start\n", Pad::Ps1).is_none(), "the player's own lines stay");
    }

    #[test]
    fn windows_line_ends_are_fixed_too() {
        assert_eq!(fix("a\r\ntouchpad_center = back\r\n").as_deref(), Some("a\r\ntouchpad_right = back\n"));
    }
}

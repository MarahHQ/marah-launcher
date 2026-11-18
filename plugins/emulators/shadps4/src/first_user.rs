//! shadPS4's first user, made before shadPS4's first start.
//!
//! shadPS4 v0.21.0 makes the folder of its first user (`home/1000`) at its first start, and then
//! asks in a box with four buttons what to do with saves in its old place (`savedata/1`), even
//! when there are none (`CreateDefaultUsers` in `src/core/user_manager.cpp`). A player with only
//! a controller can't answer it. With no old saves, the plugin makes that folder as shadPS4 does,
//! so shadPS4 asks nothing. With old saves, the choice is the player's: shadPS4 asks.

use emulator_kit::host::Host;
use launcher_api::*;

use crate::DATA;

/// The folders shadPS4 makes for a user.
const FOLDERS: [&str; 3] = ["savedata", "trophy", "inputs"];

/// Makes `home/1000` in shadPS4's first data folder when it is missing and there are no old
/// saves. A failure is only logged: shadPS4 asks as before.
pub async fn set_up(host: &Host) {
    if let Err(e) = try_set_up(host).await {
        host.log.warn(&format!("shadPS4's first user wasn't made: {e}"));
    }
}

async fn try_set_up(host: &Host) -> PluginResult<()> {
    let Some(root) = host.folders.roots(DATA).await?.first().map(|r| r.root) else { return Ok(()) };
    let at = |path: &str| -> PluginResult<HostPath> { Ok(HostPath::new(DATA, root, RelPath::new(path)?)) };
    let user = at("home/1000")?;
    if host.folders.exists(&user).await? || !host.folders.list(&at("savedata/1")?).await?.is_empty() {
        return Ok(());
    }
    for folder in FOLDERS {
        // A folder is made with a file in it: the host makes parent folders as it writes.
        let keep = user.join(folder)?.join(".keep")?;
        host.folders.write(&keep, b"").await?;
        host.folders.remove(&keep).await?;
    }
    Ok(())
}

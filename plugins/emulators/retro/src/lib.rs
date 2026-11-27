//! NES, Genesis and Atari 2600 games in one plugin, on the shared RetroArch: each console is a
//! [`RetroConsole`] with its core, and [`RetroConsoles`] routes the host's calls to it. Each
//! console's games are the ROM files in its folder of the player's game folders (`NES/`,
//! `GENESIS/`, `ATARI2600/`), and the player turns each console on or off on the plugin's page.

use emulator_kit::games::RomSystem;
use emulator_kit::retro_consoles::{ConsolePart, RetroConsoles};
use emulator_kit::retroarch::{Files, RetroConsole};
use launcher_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

/// NES game files.
pub static NES_SYSTEM: RomSystem = RomSystem {
    folder: "NES",
    games: &["nes", "fds", "unf", "unif", "zip"],
    files: &[],
    playlist: None,
    thumbnails: Some("Nintendo_-_Nintendo_Entertainment_System"),
};

/// Genesis, Mega Drive, Master System and Game Gear game files.
pub static GENESIS_SYSTEM: RomSystem = RomSystem {
    folder: "GENESIS",
    games: &["md", "gen", "smd", "bin", "sms", "gg", "zip"],
    files: &[],
    playlist: None,
    thumbnails: Some("Sega_-_Mega_Drive_-_Genesis"),
};

/// Atari 2600 game files.
pub static ATARI2600_SYSTEM: RomSystem = RomSystem {
    folder: "ATARI2600",
    games: &["a26", "bin", "zip"],
    files: &[],
    playlist: None,
    thumbnails: Some("Atari_-_2600"),
};

/// The NES, on FCEUmm.
pub struct Nes;

impl RetroConsole for Nes {
    const CONSOLE: &'static str = "NES";
    const SYSTEM: &'static RomSystem = &NES_SYSTEM;
    const CORE: &'static str = "fceumm";
    const SECTION: &'static str = "retro-nes";
    const CORE_NAME: &'static str = "FCEUmm";
}

/// The Genesis, on Genesis Plus GX.
pub struct Genesis;

impl RetroConsole for Genesis {
    const CONSOLE: &'static str = "GENESIS";
    const SYSTEM: &'static RomSystem = &GENESIS_SYSTEM;
    const CORE: &'static str = "genesis_plus_gx";
    const SECTION: &'static str = "retro-genesis";
    const CORE_NAME: &'static str = "Genesis Plus GX";
}

/// The Atari 2600, on Stella.
pub struct Atari2600;

impl RetroConsole for Atari2600 {
    const CONSOLE: &'static str = "ATARI2600";
    const SYSTEM: &'static RomSystem = &ATARI2600_SYSTEM;
    const CORE: &'static str = "stella";
    const SECTION: &'static str = "retro-atari2600";
    const CORE_NAME: &'static str = "Stella";
}

/// What the plugin provides, with the builds' files from `files`: a test serves its own.
pub async fn provided_with(ctx: &PluginContext, files: Files) -> PluginResult<Provided> {
    let parts = vec![
        ConsolePart::of::<Nes>(ctx, files).await?,
        ConsolePart::of::<Genesis>(ctx, files).await?,
        ConsolePart::of::<Atari2600>(ctx, files).await?,
    ];
    RetroConsoles::provided(parts)
}

/// The Retro system's pictures and each console's, in `assets/<name>/`, as the manifest's `[consoles.system]` names
/// them (`scripts/make-system-art.sh` makes them).
fn art() -> std::sync::Arc<dyn launcher_api::Files> {
    macro_rules! asset {
        ($path:literal) => {
            ($path, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path)).as_slice())
        };
    }
    const FILES: &[(&str, &[u8])] = &[
        asset!("retro/icon.png"),
        asset!("retro/cover.jpg"),
        asset!("nes/icon.png"),
        asset!("nes/cover.jpg"),
        asset!("genesis/icon.png"),
        asset!("genesis/cover.jpg"),
        asset!("atari2600/icon.png"),
    ];
    std::sync::Arc::new(EmbeddedFiles::new(FILES))
}

pub struct RetroPlugin;

#[async_trait]
impl Plugin for RetroPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for RetroPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let provided = provided_with(&ctx, emulator_kit::retroarch::files).await?;
        Ok(Provided { assets: Some(art()), ..provided })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(RetroPlugin)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_lists_each_console_with_its_core_in_the_pinned_pack() {
        let manifest = RetroPlugin.manifest();
        let consoles: Vec<&str> = manifest.consoles.iter().map(|c| c.console.as_str()).collect();
        assert_eq!(consoles, [Nes::CONSOLE, Genesis::CONSOLE, Atari2600::CONSOLE]);
        assert!(manifest.consoles.iter().all(|c| c.on), "each is on until the player turns it off");
        assert_eq!(manifest.shared_data.as_ref().map(ShareName::as_str), Some("retroarch"));
        for core in [Nes::CORE, Genesis::CORE, Atari2600::CORE] {
            assert!(emulator_kit::retroarch::CORES.contains(&core), "{core}");
        }
        let replaced: Vec<(&str, &str)> =
            manifest.replaces.iter().map(|r| (r.plugin.as_str(), r.console.as_str())).collect();
        assert_eq!(replaced, [("emu-nes", "NES"), ("emu-genesis", "GENESIS")]);
    }
}

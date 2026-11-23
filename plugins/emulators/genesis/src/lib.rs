//! Genesis, Mega Drive, Master System and Game Gear games, on RetroArch's Genesis Plus GX core, as a plugin. The kit's
//! [`emulator_kit::retroarch`] runs it: RetroArch is one program for every retro plugin, in the
//! data folder they share. Games are the ROM files in `GENESIS/` of the player's game folders.

use emulator_kit::games::RomSystem;
use emulator_kit::retroarch::{Retro, RetroConsole};
use launcher_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

/// GENESIS game files.
pub static SYSTEM: RomSystem = RomSystem {
    folder: "GENESIS",
    games: &["md", "gen", "smd", "bin", "sms", "gg", "zip"],
    files: &[],
    playlist: None,
    thumbnails: Some("Sega_-_Mega_Drive_-_Genesis"),
};

/// The Genesis on RetroArch.
pub struct Genesis;

impl RetroConsole for Genesis {
    const CONSOLE: &'static str = "GENESIS";
    const SYSTEM: &'static RomSystem = &SYSTEM;
    const CORE: &'static str = "genesis_plus_gx";
    const SECTION: &'static str = "emu-genesis";
    const CORE_NAME: &'static str = "Genesis Plus GX";
}

pub struct GenesisPlugin;

#[async_trait]
impl Plugin for GenesisPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for GenesisPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let provided = Retro::<Genesis>::provided(&ctx).await?;
        Ok(Provided { assets: Some(launcher_api::system_art!()), ..provided })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(GenesisPlugin)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_reads_and_plays_its_console_on_the_shared_retroarch() {
        let manifest = GenesisPlugin.manifest();
        assert_eq!(manifest.console.as_ref().map(|c| c.as_str()), Some(Genesis::CONSOLE));
        assert_eq!(manifest.shared_data.as_ref().map(ShareName::as_str), Some("retroarch"));
        assert!(emulator_kit::retroarch::CORES.contains(&Genesis::CORE));
    }
}

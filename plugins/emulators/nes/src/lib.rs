//! NES games, on RetroArch's FCEUmm core, as a plugin. The kit's
//! [`emulator_kit::retroarch`] runs it: RetroArch is one program for every retro plugin, in the
//! data folder they share. Games are the ROM files in `NES/` of the player's game folders.

use emulator_kit::games::RomSystem;
use emulator_kit::retroarch::{Retro, RetroConsole};
use launcher_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

/// NES game files.
pub static SYSTEM: RomSystem = RomSystem {
    folder: "NES",
    games: &["nes", "fds", "unf", "unif", "zip"],
    files: &[],
    playlist: None,
    thumbnails: Some("Nintendo_-_Nintendo_Entertainment_System"),
};

/// The NES on RetroArch.
pub struct Nes;

impl RetroConsole for Nes {
    const CONSOLE: &'static str = "NES";
    const SYSTEM: &'static RomSystem = &SYSTEM;
    const CORE: &'static str = "fceumm";
    const SECTION: &'static str = "emu-nes";
    const CORE_NAME: &'static str = "FCEUmm";
}

pub struct NesPlugin;

#[async_trait]
impl Plugin for NesPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for NesPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let provided = Retro::<Nes>::provided(&ctx).await?;
        Ok(Provided { assets: Some(launcher_api::system_art!()), ..provided })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(NesPlugin)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_reads_and_plays_its_console_on_the_shared_retroarch() {
        let manifest = NesPlugin.manifest();
        assert_eq!(manifest.console.as_ref().map(|c| c.as_str()), Some(Nes::CONSOLE));
        assert_eq!(manifest.shared_data.as_ref().map(ShareName::as_str), Some("retroarch"));
        assert!(emulator_kit::retroarch::CORES.contains(&Nes::CORE));
    }
}

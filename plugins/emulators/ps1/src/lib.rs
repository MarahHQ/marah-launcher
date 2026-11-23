//! PS1 games, on RetroArch's PCSX ReARMed core, as a plugin. The kit's
//! [`emulator_kit::retroarch`] runs it: RetroArch is one program for every retro plugin, in the
//! data folder they share. Games are the ROM files in `PS1/` of the player's game folders.

use emulator_kit::games::RomSystem;
use emulator_kit::retroarch::{Retro, RetroConsole};
use launcher_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

/// PS1 game files.
pub static SYSTEM: RomSystem = RomSystem {
    folder: "PS1",
    games: &["cue", "chd", "pbp", "m3u"],
    files: &["bin", "img", "sub", "ccd"],
    playlist: Some("m3u"),
    thumbnails: Some("Sony_-_PlayStation"),
};

/// The PS1 on RetroArch.
pub struct Ps1;

impl RetroConsole for Ps1 {
    const CONSOLE: &'static str = "PS1";
    const SYSTEM: &'static RomSystem = &SYSTEM;
    const CORE: &'static str = "pcsx_rearmed";
    const SECTION: &'static str = "emu-ps1";
    const CORE_NAME: &'static str = "PCSX-ReARMed";
    const BIOS: bool = true;
}

pub struct Ps1Plugin;

#[async_trait]
impl Plugin for Ps1Plugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for Ps1Plugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let provided = Retro::<Ps1>::provided(&ctx).await?;
        Ok(Provided { assets: Some(launcher_api::system_art!()), ..provided })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(Ps1Plugin)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_reads_and_plays_its_console_on_the_shared_retroarch() {
        let manifest = Ps1Plugin.manifest();
        assert_eq!(manifest.console.as_ref().map(|c| c.as_str()), Some(Ps1::CONSOLE));
        assert_eq!(manifest.shared_data.as_ref().map(ShareName::as_str), Some("retroarch"));
        assert!(emulator_kit::retroarch::CORES.contains(&Ps1::CORE));
    }
}

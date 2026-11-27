//! Seam 2: the capability contracts catch a plugin that breaks a rule. The KytyPS5 and
//! shadPS4 plugins run the full suites in `kyty.rs` and `shadps4.rs`; the testkit's scripted
//! emulator runs them here.

use launcher_api::{CompatLevel, PluginId};
use launcher_testkit::FakeHost;
use launcher_testkit::contract;

/// A catalog that lists one game twice.
struct Twice;

#[launcher_api::async_trait]
impl launcher_api::Catalog for Twice {
    async fn list(
        &self,
        _: launcher_api::PageRequest,
    ) -> launcher_api::PluginResult<launcher_api::Page<launcher_api::GameSummary>> {
        let game = launcher_api::GameSummary {
            id: "same".into(),
            title: "Same".into(),
            platform: "PS4".into(),
            cover: None,
            tags: vec![],
            release_year: None,
            title_id: None,
            content_id: None,
        };
        Ok(launcher_api::Page { items: vec![game.clone(), game], total: None })
    }
    async fn search(
        &self,
        _: &launcher_api::SearchQuery,
    ) -> launcher_api::PluginResult<Vec<launcher_api::GameSummary>> {
        Ok(vec![])
    }
    async fn details(&self, game: &str) -> launcher_api::PluginResult<launcher_api::GameDetails> {
        Err(launcher_api::PluginError::NotFound(game.into()))
    }
    async fn artwork(
        &self,
        _: &str,
        _: launcher_api::ArtworkKind,
    ) -> launcher_api::PluginResult<Option<launcher_api::ImageSource>> {
        Ok(None)
    }
}

#[tokio::test]
#[should_panic(expected = "contract catalog: game ids must be unique")]
async fn the_contract_catches_a_plugin_that_breaks_a_rule() {
    let plugin = launcher_testkit::StubPlugin::from_toml(
        "id = \"twice\"\nkind = \"emulator\"\nname = \"Twice\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\ncapabilities = [\"catalog\"]",
    )
    .provides(|_| Ok(launcher_api::Provided { catalog: Some(std::sync::Arc::new(Twice)), ..Default::default() }));
    let test = FakeHost::new().plugin(plugin).boot_core().await;
    contract::emulator(&test, &PluginId::new("twice").unwrap()).await;
}

#[tokio::test]
async fn the_fake_emulator_keeps_every_contract() {
    let host = FakeHost::new();
    let mut emulator = launcher_testkit::FakeEmulator::new(vec!["Alpha", "Beta"]);
    emulator.build_sha = host.http().serve(&emulator.build_url, vec![7u8; 20_000]);
    let test = host.plugin(emulator.plugin("emu-a")).boot_core().await;
    contract::emulator(&test, &PluginId::new("emu-a").unwrap()).await;
    assert_eq!(emulator.exits().len(), 1, "the launch contract's game exited");
}

/// Its build choices are left out: their installer never installs.
#[tokio::test]
async fn the_fake_catalog_keeps_the_contracts_of_its_catalog_slots_launcher_and_compatibility() {
    for (id, emulator) in [
        ("emu-tested", launcher_testkit::FakeCatalog::new("emu-tested").tested()),
        ("emu-lists", launcher_testkit::FakeCatalog::new("emu-lists").lists(&[("CUSA00000", CompatLevel::Playable)])),
    ] {
        let plugin = emulator.games(["Alpha", "Beta", "Gamma"]).playable().key("Cross", "J").build();
        let test = FakeHost::new().plugin(plugin).boot_core().await;
        contract::emulator(&test, &PluginId::new(id).unwrap()).await;
    }
}

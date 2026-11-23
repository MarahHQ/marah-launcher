//! Seam 1, core app API: tracing spans around plugin calls.
//!
//! Its own test binary: `tracing` caches per-callsite interest across threads, so a recorder
//! on one test thread can miss spans while other tests run in parallel.

use launcher_api::PluginId;
use launcher_core::Command;
use launcher_core::system::SystemCommand;
use launcher_testkit::{FakeHost, FakeSystem, SpanRecorder, StubPlugin};

fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

fn manifest(id: &str, extra: &str) -> String {
    format!("id = \"{id}\"\nkind = \"emulator\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.5\"\n{extra}")
}

#[test]
fn every_plugin_call_runs_in_a_span_tagged_with_the_plugin_id() {
    let recorder = SpanRecorder::default();
    let _guard = recorder.install();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let host = FakeHost::new().plugin(StubPlugin::from_toml(&manifest("traced", ""))).boot().await;
        host.kernel.activate(&id("traced")).await.unwrap();
    });

    let spans = recorder.spans("plugin_call");
    let fields = spans.first().expect("a plugin_call span");
    assert!(fields.contains(&("plugin_id".into(), "traced".into())), "{fields:?}");
    assert!(fields.contains(&("method".into(), "init".into())), "{fields:?}");
}

#[test]
fn each_system_read_names_its_capability_and_method() {
    let recorder = SpanRecorder::default();
    let _guard = recorder.install();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let t = FakeHost::new().plugin(FakeSystem::new().plugin()).boot_core().await;
        t.core.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();
    });

    let methods: Vec<String> = recorder
        .spans("plugin_call")
        .into_iter()
        .filter(|fields| fields.contains(&("plugin_id".into(), "fake-system".into())))
        .filter_map(|fields| fields.into_iter().find(|(name, _)| name == "method").map(|(_, method)| method))
        .filter(|method| method != "init")
        .collect();
    for method in ["network.state", "os_updates.state", "power.state", "audio.state"] {
        assert!(methods.iter().any(|m| m == method), "{method} in {methods:?}");
    }
    assert!(methods.iter().all(|m| m.contains('.')), "every call says its capability: {methods:?}");
}

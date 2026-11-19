//! Seam 1, core app API: commands in, view models and events out, one module per area. The
//! scripted emulator and the steps more than one area uses are here.

mod downloads;
mod emulators;
mod library;
mod navigation;
mod results;
mod sessions;
mod settings;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::common::id;
use futures::StreamExt;
use launcher_api::*;
use launcher_core::Front;
use launcher_core::{Area, Clock, Command, Core, CoreError, Pref, Route, SessionCmd};
use launcher_testkit::{
    CloseAnswer, FAKE_EMULATOR_PERMISSIONS, FakeEmulator, FakeHost, Shown, Signal, SpawnedProcess, StubPlugin, TestCore,
};
use launcher_ui_api::ScreenId;

fn key(s: &str) -> GameKey {
    s.parse().unwrap()
}

/// The fake host's `watch_every`: how often the game watcher looks at the desktop.
const WATCH_EVERY: Duration = Duration::from_millis(20);

/// Boots a core with `emu` as `emu-a`, with its build served over fake HTTP.
async fn core_with(mut emu: FakeEmulator) -> (TestCore, FakeEmulator) {
    let host = FakeHost::new();
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let core = host.plugin(emu.plugin("emu-a")).boot_core().await;
    (core, emu)
}

/// Turns Install emulators automatically off, as a player can: the boot installs nothing, and
/// Play asks first.
async fn no_auto_install(t: &TestCore) {
    t.core.dispatch(Command::Pref(Pref::AutoInstall, false.into())).await.unwrap();
}

/// Reads the library at the start, as the app does, and waits until the boot screen can go:
/// the start's installs are over.
async fn boot(t: &TestCore) {
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    t.until(Area::Library, |core| core.view().boot.ready).await;
}

async fn install(core: &Core) {
    core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap();
}

/// Installs the emulator and starts `emu-a:G0`.
async fn play(t: &TestCore) -> (SessionId, SpawnedProcess) {
    install(&t.core).await;
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    let session = SessionId(t.core.view().running.expect("a running game").session);
    (session, process)
}

/// Moves the clock one round of the game watcher at a time until `done` is true, for at most
/// five seconds of it: the watcher's rounds are what change the desktop and the game's state.
async fn until(t: &TestCore, what: &str, done: impl Fn() -> bool) {
    assert!(t.until_clock(WATCH_EVERY, 250, done).await, "not {what} after five seconds of the clock");
}

async fn closed(events: &mut futures::stream::BoxStream<'static, Event>) -> ExitInfo {
    loop {
        if let Event::ProcessExited { exit, .. } = events.next().await.expect("the events ended first") {
            return exit;
        }
    }
}

/// What the Emulators screen shows as installed for `emu-a`.
async fn installed(core: &Core) -> Vec<String> {
    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = core.view();
    view.emulators.emulators.into_iter().find(|e| e.plugin == id("emu-a")).map(|e| e.installed).unwrap_or_default()
}

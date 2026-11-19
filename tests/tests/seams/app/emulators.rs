//! Emulators: installing before the first play, updates, and their lists.

use super::*;

#[tokio::test]
async fn playing_a_game_whose_emulator_isn_t_installed_asks_first_then_installs_and_plays() {
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_size = Some(20_000);
    let (t, _) = core_with(emu).await;
    no_auto_install(&t).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let prompt = t.core.view().install_prompt.expect("a prompt before anything downloads");
    assert_eq!((prompt.emulator.as_str(), prompt.version.as_str()), ("emu-a", "1.0"));
    assert_eq!((prompt.game.as_str(), prompt.size_bytes), ("A", Some(20_000)));
    assert!(installed(&t.core).await.is_empty() && t.spawner.spawned().is_empty(), "nothing happens yet");

    t.core.dispatch(Command::ConfirmInstall).await.unwrap();
    assert!(t.core.view().install_prompt.is_none());
    let process = t.spawner.wait_for(1).await;
    assert!(process.command.program.ends_with("builds/1.0/emu"), "{:?}", process.command.program);
}

#[tokio::test]
async fn a_plugin_named_by_its_console_names_its_program_where_a_sentence_means_the_program() {
    let host = FakeHost::new().first_run();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let names = "name = \"PS5\"\nemulator = \"KytyPS5\"";
    let t = host.plugin(emu.plugin_named("emu-a", names, FAKE_EMULATOR_PERMISSIONS)).boot_core().await;
    no_auto_install(&t).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let steps: Vec<String> = t.core.view().boot.steps.into_iter().map(|s| s.label).collect();
    // The boot screen names the console, as Home does; the Game Hub's sentence names the program.
    assert!(steps.contains(&"PS5".to_owned()), "the first start's step: {steps:?}");
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.expect("the Game Hub");
    assert_eq!(detail.launch_hint, "Play installs KytyPS5 first");

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let prompt = t.core.view().install_prompt.expect("a prompt before anything downloads");
    assert_eq!(prompt.emulator, "KytyPS5", "the program downloads, not the console");
    t.core.dispatch(Command::CancelInstall).await.unwrap();

    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let row = |t: &TestCore| t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("emu-a")).unwrap();
    assert_eq!((row(&t).name, row(&t).build_label), ("PS5".to_owned(), String::new()), "the list names the console");
    let builds = t.core.view().emulators.emulators.into_iter().find(|e| e.plugin == id("emu-a")).unwrap();
    assert_eq!(builds.name, "KytyPS5", "the builds are the program's");
    install(&t.core).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!((row(&t).build, row(&t).build_label), ("1.0".to_owned(), "KytyPS5 1.0".to_owned()));

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.spawner.wait_for(1).await;
    let running = t.core.view().running.expect("the game runs");
    assert_eq!(
        (running.emulator.as_str(), running.page.as_str()),
        ("KytyPS5", "PS5"),
        "the keys' page is the plugin's"
    );
}

#[tokio::test]
async fn not_now_leaves_the_emulator_uninstalled() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.core.dispatch(Command::CancelInstall).await.unwrap();
    assert!(t.core.view().install_prompt.is_none());
    assert!(installed(&t.core).await.is_empty());
    assert!(t.spawner.spawned().is_empty());
}

#[tokio::test]
async fn a_newer_build_installs_by_itself_when_no_game_runs() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()));
}

#[tokio::test]
async fn an_update_waits_until_the_emulator_s_game_closes() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"], "not while the game runs");

    process.exit(0);
    for _ in 0..100 {
        if installed(&t.core).await.contains(&"2.0".to_owned()) {
            return;
        }
        t.clock.advance(WATCH_EVERY).await;
    }
    panic!("the update didn't run after the game closed");
}

#[tokio::test]
async fn automatic_updates_can_be_turned_off() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    assert!(t.core.view().settings.auto_update, "on by default");
    t.core.dispatch(Command::Pref(Pref::AutoUpdate, false.into())).await.unwrap();
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"]);
    assert!(!t.core.view().settings.auto_update);
}

#[tokio::test]
async fn a_game_doesn_t_start_while_its_emulator_updates() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    emu.release("2.0");
    let gate = t.http.hold_after(&emu.build_url, 10);

    let core = t.core.clone();
    let update = tokio::spawn(async move { core.dispatch(Command::UpdateEmulator { plugin: id("emu-a") }).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;
    let err = t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap_err();
    assert!(err.to_string().contains("updating"), "{err}");
    assert!(t.spawner.spawned().is_empty());

    gate.release();
    update.await.unwrap().unwrap();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    assert_eq!(t.spawner.wait_for(1).await.command.args, ["G0"]);
}

#[tokio::test]
async fn an_emulator_build_doesn_t_install_twice_at_once() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let gate = t.http.hold_after(&emu.build_url, 10);
    let core = t.core.clone();
    let first = tokio::spawn(async move { install(&core).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;
    let saved = |t: &TestCore| t.core.kernel().config().get(&["app", "installs", "emu-a"]);
    assert!(!saved(&t).is_null(), "the install is saved while it runs");

    // A second install would wait at the same gate: it must be refused at once.
    for second in [
        Command::InstallEmulator { plugin: id("emu-a"), version: None },
        Command::InstallEmulator { plugin: id("emu-a"), version: Some("1.0".into()) },
    ] {
        let refused = tokio::time::timeout(Duration::from_secs(5), t.core.dispatch(second))
            .await
            .expect("the second install is refused, not run");
        let err = refused.unwrap_err();
        assert!(err.to_string().contains("emu-a is updating: wait for it"), "{err}");
    }
    assert!(busy(&t.core), "the first install still runs");
    assert!(!saved(&t).is_null(), "its record stays saved");

    gate.release();
    first.await.unwrap();
    assert!(!busy(&t.core));
    assert!(saved(&t).is_null(), "the record goes once the install is done");
    assert_eq!(installed(&t.core).await, ["1.0"]);
}

/// The boot step of `emu-a`: its detail and its state.
fn step(t: &TestCore) -> (String, String) {
    let step = t.core.view().boot.steps.into_iter().find(|s| s.label == "emu-a").expect("emu-a's boot step");
    (step.detail, step.state)
}

#[tokio::test]
async fn a_look_for_updates_skips_an_emulator_that_is_updating() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    emu.release("2.0");
    let gate = t.http.hold_after(&emu.build_url, 10);
    let core = t.core.clone();
    let update = tokio::spawn(async move { core.dispatch(Command::UpdateEmulator { plugin: id("emu-a") }).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    gate.release();
    update.await.unwrap().unwrap();
    assert!(t.core.view().notices.is_empty(), "{:?}", t.core.view().notices);
    assert_ne!(step(&t).1, "failed", "{:?}", step(&t));
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()));
}

#[tokio::test]
async fn an_update_that_waited_for_a_game_skips_an_emulator_that_is_updating() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    emu.release("2.0");
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"], "the update waits for the game");
    // The player updates by hand while the game runs.
    let gate = t.http.hold_after(&emu.build_url, 10);
    let core = t.core.clone();
    let update = tokio::spawn(async move { core.dispatch(Command::UpdateEmulator { plugin: id("emu-a") }).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;

    process.exit(0);
    until(&t, "the game closed", || t.core.view().running.is_none()).await;
    gate.release();
    update.await.unwrap().unwrap();
    // A late update from the closed game would say the emulator is up to date.
    for _ in 0..10 {
        t.clock.advance(WATCH_EVERY).await;
    }
    assert!(t.core.view().notices.is_empty(), "{:?}", t.core.view().notices);
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()));
}

#[tokio::test]
async fn an_install_record_that_no_install_holds_doesn_t_stop_updates() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    // A resume that stopped before its first step leaves its record, and nothing holds it.
    let saved = |t: &TestCore| t.core.kernel().config().get(&["app", "installs", "emu-a"]);
    let record = serde_json::json!({ "version": "1.0", "from": null, "phase": "downloading" });
    t.core.kernel().config().set(&["app", "installs", "emu-a"], record).unwrap();
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()));
    assert!(saved(&t).is_null(), "the update's end takes the record");
}

#[tokio::test]
async fn an_emulator_build_isn_t_removed_while_it_updates() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let t = host.plugin(emu.plugin_named("emu-a", "name = \"PS5\"", FAKE_EMULATOR_PERMISSIONS)).boot_core().await;
    install(&t.core).await;
    emu.release("2.0");
    let gate = t.http.hold_after(&emu.build_url, 10);
    let core = t.core.clone();
    let update = tokio::spawn(async move { core.dispatch(Command::UpdateEmulator { plugin: id("emu-a") }).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;

    let uninstall = Command::UninstallEmulator { plugin: id("emu-a"), version: "1.0".into() };
    let err = t.core.dispatch(uninstall).await.unwrap_err();
    // The refusal names the plugin as the launch refusal does, with the same word.
    assert!(err.to_string().contains("PS5 is updating: wait for it"), "{err}");
    gate.release();
    update.await.unwrap().unwrap();
    assert_eq!(installed(&t.core).await, ["2.0"], "the update ends as if no uninstall came");
}

#[tokio::test]
async fn opening_another_plugin_replaces_the_open_one_so_back_goes_to_the_list() {
    let t = FakeHost::new()
        .plugin(FakeEmulator::new(vec!["A"]).plugin("emu-a"))
        .plugin(FakeEmulator::new(vec!["B"]).plugin("emu-b"))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Plugin(id("emu-a")))).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Plugin(id("emu-b")))).await.unwrap();
    assert_eq!(t.core.view().plugin_page.map(|p| p.plugin.id), Some(id("emu-b")));

    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Emulators);
}

#[tokio::test]
async fn emulators_and_themes_each_have_their_own_list() {
    let t = FakeHost::new().plugin(FakeEmulator::new(vec!["A"]).plugin("emu-a")).boot_core().await;
    t.core.dispatch(Command::Navigate(Route::Themes)).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Themes);
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Emulators);
}

#[tokio::test]
async fn an_emulator_s_row_shows_its_installed_build_not_the_plugin_s_version() {
    let (t, _emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let row = |t: &TestCore| t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("emu-a")).unwrap();
    assert_eq!(row(&t).build, "", "not installed yet");

    install(&t.core).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(row(&t).build, "1.0");
}

#[tokio::test]
async fn emulator_updates_install_at_the_boot_then_every_6_hours() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    emu.release("2.0");
    t.clock.advance(Duration::from_secs(60)).await;
    assert_eq!(installed(&t.core).await, ["1.0"], "not before the boot");

    boot(&t).await;
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()), "the boot found and installed 2.0");

    emu.release("3.0");
    t.clock.advance(Duration::from_secs(5 * 3600)).await;
    assert!(!installed(&t.core).await.contains(&"3.0".to_owned()), "not before 6 hours");
    t.clock.advance(Duration::from_secs(3600)).await;
    let mut found = false;
    for _ in 0..50 {
        found = installed(&t.core).await.contains(&"3.0".to_owned());
        if found {
            break;
        }
        t.clock.advance(Duration::from_millis(100)).await;
    }
    assert!(found, "the check 6 hours later installed 3.0");
}

#[tokio::test]
async fn an_emulator_with_no_build_gets_its_newest_build_at_the_boot() {
    let (t, _emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    assert!(t.core.view().settings.auto_install, "on by default");
    assert!(installed(&t.core).await.is_empty(), "nothing before the boot");

    boot(&t).await;
    assert_eq!(installed(&t.core).await, ["1.0"]);
}

#[tokio::test]
async fn with_install_automatically_off_nothing_installs_at_the_boot() {
    let (t, _emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    no_auto_install(&t).await;
    assert!(!t.core.view().settings.auto_install);

    boot(&t).await;
    assert!(installed(&t.core).await.is_empty());
    assert!(t.http.requests().is_empty(), "nothing downloads");
}

#[tokio::test]
async fn an_emulator_that_uses_your_own_build_gets_no_build_at_the_boot() {
    let (t, _emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    // "Use your own build" is on, but no program is picked yet: the plugin lists no build.
    t.core.kernel().config().set(&["plugins", "emu-a", "emulator", "own_build"], true.into()).unwrap();

    boot(&t).await;
    assert!(installed(&t.core).await.is_empty());
    assert!(t.http.requests().is_empty(), "nothing downloads");
    let step = t.core.view().boot.steps.into_iter().find(|s| s.label == "emu-a").unwrap();
    assert_eq!((step.detail.as_str(), step.state.as_str()), ("Own build: none picked yet", "done"), "nothing failed");
}

#[tokio::test]
async fn a_failed_install_at_the_boot_is_a_notice_and_doesn_t_run_again() {
    // The build isn't served: its download fails.
    let t = FakeHost::new().plugin(FakeEmulator::new(vec!["A"]).plugin("emu-a")).boot_core().await;

    boot(&t).await;
    let tries = t.http.requests().len();
    assert!(tries > 0, "the boot tried the download");
    let notices = t.core.view().notices;
    assert_eq!(notices.iter().filter(|n| n.contains("Could not install emu-a")).count(), 1, "{notices:?}");
    let step = t.core.view().boot.steps.into_iter().find(|s| s.label == "emu-a").unwrap();
    assert_eq!((step.detail.as_str(), step.state.as_str()), ("Could not install", "failed"));

    // Two looks for updates later, six hours apart.
    t.clock.advance(Duration::from_secs(13 * 3600)).await;
    assert_eq!(t.http.requests().len(), tries, "not again while the app runs");
    assert!(installed(&t.core).await.is_empty());
}

#[tokio::test]
async fn an_update_at_the_boot_waits_until_the_emulator_s_game_closes() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    emu.release("2.0");

    boot(&t).await;
    assert_eq!(installed(&t.core).await, ["1.0"], "not while the game runs, and the boot doesn't wait for it");

    process.exit(0);
    for _ in 0..100 {
        if installed(&t.core).await.contains(&"2.0".to_owned()) {
            return;
        }
        t.clock.advance(WATCH_EVERY).await;
    }
    panic!("the update didn't run after the game closed");
}

/// The start's installs run side by side: a second emulator's download doesn't wait for the
/// first's to end.
#[tokio::test]
async fn the_start_s_installs_download_side_by_side() {
    let host = FakeHost::new();
    let mut gates = Vec::new();
    let mut emus = Vec::new();
    for id in ["emu-a", "emu-b"] {
        let mut emu = FakeEmulator::new(vec!["A"]);
        emu.build_url = format!("https://builds.example/{id}.zip");
        emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_480]);
        emu.build_size = Some(20_480);
        // Each download stops part-way until its gate opens.
        gates.push(host.http().hold_after(&emu.build_url, 8_192));
        emus.push((id, emu));
    }
    let mut host = host;
    for (id, emu) in emus {
        host = host.plugin(emu.plugin(id));
    }
    let t = host.boot_core().await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();

    let both = t
        .until_clock(Duration::from_millis(100), 50, || {
            let steps = t.core.view().boot.steps;
            ["emu-a", "emu-b"].iter().all(|id| steps.iter().any(|s| s.label == *id && s.detail == "Downloading · 40%"))
        })
        .await;
    assert!(both, "both download at once: {:?}", t.core.view().boot.steps);
    for gate in gates {
        gate.release();
    }
}

/// The boot screen stays while the start's install downloads, with the download's progress in
/// its checklist on any run, and goes once the build is in.
#[tokio::test]
async fn the_boot_screen_shows_the_start_s_install_and_waits_for_it() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_480]);
    emu.build_size = Some(20_480);
    let gate = host.http().hold_after(&emu.build_url, 8_192);
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let shown = t
        .until_clock(Duration::from_millis(100), 50, || {
            t.core.view().boot.steps.iter().any(|s| s.label == "emu-a" && s.detail == "Downloading · 40%")
        })
        .await;
    assert!(shown, "{:?}", t.core.view().boot.steps);
    let boot = t.core.view().boot;
    assert!(!boot.ready, "it waits for the install");
    assert!(boot.installing, "the checklist shows on a run that isn't the first");
    assert!(!boot.welcome);
    let step = boot.steps.iter().find(|s| s.label == "emu-a").unwrap();
    assert_eq!((step.detail.as_str(), step.state.as_str()), ("Downloading · 40%", "working"));
    assert!(boot.progress > 0.6 && boot.progress < 1.0, "past the library, not done: {}", boot.progress);

    gate.release();
    t.until(Area::Library, |core| core.view().boot.ready).await;
    let boot = t.core.view().boot;
    assert!(!boot.installing);
    let step = boot.steps.iter().find(|s| s.label == "emu-a").unwrap();
    assert_eq!((step.detail.as_str(), step.state.as_str()), ("Latest build", "done"));
}

/// A download that stops moving doesn't keep the player out: the boot screen goes after two
/// minutes without progress, and the install goes on behind Home.
#[tokio::test]
async fn a_start_install_that_hangs_lets_home_open_after_two_minutes() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let _gate = host.http().hold_after(&emu.build_url, 5_000);
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;
    let started = t.clock.now();

    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let ready = t.until_clock(Duration::from_secs(1), 200, || t.core.view().boot.ready).await;
    assert!(ready, "the boot screen went");
    let waited = t.clock.now() - started;
    assert!(waited >= Duration::from_secs(120) && waited < Duration::from_secs(130), "{waited:?}");
    let step = t.core.view().boot.steps.into_iter().find(|s| s.label == "emu-a").unwrap();
    assert_eq!(step.state, "working", "still downloading behind Home");
}

/// With nothing to install, the boot doesn't wait: the boot screen goes with the library.
#[tokio::test]
async fn with_nothing_to_install_the_boot_screen_goes_with_the_library() {
    let (t, _emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let boot = t.core.view().boot;
    assert!(boot.ready && !boot.installing);
}

/// Home lists the systems by name, after All games; the Emulators sheet lists the emulators by
/// name, then the services by name, whatever order the plugins loaded in.
#[tokio::test]
async fn systems_and_emulators_are_listed_by_name_and_services_last() {
    let t = FakeHost::new()
        .plugin(launcher_testkit::FakeStore::new("store-z").build())
        .plugin(launcher_testkit::FakeCatalog::new("emu-z").named("Zeta").system("PS5").playable().games([]).build())
        .plugin(launcher_testkit::FakeCatalog::new("emu-m").named("Mid").games([]).build())
        .plugin(launcher_testkit::FakeStore::new("store-a").build())
        .plugin(launcher_testkit::FakeCatalog::new("emu-a").named("Alpha").system("PS4").playable().games([]).build())
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = t.core.view();
    let systems: Vec<&str> = view.emulators.emulators.iter().map(|e| e.system_name.as_str()).collect();
    assert_eq!(systems, ["PlayStation 4", "PlayStation 5"], "Home's systems by name");
    let rows: Vec<(&str, &str)> =
        view.emulators.emulator_plugins.iter().map(|p| (p.kind.as_str(), p.id.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("emulator", "emu-a"),
            ("emulator", "emu-m"),
            ("emulator", "emu-z"),
            ("service", "store-a"),
            ("service", "store-z")
        ],
        "the emulators by name, then the services by name"
    );
}

//! Seam 1, core app API: the machine's settings from a system plugin. The core reads the
//! network, Wi-Fi, Bluetooth, sound, power and battery from the first system plugin that
//! provides each, shows each one nobody provides as unavailable, runs long work (joining,
//! pairing, installing) as jobs, and stays up when the plugin hangs or panics.

use std::path::PathBuf;

use launcher_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection, InstallDisk, Installed, NetworkStatus, OsUpdate,
    WifiNetwork,
};
use launcher_core::system::{Place, SystemCommand};
use launcher_core::{Area, Command};
use launcher_testkit::{FakeHost, FakeSystem, StubPlugin};

fn wifi(ssid: &str, signal: u8, secured: bool) -> WifiNetwork {
    WifiNetwork { ssid: ssid.into(), signal, secured, saved: false, active: false }
}

fn device(address: &str, name: &str, paired: bool, connected: bool) -> BluetoothDevice {
    BluetoothDevice { address: address.into(), name: name.into(), paired, connected }
}

fn output(name: &str, description: &str, current: bool) -> AudioOutput {
    AudioOutput { name: name.into(), description: description.into(), current }
}

/// A machine on wired network, with Wi-Fi in reach, one controller connected, sound on the TV
/// and a laptop's battery.
fn machine() -> FakeSystem {
    let system = FakeSystem::new();
    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wired, name: "Wired connection 1".into(), signal: None };
        m.wifi = vec![wifi("Home", 82, true), wifi("Cafe", 30, false)];
        m.wifi_scanned = vec![wifi("Home", 82, true), wifi("Cafe", 30, false), wifi("Neighbour", 10, true)];
        m.passwords = vec![("Home".into(), "secret123".into())];
        m.bluetooth_on = true;
        m.devices = vec![device("AA:BB", "DualSense", true, true), device("CC:DD", "Headset", true, false)];
        m.found = vec![device("EE:FF", "Xbox Controller", false, false)];
        m.outputs = vec![output("hdmi", "HDMI / DisplayPort", true), output("headset", "Headset", false)];
        m.level = AudioLevel { volume: 40, muted: false };
        m.battery = Some(Battery { percent: 18, charging: false });
    });
    system
}

async fn ask(cmd: SystemCommand, t: &launcher_testkit::TestCore) {
    t.core.dispatch(Command::System(cmd)).await.unwrap();
}

#[tokio::test]
async fn without_a_system_plugin_each_setting_is_unavailable_and_says_why() {
    let t = FakeHost::new().boot_core().await;
    t.core.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();
    let vm = t.core.view().system;
    assert!(!vm.whole_screen, "a desktop");
    assert!(!vm.area("network").ready() && !vm.area("audio").ready() && !vm.area("bluetooth").ready());
    assert!(!(vm.area("power").ready() && vm.area("power").reason.is_empty()));
    assert!(!vm.area("network").reason.is_empty());
    assert!(!vm.battery_shown);
    let joined = t.core.dispatch(Command::System(SystemCommand::JoinWifi { ssid: "Home".into(), password: None }));
    assert!(joined.await.is_err(), "nothing to join with");
}

#[tokio::test]
async fn the_machine_s_network_sound_bluetooth_and_battery_are_read() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(vm.area("network").ready() && vm.area("audio").ready() && vm.area("bluetooth").ready());
    assert!(vm.area("power").ready() && vm.area("power").reason.is_empty());
    assert_eq!((vm.connection.as_str(), vm.network_name.as_str()), ("wired", "Wired connection 1"));
    assert_eq!(vm.wifi.iter().map(|w| (w.ssid.as_str(), w.bars)).collect::<Vec<_>>(), [("Home", 4), ("Cafe", 2)]);
    assert_eq!((vm.volume, vm.muted), (40, false));
    assert_eq!(
        vm.outputs.iter().filter(|o| o.current).map(|o| o.description.as_str()).collect::<Vec<_>>(),
        ["HDMI / DisplayPort"]
    );
    assert!(vm.bluetooth_on);
    assert_eq!(vm.connected_devices, 1);
    assert_eq!((vm.battery_shown, vm.battery_percent, vm.battery_charging), (true, 18, false));
}

#[tokio::test]
async fn a_scan_finds_more_networks_and_joining_a_secured_one_takes_its_password() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::ScanWifi, &t).await;
    assert!(t.core.view().system.wifi.iter().any(|w| w.ssid == "Neighbour"), "the scan's networks");

    ask(SystemCommand::JoinWifi { ssid: "Home".into(), password: Some("wrong".into()) }, &t).await;
    let job = t.core.view().system.jobs.last().cloned().expect("a join job");
    assert_eq!((job.kind.as_str(), job.target.as_str(), job.state.as_str()), ("join", "Home", "failed"));
    assert!(job.text.contains("password is wrong"), "{}", job.text);

    ask(SystemCommand::JoinWifi { ssid: "Home".into(), password: Some("secret123".into()) }, &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("done", "Connected"));
    // The job's end reads the machine again.
    t.clock.advance(std::time::Duration::from_millis(50)).await;
    let vm = t.core.view().system;
    assert_eq!((vm.connection.as_str(), vm.network_name.as_str(), vm.signal_bars), ("wifi", "Home", 4));
}

#[tokio::test]
async fn a_running_join_shows_until_it_ends_and_can_be_cancelled() {
    let system = machine();
    system.with(|m| m.hold_jobs = true);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::JoinWifi { ssid: "Cafe".into(), password: None }, &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str(), job.percent), ("running", "Joining Cafe", -1));

    t.core.dispatch(Command::System(SystemCommand::CancelJob(job.id as u64))).await.unwrap();
    system.finish_jobs();
    assert_eq!(t.core.view().system.jobs.last().unwrap().state, "done");
    assert!(t.core.dispatch(Command::System(SystemCommand::CancelJob(job.id as u64))).await.is_err(), "it ended");
}

#[tokio::test]
async fn a_scan_finds_a_controller_to_pair_and_pairing_connects_it() {
    let system = machine();
    system.with(|m| m.pair_refusals = vec![("CC:DD".into(), "The device didn't answer".into())]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::ScanBluetooth, &t).await;
    t.clock.advance(std::time::Duration::from_millis(50)).await;
    let found = t.core.view().system.devices;
    assert!(found.iter().any(|d| d.name == "Xbox Controller" && !d.paired), "{found:?}");

    ask(SystemCommand::Pair("EE:FF".into()), &t).await;
    t.clock.advance(std::time::Duration::from_millis(50)).await;
    let vm = t.core.view().system;
    assert_eq!(vm.jobs.last().map(|j| (j.kind.as_str(), j.state.as_str())), Some(("pair", "done")));
    assert!(vm.devices.iter().any(|d| d.address == "EE:FF" && d.paired && d.connected));
    assert_eq!(vm.connected_devices, 2);

    ask(SystemCommand::Pair("CC:DD".into()), &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("failed", "The device didn't answer"));

    ask(SystemCommand::DisconnectDevice("AA:BB".into()), &t).await;
    ask(SystemCommand::RemoveDevice("CC:DD".into()), &t).await;
    ask(SystemCommand::SetBluetooth(false), &t).await;
    let vm = t.core.view().system;
    assert!(!vm.bluetooth_on);
    assert!(vm.devices.iter().all(|d| d.address != "CC:DD"));
    assert!(vm.devices.iter().any(|d| d.address == "AA:BB" && !d.connected));
}

#[tokio::test]
async fn volume_mute_and_the_output_change_and_show_at_once() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    ask(SystemCommand::SetVolume(150), &t).await;
    assert_eq!(t.core.view().system.volume, 100, "at most 100");
    assert_eq!(system.read(|m| m.level.volume), 100);
    ask(SystemCommand::SetMuted(true), &t).await;
    assert!(t.core.view().system.muted);
    ask(SystemCommand::SetOutput("headset".into()), &t).await;
    let current: Vec<_> = t.core.view().system.outputs.into_iter().filter(|o| o.current).map(|o| o.name).collect();
    assert_eq!(current, ["headset"]);
    assert!(t.core.dispatch(Command::System(SystemCommand::SetOutput("nope".into()))).await.is_err());
}

#[tokio::test]
async fn restart_and_shut_down_are_asked_of_the_system() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Restart, &t).await;
    ask(SystemCommand::ShutDown, &t).await;
    assert_eq!(system.read(|m| (m.restarts, m.shutdowns)), (1, 1));
}

#[tokio::test]
async fn a_setting_the_plugin_can_t_use_here_shows_its_reason() {
    let system = machine();
    system.with(|m| m.unavailable = vec![("bluetooth", "No Bluetooth adapter".into())]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(!vm.area("bluetooth").ready());
    assert_eq!(vm.area("bluetooth").reason, "No Bluetooth adapter");
    assert!(vm.area("network").ready(), "the rest still works");
}

#[tokio::test]
async fn a_plugin_providing_some_capabilities_leaves_the_others_unavailable() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin_with(&["power"])).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(vm.area("power").ready() && vm.area("power").reason.is_empty() && vm.battery_shown);
    assert!(!vm.area("network").ready() && !vm.area("audio").ready() && !vm.area("bluetooth").ready());
}

#[tokio::test]
async fn a_hanging_or_panicking_system_plugin_leaves_the_core_working() {
    let system = machine();
    system.with(|m| {
        m.hangs = vec!["status"];
        m.panics = vec!["battery"];
    });
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert_eq!(vm.connection, "none", "the hung read gave nothing");
    assert!(!vm.battery_shown, "the panicking read gave nothing");
    assert_eq!(vm.volume, 40, "the other reads still came");
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
}

/// Where the launcher runs doesn't decide what the player can manage: on a desktop and as the
/// whole screen the view lists the same areas in the same states, from the plugin alone.
#[tokio::test]
async fn on_a_desktop_every_area_is_read_as_on_the_whole_screen() {
    let states =
        |vm: &launcher_ui_api::SystemVm| vm.areas.iter().map(|a| (a.name.clone(), a.state.clone())).collect::<Vec<_>>();
    let mut seen = Vec::new();
    for place in [Place::default(), Place { whole_screen: true, own_os: true, stick: false }] {
        let system = machine();
        let t = FakeHost::new().place(place).plugin(system.plugin()).boot_core().await;
        ask(SystemCommand::Refresh, &t).await;
        let vm = t.core.view().system;
        for area in ["network", "audio", "bluetooth", "power"] {
            assert!(vm.area(area).ready(), "{place:?}: {area} is ready");
        }
        assert_eq!((vm.connection.as_str(), vm.volume, vm.connected_devices), ("wired", 40, 1), "{place:?}");
        seen.push(states(&vm));
    }
    assert_eq!(seen[0], seen[1], "the same areas, in the same states");
}

#[tokio::test]
async fn the_view_says_whether_marah_is_the_whole_screen() {
    let place = Place { whole_screen: true, own_os: false, stick: false };
    let t = FakeHost::new().place(place).boot_core().await;
    let vm = t.core.view().system;
    assert!(vm.whole_screen && !vm.own_os && !vm.stick);
}

#[tokio::test]
async fn only_a_system_plugin_may_provide_the_machine_s_settings_or_run_programs_long() {
    // The kind governs privileges only: a system plugin may provide data capabilities too.
    let service = StubPlugin::from_toml(
        "id = \"svc\"\nname = \"Svc\"\nversion = \"1.0.0\"\nkind = \"service\"\nhost_api = \"*\"\ncapabilities = [\"network\"]\n",
    );
    let system_with_games = StubPlugin::from_toml(
        "id = \"sys\"\nname = \"Sys\"\nversion = \"1.0.0\"\nkind = \"system\"\nhost_api = \"*\"\ncapabilities = [\"catalog\"]\n",
    );
    let long = StubPlugin::from_toml(
        "id = \"emu\"\nname = \"Emu\"\nversion = \"1.0.0\"\nkind = \"emulator\"\nhost_api = \"*\"\ncapabilities = []\n\
         [permissions]\nprocess = [\"system:sleep\"]\nlong_running = [\"system:sleep\"]\n",
    );
    let unlisted = StubPlugin::from_toml(
        "id = \"sys2\"\nname = \"Sys2\"\nversion = \"1.0.0\"\nkind = \"system\"\nhost_api = \"*\"\ncapabilities = []\n\
         [permissions]\npolkit = [\"system:systemctl\"]\n",
    );
    let t = FakeHost::new().plugin(service).plugin(system_with_games).plugin(long).plugin(unlisted).boot_core().await;
    let rejected: Vec<String> = t.report.rejected.iter().map(|r| r.source.clone()).collect();
    for id in ["svc", "emu", "sys2"] {
        assert!(rejected.contains(&id.to_string()), "{id} is refused: {rejected:?}");
    }
    assert!(!rejected.contains(&"sys".to_string()), "a system plugin may list games: {rejected:?}");
}

#[tokio::test]
async fn a_disk_s_size_has_one_decimal_below_10_gb_and_is_whole_from_there() {
    let system = machine();
    let disk = |path: &str, bytes: u64| InstallDisk {
        path: PathBuf::from(path),
        bytes,
        windows: false,
        usb: true,
        model: "SanDisk Ultra".into(),
    };
    system.with(|m| {
        // A "32 GB" stick holds a little less than 32 billion bytes.
        m.disks = vec![disk("/dev/sdb", 1_900_000_000), disk("/dev/sdc", 31_914_983_424)];
    });
    let place = Place { whole_screen: true, own_os: true, stick: true };
    let t = FakeHost::new().place(place).plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let sizes: Vec<String> = t.core.view().system.disks.iter().map(|d| d.size.clone()).collect();
    assert_eq!(sizes, ["1.9 GB", "32 GB"]);
}

#[tokio::test]
async fn on_the_stick_the_disks_are_listed_and_installing_reports_its_progress() {
    let system = machine();
    system.with(|m| {
        m.disks = vec![InstallDisk {
            path: PathBuf::from("/dev/nvme0n1"),
            bytes: 512_000_000_000,
            windows: true,
            usb: false,
            model: "Samsung 970".into(),
        }];
        m.install_steps = vec![(10, "Preparing the disk".into()), (60, "Copying Marah OS".into())];
        m.install_result = Some(Ok(Installed { edition: "minimal-nvidia".into(), mok: true, nvidia_later: false }));
        m.hold_jobs = true;
    });
    let place = Place { whole_screen: true, own_os: true, stick: true };
    let t = FakeHost::new().place(place).plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let disk = t.core.view().system.disks.first().cloned().expect("the disk");
    assert_eq!((disk.path.as_str(), disk.size.as_str(), disk.windows), ("/dev/nvme0n1", "512 GB", true));

    ask(SystemCommand::InstallOs(PathBuf::from("/dev/nvme0n1")), &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!(
        (job.kind.as_str(), job.state.as_str(), job.percent, job.text.as_str()),
        ("install", "running", 60, "Copying Marah OS")
    );
    system.finish_jobs();
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("done", "Marah OS is installed"));
    assert!(job.mok && !job.nvidia_later, "Secure Boot's key screen comes at the restart");
    assert_eq!(system.read(|m| m.installed_on.clone()), [PathBuf::from("/dev/nvme0n1")]);

    ask(SystemCommand::ChoosePartitions, &t).await;
    assert_eq!(system.read(|m| m.partition_choices), 1);
}

/// A cancel of the install is a request to the installer: the job runs on, and ends as the
/// installer says. Here the installer had started the erase, so it went on to the end.
#[tokio::test]
async fn a_cancelled_os_install_ends_as_the_installer_says() {
    let system = machine();
    system.with(|m| {
        m.install_steps = vec![(60, "Copying Marah OS".into())];
        m.install_result = Some(Ok(Installed { edition: "minimal".into(), mok: false, nvidia_later: false }));
        m.hold_jobs = true;
    });
    let place = Place { whole_screen: true, own_os: true, stick: true };
    let t = FakeHost::new().place(place).plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::InstallOs(PathBuf::from("/dev/nvme0n1")), &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();

    t.core.dispatch(Command::System(SystemCommand::CancelJob(job.id as u64))).await.expect("accepted");
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("running", "Copying Marah OS"));

    system.finish_jobs();
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("done", "Marah OS is installed"));
}

#[tokio::test]
async fn away_from_the_stick_marah_os_isn_t_installed() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    let install = t.core.dispatch(Command::System(SystemCommand::InstallOs(PathBuf::from("/dev/sda"))));
    assert!(install.await.is_err());
    assert!(system.read(|m| m.installed_on.is_empty()));
    ask(SystemCommand::Refresh, &t).await;
    assert!(t.core.view().system.disks.is_empty());
}

#[tokio::test]
async fn on_marah_os_a_staged_update_says_restart_and_a_newer_launcher_asks_the_os_to_check() {
    use launcher_core::launcher_update::Install;
    let system = machine();
    system.with(|m| m.os_update = OsUpdate::Staged { version: "2026.10.24".into() });
    let host = FakeHost::new()
        .install(Install::MarahOs)
        .place(Place { whole_screen: true, own_os: true, stick: false })
        .plugin(system.plugin());
    let latest = "https://github.com/owner/marah-launcher/releases/latest/download/latest.json";
    host.http().serve(latest, br#"{ "version": "9.9.9", "assets": {} }"#.to_vec());
    let t = host.boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert_eq!(vm.os_update, "staged");
    assert!(vm.os_update_text.contains("Restart") && vm.os_update_text.contains("2026.10.24"), "{}", vm.os_update_text);

    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    // The check runs in a task of its own.
    t.clock.run_until_idle().await;
    assert_eq!(system.read(|m| m.os_checks), 1, "the OS looks for the update that brings it");
}

#[tokio::test]
async fn the_machine_is_read_again_every_five_seconds_on_the_clock_and_not_between() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    t.until(Area::System, |core| core.view().system.volume == 40).await;

    // Changed outside Marah Launcher: the next read is five seconds after the first.
    system.with(|m| m.level.volume = 70);
    t.clock.advance(std::time::Duration::from_millis(4500)).await;
    assert_eq!(t.core.view().system.volume, 40, "not read again yet");
    t.clock.advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(t.core.view().system.volume, 70);
}

const AT_20: &str = "Battery at 20%. Plug in soon.";
const AT_10: &str = "Battery at 10%. Save your game and plug in.";

/// A machine with a battery, or without one, for the low battery notices.
struct Unplugged {
    system: FakeSystem,
    t: launcher_testkit::TestCore,
}

impl Unplugged {
    async fn new(battery: Option<Battery>) -> Self {
        let system = machine();
        system.with(|m| m.battery = battery);
        let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
        Unplugged { system, t }
    }

    /// The notices after the battery reads `battery`.
    async fn read(&self, battery: Option<Battery>) -> Vec<String> {
        self.system.with(|m| m.battery = battery);
        ask(SystemCommand::Refresh, &self.t).await;
        self.t.core.view().notices
    }
}

fn on_battery(percent: u8) -> Option<Battery> {
    Some(Battery { percent, charging: false })
}

fn charging(percent: u8) -> Option<Battery> {
    Some(Battery { percent, charging: true })
}

#[tokio::test]
async fn on_battery_a_notice_shows_when_the_level_falls_to_20_and_again_at_10() {
    let pc = Unplugged::new(on_battery(50)).await;
    assert!(pc.read(on_battery(21)).await.is_empty(), "above 20%");
    assert_eq!(pc.read(on_battery(20)).await, [AT_20]);
    assert_eq!(pc.read(on_battery(11)).await, [AT_20], "no new notice between 20% and 10%");
    assert_eq!(pc.read(on_battery(10)).await, [AT_20, AT_10]);
    assert_eq!(pc.read(on_battery(3)).await, [AT_20, AT_10], "no new notice under 10%");
}

#[tokio::test]
async fn a_low_battery_notice_does_not_repeat_while_the_level_stays_low() {
    let pc = Unplugged::new(on_battery(18)).await;
    assert_eq!(pc.read(on_battery(18)).await, [AT_20], "the first read under 20% warns");
    dismiss_all(&pc.t).await;
    assert!(pc.read(on_battery(17)).await.is_empty());
    assert!(pc.read(on_battery(21)).await.is_empty(), "a level that rises without charging");
    assert!(pc.read(on_battery(20)).await.is_empty(), "does not warn again");
    pc.system.with(|m| m.panics = vec!["battery"]);
    assert!(pc.read(on_battery(20)).await.is_empty(), "a failed read");
    pc.system.with(|m| m.panics.clear());
    assert!(pc.read(on_battery(19)).await.is_empty(), "does not warn again");
}

#[tokio::test]
async fn a_fall_past_both_levels_at_once_shows_only_the_10_notice() {
    let pc = Unplugged::new(on_battery(25)).await;
    assert_eq!(pc.read(on_battery(8)).await, [AT_10]);
    assert_eq!(pc.read(on_battery(7)).await, [AT_10]);
}

#[tokio::test]
async fn charging_resets_the_low_battery_notices() {
    let pc = Unplugged::new(on_battery(30)).await;
    assert_eq!(pc.read(on_battery(9)).await, [AT_10]);
    dismiss_all(&pc.t).await;
    assert!(pc.read(charging(9)).await.is_empty(), "no notice while it charges");
    assert!(pc.read(charging(30)).await.is_empty());
    assert_eq!(pc.read(on_battery(20)).await, [AT_20], "unplugged again, it warns again");
    assert_eq!(pc.read(on_battery(10)).await, [AT_20, AT_10]);
}

#[tokio::test]
async fn no_low_battery_notice_while_charging_or_without_a_battery() {
    let pc = Unplugged::new(charging(15)).await;
    assert!(pc.read(charging(5)).await.is_empty(), "charging");
    let desktop = Unplugged::new(None).await;
    assert!(desktop.read(None).await.is_empty(), "a desktop PC");
}

#[tokio::test]
async fn a_low_battery_notice_comes_while_a_game_runs() {
    let system = machine();
    system.with(|m| m.battery = on_battery(50));
    let catalog = launcher_testkit::FakeCatalog::new("emu-a").playable().games(["Alpha"]).build();
    let t = FakeHost::new().plugin(system.plugin()).plugin(catalog).boot_core().await;
    t.core.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh)).await.unwrap();
    let game = "emu-a:G0".parse().unwrap();
    t.core.dispatch(Command::Session(launcher_core::SessionCmd::Launch(game))).await.unwrap();
    t.spawner.wait_for(1).await;
    assert!(t.core.game_running());
    let pc = Unplugged { system, t };
    assert_eq!(pc.read(on_battery(20)).await, [AT_20]);
}

async fn dismiss_all(t: &launcher_testkit::TestCore) {
    t.core.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll)).await.unwrap();
}

/// A slow read holds none of the other parts: the drives, Bluetooth, the sound and the battery
/// are there while the network still reads, and the network comes when its read ends.
#[tokio::test]
async fn a_slow_network_read_holds_none_of_the_other_parts() {
    let system = machine();
    system.with(|m| m.slow.push(("status", std::time::Duration::from_secs(20))));
    let t = FakeHost::new().call_timeout(std::time::Duration::from_secs(60)).plugin(system.plugin()).boot_core().await;
    t.clock.advance(std::time::Duration::from_secs(1)).await;
    let vm = t.core.view().system;
    assert!(vm.area("drives").ready(), "the drives are read");
    assert!(vm.bluetooth_on && vm.volume == 40 && vm.battery_percent == 18, "the other parts are read: {vm:?}");
    assert_eq!(vm.connection, "none", "the network still reads");
    t.clock.advance(std::time::Duration::from_secs(20)).await;
    assert_eq!(t.core.view().system.connection, "wired", "its read ended");
}

/// The parts are read at once, and so are the independent reads inside one: each read begins
/// before any of them ends.
#[tokio::test]
async fn the_parts_and_their_reads_run_at_once() {
    let system = machine();
    let reads = ["status", "wifi", "powered", "devices", "outputs", "level", "battery", "drives.unavailable"];
    system.with(|m| m.slow = reads.iter().map(|r| (*r, std::time::Duration::from_secs(1))).collect());
    let t = FakeHost::new().call_timeout(std::time::Duration::from_secs(60)).plugin(system.plugin()).boot_core().await;
    t.clock.advance(std::time::Duration::from_secs(2)).await;
    let calls = system.read(|m| m.calls.clone());
    let first = |name: &str| *calls.iter().find(|(method, _, _)| *method == name).unwrap_or_else(|| panic!("{name}"));
    let last_began = reads.iter().map(|r| first(r).1).max().unwrap();
    let first_ended = reads.iter().map(|r| first(r).2).min().unwrap();
    assert!(last_began < first_ended, "every read began before one ended: {calls:?}");
}

/// The one reader's rule for a read that fails: the first leaves the area unsupported, with the
/// error, so it doesn't stay loading; a later one keeps what the last read said.
#[tokio::test]
async fn a_failed_read_keeps_the_last_state_and_a_failed_first_read_says_why() {
    let system = machine();
    system.with(|m| m.panics = vec!["level"]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert_eq!(vm.area("audio").state, "unsupported", "{vm:?}");
    assert!(!vm.area("audio").ready());
    assert!(vm.area("audio").reason.starts_with("Couldn't read it"), "{:?}", vm.area("audio"));

    system.with(|m| m.panics.clear());
    ask(SystemCommand::Refresh, &t).await;
    assert_eq!((t.core.view().system.area("audio").ready(), t.core.view().system.volume), (true, 40));

    system.with(|m| {
        m.panics = vec!["level"];
        m.level.volume = 10;
    });
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert_eq!((vm.area("audio").ready(), vm.volume), (true, 40), "the failed read changed nothing");
}

/// Restart and shut down that the system refuses here: the battery still shows, and Restart
/// is refused before anything closes.
#[tokio::test]
async fn a_power_area_that_refuses_restart_still_shows_the_battery() {
    let system = machine();
    system.with(|m| m.unavailable = vec![("power", "Restart from the system's own menu".into())]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(vm.area("power").ready() && !vm.area("power").reason.is_empty() && vm.battery_shown, "{vm:?}");
    assert_eq!(vm.area("power").reason, "Restart from the system's own menu");
    let restart = t.core.dispatch(Command::System(SystemCommand::Restart)).await;
    assert!(restart.unwrap_err().to_string().contains("system's own menu"));
    assert_eq!(system.read(|m| m.restarts), 0);
}

/// A refusal of restart and shut down with no words still says why: an empty reason means they
/// work, so the themes would show Restart and Shut down.
#[tokio::test]
async fn a_power_refusal_with_no_reason_still_says_why() {
    let system = machine();
    system.with(|m| m.unavailable = vec![("power", String::new())]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(vm.area("power").ready() && !vm.area("power").reason.is_empty(), "{vm:?}");
    assert!(t.core.dispatch(Command::System(SystemCommand::Restart)).await.is_err());
    assert_eq!(system.read(|m| m.restarts), 0);
}

/// A system plugin's VPN area says what the system is with its first read. Before it nothing is
/// known: "Download only through a VPN" may be turned on. After it, a system that can't bind the
/// downloads to a VPN says why, and the setting can't be turned on.
#[tokio::test]
async fn the_vpn_area_s_reason_shows_after_its_first_read() {
    let system = machine();
    system.with(|m| m.unavailable = vec![("vpn", "Not available on Windows".into())]);
    let t = FakeHost::new().plugin(system.plugin_with(&["vpn"])).boot_core().await;
    t.core.dispatch(Command::Vpn(launcher_core::VpnCmd::SetOnly(true))).await.unwrap();
    t.core.dispatch(Command::Vpn(launcher_core::VpnCmd::SetOnly(false))).await.unwrap();
    ask(SystemCommand::Refresh, &t).await;
    assert_eq!(t.core.view().settings.vpn.problem, "Not available on Windows");
    assert!(t.core.dispatch(Command::Vpn(launcher_core::VpnCmd::SetOnly(true))).await.is_err());
}

/// The view lists every area, in the contract's order, each in its state: what a plugin provides
/// is ready, what nobody provides is absent and says so, and nothing runs in either.
#[tokio::test]
async fn the_view_lists_every_area_in_the_contract_s_order() {
    let t = FakeHost::new().plugin(machine().plugin_with(&["power"])).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let areas = t.core.view().system.areas;
    let names: Vec<&str> = areas.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, launcher_ui_api::AREAS);
    let state = |name: &str| areas.iter().find(|a| a.name == name).map(|a| (a.state.clone(), a.reason.is_empty()));
    assert_eq!(state("power"), Some(("ready".into(), true)));
    assert_eq!(state("bluetooth"), Some(("absent".into(), false)), "an absent area says why");
    assert!(areas.iter().all(|a| !a.busy));
}

/// The ids of the tray items in the bar, in order.
fn bar(core: &launcher_core::Core) -> Vec<String> {
    let view = core.view();
    view.tray_bar.iter().map(|&i| view.tray[i as usize].id.clone()).collect()
}

/// The player's choice for one tray item is saved by its id, the others keep theirs, an id the
/// tray doesn't have is refused, and the choices come back at the next start.
#[tokio::test]
async fn the_tray_s_choices_are_saved_by_id_and_kept_after_a_restart() {
    let t = FakeHost::new().plugin(machine().plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    assert_eq!(bar(&t.core), ["network", "bluetooth", "power"], "the internet, the connected controller, the battery");

    let choose = |item: &str, show| Command::Tray { item: item.into(), show };
    t.core.dispatch(choose("audio", launcher_core::TrayShow::Always)).await.unwrap();
    t.core.dispatch(choose("network", launcher_core::TrayShow::Tray)).await.unwrap();
    assert_eq!(bar(&t.core), ["audio", "bluetooth", "power"]);
    assert_eq!(
        t.core.kernel().config().get(&["app", "tray"]),
        serde_json::json!({"audio": "always", "network": "tray"}),
        "one key per item the player chose"
    );
    assert!(t.core.dispatch(choose("clock", launcher_core::TrayShow::Always)).await.is_err(), "no such item");

    let again = launcher_core::Core::start(t.kernel.clone()).await;
    again.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();
    assert_eq!(bar(&again), ["audio", "bluetooth", "power"], "the choices are kept");
    assert_eq!(again.view().tray_item("network").show, "tray");
}

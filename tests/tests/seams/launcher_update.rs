//! Seam 1, core app API: the launcher's own update. The release's latest.json says the
//! newest version and each kind of install's package; the core downloads this install's,
//! checks its SHA-256, and puts it in the way that install takes: kept for the app to swap in
//! at the next start, handed to the system's package manager, or left to Marah OS.

use std::io::Write;

use launcher_core::launcher_update::{Install, apply_staged, package_key, staged};
use launcher_core::{Command, PackageError, PackageManager};
use launcher_testkit::FakeHost;

const LATEST: &str = "https://github.com/owner/marah-launcher/releases/latest/download/latest.json";

/// A package archive as the release makes it: one top folder holding the launcher.
fn package(marker: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(marker.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "marah-launcher-linux-x86_64/VERSION", marker.as_bytes()).unwrap();
        tar.into_inner().unwrap().finish().unwrap().flush().unwrap();
    }
    bytes
}

/// This system's package for an unpacked install, as the release names it, and its bytes: a
/// zip on Windows (its files go in one by one), a tar.gz elsewhere (the folder is swapped).
fn folder_package(marker: &str) -> (&'static str, Vec<u8>) {
    if !cfg!(windows) {
        let url = "https://github.com/owner/marah-launcher/releases/download/v9.9.9/marah-launcher-linux-x86_64.tar.gz";
        return (url, package(marker));
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("marah-launcher-windows-x86_64/VERSION", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(marker.as_bytes()).unwrap();
    let url = "https://github.com/owner/marah-launcher/releases/download/v9.9.9/marah-launcher-windows-x86_64.zip";
    (url, zip.finish().unwrap().into_inner())
}

fn manifest(version: &str, url: &str, sha256: &str) -> Vec<u8> {
    manifest_for(Install::Folder, version, url, sha256)
}

fn manifest_for(install: Install, version: &str, url: &str, sha256: &str) -> Vec<u8> {
    let mut assets = serde_json::Map::new();
    assets.insert(package_key(install).unwrap(), serde_json::json!({ "url": url, "sha256": sha256 }));
    serde_json::json!({ "version": version, "assets": assets }).to_string().into_bytes()
}

#[tokio::test]
async fn a_newer_release_is_found_downloaded_checked_and_kept_for_the_next_start() {
    let host = FakeHost::new();
    let (url, bytes) = folder_package("9.9.9");
    let sha = host.http().serve(url, bytes);
    host.http().serve(LATEST, manifest("9.9.9", url, &sha));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!((update.state.as_str(), update.version.as_str()), ("available", "9.9.9"));

    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!(update.state, "ready", "{}", update.text);
    let kept = staged(t.data_dir()).expect("kept for the next start");
    assert_eq!(kept.version, "9.9.9");

    // The next start: the kept package replaces the install folder.
    let install = tempfile::tempdir().unwrap();
    let root = install.path().join("marah-launcher-linux-x86_64");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("VERSION"), "0.1.0").unwrap();
    assert_eq!(apply_staged(t.data_dir(), &root).unwrap(), Some("9.9.9".into()));
    assert_eq!(std::fs::read_to_string(root.join("VERSION")).unwrap(), "9.9.9");
    assert!(staged(t.data_dir()).is_none(), "it is tried once");
    // The old version, still running, stays until the start after: beside the folder, or on
    // Windows, where a running program's folder can't be renamed, each file beside itself.
    let old =
        if cfg!(windows) { root.join("VERSION.old") } else { install.path().join("marah-launcher-linux-x86_64.old") };
    let old_version = if cfg!(windows) { old.clone() } else { old.join("VERSION") };
    assert_eq!(std::fs::read_to_string(old_version).unwrap(), "0.1.0");
    launcher_core::launcher_update::clean_old_files(&root);
    assert!(!old.exists(), "the next start removes it");
}

#[tokio::test]
async fn a_download_that_doesn_t_match_its_sha256_is_refused() {
    let host = FakeHost::new();
    let url = "https://github.com/owner/marah-launcher/releases/download/v9.9.9/marah-launcher-linux-x86_64.tar.gz";
    host.http().serve(url, package("9.9.9"));
    host.http().serve(LATEST, manifest("9.9.9", url, &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!(update.state, "failed");
    assert!(update.text.contains("SHA-256"), "{}", update.text);
    assert!(staged(t.data_dir()).is_none(), "nothing kept");
}

#[tokio::test]
async fn an_empty_release_repo_turns_checks_off_and_an_older_release_is_no_update() {
    let host = FakeHost::new();
    host.http().serve(LATEST, manifest("0.0.1", "https://github.com/owner/marah-launcher/x.tar.gz", &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "".into()).unwrap();
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "off");

    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "current");
}

#[tokio::test]
async fn without_a_setting_the_launcher_reads_marah_s_own_releases() {
    let host = FakeHost::new();
    let own = format!(
        "https://github.com/{}/releases/latest/download/latest.json",
        launcher_core::launcher_update::DEFAULT_REPO
    );
    host.http().serve(&own, manifest("0.0.1", "https://github.com/MarahHQ/marah-launcher/x.tar.gz", &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "current", "read from {own}");
}

#[test]
fn each_kind_of_install_reads_its_own_entry_in_latest_json() {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    let arch = std::env::consts::ARCH;
    assert_eq!(package_key(Install::Folder), Some(format!("{os}-{arch}")));
    assert_eq!(package_key(Install::MacApp), Some(format!("{os}-{arch}")));
    assert_eq!(package_key(Install::AppImage), Some(format!("linux-{arch}-appimage")));
    assert_eq!(package_key(Install::Rpm), Some(format!("linux-{arch}-rpm")));
    assert_eq!(package_key(Install::Deb), Some(format!("linux-{arch}-deb")));
    assert_eq!(package_key(Install::Arch), Some(format!("linux-{arch}-arch")));
    assert_eq!(package_key(Install::WindowsSetup), Some(format!("windows-{arch}-setup")));
    assert_eq!(package_key(Install::MarahOs), None, "Marah OS updates the launcher itself");
}

/// A packaged install's release: the rpm, served at its release address.
async fn packaged(install: Install, file: &str) -> (FakeHost, String) {
    let host = FakeHost::new().install(install);
    let url = format!("https://github.com/owner/marah-launcher/releases/download/v9.9.9/{file}");
    let sha = host.http().serve(&url, b"a package".to_vec());
    host.http().serve(LATEST, manifest_for(install, "9.9.9", &url, &sha));
    (host, url)
}

#[tokio::test]
async fn a_packaged_install_hands_the_checked_package_to_its_package_manager_then_offers_a_restart() {
    for (install, file, manager) in [
        (Install::Rpm, "marah-launcher-9.9.9.x86_64.rpm", PackageManager::Dnf),
        (Install::Deb, "marah-launcher_9.9.9_amd64.deb", PackageManager::Apt),
        (Install::Arch, "marah-launcher-9.9.9-1-x86_64.pkg.tar.zst", PackageManager::Pacman),
    ] {
        let (host, _) = packaged(install, file).await;
        let login_session = host.login_session().clone();
        let t = host.boot_core().await;
        t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

        t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
        let update = t.core.view().settings.launcher_update;
        assert_eq!((update.state.as_str(), update.method.as_str()), ("available", "package"));

        t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
        let update = t.core.view().settings.launcher_update;
        assert_eq!(update.state, "installed", "{}", update.text);
        assert!(update.text.contains("restart"), "{}", update.text);
        let handed = login_session.packages();
        assert_eq!(handed.len(), 1, "{install:?}");
        assert_eq!(handed[0].1, manager);
        assert_eq!(handed[0].0.file_name().unwrap(), file);
        assert!(staged(t.data_dir()).is_none(), "the package manager put it in: nothing for the next start");
    }
}

#[tokio::test]
async fn a_closed_password_prompt_or_a_package_manager_error_leaves_this_version_and_says_why() {
    for (refusal, says) in [
        (PackageError::Cancelled, "password"),
        (PackageError::Failed("nothing provides libmpv".into()), "nothing provides libmpv"),
    ] {
        let (host, _) = packaged(Install::Rpm, "marah-launcher-9.9.9.x86_64.rpm").await;
        host.login_session().refuse_packages(refusal);
        let t = host.boot_core().await;
        t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

        t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
        t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
        let update = t.core.view().settings.launcher_update;
        assert_eq!(update.state, "failed");
        assert!(update.text.contains(says), "{}", update.text);
        assert!(staged(t.data_dir()).is_none());
    }
}

#[tokio::test]
async fn a_package_that_doesn_t_match_its_sha256_never_reaches_the_package_manager() {
    let host = FakeHost::new().install(Install::Deb);
    let url = "https://github.com/owner/marah-launcher/releases/download/v9.9.9/marah-launcher_9.9.9_amd64.deb";
    host.http().serve(url, b"altered".to_vec());
    host.http().serve(LATEST, manifest_for(Install::Deb, "9.9.9", url, &"0".repeat(64)));
    let login_session = host.login_session().clone();
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "failed");
    assert!(login_session.packages().is_empty());
}

#[tokio::test]
async fn an_installer_install_keeps_the_new_installer_for_the_next_start() {
    let host = FakeHost::new().install(Install::WindowsSetup);
    let url =
        "https://github.com/owner/marah-launcher/releases/download/v9.9.9/marah-launcher-windows-x86_64-setup.exe";
    let sha = host.http().serve(url, b"an installer".to_vec());
    host.http().serve(LATEST, manifest_for(Install::WindowsSetup, "9.9.9", url, &sha));
    let login_session = host.login_session().clone();
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.method, "installer");
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "ready");
    let kept = staged(t.data_dir()).expect("kept for the next start");
    assert_eq!(kept.file.file_name().unwrap(), "marah-launcher-windows-x86_64-setup.exe");
    assert!(login_session.packages().is_empty());
}

#[tokio::test]
async fn on_marah_os_a_newer_release_comes_with_the_next_system_update() {
    let host = FakeHost::new().install(Install::MarahOs);
    host.http().serve(LATEST, manifest("9.9.9", "https://github.com/owner/marah-launcher/x.tar.gz", &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();

    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!((update.state.as_str(), update.method.as_str(), update.version.as_str()), ("system", "system", "9.9.9"));
    assert!(update.text.contains("system update"), "{}", update.text);
    assert!(
        t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.is_err(),
        "nothing for the launcher to download"
    );
}

#[tokio::test]
async fn a_folder_or_app_install_swaps_its_update_in_at_the_next_start() {
    for install in [Install::Folder, Install::AppImage, Install::MacApp, Install::WindowsZip] {
        let t = FakeHost::new().install(install).boot_core().await;
        assert_eq!(t.core.view().settings.launcher_update.method, "swap", "{install:?}");
    }
}

#[test]
fn the_install_is_told_by_the_files_beside_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path();
    assert_eq!(Install::of(bin, false), Install::Unknown, "a build that isn't installed");

    std::fs::write(bin.join("LICENSE"), "").unwrap();
    std::fs::create_dir(bin.join("extensions")).unwrap();
    let folder = if cfg!(windows) { Install::WindowsZip } else { Install::Folder };
    assert_eq!(Install::of(bin, false), folder);

    std::fs::write(bin.join("installed-by-setup"), "").unwrap();
    assert_eq!(Install::of(bin, false), Install::WindowsSetup);

    for (format, install) in [("rpm\n", Install::Rpm), ("deb", Install::Deb), ("arch", Install::Arch)] {
        std::fs::write(bin.join("packaged"), format).unwrap();
        assert_eq!(Install::of(bin, false), install);
    }
    assert_eq!(Install::of(bin, true), Install::MarahOs, "Marah OS's packaged launcher");

    let app = dir.path().join("MarahLauncher.app/Contents/MacOS");
    std::fs::create_dir_all(&app).unwrap();
    assert_eq!(Install::of(&app, false), Install::MacApp);
}

/// The AppImage's release address, its zsync control file beside it.
const APPIMAGE: &str =
    "https://github.com/owner/marah-launcher/releases/download/v9.9.9/Marah-Launcher-x86_64.AppImage";
/// What `zsyncmake -b 1024` (0.6.2, Ubuntu 22.04's, as the release build has) made from
/// [`xorshift`]`(100_000)`: 98 blocks, the last one short.
const CONTROL: &[u8] = include_bytes!("../../fixtures/zsync/xorshift-100000.zsync");

/// Bytes from a xorshift32 generator: the new AppImage the control file describes.
fn xorshift(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x2545_f491;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// The running AppImage: the new one with a byte changed in blocks 5, 40 and 80.
fn old_appimage(new: &[u8]) -> Vec<u8> {
    let mut old = new.to_vec();
    for block in [5, 40, 80] {
        old[block * 1024 + 100] ^= 0xff;
    }
    old
}

/// An AppImage install running `old`, and a release of the new AppImage with `control` beside
/// it (none: no control file) and `sha256` in latest.json (none: the new AppImage's own).
async fn appimage_release(
    old: &[u8],
    control: Option<&[u8]>,
    sha256: Option<&str>,
) -> (launcher_testkit::TestCore, tempfile::TempDir) {
    let install = tempfile::tempdir().unwrap();
    let running = install.path().join("Marah-Launcher-x86_64.AppImage");
    std::fs::write(&running, old).unwrap();
    let host = FakeHost::new().install(Install::AppImage).appimage(&running);
    let sha = host.http().serve(APPIMAGE, xorshift(100_000));
    if let Some(control) = control {
        host.http().serve(&format!("{APPIMAGE}.zsync"), control.to_vec());
    }
    host.http().serve(LATEST, manifest_for(Install::AppImage, "9.9.9", APPIMAGE, sha256.unwrap_or(&sha)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/marah-launcher".into()).unwrap();
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Check)).await.unwrap();
    (t, install)
}

/// The AppImage requests so far: each one's Range header, or `None` for the whole file.
fn appimage_requests(t: &launcher_testkit::TestCore) -> Vec<Option<String>> {
    t.http.sent_ranges().into_iter().filter(|(url, _)| url == APPIMAGE).map(|(_, range)| range).collect()
}

async fn download(t: &launcher_testkit::TestCore) {
    t.core.dispatch(Command::LauncherUpdate(launcher_core::LauncherUpdateCmd::Download)).await.unwrap();
}

/// The update kept for the next start is the new AppImage, byte for byte.
fn kept_is_the_new_appimage(t: &launcher_testkit::TestCore) {
    let update = t.core.view().settings.launcher_update;
    assert_eq!(update.state, "ready", "{}", update.text);
    let kept = staged(t.data_dir()).expect("kept for the next start");
    assert_eq!(kept.file.file_name().unwrap(), "Marah-Launcher-x86_64.AppImage");
    assert!(std::fs::read(&kept.file).unwrap() == xorshift(100_000), "byte for byte the new AppImage");
}

#[tokio::test]
async fn an_appimage_update_downloads_only_the_blocks_that_changed() {
    let new = xorshift(100_000);
    let (t, _install) = appimage_release(&old_appimage(&new), Some(CONTROL), None).await;
    download(&t).await;
    kept_is_the_new_appimage(&t);
    let ranges = [5, 40, 80].map(|b| Some(format!("bytes={}-{}", b * 1024, b * 1024 + 1023)));
    assert_eq!(appimage_requests(&t), ranges, "only the changed blocks, and never the whole file");
}

#[tokio::test]
async fn an_appimage_update_shows_how_much_it_downloads_of_the_whole() {
    let new = xorshift(100_000);
    let (t, _install) = appimage_release(&old_appimage(&new), Some(CONTROL), None).await;
    let gate = t.http.hold_next(APPIMAGE);
    let watch = async {
        for _ in 0..500 {
            let update = t.core.view().settings.launcher_update;
            if update.state == "downloading" && update.text.contains(" of ") {
                gate.release();
                return update.text;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        gate.release();
        t.core.view().settings.launcher_update.text
    };
    let ((), text) = tokio::join!(download(&t), watch);
    // 3 blocks of 1024 bytes, of 100,000.
    assert!(text.contains("0.0 MB of 0.1 MB"), "{text}");
    kept_is_the_new_appimage(&t);
}

#[tokio::test]
async fn without_a_control_file_the_appimage_is_downloaded_whole() {
    let new = xorshift(100_000);
    let (t, _install) = appimage_release(&old_appimage(&new), None, None).await;
    download(&t).await;
    kept_is_the_new_appimage(&t);
    assert_eq!(appimage_requests(&t), [None]);
}

#[tokio::test]
async fn a_broken_control_file_or_a_server_without_ranges_means_the_whole_appimage() {
    let new = xorshift(100_000);
    let mut cut = CONTROL.to_vec();
    cut.truncate(CONTROL.len() - 3);
    let (t, _install) = appimage_release(&old_appimage(&new), Some(&cut), None).await;
    download(&t).await;
    kept_is_the_new_appimage(&t);
    assert_eq!(appimage_requests(&t), [None], "a cut control file");

    let (t, _install) = appimage_release(&old_appimage(&new), Some(CONTROL), None).await;
    t.http.ignore_ranges();
    download(&t).await;
    kept_is_the_new_appimage(&t);
    assert_eq!(appimage_requests(&t).last(), Some(&None), "the server sent the whole file for a range");
}

#[tokio::test]
async fn an_appimage_that_doesn_t_match_its_sha256_is_refused_after_a_whole_download() {
    let new = xorshift(100_000);
    let (t, _install) = appimage_release(&old_appimage(&new), Some(CONTROL), Some(&"0".repeat(64))).await;
    download(&t).await;
    let update = t.core.view().settings.launcher_update;
    assert_eq!(update.state, "failed");
    assert!(update.text.contains("SHA-256"), "{}", update.text);
    let asked = appimage_requests(&t);
    assert_eq!(asked.len(), 4, "three ranges, then the whole file: {asked:?}");
    assert_eq!(asked.last(), Some(&None));
    assert!(staged(t.data_dir()).is_none(), "nothing kept");
}

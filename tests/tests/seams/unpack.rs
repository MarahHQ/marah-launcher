//! Seam 1, core app API: archives a download plan asks to unpack. The host unpacks after the
//! checks, refuses paths that leave the folder, and runs an AppImage's extract only on Linux.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use launcher_api::*;
use launcher_core::{Command, CoreError};
use launcher_testkit::{FakeHost, StubPlugin, TestCore};

const URL: &str = "https://builds.example/emu.archive";
const DEST: &str = "downloads/emu.archive";

/// Plans one archive, and records what the host hands the installer.
#[derive(Clone)]
struct ArchiveEmu {
    sha256: String,
    format: ArchiveFormat,
    /// Where the download goes; a bare AppImage keeps this file's name.
    dest: &'static str,
    /// The archive's paths to unpack; empty for all.
    only: Vec<String>,
    got: Arc<Mutex<Vec<DownloadedArtifact>>>,
}

#[async_trait]
impl Downloader for ArchiveEmu {
    async fn plan(&self, _: &DownloadRequest) -> PluginResult<DownloadPlan> {
        Ok(DownloadPlan {
            label: "Emu 1.0".into(),
            artifacts: vec![Artifact {
                url: URL.into(),
                size: None,
                sha256: self.sha256.clone(),
                signature: None,
                dest: RelPath::new(self.dest)?,
                unpack: Some(Unpack {
                    format: self.format,
                    into: RelPath::new("builds/1.0")?,
                    only: self.only.clone(),
                }),
            }],
        })
    }
}

#[async_trait]
impl Installer for ArchiveEmu {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(vec![VersionInfo { version: "1.0".into(), notes: None }])
    }
    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        Ok(vec![])
    }
    async fn install(&self, version: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        self.got.lock().unwrap().extend_from_slice(files);
        Ok(InstalledVersion { version: version.into(), path: RelPath::new("builds/1.0")?, own: false })
    }
    async fn uninstall(&self, _: &str) -> PluginResult<()> {
        Ok(())
    }
}

async fn boot(host: FakeHost, archive: Vec<u8>, format: ArchiveFormat) -> (TestCore, ArchiveEmu) {
    boot_with(host, archive, format, "[\"builds/*/*.AppImage\"]").await
}

async fn boot_with(host: FakeHost, archive: Vec<u8>, format: ArchiveFormat, process: &str) -> (TestCore, ArchiveEmu) {
    boot_full(host, archive, format, process, Vec::new(), "").await
}

/// [`boot_with`], with only the paths `only` of the archive, and `top` at the top of the
/// manifest.
async fn boot_full(
    host: FakeHost,
    archive: Vec<u8>,
    format: ArchiveFormat,
    process: &str,
    only: Vec<String>,
    top: &str,
) -> (TestCore, ArchiveEmu) {
    let emu = ArchiveEmu { sha256: host.http().serve(URL, archive), format, dest: DEST, only, got: Arc::default() };
    let e = emu.clone();
    let plugin = StubPlugin::from_toml(&format!(
        "{top}id = \"emu\"\nkind = \"emulator\"\nname = \"Emu\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         capabilities = [\"downloader\", \"installer\"]\n[permissions]\nnetwork = [\"builds.example\"]\nprocess = {process}"
    ))
    .provides(move |_| {
        let e = Arc::new(e.clone());
        Ok(Provided { downloader: Some(e.clone()), installer: Some(e), ..Provided::default() })
    });
    (host.plugin(plugin).boot_core().await, emu)
}

async fn install(t: &TestCore) -> CoreResult {
    t.core.dispatch(Command::InstallEmulator { plugin: PluginId::new("emu").unwrap(), version: None }).await
}

type CoreResult = Result<(), CoreError>;

fn tar_gz(build: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>)) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    build(&mut tar);
    tar.into_inner().unwrap().finish().unwrap()
}

fn file(tar: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>, path: &str, mode: u32, data: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_entry_type(tar::EntryType::Regular);
    // Written raw, so a test can put a path the builder would refuse.
    header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
    header.set_cksum();
    tar.append(&header, data).unwrap();
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn plugin_dir(t: &TestCore) -> std::path::PathBuf {
    t.data_dir().join("plugins/emu")
}

#[tokio::test]
async fn a_tar_gz_is_unpacked_with_its_modes_and_links_and_the_archive_is_deleted() {
    let archive = tar_gz(|tar| {
        file(tar, "bin/emu", 0o755, b"#!/bin/sh\n");
        file(tar, "lib/libx.so.1", 0o644, b"lib");
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        tar.append_link(&mut link, "lib/libx.so", "libx.so.1").unwrap();
    });
    let (t, emu) = boot(FakeHost::new(), archive, ArchiveFormat::TarGz).await;

    install(&t).await.unwrap();

    let build = plugin_dir(&t).join("builds/1.0");
    assert_eq!(std::fs::read(build.join("bin/emu")).unwrap(), b"#!/bin/sh\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(build.join("bin/emu")).unwrap().permissions().mode() & 0o111, 0o111);
        assert_eq!(std::fs::read(build.join("lib/libx.so")).unwrap(), b"lib", "the link works");
    }
    assert!(!plugin_dir(&t).join("downloads/emu.archive").exists(), "the archive is gone");
    let got = emu.got.lock().unwrap().clone();
    assert_eq!(got[0].unpacked, Some(RelPath::new("builds/1.0").unwrap()));
}

#[tokio::test]
async fn an_archive_path_that_leaves_the_folder_fails_the_install_and_writes_nothing_outside() {
    let archive = tar_gz(|tar| file(tar, "../../escaped", 0o644, b"x"));
    let (t, emu) = boot(FakeHost::new(), archive, ArchiveFormat::TarGz).await;

    let err = install(&t).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("leaves the folder")), "{err}");
    assert!(!plugin_dir(&t).join("escaped").exists() && !t.data_dir().join("plugins/escaped").exists());
    assert!(!plugin_dir(&t).join("builds/1.0").exists());
    assert!(emu.got.lock().unwrap().is_empty());

    let archive = zip(&[("../outside.txt", b"x")]);
    let (t, _) = boot(FakeHost::new(), archive, ArchiveFormat::Zip).await;
    let err = install(&t).await.unwrap_err();
    assert!(matches!(&err, CoreError::Download(m) if m.contains("leaves the folder")), "{err}");
    assert!(!plugin_dir(&t).join("outside.txt").exists());
}

#[tokio::test]
async fn a_tar_link_out_of_the_folder_is_refused() {
    let archive = tar_gz(|tar| {
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        tar.append_link(&mut link, "etc", "../../../../etc").unwrap();
    });
    let (t, _) = boot(FakeHost::new(), archive, ArchiveFormat::TarGz).await;

    let err = install(&t).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("links out")), "{err}");
}

#[tokio::test]
async fn an_appimage_in_a_zip_is_extracted_by_running_it_on_linux() {
    let archive = zip(&[("Shadps4-sdl.AppImage", b"\x7fELF appimage"), ("README.txt", b"skip me")]);
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let (t, _) = boot(host, archive, ArchiveFormat::AppImageInZip).await;
    let core = t.core.clone();
    let install = tokio::spawn(async move {
        core.dispatch(Command::InstallEmulator { plugin: PluginId::new("emu").unwrap(), version: None }).await
    });

    // What the AppImage does when it runs: unpack itself next to it.
    let process = tokio::time::timeout(Duration::from_secs(5), t.spawner.wait_for(1)).await.unwrap();
    assert_eq!(process.command.args, ["--appimage-extract"]);
    assert_eq!(process.command.program, process.command.cwd.join("Shadps4-sdl.AppImage"));
    std::fs::create_dir_all(process.command.cwd.join("squashfs-root")).unwrap();
    std::fs::write(process.command.cwd.join("squashfs-root/AppRun"), b"#!/bin/sh\n").unwrap();
    process.exit(0);
    install.await.unwrap().unwrap();

    let build = plugin_dir(&t).join("builds/1.0");
    assert!(build.join("squashfs-root/AppRun").is_file());
    assert!(!build.join("Shadps4-sdl.AppImage").exists(), "the AppImage is gone once extracted");
    assert!(!build.join("README.txt").exists(), "only the AppImage comes out of the zip");
}

#[tokio::test]
async fn a_bare_appimage_is_extracted_by_running_it_on_linux_under_its_own_name() {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let emu = ArchiveEmu {
        sha256: host.http().serve(URL, b"\x7fELF appimage".to_vec()),
        format: ArchiveFormat::AppImage,
        dest: "downloads/lichii-linux-x86_64.AppImage",
        only: Vec::new(),
        got: Arc::default(),
    };
    let e = emu.clone();
    let plugin = StubPlugin::from_toml(
        "id = \"emu\"\nkind = \"emulator\"\nname = \"Emu\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         capabilities = [\"downloader\", \"installer\"]\n[permissions]\nnetwork = [\"builds.example\"]\n\
         process = [\"builds/*/*.AppImage\"]",
    )
    .provides(move |_| {
        let e = Arc::new(e.clone());
        Ok(Provided { downloader: Some(e.clone()), installer: Some(e), ..Provided::default() })
    });
    let t = host.plugin(plugin).boot_core().await;
    let core = t.core.clone();
    let install = tokio::spawn(async move {
        core.dispatch(Command::InstallEmulator { plugin: PluginId::new("emu").unwrap(), version: None }).await
    });

    let process = tokio::time::timeout(Duration::from_secs(5), t.spawner.wait_for(1)).await.unwrap();
    assert_eq!(process.command.args, ["--appimage-extract"]);
    assert_eq!(process.command.program, process.command.cwd.join("lichii-linux-x86_64.AppImage"));
    std::fs::create_dir_all(process.command.cwd.join("squashfs-root")).unwrap();
    std::fs::write(process.command.cwd.join("squashfs-root/AppRun"), b"#!/bin/sh\n").unwrap();
    process.exit(0);
    install.await.unwrap().unwrap();

    let build = plugin_dir(&t).join("builds/1.0");
    assert!(build.join("squashfs-root/AppRun").is_file());
    assert!(!build.join("lichii-linux-x86_64.AppImage").exists(), "the AppImage is gone once extracted");
}

#[tokio::test]
async fn an_appimage_whose_extract_never_ends_is_killed_after_ten_minutes() {
    let archive = zip(&[("Emu.AppImage", b"\x7fELF appimage")]);
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let (t, _) = boot(host, archive, ArchiveFormat::AppImageInZip).await;
    let core = t.core.clone();
    let install = tokio::spawn(async move {
        core.dispatch(Command::InstallEmulator { plugin: PluginId::new("emu").unwrap(), version: None }).await
    });
    let process = t.spawner.wait_for(1).await;

    // The core's limit for an extract is ten minutes.
    t.clock.advance(Duration::from_secs(599)).await;
    assert!(!install.is_finished(), "a slow extract has ten minutes");
    assert!(process.signals().is_empty());

    t.clock.advance(Duration::from_secs(1)).await;
    let err = install.await.unwrap().unwrap_err();
    assert!(matches!(&err, CoreError::Download(m) if m.contains("took too long")), "{err}");
    assert_eq!(process.signals(), [launcher_testkit::Signal::Kill]);
    assert!(!plugin_dir(&t).join("builds/1.0").exists(), "nothing half unpacked stays");
}

#[tokio::test]
async fn a_tampered_appimage_is_deleted_before_anything_runs_and_other_systems_refuse_it() {
    let archive = zip(&[("Emu.AppImage", b"\x7fELF")]);
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let tampered = ArchiveEmu {
        sha256: "00".repeat(32),
        format: ArchiveFormat::AppImageInZip,
        dest: DEST,
        only: Vec::new(),
        got: Arc::default(),
    };
    host.http().serve(URL, archive.clone());
    let e = tampered.clone();
    let plugin = StubPlugin::from_toml(
        "id = \"emu\"\nkind = \"emulator\"\nname = \"Emu\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         capabilities = [\"downloader\", \"installer\"]\n[permissions]\nnetwork = [\"builds.example\"]",
    )
    .provides(move |_| {
        let e = Arc::new(e.clone());
        Ok(Provided { downloader: Some(e.clone()), installer: Some(e), ..Provided::default() })
    });
    let t = host.plugin(plugin).boot_core().await;

    let err = install(&t).await.unwrap_err();
    assert!(matches!(&err, CoreError::Download(m) if m.contains("checksum")), "{err}");
    assert!(t.spawner.spawned().is_empty(), "nothing ran");
    assert!(!plugin_dir(&t).join("downloads/emu.archive").exists());

    let (t, _) = boot(FakeHost::new().platform(Some(Platform::Macos)), archive, ArchiveFormat::AppImageInZip).await;
    let err = install(&t).await.unwrap_err();
    assert!(matches!(&err, CoreError::Download(m) if m.contains("Linux only")), "{err}");
    assert!(t.spawner.spawned().is_empty());
}

#[tokio::test]
async fn a_finished_unpack_or_a_checked_file_is_not_downloaded_again() {
    let archive = tar_gz(|tar| file(tar, "emu", 0o755, b"x"));
    let (t, _) = boot(FakeHost::new(), archive.clone(), ArchiveFormat::TarGz).await;
    install(&t).await.unwrap();
    assert_eq!(t.http.requests().len(), 1);

    install(&t).await.unwrap();
    assert_eq!(t.http.requests().len(), 1, "the unpacked folder came from this very archive");

    // The app stopped after the download, before the unpack: the file is checked and used.
    std::fs::remove_dir_all(plugin_dir(&t).join("builds/1.0")).unwrap();
    std::fs::write(plugin_dir(&t).join("downloads/emu.archive"), &archive).unwrap();
    install(&t).await.unwrap();
    assert_eq!(t.http.requests().len(), 1, "no request for a file that passes the checks");
    assert!(Path::new(&plugin_dir(&t).join("builds/1.0/emu")).is_file());
}

#[tokio::test]
async fn an_appimage_runs_to_unpack_only_with_the_plugin_s_process_permission() {
    let archive = zip(&[("Emu.AppImage", b"\x7fELF")]);
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let (t, _) = boot_with(host, archive, ArchiveFormat::AppImageInZip, "[]").await;

    let err = install(&t).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("permissions.process")), "{err}");
    assert!(t.spawner.spawned().is_empty(), "nothing ran");
}

#[cfg(unix)]
#[tokio::test]
async fn a_zip_can_t_write_out_of_the_folder_through_its_own_links() {
    // `x/y/l` points two folders up, which is still inside; `a` then goes up two more from
    // there, and `a/evil` would land outside.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.add_symlink("x/y/l", "../..", options).unwrap();
    zip.add_symlink("a", "x/y/l/../..", options).unwrap();
    zip.start_file("a/evil", options).unwrap();
    zip.write_all(b"x").unwrap();
    let archive = zip.finish().unwrap().into_inner();
    let (t, _) = boot(FakeHost::new(), archive, ArchiveFormat::Zip).await;

    let err = install(&t).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("link")), "{err}");
    assert!(!plugin_dir(&t).join("builds/evil").exists() && !plugin_dir(&t).join("evil").exists());
}

/// A 7z that holds many programs: only the paths the plan names come out, through the host's
/// archives.
#[tokio::test]
async fn a_7z_unpacks_only_the_paths_the_plan_names() {
    let host = FakeHost::new();
    host.archives().holds(
        "emu.archive",
        &[
            ("pack/cores/a.so", b"a" as &[u8]),
            ("pack/cores/b.so", b"b"),
            ("pack/info/b.info", b"i"),
            ("pack/big.bin", b"big"),
        ],
    );
    let only = vec!["pack/cores/b.so".to_owned(), "pack/info".to_owned()];
    let (t, _) = boot_full(host, b"7z bytes".to_vec(), ArchiveFormat::SevenZip, "[]", only, "").await;

    install(&t).await.unwrap();

    let build = plugin_dir(&t).join("builds/1.0/pack");
    assert_eq!(std::fs::read(build.join("cores/b.so")).unwrap(), b"b");
    assert!(build.join("info/b.info").is_file(), "a folder, with what is in it");
    assert!(!build.join("cores/a.so").exists() && !build.join("big.bin").exists());
    assert_eq!(t.archives.unpacked()[0].0, launcher_core::ArchiveKind::SevenZ);
}

/// A 7z that holds an AppImage with a folder beside it: the AppImage is extracted where it is,
/// on Linux, and the folder stays.
#[tokio::test]
async fn an_appimage_in_a_7z_is_extracted_where_it_is_on_linux() {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    host.archives()
        .holds("emu.archive", &[("Emu/Emu.AppImage", b"\x7fELF" as &[u8]), ("Emu/Emu.AppImage.home/cfg", b"c")]);
    let (t, _) =
        boot_full(host, b"7z".to_vec(), ArchiveFormat::AppImageIn7z, "[\"builds/*/Emu/*.AppImage\"]", vec![], "").await;
    let core = t.core.clone();
    let install = tokio::spawn(async move {
        core.dispatch(Command::InstallEmulator { plugin: PluginId::new("emu").unwrap(), version: None }).await
    });

    let process = tokio::time::timeout(Duration::from_secs(5), t.spawner.wait_for(1)).await.unwrap();
    assert_eq!(process.command.args, ["--appimage-extract"]);
    assert_eq!(process.command.program, process.command.cwd.join("Emu.AppImage"));
    std::fs::create_dir_all(process.command.cwd.join("squashfs-root")).unwrap();
    std::fs::write(process.command.cwd.join("squashfs-root/AppRun"), b"#!/bin/sh\n").unwrap();
    process.exit(0);
    install.await.unwrap().unwrap();

    let build = plugin_dir(&t).join("builds/1.0/Emu");
    assert!(build.join("squashfs-root/AppRun").is_file());
    assert!(!build.join("Emu.AppImage").exists(), "the AppImage is gone once extracted");
    assert_eq!(std::fs::read(build.join("Emu.AppImage.home/cfg")).unwrap(), b"c", "what is beside it stays");
}

/// A plan's `only` is for a 7z: another archive with it is refused.
#[tokio::test]
async fn only_part_of_an_archive_that_is_not_a_7z_is_refused() {
    let archive = zip(&[("a.txt", b"a")]);
    let (t, _) = boot_full(FakeHost::new(), archive, ArchiveFormat::Zip, "[]", vec!["a.txt".into()], "").await;
    let refused = install(&t).await.unwrap_err();
    assert!(refused.to_string().contains("only a 7z unpacks part of itself"), "{refused}");
}

/// A plugin that shares a data folder downloads into it, not into a folder of its own.
#[tokio::test]
async fn a_plugin_that_shares_a_data_folder_downloads_into_it() {
    let host = FakeHost::new();
    host.archives().holds("emu.archive", &[("prog", b"p" as &[u8])]);
    let (t, _) =
        boot_full(host, b"7z".to_vec(), ArchiveFormat::SevenZip, "[]", vec![], "shared_data = \"retro\"\n").await;

    install(&t).await.unwrap();

    assert_eq!(std::fs::read(t.data_dir().join("shared/retro/builds/1.0/prog")).unwrap(), b"p");
    assert!(!plugin_dir(&t).exists(), "no folder of its own");
}

/// A shared data folder holds another plugin's programs: a plugin from disk can't name one.
#[tokio::test]
async fn a_plugin_from_disk_that_names_a_shared_data_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sneaky");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("plugin.toml"),
        "id = \"sneaky\"\nkind = \"theme\"\nname = \"S\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
         shared_data = \"retroarch\"\n[runtime]\ntype = \"data\"\n",
    )
    .unwrap();
    let t = FakeHost::new().discover(dir.path()).boot_core().await;
    let refused = t.report().rejection(&PluginId::new("sneaky").unwrap()).expect("refused").to_string();
    assert!(refused.contains("only a built-in plugin shares a data folder"), "{refused}");
}

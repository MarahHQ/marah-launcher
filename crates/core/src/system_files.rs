//! An emulator's system files (`[system_files]` in its manifest): firmware, keys, BIOS files or
//! system modules, which the player imports from one archive on the emulator's page.
//!
//! - The page shows a "System files" block: whether they are in place, and an "Import…" row
//!   ([`launcher_api::HOST_IMPORT_SYSTEM_FILES`]) that opens the folder browser to pick the
//!   archive.
//! - The import unpacks the archive into a work folder of the data folder, through the
//!   [`crate::Archives`] port, which refuses paths that leave the folder. Then [`install`]:
//!   - finds each part's folder in it, at its top or in the one folder the archive holds;
//!   - checks the files `SHA256SUMS` lists, when the archive has one;
//!   - copies the files each part takes into a folder beside its target (`<target>.part`), so
//!     the swap is a rename on one disk;
//!   - swaps them in: a `replace` part's whole folder, another part's files one by one. When a
//!     part can't be swapped in, every part goes back as it was.
//!
//! The import writes nothing until every part is checked: a wrong archive changes nothing.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use launcher_api::{PluginId, SystemFiles, SystemFilesPart};
use sha2::Digest;

use crate::{ArchiveKind, Core, CoreError, CoreResult};

/// The work folder of an import, in the data folder: `tmp/system-files/<plugin>`.
const WORK: &str = "tmp/system-files";

/// What the page says of a plugin's system files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Status {
    /// Every part's files are in place.
    pub installed: bool,
    /// An import runs.
    pub importing: bool,
    /// What the last import did, or why it failed.
    pub last: String,
    pub failed: bool,
}

/// The folder each part of `plugin`'s system files goes into, in the parts' order.
pub(crate) fn targets(core: &Core, plugin: &PluginId) -> CoreResult<(SystemFiles, Vec<PathBuf>)> {
    let kernel = core.kernel();
    let manifest = kernel.manifest(plugin)?;
    let files = manifest
        .system_files
        .clone()
        .ok_or_else(|| CoreError::Invalid(format!("{} has no system files to import", manifest.name)))?;
    let data = kernel.host().plugin_dir(plugin);
    let folders = kernel.host_folders(plugin);
    let mut dirs = Vec::new();
    for part in &files.parts {
        let base = match &part.folder {
            None => data.clone(),
            Some(folder) => {
                let folders = folders.as_ref().ok_or_else(|| CoreError::UnknownPlugin(plugin.clone()))?;
                let (roots, _) = folders.roots_of(folder).map_err(|e| CoreError::Invalid(e.to_string()))?;
                roots.into_iter().next().ok_or_else(|| CoreError::Invalid(format!("the folder {folder} isn't set")))?
            }
        };
        dirs.push(part.into.as_str().split('/').filter(|p| !p.is_empty()).fold(base, |p, s| p.join(s)));
    }
    Ok((files, dirs))
}

/// Every part's files are in place: its required files, or one file it takes.
pub(crate) fn installed(files: &SystemFiles, dirs: &[PathBuf]) -> bool {
    files.parts.iter().zip(dirs).all(|(part, dir)| {
        let found = taken(part, dir).unwrap_or_default();
        let names: Vec<String> =
            found.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into()).collect();
        !found.is_empty() && part.required.iter().all(|r| names.iter().any(|n| n.eq_ignore_ascii_case(r)))
    })
}

/// The files of `dir` and its folders that `part` takes, as paths relative to `dir`. Links are
/// left out. `None` when `dir` isn't a folder.
fn taken(part: &SystemFilesPart, dir: &Path) -> Option<Vec<PathBuf>> {
    if !dir.is_dir() {
        return None;
    }
    let mut out = Vec::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(rel) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(dir.join(&rel)) else { continue };
        for entry in entries.filter_map(Result::ok) {
            let Ok(kind) = entry.file_type() else { continue };
            let path = rel.join(entry.file_name());
            if kind.is_dir() {
                todo.push(path);
            } else if kind.is_file() && part.takes(&entry.file_name().to_string_lossy()) {
                out.push(path);
            }
        }
    }
    out.sort();
    Some(out)
}

/// Where the parts' folders are in the unpacked archive `unpacked`: its top, or the one folder
/// it holds when the parts aren't at its top (an archive of the folder itself).
fn archive_root(files: &SystemFiles, unpacked: &Path) -> PathBuf {
    let at_top = |root: &Path| files.parts.iter().any(|p| root.join(p.from.as_str()).is_dir());
    if at_top(unpacked) {
        return unpacked.to_path_buf();
    }
    let folders: Vec<PathBuf> = std::fs::read_dir(unpacked)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with(['.', '_']))
        .map(|e| e.path())
        .collect();
    match folders.as_slice() {
        [one] if at_top(one) => one.clone(),
        _ => unpacked.to_path_buf(),
    }
}

/// Checks the files `SHA256SUMS` lists at `root`, when it is there.
fn check_sums(root: &Path) -> Result<usize, String> {
    let list = root.join(launcher_api::SYSTEM_FILES_CHECKSUMS);
    let Ok(text) = std::fs::read_to_string(&list) else { return Ok(0) };
    let mut checked = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((sum, name)) = line.split_once(char::is_whitespace) else {
            return Err(format!("SHA256SUMS has a line that isn't a checksum and a file: {line}"));
        };
        let name = name.trim_start().trim_start_matches('*').trim_start_matches("./");
        let path =
            launcher_api::RelPath::new(name).map_err(|_| format!("SHA256SUMS lists {name}, outside the archive"))?;
        let file = path.as_str().split('/').fold(root.to_path_buf(), |p, s| p.join(s));
        let got = sha256(&file).map_err(|_| format!("SHA256SUMS lists {name}, which isn't in the archive"))?;
        if !got.eq_ignore_ascii_case(sum) {
            return Err(format!("{name} isn't the file SHA256SUMS lists: the archive is damaged or changed"));
        }
        checked += 1;
    }
    Ok(checked)
}

fn sha256(file: &Path) -> std::io::Result<String> {
    let mut reader = std::fs::File::open(file)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

/// `dir` with `suffix` added to its name: `registered.part`.
fn beside(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    dir.with_file_name(name)
}

/// A part swapped in, to put back if a later part fails.
enum Swapped {
    /// The whole folder: the old one, if there was one, is at `<dir>.old`.
    Folder { dir: PathBuf, had: bool },
    /// Files one by one: each new file, and whether an old one waits at `<dir>.old/<file>`.
    Files { dir: PathBuf, files: Vec<(PathBuf, bool)> },
}

/// Copies the system files of the archive unpacked at `unpacked` into the parts' folders
/// `dirs`, as the module says. The number of files it copied, or why it changed nothing.
pub(crate) fn install(unpacked: &Path, files: &SystemFiles, dirs: &[PathBuf]) -> Result<usize, String> {
    let root = archive_root(files, unpacked);
    check_sums(&root)?;
    let expected = files.parts.iter().map(|p| format!("{}/", p.from.as_str())).collect::<Vec<_>>().join(", ");
    let mut plans: Vec<Vec<PathBuf>> = Vec::new();
    for part in &files.parts {
        let from = root.join(part.from.as_str());
        let found = taken(part, &from).ok_or_else(|| {
            format!("This isn't {}: it has no {}/ folder. It should hold {expected}", files.archive, part.from.as_str())
        })?;
        if found.is_empty() {
            return Err(format!("{}/ in the archive has none of the files {} needs", part.from.as_str(), files.name));
        }
        let names: Vec<String> =
            found.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        if let Some(missing) = part.required.iter().find(|r| !names.iter().any(|n| n.eq_ignore_ascii_case(r))) {
            return Err(format!("{}/ in the archive has no {missing}", part.from.as_str()));
        }
        plans.push(found);
    }
    // Staged beside each target, so each swap is a rename on one disk.
    let staged: Vec<PathBuf> = dirs.iter().map(|d| beside(d, ".part")).collect();
    let cleanup = |also_old: bool| {
        for dir in dirs {
            let _ = std::fs::remove_dir_all(beside(dir, ".part"));
            if also_old {
                let _ = std::fs::remove_dir_all(beside(dir, ".old"));
            }
        }
    };
    cleanup(true);
    let copied = (|| -> std::io::Result<usize> {
        let mut count = 0;
        for ((part, found), stage) in files.parts.iter().zip(&plans).zip(&staged) {
            for rel in found {
                let to = stage.join(rel);
                std::fs::create_dir_all(to.parent().unwrap_or(stage))?;
                std::fs::copy(root.join(part.from.as_str()).join(rel), &to)?;
                count += 1;
            }
        }
        Ok(count)
    })();
    let count = match copied {
        Ok(count) => count,
        Err(e) => {
            cleanup(false);
            return Err(format!("The files couldn't be copied: {e}"));
        }
    };
    let mut done: Vec<Swapped> = Vec::new();
    for ((part, found), (dir, stage)) in files.parts.iter().zip(&plans).zip(dirs.iter().zip(&staged)) {
        match swap_in(part, found, dir, stage, &mut done) {
            Ok(()) => {}
            Err(e) => {
                for swapped in done.into_iter().rev() {
                    put_back(swapped);
                }
                cleanup(true);
                return Err(format!("{} couldn't be written: {e}. Nothing changed", dir.display()));
            }
        }
    }
    cleanup(true);
    Ok(count)
}

/// Swaps one part's staged files in, and records how to put it back.
fn swap_in(
    part: &SystemFilesPart,
    found: &[PathBuf],
    dir: &Path,
    stage: &Path,
    done: &mut Vec<Swapped>,
) -> std::io::Result<()> {
    let old = beside(dir, ".old");
    if part.replace {
        let had = dir.exists();
        if had {
            std::fs::rename(dir, &old)?;
        }
        if let Err(e) = std::fs::rename(stage, dir) {
            if had {
                let _ = std::fs::rename(&old, dir);
            }
            return Err(e);
        }
        done.push(Swapped::Folder { dir: dir.to_path_buf(), had });
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let mut files = Vec::new();
    let result = (|| -> std::io::Result<()> {
        for rel in found {
            let (to, kept) = (dir.join(rel), old.join(rel));
            std::fs::create_dir_all(to.parent().unwrap_or(dir))?;
            let had = to.exists();
            if had {
                std::fs::create_dir_all(kept.parent().unwrap_or(&old))?;
                std::fs::rename(&to, &kept)?;
            }
            files.push((rel.clone(), had));
            std::fs::rename(stage.join(rel), &to)?;
        }
        Ok(())
    })();
    done.push(Swapped::Files { dir: dir.to_path_buf(), files });
    result
}

/// Puts a part back as it was before the import.
fn put_back(swapped: Swapped) {
    match swapped {
        Swapped::Folder { dir, had } => {
            let _ = std::fs::remove_dir_all(&dir);
            if had {
                let _ = std::fs::rename(beside(&dir, ".old"), &dir);
            }
        }
        Swapped::Files { dir, files } => {
            let old = beside(&dir, ".old");
            for (rel, had) in files.into_iter().rev() {
                let _ = std::fs::remove_file(dir.join(&rel));
                if had {
                    let _ = std::fs::rename(old.join(&rel), dir.join(&rel));
                }
            }
        }
    }
}

/// What the archive at `file` is, by its name: a zip, a 7z, a rar or a tar.
fn archive_kind(file: &Path) -> Option<ArchiveKind> {
    let (kind, _, part) = crate::archives::part_of(&file.to_string_lossy())?;
    (part == 1 && kind != ArchiveKind::Iso).then_some(kind)
}

/// The browser lists this file for a system files import.
pub(crate) fn is_archive(file: &Path) -> bool {
    archive_kind(file).is_some()
}

/// Imports `plugin`'s system files from the archive `file`, as the module says. Its page says
/// what it did; a refusal is also the result.
pub(crate) async fn import(core: &Core, plugin: &PluginId, file: &Path) -> CoreResult<()> {
    let (files, dirs) = targets(core, plugin)?;
    let kind = archive_kind(file)
        .ok_or_else(|| CoreError::Invalid(format!("{} isn't an archive: pick {}", file.display(), files.archive)))?;
    let builds = core.builds();
    if builds.system_files(plugin).importing {
        return Err(CoreError::Invalid(format!("{} are importing: wait for it", files.name)));
    }
    builds.set_system_files(plugin, Status { importing: true, ..builds.system_files(plugin) });
    let work = core.kernel().host().data_dir.join(WORK).join(plugin.as_str());
    let _ = tokio::fs::remove_dir_all(&work).await;
    let result = async {
        tokio::fs::create_dir_all(&work).await.map_err(|e| e.to_string())?;
        let ports = core.ports();
        let free = ports.drives.free_bytes(&work);
        let progress: crate::UnpackProgress = Arc::new(|_, _| {});
        let parts = [file.to_path_buf()];
        let unpacked =
            ports.archives.unpack(kind, &parts, &work, free, progress, Arc::new(AtomicBool::new(false)), &[]);
        unpacked.await.map_err(|e| e.to_string())?;
        let (from, to, what) = (work.clone(), dirs.clone(), files.clone());
        tokio::task::spawn_blocking(move || install(&from, &what, &to)).await.map_err(|e| e.to_string())?
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    let status = match &result {
        Ok(count) => {
            Status { installed: installed(&files, &dirs), importing: false, last: imported_text(*count), failed: false }
        }
        Err(why) => Status { installed: installed(&files, &dirs), importing: false, last: why.clone(), failed: true },
    };
    builds.set_system_files(plugin, status);
    core.changed(crate::changes::Area::Plugins);
    match result {
        Ok(count) => {
            core.notices().info(format!("{}: {}", files.name, imported_text(count)));
            Ok(())
        }
        Err(why) => {
            core.notices().problem(format!("{}: {why}", files.name));
            Err(CoreError::Invalid(why))
        }
    }
}

fn imported_text(count: usize) -> String {
    if count == 1 { "1 file imported".into() } else { format!("{count} files imported") }
}

/// Reads again whether `plugin`'s system files are in place, for its page.
pub(crate) async fn refresh(core: &Core, plugin: &PluginId) {
    let Ok((files, dirs)) = targets(core, plugin) else { return };
    let here = tokio::task::spawn_blocking(move || installed(&files, &dirs)).await.unwrap_or(false);
    let builds = core.builds();
    builds.set_system_files(plugin, Status { installed: here, ..builds.system_files(plugin) });
}

/// The page's "System files" block of `plugin`, from its manifest and `status`.
pub(crate) fn block(files: &SystemFiles, status: &Status) -> launcher_api::SlotItem {
    use launcher_api::{BlockRow, SlotContent, SlotItem, Tone};
    let state = if status.importing {
        BlockRow { label: "Status".into(), value: "Importing…".into(), progress: Some(0.0), ..Default::default() }
    } else if status.installed {
        BlockRow { label: "Status".into(), value: "Installed".into(), tone: Tone::Success, ..Default::default() }
    } else if files.optional {
        BlockRow {
            label: "Status".into(),
            value: "Not installed".into(),
            hint: "Most games play without them; some need them".into(),
            ..Default::default()
        }
    } else {
        BlockRow {
            label: "Status".into(),
            value: "Missing".into(),
            tone: Tone::Warning,
            hint: "Games may not start without them".into(),
            ..Default::default()
        }
    };
    let mut rows = vec![state];
    if !status.last.is_empty() {
        let tone = if status.failed { Tone::Danger } else { Tone::Success };
        rows.push(BlockRow {
            label: "Last import".into(),
            value: String::new(),
            hint: status.last.clone(),
            tone,
            ..Default::default()
        });
    }
    rows.push(BlockRow {
        label: "Import…".into(),
        value: String::new(),
        hint: format!("Pick {}: see the guide in the docs", files.archive),
        action: (!status.importing).then(|| launcher_api::HOST_IMPORT_SYSTEM_FILES.to_owned()),
        ..Default::default()
    });
    SlotItem {
        id: "system-files".into(),
        order: launcher_api::Order::default(),
        content: SlotContent::Block { title: files.name.clone(), rows },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_api::RelPath;

    fn part(from: &str, files: &[&str], required: &[&str], replace: bool) -> SystemFilesPart {
        SystemFilesPart {
            from: RelPath::new(from).unwrap(),
            folder: None,
            into: RelPath::new(from).unwrap(),
            files: files.iter().map(|s| s.to_string()).collect(),
            required: required.iter().map(|s| s.to_string()).collect(),
            replace,
        }
    }

    /// Firmware that replaces its folder, and keys that join theirs.
    fn switch() -> SystemFiles {
        SystemFiles {
            name: "Switch firmware and keys".into(),
            archive: "switch.zip".into(),
            optional: false,
            parts: vec![
                part("firmware", &["*.nca"], &[], true),
                part("keys", &["prod.keys", "title.keys"], &["prod.keys"], false),
            ],
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        out.sort();
        out
    }

    /// An unpacked archive, and the two target folders under `out`.
    fn setup() -> (tempfile::TempDir, PathBuf, Vec<PathBuf>) {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("unpacked");
        write(&archive.join("firmware/a.nca"), "new a");
        write(&archive.join("firmware/b.cnmt.nca"), "new b");
        write(&archive.join("firmware/VERSION"), "22.5.0");
        write(&archive.join("keys/prod.keys"), "prod");
        write(&archive.join("keys/title.keys"), "title");
        write(&archive.join("README.md"), "how to");
        let dirs = vec![temp.path().join("out/registered"), temp.path().join("out/keys")];
        (temp, archive, dirs)
    }

    #[test]
    fn an_import_replaces_the_firmware_whole_and_adds_the_keys_beside_the_player_s_files() {
        let (_temp, archive, dirs) = setup();
        write(&dirs[0].join("old.nca"), "old firmware");
        write(&dirs[1].join("prod.keys"), "old prod");
        write(&dirs[1].join("mine.txt"), "the player's");

        assert_eq!(install(&archive, &switch(), &dirs), Ok(4));

        assert_eq!(names(&dirs[0]), ["a.nca", "b.cnmt.nca"], "no old firmware, no other file");
        assert_eq!(read(&dirs[1].join("prod.keys")), "prod");
        assert_eq!(read(&dirs[1].join("mine.txt")), "the player's", "the player's own files stay");
        assert!(installed(&switch(), &dirs));
        let out = dirs[0].parent().unwrap();
        assert_eq!(names(out), ["keys", "registered"], "no work folder stays");
    }

    #[test]
    fn an_archive_of_the_folder_itself_imports_and_its_checksums_are_checked() {
        let (_temp, archive, dirs) = setup();
        let wrapped = archive.parent().unwrap().join("wrapped");
        std::fs::create_dir_all(&wrapped).unwrap();
        std::fs::rename(&archive, wrapped.join("switch")).unwrap();
        let root = wrapped.join("switch");
        let sum = |text: &str| hex::encode(sha2::Sha256::digest(text));
        write(
            &root.join("SHA256SUMS"),
            &format!("{}  firmware/a.nca\n{} *./keys/prod.keys\n", sum("new a"), sum("prod")),
        );
        assert_eq!(install(&wrapped, &switch(), &dirs), Ok(4));

        write(&root.join("keys/prod.keys"), "changed");
        let refused = install(&wrapped, &switch(), &dirs).unwrap_err();
        assert!(refused.contains("keys/prod.keys isn't the file SHA256SUMS lists"), "{refused}");
        write(&root.join("SHA256SUMS"), &format!("{}  ../outside\n", sum("x")));
        assert!(install(&wrapped, &switch(), &dirs).unwrap_err().contains("outside the archive"));
    }

    #[test]
    fn a_wrong_archive_changes_nothing_and_says_what_it_should_hold() {
        let (_temp, archive, dirs) = setup();
        write(&dirs[0].join("old.nca"), "old firmware");
        std::fs::remove_dir_all(archive.join("keys")).unwrap();
        let refused = install(&archive, &switch(), &dirs).unwrap_err();
        assert!(
            refused.contains("This isn't switch.zip: it has no keys/ folder. It should hold firmware/, keys/"),
            "{refused}"
        );
        assert_eq!(names(&dirs[0]), ["old.nca"], "nothing changed");

        write(&archive.join("keys/title.keys"), "title");
        let refused = install(&archive, &switch(), &dirs).unwrap_err();
        assert_eq!(refused, "keys/ in the archive has no prod.keys");
        assert_eq!(names(&dirs[0]), ["old.nca"], "nothing changed");
    }

    #[test]
    fn when_a_part_can_t_be_written_every_part_goes_back() {
        let (_temp, archive, dirs) = setup();
        write(&dirs[0].join("old.nca"), "old firmware");
        // A file where the keys' folder should be: the keys can't go in.
        write(&dirs[1], "in the way");
        let refused = install(&archive, &switch(), &dirs).unwrap_err();
        assert!(refused.ends_with("Nothing changed"), "{refused}");
        assert_eq!(names(&dirs[0]), ["old.nca"], "the firmware is put back");
        assert_eq!(read(&dirs[0].join("old.nca")), "old firmware");
        let out = dirs[0].parent().unwrap();
        assert_eq!(names(out), ["keys", "registered"], "no work folder stays");
    }

    #[test]
    fn the_status_says_missing_until_every_part_has_its_files() {
        let (_temp, _archive, dirs) = setup();
        assert!(!installed(&switch(), &dirs));
        write(&dirs[0].join("a.nca"), "a");
        write(&dirs[1].join("title.keys"), "t");
        assert!(!installed(&switch(), &dirs), "prod.keys is required");
        write(&dirs[1].join("prod.keys"), "p");
        assert!(installed(&switch(), &dirs));
    }
}

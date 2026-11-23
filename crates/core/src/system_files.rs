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
//!   - copies the files each part takes into a folder beside its target
//!     (`<target>.import-part`), so the swap is a rename on one disk;
//!   - swaps them in: a `replace` part's whole folder, another part's files one by one. When a
//!     part can't be swapped in, every part goes back as it was.
//!
//! The import writes nothing until every part is checked: a wrong archive changes nothing. One
//! import runs at a time, as the RetroArch emulators share one folder. An import that stopped
//! halfway (the app quit) left `<target>.import-old`: the next import puts a whole folder back
//! from it, and refuses while it holds files it can't place, naming it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use launcher_api::{PluginId, SystemFiles, SystemFilesPart};
use sha2::Digest;

use crate::{ArchiveKind, Core, CoreError, CoreResult};

/// The work folder of an import, in the data folder: `tmp/system-files/<plugin>`.
const WORK: &str = "tmp/system-files";
/// The folder beside a target that holds the files an import copied, before they swap in.
const STAGED: &str = ".import-part";
/// The folder beside a target that holds what an import swapped out, until it ends.
const SWAPPED_OUT: &str = ".import-old";

/// What the page says of a plugin's system files.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Status {
    /// Every part's files are in place.
    pub installed: bool,
    /// An import runs.
    pub importing: bool,
    /// How far the import's unpack is, from 0 to 1.
    pub progress: f32,
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
        dirs.push(part.into.under(&base));
    }
    Ok((files, dirs))
}

/// Every part's files are in place: its required files, or one file it takes.
pub(crate) fn installed(files: &SystemFiles, dirs: &[PathBuf]) -> bool {
    files.parts.iter().zip(dirs).all(|(part, dir)| {
        let found = taken(part, dir).unwrap_or_default();
        !found.is_empty() && missing(part, &found).is_none()
    })
}

/// The first file `part` requires that isn't among `found`.
fn missing<'a>(part: &'a SystemFilesPart, found: &[PathBuf]) -> Option<&'a str> {
    let names: Vec<String> = found.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into()).collect();
    part.required.iter().map(String::as_str).find(|r| !names.iter().any(|n| n.eq_ignore_ascii_case(r)))
}

/// `path` is a folder itself, not a link to one.
fn real_folder(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
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
    let at_top = |root: &Path| files.parts.iter().any(|p| real_folder(&p.from.under(root)));
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

/// Checks the files `SHA256SUMS` lists at `root`, when it is there. A line is a checksum and a
/// path, as `sha256sum` writes it: `*` before the path for a binary read, a backslash before the
/// line for a path with a backslash or a line break in it, and backslashes between folders as on
/// Windows.
fn check_sums(root: &Path) -> Result<usize, String> {
    let list = root.join(launcher_api::SYSTEM_FILES_CHECKSUMS);
    let Ok(text) = std::fs::read_to_string(&list) else { return Ok(0) };
    let mut checked = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (escaped, line) = match line.strip_prefix('\\') {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let parsed = line
            .split_once(char::is_whitespace)
            .filter(|(sum, _)| sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()));
        let Some((sum, name)) = parsed else {
            return Err(format!("SHA256SUMS has a line that isn't a checksum and a file: {line}"));
        };
        let mut name = name.trim_start().trim_start_matches('*').to_owned();
        if escaped {
            name = name.replace("\\n", "\n").replace("\\\\", "\\");
        }
        let name = name.replace('\\', "/");
        let name = name.trim_start_matches("./");
        let path =
            launcher_api::RelPath::new(name).map_err(|_| format!("SHA256SUMS lists {name}, outside the archive"))?;
        let file = path.under(root);
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
    /// The whole folder: the old one, if there was one, is at `<dir>.import-old`.
    Folder { dir: PathBuf, had: bool },
    /// Files one by one: each new file, and whether an old one waits at
    /// `<dir>.import-old/<file>`.
    Files { dir: PathBuf, files: Vec<(PathBuf, bool)> },
}

/// Gets the targets ready for an import: an earlier import's staged copies go, and a whole
/// folder an import swapped out and never put back comes back. A swapped-out folder that can't
/// come back stops the import: it may hold the player's only copy.
fn recover(dirs: &[PathBuf]) -> Result<(), String> {
    for dir in dirs {
        let staged = beside(dir, STAGED);
        if staged.exists() {
            std::fs::remove_dir_all(&staged).map_err(|e| format!("{} can't be cleared: {e}", staged.display()))?;
        }
        let old = beside(dir, SWAPPED_OUT);
        if !old.exists() {
            continue;
        }
        if !dir.exists() {
            std::fs::rename(&old, dir).map_err(|e| format!("{} can't be put back: {e}", old.display()))?;
        } else {
            return Err(format!(
                "An import that stopped left {}: move what you need out of it, delete it, then import again",
                old.display()
            ));
        }
    }
    Ok(())
}

/// Copies the system files of the archive unpacked at `unpacked` into the parts' folders
/// `dirs`, as the module says. The number of files it copied, or why it changed nothing.
pub(crate) fn install(unpacked: &Path, files: &SystemFiles, dirs: &[PathBuf]) -> Result<usize, String> {
    let root = archive_root(files, unpacked);
    // A list beside the archive's one folder covers the files in it too.
    if root != unpacked {
        check_sums(unpacked)?;
    }
    check_sums(&root)?;
    let expected = files.parts.iter().map(|p| format!("{}/", p.from.as_str())).collect::<Vec<_>>().join(", ");
    let mut plans: Vec<Vec<PathBuf>> = Vec::new();
    for part in &files.parts {
        let from = part.from.under(&root);
        let found = taken(part, &from).filter(|_| real_folder(&from)).ok_or_else(|| {
            format!("This isn't {}: it has no {}/ folder. It should hold {expected}", files.archive, part.from.as_str())
        })?;
        if found.is_empty() {
            return Err(format!("{}/ in the archive has none of the files {} needs", part.from.as_str(), files.name));
        }
        if let Some(name) = missing(part, &found) {
            return Err(format!("{}/ in the archive has no {name}", part.from.as_str()));
        }
        plans.push(found);
    }
    recover(dirs)?;
    // Staged beside each target, so each swap is a rename on one disk.
    let staged: Vec<PathBuf> = dirs.iter().map(|d| beside(d, STAGED)).collect();
    let copied = (|| -> std::io::Result<usize> {
        let mut count = 0;
        for ((part, found), stage) in files.parts.iter().zip(&plans).zip(&staged) {
            for rel in found {
                let to = stage.join(rel);
                std::fs::create_dir_all(to.parent().unwrap_or(stage))?;
                std::fs::copy(part.from.under(&root).join(rel), &to)?;
                count += 1;
            }
        }
        Ok(count)
    })();
    let count = match copied {
        Ok(count) => count,
        Err(e) => {
            let left = clear(&staged);
            return Err(format!("The files couldn't be copied: {e}. Nothing changed{left}"));
        }
    };
    let mut done: Vec<Swapped> = Vec::new();
    for ((part, found), (dir, stage)) in files.parts.iter().zip(&plans).zip(dirs.iter().zip(&staged)) {
        if let Err(e) = swap_in(part, found, dir, stage, &mut done) {
            let stuck: Vec<String> = done.into_iter().rev().flat_map(put_back).collect();
            let left = clear(&staged);
            if !stuck.is_empty() {
                return Err(format!(
                    "{} couldn't be written: {e}. These couldn't be put back: {}",
                    dir.display(),
                    stuck.join(", ")
                ));
            }
            let left_old = clear(&dirs.iter().map(|d| beside(d, SWAPPED_OUT)).collect::<Vec<_>>());
            return Err(format!("{} couldn't be written: {e}. Nothing changed{left}{left_old}", dir.display()));
        }
    }
    let left = clear(&staged);
    let left_old = clear(&dirs.iter().map(|d| beside(d, SWAPPED_OUT)).collect::<Vec<_>>());
    if !(left.is_empty() && left_old.is_empty()) {
        tracing::warn!("system files imported, but work folders stay:{left}{left_old}");
    }
    Ok(count)
}

/// Deletes the import's own work folders `dirs`. Empty when all went, else a sentence part that
/// names the ones that stay.
fn clear(dirs: &[PathBuf]) -> String {
    let stuck: Vec<String> = dirs
        .iter()
        .filter(|d| d.exists() && std::fs::remove_dir_all(d).is_err())
        .map(|d| d.display().to_string())
        .collect();
    if stuck.is_empty() { String::new() } else { format!(", but {} stay", stuck.join(", ")) }
}

/// Swaps one part's staged files in, and records how to put it back.
fn swap_in(
    part: &SystemFilesPart,
    found: &[PathBuf],
    dir: &Path,
    stage: &Path,
    done: &mut Vec<Swapped>,
) -> std::io::Result<()> {
    let old = beside(dir, SWAPPED_OUT);
    if part.replace {
        let had = dir.exists();
        if had {
            std::fs::rename(dir, &old)?;
        }
        if let Err(e) = std::fs::rename(stage, dir) {
            if had && let Err(back) = std::fs::rename(&old, dir) {
                return Err(std::io::Error::other(format!("{e}, and {} couldn't be put back: {back}", old.display())));
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

/// Puts a part back as it was before the import. The paths it couldn't put back, if any.
fn put_back(swapped: Swapped) -> Vec<String> {
    let mut stuck = Vec::new();
    let mut fail = |path: &Path, done: std::io::Result<()>| {
        if done.is_err() {
            stuck.push(path.display().to_string());
        }
    };
    match swapped {
        Swapped::Folder { dir, had } => {
            fail(&dir, std::fs::remove_dir_all(&dir));
            if had {
                fail(&dir, std::fs::rename(beside(&dir, SWAPPED_OUT), &dir));
            }
        }
        Swapped::Files { dir, files } => {
            let old = beside(&dir, SWAPPED_OUT);
            for (rel, had) in files.into_iter().rev() {
                let to = dir.join(&rel);
                fail(&to, std::fs::remove_file(&to));
                if had {
                    fail(&to, std::fs::rename(old.join(&rel), &to));
                }
            }
        }
    }
    stuck
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
    if !builds.start_import(plugin) {
        return Err(CoreError::Invalid("Another import of system files runs: wait for it".into()));
    }
    let work = core.kernel().host().data_dir.join(WORK).join(plugin.as_str());
    let _ = tokio::fs::remove_dir_all(&work).await;
    let result = async {
        tokio::fs::create_dir_all(&work).await.map_err(|e| e.to_string())?;
        let ports = core.ports();
        let free = ports.drives.free_bytes(&work);
        // The bar moves at each whole percent.
        let progress: crate::UnpackProgress = {
            let (builds, plugin) = (builds.clone(), plugin.clone());
            Arc::new(move |done, total| {
                let part = if total > 0 { (done as f32 / total as f32).min(1.0) } else { 0.0 };
                let status = builds.system_files(&plugin);
                if (part * 100.0).floor() != (status.progress * 100.0).floor() {
                    builds.set_system_files(&plugin, Status { progress: part, ..status });
                }
            })
        };
        let parts = [file.to_path_buf()];
        let unpacked =
            ports.archives.unpack(kind, &parts, &work, free, progress, Arc::new(AtomicBool::new(false)), &[]);
        unpacked.await.map_err(|e| e.to_string())?;
        let (from, to, what) = (work.clone(), dirs.clone(), files.clone());
        tokio::task::spawn_blocking(move || install(&from, &what, &to)).await.map_err(|e| e.to_string())?
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    let (last, failed) = match &result {
        Ok(count) => (imported_text(*count), false),
        Err(why) => (why.clone(), true),
    };
    let here = installed(&files, &dirs);
    builds.set_system_files(plugin, Status { installed: here, importing: false, progress: 0.0, last, failed });
    // The plugins that share a folder with this one see the files too.
    for other in core.kernel().of_kind(launcher_api::PluginKind::Emulator) {
        if &other != plugin {
            refresh(core, &other).await;
        }
    }
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
        BlockRow {
            label: "Status".into(),
            value: "Importing…".into(),
            progress: Some(status.progress),
            ..Default::default()
        }
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
    fn an_import_that_stopped_halfway_is_put_right_or_named_and_never_deleted() {
        let (_temp, archive, dirs) = setup();
        // The app quit after it swapped the old firmware out, before the new one came in.
        write(&beside(&dirs[0], SWAPPED_OUT).join("old.nca"), "old firmware");
        write(&beside(&dirs[1], STAGED).join("prod.keys"), "half copied");
        assert_eq!(install(&archive, &switch(), &dirs), Ok(4));
        assert_eq!(names(&dirs[0]), ["a.nca", "b.cnmt.nca"]);

        // Old files beside a target that is there: they may be the player's only copy.
        write(&beside(&dirs[1], SWAPPED_OUT).join("prod.keys"), "the player's old keys");
        let refused = install(&archive, &switch(), &dirs).unwrap_err();
        assert!(refused.starts_with("An import that stopped left"), "{refused}");
        assert_eq!(read(&beside(&dirs[1], SWAPPED_OUT).join("prod.keys")), "the player's old keys");
    }

    #[test]
    fn checksums_read_as_sha256sum_writes_them_on_every_system() {
        let (_temp, archive, dirs) = setup();
        let sum = |text: &str| hex::encode(sha2::Sha256::digest(text));
        let list = format!("{}  firmware\\a.nca\n\\{} *keys/prod.keys\n", sum("new a"), sum("prod"));
        write(&archive.join("SHA256SUMS"), &list);
        assert_eq!(install(&archive, &switch(), &dirs), Ok(4));
        write(&archive.join("SHA256SUMS"), "abc  firmware/a.nca\n");
        let refused = install(&archive, &switch(), &dirs).unwrap_err();
        assert!(refused.contains("isn't a checksum and a file"), "{refused}");
    }

    #[cfg(unix)]
    #[test]
    fn a_part_s_folder_that_is_a_link_is_no_folder_of_the_archive() {
        let (temp, archive, dirs) = setup();
        let outside = temp.path().join("outside");
        write(&outside.join("prod.keys"), "not from the archive");
        std::fs::remove_dir_all(archive.join("keys")).unwrap();
        std::os::unix::fs::symlink(&outside, archive.join("keys")).unwrap();
        let refused = install(&archive, &switch(), &dirs).unwrap_err();
        assert!(refused.contains("it has no keys/ folder"), "{refused}");
        assert!(!dirs[1].exists(), "nothing written");
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

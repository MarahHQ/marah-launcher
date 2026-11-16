//! The product's names come from `brand.toml`. The values are pinned here, so a name that
//! leaves the program (a folder, a file, an id) changes only on purpose. And the name may stay
//! only in the files listed in `tests/fixtures/brand/carry-the-name.txt`: a new file that
//! holds it fails, and so does a listed file that no longer does, so the list only shrinks.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use launcher_api::brand;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Each name that leaves the program, as it is today. Changing one moves what players have:
/// change it here on purpose, with the move it needs.
#[test]
fn the_names_that_leave_the_program_are_pinned() {
    assert_eq!(brand::FILES_DATA_FOLDER, "marah-launcher");
    assert_eq!(brand::FILES_GAME_FILE, "marah_game.json");
    assert_eq!(brand::FILES_ART_FOLDER, "marah_art");
    assert_eq!(brand::FILES_BACKUP_PREFIX, "marah-backup-");
    assert_eq!(brand::FILES_BACKUPS_FOLDER, "Marah Launcher Backups");
    assert_eq!(brand::IDS_APP_ID, "io.github.marahhq.marah-launcher");
    assert_eq!(brand::IDS_UPDATE_REPO, "MarahHQ/marah-launcher");
    assert_eq!(brand::IDS_BINARY, "marah-launcher");
    assert_eq!(brand::IDS_AUTOSTART_FILE, "marah-launcher.desktop");
    assert_eq!(brand::IDS_WINDOWS_RUN_VALUE, "Marah Launcher");
    assert_eq!(brand::ENV_PREFIX, "MARAH_");
    assert_eq!(brand::OS_RELEASE_MARKER, "/usr/lib/marah-os/release");
    assert_eq!(brand::OS_INSTALLER, "/usr/libexec/marah-os/install");
    assert_eq!(brand::OS_INSTALL_COMMAND, "marah-os-install");
    assert_eq!(brand::OS_UPDATE_SERVICE, "marah-os-update.service");
    assert_eq!(brand::FILES_SANDBOX_NO_BUS, "/run/marah-sandbox/no-session-bus");
}

/// The files the walk skips: what the build makes, what git keeps, and code that isn't ours.
const SKIP: [&str; 6] = ["target", ".git", "third_party", "dist", "node_modules", ".claude"];

/// The list's own file names the product in its lines: it is the list.
const LIST: &str = "tests/fixtures/brand/carry-the-name.txt";

/// Every file under the repository that holds `words` (any of them, in any case), relative.
fn files_with(words: &[String]) -> BTreeSet<String> {
    let root = root();
    let mut found = BTreeSet::new();
    let mut folders = vec![root.clone()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    folders.push(path);
                }
                continue;
            }
            let relative = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            if relative == LIST {
                continue;
            }
            // A link's text is where it points, so a link left at an old path is found too.
            let text = match std::fs::read_link(&path) {
                Ok(target) => target.to_string_lossy().into_owned().into_bytes(),
                Err(_) => std::fs::read(&path).unwrap_or_default(),
            };
            let text = String::from_utf8_lossy(&text).to_lowercase();
            let named = words.iter().any(|w| text.contains(w.as_str()) || relative.to_lowercase().contains(w.as_str()));
            if named {
                found.insert(relative);
            }
        }
    }
    found
}

#[test]
fn the_name_is_only_in_the_listed_files() {
    let words = [brand::PRODUCT_SHORT_NAME.to_lowercase(), brand::PRODUCT_OWNER.to_lowercase()];
    let found = files_with(&words);
    let listed: BTreeSet<String> = std::fs::read_to_string(root().join(LIST))
        .unwrap()
        .lines()
        .map(|l| l.split('#').next().unwrap().trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    let new: Vec<&String> = found.difference(&listed).collect();
    let clean: Vec<&String> = listed.difference(&found).collect();
    assert!(new.is_empty(), "the name in files not on the list: take it from brand.toml instead:\n{new:#?}");
    assert!(clean.is_empty(), "these no longer hold the name: take them off {LIST}:\n{clean:#?}");
}

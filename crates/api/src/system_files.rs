//! An emulator's system files: what it needs besides its program and the games, such as
//! firmware, keys, BIOS files or system modules. The player imports them from one archive on
//! the emulator's page, and the host copies each part of the archive where the manifest says
//! (`[system_files]`):
//!
//! ```toml
//! [system_files]
//! name = "PS4 system files"
//! archive = "ps4-system-files.zip"
//!
//! [[system_files.part]]
//! from = "sys_modules"        # a folder in the archive
//! folder = "shadps4-data"     # a [[permissions.filesystem]] folder the plugin writes
//! into = "sys_modules"        # the path in that folder
//! files = ["*.sprx"]          # the files it takes, by name
//! ```
//!
//! A part without `folder` goes into the plugin's data folder, the shared one when the plugin
//! names `shared_data`. A part with `replace` swaps its whole target folder for the archive's,
//! so no old file stays beside the new ones (a firmware). The import is all or nothing: when a
//! part can't be written, every part is put back as it was.

use serde::{Deserialize, Serialize};

use crate::RelPath;

/// A slot action the host runs itself: opens the folder browser to pick the archive of the
/// plugin's system files, then imports it. The host shows the button on the plugin's page.
pub const HOST_IMPORT_SYSTEM_FILES: &str = "host:import-system-files";

/// The name of the checksum list an archive may hold at its top: `<sha256>  <path>` a line, as
/// `sha256sum` writes it. When it is there, every file it lists must be in the archive and match.
pub const SYSTEM_FILES_CHECKSUMS: &str = "SHA256SUMS";

/// `[system_files]`: what the player imports, and where each part goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFiles {
    /// What the page calls them: "PS4 system files".
    pub name: String,
    /// The archive's usual name, which the page's hint and the docs name:
    /// "ps4-system-files.zip".
    pub archive: String,
    /// Games play without them, though some need them (BIOS files): the page doesn't warn
    /// while they are missing.
    #[serde(default)]
    pub optional: bool,
    /// What goes where, at least one.
    #[serde(rename = "part")]
    pub parts: Vec<SystemFilesPart>,
}

/// `[[system_files.part]]`: one folder of the archive and where its files go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFilesPart {
    /// The folder in the archive, with its subfolders: "sys_modules", "firmware".
    pub from: RelPath,
    /// The `[[permissions.filesystem]]` folder the files go into, which the plugin writes.
    /// `None` for the plugin's data folder.
    #[serde(default)]
    pub folder: Option<String>,
    /// The folder in that folder the files go into: "nand/system/Contents/registered". Never
    /// its top: the import works in a folder beside it.
    pub into: RelPath,
    /// The files the part takes, by name: `*.nca`, `prod.keys`. A `*` stands for any text.
    /// Empty takes every file. The archive's other files stay out.
    #[serde(default)]
    pub files: Vec<String>,
    /// Files the part must have, by name: `prod.keys`. With none, the part needs one file.
    #[serde(default)]
    pub required: Vec<String>,
    /// The target folder is swapped for the archive's as a whole: none of its files stay.
    #[serde(default)]
    pub replace: bool,
}

impl SystemFilesPart {
    /// The part takes a file of this name.
    pub fn takes(&self, name: &str) -> bool {
        self.files.is_empty() || self.files.iter().any(|pattern| matches(pattern, name))
    }
}

/// `name` matches `pattern`, where each `*` stands for any text, in any case.
pub fn matches(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    let mut pieces = pattern.split('*');
    let first = pieces.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else { return false };
    let pieces: Vec<&str> = pieces.collect();
    let Some((last, middle)) = pieces.split_last() else { return rest.is_empty() };
    for piece in middle {
        match rest.find(piece) {
            Some(at) => rest = &rest[at + piece.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_s_star_stands_for_any_text_in_any_case() {
        assert!(matches("*.nca", "00c0.cnmt.nca"));
        assert!(matches("*.NCA", "a.nca"));
        assert!(matches("prod.keys", "prod.keys"));
        assert!(!matches("prod.keys", "prod.keys.bak"));
        assert!(!matches("*.nca", "a.nca.part"));
        assert!(matches("scph*.bin", "scph5501.bin"));
        assert!(matches("*", "anything"));
        assert!(!matches("a*b*c", "ab"));
        assert!(matches("a*b*c", "a-b-c"));
    }

    #[test]
    fn a_part_takes_every_file_without_names() {
        let part = SystemFilesPart {
            from: RelPath::new("system").unwrap(),
            folder: None,
            into: RelPath::root(),
            files: Vec::new(),
            required: Vec::new(),
            replace: false,
        };
        assert!(part.takes("bios.bin"));
        let keys = SystemFilesPart { files: vec!["prod.keys".into(), "title.keys".into()], ..part };
        assert!(keys.takes("title.keys") && !keys.takes("title.keys_autogenerated"));
    }
}

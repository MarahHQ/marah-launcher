//! `plugin.toml` and `theme.toml`: one manifest format for every kind of plugin.

use std::collections::BTreeSet;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::{PluginId, RelPath};

/// What a plugin is. Every kind but `theme` has code ([`crate::CodePlugin`]) and may provide
/// any data capability; the kind decides only the privileges. Only a `system` plugin may
/// provide the system capabilities ([`Capability::SYSTEM`]), run programs long
/// ([`Permissions::long_running`]) or through polkit ([`Permissions::polkit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// Runs games: catalog, builds, settings, launching.
    Emulator,
    /// Adds to the games other plugins find, and runs none itself: a store's art and details,
    /// for example.
    Service,
    /// Files only: no code and no init.
    Theme,
    /// The machine's own settings and devices: network, VPNs, Bluetooth, sound, power, and on
    /// Marah OS its updates and installer.
    System,
}

/// A console an emulator plays, as its manifest names it: `PS5`, `PS4`, `PC`. It is kept in
/// uppercase, so `ps5` and `PS5` are one console. A name has ASCII letters, digits and `-`
/// only, so it is safe in a file name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Console(String);

impl Console {
    /// The console `name`, in any case. An error for an empty name or one with other
    /// characters.
    pub fn new(name: &str) -> Result<Self, String> {
        let name = name.trim();
        let plain = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !plain {
            return Err(format!("a console is ASCII letters, digits and -, not {name:?}"));
        }
        Ok(Self(name.to_ascii_uppercase()))
    }

    /// The name in uppercase: `PS5`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name of the console's catalog file, without `.json`: the name in lowercase, `ps5`.
    pub fn file_name(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl std::fmt::Display for Console {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Console {
    type Error = String;
    fn try_from(name: String) -> Result<Self, String> {
        Self::new(&name)
    }
}

impl From<Console> for String {
    fn from(console: Console) -> String {
        console.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    /// The platform this binary was built for, if the launcher supports it.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else {
            None
        }
    }
}

/// An extension point a plugin can provide. The manifest declares them; the host checks that
/// `init` provides exactly the declared set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Catalog,
    Downloader,
    Installer,
    Configurator,
    Launcher,
    Compatibility,
    Slots,
    GameInstaller,
    /// Describes games from a store's catalog: art, rating, genres, description. See
    /// [`crate::Metadata`].
    Metadata,
    /// Lists the games an emulator's catalog offers to download. See [`crate::Releases`].
    Releases,
    /// Names what of the plugin's data a backup holds. See [`crate::Backup`].
    Backup,
    /// Work the plugin does on its own at the start, which the boot screen waits for. See
    /// [`crate::StartupWork`].
    Startup,
    /// A theme manifest declares only this. The theme's files come from [`crate::ThemePlugin`],
    /// not from `init`, so no emulator can provide it.
    Theme,
    /// System plugins only, like the rest below: network and Wi-Fi. Each system capability is
    /// one [`crate::SystemArea`], provided through [`crate::AreaSettings`].
    Network,
    Vpn,
    Bluetooth,
    Audio,
    Power,
    OsUpdates,
    OsInstall,
    /// Not `storage`: that is a plugin's own data folder.
    Drives,
}

impl Capability {
    /// The capabilities only a system plugin provides.
    pub const SYSTEM: [Capability; 8] = [
        Capability::Network,
        Capability::Vpn,
        Capability::Bluetooth,
        Capability::Audio,
        Capability::Power,
        Capability::OsUpdates,
        Capability::OsInstall,
        Capability::Drives,
    ];
}

/// What a plugin may do outside its own data folder. The host enforces every entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Hosts the plugin may reach over HTTP: an exact name (`github.com`) or a suffix
    /// pattern (`*.github.com`, which also matches `github.com`).
    #[serde(default)]
    pub network: Vec<String>,
    /// Named host folders the plugin may read, and write if `write = true`. The player maps
    /// each name to real folders; the plugin reaches them through [`crate::Folders`].
    #[serde(default)]
    pub filesystem: Vec<PathGrant>,
    /// Programs inside the data folder the plugin may launch. `*` as the last part matches
    /// any file name, for example `builds/*/kyty`. Empty means the plugin may not spawn.
    #[serde(default)]
    pub process: Vec<String>,
    /// System plugins only: entries of `process` that may run with no time limit, their output
    /// read line by line ([`crate::Programs::stream`]), for example `system:bluetoothctl`.
    #[serde(default)]
    pub long_running: Vec<String>,
    /// System plugins only: entries of `process` that may run as the administrator through
    /// polkit (`pkexec`), which asks the player when its rules say so.
    #[serde(default)]
    pub polkit: Vec<String>,
    /// Links the host may open for the plugin with the system's handler, by how they start:
    /// `steam://install/` lets a release's `store` link ask Steam to install a game. Empty means
    /// none.
    #[serde(default)]
    pub links: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathGrant {
    /// The folder's name: lowercase letters, digits and `-`, for example `pc-games`. Plugins
    /// that ask for the same name share the player's mapping once the player sets one; until
    /// then each plugin uses its own `default`.
    pub folder: String,
    #[serde(default)]
    pub write: bool,
    /// Where the folder is until the player maps it. `~/` means the home folder. The shared
    /// folders ([`SHARED_FOLDERS`]) have the host's default.
    #[serde(default)]
    pub default: Option<DefaultFolder>,
    /// Another default on some systems, for example `macos = "~/Library/Application Support/X"`.
    #[serde(default)]
    pub default_on: std::collections::BTreeMap<Platform, String>,
    /// What Settings calls the folder, for example "Packages to install". The name if none.
    #[serde(default)]
    pub title: Option<String>,
    /// A line under the title in Settings.
    #[serde(default)]
    pub help: Option<String>,
    /// On the plugin's page: the key of the setting this folder's row follows. At the end of
    /// the plugin's settings if none. Needs the plugin to have settings.
    #[serde(default)]
    pub after: Option<String>,
    /// More folders, from a list in a default folder, while the player hasn't set the folder:
    /// Steam's own folder lists its libraries on other drives.
    #[serde(default)]
    pub listed_in: Option<ListedFolders>,
    /// The folder's name before a rename. At the first start with it, the host copies the
    /// player's folders saved under it to `folder`, when `folder` has none.
    #[serde(default)]
    pub was: Option<String>,
}

/// Folders a file lists, read by the host: each value of `key` in `file` (a path in the
/// folder), in Valve's text format (`"key" "value"`), that is an absolute path to a folder
/// holding `holds`. `{ file = "steamapps/libraryfolders.vdf", key = "path", holds =
/// "steamapps" }` are the libraries Steam lists. A listed folder is read only, as the grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedFolders {
    pub file: String,
    pub key: String,
    pub holds: String,
}

/// A grant's `default`: one folder, or a list of places to look.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DefaultFolder {
    /// `default = "~/Games"`: this folder, also when it doesn't exist yet.
    One(String),
    /// `default = ["~/KytyPS5", "~/src/KytyPS5"]`: each folder in the list that exists, in
    /// order. None when none exists.
    Search(Vec<String>),
}

/// The player's game folders, shared by every plugin that asks for it: games of any console,
/// each install in its console's folder inside the first one (`PS5`, `PS4`).
pub const GAMES_FOLDER: &str = "games";

/// A game folder's file of what the launcher knows of the game. Not the product's name: a
/// rename of the product leaves players' game folders as they are.
pub const GAME_FILE: &str = "launcher_game.json";
/// A game folder's folder of art, beside [`GAME_FILE`].
pub const ART_FOLDER: &str = "launcher_art";
/// Where the games folder is until the player maps it.
pub const GAMES_DEFAULT: &str = "~/Games";
/// The player's packages to install, shared by every plugin that installs them: each plugin
/// looks in its console's folder inside each one (`PS5`, `PS4`), as installs go to the games
/// folder.
pub const PACKAGES_FOLDER: &str = "packages";
/// Where the packages folder is until the player maps it.
pub const PACKAGES_DEFAULT: &str = "~/Downloads";

/// A folder the host owns and shows once, in Settings, for every plugin that asks for it:
/// its name, default, title and help line. A plugin's own default, title and help for it
/// are left out.
pub struct SharedFolder {
    pub name: &'static str,
    pub default: &'static str,
    pub title: &'static str,
    pub help: &'static str,
}

pub const SHARED_FOLDERS: [SharedFolder; 2] = [
    SharedFolder {
        name: GAMES_FOLDER,
        default: GAMES_DEFAULT,
        title: "Game folders",
        help: "Games from any console. Installs go to PS5 or PS4 inside the first folder.",
    },
    SharedFolder {
        name: PACKAGES_FOLDER,
        default: PACKAGES_DEFAULT,
        title: "Packages to install",
        help: "Game packages (.pkg) and disc images in a console's folder, for example PS5 or PS4, can be installed.",
    },
];

/// The shared folder named `name`, if it is one.
pub fn shared_folder(name: &str) -> Option<&'static SharedFolder> {
    SHARED_FOLDERS.iter().find(|f| f.name == name)
}

impl PathGrant {
    /// The one folder the grant means until the player maps it on `platform`: the grant's
    /// default for that system, its default, or the host's. `None` when the default is a
    /// list to search: see [`PathGrant::search_paths`].
    pub fn default_path(&self, platform: Option<Platform>) -> Option<&str> {
        let (own, searched) = match &self.default {
            Some(DefaultFolder::One(path)) => (Some(path.as_str()), false),
            Some(DefaultFolder::Search(_)) => (None, true),
            None => (None, false),
        };
        platform
            .and_then(|p| self.default_on.get(&p))
            .map(String::as_str)
            .or(own)
            .or_else(|| if searched { None } else { shared_folder(&self.folder).map(|f| f.default) })
    }

    /// The places to look for the folder until the player maps it, when the default is a
    /// list and [`PathGrant::default_path`] has none. Empty otherwise.
    pub fn search_paths(&self, platform: Option<Platform>) -> &[String] {
        match &self.default {
            Some(DefaultFolder::Search(paths)) if self.default_path(platform).is_none() => paths,
            _ => &[],
        }
    }

    pub fn valid_name(name: &str) -> bool {
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }
}

/// One part of a process rule: `*` matches any name, `*.AppImage` any name with that ending.
fn part_matches(rule: &str, part: &str) -> bool {
    rule == "*" || rule == part || rule.strip_prefix('*').is_some_and(|end| !end.is_empty() && part.ends_with(end))
}

impl Permissions {
    /// Whether the plugin may have the host open `link`: it starts with one of `links`, and is
    /// one line of plain characters.
    pub fn allows_link(&self, link: &str) -> bool {
        let plain = !link.is_empty() && link.chars().all(|c| c.is_ascii_graphic());
        plain && self.links.iter().any(|prefix| !prefix.is_empty() && link.starts_with(prefix.as_str()))
    }

    /// The grant for a named folder, if the manifest asks for it.
    pub fn folder(&self, name: &str) -> Option<&PathGrant> {
        self.filesystem.iter().find(|g| g.folder == name)
    }

    pub fn allows_host(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.network.iter().any(|rule| {
            let rule = rule.to_ascii_lowercase();
            match rule.strip_prefix("*.") {
                Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
                None => host == rule,
            }
        })
    }

    /// Whether `permissions.process` lets the plugin start `program`:
    ///
    /// - in the data folder, a path pattern such as `builds/*/kyty` or `builds/*/*.AppImage`;
    /// - in a host folder, `folder:<name>` (any file in it), for a folder the manifest asks for;
    /// - on the system `PATH`, `system:<name>`, such as `system:wine`.
    pub fn allows_program(&self, program: &crate::Location) -> bool {
        use crate::Location;
        match program {
            Location::Data(path) => {
                let parts: Vec<&str> = path.as_str().split('/').collect();
                self.process.iter().filter(|rule| !rule.contains(':')).any(|rule| {
                    let rule_parts: Vec<&str> = rule.split('/').collect();
                    rule_parts.len() == parts.len() && rule_parts.iter().zip(&parts).all(|(r, p)| part_matches(r, p))
                })
            }
            Location::Folder(path) => {
                self.folder(&path.folder).is_some()
                    && self.process.iter().any(|rule| rule.strip_prefix("folder:") == Some(path.folder.as_str()))
            }
            Location::System(name) => {
                let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != "..";
                plain && self.process.iter().any(|rule| rule.strip_prefix("system:") == Some(name.as_str()))
            }
        }
    }

    /// Whether `program` may run long ([`Permissions::long_running`]).
    pub fn allows_long_running(&self, program: &crate::Location) -> bool {
        self.allows_program(program) && Self::listed(&self.long_running, program)
    }

    /// Whether `program` may run through polkit ([`Permissions::polkit`]).
    pub fn allows_polkit(&self, program: &crate::Location) -> bool {
        self.allows_program(program) && Self::listed(&self.polkit, program)
    }

    /// Whether a system program is named in `rules` (`system:<name>`).
    fn listed(rules: &[String], program: &crate::Location) -> bool {
        let crate::Location::System(name) = program else { return false };
        rules.iter().any(|rule| rule.strip_prefix("system:") == Some(name.as_str()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub id: PluginId,
    #[serde(default = "any_version")]
    pub version: VersionReq,
    /// An optional dependency is loaded first when present, and ignored when missing.
    #[serde(default)]
    pub optional: bool,
}

fn any_version() -> VersionReq {
    VersionReq::STAR
}

/// When the host calls `init`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activation {
    /// At boot.
    Eager,
    /// The first time the host needs one of the plugin's capabilities.
    #[default]
    Lazy,
}

/// How the plugin's code runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Runtime {
    /// Compiled into the host binary (first-party only).
    #[default]
    Native,
    /// No code: a data-only theme.
    Data,
}

/// What a plugin's games need from the computer (`[requires]`). Games won't start without
/// `vulkan` or `avx2`; below `recommended` they may run slowly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Requirements {
    /// The Vulkan version the graphics card must support, for example `"1.3"`.
    pub vulkan: Option<String>,
    /// The processor must have AVX2 (x86-64-v3). Not checked where the system can't say.
    pub avx2: bool,
    pub recommended: Recommended,
}

/// What a plugin's games need to run well (`[requires.recommended]`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Recommended {
    pub cpu_threads: Option<u32>,
    pub memory_gb: Option<f32>,
    pub gpu_memory_gb: Option<f32>,
    /// A graphics card of its own, not one built into the processor.
    pub discrete_gpu: bool,
}

/// One button of the console and the keyboard keys that press it: "Cross" and "J".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBinding {
    pub button: String,
    pub keys: String,
}

/// The manifest shared by emulator and theme plugins.
///
/// Sections for one kind only (such as `[theme]`) are kept in `extra` and parsed by the crate
/// that owns them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: PluginId,
    pub kind: PluginKind,
    /// What the player sees for the plugin: in the plugin list, on its page, and where a
    /// sentence means the console it plays. An emulator names its console: `PS5`.
    pub name: String,
    /// For an emulator, the name of the program it installs and runs, for example `KytyPS5`.
    /// Sentences about the program use it: its install, its builds, its first-play prompt.
    /// Empty when the program has no name of its own (PC). See [`PluginManifest::emulator_name`].
    #[serde(default)]
    pub emulator: String,
    pub version: Version,
    /// The `launcher-api` versions this plugin works with.
    pub host_api: VersionReq,
    /// The systems the plugin runs on. Empty means every system.
    #[serde(default)]
    pub platforms: Vec<Platform>,
    /// For an emulator, the console it plays, for example `PS5`, `PS4` or `PC`. A catalog
    /// plugin gives the emulator that console's catalog. `None` gives it no catalog. Not the
    /// systems the plugin runs on: see `platforms`.
    #[serde(default)]
    pub console: Option<Console>,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub activation: Activation,
    #[serde(default)]
    pub runtime: Runtime,
    /// Folder of images and other files the plugin's slot data may refer to.
    #[serde(default)]
    pub assets: Option<RelPath>,
    /// JSON Schema of the plugin-wide settings, usually generated with `schemars`.
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    /// What the computer needs for the plugin's games: the host compares it with this PC.
    #[serde(default)]
    pub requires: Requirements,
    /// An emulator's keyboard keys for the console's buttons (`[[keymap]]`), in the order the
    /// launch splash lists them. Empty when the plugin lists none.
    #[serde(default)]
    pub keymap: Vec<KeyBinding>,
    /// For an emulator with a game installer: what it installs from, in one or two sentences.
    /// The install sheet shows it. Empty when the plugin says nothing.
    #[serde(default)]
    pub install_note: String,
    /// The system the plugin's games show under (`[system]`): what the Library's chips and a
    /// card's badge call it, and its color. Themes draw it from this, not from the system's code.
    #[serde(default)]
    pub system: Option<SystemLook>,
    /// The data folder this plugin shares with every plugin that names the same one
    /// (`shared_data = "retroarch"`): its storage, downloads and programs live there, not in a
    /// folder of its own. For emulators that run one program between them.
    #[serde(default)]
    pub shared_data: Option<ShareName>,
    /// The files the emulator needs besides its program and the games, which the player imports
    /// from one archive on its page (`[system_files]`): see [`crate::SystemFiles`].
    #[serde(default)]
    pub system_files: Option<crate::SystemFiles>,
    /// For an emulator that plays several consoles (`[[consoles]]`): each console, with what
    /// the single fields above say for one (`console`, `[system]`, `keymap`, `install_note`,
    /// `[system_files]`). The player turns each on or off on the plugin's page. A manifest
    /// with `[[consoles]]` leaves those single fields out. Read both forms with
    /// [`PluginManifest::played`].
    #[serde(default)]
    pub consoles: Vec<PluginConsole>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// One console of an emulator that plays several (`[[consoles]]`). Its `console` and its
/// `[consoles.system]` id are one code (`NES`): its games' `platform`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConsole {
    pub console: Console,
    pub system: SystemLook,
    #[serde(default)]
    pub keymap: Vec<KeyBinding>,
    #[serde(default)]
    pub install_note: String,
    #[serde(default)]
    pub system_files: Option<crate::SystemFiles>,
    /// The console is on until the player turns it off. With `false`, it is off until they
    /// turn it on.
    #[serde(default = "on_by_default")]
    pub on: bool,
}

fn on_by_default() -> bool {
    true
}

/// One console an emulator plays, from `[[consoles]]` or from the manifest's single fields: the
/// form the host reads. See [`PluginManifest::played`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played<'a> {
    /// The console whose catalog the emulator gets. `None` gets it no catalog.
    pub console: Option<&'a Console>,
    /// The system its games show under.
    pub system: Option<&'a SystemLook>,
    pub keymap: &'a [KeyBinding],
    pub install_note: &'a str,
    pub system_files: Option<&'a crate::SystemFiles>,
    /// Its manifest turns it on until the player says otherwise.
    pub on: bool,
    /// It is one of `[[consoles]]`: the player can turn it on or off by itself.
    pub listed: bool,
}

impl<'a> Played<'a> {
    /// The code its games' `platform` has: its system's id, else its console's.
    pub fn platform(&self) -> Option<&'a str> {
        self.system.map(|s| s.id.as_str()).or(self.console.map(Console::as_str))
    }
}

/// The name of a data folder plugins share: lowercase ASCII letters, digits and `-`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ShareName(String);

impl ShareName {
    pub fn new(name: &str) -> Result<Self, String> {
        let plain = !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !plain {
            return Err(format!("a shared data folder's name is lowercase letters, digits and -, not {name:?}"));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ShareName {
    type Error = String;
    fn try_from(name: String) -> Result<Self, String> {
        Self::new(&name)
    }
}

impl From<ShareName> for String {
    fn from(name: ShareName) -> String {
        name.0
    }
}

/// A system's look: `id` is the code a game's `platform` has (`PS5`, `PC`, `Steam`), `name` its
/// full name ("PlayStation 5"), and `color` its color as `#rrggbb`. Without a color a theme
/// draws the system in its neutral look. `icon` and `cover` are pictures in the plugin's assets:
/// a square icon, and a wide picture a theme can show behind the system. A theme can put its own
/// in their place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemLook {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub icon: Option<RelPath>,
    #[serde(default)]
    pub cover: Option<RelPath>,
}

impl SystemLook {
    /// `color` as red, green and blue; `None` without one, or when it isn't `#rrggbb`.
    pub fn rgb(&self) -> Option<[u8; 3]> {
        let hex = self.color.as_deref()?.strip_prefix('#').filter(|h| h.len() == 6)?;
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        Some([byte(0)?, byte(2)?, byte(4)?])
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("invalid manifest: {0}")]
pub struct ManifestError(pub String);

impl PluginManifest {
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|e| ManifestError(e.message().to_owned()))
    }

    pub fn supports(&self, platform: Platform) -> bool {
        self.platforms.is_empty() || self.platforms.contains(&platform)
    }

    /// The name of the program the plugin runs: `emulator`, or `name` when it has none.
    pub fn emulator_name(&self) -> &str {
        if self.emulator.is_empty() { &self.name } else { &self.emulator }
    }

    /// The consoles the plugin plays: each of `[[consoles]]`, or one from the single fields. A
    /// plugin with neither (a service, a theme) plays none.
    pub fn played(&self) -> Vec<Played<'_>> {
        if !self.consoles.is_empty() {
            return self
                .consoles
                .iter()
                .map(|c| Played {
                    console: Some(&c.console),
                    system: Some(&c.system),
                    keymap: &c.keymap,
                    install_note: &c.install_note,
                    system_files: c.system_files.as_ref(),
                    on: c.on,
                    listed: true,
                })
                .collect();
        }
        let single = Played {
            console: self.console.as_ref(),
            system: self.system.as_ref(),
            keymap: &self.keymap,
            install_note: &self.install_note,
            system_files: self.system_files.as_ref(),
            on: true,
            listed: false,
        };
        let plays = single.console.is_some()
            || single.system.is_some()
            || !single.keymap.is_empty()
            || !single.install_note.is_empty()
            || single.system_files.is_some();
        if plays { vec![single] } else { Vec::new() }
    }

    /// The console a game with `platform` is of: the one of `[[consoles]]` with that code, or
    /// the single one whatever the code.
    pub fn played_for(&self, platform: &str) -> Option<Played<'_>> {
        let played = self.played();
        if self.consoles.is_empty() {
            return played.into_iter().next();
        }
        played.into_iter().find(|p| p.platform().is_some_and(|c| c.eq_ignore_ascii_case(platform)))
    }

    /// The `[[consoles]]` entry for `console`.
    pub fn listed_console(&self, console: &str) -> Option<&PluginConsole> {
        self.consoles.iter().find(|c| c.console.as_str().eq_ignore_ascii_case(console))
    }
}

#[cfg(test)]
mod tests {

    /// The links a plugin may have the host open: only those its manifest lists, by how they
    /// start, and on one line.
    #[test]
    fn a_link_opens_only_as_the_manifest_allows() {
        let permissions = Permissions { links: vec!["steam://install/".into()], ..Permissions::default() };
        assert!(permissions.allows_link("steam://install/440"));
        assert!(!permissions.allows_link("steam://run/440"));
        assert!(!permissions.allows_link("steam://install/440\nrm"));
        assert!(!permissions.allows_link("steam://install/ 440"));
        assert!(!Permissions::default().allows_link("steam://install/440"));
    }
    use super::*;

    const KYTY: &str = r#"
        id = "emu-kyty"
        kind = "emulator"
        name = "Native example"
        version = "0.1.0"
        host_api = "^0.7"
        platforms = ["linux", "macos"]
        capabilities = ["catalog", "launcher"]

        [permissions]
        network = ["*.github.com"]
        process = ["builds/*/kyty"]

        [[dependencies]]
        id = "firmware"
        version = ">=1.0"
        optional = true

        [theme]
        ui_api = "^0.3"
    "#;

    #[test]
    fn parses_every_section_and_keeps_kind_specific_ones() {
        let m = PluginManifest::from_toml(KYTY).unwrap();
        assert_eq!(m.id.as_str(), "emu-kyty");
        assert_eq!(m.kind, PluginKind::Emulator);
        assert!(m.capabilities.contains(&Capability::Launcher));
        assert_eq!(m.dependencies[0].id.as_str(), "firmware");
        assert!(m.dependencies[0].optional);
        assert_eq!(m.activation, Activation::Lazy);
        assert_eq!(m.runtime, Runtime::Native);
        assert!(m.extra.contains_key("theme"));
        assert!(m.supports(Platform::Linux));
        assert!(!m.supports(Platform::Windows));
    }

    #[test]
    fn an_emulator_may_list_its_keyboard_keys_in_order() {
        let m = PluginManifest::from_toml(&format!(
            "{KYTY}\n[[keymap]]\nbutton = \"Cross\"\nkeys = \"J\"\n[[keymap]]\nbutton = \"Circle\"\nkeys = \"L\"\n"
        ))
        .unwrap();
        let keys: Vec<_> = m.keymap.iter().map(|k| (k.button.as_str(), k.keys.as_str())).collect();
        assert_eq!(keys, [("Cross", "J"), ("Circle", "L")]);
        assert!(!m.extra.contains_key("keymap"), "a field of its own");
        assert!(PluginManifest::from_toml(KYTY).unwrap().keymap.is_empty(), "none by default");
        let wrong = format!("{KYTY}\n[[keymap]]\nbutton = \"Cross\"\nkey = \"J\"\n");
        assert!(PluginManifest::from_toml(&wrong).is_err(), "a misspelt field is an error");
    }

    #[test]
    fn an_emulator_may_say_what_it_installs_from() {
        let m = PluginManifest::from_toml(&format!("install_note = \"Installs from a folder.\"\n{KYTY}")).unwrap();
        assert_eq!(m.install_note, "Installs from a folder.");
        assert!(!m.extra.contains_key("install_note"), "a field of its own");
        assert_eq!(PluginManifest::from_toml(KYTY).unwrap().install_note, "", "none by default");
        let shared = PluginManifest::from_toml(&format!("shared_data = \"retroarch\"\n{KYTY}")).unwrap();
        assert_eq!(shared.shared_data.as_ref().map(ShareName::as_str), Some("retroarch"));
        assert!(!shared.extra.contains_key("shared_data"), "a field of its own");
        assert_eq!(PluginManifest::from_toml(KYTY).unwrap().shared_data, None, "a folder of its own by default");
        assert!(PluginManifest::from_toml(&format!("shared_data = \"../x\"\n{KYTY}")).is_err(), "a plain name only");
    }

    #[test]
    fn system_files_name_their_archive_and_where_each_part_goes() {
        let text = format!(
            "{KYTY}\n[system_files]\nname = \"Switch firmware and keys\"\narchive = \"switch.zip\"\n\
             [[system_files.part]]\nfrom = \"firmware\"\nfolder = \"lichii-data\"\n\
             into = \"nand/system/Contents/registered\"\nfiles = [\"*.nca\"]\nreplace = true\n\
             [[system_files.part]]\nfrom = \"keys\"\ninto = \"keys\"\nrequired = [\"prod.keys\"]\n"
        );
        let manifest = PluginManifest::from_toml(&text).unwrap();
        let files = manifest.system_files.expect("[system_files]");
        assert_eq!((files.name.as_str(), files.archive.as_str()), ("Switch firmware and keys", "switch.zip"));
        let (firmware, keys) = (&files.parts[0], &files.parts[1]);
        assert_eq!(firmware.folder.as_deref(), Some("lichii-data"));
        assert_eq!(firmware.into.as_str(), "nand/system/Contents/registered");
        assert!(firmware.replace && firmware.takes("a.nca") && !firmware.takes("VERSION"));
        assert_eq!((keys.folder.as_deref(), keys.into.as_str(), keys.replace), (None, "keys", false));
        assert!(!files.optional, "the page warns while they are missing");
        assert_eq!(keys.required, ["prod.keys"]);
        assert!(!manifest.extra.contains_key("system_files"), "a field of its own");
        assert_eq!(PluginManifest::from_toml(KYTY).unwrap().system_files, None);
        let climbs = text.replace("from = \"keys\"", "from = \"../keys\"");
        assert!(PluginManifest::from_toml(&climbs).is_err(), "a part's folder stays in the archive");
    }

    #[test]
    fn an_emulator_of_several_consoles_lists_each_and_one_of_one_console_reads_the_same_way() {
        let text = format!(
            "{KYTY}\n[[consoles]]\nconsole = \"nes\"\ninstall_note = \"NES ROMs.\"\n\
             [consoles.system]\nid = \"NES\"\nname = \"Nintendo Entertainment System\"\n\
             [[consoles.keymap]]\nbutton = \"A\"\nkeys = \"X\"\n\
             [[consoles]]\nconsole = \"ATARI2600\"\non = false\n\
             [consoles.system]\nid = \"ATARI2600\"\nname = \"Atari 2600\"\n\
             [consoles.system_files]\nname = \"BIOS\"\narchive = \"a.zip\"\n\
             [[consoles.system_files.part]]\nfrom = \"system\"\ninto = \"system\"\n"
        );
        let m = PluginManifest::from_toml(&text).unwrap();
        assert!(!m.extra.contains_key("consoles"), "a field of its own");
        let played = m.played();
        let codes: Vec<_> = played.iter().map(|p| p.platform()).collect();
        assert_eq!(codes, [Some("NES"), Some("ATARI2600")]);
        assert!(played.iter().all(|p| p.listed));
        assert_eq!((played[0].on, played[1].on), (true, false), "on unless the manifest says off");
        assert_eq!(played[0].keymap[0].keys, "X");
        assert_eq!(played[0].install_note, "NES ROMs.");
        assert_eq!(played[1].system_files.map(|f| f.archive.as_str()), Some("a.zip"));
        assert_eq!(m.played_for("ATARI2600").and_then(|p| p.console).map(Console::as_str), Some("ATARI2600"));
        assert_eq!(m.played_for("PS1"), None, "a code it doesn't play");
        assert!(m.listed_console("atari2600").is_some());

        let one = PluginManifest::from_toml(&format!(
            "console = \"PS5\"\n{KYTY}\n[system]\nid = \"PS5\"\nname = \"PlayStation 5\"\n"
        ))
        .unwrap();
        let played = one.played();
        assert_eq!(played.len(), 1);
        assert!(played[0].on && !played[0].listed, "one console is on with its plugin");
        assert_eq!(one.played_for("anything").and_then(|p| p.platform()), Some("PS5"));
        assert!(PluginManifest::from_toml(KYTY).unwrap().played().is_empty(), "plays no console");
        let wrong = text.replace("on = false", "of = false");
        assert!(PluginManifest::from_toml(&wrong).is_err(), "a misspelt field is an error");
    }

    #[test]
    fn an_emulator_may_name_its_software_apart_from_the_plugin() {
        let m = PluginManifest::from_toml(&format!("emulator = \"KytyPS5\"\n{KYTY}")).unwrap();
        assert_eq!(m.emulator, "KytyPS5");
        assert_eq!(m.emulator_name(), "KytyPS5");
        assert_eq!(m.name, "Native example", "the plugin keeps its own name");
        assert!(!m.extra.contains_key("emulator"), "a field of its own");
        let plain = PluginManifest::from_toml(KYTY).unwrap();
        assert_eq!(plain.emulator, "", "none by default");
        assert_eq!(plain.emulator_name(), "Native example", "without one, the plugin's name");
    }

    #[test]
    fn an_emulator_names_its_console_in_any_case() {
        let with = |console: &str| {
            PluginManifest::from_toml(&format!(
                "id = \"emu\"\nkind = \"emulator\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
                 console = \"{console}\"\n"
            ))
        };
        let ps5 = with("ps5").unwrap().console.unwrap();
        assert_eq!(ps5, Console::new("PS5").unwrap(), "one console in either case");
        assert_eq!((ps5.as_str(), ps5.file_name().as_str()), ("PS5", "ps5"));
        assert!(with("../ps5").is_err(), "letters, digits and dashes only");
        assert!(Console::new(" ").is_err());
        assert_eq!(PluginManifest::from_toml(KYTY).unwrap().console, None);
    }

    #[test]
    fn folder_and_system_rules_need_their_own_pattern() {
        use crate::{HostPath, Location};
        let m = PluginManifest::from_toml(
            "id = \"pc\"\nkind = \"emulator\"\nname = \"PC\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
             [permissions]\nprocess = [\"folder:pc-games\", \"folder:not-granted\", \"system:wine\"]\n\
             [[permissions.filesystem]]\nfolder = \"pc-games\"",
        )
        .unwrap();
        let p = &m.permissions;
        let in_folder = |name: &str| Location::Folder(HostPath::new(name, 0, RelPath::new("Game/start.sh").unwrap()));
        assert!(p.allows_program(&in_folder("pc-games")));
        assert!(!p.allows_program(&in_folder("not-granted")), "the folder itself must be granted");
        assert!(!p.allows_program(&in_folder("other")));
        assert!(p.allows_program(&Location::System("wine".into())));
        assert!(!p.allows_program(&Location::System("umu-run".into())));
        assert!(!p.allows_program(&Location::System("/usr/bin/wine".into())), "a name, never a path");
        assert!(
            !p.allows_program(&Location::Data(RelPath::new("pc-games/Game/start.sh").unwrap())),
            "folder rules match no data path"
        );
    }

    #[test]
    fn network_and_process_rules_match_only_what_they_name() {
        let m = PluginManifest::from_toml(KYTY).unwrap();
        let p = &m.permissions;
        assert!(p.allows_host("github.com"));
        assert!(p.allows_host("api.GitHub.com"));
        assert!(!p.allows_host("evilgithub.com"));
        assert!(!p.allows_host("github.com.evil.net"));
        let data = |p: &str| crate::Location::Data(RelPath::new(p).unwrap());
        assert!(p.allows_program(&data("builds/1.2/kyty")));
        assert!(!p.allows_program(&data("builds/1.2/other")));
        assert!(!p.allows_program(&data("builds/1.2/x/kyty")));
        assert!(part_matches("*.AppImage", "Shadps4-sdl.AppImage"));
        assert!(!part_matches("*.AppImage", "Shadps4-sdl.AppImage.sh"));
        assert!(!part_matches("*.AppImage", ".AppImage.x"));
    }

    #[test]
    fn a_folder_default_is_one_path_or_a_list_to_search() {
        let grant = |default: &str| {
            let m = PluginManifest::from_toml(&format!(
                "id = \"emu\"\nkind = \"emulator\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.7\"\n\
                 [[permissions.filesystem]]\nfolder = \"emu-build\"\n{default}\n"
            ))
            .unwrap();
            m.permissions.filesystem[0].clone()
        };
        let one = grant("default = \"~/Emu\"");
        assert_eq!(one.default_path(None), Some("~/Emu"));
        assert!(one.search_paths(None).is_empty());

        let list = grant("default = [\"~/Emu\", \"~/src/Emu\"]\ndefault_on = { macos = \"~/Applications/Emu\" }");
        assert_eq!(list.default_path(None), None);
        assert_eq!(list.search_paths(Some(Platform::Linux)), ["~/Emu", "~/src/Emu"]);
        assert_eq!(list.default_path(Some(Platform::Macos)), Some("~/Applications/Emu"), "a system's own default wins");
        assert!(list.search_paths(Some(Platform::Macos)).is_empty());

        assert_eq!(grant("").default_path(None), None);
    }

    #[test]
    fn rejects_bad_ids_and_bad_versions() {
        let bad_id = KYTY.replace("\"emu-kyty\"", "\"Emu Native\"");
        assert!(PluginManifest::from_toml(&bad_id).is_err());
        let bad_req = KYTY.replace("\"^0.7\"", "\"not a range\"");
        assert!(PluginManifest::from_toml(&bad_req).is_err());
    }
}

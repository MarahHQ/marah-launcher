//! Installed console games in a named host folder: folders with `sce_sys/param.json` (PS5)
//! or `sce_sys/param.sfo` (PS4), Switch game files (see [`switch_file`]), and the files of a
//! retro console's games (see [`RomSystem`]), in its own folder only.
//!
//! Each of the player's folders is searched three folders down, for layouts like
//! `Games/PS5/<Title>/<PPSAxxxxx-app0>`.

use std::sync::Mutex;

use launcher_api::serde_json::{self, Value};
use launcher_api::{DirEntry, Folders, HostPath, PluginError, PluginResult, RelPath};

use crate::sfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Console {
    Ps5,
    Ps4,
    /// Switch and Switch 2 games, each one file.
    Switch,
    /// A retro console's games, each one file, in the console's own folder.
    Rom(&'static RomSystem),
}

/// A retro console's game files.
#[derive(Debug, PartialEq, Eq)]
pub struct RomSystem {
    /// Its folder inside each games folder (`NES`). Its games are looked for there alone: a
    /// `.zip` or a `.bin` elsewhere may be another console's.
    pub folder: &'static str,
    /// The extensions of a file that is a game, in lowercase: `cue`, not the `bin` it names.
    pub games: &'static [&'static str],
    /// The extensions of the files a game is made of, which an install copies: `cue` and `bin`.
    pub files: &'static [&'static str],
    /// A playlist that names a game's discs (`m3u`): a folder with one lists that alone.
    pub playlist: Option<&'static str>,
    /// The console's repository on libretro-thumbnails (`Nintendo_-_Nintendo_Entertainment_System`):
    /// its pictures are named after each game's No-Intro or Redump name, as a ROM named so is.
    pub thumbnails: Option<&'static str>,
}

/// Where libretro-thumbnails keeps its pictures.
pub const THUMBNAILS: &str = "https://raw.githubusercontent.com/libretro-thumbnails";

/// The address of the picture of `kind` (`Named_Boxarts`, `Named_Snaps`) for the game in the
/// file `name`: its name without the extension, with the characters libretro replaces
/// (`&*/:`<>?\|"`) as `_`. `None` for a console with no repository.
pub fn thumbnail(system: &RomSystem, kind: &str, name: &str) -> Option<String> {
    let repo = system.thumbnails?;
    let stem = name.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(name);
    let safe: String = stem.chars().map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c }).collect();
    let mut encoded = String::new();
    for byte in format!("{safe}.png").bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    Some(format!("{THUMBNAILS}/{repo}/master/{kind}/{encoded}"))
}

/// The lowercase extension of `name`, if it has one.
fn extension(name: &str) -> Option<String> {
    name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()).map(|(_, e)| e.to_ascii_lowercase())
}

impl RomSystem {
    /// `name` is one of this console's game files: a game, or a file a game is made of.
    pub fn is_file(&self, name: &str) -> bool {
        extension(name).is_some_and(|e| self.files.contains(&e.as_str()) || self.games.contains(&e.as_str()))
    }

    /// `name` is a game of this console.
    pub fn is_game(&self, name: &str) -> bool {
        extension(name).is_some_and(|e| self.games.contains(&e.as_str()))
    }
}

/// A game's title from its file name: the name without its extension and its tags in brackets,
/// "Super Mario Bros. (World) [!].nes" is "Super Mario Bros.". A name that is only tags keeps them.
pub fn rom_title(name: &str) -> String {
    let stem = name.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(name);
    let cut = stem.find(['(', '[']).map_or(stem, |at| &stem[..at]).trim();
    if cut.is_empty() { stem.trim().to_owned() } else { cut.to_owned() }
}

/// The extensions of a Switch game file: a package (`.nsp`), a cartridge image (`.xci`),
/// their compressed forms (`.nsz`, `.xcz`), and a Switch 2 package (`.nup`).
pub const SWITCH_FILES: [&str; 5] = ["nsp", "nsz", "xci", "xcz", "nup"];

/// Whether `name` is a Switch game file of any kind: a game, an update or DLC.
pub fn is_switch_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| SWITCH_FILES.iter().any(|s| e.eq_ignore_ascii_case(s)))
}

/// The Switch game in the file `name`, from the name alone: its files are encrypted, and their
/// keys are the player's. A name like `Title [0100ABCDEF012000][v131072].nsp` gives the title,
/// the title id and the version. `None` for another kind of file, and for an update or DLC: a
/// base game's title id ends in `000`, a tag that says "DLC" is never the game (a standalone app
/// such as an art book has a game's title id), and a name without a title id says "update",
/// "upd" or "dlc".
pub fn switch_file(name: &str) -> Option<(String, String, String)> {
    if !is_switch_file(name) {
        return None;
    }
    let (stem, _) = name.rsplit_once('.')?;
    let groups: Vec<&str> = stem.split('[').skip(1).filter_map(|g| g.split_once(']').map(|(g, _)| g.trim())).collect();
    let title_id = groups
        .iter()
        .find(|g| g.len() == 16 && g.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|g| g.to_ascii_uppercase())
        .unwrap_or_default();
    if !title_id.is_empty() && !title_id.ends_with("000") {
        return None;
    }
    let words = |text: &str| -> Vec<String> {
        text.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).map(str::to_owned).collect()
    };
    // A tag that says DLC: a release's standalone apps (an art book) have a game's title id.
    if groups.iter().any(|g| words(g).iter().any(|w| w == "dlc")) {
        return None;
    }
    if title_id.is_empty() && words(stem).iter().any(|w| matches!(w.as_str(), "update" | "upd" | "dlc")) {
        return None;
    }
    let version = groups
        .iter()
        .find(|g| g.len() > 1 && g[..1].eq_ignore_ascii_case("v") && g[1..].chars().all(|c| c.is_ascii_digit()))
        .map(|g| g.to_ascii_lowercase())
        .unwrap_or_default();
    let title = stem.split('[').next().unwrap_or(stem).trim();
    let title = if title.is_empty() { stem.trim() } else { title };
    Some((title.to_owned(), title_id, version))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundGame {
    /// The title id, with `-2`, `-3`… for more copies of the same game.
    pub id: String,
    pub title: String,
    pub title_id: String,
    pub content_id: String,
    pub version: String,
    /// The age the game is rated for, from `ageLevel.default` (PS5 only).
    pub age: Option<u8>,
    /// The game's folder: what its size counts and an uninstall deletes. For a Switch game, the
    /// folder that holds it alone with its update and DLC (an install's `Switch/<title>`), else
    /// its file.
    pub dir: HostPath,
    /// The file a Switch game starts. `None` for a game that is a folder.
    pub file: Option<HostPath>,
    pub icon: Option<HostPath>,
    pub background: Option<HostPath>,
    /// Pictures on the web when the game's files have none: a retro game's box art and a
    /// screenshot from libretro-thumbnails ([`RomSystem::thumbnails`]).
    pub web_cover: Option<String>,
    pub web_background: Option<String>,
}

/// The most a metadata file may be.
const MAX_PARAM: u64 = 1024 * 1024;

async fn read(folders: &dyn Folders, path: &HostPath) -> Option<Vec<u8>> {
    let bytes = folders.read_range(path, 0, MAX_PARAM + 1).await.ok()?;
    (bytes.len() as u64 <= MAX_PARAM).then_some(bytes)
}

async fn file_in(folders: &dyn Folders, dir: &HostPath, names: &[&str]) -> Option<HostPath> {
    for name in names {
        let path = dir.join(name).ok()?;
        if folders.exists(&path).await.unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

fn folder_name(dir: &HostPath) -> String {
    dir.path.file_name().unwrap_or("game").to_owned()
}

/// A PS5 game's title id and name, from its `param.json`. The title id must be letters and
/// digits, since it names the game's folder.
pub fn ps5_param(bytes: &[u8]) -> Option<(String, String)> {
    let p: Value = serde_json::from_slice(bytes).ok()?;
    let title_id = p["titleId"].as_str().filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric()))?;
    let localized = &p["localizedParameters"];
    let language = localized["defaultLanguage"].as_str().unwrap_or("en-US");
    let title =
        localized[language]["titleName"].as_str().or(localized["en-US"]["titleName"].as_str()).unwrap_or(title_id);
    Some((title_id.to_owned(), title.to_owned()))
}

/// The game in `dir`, if it holds one for `console`.
pub async fn read_game(folders: &dyn Folders, console: Console, dir: &HostPath) -> Option<FoundGame> {
    let sce = dir.join("sce_sys").ok()?;
    match console {
        // A Switch or a retro game is a file: see `scan`.
        Console::Switch | Console::Rom(_) => None,
        Console::Ps5 => {
            let p: Value = serde_json::from_slice(&read(folders, &sce.join("param.json").ok()?).await?).ok()?;
            let localized = &p["localizedParameters"];
            let language = localized["defaultLanguage"].as_str().unwrap_or("en-US");
            let title_id = p["titleId"].as_str().unwrap_or_default().to_owned();
            let title = localized[language]["titleName"]
                .as_str()
                .or(localized["en-US"]["titleName"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| if title_id.is_empty() { folder_name(dir) } else { title_id.clone() });
            Some(FoundGame {
                id: String::new(),
                title,
                content_id: p["contentId"].as_str().unwrap_or_default().to_owned(),
                version: p["contentVersion"].as_str().unwrap_or_default().to_owned(),
                age: p["ageLevel"]["default"].as_u64().and_then(|a| u8::try_from(a).ok()).filter(|a| *a > 0),
                title_id,
                dir: dir.clone(),
                file: None,
                icon: file_in(folders, &sce, &["icon0.png"]).await,
                background: file_in(folders, &sce, &["pic0.png"]).await,
                web_cover: None,
                web_background: None,
            })
        }
        Console::Ps4 => {
            let p = sfo::parse(&read(folders, &sce.join("param.sfo").ok()?).await?)?;
            let get = |k: &str| p.get(k).map(|v| v.text().to_owned()).unwrap_or_default();
            // Only full games: updates ("gp") and add-ons have a param.sfo too.
            if !get("CATEGORY").starts_with("gd") {
                return None;
            }
            let title_id = get("TITLE_ID");
            let title = Some(get("TITLE"))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| if title_id.is_empty() { folder_name(dir) } else { title_id.clone() });
            Some(FoundGame {
                id: String::new(),
                title,
                content_id: get("CONTENT_ID"),
                version: get("APP_VER"),
                age: None,
                title_id,
                dir: dir.clone(),
                file: None,
                icon: file_in(folders, &sce, &["icon0.png"]).await,
                background: file_in(folders, &sce, &["pic1.png", "pic0.png"]).await,
                web_cover: None,
                web_background: None,
            })
        }
    }
}

/// How far below a game folder a game is looked for: a console folder (`PS5`), a folder of
/// the player's, then the game.
const SCAN_DEPTH: usize = 3;

/// The retro games among the files of `dir`, at `depth` below the console's folder: each game
/// file, or only the playlists when the folder has one (they name the folder's discs). A folder
/// below the console's that holds one game and no folder but its art (an install's
/// `NES/<title>`) is that game's folder, with its other files (a disc's tracks): an uninstall
/// deletes it. Else each game is its own file.
fn rom_games(system: &RomSystem, dir: &HostPath, depth: usize, entries: &[DirEntry]) -> PluginResult<Vec<FoundGame>> {
    let files: Vec<DirEntry> = entries.iter().filter(|e| !e.is_dir).cloned().collect();
    let files = files.as_slice();
    let folders = entries.iter().any(|e| e.is_dir && e.name != launcher_api::ART_FOLDER);
    let listed = |e: &&DirEntry| match system.playlist {
        Some(playlist) if files.iter().any(|f| extension(&f.name).as_deref() == Some(playlist)) => {
            extension(&e.name).as_deref() == Some(playlist)
        }
        _ => system.is_game(&e.name),
    };
    let games: Vec<&DirEntry> = files.iter().filter(listed).collect();
    let own_folder = depth >= 1 && games.len() == 1 && !folders;
    let mut out = Vec::new();
    for entry in games {
        // A name that can't be a path is no game this plugin can name.
        let Ok(file) = dir.join(&entry.name) else { continue };
        out.push(FoundGame {
            id: String::new(),
            title: rom_title(&entry.name),
            title_id: String::new(),
            content_id: String::new(),
            version: String::new(),
            age: None,
            dir: if own_folder { dir.clone() } else { file.clone() },
            file: Some(file),
            icon: None,
            background: None,
            web_cover: thumbnail(system, "Named_Boxarts", &entry.name),
            web_background: thumbnail(system, "Named_Snaps", &entry.name),
        });
    }
    Ok(out)
}

/// The Switch games among the files of `dir`, at `depth` below the root. When `dir` is at least
/// two folders down (`Switch/<title>`) and holds one game, it is that game's folder; else each
/// game is its own file.
fn switch_games(dir: &HostPath, depth: usize, files: &[DirEntry]) -> PluginResult<Vec<FoundGame>> {
    let games: Vec<(&DirEntry, (String, String, String))> =
        files.iter().filter_map(|f| switch_file(&f.name).map(|g| (f, g))).collect();
    let own_folder = depth >= 2 && games.len() == 1;
    let mut out = Vec::new();
    for (entry, (title, title_id, version)) in games {
        // A name that can't be a path is no game this plugin can name.
        let Ok(file) = dir.join(&entry.name) else { continue };
        out.push(FoundGame {
            id: String::new(),
            title,
            title_id,
            content_id: String::new(),
            version,
            age: None,
            dir: if own_folder { dir.clone() } else { file.clone() },
            file: Some(file),
            icon: None,
            background: None,
            web_cover: None,
            web_background: None,
        });
    }
    Ok(out)
}

/// Every game for `console` in the folder named `folder`, sorted by title. A game folder may
/// hold games of other consoles too: each console's games are told apart by their files. An
/// install's work folder (`<name>.part`) is never read, nor an entry whose name can't be a path
/// (a `:` in it): the rest of the folder is read as usual.
pub async fn scan(folders: &dyn Folders, folder: &str, console: Console) -> PluginResult<Vec<FoundGame>> {
    let mut found: Vec<FoundGame> = Vec::new();
    for root in folders.roots(folder).await? {
        let top = match console {
            Console::Rom(system) => RelPath::new(system.folder)?,
            _ => RelPath::root(),
        };
        let mut level = vec![HostPath::new(folder, root.root, top)];
        for depth in 0..=SCAN_DEPTH {
            let mut next = Vec::new();
            for dir in level {
                if let Some(game) = read_game(folders, console, &dir).await {
                    found.push(game);
                    continue;
                }
                // A Switch or a retro game is a file, so the deepest folders are listed too.
                let files_are_games = matches!(console, Console::Switch | Console::Rom(_));
                if depth == SCAN_DEPTH && !files_are_games {
                    continue;
                }
                // A root that can't be listed (not there yet) holds no games.
                let entries = folders.list(&dir).await.unwrap_or_default();
                let entries: Vec<DirEntry> = entries.into_iter().filter(|e| !e.name.starts_with('.')).collect();
                if depth < SCAN_DEPTH {
                    for child in entries.iter().filter(|e| e.is_dir && !e.name.ends_with(".part")) {
                        if let Ok(child) = dir.join(&child.name) {
                            next.push(child);
                        }
                    }
                }
                let files = || entries.iter().filter(|e| !e.is_dir).cloned().collect::<Vec<DirEntry>>();
                match console {
                    Console::Switch => found.extend(switch_games(&dir, depth, &files())?),
                    Console::Rom(system) => found.extend(rom_games(system, &dir, depth, &entries)?),
                    Console::Ps5 | Console::Ps4 => {}
                }
            }
            level = next;
        }
    }
    found.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.dir.cmp(&b.dir)));
    let mut taken = std::collections::HashSet::new();
    for game in &mut found {
        let base = if game.title_id.is_empty() { game.title.clone() } else { game.title_id.clone() };
        let mut id = base.clone();
        let mut n = 1;
        while !taken.insert(id.clone()) {
            n += 1;
            id = format!("{base}-{n}");
        }
        game.id = id;
    }
    Ok(found)
}

/// The games last found, by id. A miss scans again: the host may ask about a game before it
/// lists them (after a restart, for example).
pub struct GameIndex {
    pub folder: &'static str,
    pub console: Console,
    games: Mutex<Vec<FoundGame>>,
    /// Each game's size on disk, by folder, worked out once.
    sizes: Mutex<std::collections::HashMap<HostPath, u64>>,
}

impl GameIndex {
    pub fn new(folder: &'static str, console: Console) -> Self {
        Self { folder, console, games: Mutex::new(Vec::new()), sizes: Mutex::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<FoundGame>> {
        self.games.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Scans again and returns every game.
    pub async fn refresh(&self, folders: &dyn Folders) -> PluginResult<Vec<FoundGame>> {
        let games = scan(folders, self.folder, self.console).await?;
        *self.lock() = games.clone();
        Ok(games)
    }

    pub async fn find(&self, folders: &dyn Folders, id: &str) -> PluginResult<FoundGame> {
        if let Some(game) = self.lock().iter().find(|g| g.id == id).cloned() {
            return Ok(game);
        }
        self.refresh(folders).await?.into_iter().find(|g| g.id == id).ok_or_else(|| PluginError::NotFound(id.into()))
    }

    /// The game's details, with its size on disk.
    pub async fn details(
        &self,
        folders: &dyn Folders,
        id: &str,
        platform: &str,
    ) -> PluginResult<launcher_api::GameDetails> {
        let game = self.find(folders, id).await?;
        let cached = self.sizes.lock().unwrap_or_else(|p| p.into_inner()).get(&game.dir).copied();
        let size = match cached {
            Some(size) => size,
            None => {
                let size = folders.size(&game.dir).await.unwrap_or(0);
                self.sizes.lock().unwrap_or_else(|p| p.into_inner()).insert(game.dir.clone(), size);
                size
            }
        };
        let mut details = game.details(platform);
        details.size_bytes = (size > 0).then_some(size);
        Ok(details)
    }
}

/// The region a content id was published for, from its first two letters.
pub fn region(content_id: &str) -> Option<&'static str> {
    Some(match content_id.get(..2)? {
        "UP" => "North America",
        "EP" => "Europe",
        "JP" => "Japan",
        "HP" => "Asia",
        "KP" => "Korea",
        _ => return None,
    })
}

impl FoundGame {
    /// The catalog card, with the game's own icon as its cover.
    pub fn summary(&self, platform: &str) -> launcher_api::GameSummary {
        launcher_api::GameSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            platform: platform.to_owned(),
            cover: self
                .icon
                .clone()
                .map(launcher_api::ImageSource::Folder)
                .or_else(|| self.web_cover.clone().map(launcher_api::ImageSource::Url)),
            tags: Vec::new(),
            release_year: None,
            title_id: (!self.title_id.is_empty()).then(|| self.title_id.clone()),
            content_id: (!self.content_id.is_empty()).then(|| self.content_id.clone()),
        }
    }

    /// The details the game's files hold. Its size is [`GameIndex::details`]'s.
    pub fn details(&self, platform: &str) -> launcher_api::GameDetails {
        use launcher_api::GameFact;
        let mut facts = Vec::new();
        if let Some(region) = region(&self.content_id) {
            facts.push(GameFact::new("region", "Region", region));
        }
        if let Some(age) = self.age {
            facts.push(GameFact::new("age", "Age rating", format!("{age}+")));
        }
        if !self.version.is_empty() {
            facts.push(GameFact::new("version", "Version", &self.version));
        }
        if !self.title_id.is_empty() {
            facts.push(GameFact::new("title-id", "Title ID", &self.title_id));
        }
        if !self.content_id.is_empty() {
            facts.push(GameFact::new("content-id", "Content ID", &self.content_id));
        }
        if let Some(file) = &self.file {
            facts.push(GameFact::new("file", "File", file.path.as_str()));
        }
        if self.file.as_ref() != Some(&self.dir) {
            facts.push(GameFact::new("folder", "Folder", self.dir.path.as_str()));
        }
        launcher_api::GameDetails {
            folder: Some(self.dir.clone()),
            also: Vec::new(),
            uninstall_link: None,
            summary: self.summary(platform),
            // A game's files have no description.
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: self
                .background
                .clone()
                .map(launcher_api::ImageSource::Folder)
                .or_else(|| self.web_background.clone().map(launcher_api::ImageSource::Url)),
            facts,
        }
    }
}

/// Games whose title or title id contains `query`, title matches first, at most `limit`.
pub fn search(games: Vec<FoundGame>, query: &launcher_api::SearchQuery) -> Vec<FoundGame> {
    let needle = query.text.to_lowercase();
    let mut hits: Vec<FoundGame> = games
        .into_iter()
        .filter(|g| g.title.to_lowercase().contains(&needle) || g.title_id.to_lowercase().contains(&needle))
        .collect();
    hits.sort_by_key(|g| !g.title.to_lowercase().starts_with(&needle));
    hits.truncate(query.limit as usize);
    hits
}

/// A badge for a result on `platform`: "In-game on macOS", or "In-game on Linux · untested on
/// macOS" when the result comes from another system. Its short form, for cards: "In-game", or
/// "Linux · In-game".
pub fn compat_badge(
    info: &launcher_api::CompatInfo,
    platform: Option<launcher_api::Platform>,
) -> launcher_api::SlotContent {
    let (level, tone) = level_badge(info.level);
    let here = crate::compat::os_name(platform);
    let (label, short) = match info.source.as_deref() {
        _ if info.level == launcher_api::CompatLevel::Unknown => (level.to_owned(), level.to_owned()),
        Some(from) if from == here => (format!("{level} on {here}"), level.to_owned()),
        Some(from) => {
            let os = match from {
                "Windows" => "Win",
                "macOS" => "Mac",
                other => other,
            };
            (format!("{level} on {from} · untested on {here}"), format!("{os} · {level}"))
        }
        None => (level.to_owned(), level.to_owned()),
    };
    launcher_api::SlotContent::Badge { label, short, tone, icon: None }
}

/// A badge for a compatibility level.
fn level_badge(level: launcher_api::CompatLevel) -> (&'static str, launcher_api::Tone) {
    use launcher_api::{CompatLevel, Tone};
    match level {
        CompatLevel::Playable => ("Playable", Tone::Success),
        CompatLevel::Ingame => ("In-game", Tone::Success),
        CompatLevel::Menus => ("Menus", Tone::Warning),
        CompatLevel::Boots => ("Boots", Tone::Warning),
        CompatLevel::Nothing => ("Doesn't start", Tone::Danger),
        CompatLevel::Unknown => ("Not tested", Tone::Neutral),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS1: RomSystem = RomSystem {
        folder: "PS1",
        games: &["cue", "chd"],
        files: &["bin"],
        playlist: Some("m3u"),
        thumbnails: Some("Sony_-_PlayStation"),
    };

    #[test]
    fn a_rom_s_pictures_are_named_after_its_file_as_libretro_names_them() {
        assert_eq!(
            thumbnail(&PS1, "Named_Boxarts", "Crash Bandicoot (USA).cue").as_deref(),
            Some(
                "https://raw.githubusercontent.com/libretro-thumbnails/Sony_-_PlayStation/master/Named_Boxarts/Crash%20Bandicoot%20%28USA%29.png"
            )
        );
        let odd = thumbnail(&PS1, "Named_Snaps", "Tom & Jerry: Fists.cue").unwrap();
        assert!(odd.ends_with("/Named_Snaps/Tom%20_%20Jerry_%20Fists.png"), "{odd}");
        assert_eq!(thumbnail(&RomSystem { thumbnails: None, ..PS1 }, "Named_Boxarts", "a.cue"), None);
    }

    #[test]
    fn a_rom_s_title_is_its_name_without_tags() {
        assert_eq!(rom_title("Super Mario Bros. (World) [!].nes"), "Super Mario Bros.");
        assert_eq!(rom_title("Sonic the Hedgehog.md"), "Sonic the Hedgehog");
        assert_eq!(rom_title("(Prototype).nes"), "(Prototype)", "a name of tags alone keeps them");
        assert_eq!(rom_title("noext"), "noext");
    }

    #[test]
    fn a_game_file_is_one_whose_extension_says_a_game_and_a_part_is_copied_too() {
        assert!(PS1.is_game("Crash (USA).CUE") && !PS1.is_game("Crash (USA) (Track 1).bin"));
        assert!(PS1.is_file("Crash (USA) (Track 1).bin") && PS1.is_file("Crash.chd"));
        assert!(!PS1.is_file("readme.txt") && !PS1.is_file(".bin"));
    }

    #[test]
    fn a_folder_with_a_playlist_lists_the_playlist_not_its_discs() {
        let dir = HostPath::new("games", 0, RelPath::new("PS1/FF7").unwrap());
        let file = |name: &str| DirEntry { name: name.into(), is_dir: false, size: 1 };
        let discs = [file("FF7 (Disc 1).cue"), file("FF7 (Disc 1).bin"), file("FF7 (Disc 2).cue")];
        let titles = |files: &[DirEntry]| -> Vec<String> {
            rom_games(&PS1, &dir, 1, files).unwrap().into_iter().map(|g| g.title).collect()
        };
        assert_eq!(titles(&discs), ["FF7", "FF7"], "each disc, without a playlist");
        let with_list = [discs.as_slice(), &[file("FF7.m3u")]].concat();
        let games = rom_games(&PS1, &dir, 1, &with_list).unwrap();
        assert_eq!(games.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["FF7"]);
        assert_eq!(games[0].file.as_ref().unwrap().path.as_str(), "PS1/FF7/FF7.m3u", "the playlist starts it");
        assert_eq!(games[0].dir, dir, "one game below the console's folder: the folder is the game's");
        let loose = HostPath::new("games", 0, RelPath::new("PS1").unwrap());
        let at_top = rom_games(&PS1, &loose, 0, &[file("Racer.cue"), file("Racer.bin")]).unwrap();
        assert_eq!(at_top[0].dir.path.as_str(), "PS1/Racer.cue", "in the console's folder itself, its file");
        let folder = |name: &str| DirEntry { name: name.into(), is_dir: true, size: 0 };
        let with_art = rom_games(&PS1, &dir, 1, &[file("FF7.cue"), folder("launcher_art")]).unwrap();
        assert_eq!(with_art[0].dir, dir, "its art's folder is its own");
        let with_more = rom_games(&PS1, &dir, 1, &[file("FF7.cue"), folder("Hacks")]).unwrap();
        assert_eq!(with_more[0].dir.path.as_str(), "PS1/FF7/FF7.cue", "a folder of other games isn't deleted with it");
    }

    #[test]
    fn a_switch_file_s_name_gives_its_title_id_and_version_and_updates_are_not_games() {
        let game = switch_file;
        let owned = |t: &str, id: &str, v: &str| Some((t.to_owned(), id.to_owned(), v.to_owned()));
        assert_eq!(game("Celeste [01002b30028f6000][v0].nsp"), owned("Celeste", "01002B30028F6000", "v0"));
        assert_eq!(game("Kirby Air Riders [NUP].NUP"), owned("Kirby Air Riders", "", ""));
        assert_eq!(game("Detention.xcz"), owned("Detention", "", ""));
        assert_eq!(
            game("Dark Souls II (2) [0100ABCDEF012000][US][v65536].xci"),
            owned("Dark Souls II (2)", "0100ABCDEF012000", "v65536")
        );
        assert_eq!(game("Celeste [01002B30028F6800][v262144].nsp"), None, "an update");
        assert_eq!(game("Celeste DLC [01002B30028F7001][v0].nsp"), None, "an add-on");
        let art_book =
            "Sonic x Shadow Generation [Standalone DLC Digital Art Book] [010021301E9A4000][v0] (0.48 GB).nsz";
        assert_eq!(game(art_book), None, "a standalone app the release calls DLC");
        assert_eq!(game("Kirby Air Riders Update 1.1.nsp"), None, "an update by its name");
        assert_eq!(game("readme.txt"), None);
        assert_eq!(game("nsp"), None);
    }
}

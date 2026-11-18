//! Which of the player's games a Catalog offer is, and the offer's release that counts: pure
//! reads over the library, the downloads and the game installs.
//!
//! An offer and an installed game are two records with two keys: the catalog plugin's and the
//! emulator's. [`Ownership`] links them, by the download that installed the game, by title id,
//! by name on the same console, or by a game's whole name that starts the offer's: a game
//! named from its file, "Mario Strikers Battle League", is the offer "Mario Strikers: Battle
//! League Football". The view asks it about every offer, so it builds its maps once.
//!
//! An offer the player has answers as the installed game it is. [`Ownership::game`] gives a
//! game key [`AsInstalled`], and the view, the grids and the versions read the player's
//! result, the community result and the game Play starts through it.

use std::collections::{BTreeMap, HashMap};

use launcher_api::{CompatInfo, CompatLevel, GameKey, PluginId, Release};

use crate::game_installs::{GamePhase, InstallsSnapshot};
use crate::library::{Entry, InstalledOffer, LibrarySnapshot};
use crate::releases::Offer;
use crate::torrents::TorrentJob;

/// The emulators' installed games by title id and by name, with the downloads and installs
/// that say which game a download became.
pub(crate) struct Ownership<'a> {
    library: &'a LibrarySnapshot,
    /// The downloads, whose installs say which game an offer is.
    jobs: &'a BTreeMap<String, TorrentJob>,
    /// The game installs, which say which game a download installed.
    installs: &'a InstallsSnapshot,
    /// The installed games.
    games: Vec<&'a Entry>,
    /// The Catalog's offers by key.
    offers: HashMap<&'a GameKey, &'a Offer>,
    by_id: HashMap<(&'a PluginId, &'a str), &'a Entry>,
    /// By the emulator, and the console and the title as letters and digits (`ps5/astrobot`).
    by_name: HashMap<(&'a PluginId, String), &'a Entry>,
    /// The games of [`MIN_PREFIX_WORDS`] words or more, with their console and their title's
    /// words, for a name that starts an offer's.
    by_words: Vec<(&'a Entry, String, Vec<String>)>,
    /// The games of [`MIN_PREFIX_WORDS`] words or more by the emulator, the console and their
    /// [`loose`] name, for a name that differs from an offer's by a plural or a bracket.
    by_loose: HashMap<(&'a PluginId, String, String), Vec<&'a Entry>>,
}

/// The fewest words a game's name has for it to link to an offer whose name it starts: one
/// word ("Doom") starts too many names.
const MIN_PREFIX_WORDS: usize = 2;

impl<'a> Ownership<'a> {
    /// Over the Library's games.
    pub fn new(
        library: &'a LibrarySnapshot,
        jobs: &'a BTreeMap<String, TorrentJob>,
        installs: &'a InstallsSnapshot,
    ) -> Self {
        Self::over(library.library(), library, jobs, installs)
    }

    /// Over `games`, a list the library doesn't hold yet: a library load's.
    pub fn over(
        games: impl IntoIterator<Item = &'a Entry>,
        library: &'a LibrarySnapshot,
        jobs: &'a BTreeMap<String, TorrentJob>,
        installs: &'a InstallsSnapshot,
    ) -> Self {
        let games: Vec<&Entry> = games.into_iter().collect();
        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();
        let mut by_words = Vec::new();
        let mut by_loose: HashMap<_, Vec<&Entry>> = HashMap::new();
        let offer_names: std::collections::HashSet<String> = library
            .offers
            .iter()
            .flat_map(|o| o.releases.iter().filter_map(|r| name(&o.entry.summary.platform, &r.title)))
            .collect();
        // A game named after its dashed folder: the offer name its folder gives, first, so the
        // title it lost a number from doesn't take another game's offer.
        let mut named_by_folder = std::collections::HashSet::new();
        for e in &games {
            if let Some(name) = folder_name(&e.key.local, &e.summary.platform, &offer_names) {
                by_name.entry((&e.key.plugin, name)).or_insert(*e);
                named_by_folder.insert(&e.key);
            }
        }
        for e in &games {
            let title = words(&e.summary.title);
            if title.len() >= MIN_PREFIX_WORDS {
                by_words.push((*e, e.summary.platform.to_ascii_lowercase(), title));
                let key = (&e.key.plugin, e.summary.platform.to_ascii_lowercase(), loose(&e.summary.title));
                by_loose.entry(key).or_default().push(*e);
            }
            if let Some(id) = e.summary.title_id.as_deref().filter(|t| !t.is_empty()) {
                by_id.entry((&e.key.plugin, id)).or_insert(*e);
            }
            if !named_by_folder.contains(&e.key)
                && let Some(name) = name(&e.summary.platform, &e.summary.title)
            {
                by_name.entry((&e.key.plugin, name)).or_insert(*e);
            }
        }
        let offers = library.offers.iter().map(|o| (&o.entry.key, o)).collect();
        Self { library, jobs, installs, games, offers, by_id, by_name, by_words, by_loose }
    }

    /// The game `key` as the player has it: a Library game is itself; a Catalog offer is the
    /// installed game it is, or none.
    pub fn game<'k>(&'k self, key: &'k GameKey) -> AsInstalled<'k> {
        let offer = self.offers.get(key).copied();
        let installed = match offer {
            Some(offer) => self.installed_game(offer).map(|g| &g.key),
            None => Some(key),
        };
        AsInstalled { library: self.library, key, offer, installed }
    }

    /// The game of the emulator's library that the download `job` installed: the one with the
    /// title its install reported. A PC game has no title id, so this is how its download
    /// finds it.
    pub fn installed_from(&self, job: &TorrentJob) -> Option<&'a Entry> {
        let install = self.installs.latest(&job.emulator, &job.source)?;
        let title = &install.installed.as_ref().filter(|_| install.phase == GamePhase::Done)?.title;
        self.games.iter().copied().find(|e| e.key.plugin == job.emulator && &e.summary.title == title)
    }

    /// The installed game an offer is, from the emulator's library: the game a download of one
    /// of its releases installed, else the game with one of its title ids, else the game of its
    /// name on the same console. A PS4 and a PS5 game of one name are two games.
    pub fn installed_game(&self, offer: &Offer) -> Option<&'a Entry> {
        self.linked(offer).or_else(|| self.by_prefix(offer)).or_else(|| self.by_loose(offer))
    }

    /// The one installed game, of the offer's emulator and console, whose [`loose`] name is one
    /// of [`loose_names`] of the offer's releases. `None` when no game or more than one fits.
    fn by_loose(&self, offer: &Offer) -> Option<&'a Entry> {
        let console = offer.entry.summary.platform.to_ascii_lowercase();
        let mut found: Option<&'a Entry> = None;
        for release in &offer.releases {
            for name in loose_names(&release.title) {
                let Some(games) = self.by_loose.get(&(&offer.emulator, console.clone(), name)) else { continue };
                for game in games {
                    match found {
                        Some(other) if other.key != game.key => return None,
                        _ => found = Some(game),
                    }
                }
            }
        }
        found
    }

    /// The installed game the offer is, by its download, its title id or its name.
    fn linked(&self, offer: &Offer) -> Option<&'a Entry> {
        let downloaded = self.jobs.values().filter(|j| j.offer == offer.entry.key).find_map(|j| self.installed_from(j));
        let emulator = &offer.emulator;
        let by_id = || {
            let mut ids = offer.releases.iter().flat_map(|r| std::iter::once(&r.title_id).chain(&r.title_ids));
            ids.find_map(|t| self.by_id.get(&(emulator, t.as_str())).copied())
        };
        let by_name = || {
            let console = &offer.entry.summary.platform;
            let mut names = offer.releases.iter().filter_map(|r| name(console, &r.title));
            names.find_map(|name| self.by_name.get(&(emulator, name)).copied())
        };
        downloaded.or_else(by_id).or_else(by_name)
    }

    /// The one installed game, of the offer's emulator and console, whose whole name starts the
    /// name of one of the offer's releases, word by word. `None` when no game or more than one
    /// fits.
    fn by_prefix(&self, offer: &Offer) -> Option<&'a Entry> {
        let console = offer.entry.summary.platform.to_ascii_lowercase();
        let names: Vec<Vec<String>> = offer.releases.iter().map(|r| words(&r.title)).collect();
        let mut found: Option<&'a Entry> = None;
        for (game, on, title) in &self.by_words {
            if game.key.plugin != offer.emulator || *on != console {
                continue;
            }
            if names.iter().any(|name| name.len() > title.len() && name.starts_with(title)) {
                match found {
                    Some(other) if other.key != game.key => return None,
                    _ => found = Some(game),
                }
            }
        }
        found
    }

    /// The release an offer's card shows: the version the player has, then a release whose
    /// download installed the game, then one with a magnet link, then one with art, then the
    /// highest version, then the one most people share. Of equals, the newest upload.
    pub fn best_release<'o>(&self, offer: &'o Offer) -> &'o Release {
        use crate::releases::version_key;
        let has = self
            .installed_game(offer)
            .and_then(|e| self.library.detail(&e.key))
            .and_then(|d| d.details.facts.iter().find(|f| f.id == "version"))
            .map(|f| version_key(&f.value))
            .filter(|v| !v.is_empty());
        let rank = |r: &Release| {
            let downloaded = self.jobs.get(&r.id).and_then(|j| self.installed_from(j)).is_some();
            (
                has.as_ref().is_some_and(|v| *v == version_key(&r.version)),
                downloaded,
                !r.torrent.is_empty(),
                r.metadata.is_some() || !r.image_urls.is_empty(),
                version_key(&r.version),
                r.seeders.unwrap_or(0),
            )
        };
        // The releases are newest upload first, and `max_by_key` keeps the last of equals.
        offer.releases.iter().rev().max_by_key(|r| rank(r)).unwrap_or(&offer.releases[0])
    }

    /// The offer each installed game is, by the game, with the offer's cover.
    ///
    /// A game whose name starts the names of two offers or more is none of them: which one it is
    /// can't be told.
    pub fn installed_offers(&self, offers: &[Offer]) -> Vec<InstalledOffer> {
        let links: Vec<(&Offer, &Entry, bool)> = offers
            .iter()
            .filter_map(|offer| match self.linked(offer) {
                Some(game) => Some((offer, game, false)),
                None => self.by_prefix(offer).map(|game| (offer, game, true)),
            })
            .collect();
        let mut by_prefix: HashMap<&GameKey, usize> = HashMap::new();
        for (_, game, prefix) in &links {
            if *prefix {
                *by_prefix.entry(&game.key).or_default() += 1;
            }
        }
        links
            .into_iter()
            .filter(|(_, game, prefix)| !prefix || by_prefix.get(&game.key) == Some(&1))
            .map(|(offer, game, _)| {
                let cover = offer.entry.summary.cover.clone().map(|c| (offer.entry.key.plugin.clone(), c));
                (game.key.clone(), offer.entry.key.clone(), cover)
            })
            .collect()
    }
}

/// A game key as the installed game it is (see [`Ownership::game`]). What the player did with
/// the game is kept under the installed game's key: Play starts it, and its play time and its
/// result from playing are saved for it. So an offer reads them from there.
#[derive(Clone, Copy)]
pub(crate) struct AsInstalled<'a> {
    library: &'a LibrarySnapshot,
    key: &'a GameKey,
    offer: Option<&'a Offer>,
    installed: Option<&'a GameKey>,
}

impl<'a> AsInstalled<'a> {
    /// The Catalog offer the key is; `None` for a Library game.
    pub fn offer(&self) -> Option<&'a Offer> {
        self.offer
    }

    /// The installed game: a Library game itself, the one an offer is, or none for an offer the
    /// player doesn't have.
    pub fn installed(&self) -> Option<&'a GameKey> {
        self.installed
    }

    /// The game Play starts, whose play time counts and which runs: the installed game, else
    /// the key itself.
    pub fn played(&self) -> &'a GameKey {
        self.installed.unwrap_or(self.key)
    }

    /// The player's own result: the one for the key, else the one for the installed game.
    pub fn player_result(&self) -> Option<CompatLevel> {
        let result = |key: &GameKey| self.library.result(key);
        result(self.key).or_else(|| self.installed.and_then(result))
    }

    /// The community list's result: the key's own (a Library game's emulator's `status`), the
    /// installed game's, then the result the emulator's list gave for one of an offer's title
    /// ids ([`LibrarySnapshot::community`]). A result that is no level (`Unknown`, as `status`
    /// says for a game the list doesn't know) gives way to one with a level; with none, the
    /// first stays, for its list's page.
    pub fn community(&self) -> Option<&'a CompatInfo> {
        let library = self.library;
        let compat = |key: &GameKey| library.detail(key).and_then(|d| d.compat.as_ref());
        let installed = self.installed.filter(|g| *g != self.key).and_then(compat);
        let mut found = [compat(self.key), installed, library.community.get(self.key)].into_iter().flatten();
        let first = found.clone().next();
        found.find(|c| c.level != CompatLevel::Unknown).or(first)
    }

    /// How far the game gets, for the "In-game" filter and the sort: the player's own result,
    /// else the community's.
    pub fn reach(&self) -> CompatLevel {
        self.player_result().or_else(|| self.community().map(|c| c.level)).unwrap_or(CompatLevel::Unknown)
    }
}

/// A title's words, in lower case: letters and digits. An apostrophe joins its word
/// ("Assassin's" is "assassins").
fn words(title: &str) -> Vec<String> {
    let title = title.to_lowercase().replace(['\'', '\u{2019}'], "");
    title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// A word without a plural `s`: "generations" is "generation". Not "ss" ("boss"), nor a word of
/// three letters or fewer ("gas").
fn singular(word: &str) -> &str {
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") { &word[..word.len() - 1] } else { word }
}

/// A title as its words in the singular, for a match that a plural doesn't break.
fn loose(title: &str) -> String {
    words(title).iter().map(|w| singular(w)).collect::<Vec<_>>().join(" ")
}

/// The [`loose`] names an offer's release title stands for: the title without a "+ 5 DLC"
/// tail, and for a name with a bracket, as "Teenage Mutant Ninja Turtles (TMNT) Splintered
/// Fate", the name without the bracket and the bracket's short name with the rest ("TMNT
/// Splintered Fate").
fn loose_names(title: &str) -> Vec<String> {
    let title = title.split(" + ").next().unwrap_or(title);
    let mut names = vec![title.to_owned()];
    if let Some(open) = title.find('(')
        && let Some(close) = title[open..].find(')').map(|c| open + c)
    {
        let (before, inner, after) = (&title[..open], &title[open + 1..close], &title[close + 1..]);
        names.push(format!("{before} {after}"));
        names.push(format!("{inner} {after}"));
    }
    names.iter().map(|n| loose(n)).filter(|n| n.split(' ').count() >= MIN_PREFIX_WORDS).collect()
}

/// The offer name of a game named after its folder: a folder whose name has its words joined by
/// dashes and numbers at the end (`world-of-goo-2-1-0-13120-81588`) is the offer, of the names
/// in `offers`, of its words with the most of those numbers ("World of Goo 2").
fn folder_name(folder: &str, console: &str, offers: &std::collections::HashSet<String>) -> Option<String> {
    if folder.contains(' ') || folder.matches('-').count() < 2 {
        return None;
    }
    let parts: Vec<&str> = folder.split('-').filter(|p| !p.is_empty()).collect();
    let numbers = parts.iter().rev().take_while(|p| p.chars().all(|c| c.is_ascii_digit())).count();
    if numbers == 0 || numbers == parts.len() {
        return None;
    }
    (0..=numbers)
        .rev()
        .filter_map(|kept| name(console, &parts[..parts.len() - numbers + kept].join(" ")))
        .find(|name| offers.contains(name))
}

/// A game's name to find it by: its console and its title, as letters and digits.
fn name(console: &str, title: &str) -> Option<String> {
    let title = crate::releases::plain(title);
    (!title.is_empty()).then(|| format!("{}/{title}", console.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use launcher_api::{GameDetails, GameFact, GameSummary};

    use super::*;
    use crate::changes::{Changes, Notifier};
    use crate::library::{Detail, LibrarySubsystem, Records};

    fn key(text: &str) -> GameKey {
        text.parse().unwrap()
    }

    fn summary(id: &str, title: &str, platform: &str, title_id: Option<&str>) -> GameSummary {
        GameSummary {
            id: id.into(),
            title: title.into(),
            platform: platform.into(),
            cover: None,
            tags: Vec::new(),
            release_year: None,
            title_id: title_id.map(str::to_owned),
            content_id: None,
        }
    }

    fn game(key_text: &str, title: &str, platform: &str, title_id: Option<&str>) -> Entry {
        let key = key(key_text);
        Entry { summary: summary(&key.local, title, platform, title_id), key }
    }

    fn release(id: &str, title: &str, title_id: &str, version: &str) -> Release {
        Release {
            id: id.into(),
            title: title.into(),
            title_id: title_id.into(),
            version: version.into(),
            platform: "PS5".into(),
            ..Release::default()
        }
    }

    fn offer(key_text: &str, platform: &str, releases: Vec<Release>) -> Offer {
        let key = key(key_text);
        let entry = Entry { summary: summary(&key.local, &releases[0].title, platform, None), key };
        Offer { entry, emulator: PluginId::new("emu").unwrap(), releases }
    }

    /// A library of `games`, with these details, and these offers.
    fn library(games: Vec<Entry>, details: Vec<(GameKey, Detail)>, offers: Vec<Offer>) -> LibrarySnapshot {
        let config = Arc::new(crate::ConfigService::in_memory());
        let notifier = Notifier::new(Changes::new(), crate::EventBus::default());
        let library = LibrarySubsystem::new(Records::new(config), notifier);
        library.set_library(games, HashMap::new(), Vec::new());
        for (key, detail) in details {
            library.set_detail(&key, detail, None);
        }
        let mut snapshot = library.snapshot();
        snapshot.offers = Arc::new(offers);
        snapshot
    }

    fn owned<T>(library: &LibrarySnapshot, read: impl FnOnce(&Ownership) -> T) -> T {
        let (jobs, installs) = (BTreeMap::new(), InstallsSnapshot::default());
        read(&Ownership::new(library, &jobs, &installs))
    }

    /// The offer each installed game links to, by key.
    fn links(library: &LibrarySnapshot) -> Vec<(String, Option<String>)> {
        owned(library, |o| {
            library
                .offers
                .iter()
                .map(|offer| (offer.entry.key.to_string(), o.installed_game(offer).map(|e| e.key.to_string())))
                .collect()
        })
    }

    /// A name that differs from the offer's by a plural, by a "+ 5 DLC" tail, or by the long
    /// form of a name the offer gives in brackets still links: "TMNT Splintered Fate" is
    /// "Teenage Mutant Ninja Turtles (TMNT) Splintered Fate + 5 DLC".
    #[test]
    fn a_name_links_across_a_plural_a_dlc_tail_and_a_bracketed_short_name() {
        let sonic = game("emu:sonic", "Sonic x Shadow Generation", "SWITCH", None);
        let tmnt = game("emu:tmnt", "TMNT Splintered Fate", "SWITCH", None);
        let offers = vec![
            offer("cat:o1", "SWITCH", vec![release("a", "SONIC X SHADOW GENERATIONS", "", "1.0")]),
            offer(
                "cat:o2",
                "SWITCH",
                vec![release("b", "Teenage Mutant Ninja Turtles (TMNT) Splintered Fate + 5 DLC", "", "1.0")],
            ),
            offer("cat:o3", "SWITCH", vec![release("c", "Shadow Fate", "", "1.0")]),
        ];
        let library = library(vec![sonic, tmnt], Vec::new(), offers);
        assert_eq!(
            links(&library),
            [
                ("cat:o1".to_owned(), Some("emu:sonic".to_owned())),
                ("cat:o2".to_owned(), Some("emu:tmnt".to_owned())),
                ("cat:o3".to_owned(), None),
            ]
        );
    }

    /// A PC game named after its dashed folder lost the numbers at the end of the folder's name:
    /// the offer whose name is the folder's words with the most of those numbers links to it.
    #[test]
    fn a_dashed_folder_links_to_the_offer_with_the_most_of_its_numbers() {
        let one = game("emu:world-of-goo-1-6-538-87464", "World Of Goo", "PC", None);
        let two = game("emu:world-of-goo-2-1-0-13120-81588", "World Of Goo", "PC", None);
        let offers = vec![
            offer("cat:goo", "PC", vec![release("a", "World of Goo", "", "1.6")]),
            offer("cat:goo2", "PC", vec![release("b", "World of Goo 2", "", "2.1")]),
        ];
        let library = library(vec![one, two], Vec::new(), offers);
        assert_eq!(
            links(&library),
            [
                ("cat:goo".to_owned(), Some("emu:world-of-goo-1-6-538-87464".to_owned())),
                ("cat:goo2".to_owned(), Some("emu:world-of-goo-2-1-0-13120-81588".to_owned())),
            ]
        );
    }

    #[test]
    fn an_offer_is_the_installed_game_with_one_of_its_title_ids() {
        let astro = game("emu:G1", "Astro Bot (Japan)", "PS5", Some("PPSA01234"));
        let mut other_region = release("a", "Astro", "PPSA00001", "1.0");
        other_region.title_ids = vec!["PPSA01234".into()];
        let library = library(vec![astro], Vec::new(), vec![offer("cat:o1", "PS5", vec![other_region])]);
        let found = owned(&library, |o| o.installed_game(&library.offers[0]).map(|e| e.key.clone()));
        assert_eq!(found, Some(key("emu:G1")), "a title id the release also carries");
    }

    #[test]
    fn an_offer_is_the_installed_game_of_its_name_on_the_same_console_only() {
        let library = library(
            vec![game("emu:G1", "Astro's Playroom", "PS5", None)],
            Vec::new(),
            vec![
                offer("cat:ps5", "PS5", vec![release("a", "ASTRO'S PLAYROOM", "", "1.0")]),
                offer("cat:ps4", "PS4", vec![release("b", "Astro's Playroom", "", "1.0")]),
            ],
        );
        owned(&library, |o| {
            assert_eq!(o.installed_game(&library.offers[0]).map(|e| e.key.clone()), Some(key("emu:G1")));
            assert!(o.installed_game(&library.offers[1]).is_none(), "a PS4 game of one name is another game");
        });
    }

    #[test]
    fn an_offer_is_the_installed_game_whose_whole_name_starts_its_name() {
        let library = library(
            vec![game("emu:G1", "Mario Strikers Battle League", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Mario Strikers: Battle League Football", "", "1.3.2")])],
        );
        let found = owned(&library, |o| o.installed_game(&library.offers[0]).map(|e| e.key.clone()));
        assert_eq!(found, Some(key("emu:G1")), "the EU name says more than the file's");
        let linked = owned(&library, |o| o.installed_offers(&library.offers));
        assert_eq!(
            linked.iter().map(|(g, o, _)| (g.clone(), o.clone())).collect::<Vec<_>>(),
            [(key("emu:G1"), key("cat:o1"))]
        );
    }

    #[test]
    fn a_name_that_starts_another_links_only_when_it_is_sure() {
        // One word is too little: "Doom" starts many names.
        let one_word = library(
            vec![game("emu:G1", "Doom", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Doom Eternal", "", "1.0")])],
        );
        assert!(owned(&one_word, |o| o.installed_game(&one_word.offers[0]).is_none()), "one word");

        // Two games whose names both start the offer's: neither is it.
        let two_games = library(
            vec![game("emu:G1", "Mario Kart", "Switch", None), game("emu:G2", "Mario Kart 8", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Mario Kart 8 Deluxe", "", "1.0")])],
        );
        assert!(owned(&two_games, |o| o.installed_game(&two_games.offers[0]).is_none()), "two games fit");

        // A game whose name starts two offers' names is neither, for art and game files.
        let two_offers = library(
            vec![game("emu:G1", "Mario Kart", "Switch", None)],
            Vec::new(),
            vec![
                offer("cat:o1", "Switch", vec![release("a", "Mario Kart 8 Deluxe", "", "1.0")]),
                offer("cat:o2", "Switch", vec![release("b", "Mario Kart World", "", "1.0")]),
            ],
        );
        assert!(owned(&two_offers, |o| o.installed_offers(&two_offers.offers)).is_empty(), "two offers fit");

        // Words, not letters: "Mario Strike" doesn't start "Mario Strikers".
        let half_word = library(
            vec![game("emu:G1", "Mario Strike", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Mario Strikers Battle League", "", "1.0")])],
        );
        assert!(owned(&half_word, |o| o.installed_game(&half_word.offers[0]).is_none()), "half a word");
    }

    #[test]
    fn an_offer_of_another_emulator_is_not_the_game() {
        let library = library(
            vec![game("other:G1", "Astro", "PS5", Some("PPSA01234"))],
            Vec::new(),
            vec![offer("cat:o1", "PS5", vec![release("a", "Astro", "PPSA01234", "1.0")])],
        );
        assert!(owned(&library, |o| o.installed_game(&library.offers[0]).is_none()));
    }

    /// The installed game Astro (`emu:G1`), its offer `cat:o1`, and `cat:o2`, an offer the
    /// player doesn't have. `details` are the installed game's.
    fn astro_and_another(details: Vec<(GameKey, Detail)>) -> LibrarySnapshot {
        library(
            vec![game("emu:G1", "Astro", "PS5", Some("PPSA01234"))],
            details,
            vec![
                offer("cat:o1", "PS5", vec![release("a", "Astro", "PPSA01234", "1.0")]),
                offer("cat:o2", "PS5", vec![release("b", "Other", "PPSA09999", "1.0")]),
            ],
        )
    }

    /// Results by game key, as a table test's case gives them.
    type ByKey = &'static [(&'static str, CompatLevel)];

    /// A community result of `level`, with its list's page.
    fn compat(level: CompatLevel) -> CompatInfo {
        CompatInfo { level, notes: None, source: None, page: Some("https://list.example".into()), tested: None }
    }

    /// The installed game's details, with its emulator's community result.
    fn astro_detail(level: CompatLevel) -> (GameKey, Detail) {
        let details = GameDetails {
            summary: summary("G1", "Astro", "PS5", Some("PPSA01234")),
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: None,
            facts: Vec::new(),
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
        };
        let mut detail = Detail::of(details);
        detail.compat = Some(compat(level));
        (key("emu:G1"), detail)
    }

    #[test]
    fn a_key_answers_as_the_installed_game_it_is() {
        let library = astro_and_another(Vec::new());
        // (key, the installed game, the game Play starts, an offer)
        let cases = [
            ("cat:o1", Some("emu:G1"), "emu:G1", true),
            ("cat:o2", None, "cat:o2", true),
            ("emu:G1", Some("emu:G1"), "emu:G1", false),
        ];
        owned(&library, |o| {
            for (asked, installed, played, is_offer) in cases {
                let asked = key(asked);
                let game = o.game(&asked);
                assert_eq!(game.installed(), installed.map(key).as_ref(), "{asked}: the installed game");
                assert_eq!(game.played(), &key(played), "{asked}: Play starts it");
                assert_eq!(game.offer().is_some(), is_offer, "{asked}: an offer");
            }
        });
    }

    #[test]
    fn the_player_s_result_is_the_key_s_own_else_the_installed_game_s() {
        use CompatLevel::{Ingame, Menus};
        // (the results saved, the key, its result)
        let cases: [(ByKey, &str, Option<CompatLevel>); 5] = [
            (&[("emu:G1", Menus)], "cat:o1", Some(Menus)),
            (&[("emu:G1", Menus), ("cat:o1", Ingame)], "cat:o1", Some(Ingame)),
            (&[("emu:G1", Menus)], "cat:o2", None),
            (&[("emu:G1", Menus)], "emu:G1", Some(Menus)),
            (&[], "cat:o1", None),
        ];
        for (saved, asked, expected) in cases {
            let mut library = astro_and_another(Vec::new());
            library.results = saved.iter().map(|(k, l)| ((*k).to_owned(), *l)).collect();
            let found = owned(&library, |o| o.game(&key(asked)).player_result());
            assert_eq!(found, expected, "{asked} with {saved:?}");
        }
    }

    #[test]
    fn the_community_result_is_the_installed_game_s_else_the_list_s_for_the_offer() {
        use CompatLevel::{Boots, Ingame, Playable, Unknown};
        // (the installed game's own, the list's by offer, the key, its result)
        let cases: [(Option<CompatLevel>, ByKey, &str, Option<CompatLevel>); 7] = [
            (Some(Playable), &[], "cat:o1", Some(Playable)),
            (Some(Playable), &[], "emu:G1", Some(Playable)),
            (Some(Playable), &[], "cat:o2", None),
            (None, &[("cat:o2", Boots)], "cat:o2", Some(Boots)),
            // The list doesn't know the installed game's title id: its look-up for the offer's
            // title id says more.
            (Some(Unknown), &[("cat:o1", Ingame)], "cat:o1", Some(Ingame)),
            // Nothing else known: the game's own, with its list's page.
            (Some(Unknown), &[], "cat:o1", Some(Unknown)),
            (None, &[], "cat:o1", None),
        ];
        for (own, listed, asked, expected) in cases {
            let mut library = astro_and_another(own.map(astro_detail).into_iter().collect());
            library.community = Arc::new(listed.iter().map(|(k, l)| (key(k), compat(*l))).collect());
            let found = owned(&library, |o| o.game(&key(asked)).community().cloned());
            assert_eq!(found, expected.map(compat), "{asked}: own {own:?}, listed {listed:?}");
        }
    }

    #[test]
    fn how_far_a_game_gets_is_the_player_s_result_else_the_community_s() {
        use CompatLevel::{Boots, Nothing, Playable, Unknown};
        // (the player's result for the installed game, the installed game's community result,
        // the key, how far it gets)
        let cases = [
            (Some(Nothing), Some(Playable), "cat:o1", Nothing),
            (None, Some(Playable), "cat:o1", Playable),
            (None, Some(Boots), "emu:G1", Boots),
            (None, None, "cat:o1", Unknown),
            (Some(Nothing), Some(Playable), "cat:o2", Unknown),
        ];
        for (result, own, asked, expected) in cases {
            let mut library = astro_and_another(own.map(astro_detail).into_iter().collect());
            library.results = result.map(|l| ("emu:G1".to_owned(), l)).into_iter().collect();
            let found = owned(&library, |o| o.game(&key(asked)).reach());
            assert_eq!(found, expected, "{asked}: result {result:?}, community {own:?}");
        }
    }

    #[test]
    fn the_best_release_is_the_version_the_player_has_then_the_highest() {
        let releases = vec![
            release("new", "Astro", "PPSA01234", "1.00"),
            release("high", "Astro", "PPSA01234", "1.05"),
            release("mine", "Astro", "PPSA01234", "1.02"),
        ];
        let not_installed = library(Vec::new(), Vec::new(), vec![offer("cat:o1", "PS5", releases.clone())]);
        assert_eq!(owned(&not_installed, |o| o.best_release(&not_installed.offers[0]).id.clone()), "high");

        let astro = game("emu:G1", "Astro", "PS5", Some("PPSA01234"));
        let details = GameDetails {
            summary: astro.summary.clone(),
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: None,
            facts: vec![GameFact::new("version", "Version", "01.02")],
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
        };
        let installed =
            library(vec![astro], vec![(key("emu:G1"), Detail::of(details))], vec![offer("cat:o1", "PS5", releases)]);
        assert_eq!(owned(&installed, |o| o.best_release(&installed.offers[0]).id.clone()), "mine");
    }

    #[test]
    fn each_installed_game_names_the_offer_it_is() {
        let library = library(
            vec![game("emu:G1", "Astro", "PS5", Some("PPSA01234"))],
            Vec::new(),
            vec![
                offer("cat:o1", "PS5", vec![release("a", "Astro", "PPSA01234", "1.0")]),
                offer("cat:o2", "PS5", vec![release("b", "Other", "PPSA09999", "1.0")]),
            ],
        );
        let found = owned(&library, |o| o.installed_offers(&library.offers));
        assert_eq!(found, vec![(key("emu:G1"), key("cat:o1"), None)]);
    }
}

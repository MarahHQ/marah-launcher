//! One Catalog card per game: the offers of one game on several consoles (its versions), and
//! the version this PC runs best. Only the Catalog's "All" grid and the Game Hub's versions use
//! the groups: each offer keeps its key, its releases and its downloads.
//!
//! Two offers are versions of one game when, on two consoles, their titles without the edition
//! are the same words, and the dates or the publishers agree (see [`compare`]). A wrong merge
//! hides a game, so a doubt keeps them apart.

use std::cmp::Reverse;
use std::collections::HashMap;

use launcher_api::{CompatLevel, FactKind, GameKey, Platform, PluginId, Requirements};

use crate::hardware::Verdict;
use crate::library::LibrarySnapshot;
use crate::ownership::Ownership;
use crate::releases::Offer;

/// Two offers of one name on two consoles came out at most this many years apart to be one
/// game. A port to PC comes up to four years after the console's (God of War: 2018 and 2022);
/// Resident Evil 4 of 2016 (PS4) and its remake of 2023 are two games.
pub(crate) const PORT_YEARS: i32 = 4;

/// The shortest title, in letters and digits, that groups across consoles: a shorter one is
/// too likely another game's.
const SHORTEST_TITLE: usize = 3;

/// Words in publishers' names that don't tell one publisher from another.
const PUBLISHER_FILLER: [&str; 20] = [
    "the",
    "and",
    "inc",
    "ltd",
    "llc",
    "co",
    "corp",
    "corporation",
    "company",
    "limited",
    "gmbh",
    "sa",
    "entertainment",
    "interactive",
    "games",
    "game",
    "studios",
    "studio",
    "publishing",
    "software",
];

/// The Catalog's games of more than one version.
#[derive(Clone, Debug, Default)]
pub(crate) struct GameGroups {
    /// Each game's versions, two or more, in the Catalog's order.
    members: Vec<Vec<GameKey>>,
    /// The game each version is, by its key.
    by_key: HashMap<GameKey, usize>,
}

impl GameGroups {
    /// The versions of each game among `offers` (see [`compare`]).
    pub fn new(offers: &[Offer]) -> Self {
        let facts: Vec<Facts> = offers.iter().map(Facts::of).collect();
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, f) in facts.iter().enumerate() {
            if let Some(name) = &f.name {
                by_name.entry(name).or_default().push(i);
            }
        }
        let mut names: Vec<Vec<usize>> = by_name.into_values().filter(|b| b.len() > 1).collect();
        names.sort();
        let mut groups = Self::default();
        for name in names {
            for game in games_of(&facts, name) {
                let at = groups.members.len();
                let keys: Vec<GameKey> = game.iter().map(|&i| offers[i].entry.key.clone()).collect();
                for key in &keys {
                    groups.by_key.insert(key.clone(), at);
                }
                groups.members.push(keys);
            }
        }
        groups
    }

    /// The versions of the game `key` is, in the Catalog's order: only `key` for a game of one.
    pub fn versions<'a>(&'a self, key: &'a GameKey) -> &'a [GameKey] {
        match self.by_key.get(key) {
            Some(&at) => &self.members[at],
            None => std::slice::from_ref(key),
        }
    }

    /// Which game `key` is: the same number for each of its versions. `None` for a game of one.
    pub fn game(&self, key: &GameKey) -> Option<usize> {
        self.by_key.get(key).copied()
    }
}

/// What the grouping reads of an offer.
struct Facts {
    /// The title's words without its edition, joined; `None` when it is too short to group.
    name: Option<String>,
    console: String,
    /// The year its game first came out, by its releases' dates.
    year: Option<i32>,
    /// The publisher's telling words, in lower case; empty when no release says.
    publisher: Vec<String>,
}

impl Facts {
    fn of(offer: &Offer) -> Self {
        Self {
            name: game_name(&offer.entry.summary.title),
            console: offer.entry.summary.platform.to_lowercase(),
            year: offer.releases.iter().filter_map(year_of).min(),
            publisher: offer
                .releases
                .iter()
                .map(|r| r.fact_text(FactKind::Publisher))
                .chain(offer.releases.iter().filter_map(|r| r.metadata.as_ref()).map(|m| m.publisher.as_str()))
                .map(publisher_words)
                .find(|words| !words.is_empty())
                .unwrap_or_default(),
        }
    }
}

/// A title as a grouping key across consoles: its words without the edition, joined.
/// "Racer: Deluxe Edition" is "racer", "Racer 2" is "racer2". `None` for a title of fewer than
/// [`SHORTEST_TITLE`] letters and digits, with no letter, or spelled out letter by letter
/// ("S.P.L.I.T" would be another game's "Split").
fn game_name(title: &str) -> Option<String> {
    let words = crate::releases::edition_free_words(title);
    let spelled = words.len() > 1 && words.iter().all(|w| w.chars().count() == 1);
    let name = words.concat();
    let long = name.chars().count() >= SHORTEST_TITLE && name.chars().any(char::is_alphabetic);
    (long && !spelled).then_some(name)
}

/// The year a release's game came out, from the catalog's date or the store's.
fn year_of(release: &launcher_api::Release) -> Option<i32> {
    crate::releases::release_day(release)?.get(..4)?.parse().ok()
}

/// The words of a publisher's name that tell it apart: "Sony Interactive Entertainment" is
/// `["sony"]`.
fn publisher_words(name: &str) -> Vec<String> {
    let name = name.to_lowercase();
    let words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    words.filter(|w| !PUBLISHER_FILLER.contains(w)).map(str::to_owned).collect()
}

/// What two offers of one name say about being one game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Says {
    Same,
    Other,
    Unsure,
}

/// Whether two offers of one name are versions of one game:
///
/// - on one console they are two (one console's versions are one offer already);
/// - with both years known: one game at most [`PORT_YEARS`] apart, whoever publishes each (a
///   port often has a publisher of its own), else two;
/// - else with both publishers known: one game when they have a telling word in common, else two;
/// - else nothing says.
///
/// A name alone is no proof: "Prey" (2006) and "Prey" (2017) are two games.
fn compare(a: &Facts, b: &Facts) -> Says {
    if a.console == b.console {
        return Says::Other;
    }
    let same = |yes: bool| if yes { Says::Same } else { Says::Other };
    match (a.year, b.year) {
        (Some(x), Some(y)) => same((x - y).abs() <= PORT_YEARS),
        _ if a.publisher.is_empty() || b.publisher.is_empty() => Says::Unsure,
        _ => same(a.publisher.iter().any(|w| b.publisher.contains(w))),
    }
}

/// The games of two or more versions among the offers of one name (`name`, in the Catalog's
/// order). A version joins a game when no version of it says otherwise and one says it is the
/// same (see [`compare`]). The dated offers go first, oldest first; an offer that fits two
/// games joins neither: it can't tell which.
fn games_of(facts: &[Facts], name: Vec<usize>) -> Vec<Vec<usize>> {
    let mut order = name;
    order.sort_by_key(|&i| (facts[i].year.is_none(), facts[i].year, i));
    let mut games: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let fits = |game: &Vec<usize>| {
            let says: Vec<Says> = game.iter().map(|&j| compare(&facts[i], &facts[j])).collect();
            !says.contains(&Says::Other) && says.contains(&Says::Same)
        };
        let fit: Vec<usize> = (0..games.len()).filter(|&g| fits(&games[g])).collect();
        match fit.as_slice() {
            [g] => games[*g].push(i),
            [] => games.push(vec![i]),
            _ => {}
        }
    }
    games.retain(|g| g.len() > 1);
    for g in &mut games {
        g.sort_unstable();
    }
    games
}

/// How well this PC meets an emulator's needs, worst first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Speed {
    /// Won't start.
    No,
    /// The check hasn't run, or the system can't say.
    Unknown,
    /// Runs, slower than its games need.
    Slow,
    /// Meets what it recommends.
    Good,
}

/// What the best pick knows about one emulator, from its manifest and this PC.
#[derive(Clone, Debug)]
pub(crate) struct EmulatorFit {
    pub speed: Speed,
    /// What it recommends (`[requires.recommended]`): GPU memory, memory, threads. A plugin that
    /// emulates nothing, such as the PC's, recommends nothing.
    pub recommends: [u32; 3],
}

impl EmulatorFit {
    /// From the emulator's `requires` and its verdict on this PC; `None` before the check.
    pub fn new(requires: &Requirements, verdict: Option<&Verdict>) -> Self {
        let speed = match verdict {
            Some(Verdict::Good) => Speed::Good,
            Some(Verdict::Slow(_)) => Speed::Slow,
            Some(Verdict::No(_)) => Speed::No,
            Some(Verdict::Unknown(_)) | None => Speed::Unknown,
        };
        let r = &requires.recommended;
        let tenths = |gb: Option<f32>| gb.map_or(0, |gb| (gb * 10.0).round() as u32);
        Self { speed, recommends: [tenths(r.gpu_memory_gb), tenths(r.memory_gb), r.cpu_threads.unwrap_or(0)] }
    }
}

/// What this PC is, for the best pick: its system and each emulator's fit.
#[derive(Clone, Debug, Default)]
pub(crate) struct ThisPc {
    pub system: Option<Platform>,
    pub emulators: HashMap<PluginId, EmulatorFit>,
}

/// Where a version's reach comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Said {
    /// The player's own result.
    Player,
    /// The emulator's community list.
    Community,
    /// A PC release made for this system: it runs as it is.
    Native,
    /// A compatibility layer's rating (ProtonDB's, for a Windows release off Windows).
    Layer,
    /// Nobody says.
    Nobody,
}

/// How one version runs on this PC. The best version is the greatest: compared by `starts`,
/// then `reach`, then `speed`, then `light`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Fit {
    /// It can start here: its emulator's needs are met, and a release is for this system.
    pub starts: bool,
    /// How far it gets: Playable 5, In-game 4, Menus 3, Boots 2, not tested 1, doesn't start 0.
    pub reach: u8,
    pub speed: Speed,
    /// What its emulator recommends, less first.
    pub light: Reverse<[u32; 3]>,
}

/// One version of a game and how it runs here.
#[derive(Clone, Debug)]
pub(crate) struct Version<'a> {
    pub offer: &'a Offer,
    pub fit: Fit,
    pub level: CompatLevel,
    pub said: Said,
}

/// A compatibility level as a reach to compare: a game that doesn't start is below one nobody
/// tested.
fn reach_of(level: CompatLevel) -> u8 {
    match level {
        CompatLevel::Nothing => 0,
        CompatLevel::Unknown => 1,
        CompatLevel::Boots => 2,
        CompatLevel::Menus => 3,
        CompatLevel::Ingame => 4,
        CompatLevel::Playable => 5,
    }
}

/// A compatibility layer's rating as a level: ProtonDB's words.
fn layer_level(rating: &str) -> CompatLevel {
    match rating.to_lowercase().as_str() {
        "platinum" | "gold" => CompatLevel::Playable,
        "silver" | "bronze" => CompatLevel::Ingame,
        "borked" => CompatLevel::Nothing,
        _ => CompatLevel::Unknown,
    }
}

/// The community list's result for `offer`'s game, as the core has it: an offer's details carry
/// none, so it is the result of the installed game the offer is (the emulator's list, looked up
/// by its title id). The list itself stays in the emulator's plugin, so an offer the player
/// doesn't have has none.
pub(crate) fn community(library: &LibrarySnapshot, owned: &Ownership, offer: &Offer) -> Option<CompatLevel> {
    let level = |key: &GameKey| library.detail(key).and_then(|d| d.compat.as_ref()).map(|c| c.level);
    level(&offer.entry.key).or_else(|| owned.installed_game(offer).and_then(|game| level(&game.key)))
}

/// How `offer`'s game runs here: from the player's result, the community's list, a release for
/// this system, or the layer's rating, in that order; then the emulator's fit.
pub(crate) fn version<'a>(library: &LibrarySnapshot, owned: &Ownership, pc: &ThisPc, offer: &'a Offer) -> Version<'a> {
    let key = &offer.entry.key;
    let community = community(library, owned, offer);
    let native = pc.system.and_then(|here| {
        let label = match here {
            Platform::Linux => "linux",
            Platform::Macos => "macos",
            Platform::Windows => "windows",
        };
        offer.releases.iter().any(|r| r.system == label).then_some(CompatLevel::Playable)
    });
    let layer = Some(layer_level(&crate::releases::compatibility(offer).rating));
    let found = [
        (library.result(key), Said::Player),
        (community, Said::Community),
        (native, Said::Native),
        (layer, Said::Layer),
    ];
    let (level, said) = found
        .into_iter()
        .find_map(|(level, said)| level.filter(|l| *l != CompatLevel::Unknown).map(|l| (l, said)))
        .unwrap_or((CompatLevel::Unknown, Said::Nobody));
    let emulator = pc.emulators.get(&offer.emulator);
    let speed = emulator.map_or(Speed::Unknown, |e| e.speed);
    let for_here = offer.releases.iter().any(|r| crate::releases::not_here(&r.system, pc.system).is_empty());
    let fit = Fit {
        starts: speed != Speed::No && for_here && level != CompatLevel::Nothing,
        reach: reach_of(level),
        speed,
        light: Reverse(emulator.map_or([0; 3], |e| e.recommends)),
    };
    Version { offer, fit, level, said }
}

/// The versions of `offers`, best for this PC first. On a tie, the Catalog's order stays.
pub(crate) fn best_first<'a>(
    library: &LibrarySnapshot,
    owned: &Ownership,
    pc: &ThisPc,
    offers: &[&'a Offer],
) -> Vec<Version<'a>> {
    let mut versions: Vec<Version<'a>> = offers.iter().map(|o| version(library, owned, pc, o)).collect();
    versions.sort_by_key(|v| Reverse(v.fit));
    versions
}

/// Why `best` is the best version, beside the next one: the first thing that sets it apart.
/// Empty for a game of one version, and when no version can start here: none is the best.
pub(crate) fn why_best(best: &Version, next: Option<&Version>, words: impl Fn(CompatLevel) -> &'static str) -> String {
    let Some(next) = next.filter(|_| best.fit.starts) else { return String::new() };
    let (a, b) = (&best.fit, &next.fit);
    if a.starts != b.starts {
        return "The other versions can't start on this PC".into();
    }
    if a.reach != b.reach {
        let level = words(best.level);
        return match best.said {
            Said::Player => format!("Gets the furthest: {level}, in your own result"),
            Said::Community => format!("Gets the furthest: {level}, as players report"),
            Said::Native => "Made for this system: no emulator, no compatibility layer".into(),
            Said::Layer => format!("Gets the furthest: {level}, as its compatibility layer's rating says"),
            Said::Nobody => "The others don't start".into(),
        };
    }
    if a.speed != b.speed {
        return "This PC meets what its emulator recommends".into();
    }
    if a.light != b.light {
        return "Its plugin recommends the least of this PC".into();
    }
    "As good as the others: the newest in the Catalog".into()
}

#[cfg(test)]
mod tests {
    use launcher_api::{Fact, GameSummary, Release};

    use super::*;
    use crate::library::Entry;

    /// An offer of `title` on `console`, its release dated `date` and published by `publisher`
    /// (empty for none).
    fn offer(title: &str, console: &str, date: &str, publisher: &str) -> Offer {
        let mut facts = Vec::new();
        if !date.is_empty() {
            facts.push(Fact::new(FactKind::ReleaseDate, "Release date", date));
        }
        if !publisher.is_empty() {
            facts.push(Fact::new(FactKind::Publisher, "Publisher", publisher));
        }
        let release = Release { title: title.into(), platform: console.into(), facts, ..Release::default() };
        let local = format!("{}-{}", crate::releases::plain(title), console.to_lowercase());
        let key: GameKey = format!("cat:{local}").parse().unwrap();
        let summary = GameSummary {
            id: local,
            title: title.into(),
            platform: console.into(),
            cover: None,
            tags: Vec::new(),
            release_year: None,
            title_id: None,
            content_id: None,
        };
        Offer {
            entry: Entry { key, summary },
            emulator: PluginId::new(console.to_lowercase()).unwrap(),
            releases: vec![release],
        }
    }

    /// An offer as its title, console, release date and publisher.
    type Row<'a> = (&'a str, &'a str, &'a str, &'a str);

    /// The groups of `offers`, each as its members' indexes.
    fn grouped(offers: &[Offer]) -> Vec<Vec<usize>> {
        let groups = GameGroups::new(offers);
        let index = |key: &GameKey| offers.iter().position(|o| &o.entry.key == key).unwrap();
        groups.members.iter().map(|m| m.iter().map(index).collect()).collect()
    }

    #[test]
    fn one_game_on_several_consoles_is_one_group() {
        // (offers, the groups they make)
        let merge: [(&[Row], &[&[usize]]); 7] = [
            (&[("Astro Bot", "PS5", "2024-09-06", "Sony"), ("Astro Bot", "PC", "2024", "")], &[&[0, 1]]),
            // A port years after, by another publisher.
            (
                &[
                    ("God of War", "PS4", "2018", "Sony Interactive Entertainment"),
                    ("God of War", "PC", "2022", "PlayStation PC LLC"),
                ],
                &[&[0, 1]],
            ),
            // Editions group as on one console; punctuation and case don't count.
            (
                &[
                    ("Ghost of Tsushima", "PS4", "2020", ""),
                    ("Ghost of Tsushima: Deluxe Edition", "PS5", "2021", ""),
                    ("GHOST OF TSUSHIMA", "PC", "2024", ""),
                ],
                &[&[0, 1, 2]],
            ),
            (
                &[("Marvel's Spider-Man", "PS4", "2018", ""), ("Marvel\u{2019}s Spider Man", "PC", "2022", "")],
                &[&[0, 1]],
            ),
            (&[("Tomb Raider: Definitive Edition", "PS4", "2014", ""), ("Tomb Raider", "PC", "2013", "")], &[&[0, 1]]),
            // No years, one publisher.
            (
                &[("Celeste", "PS4", "", "Maddy Makes Games Inc."), ("Celeste", "PC", "", "Maddy Makes Games")],
                &[&[0, 1]],
            ),
            // An undated version joins the one game of its name it fits.
            (
                &[
                    ("Hades", "PS5", "2021", "Supergiant Games"),
                    ("Hades", "PC", "2020", ""),
                    ("Hades", "PS4", "", "Supergiant"),
                ],
                &[&[0, 1, 2]],
            ),
        ];
        for (offers, groups) in merge {
            let offers: Vec<Offer> = offers.iter().map(|(t, c, d, p)| offer(t, c, d, p)).collect();
            assert_eq!(
                grouped(&offers),
                groups,
                "{:?}",
                offers.iter().map(|o| &o.entry.summary.title).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn near_misses_stay_apart() {
        // Each row: two or more offers that must not be one game.
        let apart: [(&str, &[Row]); 14] = [
            ("a sequel's number", &[("Alan Wake", "PS4", "2010", "Remedy"), ("Alan Wake 2", "PS5", "2023", "Remedy")]),
            (
                "a sequel's numeral",
                &[
                    ("Final Fantasy VII", "PS4", "2015", "Square Enix"),
                    ("Final Fantasy VIII", "PC", "2015", "Square Enix"),
                ],
            ),
            (
                "a remaster",
                &[("Alan Wake", "PS4", "2010", "Remedy"), ("Alan Wake Remastered", "PS5", "2021", "Remedy")],
            ),
            ("a remake", &[("Demon's Souls", "PS4", "2009", "Sony"), ("Demon's Souls Remake", "PS5", "2020", "Sony")]),
            (
                "a director's cut",
                &[
                    ("Death Stranding", "PS4", "2019", "Sony"),
                    ("Death Stranding Director's Cut", "PS5", "2021", "Sony"),
                ],
            ),
            (
                "a standalone expansion",
                &[("Alan Wake", "PC", "2012", "Remedy"), ("Alan Wake: American Nightmare", "PS4", "2012", "Remedy")],
            ),
            ("one name, years apart", &[("Prey", "PC", "2006", "2K"), ("Prey", "PS4", "2017", "Bethesda")]),
            (
                "one name, years apart and one publisher",
                &[("Resident Evil 4", "PS4", "2016", "Capcom"), ("Resident Evil 4", "PS5", "2023", "Capcom")],
            ),
            ("one console", &[("Doom", "PS4", "2016", "Bethesda"), ("Doom", "PS4", "2016", "Bethesda")]),
            ("no year, no publisher", &[("Tetris", "PS4", "", ""), ("Tetris", "PC", "", "")]),
            ("no year, other publishers", &[("Outlaws", "PS4", "", "Ubisoft"), ("Outlaws", "PC", "", "LucasArts")]),
            ("a title too short", &[("Ys", "PS4", "2016", "NIS"), ("Ys", "PC", "2016", "NIS")]),
            ("a title of digits only", &[("1979", "PS4", "2016", "Ninja"), ("1979", "PC", "2016", "Ninja")]),
            (
                "a title spelled out",
                &[("Split", "PS4", "2023", "FreeMind"), ("s.p.l.i.t", "PC", "2025", "Mike Klubnika")],
            ),
        ];
        for (why, offers) in apart {
            let offers: Vec<Offer> = offers.iter().map(|(t, c, d, p)| offer(t, c, d, p)).collect();
            assert!(grouped(&offers).is_empty(), "{why}");
        }
    }

    #[test]
    fn an_undated_version_that_fits_two_games_joins_neither() {
        let offers = [
            offer("Doom", "PC", "1993", "id Software"),
            offer("Doom", "PS4", "2016", "id Software"),
            offer("Doom", "PS5", "2016", "id Software"),
            offer("Doom", "Switch", "", "id Software"),
        ];
        assert_eq!(grouped(&offers), [[1, 2]], "the 2016 game; the 1993 one and the undated one stay alone");
    }

    #[test]
    fn a_version_knows_its_game() {
        let offers = [
            offer("Astro Bot", "PS5", "2024", ""),
            offer("Astro Bot", "PC", "2024", ""),
            offer("Quest", "PS4", "", ""),
        ];
        let groups = GameGroups::new(&offers);
        let keys: Vec<&GameKey> = offers.iter().map(|o| &o.entry.key).collect();
        assert_eq!(groups.versions(keys[1]), [keys[0].clone(), keys[1].clone()]);
        assert_eq!(groups.versions(keys[2]), [keys[2].clone()], "a game of one");
        assert_eq!(groups.game(keys[0]), groups.game(keys[1]));
        assert_eq!(groups.game(keys[2]), None);
    }

    fn emulator(speed: Speed, recommends: [u32; 3]) -> EmulatorFit {
        EmulatorFit { speed, recommends }
    }

    /// An offer for `console`, its releases for `system` (empty for a console's), rated `layer`
    /// by a compatibility layer (empty for none).
    fn on(console: &str, system: &str, layer: &str) -> Offer {
        let mut offer = offer("Game", console, "", "");
        offer.releases[0].system = system.into();
        if !layer.is_empty() {
            offer.releases[0].facts.push(Fact::new(FactKind::Compatibility, "ProtonDB", layer));
        }
        offer
    }

    fn library(results: &[(&Offer, CompatLevel)]) -> LibrarySnapshot {
        let mut library = LibrarySnapshot::default();
        library.results = results.iter().map(|(o, l)| (o.entry.key.to_string(), *l)).collect();
        library
    }

    /// The consoles of `offers`, best for this PC first, and why the first.
    fn pick(pc: &ThisPc, library: &LibrarySnapshot, offers: &[&Offer]) -> (Vec<String>, String) {
        let (jobs, installs) = (Default::default(), Default::default());
        let versions = best_first(library, &Ownership::new(library, &jobs, &installs), pc, offers);
        let why = why_best(&versions[0], versions.get(1), |l| l.as_str());
        (versions.iter().map(|v| v.offer.entry.summary.platform.clone()).collect(), why)
    }

    #[test]
    fn the_best_version_is_the_one_this_pc_runs_best() {
        let ps5 = on("PS5", "", "");
        let ps4 = on("PS4", "", "");
        let linux = on("PC", "linux", "");
        let windows = on("PC", "windows", "silver");
        let pc = |kyty: Speed, shad: Speed| ThisPc {
            system: Some(Platform::Linux),
            emulators: HashMap::from([
                (PluginId::new("ps5").unwrap(), emulator(kyty, [75, 145, 12])),
                (PluginId::new("ps4").unwrap(), emulator(shad, [35, 70, 8])),
                (PluginId::new("pc").unwrap(), emulator(Speed::Good, [0; 3])),
            ]),
        };
        let none = library(&[]);
        let good = pc(Speed::Good, Speed::Good);

        // A native release needs no emulator.
        let (order, why) = pick(&good, &none, &[&ps5, &ps4, &linux]);
        assert_eq!(order, ["PC", "PS4", "PS5"]);
        assert_eq!(why, "Made for this system: no emulator, no compatibility layer");

        // Untested on both consoles, the same speed: the lighter emulator.
        let (order, why) = pick(&good, &none, &[&ps5, &ps4]);
        assert_eq!(order, ["PS4", "PS5"]);
        assert_eq!(why, "Its plugin recommends the least of this PC");

        // Players' results go before speed and weight.
        let playable = library(&[(&ps5, CompatLevel::Playable), (&ps4, CompatLevel::Menus)]);
        let (order, why) = pick(&pc(Speed::Slow, Speed::Good), &playable, &[&ps4, &ps5]);
        assert_eq!(order, ["PS5", "PS4"]);
        assert_eq!(why, "Gets the furthest: playable, in your own result");

        // What this PC can't start goes last, whatever its reach.
        let (order, why) = pick(&pc(Speed::No, Speed::Good), &playable, &[&ps5, &ps4]);
        assert_eq!(order, ["PS4", "PS5"]);
        assert_eq!(why, "The other versions can't start on this PC");

        // Of one reach, the one this PC meets the recommended parts of.
        let (order, why) = pick(&pc(Speed::Good, Speed::Slow), &none, &[&ps4, &ps5]);
        assert_eq!(order, ["PS5", "PS4"]);
        assert_eq!(why, "This PC meets what its emulator recommends");

        // A Windows release through the layer, rated silver (in-game), beats an untested console.
        let (order, _) = pick(&good, &none, &[&ps4, &windows]);
        assert_eq!(order, ["PC", "PS4"]);

        // A Mac release can't start on Linux.
        let mac = on("PC", "macos", "");
        let (order, _) = pick(&good, &none, &[&mac, &ps4]);
        assert_eq!(order, ["PS4", "PC"]);

        // When no version can start, none is the best.
        let (_, why) = pick(&pc(Speed::No, Speed::No), &none, &[&ps5, &ps4]);
        assert_eq!(why, "");

        // A game that doesn't start goes below an untested one.
        let broken = library(&[(&ps4, CompatLevel::Nothing)]);
        let (order, _) = pick(&good, &broken, &[&ps4, &ps5]);
        assert_eq!(order, ["PS5", "PS4"]);
    }

    #[test]
    fn before_the_check_the_order_is_the_catalog_s_but_for_the_lighter_emulator() {
        let (ps5, ps4) = (on("PS5", "", ""), on("PS4", "", ""));
        let unchecked = ThisPc {
            system: None,
            emulators: HashMap::from([
                (PluginId::new("ps5").unwrap(), emulator(Speed::Unknown, [75, 145, 12])),
                (PluginId::new("ps4").unwrap(), emulator(Speed::Unknown, [35, 70, 8])),
            ]),
        };
        let (order, _) = pick(&unchecked, &library(&[]), &[&ps5, &ps4]);
        assert_eq!(order, ["PS4", "PS5"]);
        let (order, why) = pick(&ThisPc::default(), &library(&[]), &[&ps5, &ps4]);
        assert_eq!(order, ["PS5", "PS4"], "nothing known: the Catalog's order");
        assert_eq!(why, "As good as the others: the newest in the Catalog");
    }
}

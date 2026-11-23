//! The catalog sources: the forums each console's catalog comes from, in the format of the
//! project's `assets/catalog-sources.json`. The catalog collector reads the same file. The root
//! is each catalog, by its file name without `.json`:
//!
//! ```json
//! { "ps5": { "platform": "PS5", "forums": [
//!     { "key": "ps5", "url": "https://xyz.net/forum/viewforum.php?f=546", "tag": "PS5" }] } }
//! ```
//!
//! - A catalog's `platform` is its console, as an emulator's manifest names it.
//! - A forum's `url` is its first listing page. The plugin never fetches it: it takes the
//!   forum's `f=` number for the refresh's addresses (see [`crate::refresh::API`]) and writes the
//!   `url` into the `source` of the topics it adds.
//! - A forum has a console `tag` or a PC `system`, and `wine` is `true` for a forum whose games
//!   are packed with Wine.
//! - A forum of many consoles has `titles`: the tags of its catalog's console. The catalog takes
//!   only the topics one of whose tags before the name is one of them ([`Forum::takes`]). So one
//!   forum can feed two catalogs, each with its own `titles`.
//!
//! A file reads only when all of it does: every forum's `url` starts with `https://`, because
//! it goes into the `source` of each topic the refresh adds.
//!
//! The catalog sources are the one place that says what a forum's topics are for (see
//! [`Forums`]): a catalog file names its forum, and [`crate::format`] looks it up here.

use std::collections::{BTreeMap, HashMap};

use launcher_api::Console;
use launcher_api::serde::Deserialize;
use launcher_api::serde_json;

use crate::https_host;
use crate::refresh::forum_number;

/// The forums of each console's catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(crate = "launcher_api::serde", transparent)]
pub struct Sources {
    /// Each catalog, by its file name without `.json` (`ps5`): the root of the file.
    pub catalogs: BTreeMap<String, SourceCatalog>,
}

/// One console's catalog and the forums it comes from.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(crate = "launcher_api::serde")]
pub struct SourceCatalog {
    /// The console, as an emulator's manifest names it (`PS5`). The file names it `platform`,
    /// as the catalog collector does.
    #[serde(rename = "platform")]
    pub console: Console,
    pub forums: Vec<Forum>,
}

/// One forum.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(crate = "launcher_api::serde")]
pub struct Forum {
    /// The name the collector uses for it (`ps5`).
    pub key: String,
    /// Its first listing page, `.../viewforum.php?f=<number>`.
    pub url: String,
    /// For a console forum, the tag that starts its titles (`PS5`).
    #[serde(default)]
    pub tag: Option<String>,
    /// For a PC forum, the system its games are for (`linux`).
    #[serde(default)]
    pub system: Option<String>,
    /// Its games are packed with Wine.
    #[serde(default)]
    pub wine: bool,
    /// For a forum of many consoles, the title tags of this catalog's console (`NES`, `Dendy`).
    /// Empty: every topic is this catalog's.
    #[serde(default)]
    pub titles: Vec<String>,
}

impl Forum {
    /// The topic titled `title` belongs to this forum's catalog: the forum has no `titles`, or
    /// one of the `[…]` tags before the title's name is one of them, in any case.
    pub fn takes(&self, title: &str) -> bool {
        self.titles.is_empty()
            || leading_tags(title).any(|tag| self.titles.iter().any(|t| t.trim().eq_ignore_ascii_case(tag)))
    }
}

/// The `[…]` tags before a title's name, trimmed: `[Обновлено] [PS] Crash` gives `Обновлено`, `PS`.
fn leading_tags(title: &str) -> impl Iterator<Item = &str> {
    let mut rest = title.trim_start();
    std::iter::from_fn(move || {
        let inner = rest.strip_prefix('[')?;
        let (tag, after) = inner.split_once(']')?;
        rest = after.trim_start();
        Some(tag.trim())
    })
}

/// Reads a catalog sources file.
pub fn read(bytes: &[u8]) -> Result<Sources, String> {
    let sources: Sources = serde_json::from_slice(bytes)
        .map_err(|e| format!("this is not a catalog sources file in the current format: {e}"))?;
    for url in sources.catalogs.values().flat_map(|c| c.forums.iter().map(|f| f.url.as_str())) {
        if https_host(url).is_none() {
            return Err(format!("not a catalog sources file: {url} is not an https:// address"));
        }
    }
    Ok(sources)
}

impl Sources {
    /// The sources with only the forums whose `url` has an `f=` number, and each forum left
    /// out, as `<key>: <url>`. A catalog with no forum left is left out.
    pub fn usable(mut self) -> (Sources, Vec<String>) {
        let mut unusable = Vec::new();
        for catalog in self.catalogs.values_mut() {
            catalog.forums.retain(|f| {
                let ok = forum_number(&f.url).is_some();
                if !ok {
                    unusable.push(format!("{}: {}", f.key, f.url));
                }
                ok
            });
        }
        self.catalogs.retain(|_, c| !c.forums.is_empty());
        (self, unusable)
    }
}

/// What a forum's topics are for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForumFacts {
    /// The console of the forum's catalog (`PS5`).
    pub console: Console,
    /// For a PC forum, the system its games are for (`linux`).
    pub system: Option<String>,
    /// Its games are packed with Wine.
    pub wine: bool,
}

/// Each forum of the catalog sources, by its `f=` number and by its key, with what its topics
/// are for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Forums {
    /// A forum that feeds two catalogs has the facts of each.
    by_number: HashMap<String, Vec<ForumFacts>>,
    by_key: HashMap<String, ForumFacts>,
}

impl Forums {
    /// The forum a catalog file or a topic names: the one of the `f=` number in its `source`,
    /// else, with no number there, the one whose key is its `platform` (`ps5`, `wine`), as the
    /// collector's listing of one forum names it. Of a forum that feeds two catalogs, the one
    /// whose console is the `platform`. `None` for a forum the sources don't have.
    pub fn named(&self, source: Option<&str>, platform: Option<&str>) -> Option<&ForumFacts> {
        match source.and_then(forum_number) {
            Some(number) => {
                let all = self.by_number.get(&number)?;
                let platform = platform.map(str::trim).unwrap_or_default();
                all.iter().find(|f| f.console.as_str().eq_ignore_ascii_case(platform)).or(all.first())
            }
            None => self.by_key.get(&platform?.trim().to_ascii_lowercase()),
        }
    }
}

impl Sources {
    /// What each forum's topics are for.
    pub fn forums(&self) -> Forums {
        let mut forums = Forums::default();
        for catalog in self.catalogs.values() {
            for forum in &catalog.forums {
                let facts =
                    ForumFacts { console: catalog.console.clone(), system: forum.system.clone(), wine: forum.wine };
                if let Some(number) = forum_number(&forum.url) {
                    forums.by_number.entry(number).or_default().push(facts.clone());
                }
                forums.by_key.insert(forum.key.to_ascii_lowercase(), facts);
            }
        }
        forums
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: &str = r#"{
          "ps5": { "platform": "PS5", "forums": [{ "key": "ps5", "url": "https://xyz.net/f?f=1", "tag": "PS5" }] },
          "pc": { "platform": "PC", "forums": [
            { "key": "linux", "url": "https://xyz.net/f?f=2", "system": "linux" },
            { "key": "wine", "url": "https://xyz.net/f?t=3", "system": "linux", "wine": true }] } }"#;

    #[test]
    fn a_sources_file_reads_with_its_forums() {
        let sources = read(SOURCES.as_bytes()).unwrap();
        assert_eq!(sources.catalogs["pc"].console.as_str(), "PC");
        let wine = &sources.catalogs["pc"].forums[1];
        assert_eq!((wine.key.as_str(), wine.system.as_deref(), wine.wine), ("wine", Some("linux"), true));
        assert_eq!(sources.catalogs["ps5"].forums[0].tag.as_deref(), Some("PS5"));
        assert!(read(b"{ not json").is_err());
        assert!(read(br#"{ "api": "https://xyz.net/f/" }"#).is_err(), "the root is the catalogs");
    }

    #[test]
    fn a_sources_file_with_an_address_that_is_not_https_does_not_read() {
        let plain = SOURCES.replace("https://xyz.net/f?f=2", "http://xyz.net/f?f=2");
        assert!(read(plain.as_bytes()).unwrap_err().contains("http://xyz.net/f?f=2 is not an https:// address"));
    }

    #[test]
    fn a_forum_with_no_f_number_is_left_out_and_named() {
        let (kept, unusable) = read(SOURCES.as_bytes()).unwrap().usable();
        assert_eq!(unusable, ["wine: https://xyz.net/f?t=3"]);
        let keys: Vec<&str> = kept.catalogs["pc"].forums.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["linux"]);
        assert_eq!(kept.catalogs["ps5"].forums.len(), 1);
    }

    #[test]
    fn a_forum_is_found_by_its_number_else_by_its_key() {
        let forums = read(SOURCES.as_bytes()).unwrap().forums();
        let linux = forums.named(Some("https://xyz.net/f?f=2"), Some("ps5")).unwrap();
        assert_eq!((linux.console.as_str(), linux.system.as_deref(), linux.wine), ("PC", Some("linux"), false));
        let wine = forums.named(None, Some("Wine")).unwrap();
        assert_eq!((wine.console.as_str(), wine.wine), ("PC", true), "a forum with no f= number has its key");
        assert_eq!(forums.named(Some("https://xyz.dev/catalog"), Some("ps5")).unwrap().console.as_str(), "PS5");
        assert_eq!(forums.named(Some("https://xyz.net/f?f=9"), Some("ps5")), None, "a number the sources don't have");
        assert_eq!(forums.named(None, None), None);
    }

    /// One forum of many consoles feeds two catalogs, each with the titles of its console.
    const MIXED: &str = r#"{
          "nes": { "platform": "NES", "forums": [
            { "key": "nes", "url": "https://xyz.net/f?f=7", "titles": ["NES", "Dendy"] }] },
          "genesis": { "platform": "Genesis", "forums": [
            { "key": "genesis", "url": "https://xyz.net/f?f=7", "titles": ["SMD", "Sega Genesis"] }] } }"#;

    #[test]
    fn a_forum_with_titles_takes_the_topics_one_of_whose_first_tags_is_one_of_them() {
        let sources = read(MIXED.as_bytes()).unwrap();
        let nes = &sources.catalogs["nes"].forums[0];
        assert!(nes.takes("[NES] Battletoads [U] [1.2 MB]"));
        assert!(nes.takes("[Обновлено] [dendy] Contra [RUS]"), "any of the tags before the name, in any case");
        assert!(!nes.takes("[SNES] Contra III"), "a whole tag, not part of one");
        assert!(!nes.takes("[Amiga] NES Remix"), "only the tags before the name");
        assert!(!nes.takes("Battletoads"));
        let genesis = &sources.catalogs["genesis"].forums[0];
        assert!(genesis.takes("[Sega Genesis] Sonic the Hedgehog"));
        let ps5 = &read(SOURCES.as_bytes()).unwrap().catalogs["ps5"].forums[0];
        assert!(ps5.takes("[Amiga] anything"), "a forum without titles takes every topic");
    }

    #[test]
    fn a_topic_of_a_forum_that_feeds_two_catalogs_is_its_platform_s() {
        let forums = read(MIXED.as_bytes()).unwrap().forums();
        let url = Some("https://xyz.net/f?f=7");
        assert_eq!(forums.named(url, Some("GENESIS")).unwrap().console.as_str(), "GENESIS");
        assert_eq!(forums.named(url, Some("nes")).unwrap().console.as_str(), "NES");
    }
}

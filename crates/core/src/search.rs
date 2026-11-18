//! The search of the Library and the Catalog: how well a game answers what the player typed.
//!
//! With a controller each letter takes a few presses, so the search forgives how a title was
//! typed:
//! - accents, case and punctuation don't count: "pokemon" finds Pokémon, "spider man" and
//!   "spiderman" find Spider-Man;
//! - words typed together: "godofwar" finds God of War;
//! - initials, with a number at the end: "tlou", "gow", "re4", "ff7";
//! - numbers either way, up to 20: "7" finds VII and "vii" finds 7;
//! - a genre: "racing" finds racing games, after the games with the word in their name;
//! - small typos, when nothing else matches: "witchr" finds The Witcher. The longer the word,
//!   the more typos it may have.
//!
//! A [`score`] ranks the games: a title that is what was typed, or starts with it, comes
//! first, then the titles that have each word, then the near spellings.

/// The words a title may leave out of its initials: "tlou2" is The Last of Us Part II.
const SKIPPED: [&str; 6] = ["the", "a", "an", "of", "and", "part"];

const ROMAN: [&str; 20] = [
    "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv", "xvi", "xvii",
    "xviii", "xix", "xx",
];
const ARABIC: [&str; 20] =
    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20"];

/// How well one typed word matches a game, best last.
const TYPO: u32 = 1;
const GENRE: u32 = 2;
/// The whole search inside the title: "borne" in Bloodborne.
const INSIDE: u32 = 3;
/// The start of a word of the title ids, the publisher, the region or the version.
const OTHER: u32 = 4;
/// Two-letter initials: "er" for Elden Ring.
const SHORT_INITIALS: u32 = 5;
/// The start of a word of the title, or of its words typed together.
const START: u32 = 6;
/// A whole word of the title, or initials of three letters or more.
const WHOLE: u32 = 7;

/// How far the title leads, times this, comes before how well the words match.
const LEAD_STEP: u32 = 8;

/// `text` in lower case, without accents, with an apostrophe joining its word ("Assassin's" is
/// "assassins"), "&" as "and", and every other character that isn't a letter or a digit as a
/// space.
pub(crate) fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            '\'' | '\u{2019}' | '\u{0300}'..='\u{036f}' => {}
            '&' => folded.push_str(" and "),
            c if c.is_ascii_alphanumeric() => folded.push(c),
            c => match plain(c) {
                Some(letters) => folded.push_str(letters),
                None if c.is_alphanumeric() => folded.push(c),
                None => folded.push(' '),
            },
        }
    }
    folded
}

/// The plain letters of a lower-case letter with an accent, or of a joined one.
fn plain(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

/// The words of a folded text.
fn words_of(text: &str) -> Vec<String> {
    fold(text).split_whitespace().map(str::to_owned).collect()
}

/// The same number the other way: "7" for "vii", "vii" for "7". Up to 20.
fn other_number(word: &str) -> Option<&'static str> {
    match ROMAN.iter().position(|r| *r == word) {
        Some(i) => Some(ARABIC[i]),
        None => ARABIC.iter().position(|a| *a == word).map(|i| ROMAN[i]),
    }
}

/// The two words are the same, or the same number.
fn same(a: &str, b: &str) -> bool {
    a == b || other_number(a) == Some(b)
}

/// What the player typed, ready to match.
#[derive(Clone, Debug)]
pub(crate) struct Query {
    words: Vec<String>,
    /// The words typed together.
    joined: String,
    /// The words, with one space between them.
    phrase: String,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let words = words_of(text);
        Self { joined: words.concat(), phrase: words.join(" "), words }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// A title, ready to match.
#[derive(Clone, Debug)]
pub(crate) struct Name {
    words: Vec<String>,
    /// The words typed together, and where each word starts in it.
    joined: String,
    starts: Vec<usize>,
    /// The words typed together, each number from 1 to 20 in digits: "finalfantasy7".
    joined_digits: String,
    digit_starts: Vec<usize>,
    phrase: String,
    /// Where the title may be said to start: its first word, and the one after "The".
    leads: Vec<usize>,
}

impl Name {
    pub fn new(title: &str) -> Self {
        let words = words_of(title);
        let joined_at = |words: &mut dyn Iterator<Item = String>| {
            let (mut joined, mut starts) = (String::new(), Vec::new());
            for word in words {
                starts.push(joined.len());
                joined.push_str(&word);
            }
            (joined, starts)
        };
        let (joined, starts) = joined_at(&mut words.iter().cloned());
        let digits = words.iter().map(|w| match other_number(w) {
            Some(n) if ROMAN.contains(&w.as_str()) => n.to_owned(),
            _ => w.clone(),
        });
        let (joined_digits, digit_starts) = joined_at(&mut digits.into_iter());
        let leads = [Some(0), (words.len() > 1 && words[0] == "the").then_some(1)].into_iter().flatten().collect();
        Self { phrase: words.join(" "), joined, starts, joined_digits, digit_starts, leads, words }
    }

    /// [`Name::new`] of `title`, kept: each key of a search reads every title again. The names
    /// are the thread's own, so a search takes no lock.
    pub fn of(title: &str) -> std::rc::Rc<Name> {
        /// More titles than any catalog has; past it the names start over.
        const MOST: usize = 65_536;
        thread_local! {
            static NAMES: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<Name>>> = Default::default();
        }
        NAMES.with_borrow_mut(|names| {
            if let Some(name) = names.get(title) {
                return name.clone();
            }
            if names.len() >= MOST {
                names.clear();
            }
            let name = std::rc::Rc::new(Name::new(title));
            names.insert(title.to_owned(), name.clone());
            name
        })
    }

    /// 2: the title is what was typed; 1: it starts with it, or with its initials; 0: neither.
    fn lead(&self, q: &Query) -> u32 {
        let mut best = 0;
        for &at in &self.leads {
            let words = &self.words[at..];
            let joined = &self.joined[self.starts[at]..];
            let joined_digits = &self.joined_digits[self.digit_starts[at]..];
            let all_same = words.len() == q.words.len() && words.iter().zip(&q.words).all(|(a, b)| same(a, b));
            if all_same || joined == q.joined || joined_digits == q.joined {
                return 2;
            }
            let Some((last, before)) = q.words.split_last() else { continue };
            let along = q.words.len() <= words.len()
                && words.iter().zip(before).all(|(a, b)| same(a, b))
                && (same(&words[before.len()], last) || words[before.len()].starts_with(last.as_str()));
            let initials = q.joined.len() >= 3 && initials(&q.joined, words);
            if along || joined.starts_with(&q.joined) || joined_digits.starts_with(&q.joined) || initials {
                best = 1;
            }
        }
        best
    }

    /// How well the typed word `w` matches the title; 0 for not at all.
    fn quality(&self, w: &str) -> u32 {
        if self.words.iter().any(|x| same(x, w)) {
            return WHOLE;
        }
        let long = w.len() >= 3;
        let initials = w.len() >= 2 && (0..self.words.len()).any(|i| initials(w, &self.words[i..]));
        if initials && long {
            return WHOLE;
        }
        let starts = |joined: &str, starts: &[usize]| starts.iter().any(|&s| joined[s..].starts_with(w));
        if starts(&self.joined, &self.starts) || starts(&self.joined_digits, &self.digit_starts) {
            return START;
        }
        if initials { SHORT_INITIALS } else { 0 }
    }

    /// The word of the title `w` is a near spelling of the start of: one typo in a word of 4
    /// to 7 letters, two in a longer one. The first letter must be right, and a word with a
    /// digit has none.
    fn near(&self, w: &str) -> bool {
        let typed: Vec<char> = w.chars().collect();
        let most = match typed.len() {
            0..=3 => return false,
            4..=7 => 1,
            8..=LONGEST_TYPO => 2,
            _ => return false,
        };
        if typed.iter().any(char::is_ascii_digit) {
            return false;
        }
        self.words.iter().any(|word| word.starts_with(typed[0]) && prefix_distance(&typed, word, most) <= most)
    }
}

/// `q` is the initials of `words` from the first: a letter a word, a number whole (either way),
/// and "the", "of" and the like may be left out.
fn initials(q: &str, words: &[String]) -> bool {
    if q.is_empty() {
        return true;
    }
    let Some((first, rest)) = words.split_first() else { return false };
    let skip = || SKIPPED.contains(&first.as_str()) && initials(q, rest);
    let number = first.chars().all(|c| c.is_ascii_digit()) || ROMAN.contains(&first.as_str());
    if number {
        let keys = std::iter::once(first.as_str()).chain(other_number(first));
        // A roman numeral of one letter is a word's initial too: the "v" of "V Rising".
        let letter = first.chars().next().map(|c| &first[..c.len_utf8()]).filter(|_| first.len() == 1);
        return keys.chain(letter).any(|k| q.strip_prefix(k).is_some_and(|r| initials(r, rest))) || skip();
    }
    let initial = first.chars().next().map_or("", |c| &first[..c.len_utf8()]);
    q.strip_prefix(initial).is_some_and(|r| initials(r, rest)) || skip()
}

/// The longest typed word that may have typos: the distance takes no memory but the stack.
const LONGEST_TYPO: usize = 32;

/// The edit distance from `typed` to the nearest start of `word`, a swap of two letters as one
/// edit. Past `most` (at most 2) the count stops: anything over it is `most + 1`. `typed` has
/// at most [`LONGEST_TYPO`] letters.
fn prefix_distance(typed: &[char], word: &str, most: usize) -> usize {
    const ROW: usize = LONGEST_TYPO + 3;
    let mut letters = ['\0'; ROW];
    let mut m = 0;
    for c in word.chars().take(typed.len() + most) {
        letters[m] = c;
        m += 1;
    }
    let word = &letters[..m];
    let mut before = [0usize; ROW];
    let mut last = [0usize; ROW];
    for (j, cell) in last.iter_mut().enumerate().take(m + 1) {
        *cell = j;
    }
    for i in 1..=typed.len() {
        let mut row = [i; ROW];
        for j in 1..=m {
            let cost = usize::from(typed[i - 1] != word[j - 1]);
            row[j] = (last[j] + 1).min(row[j - 1] + 1).min(last[j - 1] + cost);
            if i > 1 && j > 1 && typed[i - 1] == word[j - 2] && typed[i - 2] == word[j - 1] {
                row[j] = row[j].min(before[j - 2] + 1);
            }
        }
        if row[..=m].iter().all(|d| *d > most) {
            return most + 1;
        }
        before = last;
        last = row;
    }
    last[..=m].iter().copied().min().unwrap_or(typed.len())
}

/// A game's other fields: its title ids, publisher, region and version. Their words are read
/// only when a typed word matches no title.
#[derive(Debug, Default)]
pub(crate) struct Other<'a> {
    fields: Vec<&'a str>,
    words: std::cell::OnceCell<Vec<String>>,
}

impl<'a> Other<'a> {
    pub fn new(fields: impl IntoIterator<Item = &'a str>) -> Self {
        Self { fields: fields.into_iter().collect(), words: std::cell::OnceCell::new() }
    }

    fn starts(&self, w: &str) -> bool {
        let words = self.words.get_or_init(|| self.fields.iter().flat_map(|f| words_of(f)).collect());
        words.iter().any(|x| x.starts_with(w))
    }
}

/// "bf1" as "bf" and "1": the letters and the digits typed together.
fn split_digits(w: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut digit: Option<bool> = None;
    for (i, c) in w.char_indices() {
        let now = c.is_ascii_digit();
        if digit.is_some_and(|d| d != now) {
            parts.push(&w[start..i]);
            start = i;
        }
        digit = Some(now);
    }
    parts.push(&w[start..]);
    parts
}

/// How well a game matches `q`, higher is better; `None` when it doesn't. `names` are its
/// titles, `other` its other fields, and `genres` its genres, read only when a word matches
/// nothing else. With `typos`, a word that matches nothing may be a near spelling of a word of
/// a title. Every word must match; the whole search inside a title matches too.
pub(crate) fn score(
    q: &Query,
    names: &[&Name],
    other: &Other<'_>,
    genres: &dyn Fn() -> Vec<String>,
    typos: bool,
) -> Option<u32> {
    if q.is_empty() {
        return Some(0);
    }
    let lead = names.iter().map(|n| n.lead(q)).max().unwrap_or(0);
    let inside = names.iter().any(|n| n.phrase.contains(&q.phrase));
    let mut least = WHOLE;
    for w in &q.words {
        let quality = word_quality(w, names, other, genres, typos);
        if quality == 0 && lead == 0 && !inside {
            return None;
        }
        least = least.min(if quality == 0 { INSIDE } else { quality });
    }
    Some(lead * LEAD_STEP + least)
}

fn word_quality(w: &str, names: &[&Name], other: &Other<'_>, genres: &dyn Fn() -> Vec<String>, typos: bool) -> u32 {
    let best = names.iter().map(|n| n.quality(w)).max().unwrap_or(0);
    if best > 0 {
        return best;
    }
    // Letters and a number typed together, each the start of a word: "tekken8", "bf1".
    let parts = split_digits(w);
    if parts.len() > 1 && parts.iter().all(|p| names.iter().any(|n| n.quality(p) >= START)) {
        return START;
    }
    if other.starts(w) {
        return OTHER;
    }
    let genre = |g: &String| words_of(g).iter().any(|x| x == w || (w.len() >= 4 && x.starts_with(w)));
    if w.len() >= 3 && genres().iter().any(genre) {
        return GENRE;
    }
    if typos && names.iter().any(|n| n.near(w)) {
        return TYPO;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Titles to search, with their genres: look-alikes on purpose.
    const GAMES: [(&str, &str); 24] = [
        ("God of War Ragnarök", "Action"),
        ("God of War III Remastered", "Action"),
        ("Godfall", "Action"),
        ("The Last of Us Part I", "Action, Adventure"),
        ("The Last of Us Part II Remastered", "Action, Adventure"),
        ("Resident Evil 4", "Horror"),
        ("Resident Evil Village", "Horror"),
        ("Final Fantasy VII Rebirth", "RPG"),
        ("Crisis Core: Final Fantasy VII Reunion", "RPG"),
        ("Final Fantasy XVI", "RPG"),
        ("Grand Theft Auto V", "Action"),
        ("Gran Turismo 7", "Racing"),
        ("Need for Speed Unbound", "Racing"),
        ("Marvel's Spider-Man 2", "Action"),
        ("The Witcher 3: Wild Hunt", "RPG"),
        ("Assassin's Creed Mirage", "Action"),
        ("Pokémon Legends: Arceus", "RPG"),
        ("Ōkami HD", "Adventure"),
        ("Call of Duty: Black Ops 6", "Shooter"),
        ("Code Vein", "RPG"),
        ("Elden Ring", "RPG"),
        ("TEKKEN 8", "Fighting"),
        ("Bloodborne", "Action"),
        ("Ratchet & Clank: Rift Apart", "Action"),
    ];

    /// The titles that match `text`, best first; equal ones in the table's order.
    fn find(text: &str) -> Vec<&'static str> {
        let q = Query::new(text);
        let names: Vec<Name> = GAMES.iter().map(|(title, _)| Name::new(title)).collect();
        let other = Other::default();
        let run = |typos: bool| -> Vec<(usize, u32)> {
            GAMES
                .iter()
                .enumerate()
                .filter_map(|(i, (_, genres))| {
                    let genres = || genres.split(", ").map(str::to_owned).collect();
                    score(&q, &[&names[i]], &other, &genres, typos).map(|s| (i, s))
                })
                .collect()
        };
        let mut found = run(false);
        if found.is_empty() {
            found = run(true);
        }
        found.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
        found.into_iter().map(|(i, _)| GAMES[i].0).collect()
    }

    fn first(text: &str) -> &'static str {
        find(text).first().copied().unwrap_or("(nothing)")
    }

    #[test]
    fn the_best_match_comes_first_for_each_way_of_typing() {
        let table = [
            // Typos.
            ("witchr", "The Witcher 3: Wild Hunt"),
            ("assasins creed", "Assassin's Creed Mirage"),
            ("resident evli", "Resident Evil 4"),
            ("rachet", "Ratchet & Clank: Rift Apart"),
            ("bloodbrone", "Bloodborne"),
            // Words typed together.
            ("godofwar", "God of War Ragnarök"),
            ("spiderman", "Marvel's Spider-Man 2"),
            ("witcher3", "The Witcher 3: Wild Hunt"),
            ("tekken8", "TEKKEN 8"),
            ("lastofus", "The Last of Us Part I"),
            // Initials, and initials with a number.
            ("tlou", "The Last of Us Part I"),
            ("tlou2", "The Last of Us Part II Remastered"),
            ("gow", "God of War Ragnarök"),
            ("re4", "Resident Evil 4"),
            ("ff7", "Final Fantasy VII Rebirth"),
            ("gta5", "Grand Theft Auto V"),
            ("gt7", "Gran Turismo 7"),
            ("nfs", "Need for Speed Unbound"),
            ("cod", "Call of Duty: Black Ops 6"),
            ("er", "Elden Ring"),
            // Numbers and roman numerals.
            ("final fantasy 7", "Final Fantasy VII Rebirth"),
            ("final fantasy 16", "Final Fantasy XVI"),
            ("god of war 3", "God of War III Remastered"),
            ("gran turismo vii", "Gran Turismo 7"),
            ("gta v", "Grand Theft Auto V"),
            // Accents and punctuation.
            ("pokemon", "Pokémon Legends: Arceus"),
            ("okami", "Ōkami HD"),
            ("assassins", "Assassin's Creed Mirage"),
            ("ratchet and clank", "Ratchet & Clank: Rift Apart"),
            ("spider-man", "Marvel's Spider-Man 2"),
            // A title that is what was typed, or starts with it, before the others.
            ("god", "God of War Ragnarök"),
            ("god of w", "God of War Ragnarök"),
            ("code", "Code Vein"),
            ("resident evil 4", "Resident Evil 4"),
        ];
        let wrong: Vec<String> = table
            .iter()
            .filter(|(typed, top)| first(typed) != *top)
            .map(|(typed, top)| format!("{typed:?}: {top:?} expected, got {:?}", find(typed)))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn a_title_that_starts_with_the_search_comes_before_one_that_has_it_later() {
        assert_eq!(find("final fantasy 7"), ["Final Fantasy VII Rebirth", "Crisis Core: Final Fantasy VII Reunion"]);
        assert_eq!(find("god of war"), ["God of War Ragnarök", "God of War III Remastered"]);
        assert_eq!(find("cod")[..2], ["Call of Duty: Black Ops 6", "Code Vein"], "initials, then a start");
    }

    #[test]
    fn a_genre_finds_its_games_after_the_titles_with_the_word() {
        assert_eq!(find("racing"), ["Gran Turismo 7", "Need for Speed Unbound"]);
        assert!(find("rpg").contains(&"Elden Ring"));
        assert!(!find("ac").contains(&"Bloodborne"), "two letters are too few for a genre");
        assert!(!find("act").contains(&"Bloodborne"), "three letters name a genre whole only");
    }

    #[test]
    fn typos_count_only_when_nothing_matches_and_never_in_the_first_letter() {
        assert_eq!(find("evil"), ["Resident Evil 4", "Resident Evil Village"], "no near spellings");
        assert!(find("stro").is_empty(), "a missing first letter is not a typo");
        assert!(find("gdo").is_empty(), "three letters have no typos");
        assert!(find("zzzz qqqq").is_empty());
    }

    #[test]
    fn every_word_must_match_or_the_whole_search_be_inside_a_title() {
        assert!(find("creed ring").is_empty());
        assert_eq!(find("borne"), ["Bloodborne"]);
        assert_eq!(find("of war ragn"), ["God of War Ragnarök"]);
    }

    #[test]
    fn folding_drops_accents_and_case_and_keeps_letters_of_other_scripts() {
        assert_eq!(fold("Pokémon: Ōkami & Œuvre"), "pokemon  okami  and  oeuvre");
        assert_eq!(fold("Assassin’s"), "assassins");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("ゼルダ"), "ゼルダ");
    }

    #[test]
    fn the_helpers() {
        assert_eq!(split_digits("nba2k25"), ["nba", "2", "k", "25"]);
        assert_eq!((other_number("vii"), other_number("16")), (Some("7"), Some("xvi")));
        assert_eq!(other_number("21"), None);
        let typed = |t: &str| t.chars().collect::<Vec<_>>();
        assert_eq!(prefix_distance(&typed("assasin"), "assassins", 2), 1);
        assert_eq!(prefix_distance(&typed("evli"), "evil", 1), 1, "a swap is one edit");
        assert_eq!(prefix_distance(&typed("zzzzzz"), "evil", 1), 2, "it stops past the most");
    }
}

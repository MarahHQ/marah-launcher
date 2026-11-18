//! The search of the Library and the Catalog: how well a game answers what the player typed.
//!
//! On a controller each letter takes a few presses, so the search forgives the way a title is
//! typed:
//! - case, accents and punctuation don't count: "pokemon" finds Pokémon, and "spider man",
//!   "spider-man" and "spiderman" find Spider-Man;
//! - words typed together: "godofwar" finds God of War;
//! - a title's initials, a number whole among them: "tlou", "gow", "re4", "ff7";
//! - a number as digits or as a roman numeral, up to 20: "7" finds VII, "vii" finds 7;
//! - a genre: "racing" finds racing games, after the titles that have the word;
//! - small typos, when nothing matches as typed: "witchr" finds The Witcher.
//!
//! A [`Rank`] orders the games that match: first by how the title leads ([`Lead`]), then by the
//! weakest match among the typed words ([`Hit`]).

/// Words a title's initials may leave out: "tlou" is The Last of Us, "gow" God of War.
const SMALL_WORDS: [&str; 6] = ["the", "a", "an", "of", "and", "part"];

/// The largest number a roman numeral stands for here: games count their sequels that far.
const LARGEST_NUMERAL: u32 = 20;

/// The longest typed word whose typos are looked for: longer ones are kept as typed.
const LONGEST_NEAR: usize = 32;

/// How a game's title leads with what was typed, worst first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lead {
    /// Somewhere else, or not at all.
    #[default]
    Elsewhere,
    /// The title starts with it, typed together or as initials ("God of War" for "god o", "gow").
    Starts,
    /// The title is what was typed.
    Is,
}

/// How well one typed word matches a game, worst first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Hit {
    /// A near spelling of a word of the title.
    Typo,
    /// One of the game's genres.
    Genre,
    /// Only the whole search, inside the title: "borne" in Bloodborne.
    Inside,
    /// The start of a word of the title ids, the publisher, the region or the version.
    Field,
    /// Two letters that are initials: "er" for Elden Ring.
    ShortInitials,
    /// The start of a word of the title, or of its words typed together.
    Start,
    /// A word of the title, the same number, or initials of three letters or more.
    #[default]
    Whole,
}

/// How well a game answers a search: by its [`Lead`], then by its weakest [`Hit`]. A greater
/// rank comes first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Rank {
    pub lead: Lead,
    pub weakest: Hit,
}

/// `text` as the search reads it: in lower case, without accents, an apostrophe joining its
/// word ("Assassin's" is "assassins"), "&" as "and", and a space for any other character that
/// is no letter or digit.
pub(crate) fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if c == '\'' || c == '\u{2019}' || ('\u{0300}'..='\u{036f}').contains(&c) {
            // An apostrophe, or an accent written as its own mark.
        } else if c == '&' {
            out.push_str(" and ");
        } else if let Some(letters) = unaccented(c) {
            out.push_str(letters);
        } else {
            out.push(if c.is_alphanumeric() { c } else { ' ' });
        }
    }
    out
}

/// The plain letters of a lower-case Latin letter with an accent, or of a joined one.
fn unaccented(c: char) -> Option<&'static str> {
    const TABLE: [(&str, &str); 25] = [
        ("àáâãäåāăą", "a"),
        ("æ", "ae"),
        ("çćĉċč", "c"),
        ("ďđð", "d"),
        ("èéêëēĕėęě", "e"),
        ("ĝğġģ", "g"),
        ("ĥħ", "h"),
        ("ìíîïĩīĭįı", "i"),
        ("ĵ", "j"),
        ("ķ", "k"),
        ("ĺļľŀł", "l"),
        ("ñńņň", "n"),
        ("òóôõöøōŏő", "o"),
        ("œ", "oe"),
        ("ŕŗř", "r"),
        ("śŝşš", "s"),
        ("ß", "ss"),
        ("ţťŧ", "t"),
        ("þ", "th"),
        ("ùúûüũūŭůűų", "u"),
        ("ŵ", "w"),
        ("ýÿŷ", "y"),
        ("źżž", "z"),
        ("ĳ", "ij"),
        ("ŉ", "n"),
    ];
    TABLE.iter().find(|(accented, _)| accented.contains(c)).map(|(_, plain)| *plain)
}

/// The value of a roman numeral from 1 to [`LARGEST_NUMERAL`], written the usual way.
fn roman_value(word: &str) -> Option<u32> {
    if word.is_empty() || !word.chars().all(|c| matches!(c, 'i' | 'v' | 'x')) {
        return None;
    }
    let digit = |c: char| match c {
        'i' => 1,
        'v' => 5,
        _ => 10,
    };
    let values: Vec<i64> = word.chars().map(digit).collect();
    let mut total = 0;
    for (at, value) in values.iter().enumerate() {
        match values.get(at + 1) {
            Some(next) if next > value => total -= value,
            _ => total += value,
        }
    }
    let total = u32::try_from(total).ok().filter(|n| (1..=LARGEST_NUMERAL).contains(n))?;
    // "iiii" or "vv" add up too, but no title writes them.
    (roman(total) == word).then_some(total)
}

/// `n` as a roman numeral.
fn roman(mut n: u32) -> String {
    let mut out = String::new();
    for (value, letters) in [(10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")] {
        while n >= value {
            out.push_str(letters);
            n -= value;
        }
    }
    out
}

/// The number a word stands for, written in digits or as a roman numeral.
fn number_of(word: &str) -> Option<u32> {
    if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
        return word.parse().ok();
    }
    roman_value(word)
}

/// A word's other way of writing its number, up to [`LARGEST_NUMERAL`]: "7" for "vii", "vii"
/// for "7".
fn other_way(word: &str) -> Option<String> {
    let n = number_of(word).filter(|n| (1..=LARGEST_NUMERAL).contains(n))?;
    Some(if roman_value(word).is_some() { n.to_string() } else { roman(n) })
}

/// One word of a title: as written, and its number in digits when it is a roman numeral.
#[derive(Clone, Debug)]
struct Word {
    text: String,
    /// The roman numeral's number, in digits; else the word.
    digits: String,
}

impl Word {
    fn new(text: String) -> Self {
        let digits = roman_value(&text).map_or_else(|| text.clone(), |n| n.to_string());
        Self { text, digits }
    }

    /// The same word, or the same number.
    fn is(&self, typed: &str) -> bool {
        self.text == typed || self.digits == typed || other_way(typed).is_some_and(|o| o == self.text)
    }

    /// The forms the word may take in a title's initials: its first letter, a number whole in
    /// either way, and nothing for a small word.
    fn initials(&self) -> Vec<&str> {
        let first = self.text.chars().next().map_or("", |c| &self.text[..c.len_utf8()]);
        let mut forms = Vec::with_capacity(4);
        if number_of(&self.text).is_some() {
            forms.push(self.text.as_str());
            forms.push(self.digits.as_str());
        }
        forms.push(first);
        if SMALL_WORDS.contains(&self.text.as_str()) {
            forms.push("");
        }
        forms
    }
}

/// Whether `typed` is the initials of some first words of `words`. Each word takes one of its
/// [`Word::initials`]; the places in `typed` the words so far reach are kept as a set.
fn are_initials(typed: &str, words: &[Word]) -> bool {
    let mut reached = vec![0usize];
    for word in words {
        let mut next: Vec<usize> = Vec::new();
        for at in &reached {
            for form in word.initials() {
                let end = at + form.len();
                if typed[*at..].starts_with(form) && !next.contains(&end) {
                    next.push(end);
                }
            }
        }
        if next.contains(&typed.len()) {
            return true;
        }
        if next.is_empty() {
            return false;
        }
        reached = next;
    }
    false
}

/// Whether `typed` is within `budget` edits of a start of `word`: a letter changed, added,
/// left out, or two letters swapped. Where the letters differ each edit is tried, so the cost
/// stays small for the budgets used (two at most).
fn near_start(typed: &[char], word: &[char], budget: usize) -> bool {
    match (typed, word) {
        ([], _) => true,
        (rest, []) => rest.len() <= budget,
        ([a, typed_rest @ ..], [b, word_rest @ ..]) => {
            if a == b && near_start(typed_rest, word_rest, budget) {
                return true;
            }
            if budget == 0 {
                return false;
            }
            let swapped = matches!((typed_rest.first(), word_rest.first()), (Some(x), Some(y)) if x == b && y == a);
            near_start(typed_rest, word_rest, budget - 1)
                || near_start(typed_rest, word, budget - 1)
                || near_start(typed, word_rest, budget - 1)
                || (swapped && near_start(&typed_rest[1..], &word_rest[1..], budget - 1))
        }
    }
}

/// What the player typed, ready to match.
#[derive(Clone, Debug)]
pub(crate) struct Query {
    words: Vec<String>,
    /// The words typed together.
    joined: String,
    /// The words, one space between them.
    phrase: String,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let words: Vec<String> = fold(text).split_whitespace().map(str::to_owned).collect();
        Self { joined: words.concat(), phrase: words.join(" "), words }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// A title, ready to match.
#[derive(Clone, Debug)]
pub(crate) struct Name {
    words: Vec<Word>,
    phrase: String,
    /// The words typed together from each word on, as written and with numbers in digits.
    tails: Vec<(String, String)>,
    /// Where the title may be said to start: its first word, and the one after "The".
    starts: Vec<usize>,
}

impl Name {
    pub fn new(title: &str) -> Self {
        let words: Vec<Word> = fold(title).split_whitespace().map(|w| Word::new(w.to_owned())).collect();
        let tails = (0..words.len())
            .map(|from| {
                let rest = &words[from..];
                (rest.iter().map(|w| w.text.as_str()).collect(), rest.iter().map(|w| w.digits.as_str()).collect())
            })
            .collect();
        let starts = match words.first() {
            None => Vec::new(),
            Some(first) if first.text == "the" && words.len() > 1 => vec![0, 1],
            Some(_) => vec![0],
        };
        let phrase = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        Self { words, phrase, tails, starts }
    }

    /// [`Name::new`] of `title`, kept: each key of a search reads every title again. The names
    /// are the thread's own, so a search takes no lock.
    pub fn of(title: &str) -> std::rc::Rc<Name> {
        /// More titles than any catalog has; past it the names start over.
        const MOST: usize = 65_536;
        thread_local! {
            static NAMES: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<Name>>> =
                Default::default();
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

    /// How the title leads with `q`, a search with words.
    fn lead(&self, q: &Query) -> Lead {
        let Some((last, whole)) = q.words.split_last() else { return Lead::Elsewhere };
        let mut lead = Lead::Elsewhere;
        for &from in &self.starts {
            let words = &self.words[from..];
            let (joined, joined_digits) = &self.tails[from];
            let same_words = words.len() == q.words.len() && words.iter().zip(&q.words).all(|(w, t)| w.is(t));
            if same_words || *joined == q.joined || *joined_digits == q.joined {
                return Lead::Is;
            }
            // Every typed word in order, the last one maybe unfinished.
            let in_order = whole.len() < words.len()
                && words.iter().zip(whole).all(|(w, t)| w.is(t))
                && (words[whole.len()].is(last) || words[whole.len()].text.starts_with(last.as_str()));
            let together = joined.starts_with(&q.joined) || joined_digits.starts_with(&q.joined);
            let initials = q.joined.chars().count() >= 3 && are_initials(&q.joined, words);
            if in_order || together || initials {
                lead = Lead::Starts;
            }
        }
        lead
    }

    /// How well the typed word `typed` matches the title, if at all.
    fn hit(&self, typed: &str) -> Option<Hit> {
        if self.words.iter().any(|w| w.is(typed)) {
            return Some(Hit::Whole);
        }
        let letters = typed.chars().count();
        let initials = letters >= 2 && (0..self.words.len()).any(|from| are_initials(typed, &self.words[from..]));
        if initials && letters >= 3 {
            return Some(Hit::Whole);
        }
        if self.tails.iter().any(|(joined, digits)| joined.starts_with(typed) || digits.starts_with(typed)) {
            return Some(Hit::Start);
        }
        initials.then_some(Hit::ShortInitials)
    }

    /// Whether `typed` is a near spelling of the start of a word of the title: one edit in a
    /// word of four to seven letters, two in a longer one. The first letter must be right, and
    /// a word with a digit has none.
    fn near(&self, typed: &str) -> bool {
        let typed: Vec<char> = typed.chars().collect();
        let budget = match typed.len() {
            4..=7 => 1,
            8..=LONGEST_NEAR => 2,
            _ => return false,
        };
        if typed.iter().any(char::is_ascii_digit) {
            return false;
        }
        self.words.iter().any(|w| {
            let word: Vec<char> = w.text.chars().collect();
            word.first() == typed.first() && near_start(&typed[1..], &word[1..], budget)
        })
    }
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

    /// A word of the fields starts with `typed`.
    fn has(&self, typed: &str) -> bool {
        let words = self.words.get_or_init(|| {
            self.fields.iter().flat_map(|f| fold(f).split_whitespace().map(str::to_owned).collect::<Vec<_>>()).collect()
        });
        words.iter().any(|w| w.starts_with(typed))
    }
}

/// `typed` cut where letters turn to digits and back: "bf1" is "bf" and "1".
fn letter_and_digit_runs(typed: &str) -> Vec<&str> {
    let mut runs = Vec::new();
    let mut rest = typed;
    while let Some(first) = rest.chars().next() {
        let digit = first.is_ascii_digit();
        let end = rest.find(|c: char| c.is_ascii_digit() != digit).unwrap_or(rest.len());
        runs.push(&rest[..end]);
        rest = &rest[end..];
    }
    runs
}

/// How well a game answers `q`, or `None` when it doesn't. `names` are its titles, `other` its
/// other fields, and `genres` its genres, read only when a word matches nothing else. With
/// `near`, a word that matches nothing may be a near spelling of a word of a title. Every word
/// must match, unless the whole search is inside a title.
pub(crate) fn score(
    q: &Query,
    names: &[&Name],
    other: &Other<'_>,
    genres: &dyn Fn() -> Vec<String>,
    near: bool,
) -> Option<Rank> {
    if q.is_empty() {
        return Some(Rank::default());
    }
    let lead = names.iter().map(|n| n.lead(q)).max().unwrap_or_default();
    let inside = names.iter().any(|n| n.phrase.contains(&q.phrase));
    let mut weakest = Hit::Whole;
    for typed in &q.words {
        let hit = match word_hit(typed, names, other, genres, near) {
            Some(hit) => hit,
            None if lead > Lead::Elsewhere || inside => Hit::Inside,
            None => return None,
        };
        weakest = weakest.min(hit);
    }
    Some(Rank { lead, weakest })
}

fn word_hit(
    typed: &str,
    names: &[&Name],
    other: &Other<'_>,
    genres: &dyn Fn() -> Vec<String>,
    near: bool,
) -> Option<Hit> {
    if let Some(hit) = names.iter().filter_map(|n| n.hit(typed)).max() {
        return Some(hit);
    }
    // Letters and a number typed together, each the start of a word: "tekken8".
    let runs = letter_and_digit_runs(typed);
    let starts = |run: &&str| names.iter().any(|n| n.hit(run).is_some_and(|h| h >= Hit::Start));
    if runs.len() > 1 && runs.iter().all(starts) {
        return Some(Hit::Start);
    }
    if other.has(typed) {
        return Some(Hit::Field);
    }
    // A genre's whole word, or the start of one from four letters on.
    let letters = typed.chars().count();
    let names_genre =
        |genre: &String| fold(genre).split_whitespace().any(|w| w == typed || (letters >= 4 && w.starts_with(typed)));
    if letters >= 3 && genres().iter().any(names_genre) {
        return Some(Hit::Genre);
    }
    (near && names.iter().any(|n| n.near(typed))).then_some(Hit::Typo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Titles to search, and their genres. Several look alike on purpose.
    const SHELF: [(&str, &str); 26] = [
        ("God of War", "Action"),
        ("God of War III", "Action"),
        ("Godzilla Unleashed", "Fighting"),
        ("The Last of Us Part I", "Adventure"),
        ("The Last of Us Part II", "Adventure"),
        ("Resident Evil 4", "Horror"),
        ("Resident Evil Village", "Horror"),
        ("Final Fantasy VII", "RPG"),
        ("Dissidia Final Fantasy VII", "Fighting"),
        ("Final Fantasy X", "RPG"),
        ("Metal Gear Solid V", "Action"),
        ("Kingdom Hearts III", "RPG"),
        ("Red Dead Redemption 2", "Action, Adventure"),
        ("Devil May Cry 5", "Action"),
        ("Street Fighter 6", "Fighting"),
        ("Need for Speed Heat", "Racing"),
        ("Gran Turismo 7", "Racing, Simulation"),
        ("Pokémon Scarlet", "RPG"),
        ("Ōkami HD", "Adventure"),
        ("Horizon Zero Dawn", "Action, RPG"),
        ("Sekiro: Shadows Die Twice", "Action"),
        ("Monster Hunter: World", "Action"),
        ("Ratchet & Clank", "Platformer"),
        ("Marvel's Spider-Man", "Action"),
        ("Bloodborne", "Action"),
        ("Elden Ring", "RPG"),
    ];

    /// The titles that answer `typed`, best first; equal ones in the shelf's order. Near
    /// spellings only when nothing matches as typed, as the grids search.
    fn search(typed: &str) -> Vec<&'static str> {
        let q = Query::new(typed);
        let names: Vec<Name> = SHELF.iter().map(|(title, _)| Name::new(title)).collect();
        let ranked = |near: bool| -> Vec<(usize, Rank)> {
            SHELF
                .iter()
                .enumerate()
                .filter_map(|(at, (_, genres))| {
                    let genres = || genres.split(", ").map(str::to_owned).collect();
                    score(&q, &[&names[at]], &Other::default(), &genres, near).map(|rank| (at, rank))
                })
                .collect()
        };
        let mut found = ranked(false);
        if found.is_empty() {
            found = ranked(true);
        }
        found.sort_by_key(|(_, rank)| std::cmp::Reverse(*rank));
        found.into_iter().map(|(at, _)| SHELF[at].0).collect()
    }

    #[test]
    fn each_way_of_typing_puts_the_title_meant_first() {
        let table = [
            // Small typos.
            ("bloodbrone", "Bloodborne"),
            ("sekrio", "Sekiro: Shadows Die Twice"),
            ("resident evli", "Resident Evil 4"),
            ("monstr huntr", "Monster Hunter: World"),
            ("horizn", "Horizon Zero Dawn"),
            // Words typed together.
            ("godofwar", "God of War"),
            ("spiderman", "Marvel's Spider-Man"),
            ("reddead", "Red Dead Redemption 2"),
            ("streetfighter6", "Street Fighter 6"),
            ("lastofus", "The Last of Us Part I"),
            ("finalfantasy7", "Final Fantasy VII"),
            // Initials, and initials with a number.
            ("tlou", "The Last of Us Part I"),
            ("tlou2", "The Last of Us Part II"),
            ("gow", "God of War"),
            ("gow3", "God of War III"),
            ("re4", "Resident Evil 4"),
            ("ff7", "Final Fantasy VII"),
            ("ffx", "Final Fantasy X"),
            ("mgs5", "Metal Gear Solid V"),
            ("kh3", "Kingdom Hearts III"),
            ("rdr2", "Red Dead Redemption 2"),
            ("dmc5", "Devil May Cry 5"),
            ("nfs", "Need for Speed Heat"),
            ("er", "Elden Ring"),
            // Numbers and roman numerals.
            ("final fantasy 7", "Final Fantasy VII"),
            ("final fantasy 10", "Final Fantasy X"),
            ("kingdom hearts 3", "Kingdom Hearts III"),
            ("devil may cry v", "Devil May Cry 5"),
            ("gran turismo vii", "Gran Turismo 7"),
            // Accents and punctuation.
            ("pokemon", "Pokémon Scarlet"),
            ("okami", "Ōkami HD"),
            ("marvels", "Marvel's Spider-Man"),
            ("ratchet and clank", "Ratchet & Clank"),
            ("spider-man", "Marvel's Spider-Man"),
            ("sekiro shadows", "Sekiro: Shadows Die Twice"),
            // A title that is what was typed, or starts with it, before the others.
            ("god", "God of War"),
            ("god of w", "God of War"),
            ("resident evil 4", "Resident Evil 4"),
        ];
        let wrong: Vec<String> = table
            .iter()
            .filter(|(typed, meant)| search(typed).first() != Some(meant))
            .map(|(typed, meant)| format!("{typed:?}: {meant:?} first, got {:?}", search(typed)))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn a_title_that_leads_with_the_search_comes_before_one_that_has_it_later() {
        assert_eq!(search("final fantasy 7"), ["Final Fantasy VII", "Dissidia Final Fantasy VII"]);
        assert_eq!(search("god of war"), ["God of War", "God of War III"]);
        assert_eq!(search("god"), ["God of War", "God of War III", "Godzilla Unleashed"], "a whole word first");
    }

    #[test]
    fn a_genre_finds_its_games_after_the_titles_with_the_word() {
        assert_eq!(search("racing"), ["Need for Speed Heat", "Gran Turismo 7"]);
        assert!(search("rpg").contains(&"Elden Ring"));
        assert!(!search("ac").contains(&"Bloodborne"), "two letters name no genre");
        assert!(!search("act").contains(&"Bloodborne"), "three letters name a genre's whole word only");
    }

    #[test]
    fn near_spellings_count_only_when_nothing_matches_and_never_in_the_first_letter() {
        assert_eq!(search("heat"), ["Need for Speed Heat"], "not Kingdom Hearts, one edit away");
        assert_eq!(search("hert"), ["Kingdom Hearts III", "Need for Speed Heat"], "nothing as typed: near ones");
        assert!(search("jorizon").is_empty(), "a wrong first letter is no typo");
        assert!(search("gdw").is_empty(), "three letters have no typos: not God of War");
        assert!(search("qqqq zzzz").is_empty());
    }

    #[test]
    fn every_word_must_match_unless_the_whole_search_is_inside_a_title() {
        assert!(search("bloodborne ring").is_empty());
        assert_eq!(search("borne"), ["Bloodborne"]);
        assert_eq!(search("dows die tw"), ["Sekiro: Shadows Die Twice"]);
    }

    #[test]
    fn a_title_with_no_words_matches_no_search_and_does_not_fail() {
        let q = Query::new("a");
        let none = Vec::new;
        for title in ["", "?!", "   "] {
            assert_eq!(score(&q, &[&Name::new(title)], &Other::default(), &none, true), None, "{title:?}");
        }
        let empty = Query::new("!!");
        assert!(empty.is_empty(), "punctuation alone is no search");
        assert_eq!(score(&empty, &[&Name::new("")], &Other::default(), &none, false), Some(Rank::default()));
    }

    #[test]
    fn folding_drops_case_and_accents_and_keeps_other_scripts() {
        assert_eq!(fold("Ōkami & Œuvre: Straße"), "okami  and  oeuvre  strasse");
        assert_eq!(fold("Marvel’s"), "marvels");
        assert_eq!(fold("ドラゴンクエスト"), "ドラゴンクエスト");
    }

    #[test]
    fn numbers_read_both_ways_up_to_twenty() {
        let pairs = [("iv", "4"), ("ix", "9"), ("xiv", "14"), ("xix", "19"), ("xx", "20")];
        for (numeral, digits) in pairs {
            assert_eq!(other_way(numeral).as_deref(), Some(digits));
            assert_eq!(other_way(digits).as_deref(), Some(numeral));
        }
        assert_eq!(other_way("21"), None);
        assert_eq!(roman_value("iiii"), None, "not the usual way");
        assert_eq!(roman_value("mix"), None);
    }

    #[test]
    fn near_start_allows_each_kind_of_edit() {
        let chars = |t: &str| t.chars().collect::<Vec<_>>();
        let near = |typed: &str, word: &str, budget| near_start(&chars(typed), &chars(word), budget);
        assert!(near("hoizon", "horizon", 1), "a letter left out");
        assert!(near("horrizon", "horizon", 1), "a letter added");
        assert!(near("horixon", "horizon", 1), "a letter changed");
        assert!(near("hroizon", "horizon", 1), "two letters swapped");
        assert!(near("hor", "horizon", 0), "the start of the word");
        assert!(!near("hxxizon", "horizon", 1));
        assert!(near("hxxizon", "horizon", 2));
        assert_eq!(letter_and_digit_runs("rdr2x10"), ["rdr", "2", "x", "10"]);
    }
}

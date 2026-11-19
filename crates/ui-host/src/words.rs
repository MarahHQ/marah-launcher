//! The word test: the two copies of the view model allow the same words.
//!
//! Many text fields hold one of a few words: a download's state, a sort, a screen. `vm.rs` and
//! `types.slint` each list those words in a comment, and the drift test reads neither comment.
//! This test reads the list of each field in [`rows`] from both copies, and from the code where
//! the words come from, and fails when two of them differ.
//!
//! The comments write a list in a few ways: `a | b | c`, `` `a`, `b` or `c` ``, or `a, b or c`,
//! with `"a"` quotes in places. A list starts the comment, or comes after a `:` or an ` is `.
//! A word can have a note in brackets after it. `""` and `empty` are not words: they are the
//! empty text, which most fields can hold.

use launcher_api::{ActionStyle, Tone};
use launcher_core::{JobPhase, Sort, TorrentButton};
use launcher_ui_api::{FieldKind, NoticeKind, ScreenId, SlotKind, contract};
use serde::Serialize;

const VM_RS: &str = include_str!("../../ui-api/src/vm.rs");

/// Every variant of an enum. The `match` stops compiling when the enum gains a variant, so the
/// list can't fall behind the enum.
macro_rules! every {
    ($ty:ident: $($variant:ident),+ $(,)?) => {{
        let _every = |v: $ty| match v {
            $($ty::$variant)|+ => {}
        };
        [$($ty::$variant),+]
    }};
}

/// The words `serde` writes for `values`, as a theme gets them.
fn serde_words<T: Serialize>(values: impl IntoIterator<Item = T>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| match serde_json::to_value(v) {
            Ok(serde_json::Value::String(word)) => word,
            other => panic!("not a word: {other:?}"),
        })
        .collect()
}

/// Where the Rust side of a row takes its words from.
enum Rust {
    /// The doc comment of a field in `vm.rs`: `("TorrentVm", "state")`.
    Doc(&'static str, &'static str),
    /// The field's type is an enum: its words are the enum's.
    Enum(&'static str, Vec<String>),
}

/// One field whose words the copies must agree on.
struct Row {
    rust: Rust,
    /// The struct and field in `types.slint`, `Vm` for a `Vm` property.
    slint: (&'static str, &'static str),
    /// The struct whose comment above it holds the list, when it isn't the field's own struct.
    notes: Option<&'static str>,
    /// The words of the code that writes the field.
    core: Option<Vec<String>>,
}

fn doc(rust: (&'static str, &'static str), slint: (&'static str, &'static str)) -> Row {
    Row { rust: Rust::Doc(rust.0, rust.1), slint, notes: None, core: None }
}

fn of_enum(name: &'static str, words: Vec<String>, slint: (&'static str, &'static str)) -> Row {
    Row { rust: Rust::Enum(name, words), slint, notes: None, core: None }
}

impl Row {
    fn notes(self, above: &'static str) -> Row {
        Row { notes: Some(above), ..self }
    }

    fn core(self, words: Vec<String>) -> Row {
        Row { core: Some(words), ..self }
    }

    fn name(&self) -> String {
        match &self.rust {
            Rust::Doc(owner, field) => format!("{owner}.{field}"),
            Rust::Enum(name, _) => format!("{}.{} ({name})", self.slint.0, self.slint.1),
        }
    }
}

/// The fields with a list of words. A field that gets a list gets a row:
/// `every_list_of_words_in_vm_rs_has_a_row` fails until it has one, when its doc in `vm.rs`
/// reads as a list. A list only in `types.slint`, or written as prose, needs its row by hand.
fn rows() -> Vec<Row> {
    let sorts = every!(Sort: Title, Played, Size, Smallest, Rating, Release, Compat, Added);
    let phases = every!(JobPhase: Queued, Downloading, Verifying, Unpacking, Paused, Done, Failed, Cancelled);
    let buttons = every!(
        TorrentButton: Start,
        Pause,
        Play,
        Install,
        StopSeeding,
        Cancel,
        Remove,
        RemoveUnpacked,
        CancelInstall,
        OpenStore,
    );
    vec![
        of_enum(
            "SlotKind",
            serde_words(every!(SlotKind: Badge, Action, Section, Block, SettingsPanel)),
            ("Slot", "kind"),
        ),
        of_enum("Tone", serde_words(every!(Tone: Neutral, Info, Success, Warning, Danger)), ("Slot", "tone")),
        of_enum("ActionStyle", serde_words(every!(ActionStyle: Primary, Secondary)), ("Slot", "style")),
        doc(("GameActionsVm", "primary"), ("GameActions", "primary")),
        doc(("GameCardVm", "availability"), ("GameCard", "availability")),
        doc(("LibraryVm", "sort"), ("Library", "sort")).core(sorts.iter().map(|s| s.as_str().to_owned()).collect()),
        doc(("BootStepVm", "state"), ("BootStep", "state")),
        of_enum("FieldKind", serde_words(every!(FieldKind: Toggle, Number, Text, Choice)), ("Field", "kind")),
        doc(("ReleaseVm", "system"), ("Release", "system")),
        doc(("ReleaseVm", "availability"), ("Release", "availability")),
        doc(("VerdictVm", "level"), ("Verdict", "level")),
        doc(("TorrentVm", "state"), ("Torrent", "state")).notes("TorrentPick"),
        doc(("TorrentVm", "actions"), ("Torrent", "actions"))
            .notes("TorrentPick")
            .core(buttons.iter().map(|b| b.as_str().to_owned()).collect()),
        doc(("TorrentVm", "install"), ("Torrent", "install")).notes("TorrentPick"),
        doc(("TorrentVm", "unpack"), ("Torrent", "unpack")).notes("TorrentPick"),
        doc(("TorrentPickVm", "kind"), ("TorrentPick", "kind")),
        doc(("GameDetailVm", "your_result"), ("GameDetail", "your-result")),
        doc(("DownloadVm", "state"), ("Download", "state"))
            .core(phases.iter().map(|p| p.as_str().to_owned()).collect()),
        doc(("EmulatorVm", "status"), ("Emulator", "status")),
        doc(("PluginRowVm", "kind"), ("PluginRow", "kind")),
        doc(("RunningVm", "state"), ("Running", "state")),
        doc(("VpnVm", "kind"), ("VpnItem", "kind")),
        doc(("BrowsePlaceVm", "kind"), ("BrowsePlace", "kind")),
        doc(("FolderBrowserVm", "purpose"), ("FolderBrowser", "purpose")),
        doc(("LauncherUpdateVm", "state"), ("LauncherUpdate", "state")),
        doc(("LauncherUpdateVm", "method"), ("LauncherUpdate", "method")),
        doc(("SystemVm", "connection"), ("System", "connection")),
        doc(("SystemVm", "os_update"), ("System", "os-update")),
        doc(("SystemJobVm", "kind"), ("SystemJobItem", "kind")),
        doc(("SystemJobVm", "state"), ("SystemJobItem", "state")),
        of_enum("NoticeKind", serde_words(every!(NoticeKind: Info, Success, Error)), ("Notice", "kind")),
        of_enum(
            "ScreenId",
            serde_words(every!(
                ScreenId: Home,
                Library,
                GameDetail,
                Catalog,
                Downloads,
                Settings,
                Emulators,
                Themes,
                Plugin,
                GameSettings
            )),
            ("Vm", "screen"),
        ),
        doc(("AppView", "nav_direction"), ("Vm", "nav-direction")),
    ]
}

/// Fields of `vm.rs` whose doc reads as a list but whose text is not one of a few words: text
/// for the player ("Today", "Good"), and the parts of the system that `AppView.system` has.
const NOT_WORDS: [(&str, &str); 4] = [
    ("GameCardVm", "last_played"),
    ("GameCardVm", "availability_text"),
    ("GameDetailVm", "last_played"),
    ("AppView", "system"),
];

/// A word of a list: letters, digits, `-` and `_`.
fn is_word(word: &str) -> bool {
    word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The item at the start of `s`, and the text after it: `` `a` ``, `"a"`, `""` or a bare word.
fn item(s: &str) -> Option<(&str, &str)> {
    let quoted = |quote: char| {
        let inner = s.strip_prefix(quote)?;
        let end = inner.find(quote)?;
        is_word(&inner[..end]).then(|| (&inner[..end], &inner[end + 1..]))
    };
    match s.chars().next()? {
        '`' => quoted('`'),
        '"' => quoted('"'),
        c if c.is_ascii_alphabetic() => {
            let end = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or(s.len());
            Some((&s[..end], &s[end..]))
        }
        _ => None,
    }
}

/// `s` past a note in brackets at its start, ` (like this)`.
fn past_note(s: &str) -> &str {
    let Some(note) = s.strip_prefix([' ', '\n']).and_then(|s| s.strip_prefix('(')) else { return s };
    let mut depth = 1;
    for (i, c) in note.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 1 => return &note[i + 1..],
            ')' => depth -= 1,
            _ => {}
        }
    }
    s
}

/// The list of words at the start of `s`, when it has two or more.
fn list_at(s: &str) -> Option<Vec<String>> {
    let mut rest = s.trim_start();
    let mut words = Vec::new();
    while let Some((word, after)) = item(rest) {
        let after = past_note(after);
        // A list goes on over a line break after its separator; a word at the end of a line
        // ends it.
        let flat = after.replace('\n', " ");
        let sep = [" | ", ", or ", ", ", " or "].into_iter().find(|sep| flat.starts_with(sep));
        let ends = after.is_empty() || after.starts_with(['.', ';', ':', ')', '\n']);
        if sep.is_none() && !ends {
            // Prose follows: the word was the start of a sentence.
            break;
        }
        if !word.is_empty() && word != "empty" {
            words.push(word.to_owned());
        }
        match sep {
            Some(sep) => rest = &after[sep.len()..],
            None => break,
        }
    }
    (words.len() >= 2).then_some(words)
}

/// The first list of words in `text`: at its start, or after a `:` or an ` is ` outside brackets.
fn first_list(text: &str) -> Option<Vec<String>> {
    let mut starts = vec![0];
    let mut depth = 0_u32;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => starts.push(i + 1),
            ' ' if depth == 0 && (text[i..].starts_with(" is ") || text[i..].starts_with(" is\n")) => {
                starts.push(i + 4)
            }
            _ => {}
        }
    }
    starts.into_iter().find_map(|at| list_at(&text[at..]))
}

/// The list of `field` in a comment about several fields: after `` `field` `` or `field:`, in
/// the same sentence.
fn anchored_list(text: &str, field: &str) -> Option<Vec<String>> {
    let word_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    text.match_indices(field).find_map(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + field.len()..].chars().next();
        let anchor = match (before, after) {
            (Some('`'), Some('`')) => i - 1,
            (before, Some(':')) if !before.is_some_and(|c| word_char(c) || c == '`') => i,
            _ => return None,
        };
        let sentence = &text[anchor..];
        let mut depth = 0_u32;
        let end = sentence
            .char_indices()
            .find(|&(j, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                depth == 0
                    && (c == ';' || (c == '.' && sentence[j + 1..].chars().next().is_none_or(char::is_whitespace)))
            })
            .map_or(sentence.len(), |(j, _)| j);
        first_list(&sentence[..end])
    })
}

/// A comment's lines without the `//` or `///`.
fn join(lines: &[&str]) -> String {
    lines.iter().map(|l| l.trim().trim_start_matches('/').trim()).collect::<Vec<_>>().join("\n")
}

/// The doc comment of each field of `owner` in `vm.rs`, by the field's Rust name.
fn rust_docs<'s>(source: &'s str, owner: &str) -> Option<Vec<(&'s str, String)>> {
    let mut lines = source.lines().skip_while(|l| !l.starts_with(&format!("pub struct {owner} {{")));
    lines.next()?;
    let mut fields = Vec::new();
    let mut doc = Vec::new();
    for line in lines.take_while(|l| *l != "}") {
        let trimmed = line.trim();
        if trimmed.starts_with("///") {
            doc.push(trimmed);
        } else if let Some(field) = trimmed.strip_prefix("pub ").and_then(|f| f.split_once(':')) {
            fields.push((field.0, join(&doc)));
            doc.clear();
        } else if !trimmed.starts_with("#[") {
            doc.clear();
        }
    }
    Some(fields)
}

/// The comment lines right above line `at`.
fn comment_above<'s>(lines: &[&'s str], at: usize) -> Vec<&'s str> {
    let start = lines[..at].iter().rposition(|l| !l.trim().starts_with("//")).map_or(0, |i| i + 1);
    lines[start..at].to_vec()
}

/// The list of `field` of `owner` in `types.slint`: in the comment right above the field, or in
/// the comment above the struct `notes`. That comment goes up past the one-line structs above it.
fn slint_list(source: &str, owner: &str, field: &str, notes: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = source.lines().collect();
    let declares = |l: &str| {
        if owner == "Vm" {
            l.trim()
                .strip_prefix("in-out property <")
                .and_then(|p| p.split_once("> "))
                .is_some_and(|(_, rest)| rest.strip_prefix(field).is_some_and(|r| r.starts_with([';', ':'])))
        } else {
            l.split([',', '{']).any(|part| part.trim().strip_prefix(field).is_some_and(|r| r.starts_with(':')))
        }
    };
    let start = if owner == "Vm" {
        lines.iter().position(|l| l.starts_with("export global Vm {"))?
    } else {
        lines.iter().position(|l| l.starts_with(&format!("export struct {owner} ")))?
    };
    let end = start + lines[start..].iter().position(|l| l.trim_end().ends_with('}'))?;
    let own = (start + 1..end).find(|&i| declares(lines[i])).map(|i| join(&comment_above(&lines, i)));
    if let Some(words) = own.as_deref().and_then(first_list) {
        return Some(words);
    }
    let notes_at = lines.iter().position(|l| l.starts_with(&format!("export struct {notes} ")))?;
    let one_line_struct = |l: &str| l.starts_with("export struct ") && l.trim_end().ends_with('}');
    let first = lines[..notes_at]
        .iter()
        .rposition(|l| !(l.trim().starts_with("//") || one_line_struct(l)))
        .map_or(0, |i| i + 1);
    let comment: Vec<&str> = lines[first..notes_at].iter().copied().filter(|l| l.trim().starts_with("//")).collect();
    anchored_list(&join(&comment), field)
}

/// Where the copies, and the code, differ on the rows' words.
fn differences(vm: &str, slint: &str, rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for row in rows {
        let name = row.name();
        let rust = match &row.rust {
            Rust::Doc(owner, field) => rust_docs(vm, owner)
                .and_then(|fields| fields.into_iter().find(|(f, _)| f == field))
                .and_then(|(_, doc)| first_list(&doc)),
            Rust::Enum(_, words) => Some(words.clone()),
        };
        let (owner, field) = row.slint;
        let slint = slint_list(slint, owner, field, row.notes.unwrap_or(owner));
        match (&rust, &slint) {
            (None, _) => out.push(format!("{name}: vm.rs has no list of words")),
            (_, None) => out.push(format!("{name}: types.slint has no list of words for {owner}.{field}")),
            (Some(rust), Some(slint)) if rust != slint => {
                out.push(format!("{name}: vm.rs says {}, types.slint says {}", rust.join(" | "), slint.join(" | ")))
            }
            _ => {}
        }
        if let (Some(core), Some(rust)) = (&row.core, &rust)
            && core != rust
        {
            out.push(format!("{name}: vm.rs says {}, the code says {}", rust.join(" | "), core.join(" | ")));
        }
    }
    out
}

#[test]
fn the_two_copies_of_the_view_model_allow_the_same_words() {
    let differences = differences(VM_RS, contract::TYPES_SLINT, &rows());
    assert!(differences.is_empty(), "the words drifted:\n{}", differences.join("\n"));
}

#[test]
fn every_list_of_words_in_vm_rs_has_a_row() {
    let rows = rows();
    let has_row = |owner: &str, field: &str| {
        rows.iter().any(|r| matches!(r.rust, Rust::Doc(o, f) if o == owner && f == field))
            || NOT_WORDS.contains(&(owner, field))
    };
    let owners = VM_RS.lines().filter_map(|l| l.strip_prefix("pub struct ")?.strip_suffix(" {"));
    let mut missing = Vec::new();
    for owner in owners {
        for (field, doc) in rust_docs(VM_RS, owner).unwrap_or_default() {
            if let Some(words) = first_list(&doc).filter(|_| !has_row(owner, field)) {
                missing.push(format!("{owner}.{field}: {}", words.join(" | ")));
            }
        }
    }
    assert!(missing.is_empty(), "lists of words with no row in rows():\n{}", missing.join("\n"));
}

fn words<const N: usize>(words: [&str; N]) -> Option<Vec<String>> {
    Some(words.iter().map(|w| (*w).to_owned()).collect())
}

#[test]
fn a_list_reads_in_each_way_the_copies_write_it() {
    assert_eq!(first_list("a | b | c"), words(["a", "b", "c"]));
    assert_eq!(first_list("Its state: `a`, `b`,\n`c` or `d`. More"), words(["a", "b", "c", "d"]));
    assert_eq!(first_list("`sort` is title, size (largest first), or added; more"), words(["title", "size", "added"]));
    assert_eq!(first_list("good (3: or more), low\n(1)"), words(["good", "low"]));
    assert_eq!(first_list("Its update: \"\" (none) | x | y"), words(["x", "y"]));
    assert_eq!(first_list("The main action: `play`, `get`, or empty for none."), words(["play", "get"]));
    assert_eq!(first_list("kind: \"Wi-Fi\", \"VPN\" or \"Network\""), words(["Wi-Fi", "VPN", "Network"]));
    assert_eq!(first_list("Today is a day, a week or a year."), None);
    assert_eq!(first_list("The values of `region`, `age` and `version`."), None);
    let notes = "A job's kind: x | y; state: p | q. `install`: a, b or c.";
    assert_eq!(anchored_list(notes, "state"), words(["p", "q"]));
    assert_eq!(anchored_list(notes, "kind"), words(["x", "y"]));
    assert_eq!(anchored_list(notes, "install"), words(["a", "b", "c"]));
    assert_eq!(anchored_list(notes, "stall"), None);
}

/// A view model and a contract that differ in a few words, in each way the test reads them.
mod drifted {
    pub const VM: &str = "pub struct JobVm {
    /// `queued`, `running` or
    /// `done`.
    pub state: String,
    /// Its kind: `a`, `b` or `c`.
    #[serde(rename = \"sort\")]
    pub kind: String,
    /// Some text.
    pub note: String,
}
";

    pub const SLINT: &str = "// state: queued | running | finished
export struct Job { state: string, sort: string, note: string }

export global Vm {
    // on | off | auto
    in-out property <string> mode;
}
";
}

#[test]
fn a_word_one_copy_or_the_code_has_and_the_other_hasnt_fails_the_word_test() {
    let core = ["queued", "running", "done", "failed"].map(String::from).to_vec();
    let rows = [
        doc(("JobVm", "state"), ("Job", "state")).core(core),
        doc(("JobVm", "kind"), ("Job", "sort")),
        doc(("JobVm", "note"), ("Job", "note")),
        of_enum("Mode", vec!["on".into(), "off".into()], ("Vm", "mode")),
    ];
    assert_eq!(
        differences(drifted::VM, drifted::SLINT, &rows),
        [
            "JobVm.state: vm.rs says queued | running | done, types.slint says queued | running | finished",
            "JobVm.state: vm.rs says queued | running | done, the code says queued | running | done | failed",
            "JobVm.kind: types.slint has no list of words for Job.sort",
            "JobVm.note: vm.rs has no list of words",
            "Vm.mode (Mode): vm.rs says on | off, types.slint says on | off | auto",
        ]
    );
}

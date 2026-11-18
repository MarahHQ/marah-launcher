//! The archive plan: what an unpack may write, decided from an archive's listing before
//! anything is written.
//!
//! Every [`crate::Archives`] adapter lists an archive with its format readers, then hands the
//! listing to [`plan_unpack`]. The plan refuses an entry that needs a password, a path that
//! leaves the folder (absolute, or `..`), a link that points out on its own or through the
//! archive's other links, a hard link to a link or through one, an entry at or under a link's
//! path, and an archive bigger than the free space. The adapter then writes only the plan's entries, so the rules exist once and
//! hold for the app's readers and the test kit's fake alike.

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use crate::ArchiveError;
use crate::ports::Escape;
use crate::unpack::{inside, link_stays_inside};

/// The free space an unpack leaves, beside the unpacked size.
pub const UNPACK_SPARE: u64 = 64 * 1024 * 1024;

/// What an archive entry is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    /// A link to its target, from the link's folder.
    Link(PathBuf),
    /// Another name for an entry, from the archive's top.
    HardLink(PathBuf),
    /// Something to leave out: a device, a pipe, or the archive's own top (`./`).
    Other,
}

/// One entry as a format reader lists it, unchecked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Its path as the archive spells it.
    pub name: String,
    pub size: u64,
    pub kind: EntryKind,
    /// It needs a password to read.
    pub encrypted: bool,
}

impl ArchiveEntry {
    /// An entry that needs no password.
    pub fn new(name: impl Into<String>, size: u64, kind: EntryKind) -> Self {
        Self { name: name.into(), size, kind, encrypted: false }
    }
}

/// One entry the plan lets through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    /// Its path as the archive spells it, for errors and for matching the write pass.
    pub name: String,
    /// Its path in the folder: relative, with only normal parts. Empty for the archive's own
    /// top, which is [`EntryKind::Other`].
    pub path: PathBuf,
    pub size: u64,
    /// For a link, the target the plan checked.
    pub kind: EntryKind,
}

/// What an unpack writes: one [`Planned`] for each listed entry, in the listing's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The folder the entries go into.
    pub dest: PathBuf,
    pub entries: Vec<Planned>,
    /// The bytes of every file.
    pub total: u64,
}

impl Plan {
    /// Where `entry` goes on the disk.
    pub fn out(&self, entry: &Planned) -> PathBuf {
        self.dest.join(&entry.path)
    }
}

/// Whether `needed` bytes fit in `free` with [`UNPACK_SPARE`] left; a disk that doesn't say has
/// room.
pub fn room(needed: u64, free: Option<u64>) -> Result<(), ArchiveError> {
    let needed = needed.saturating_add(UNPACK_SPARE);
    match free {
        Some(free) if free < needed => Err(ArchiveError::NoRoom { needed, free }),
        _ => Ok(()),
    }
}

/// Checks a listing and plans its unpack into `dest`, on a disk with `free` bytes (`None`
/// when the disk doesn't say). The first entry that breaks a rule refuses the whole archive.
pub fn plan_unpack(entries: Vec<ArchiveEntry>, dest: &Path, free: Option<u64>) -> Result<Plan, ArchiveError> {
    let mut planned = Vec::with_capacity(entries.len());
    for entry in entries {
        planned.push(check(entry)?);
    }
    check_links(&planned)?;
    let total = planned.iter().filter(|e| e.kind == EntryKind::File).map(|e| e.size).sum();
    room(total, free)?;
    Ok(Plan { dest: dest.to_path_buf(), entries: planned, total })
}

fn check(entry: ArchiveEntry) -> Result<Planned, ArchiveError> {
    let ArchiveEntry { name, size, kind, encrypted } = entry;
    if encrypted {
        return Err(ArchiveError::Password);
    }
    let escapes = |how| ArchiveError::Escapes { entry: name.clone(), how };
    let named = Path::new(&name);
    // `./`: the archive's own top, which some tars list first.
    if !name.is_empty() && named.components().all(|c| c == std::path::Component::CurDir) {
        return Ok(Planned { name, path: PathBuf::new(), size, kind: EntryKind::Other });
    }
    let path = inside(named).ok_or_else(|| escapes(Escape::Leaves))?;
    match &kind {
        EntryKind::Link(target) if !link_stays_inside(&path, target) => return Err(escapes(Escape::LinksOut)),
        EntryKind::HardLink(target) => {
            let target = inside(target).ok_or_else(|| escapes(Escape::LinksOut))?;
            return Ok(Planned { name, path, size, kind: EntryKind::HardLink(target) });
        }
        _ => {}
    }
    Ok(Planned { name, path, size, kind })
}

/// The most links the plan follows on one link's way. Linux follows 40, macOS 32.
const MOST_LINKS: usize = 40;

/// The listing's links by their path: the first link at each path, by its place in the
/// listing, and its target.
type Links<'a> = HashMap<&'a Path, (usize, &'a Path)>;

/// Checks the entries against the listing's links, whatever their order: each link must end
/// inside the folder once the links on its way are followed, no hard link may name a link or
/// a path through one, and nothing else may sit at a link's path or under it. A format reader or `unar` may write in another order, and an
/// entry under a link would be written wherever the link points.
fn check_links(planned: &[Planned]) -> Result<(), ArchiveError> {
    let mut links = Links::new();
    for (at, entry) in planned.iter().enumerate() {
        if let EntryKind::Link(target) = &entry.kind {
            links.entry(&entry.path).or_insert((at, target));
        }
    }
    let escapes = |entry: &Planned, how| ArchiveError::Escapes { entry: entry.name.clone(), how };
    for entry in planned {
        let out = match &entry.kind {
            EntryKind::Link(target) => !ends_inside(&entry.path, target, &links),
            // A hard link to a link is a second link at a new place, where its relative target
            // ends somewhere else. Through a link, the plan can't tell what it names.
            EntryKind::HardLink(target) => target.ancestors().any(|at| links.contains_key(at)),
            _ => false,
        };
        if out {
            return Err(escapes(entry, Escape::LinksOut));
        }
    }
    for (at, entry) in planned.iter().enumerate() {
        let on_a_link = links.get(entry.path.as_path()).is_some_and(|(link, _)| *link != at);
        let under_a_link = entry.path.ancestors().skip(1).any(|folder| links.contains_key(folder));
        if on_a_link || under_a_link {
            return Err(escapes(entry, Escape::ThroughLink));
        }
    }
    Ok(())
}

/// Whether a link at `link` to `target` ends inside the folder, following `links` on the way
/// as the system would. A `..` after a link climbs from where the link ends, not from where it
/// sits. A way through more than [`MOST_LINKS`] links counts as outside.
fn ends_inside(link: &Path, target: &Path, links: &Links) -> bool {
    let mut at = PathBuf::new();
    let mut left: VecDeque<Component> = link.parent().into_iter().flat_map(Path::components).collect();
    left.extend(target.components());
    let mut followed = 0;
    while let Some(part) = left.pop_front() {
        match part {
            Component::Normal(name) => {
                at.push(name);
                if let Some((_, to)) = links.get(at.as_path()) {
                    followed += 1;
                    if followed > MOST_LINKS {
                        return false;
                    }
                    at.pop();
                    for part in to.components().rev() {
                        left.push_front(part);
                    }
                }
            }
            Component::CurDir => {}
            Component::ParentDir if at.pop() => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::path::Component;

    use super::*;

    fn file(name: &str, size: u64) -> ArchiveEntry {
        ArchiveEntry::new(name, size, EntryKind::File)
    }

    fn link(name: &str, target: &str) -> ArchiveEntry {
        ArchiveEntry::new(name, 0, EntryKind::Link(target.into()))
    }

    fn hard_link(name: &str, target: &str) -> ArchiveEntry {
        ArchiveEntry::new(name, 0, EntryKind::HardLink(target.into()))
    }

    fn escapes(entry: &str, how: Escape) -> ArchiveError {
        ArchiveError::Escapes { entry: entry.into(), how }
    }

    fn dest() -> PathBuf {
        Path::new("/downloads/game/unpacked/game.part").to_path_buf()
    }

    #[test]
    fn a_listing_plans_each_entry_in_order_and_totals_the_files() {
        let listing = vec![
            ArchiveEntry::new("Game/", 0, EntryKind::Folder),
            file("Game/eboot.bin", 3),
            file("./readme.txt", 2),
            link("Game/lib.so", "lib.so.1"),
            hard_link("Game/again", "Game/eboot.bin"),
            ArchiveEntry::new("Game/fifo", 9, EntryKind::Other),
        ];
        let plan = plan_unpack(listing, &dest(), None).unwrap();
        let paths: Vec<_> = plan.entries.iter().map(|e| (e.name.as_str(), e.path.clone(), e.kind.clone())).collect();
        assert_eq!(
            paths,
            [
                ("Game/", "Game".into(), EntryKind::Folder),
                ("Game/eboot.bin", "Game/eboot.bin".into(), EntryKind::File),
                ("./readme.txt", "readme.txt".into(), EntryKind::File),
                ("Game/lib.so", "Game/lib.so".into(), EntryKind::Link("lib.so.1".into())),
                ("Game/again", "Game/again".into(), EntryKind::HardLink("Game/eboot.bin".into())),
                ("Game/fifo", "Game/fifo".into(), EntryKind::Other),
            ]
        );
        assert_eq!(plan.total, 5, "files only");
        assert_eq!(plan.out(&plan.entries[1]), dest().join("Game/eboot.bin"));
    }

    #[test]
    fn the_archive_s_own_top_is_left_out_not_refused() {
        let plan = plan_unpack(vec![ArchiveEntry::new("./", 0, EntryKind::Folder), file("a", 1)], &dest(), None);
        let plan = plan.unwrap();
        assert_eq!(plan.entries[0].kind, EntryKind::Other);
        assert_eq!(plan.entries[0].path, PathBuf::new());
        assert_eq!(plan.entries[1].path, PathBuf::from("a"));
    }

    /// Paths that leave the folder, as hostile archives spell them.
    const LEAVING: [&str; 10] =
        ["../evil", "a/../../evil", "/etc/passwd", "//etc/passwd", "a/./../..", "..", "a/b/../../..", "", "/", "../"];

    #[test]
    fn every_path_that_leaves_the_folder_is_refused() {
        for name in LEAVING {
            let refused = plan_unpack(vec![file("good.txt", 1), file(name, 1)], &dest(), None).unwrap_err();
            assert_eq!(refused, escapes(name, Escape::Leaves), "{name:?}");
        }
        // Even as a folder, a link or a hard link.
        assert_eq!(
            plan_unpack(vec![ArchiveEntry::new("../x/", 0, EntryKind::Folder)], &dest(), None).unwrap_err(),
            escapes("../x/", Escape::Leaves)
        );
        assert_eq!(plan_unpack(vec![link("/l", "a")], &dest(), None).unwrap_err(), escapes("/l", Escape::Leaves));
        assert_eq!(
            plan_unpack(vec![hard_link("../h", "a")], &dest(), None).unwrap_err(),
            escapes("../h", Escape::Leaves)
        );
    }

    #[test]
    fn a_link_that_points_out_of_the_folder_is_refused() {
        for (name, target) in [
            ("passwd", "/etc/passwd"),
            ("Game/passwd", "../../../etc/passwd"),
            ("link", "../outside"),
            ("a/link", "../../outside"),
            ("a/link", "b/../../../x"),
        ] {
            let refused = plan_unpack(vec![link(name, target)], &dest(), None).unwrap_err();
            assert_eq!(refused, escapes(name, Escape::LinksOut), "{name} -> {target}");
        }
        for (name, target) in [("a/b/link", "../c"), ("lib/libx.so", "libx.so.1"), ("a/link", "..")] {
            assert!(plan_unpack(vec![link(name, target)], &dest(), None).is_ok(), "{name} -> {target}");
        }
        // A hard link names another entry from the archive's top.
        for target in ["../etc/passwd", "/etc/passwd", ""] {
            let refused = plan_unpack(vec![hard_link("Game/again", target)], &dest(), None).unwrap_err();
            assert_eq!(refused, escapes("Game/again", Escape::LinksOut), "{target:?}");
        }
    }

    #[test]
    fn links_that_chain_out_of_the_folder_are_refused() {
        // `d/l` ends at the top, so `x` ends one folder above it.
        let chained = vec![link("d/l", ".."), link("x", "d/l/.."), file("x/evil.txt", 4)];
        assert_eq!(plan_unpack(chained, &dest(), None).unwrap_err(), escapes("x", Escape::LinksOut));
        // The order of the listing doesn't matter.
        let reversed = vec![file("x/evil.txt", 4), link("x", "d/l/.."), link("d/l", "..")];
        assert_eq!(plan_unpack(reversed, &dest(), None).unwrap_err(), escapes("x", Escape::LinksOut));
        for (name, target) in [("x", "d/l/.."), ("a/x", "../d/l/../.."), ("x", "d/l/../d/l/..")] {
            let refused = plan_unpack(vec![link("d/l", ".."), link(name, target)], &dest(), None).unwrap_err();
            assert_eq!(refused, escapes(name, Escape::LinksOut), "{name} -> {target}");
        }
        // A `..` after a link climbs from where the link ends: `y` ends at the top.
        let shallower = vec![link("x", "y/.."), link("y", ".")];
        assert_eq!(plan_unpack(shallower, &dest(), None).unwrap_err(), escapes("x", Escape::LinksOut));
    }

    #[test]
    fn a_link_through_links_that_stay_inside_is_planned() {
        // A macOS framework, as a bundle lists it.
        let framework = vec![
            file("Qt.framework/Versions/A/Qt", 3),
            link("Qt.framework/Versions/Current", "A"),
            link("Qt.framework/Qt", "Versions/Current/Qt"),
            link("Qt.framework/Resources", "Versions/Current/Resources"),
        ];
        assert!(plan_unpack(framework, &dest(), None).is_ok());
    }

    #[test]
    fn an_entry_under_a_link_is_refused() {
        let through = |listing: Vec<ArchiveEntry>, entry: &str| {
            assert_eq!(plan_unpack(listing, &dest(), None).unwrap_err(), escapes(entry, Escape::ThroughLink));
        };
        through(vec![link("a", "b"), file("b/keep", 1), file("a/f", 1)], "a/f");
        through(vec![file("a/f", 1), link("a", "b")], "a/f");
        through(vec![link("a", "b"), ArchiveEntry::new("a/c/", 0, EntryKind::Folder)], "a/c/");
        through(vec![link("a", "b"), link("a/l", "x")], "a/l");
        through(vec![link("a", "b"), hard_link("a/h", "b")], "a/h");
        // Something else at a link's own place: the link or the entry wins by the order of
        // the writes, so neither may.
        through(vec![link("a", "b"), file("a", 1)], "a");
        through(vec![file("a", 1), link("a", "b")], "a");
        through(vec![link("a", "b"), link("a", "c")], "a");
    }

    #[test]
    fn a_hard_link_to_a_link_or_through_one_is_refused() {
        // `h` would be a second link to `../..`, but at the top, where it points out.
        let refused = plan_unpack(vec![link("d/e/s", "../.."), hard_link("h", "d/e/s")], &dest(), None).unwrap_err();
        assert_eq!(refused, escapes("h", Escape::LinksOut));
        let refused = plan_unpack(vec![hard_link("h", "d/e/s"), link("d/e/s", "../..")], &dest(), None).unwrap_err();
        assert_eq!(refused, escapes("h", Escape::LinksOut), "in either order");
        // `a/s` is no listed link, but on the disk it reaches the link `b/c/s`.
        let through = vec![link("a", "b/c"), link("b/c/s", "../.."), hard_link("h", "a/s")];
        assert_eq!(plan_unpack(through, &dest(), None).unwrap_err(), escapes("h", Escape::LinksOut));
        // Even a link that stays inside: the plan can't say where a copy of it would end.
        let inside = vec![file("b/f", 1), link("a", "b"), hard_link("h", "a/f")];
        assert_eq!(plan_unpack(inside, &dest(), None).unwrap_err(), escapes("h", Escape::LinksOut));
    }

    #[test]
    fn a_link_loop_is_refused() {
        let refused = plan_unpack(vec![link("a", "b"), link("b", "a")], &dest(), None).unwrap_err();
        assert_eq!(refused, escapes("a", Escape::LinksOut));
        let refused = plan_unpack(vec![link("a/self", "../a/self")], &dest(), None).unwrap_err();
        assert_eq!(refused, escapes("a/self", Escape::LinksOut));
    }

    #[test]
    fn an_entry_that_needs_a_password_refuses_the_archive() {
        let mut locked = file("Game/data.bin", 4);
        locked.encrypted = true;
        let refused = plan_unpack(vec![file("readme.txt", 1), locked], &dest(), None).unwrap_err();
        assert_eq!(refused, ArchiveError::Password);
        assert_eq!(refused.to_string(), "This archive needs a password, which Marah Launcher can't ask for");
    }

    #[test]
    fn an_archive_bigger_than_the_free_space_is_refused() {
        assert!(room(10, Some(UNPACK_SPARE + 10)).is_ok());
        let refused = room(5_000_000_000, Some(1_000_000_000)).unwrap_err();
        assert_eq!(refused, ArchiveError::NoRoom { needed: 5_000_000_000 + UNPACK_SPARE, free: 1_000_000_000 });
        assert_eq!(refused.to_string(), "This archive needs 5.1 GB free, and the disk has 1.0 GB");
        assert!(room(u64::MAX, None).is_ok(), "a disk that doesn't say");

        let listing = || vec![file("a", 600), file("b", 400)];
        assert!(plan_unpack(listing(), &dest(), Some(1000 + UNPACK_SPARE)).is_ok());
        let refused = plan_unpack(listing(), &dest(), Some(999 + UNPACK_SPARE)).unwrap_err();
        assert_eq!(refused, ArchiveError::NoRoom { needed: 1000 + UNPACK_SPARE, free: 999 + UNPACK_SPARE });
    }

    /// Where `path` (from the folder's top) ends, by walking its parts and the targets of the
    /// plan's `links` on the way, as the system would; `None` when it climbs out or goes
    /// through more links than `budget` allows.
    fn resolve(path: &Path, links: &HashMap<PathBuf, PathBuf>, budget: &mut usize) -> Option<Vec<String>> {
        let mut at: Vec<String> = Vec::new();
        for part in path.components() {
            match part {
                Component::Normal(p) => {
                    at.push(p.to_string_lossy().into());
                    if let Some(target) = links.get(&at.iter().collect::<PathBuf>()) {
                        *budget = budget.checked_sub(1)?;
                        at.pop();
                        at = resolve(&at.iter().collect::<PathBuf>().join(target), links, budget)?;
                    }
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    at.pop()?;
                }
                Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(at)
    }

    /// The properties every plan has: each path has only normal parts and stays under `dest`,
    /// no entry sits at or under another entry's link, each link resolves inside through the
    /// plan's links, each hard link names a path inside and no link at it or on its way, and the
    /// total is the files' bytes.
    fn check(listing: &[ArchiveEntry], plan: &Plan) {
        assert_eq!(plan.entries.len(), listing.len(), "one planned entry per listed entry");
        let links: HashMap<PathBuf, PathBuf> = plan
            .entries
            .iter()
            .filter_map(|e| match &e.kind {
                EntryKind::Link(target) => Some((e.path.clone(), target.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            links.len(),
            plan.entries.iter().filter(|e| matches!(e.kind, EntryKind::Link(_))).count(),
            "one link per path"
        );
        for (entry, listed) in plan.entries.iter().zip(listing) {
            assert_eq!(entry.name, listed.name);
            assert!(entry.path.components().all(|c| matches!(c, Component::Normal(_))), "{entry:?}");
            let out = plan.out(entry);
            assert!(out.starts_with(&plan.dest), "{entry:?} leaves {}", plan.dest.display());
            assert!(!entry.path.ancestors().skip(1).any(|folder| links.contains_key(folder)), "{entry:?} under a link");
            if !matches!(entry.kind, EntryKind::Link(_)) {
                assert!(!links.contains_key(&entry.path), "{entry:?} at a link");
            }
            match &entry.kind {
                EntryKind::Link(target) => {
                    let way = entry.path.parent().unwrap_or(Path::new("")).join(target);
                    let resolved = resolve(&way, &links, &mut { MOST_LINKS });
                    assert!(resolved.is_some(), "{entry:?} resolves out of the folder")
                }
                EntryKind::HardLink(target) => {
                    assert!(target.components().all(|c| matches!(c, Component::Normal(_))), "{entry:?}");
                    assert!(target.components().next().is_some(), "{entry:?}");
                    assert!(!target.ancestors().any(|at| links.contains_key(at)), "{entry:?} names a link");
                }
                EntryKind::Other if entry.path.as_os_str().is_empty() => {}
                _ => assert!(!entry.path.as_os_str().is_empty(), "{entry:?}"),
            }
        }
        // The archive's own top (`./`, `.`) is left out whatever kind it says it is.
        let top = |e: &ArchiveEntry| Path::new(&e.name).components().all(|c| c == Component::CurDir);
        let files: u64 = listing.iter().filter(|e| e.kind == EntryKind::File && !top(e)).map(|e| e.size).sum();
        assert_eq!(plan.total, files);
    }

    /// A small xorshift generator: the same names every run, no extra crate.
    struct Names(u64);

    impl Names {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
            from[(self.next() % from.len() as u64) as usize]
        }

        /// A path of up to five parts, often hostile.
        fn path(&mut self) -> String {
            const PARTS: [&str; 10] = ["a", "b", "..", ".", "", "Game", "c:", "x\\..", "...", "lib.so"];
            let lead = self.pick(&["", "", "", "/", "./", "../"]);
            let parts: Vec<&str> = (0..1 + self.next() % 5).map(|_| self.pick(&PARTS)).collect();
            format!("{lead}{}", parts.join("/"))
        }

        fn entry(&mut self) -> ArchiveEntry {
            let name = self.path();
            let kind = match self.next() % 5 {
                0 => EntryKind::Folder,
                1 => EntryKind::Link(self.path().into()),
                2 => EntryKind::HardLink(self.path().into()),
                3 => EntryKind::Other,
                _ => EntryKind::File,
            };
            ArchiveEntry::new(name, self.next() % 1000, kind)
        }
    }

    #[test]
    fn no_planned_path_leaves_the_folder_whatever_the_listing() {
        let mut names = Names(0x9e37_79b9_7f4a_7c15);
        let (mut planned, mut refused) = (0, 0);
        for _ in 0..5000 {
            let listing: Vec<_> = (0..1 + names.next() % 6).map(|_| names.entry()).collect();
            match plan_unpack(listing.clone(), &dest(), None) {
                Ok(plan) => {
                    check(&listing, &plan);
                    planned += 1;
                }
                Err(ArchiveError::Escapes { entry, .. }) => {
                    assert!(listing.iter().any(|e| e.name == entry), "names a listed entry: {entry}");
                    refused += 1;
                }
                Err(e) => panic!("only escapes for a listing with no password and no size limit: {e}"),
            }
        }
        // The generator reaches both sides.
        assert!(planned > 100 && refused > 100, "{planned} planned, {refused} refused");
    }

    #[test]
    fn the_hostile_table_plans_nothing_outside_the_folder() {
        for name in LEAVING {
            for target in LEAVING {
                for listing in [vec![link("a/b", target)], vec![file(name, 1)], vec![hard_link("a", target)]] {
                    if let Ok(plan) = plan_unpack(listing.clone(), &dest(), None) {
                        check(&listing, &plan);
                    }
                }
            }
        }
    }
}

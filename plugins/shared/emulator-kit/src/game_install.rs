//! The steps of a game install that are the same for every emulator. A plugin gives each tree
//! to copy and where it goes; [`install`] refuses a game that is installed, clears each work
//! folder (`<dest>.part`), copies the files in the background, moves each work folder into
//! place, and on an error removes every work folder.

use std::sync::Arc;

use launcher_api::{Folders, HostPath, InstallJob, InstalledGame, PluginError, PluginResult, RelPath};

use crate::fail::{Context, Result};
use crate::installs::{self, Entry, Tree, Writer};

/// Reads a part's title from its files, before they are copied.
type ReadTitle = Box<dyn FnOnce(&mut dyn Tree, &[Entry]) -> String + Send>;
/// Runs on a part's work folder once its files are in, before it moves into place.
type Then = Box<dyn FnOnce(&Writer, &HostPath) -> Result<()> + Send>;

/// One tree of a game install, and where it goes.
pub struct Part {
    dest: HostPath,
    name: String,
    tree: Box<dyn Tree + Send>,
    prefix: String,
    listed: Option<Vec<Entry>>,
    whole: Option<HostPath>,
    title: Option<ReadTitle>,
    then: Option<Then>,
}

impl Part {
    /// Every file of `tree`, into `dest`. `name` is what the player reads: the game's title,
    /// and what "is already installed" names.
    pub fn new(dest: HostPath, name: impl Into<String>, tree: Box<dyn Tree + Send>) -> Self {
        Self {
            dest,
            name: name.into(),
            tree,
            prefix: String::new(),
            listed: None,
            whole: None,
            title: None,
            then: None,
        }
    }

    /// Only the files under `prefix`, without it.
    pub fn under(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// The tree's files, listed already: the install doesn't list them again.
    pub fn listed(mut self, entries: Vec<Entry>) -> Self {
        self.listed = Some(entries);
        self
    }

    /// The host folder `from` holds these files as they are: the host may move it into place
    /// in place of a copy ([`InstallJob::take`]).
    pub fn whole(mut self, from: Option<HostPath>) -> Self {
        self.whole = from;
        self
    }

    /// The title comes from the files: `read` reads it before they are copied.
    pub fn titled(mut self, read: impl FnOnce(&mut dyn Tree, &[Entry]) -> String + Send + 'static) -> Self {
        self.title = Some(Box::new(read));
        self
    }

    /// `then` runs on the work folder once the files are in, before it moves into place.
    pub fn then(mut self, then: impl FnOnce(&Writer, &HostPath) -> Result<()> + Send + 'static) -> Self {
        self.then = Some(Box::new(then));
        self
    }
}

/// Why an install is refused: the game is there. A precondition, not bad input: the source is
/// fine, and it installs once the game is uninstalled.
fn already_installed(name: &str) -> PluginError {
    PluginError::Precondition(format!("{name} is already installed"))
}

/// The work folder of an install into `dest`: `<dest>.part` beside it, which no game index reads.
fn work_folder(dest: &HostPath) -> PluginResult<HostPath> {
    Ok(HostPath::new(&dest.folder, dest.root, RelPath::new(format!("{}.part", dest.path))?))
}

/// Installs `parts` as one job, in order, with one progress for them all, and returns the first
/// part's work folder. The job's game is the first part's. Call it from a plugin's `begin`,
/// which returns what it returns.
///
/// Before it returns, a part whose `dest` is there refuses the whole install, and nothing is
/// written. Then the parts copy in the background. On an error every work folder goes; the
/// parts already in place stay.
pub async fn install(folders: Arc<dyn Folders>, job: Arc<dyn InstallJob>, parts: Vec<Part>) -> PluginResult<HostPath> {
    // A part isn't `Sync`, so no borrow of one is held across an await.
    let dests: Vec<HostPath> = parts.iter().map(|part| part.dest.clone()).collect();
    for (at, dest) in dests.iter().enumerate() {
        if folders.exists(dest).await? {
            return Err(already_installed(&parts[at].name));
        }
    }
    let mut works = Vec::with_capacity(dests.len());
    for dest in &dests {
        let work = work_folder(dest)?;
        folders.remove(&work).await?;
        works.push(work);
    }
    let first = works.first().cloned().ok_or_else(|| PluginError::InvalidInput("nothing to install".into()))?;
    let writer = Writer { folders, runtime: tokio::runtime::Handle::current() };
    tokio::task::spawn_blocking(move || {
        let parts = parts.into_iter().zip(works.iter().cloned()).collect();
        match copy(&writer, job.as_ref(), parts) {
            Ok((dest, title)) => job.finished(Ok(InstalledGame { dest, title })),
            Err(e) => {
                // If this fails too, the folder stays as `<name>.part`, which no game index reads.
                for work in &works {
                    let _ = writer.remove(work);
                }
                job.finished(Err(PluginError::Other(e.to_string())));
            }
        }
    });
    Ok(first)
}

/// Copies each part into its work folder and moves it into place. Returns the first part's
/// place and title.
fn copy(writer: &Writer, job: &dyn InstallJob, parts: Vec<(Part, HostPath)>) -> Result<(HostPath, String)> {
    let cancel = installs::cancel_check(job);
    let mut listed = Vec::with_capacity(parts.len());
    for (mut part, work) in parts {
        let entries = match part.listed.take() {
            Some(entries) => entries,
            None => part.tree.entries(&cancel)?,
        };
        listed.push((part, entries, work));
    }
    let total = listed.iter().map(|(part, entries, _)| installs::tree_size(entries, &part.prefix)).sum();
    let mut done = 0;
    job.progress(0, total);
    let mut first = None;
    for (part, entries, work) in listed {
        let Part { dest, name, mut tree, prefix, whole, title, then, .. } = part;
        let title = match title {
            Some(read) => read(tree.as_mut(), &entries),
            None => name,
        };
        if whole.is_some_and(|from| job.take(&from, &work)) {
            done += installs::tree_size(&entries, &prefix);
            job.progress(done, total);
        } else {
            installs::write_tree_counted(writer, tree.as_mut(), &entries, &prefix, &work, job, (&mut done, total))?;
        }
        if let Some(then) = then {
            then(writer, &work)?;
        }
        writer.rename(&work, &dest)?;
        first.get_or_insert((dest, title));
    }
    first.context("Nothing to install")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::mpsc;

    use launcher_api::{DirEntry, FolderRoot, async_trait};

    use super::*;
    use crate::ensure;

    /// Host folders in memory: each file by its path. A folder is there while a file is in it.
    #[derive(Default)]
    struct MemFolders(Mutex<BTreeMap<String, Vec<u8>>>);

    impl MemFolders {
        fn paths(&self) -> Vec<String> {
            self.0.lock().unwrap().keys().cloned().collect()
        }
    }

    fn under(path: &str, dir: &HostPath) -> bool {
        let dir = dir.path.as_str();
        path == dir || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
    }

    #[async_trait]
    impl Folders for MemFolders {
        async fn roots(&self, _folder: &str) -> PluginResult<Vec<FolderRoot>> {
            Ok(Vec::new())
        }
        async fn list(&self, _dir: &HostPath) -> PluginResult<Vec<DirEntry>> {
            Ok(Vec::new())
        }
        async fn size(&self, _dir: &HostPath) -> PluginResult<u64> {
            Ok(0)
        }
        async fn exists(&self, path: &HostPath) -> PluginResult<bool> {
            Ok(self.0.lock().unwrap().keys().any(|p| under(p, path)))
        }
        async fn read_range(&self, path: &HostPath, _offset: u64, _len: u64) -> PluginResult<Vec<u8>> {
            Err(PluginError::NotFound(path.path.to_string()))
        }
        async fn write(&self, path: &HostPath, data: &[u8]) -> PluginResult<()> {
            self.0.lock().unwrap().insert(path.path.to_string(), data.to_vec());
            Ok(())
        }
        async fn write_at(&self, path: &HostPath, _offset: u64, data: &[u8]) -> PluginResult<()> {
            self.0.lock().unwrap().entry(path.path.to_string()).or_default().extend_from_slice(data);
            Ok(())
        }
        async fn rename(&self, from: &HostPath, to: &HostPath) -> PluginResult<()> {
            let mut files = self.0.lock().unwrap();
            let moved: Vec<String> = files.keys().filter(|p| under(p, from)).cloned().collect();
            for path in moved {
                let bytes = files.remove(&path).unwrap_or_default();
                files.insert(format!("{}{}", to.path, &path[from.path.as_str().len()..]), bytes);
            }
            Ok(())
        }
        async fn remove(&self, path: &HostPath) -> PluginResult<()> {
            self.0.lock().unwrap().retain(|p, _| !under(p, path));
            Ok(())
        }
        async fn set_executable(&self, _path: &HostPath) -> PluginResult<()> {
            Ok(())
        }
    }

    /// One file, `game.bin`, whose copy fails when `broken`.
    struct OneFile {
        broken: bool,
    }

    impl Tree for OneFile {
        fn entries(&mut self, _cancel: &dyn Fn() -> Result<()>) -> Result<Vec<Entry>> {
            Ok(vec![Entry { path: "game.bin".into(), directory: false, size: 4 }])
        }

        fn copy(&mut self, _index: usize, sink: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
            sink(b"data")?;
            ensure!(!self.broken, "The package is damaged");
            Ok(())
        }
    }

    /// The host's side of an install: it hands on the end.
    struct Job(Mutex<mpsc::Sender<PluginResult<InstalledGame>>>);

    impl InstallJob for Job {
        fn progress(&self, _done: u64, _total: u64) {}
        fn cancelled(&self) -> bool {
            false
        }
        fn finished(&self, result: PluginResult<InstalledGame>) {
            let _ = self.0.lock().unwrap().send(result);
        }
    }

    fn at(path: &str) -> HostPath {
        HostPath::new("games", 0, RelPath::new(path).unwrap())
    }

    fn part(path: &str, broken: bool) -> Part {
        Part::new(at(path), path, Box::new(OneFile { broken }))
    }

    /// Starts an install of `parts` and waits for its end.
    async fn run(folders: &Arc<MemFolders>, parts: Vec<Part>) -> PluginResult<InstalledGame> {
        let (send, end) = mpsc::channel();
        let job = Arc::new(Job(Mutex::new(send)));
        install(folders.clone(), job, parts).await?;
        tokio::task::spawn_blocking(move || end.recv().expect("the install ends")).await.unwrap()
    }

    #[tokio::test]
    async fn the_parts_go_into_place_and_the_game_is_the_first() {
        let folders = Arc::new(MemFolders::default());
        let game = run(&folders, vec![part("CUSA1", false), part("CUSA1-patch", false)]).await.unwrap();
        assert_eq!((game.dest, game.title), (at("CUSA1"), "CUSA1".to_owned()));
        assert_eq!(folders.paths(), ["CUSA1-patch/game.bin", "CUSA1/game.bin"]);
    }

    #[tokio::test]
    async fn a_failed_part_takes_every_work_folder_with_it_and_what_is_in_place_stays() {
        let folders = Arc::new(MemFolders::default());
        let parts = vec![part("CUSA1", false), part("CUSA1-patch", true), part("DLC", false)];
        let end = run(&folders, parts).await;
        assert!(matches!(&end, Err(PluginError::Other(e)) if e.contains("damaged")), "{end:?}");
        assert_eq!(folders.paths(), ["CUSA1/game.bin"], "no work folder is left");
    }

    #[tokio::test]
    async fn an_installed_part_refuses_the_install_before_anything_is_written() {
        let folders = Arc::new(MemFolders::default());
        folders.0.lock().unwrap().insert("CUSA1-patch/eboot.bin".into(), b"mine".to_vec());
        folders.0.lock().unwrap().insert("CUSA1.part/half.bin".into(), b"half".to_vec());
        let end = run(&folders, vec![part("CUSA1", false), part("CUSA1-patch", false)]).await;
        assert_eq!(end, Err(PluginError::Precondition("CUSA1-patch is already installed".into())));
        assert_eq!(folders.paths(), ["CUSA1-patch/eboot.bin", "CUSA1.part/half.bin"], "nothing is touched");
    }
}

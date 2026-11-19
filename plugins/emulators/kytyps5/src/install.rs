//! Game installs from PS5 packages, exFAT images and unpacked game folders.

use std::sync::Arc;

use emulator_kit::fail::Context;
use emulator_kit::game_install::{self, Part};
use emulator_kit::games;
use emulator_kit::installs::{self, Entry, ExfatImage, Format, Ps5Package, Tree, Writer};
use emulator_kit::pkg5;
use emulator_kit::read_at::HostFile;
use launcher_api::*;

use crate::{CONSOLE, Kyty};

/// The most bytes of a `param.json` read.
const PARAM_LIMIT: u64 = 1 << 20;

/// A tree the background work can take.
type GameTree = Box<dyn Tree + Send>;

/// What a game installs from.
enum Source {
    /// A package or image file the player picked.
    File(HostFile, Format),
    /// An unpacked game folder in a download: its tree, its path in the download, and its
    /// `param.json`.
    Folder(installs::HostTree, String, Vec<u8>),
    /// The package or image in a download that holds the game, opened and listed to find it.
    Listed(GameTree, Vec<Entry>),
}

/// A PS5 package that opens when it is listed, in the install's background work: opening one
/// reads its whole file system index.
struct LaterPackage {
    file: Option<HostFile>,
    package: Option<Ps5Package<HostFile>>,
}

impl Tree for LaterPackage {
    fn entries(
        &mut self,
        cancel: &dyn Fn() -> emulator_kit::fail::Result<()>,
    ) -> emulator_kit::fail::Result<Vec<Entry>> {
        if let Some(file) = self.file.take() {
            self.package = Some(Ps5Package::open(file, cancel)?);
        }
        self.package.as_mut().context("The package didn't open")?.entries(cancel)
    }

    fn copy(
        &mut self,
        index: usize,
        sink: &mut dyn FnMut(&[u8]) -> emulator_kit::fail::Result<()>,
    ) -> emulator_kit::fail::Result<()> {
        self.package.as_mut().context("The package isn't listed")?.copy(index, sink)
    }
}

/// The game folder in a listed tree and its `param.json`.
fn listed_param(tree: &mut dyn Tree, entries: &[Entry]) -> emulator_kit::fail::Result<(String, Vec<u8>)> {
    let prefix =
        installs::game_folder(entries, "param.json").context("This holds no PS5 game: it has no sce_sys/param.json")?;
    let at = entries.iter().position(|e| e.path == format!("{prefix}sce_sys/param.json")).context("param.json")?;
    let param = Writer::read_entry(tree, at, PARAM_LIMIT)?;
    Ok((prefix, param))
}

/// What `begin` finds in a source: the tree to copy, its files when they are listed already,
/// the game folder in it (a path with a `/` after it, or `""`), and the game's `param.json`.
struct Found {
    tree: GameTree,
    entries: Option<Vec<Entry>>,
    prefix: String,
    param: Vec<u8>,
}

/// Finds the game in `source`, reading as little as it can: a package's metadata block, an
/// image's folders on the way to its `param.json`. For blocking code.
fn find(source: Source, cancel: &dyn Fn() -> emulator_kit::fail::Result<()>) -> emulator_kit::fail::Result<Found> {
    let listed = |mut tree: GameTree, entries: Vec<Entry>| {
        let (prefix, param) = listed_param(tree.as_mut(), &entries)?;
        Ok(Found { tree, entries: Some(entries), prefix, param })
    };
    match source {
        Source::Folder(tree, _, param) => {
            Ok(Found { tree: Box::new(tree), entries: None, prefix: String::new(), param })
        }
        Source::Listed(tree, entries) => listed(tree, entries),
        Source::File(file, Format::Ps5Package) => match pkg5::metadata_file(&file, "param.json", PARAM_LIMIT)? {
            Some(param) => {
                let tree = Box::new(LaterPackage { file: Some(file), package: None });
                Ok(Found { tree, entries: None, prefix: String::new(), param })
            }
            // Not in the metadata block: it may be in the game's own files.
            None => {
                let mut tree = Box::new(Ps5Package::open(file, cancel)?);
                let entries = tree.entries(cancel)?;
                listed(tree, entries)
            }
        },
        Source::File(file, _) => {
            let image = ExfatImage::open(file)?;
            let (prefix, param) = image
                .nearest("sce_sys/param.json", PARAM_LIMIT, cancel)?
                .context("This holds no PS5 game: it has no sce_sys/param.json")?;
            Ok(Found { tree: Box::new(image), entries: None, prefix, param })
        }
    }
}

/// The package or image in `download` that holds the game: the one nearest the top. Update
/// and add-on packages hold no game, so they are skipped. A package that fails to open at or
/// above that depth might be the game, so its error stops the install; two at the same depth
/// are never guessed. The game's tree comes back listed, so the install doesn't list it again.
async fn game_package(folders: &Arc<dyn Folders>, download: &installs::Download) -> PluginResult<Source> {
    let mut best: Option<(usize, Vec<String>, GameTree, Vec<Entry>)> = None;
    let mut failure: Option<(usize, PluginError)> = None;
    for (file, format) in
        download.packages(folders.as_ref(), |f| matches!(f, Format::Ps5Package | Format::Exfat)).await?
    {
        let depth = file.depth();
        let reader = HostFile::new(folders.clone(), file.path.clone(), file.size);
        let opened = installs::blocking(move || {
            let mut tree: GameTree = match format {
                Format::Ps5Package => Box::new(Ps5Package::open(reader, &|| Ok(()))?),
                _ => Box::new(ExfatImage::open(reader)?),
            };
            let entries = tree.entries(&|| Ok(()))?;
            Ok((tree, entries))
        })
        .await;
        match opened {
            Ok((tree, entries)) if installs::holds_game(&entries) => match &mut best {
                Some((top, names, ..)) if depth == *top => names.push(file.rel.clone()),
                Some((top, ..)) if depth > *top => (),
                _ => best = Some((depth, vec![file.rel.clone()], tree, entries)),
            },
            Ok(_) => (),
            Err(e) => {
                if failure.as_ref().is_none_or(|f| depth < f.0) {
                    failure = Some((depth, PluginError::InvalidInput(format!("{}: {e}", file.rel))));
                }
            }
        }
    }
    if let Some((depth, e)) = failure
        && best.as_ref().is_none_or(|b| depth <= b.0)
    {
        return Err(e);
    }
    let Some((_, names, tree, entries)) = best else { return Err(download.nothing_to_install()) };
    if names.len() > 1 {
        return Err(PluginError::InvalidInput(format!(
            "Several packages hold a game ({}); choose a single release folder",
            names.join(", ")
        )));
    }
    Ok(Source::Listed(tree, entries))
}

/// The unpacked game folder in `download` that holds the game: the one nearest the top. Folders
/// at that depth with one title id are one game, and the largest is the whole game: the others
/// are patches of it, such as a backport for a console. Folders of different games are never
/// guessed, as for packages. `None` when it has no game folder.
async fn game_folder(folders: &dyn Folders, download: &installs::Download) -> PluginResult<Option<String>> {
    let found = download.game_folders("param.json");
    let depth = |p: &String| p.matches('/').count();
    let Some(top) = found.first().map(depth) else { return Ok(None) };
    let level: Vec<&String> = found.iter().filter(|p| depth(p) == top).collect();
    if let [only] = level.as_slice() {
        return Ok(Some((*only).clone()));
    }
    let mut ids = Vec::new();
    for prefix in &level {
        let param = download.read(folders, &format!("{prefix}sce_sys/param.json"), 1 << 20).await?;
        ids.push(games::ps5_param(&param).map(|(id, _)| id));
    }
    if ids[0].is_none() || ids.iter().any(|id| *id != ids[0]) {
        return Err(PluginError::InvalidInput(format!(
            "Several folders hold a game ({}, {}); choose a single release folder",
            level[0], level[1]
        )));
    }
    let size = |prefix: &str| download.files.iter().filter(|f| f.rel.starts_with(prefix)).map(|f| f.size).sum::<u64>();
    Ok(level.iter().max_by_key(|p| size(p)).map(|p| (*p).clone()))
}

#[async_trait]
impl GameInstaller for Kyty {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        self.emu.candidates().await
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.emu.host().folders.clone();
        let base = installs::games_folder(folders.as_ref(), job.as_ref(), CONSOLE).await?;
        // A download is a folder: an unpacked game in it, else the package that holds the game.
        let from = match installs::Download::scan(folders.as_ref(), source).await? {
            None => {
                let head = folders.read_range(source, 0, 512).await?;
                match installs::format(&head) {
                    Some(f @ (Format::Ps5Package | Format::Exfat)) => {
                        let size = installs::file_size(folders.as_ref(), source).await?;
                        Source::File(HostFile::new(folders.clone(), source.clone(), size), f)
                    }
                    _ => {
                        return Err(PluginError::InvalidInput(format!(
                            "{} isn't a PS5 package or exFAT image",
                            source.path
                        )));
                    }
                }
            }
            Some(download) => match game_folder(folders.as_ref(), &download).await? {
                Some(prefix) => {
                    let param =
                        download.read(folders.as_ref(), &format!("{prefix}sce_sys/param.json"), PARAM_LIMIT).await?;
                    Source::Folder(download.tree(folders.clone(), &prefix), prefix, param)
                }
                None => game_package(&folders, &download).await?,
            },
        };
        // An unpacked folder may move rather than be copied, when the host made it.
        let whole = match &from {
            Source::Folder(_, prefix, _) => Some(source.join(prefix.trim_end_matches('/'))?),
            _ => None,
        };
        // The game's title id names its folder, so `begin` reads its `param.json`: an installed
        // game is refused before anything is written. It reads only that: the tree is listed
        // in the background, unless finding the game in a download listed it already.
        let cancel_job = job.clone();
        let found = installs::blocking(move || find(from, &installs::cancel_check(cancel_job.as_ref()))).await?;
        let (title_id, title) = games::ps5_param(&found.param)
            .ok_or_else(|| PluginError::InvalidInput("Its param.json has no usable title id".into()))?;
        let dest = base.join(&format!("{title_id}-app0"))?;
        let mut part = Part::new(dest, title, found.tree).under(found.prefix).whole(whole);
        if let Some(entries) = found.entries {
            part = part.listed(entries);
        }
        game_install::install(folders, job, vec![part]).await
    }
}

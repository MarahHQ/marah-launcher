//! The PS4 system files that shadPS4 can use: 14 system modules (`sys_modules/*.sprx`) and 45
//! fonts (`fonts/font/*`, `fonts/font2/*`), in shadPS4's data folder ([`DATA`]). Some games
//! need them.
//!
//! - The list is [`TABLE`]: each file's path, size and SHA-256, from the retrobios collection
//!   on GitHub (Abdess/retrobios) at commit [`COMMIT`], with the sizes and hashes of its own
//!   `database.json` at that commit. Each download comes from that commit, never from a branch.
//! - At the start the plugin only looks at the folder. When listed files are missing, it asks
//!   over the boot screen ([`StartupWork`]); "Not now" leaves everything as it is until the
//!   next start. The System files block on the PS4 page shows the state, and Install now
//!   installs at once.
//! - An install downloads only the missing files. Each file goes to a staging folder
//!   ([`STAGING`]) first; its size and SHA-256 are checked; then one rename puts it in place.
//!   A rename never replaces a file: a file the player put there stays as it is. Files that
//!   aren't on the list are never touched.
//! - The files the plugin installed, and the commit they came from, are kept in the plugin's
//!   storage ([`MARKER`]). A listed file that is there and not in the marker is the player's own.
//! - A failure (a wrong hash, a 404, no network) ends the install: the boot screen and the block
//!   say so, and the boot goes on. The files already checked and in place stay; nothing
//!   half-written is in shadPS4's folders.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, Weak};

use emulator_kit::console::Panel;
use emulator_kit::host::Host;
use launcher_api::serde_json::{self, Value};
use launcher_api::*;
use sha2::{Digest, Sha256};

use crate::DATA;

/// The retrobios commit the files come from.
pub const COMMIT: &str = "f75b87448e52aa4f64385105583b6e937957d95d";
/// The files' folder in that commit, with its space already encoded.
const BASE: &str = "bios/Sony/PlayStation%204/shadps4";
/// The list: `<path>\t<size>\t<sha256>` a line.
const TABLE: &str = include_str!("system_files.tsv");
/// The folders shadPS4 reads the files from. Each listed file is in one of them.
const FOLDERS: [&str; 3] = ["sys_modules", "fonts/font", "fonts/font2"];
/// Where a download waits for its check, in shadPS4's data folder: outside the folders
/// shadPS4 reads.
const STAGING: &str = "system-files.part";
/// The files the plugin installed and their commit, in the plugin's storage.
const MARKER: &str = "system-files.json";

/// The actions of the System files block.
const INSTALL_NOW: &str = "install-system-files";

/// A file of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFile {
    /// In shadPS4's data folder, such as `sys_modules/libSceFont.sprx`.
    pub path: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

impl SystemFile {
    pub fn new(path: &str, size: u64, sha256: &str) -> Self {
        Self { path: path.into(), size, sha256: sha256.into() }
    }
}

/// The real list: the files of [`COMMIT`].
pub fn table() -> Vec<SystemFile> {
    TABLE
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            let (Some(path), Some(size), Some(sha256), None) = (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                panic!("system_files.tsv: a line isn't <path>\\t<size>\\t<sha256>: {line}");
            };
            let size = size.parse().unwrap_or_else(|_| panic!("system_files.tsv: a bad size: {line}"));
            SystemFile::new(path, size, sha256)
        })
        .collect()
}

/// Where `path` downloads from: raw.githubusercontent.com, at [`COMMIT`].
pub fn url(path: &str) -> String {
    let encoded: Vec<String> = path.split('/').map(encode).collect();
    format!("https://raw.githubusercontent.com/Abdess/retrobios/{COMMIT}/{BASE}/{}", encoded.join("/"))
}

/// A path segment for a URL: every byte but the unreserved ones as `%XX`.
fn encode(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// What the work does now.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Nothing ran in this run.
    Idle,
    /// The start found missing files, and asks the player first.
    Asking(Offer),
    /// The player said "Not now": nothing happens on its own in this run.
    Declined,
    Installing(Step),
    /// An install that ended well, and what it did.
    Complete(String),
    /// An install that failed, and why.
    Failed(String),
}

/// The missing files the start offers to install.
#[derive(Clone, Debug, PartialEq)]
struct Offer {
    count: usize,
    bytes: u64,
}

/// An install that runs: how far its downloads are.
#[derive(Clone, Debug, PartialEq)]
struct Step {
    /// From 0 to 1.
    progress: f32,
    /// Such as "12.0 MB of 38.1 MB".
    detail: String,
}

impl Step {
    fn at(done: u64, total: u64) -> Self {
        Self { progress: done as f32 / total.max(1) as f32, detail: format!("{} of {}", mb(done), mb(total)) }
    }
}

/// The listed files in shadPS4's folder, as they are now.
struct Found {
    /// The listed files that aren't there.
    missing: Vec<SystemFile>,
    /// The listed files that are there and that the plugin didn't install.
    own: usize,
}

pub struct SystemFiles {
    /// Itself, for the install it runs in the background.
    me: Weak<SystemFiles>,
    host: Host,
    events: Arc<dyn EventSink>,
    files: Vec<SystemFile>,
    phase: Mutex<Phase>,
    /// The start looked at the folder already: it asks once a run.
    checked: Mutex<bool>,
}

impl SystemFiles {
    pub fn new(host: Host, events: Arc<dyn EventSink>, files: Vec<SystemFile>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            host,
            events,
            files,
            phase: Mutex::new(Phase::Idle),
            checked: Mutex::new(false),
        })
    }

    fn phase(&self) -> Phase {
        self.phase.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set_phase(&self, phase: Phase) {
        *self.phase.lock().unwrap_or_else(|p| p.into_inner()) = phase;
        if let Err(e) = self.events.publish(SLOTS_CHANGED, Value::Null) {
            self.host.log.warn(&format!("could not tell the core the PS4 system files changed: {e}"));
        }
    }

    /// `path` in the first of shadPS4's data folders.
    async fn place(&self, path: &str) -> PluginResult<HostPath> {
        let root =
            self.host.folders.roots(DATA).await?.first().map(|r| r.root).ok_or_else(|| {
                PluginError::Precondition("shadPS4's data folder isn't set: set it in Settings".into())
            })?;
        Ok(HostPath::new(DATA, root, RelPath::new(path)?))
    }

    /// The files the plugin installed, from its marker.
    async fn installed(&self) -> BTreeSet<String> {
        let read = async {
            let bytes = self.host.storage.read(&RelPath::new(MARKER).ok()?).await.ok()?;
            let marker: Value = serde_json::from_slice(&bytes).ok()?;
            Some(marker["files"].as_array()?.iter().filter_map(|f| f.as_str().map(str::to_owned)).collect())
        };
        read.await.unwrap_or_default()
    }

    /// Adds `path` to the marker, with the commit it came from.
    async fn mark(&self, path: &str) -> PluginResult<()> {
        let mut files = self.installed().await;
        files.insert(path.to_owned());
        let marker = serde_json::json!({ "commit": COMMIT, "files": files });
        self.host.storage.write(&RelPath::new(MARKER)?, marker.to_string().as_bytes()).await
    }

    /// Which listed files are in shadPS4's folder: one listing of each folder, no hashing.
    async fn find(&self) -> PluginResult<Found> {
        let mut present = BTreeSet::new();
        for folder in FOLDERS {
            for entry in self.host.folders.list(&self.place(folder).await?).await? {
                present.insert(format!("{folder}/{}", entry.name));
            }
        }
        let installed = self.installed().await;
        let missing = self.files.iter().filter(|f| !present.contains(&f.path)).cloned().collect();
        let own = self.files.iter().filter(|f| present.contains(&f.path) && !installed.contains(&f.path)).count();
        Ok(Found { missing, own })
    }

    /// Starts an install in the background, unless one runs. The phase says it runs before
    /// this returns, so the boot screen waits for it.
    fn start(&self) {
        {
            let mut phase = self.phase.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(*phase, Phase::Installing(_)) {
                return;
            }
            *phase = Phase::Installing(Step::at(0, 0));
        }
        if let Err(e) = self.events.publish(SLOTS_CHANGED, Value::Null) {
            self.host.log.warn(&format!("could not tell the core the PS4 system files install started: {e}"));
        }
        let Some(this) = self.me.upgrade() else { return };
        tokio::spawn(async move {
            let phase = match this.install().await {
                Ok(message) => Phase::Complete(message),
                Err(e) => {
                    this.host.log.warn(&format!("the PS4 system files install failed: {e}"));
                    Phase::Failed(e.to_string())
                }
            };
            this.set_phase(phase);
        });
    }

    /// Downloads the missing files, checks each, and moves each into place. The staging folder
    /// goes at the end, also after a failure.
    async fn install(&self) -> PluginResult<String> {
        let staging = self.place(STAGING).await?;
        self.host.folders.remove(&staging).await?;
        let installed = self.install_missing(&staging).await;
        if let Err(e) = self.host.folders.remove(&staging).await {
            self.host.log.warn(&format!("could not remove {}: {e}", staging.path));
        }
        let count = installed?;
        Ok(format!("Installed {count} of the {} files", self.files.len()))
    }

    async fn install_missing(&self, staging: &HostPath) -> PluginResult<usize> {
        let missing = self.find().await?.missing;
        let total: u64 = missing.iter().map(|f| f.size).sum();
        let mut done = 0;
        let mut count = 0;
        self.set_phase(Phase::Installing(Step::at(0, total)));
        for file in &missing {
            let bytes = self.download(file).await?;
            let staged = staging.join(&file.path)?;
            self.host.folders.write(&staged, &bytes).await?;
            let live = self.place(&file.path).await?;
            // The player may have put the file there during the download: theirs stays.
            if self.host.folders.exists(&live).await? {
                self.host.folders.remove(&staged).await?;
            } else {
                self.host.folders.rename(&staged, &live).await?;
                self.mark(&file.path).await?;
                count += 1;
            }
            done += file.size;
            self.set_phase(Phase::Installing(Step::at(done, total)));
        }
        Ok(count)
    }

    /// The bytes of `file`, once their size and SHA-256 match the list.
    async fn download(&self, file: &SystemFile) -> PluginResult<Vec<u8>> {
        let url = url(&file.path);
        let response = self.host.http.send(HttpRequest::get(&url)).await?;
        if response.status == 404 {
            return Err(PluginError::NotFound(format!("{} isn't on GitHub (HTTP 404): {url}", file.path)));
        }
        if response.status != 200 {
            return Err(PluginError::Network(format!("{} downloaded with HTTP {}: {url}", file.path, response.status)));
        }
        if response.body.len() as u64 != file.size {
            return Err(PluginError::InvalidInput(format!(
                "{} has {} bytes, not the {} of the list",
                file.path,
                response.body.len(),
                file.size
            )));
        }
        if hex(&Sha256::digest(&response.body)) != file.sha256 {
            return Err(PluginError::InvalidInput(format!("{} doesn't match the SHA-256 of the list", file.path)));
        }
        Ok(response.body)
    }

    /// The System files block on the PS4 page.
    async fn panel(&self) -> Vec<SlotItem> {
        let phase = self.phase();
        let row = |label: &str, value: String, hint: String, tone| BlockRow {
            label: label.into(),
            value,
            hint,
            tone,
            ..BlockRow::default()
        };
        let product = launcher_api::brand::PRODUCT_NAME;
        let total = self.files.len();
        let found = self.find().await;
        let mut rows = Vec::new();
        let missing = match &found {
            Ok(found) => found.missing.len(),
            Err(_) => total,
        };
        rows.push(match &found {
            Err(e) => row("Status", "Unknown".into(), e.to_string(), Tone::Warning),
            Ok(found) if !found.missing.is_empty() => row(
                "Status",
                format!("Missing {} of {total}", found.missing.len()),
                "Some games need the PS4's system modules and fonts. Install now downloads the missing files; \
                 files already there stay as they are."
                    .into(),
                Tone::Warning,
            ),
            Ok(found) if found.own > 0 => row(
                "Status",
                "Your own files".into(),
                format!("{} of the {total} files are yours. {product} leaves them as they are.", found.own),
                Tone::Neutral,
            ),
            Ok(_) => row("Status", "Installed".into(), format!("{product} installed them"), Tone::Neutral),
        });
        match &phase {
            Phase::Installing(step) => rows.push(BlockRow {
                progress: Some(step.progress),
                value: format!("{}%", (step.progress * 100.0) as u32),
                ..row("Installing", String::new(), step.detail.clone(), Tone::Info)
            }),
            Phase::Failed(error) => {
                rows.push(row("The last install failed", String::new(), error.clone(), Tone::Danger))
            }
            Phase::Complete(message) => rows.push(row("Last install", String::new(), message.clone(), Tone::Neutral)),
            Phase::Idle | Phase::Asking(_) | Phase::Declined => {}
        }
        if !matches!(phase, Phase::Installing(_)) && missing > 0 && found.is_ok() {
            rows.push(BlockRow {
                action: Some(INSTALL_NOW.into()),
                ..row(
                    "Install now",
                    String::new(),
                    "Downloads the missing files from the retrobios collection on GitHub".into(),
                    Tone::Neutral,
                )
            });
        }
        let block = SlotContent::Block { title: "System files".into(), rows };
        vec![SlotItem { id: "system-files".into(), order: Order::default(), content: block }]
    }
}

/// The install that runs, as the boot screen shows it on one line.
#[async_trait]
impl StartupWork for SystemFiles {
    async fn status(&self) -> Option<StartupStep> {
        let (detail, progress, state) = match self.phase() {
            // Nothing runs: the boot doesn't wait on it.
            Phase::Idle | Phase::Asking(_) | Phase::Declined => return None,
            // The boot screen adds the percent.
            Phase::Installing(step) => ("Downloading".to_owned(), Some(step.progress), StartupState::Working),
            Phase::Complete(_) => ("Installed".to_owned(), Some(1.0), StartupState::Done),
            Phase::Failed(_) => ("Could not install".to_owned(), None, StartupState::Failed),
        };
        Some(StartupStep { label: "PS4 system files".into(), detail, progress, state })
    }

    /// The first call of a run looks at the folder; it asks when listed files are missing.
    async fn question(&self) -> Option<StartupQuestion> {
        let first = !std::mem::replace(&mut *self.checked.lock().unwrap_or_else(|p| p.into_inner()), true);
        if first && self.phase() == Phase::Idle {
            match self.find().await {
                Ok(found) if !found.missing.is_empty() => {
                    let bytes = found.missing.iter().map(|f| f.size).sum();
                    self.set_phase(Phase::Asking(Offer { count: found.missing.len(), bytes }));
                }
                Ok(_) => {}
                Err(e) => self.host.log.info(&format!("the PS4 system files aren't checked: {e}")),
            }
        }
        let Phase::Asking(offer) = self.phase() else { return None };
        let product = launcher_api::brand::PRODUCT_NAME;
        let what = if offer.count == self.files.len() {
            format!("the {} files", offer.count)
        } else {
            format!("the {} missing files", offer.count)
        };
        Some(StartupQuestion {
            title: "Install PS4 system files?".into(),
            text: format!(
                "Some PS4 games need the PS4's system modules and fonts. {product} downloads {what}, about {}, \
                 from the retrobios collection on GitHub into shadPS4's folder.",
                mb(offer.bytes)
            ),
            yes: "Install".into(),
            no: "Not now".into(),
        })
    }

    /// Install starts the downloads; Not now leaves everything as it is until the next start.
    async fn answer(&self, yes: bool) {
        if !matches!(self.phase(), Phase::Asking(_)) {
            return;
        }
        if yes {
            self.start();
        } else {
            self.set_phase(Phase::Declined);
        }
    }
}

#[async_trait]
impl Panel for SystemFiles {
    async fn items(&self) -> Vec<SlotItem> {
        self.panel().await
    }

    async fn run_action(&self, action: &str, _args: &Value) -> Option<PluginResult<()>> {
        (action == INSTALL_NOW).then(|| {
            self.start();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_has_the_14_modules_and_45_fonts_of_the_pinned_commit() {
        let files = table();
        assert_eq!(files.len(), 59);
        assert_eq!(files.iter().filter(|f| f.path.starts_with("sys_modules/")).count(), 14);
        let fonts = files.iter().filter(|f| f.path.starts_with("fonts/font/") || f.path.starts_with("fonts/font2/"));
        assert_eq!(fonts.count(), 45);
        assert_eq!(files.iter().map(|f| f.size).sum::<u64>(), 38_055_716);
        for file in &files {
            let folder = file.path.rsplit_once('/').unwrap().0;
            assert!(FOLDERS.contains(&folder), "{} is in a folder shadPS4 reads", file.path);
            assert!(file.sha256.len() == 64 && file.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        }
        let paths: BTreeSet<_> = files.iter().map(|f| &f.path).collect();
        assert_eq!(paths.len(), 59, "no file twice");
    }

    #[test]
    fn a_file_downloads_from_the_pinned_commit_with_its_path_encoded() {
        assert_eq!(
            url("sys_modules/libSceFont.sprx"),
            format!(
                "https://raw.githubusercontent.com/Abdess/retrobios/{COMMIT}/bios/Sony/PlayStation%204/shadps4/sys_modules/libSceFont.sprx"
            )
        );
        assert!(url("fonts/font/A b#.ttf").ends_with("/shadps4/fonts/font/A%20b%23.ttf"));
    }

    #[test]
    fn the_step_has_its_share_and_its_megabytes() {
        let step = Step::at(12_000_000, 38_055_716);
        assert_eq!(step.detail, "12.0 MB of 38.1 MB");
        assert_eq!((step.progress * 100.0) as u32, 31);
    }
}

//! One plugin for several retro consoles on the shared RetroArch: each console is a
//! [`Retro`] of its own, and [`RetroConsoles`] routes the host's calls to it. A game's id is its
//! console's code, `/`, and the console's own id (`NES/Contra`), so the games of two consoles
//! never share an id. The builds, the emulator's settings and its panel are the first console's:
//! every console runs the same RetroArch.

use std::sync::Arc;

use launcher_api::*;
use serde_json::Value;

use crate::games::RomSystem;
use crate::retroarch::{Files, Retro, RetroConsole, backup_items};

/// What one console gives: its code, its game files, and what its [`Retro`] provides.
pub struct ConsolePart {
    console: &'static str,
    system: &'static RomSystem,
    provided: Provided,
    backup: Vec<BackupItem>,
}

impl ConsolePart {
    /// The part of console `C`, with the builds' files from `files`.
    pub async fn of<C: RetroConsole>(ctx: &PluginContext, files: Files) -> PluginResult<Self> {
        Ok(Self {
            console: C::CONSOLE,
            system: C::SYSTEM,
            provided: Retro::<C>::provided_with(ctx, files).await?,
            backup: backup_items::<C>(),
        })
    }
}

/// The items every console lists the same: they are backed up once.
const SHARED_ITEMS: [&str; 2] = ["bios", "retroarch-settings"];

/// A backup item of console `console`: its id begins with the console's code in lowercase
/// (`nes-saves`), but the items every console shares keep theirs.
pub fn item_id(console: &str, item: &str) -> String {
    if SHARED_ITEMS.contains(&item) { item.to_owned() } else { format!("{}-{item}", console.to_ascii_lowercase()) }
}

/// The host's calls, each to its console.
pub struct RetroConsoles {
    parts: Vec<ConsolePart>,
}

impl RetroConsoles {
    /// What the plugin of `parts` provides the host. The first part's builds serve them all.
    pub fn provided(parts: Vec<ConsolePart>) -> PluginResult<Provided> {
        let first = parts.first().ok_or_else(|| PluginError::Other("no console".into()))?;
        let (downloader, installer) = (first.provided.downloader.clone(), first.provided.installer.clone());
        let mut items: Vec<BackupItem> = Vec::new();
        for part in &parts {
            for item in &part.backup {
                let id = item_id(part.console, &item.name);
                if !items.iter().any(|i| i.name == id) {
                    items.push(BackupItem { name: id, ..item.clone() });
                }
            }
        }
        let router = Arc::new(Self { parts });
        Ok(Provided {
            catalog: Some(router.clone()),
            downloader,
            installer,
            configurator: Some(router.clone()),
            launcher: Some(router.clone()),
            game_installer: Some(router.clone()),
            slots: Some(router),
            backup: Some(crate::backup::Items::provided(items)),
            ..Provided::default()
        })
    }

    /// The console of the game `id`, and its own id there.
    fn of<'a>(&self, id: &'a str) -> PluginResult<(&ConsolePart, &'a str)> {
        let (console, own) = id.split_once('/').ok_or_else(|| PluginError::NotFound(id.into()))?;
        let part = self.parts.iter().find(|p| p.console == console).ok_or_else(|| PluginError::NotFound(id.into()))?;
        Ok((part, own))
    }

    fn first(&self) -> &ConsolePart {
        &self.parts[0]
    }

    fn named(part: &ConsolePart, mut game: GameSummary) -> GameSummary {
        game.id = format!("{}/{}", part.console, game.id);
        game
    }
}

fn need<T: ?Sized>(got: &Option<Arc<T>>, what: &str) -> PluginResult<Arc<T>> {
    got.clone().ok_or_else(|| PluginError::Other(format!("a retro console has no {what}")))
}

#[async_trait]
impl Catalog for RetroConsoles {
    /// Every console's games, in the consoles' order, then the page of them.
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let mut all = Vec::new();
        for part in &self.parts {
            let games = need(&part.provided.catalog, "catalog")?.list(PageRequest::ALL).await?;
            all.extend(games.items.into_iter().map(|g| Self::named(part, g)));
        }
        let total = Some(all.len() as u32);
        let items = all.into_iter().skip(page.offset as usize).take(page.limit as usize).collect();
        Ok(Page { items, total })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let mut found = Vec::new();
        for part in &self.parts {
            let games = need(&part.provided.catalog, "catalog")?.search(query).await?;
            found.extend(games.into_iter().map(|g| Self::named(part, g)));
        }
        found.truncate(query.limit as usize);
        Ok(found)
    }

    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let (part, own) = self.of(game)?;
        let mut details = need(&part.provided.catalog, "catalog")?.details(own).await?;
        details.summary = Self::named(part, details.summary);
        Ok(details)
    }

    async fn artwork(&self, game: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        let (part, own) = self.of(game)?;
        need(&part.provided.catalog, "catalog")?.artwork(own, kind).await
    }
}

#[async_trait]
impl Launcher for RetroConsoles {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let (part, own) = self.of(game)?;
        need(&part.provided.launcher, "launcher")?.prepare(own, config).await
    }

    async fn open_emulator(&self, config: &Value) -> PluginResult<Option<LaunchSpec>> {
        need(&self.first().provided.launcher, "launcher")?.open_emulator(config).await
    }

    async fn on_exit(&self, game: &str, exit: &ExitInfo) -> PluginResult<()> {
        let (part, own) = self.of(game)?;
        need(&part.provided.launcher, "launcher")?.on_exit(own, exit).await
    }

    async fn close_left_running(&self) -> PluginResult<()> {
        need(&self.first().provided.launcher, "launcher")?.close_left_running().await
    }
}

impl RetroConsoles {
    /// The console of a scope's settings, and the scope as that console names it.
    fn scope(&self, scope: &ConfigScope) -> PluginResult<(&ConsolePart, ConfigScope)> {
        match scope {
            ConfigScope::Emulator => Ok((self.first(), ConfigScope::Emulator)),
            ConfigScope::Game(game) => {
                let (part, own) = self.of(game)?;
                Ok((part, ConfigScope::Game(own.to_owned())))
            }
        }
    }
}

#[async_trait]
impl Configurator for RetroConsoles {
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let (part, scope) = self.scope(scope)?;
        need(&part.provided.configurator, "configurator")?.schema(&scope).await
    }

    async fn defaults(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let (part, scope) = self.scope(scope)?;
        need(&part.provided.configurator, "configurator")?.defaults(&scope).await
    }

    async fn validate(&self, scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        let (part, scope) = self.scope(scope)?;
        need(&part.provided.configurator, "configurator")?.validate(&scope, value).await
    }
}

#[async_trait]
impl GameInstaller for RetroConsoles {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        let mut out = Vec::new();
        for part in &self.parts {
            out.extend(need(&part.provided.game_installer, "game installer")?.candidates().await?);
        }
        Ok(out)
    }

    /// The console of the folder the source is in (`NES/…` of the packages folder), else the
    /// one console whose games have the source's extension.
    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let path = source.path.as_str();
        let top = path.split('/').next().unwrap_or_default();
        let by_folder = self.parts.iter().find(|p| p.system.folder.eq_ignore_ascii_case(top));
        let name = path.rsplit('/').next().unwrap_or(path);
        let by_name: Vec<&ConsolePart> = self.parts.iter().filter(|p| p.system.is_game(name)).collect();
        let part = match (by_folder, by_name.as_slice()) {
            (Some(part), _) => part,
            (None, [one]) => *one,
            _ => {
                let folders: Vec<&str> = self.parts.iter().map(|p| p.system.folder).collect();
                return Err(PluginError::InvalidInput(format!(
                    "{path}: put it in the folder of its console in a packages folder: {}",
                    folders.join(", ")
                )));
            }
        };
        need(&part.provided.game_installer, "game installer")?.begin(source, job).await
    }
}

#[async_trait]
impl Slots for RetroConsoles {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        match &request.game {
            Some(game) => {
                let (part, own) = self.of(game)?;
                let request = SlotRequest { game: Some(own.to_owned()), ..request.clone() };
                need(&part.provided.slots, "slots")?.items(&request).await
            }
            None => need(&self.first().provided.slots, "slots")?.items(request).await,
        }
    }

    async fn run_action(&self, action: &str, game: Option<&str>, args: &Value) -> PluginResult<()> {
        match game {
            Some(game) => {
                let (part, own) = self.of(game)?;
                need(&part.provided.slots, "slots")?.run_action(action, Some(own), args).await
            }
            None => need(&self.first().provided.slots, "slots")?.run_action(action, None, args).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_console_s_items_begin_with_its_code_and_the_shared_ones_stay_as_they_are() {
        assert_eq!(item_id("NES", "saves"), "nes-saves");
        assert_eq!(item_id("ATARI2600", "states"), "atari2600-states");
        assert_eq!(item_id("GENESIS", "bios"), "bios");
        assert_eq!(item_id("GENESIS", "retroarch-settings"), "retroarch-settings");
    }
}

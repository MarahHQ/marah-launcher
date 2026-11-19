//! The start's installs, which the boot screen shows and waits for: the emulators' builds that
//! install or update at the start ([`crate::emulator_builds::start_pass`]), the installs a crash
//! stopped, and the plugins' own work ([`launcher_api::StartupWork`]), such as the Switch
//! firmware.
//!
//! - It runs once a run, right after the boot reads the emulators. The update checks every few
//!   hours never bring the boot screen back.
//! - The wait ends when nothing runs, or when nothing moved for [`STALL_LIMIT`]: a download that
//!   hangs doesn't keep the player out. What still runs goes on behind Home.
//! - A plugin asks before work that downloads ([`launcher_api::StartupQuestion`]): the wait
//!   shows the question over the boot screen and waits for the answer, one plugin at a time,
//!   once a run each.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use launcher_api::{PluginId, PluginKind, StartupQuestion, StartupState, StartupStep};

use crate::app::Core;
use crate::changes::Area;

/// How often the wait reads the plugins' work.
const READ_EVERY: Duration = Duration::from_millis(500);
/// The wait ends when nothing moved for this long.
const STALL_LIMIT: Duration = Duration::from_secs(120);

/// The start's installs, as the boot screen shows them.
#[derive(Clone, Debug, Default)]
pub struct Startup {
    /// The wait is over: the boot screen can go.
    pub over: bool,
    /// The plugins' own work, as each plugin last said, in the plugins' order.
    pub work: Vec<(PluginId, StartupStep)>,
    /// The emulators the start's pass installs or updates.
    pub emulators: BTreeSet<PluginId>,
    /// The emulators whose install or update failed in the pass.
    pub failed: BTreeSet<PluginId>,
    /// The question shown now, and the plugin that asks it.
    pub question: Option<(PluginId, StartupQuestion)>,
    /// The plugins whose question the player answered in this run: they aren't asked again.
    pub answered: BTreeSet<PluginId>,
}

impl Startup {
    /// A plugin's work runs.
    fn works(&self) -> bool {
        self.work.iter().any(|(_, step)| step.state == StartupState::Working)
    }
}

/// Starts the start's installs, right after the boot read the emulators. When nothing is to
/// install now and no plugin works, the wait is over at once; the pass still runs, for an
/// update that waits for its game to close.
pub(crate) async fn start(core: &Core) {
    let work = read_work(core).await;
    core.state().startup.work = work;
    ask(core).await;
    let emulators = crate::emulator_builds::start_pass_installs(core);
    let waits = !emulators.is_empty()
        || !installing(core).is_empty()
        || core.state().startup.works()
        || core.state().startup.question.is_some();
    core.state().startup.emulators = emulators;
    if !waits {
        core.state().startup.over = true;
    }
    let core = core.clone();
    tokio::spawn(async move {
        let pass_over = Arc::new(AtomicBool::new(false));
        {
            let (core, pass_over) = (core.clone(), pass_over.clone());
            tokio::spawn(async move {
                crate::emulator_builds::start_pass(&core).await;
                pass_over.store(true, Ordering::SeqCst);
            });
        }
        if waits {
            wait(&core, &pass_over).await;
            core.state().startup.over = true;
            core.changed(Area::Library);
        }
    });
}

/// Reads the plugins' work until nothing runs, or nothing moved for [`STALL_LIMIT`].
async fn wait(core: &Core, pass_over: &AtomicBool) {
    let clock = core.ports().clock.clone();
    let mut last = None;
    let mut still_since = clock.now();
    loop {
        let work = read_work(core).await;
        let changed = core.state().startup.work != work;
        if changed {
            core.state().startup.work = work;
            core.changed(Area::Library);
        }
        ask(core).await;
        let installing = installing(core);
        let startup = core.state().startup.clone();
        let asking = startup.question.is_some();
        if pass_over.load(Ordering::SeqCst) && installing.is_empty() && !startup.works() && !asking {
            return;
        }
        let mark = (installing, received(core), startup.work, asking);
        if last.as_ref() != Some(&mark) {
            last = Some(mark);
            still_since = clock.now();
        } else if clock.now().saturating_duration_since(still_since) >= STALL_LIMIT {
            tracing::warn!(
                "the start's installs didn't move for {STALL_LIMIT:?}: Home opens, and they go on behind it"
            );
            return;
        }
        clock.sleep(READ_EVERY).await;
    }
}

/// Shows the next plugin's question, when none is shown: the first plugin, in the plugins'
/// order, that asks one and wasn't answered in this run.
async fn ask(core: &Core) {
    if core.state().startup.question.is_some() {
        return;
    }
    let answered = core.state().startup.answered.clone();
    for plugin in startup_plugins(core) {
        if answered.contains(&plugin) {
            continue;
        }
        let Ok(provided) = core.kernel().provided(&plugin).await else { continue };
        let Some(startup) = provided.startup.clone() else { continue };
        let question = async { Ok::<_, launcher_api::PluginError>(startup.question().await) };
        if let Ok(Some(question)) = core.kernel().guard(&plugin, "startup.question", question).await {
            core.state().startup.question = Some((plugin, question));
            core.changed(Area::Library);
            return;
        }
    }
}

/// The player answered the question shown: the plugin hears it, and isn't asked again in this
/// run. With yes, its work starts, and the boot screen, while it is up, waits for it.
pub(crate) async fn answer(core: &Core, yes: bool) -> crate::CoreResult<()> {
    let Some((plugin, _)) = core.state().startup.question.take() else { return Ok(()) };
    core.state().startup.answered.insert(plugin.clone());
    if let Ok(provided) = core.kernel().provided(&plugin).await
        && let Some(startup) = provided.startup.clone()
    {
        let answer = async {
            startup.answer(yes).await;
            Ok::<_, launcher_api::PluginError>(())
        };
        let _ = core.kernel().guard(&plugin, "startup.answer", answer).await;
    }
    let work = read_work(core).await;
    core.state().startup.work = work;
    core.changed(Area::Library);
    // The next plugin's question, also when the wait already ended: a late answer doesn't
    // keep the other plugins from asking.
    ask(core).await;
    Ok(())
}

/// The plugins with the startup capability, in the plugins' order. Themes have none.
fn startup_plugins(core: &Core) -> Vec<PluginId> {
    let kernel = core.kernel();
    kernel
        .order()
        .iter()
        .filter(|p| {
            kernel.manifest(p).is_ok_and(|m| {
                m.kind != PluginKind::Theme && m.capabilities.contains(&launcher_api::Capability::Startup)
            })
        })
        .cloned()
        .collect()
}

/// Each plugin's work at this start, from the plugins that have some.
async fn read_work(core: &Core) -> Vec<(PluginId, StartupStep)> {
    let kernel = core.kernel();
    let mut work = Vec::new();
    for plugin in startup_plugins(core) {
        let Ok(provided) = kernel.provided(&plugin).await else { continue };
        let Some(startup) = provided.startup.clone() else { continue };
        let status = async { Ok::<_, launcher_api::PluginError>(startup.status().await) };
        if let Ok(Some(step)) = kernel.guard(&plugin, "startup.status", status).await {
            work.push((plugin, step));
        }
    }
    work
}

/// The emulators whose build downloads or installs now.
fn installing(core: &Core) -> Vec<PluginId> {
    core.builds().installing()
}

/// The bytes the downloads have, so a download that moves keeps the wait going.
fn received(core: &Core) -> u64 {
    core.inner.downloads.jobs().iter().map(|j| j.received).sum()
}

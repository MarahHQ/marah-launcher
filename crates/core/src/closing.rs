//! Closing down before Log out, Restart and Shut down: the running games close first, as Stop
//! closes them (asked, then killed when they don't), so none is cut off mid-save; then what
//! their launches left running (Steam's client), through its plugin first. The session then
//! ends at once. Each step shows on the screen (`AppView::closing`): the launcher is
//! going, and the player sees why it waits.

use std::time::Duration;

use launcher_api::{GameKey, PluginId};

use crate::{Area, Core};

/// The step the flow is at, as the screen shows it: its text, and the game it closes when it
/// closes one, so the screen shows that game (its title and art) as its launch did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) text: String,
    pub(crate) game: Option<(GameKey, String)>,
}

/// How often the flow looks whether what it closed has ended.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// What closes down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    LogOut,
    Restart,
    ShutDown,
}

impl Purpose {
    /// The last step, once everything closed.
    fn last_step(self) -> &'static str {
        match self {
            Purpose::LogOut => "Logging out",
            Purpose::Restart => "Restarting",
            Purpose::ShutDown => "Shutting down",
        }
    }
}

/// How long the flow waits for what it asked to close: Stop's grace before it kills, twice, and
/// a margin. Past it, the flow goes on, so nothing holds the machine up.
fn wait_limit(core: &Core) -> Duration {
    core.kernel().host().stop_grace * 2 + Duration::from_secs(2)
}

/// Waits until `done`, for at most `limit` of the core's clock. Whether it is done.
async fn wait_until(core: &Core, limit: Duration, done: impl Fn() -> bool) -> bool {
    let clock = core.ports().clock.clone();
    let deadline = clock.now() + limit;
    while !done() && clock.now() < deadline {
        clock.sleep(LOOK_EVERY).await;
    }
    done()
}

/// Shows `step` while the flow is at it.
fn show_step(core: &Core, step: Step) {
    core.state().closing = Some(step);
    core.changed(Area::Sessions);
}

/// Closes every running game, showing each, then what their launches left running, then shows
/// the last step of `purpose`. A game that won't close is killed after the host's
/// `stop_grace`; past [`wait_limit`], the flow goes on anyway.
pub(crate) async fn close_everything(core: &Core, purpose: Purpose) {
    for (session, game, title) in core.sessions().titles() {
        show_step(core, Step { text: format!("Closing {title}"), game: Some((game, title)) });
        // A game already stopping, or gone, has nothing more to stop: nothing to report.
        if let Err(e) = core.sessions().stop(session) {
            tracing::debug!(%session, error = %e, "the game was already stopping or gone");
        }
    }
    if !wait_until(core, wait_limit(core), || !core.game_running()).await {
        tracing::warn!("a game didn't end before closing down: going on");
    }
    for plugin in core.sessions().left_running() {
        close_left_running(core, &plugin).await;
    }
    show_step(core, Step { text: purpose.last_step().to_owned(), game: None });
}

/// Closes what `plugin`'s launches left running (Steam's client): the plugin is asked first,
/// as it knows how to close it cleanly, then what still runs is ended.
async fn close_left_running(core: &Core, plugin: &PluginId) {
    let clock = core.ports().clock.clone();
    let call_limit = core.kernel().host().call_timeout;
    let name =
        core.kernel().manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_else(|_| plugin.to_string());
    show_step(core, Step { text: format!("Closing {name}"), game: None });
    if let Some(launcher) = core.kernel().provided(plugin).await.ok().and_then(|p| p.launcher) {
        match clock.timeout(call_limit, launcher.close_left_running()).await {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                tracing::warn!(%plugin, error = %e, "the plugin couldn't close what its launch left running")
            }
            None => tracing::warn!(%plugin, "the plugin took too long to close what its launch left running"),
        }
    }
    let ended = || !core.sessions().leaves_running(plugin);
    if wait_until(core, wait_limit(core), ended).await {
        return;
    }
    tracing::info!(%plugin, "what its launch left running didn't close: ending it");
    core.sessions().end_left_running(plugin);
    if !wait_until(core, wait_limit(core), ended).await {
        tracing::warn!(%plugin, "what its launch left running didn't end: going on");
    }
}

/// Log out: closes what runs, then lets the launcher go ([`Core::log_out_ready`]).
pub(crate) async fn log_out(core: &Core) {
    close_everything(core, Purpose::LogOut).await;
    core.state().log_out_ready = true;
    core.changed(Area::Sessions);
}

/// Stops showing a closing step: Restart or Shut down failed, and the launcher stays.
pub(crate) fn cancelled(core: &Core) {
    core.state().closing = None;
    core.changed(Area::Sessions);
}

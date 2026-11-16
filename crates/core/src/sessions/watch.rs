//! A running game, from its spawn to its end. A game is its processes, as the
//! [`ProcessTree`](crate::ProcessTree) port lists them: the one the launcher started, its process
//! group, and what a sandbox moved out of the group below them. The game ends when none of them
//! is left, so a start script that starts the game in the background and exits keeps its
//! session. On the way the module shows the game's window, keeps `game_in_front` up to date,
//! and carries out Stop.
//!
//! A launch can hand the game to another program (`LaunchSpec::game_process`), as
//! `steam steam://rungameid/…` asks Steam's client to start it. The game is then the process
//! with those words in its command line, once it shows, and the processes below it; the program
//! started here is left to itself.

use std::time::Duration;

use launcher_api::SessionId;
use tokio::sync::oneshot;

use super::SessionsSubsystem;
use crate::{Front, ProcessExit, ProcessHandle, ProcessTree};

/// How often the module looks for a started game's window, until it shows.
const SHOW_EVERY: Duration = Duration::from_millis(250);
/// On a desktop where a game ends with its window: the window was gone this many looks in a
/// row. More than one, so a window that is remade (full screen on or off) doesn't count.
const WINDOW_GONE_FOR: u32 = 3;
/// How often Stop looks whether the game closed, within the host's `stop_grace`.
const STOP_EVERY: Duration = Duration::from_millis(50);

/// The exit of a game that a kill ended.
const KILLED: ProcessExit = ProcessExit { code: None, success: false };
/// The exit of a game another program started, which ended by itself: its code is that
/// program's to know.
const ENDED: ProcessExit = ProcessExit { code: None, success: true };

/// The process a session follows as the game's first.
enum Followed {
    /// The one the launcher started.
    Started(Box<dyn ProcessHandle>),
    /// One another program started, found by its command line.
    Found(u32),
}

/// The game's live processes. Once the process the launcher started has exited, its id is no
/// longer the game's: the system may give it to another program. With `env`, a handed-over game
/// whose real processes run detached from the reaper (Steam's runtime reparents them): those its
/// environment names are added, wherever the system put them, so the tree missing them doesn't
/// lose the game.
fn members(
    processes: &dyn ProcessTree,
    root: Option<u32>,
    root_exited: bool,
    env: Option<&(String, String)>,
) -> Vec<u32> {
    // A found process that left gives its id back to the system, which may give it to another
    // program: once the environment names the game, the game is only what it names.
    let root = root.filter(|_| !(env.is_some() && root_exited));
    let mut ids = match root {
        Some(root) => {
            let mut ids = processes.members(root);
            if root_exited {
                ids.retain(|id| *id != root);
            }
            ids
        }
        None => Vec::new(),
    };
    if let Some((key, value)) = env {
        for id in processes.with_env(key, value) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Brings the game started as `root` to the front; `false` when it has no window yet.
pub(super) async fn show(sessions: &SessionsSubsystem, root: Option<u32>, env: Option<(String, String)>) -> bool {
    sessions
        .blocking(move |desktop, processes| {
            // A game named by its environment is shown by it: the found process may have left,
            // and its id gone to another program.
            let ids = members(processes, root, env.is_some(), env.as_ref());
            !ids.is_empty() && desktop.show_game(&ids)
        })
        .await
}

/// A launch that hands the game to another program: the text of the game's command line, and
/// the processes that had it before the launch (the same game started from Steam itself),
/// which aren't the one this launch starts.
pub(super) struct HandOver {
    game_process: String,
    before: Vec<u32>,
    /// The game's processes also carry this environment variable, when the program that runs it
    /// reparents it out of the launched process's tree (`LaunchSpec::game_env`).
    env: Option<(String, String)>,
}

/// The hand-over of a launch whose game has `game_process` in its command line: looked up
/// before the launch starts anything. `env` names the game's processes for a program that
/// reparents them (Steam's runtime).
pub(super) async fn hand_over(
    sessions: &SessionsSubsystem,
    game_process: String,
    env: Option<(String, String)>,
) -> HandOver {
    let looking = game_process.clone();
    let before = sessions.blocking(move |_, processes| processes.find(&looking)).await;
    HandOver { game_process, before, env }
}

/// The session `session` of the game `process` started, until it ends: its first process's
/// exit (or a kill's). `stop` is Stop. With `hand_over`, `process` hands the game over to the
/// process with that text in its command line. `None` when nothing was played: that process
/// never showed within the host's `hand_over_timeout`, or Stop came first.
pub(super) async fn run(
    sessions: SessionsSubsystem,
    session: SessionId,
    process: Box<dyn ProcessHandle>,
    mut stop: oneshot::Receiver<()>,
    hand_over: Option<HandOver>,
) -> Option<ProcessExit> {
    let Some(HandOver { game_process, before, env }) = hand_over else {
        return Some(follow(sessions, session, Followed::Started(process), stop, None).await);
    };
    // Steam's client, when the launch started it, keeps running after the game: it is reaped
    // here. Stop leaves it be; only the end of the player's session (Log out, Restart, Shut
    // down) ends it ([`SessionsSubsystem::end_left_running`]): asked, then killed.
    let id = process.id();
    let end = id.and_then(|id| sessions.leave_running(session, id));
    let reaper = sessions.clone();
    tokio::spawn(async move {
        let mut process = process;
        let waited = match end {
            Some(end) => tokio::select! {
                exit = process.wait() => Some(exit),
                Ok(()) = end => None,
            },
            None => Some(process.wait().await),
        };
        match waited {
            Some(Ok(_)) => {}
            Some(Err(e)) => tracing::debug!(error = %e, "the program that handed the game over wasn't waited for"),
            None => {
                if let Err(e) = process.terminate().await {
                    tracing::warn!(error = %e, "could not ask what the launch left running to close");
                }
                let (clock, grace) = (reaper.clock().clone(), reaper.timings().stop_grace);
                if clock.timeout(grace, process.wait()).await.is_none() {
                    tracing::info!("what the launch left running didn't close: killing it");
                    if let Err(e) = process.kill().await {
                        tracing::warn!(error = %e, "could not kill what the launch left running");
                    }
                    if let Err(e) = process.wait().await {
                        tracing::debug!(error = %e, "what the launch left running wasn't waited for");
                    }
                }
            }
        }
        if let Some(id) = id {
            reaper.ended_left_running(id);
        }
    });
    let (timings, clock) = (sessions.timings(), sessions.clock().clone());
    let deadline = clock.now() + timings.hand_over_timeout;
    loop {
        tokio::select! {
            // Nothing of the game runs yet: nothing to close, and nothing was played.
            _ = &mut stop => return None,
            () = clock.sleep(SHOW_EVERY.min(timings.watch_every)) => {}
        }
        let looking = game_process.clone();
        let found = sessions.blocking(move |_, processes| processes.find(&looking)).await;
        if let Some(id) = found.into_iter().find(|id| !before.contains(id)) {
            tracing::info!(%session, id, "the game another program started is running");
            sessions.found(session, id);
            return Some(follow(sessions, session, Followed::Found(id), stop, env).await);
        }
        if clock.now() >= deadline {
            return None;
        }
    }
}

/// The game whose first process is `followed`, until none of its processes is left. `env` adds
/// the processes a handed-over game's environment names (see [`members`]).
async fn follow(
    sessions: SessionsSubsystem,
    session: SessionId,
    mut followed: Followed,
    mut stop: oneshot::Receiver<()>,
    env: Option<(String, String)>,
) -> ProcessExit {
    let (timings, clock) = (sessions.timings(), sessions.clock().clone());
    let root = match &followed {
        Followed::Started(process) => process.id(),
        Followed::Found(id) => Some(*id),
    };
    let deadline = clock.now() + timings.show_timeout;
    let mut first: Option<ProcessExit> = None;
    let mut shown = false;
    // Only a game whose window was seen can lose it: a desktop that can't see the game's
    // window (a Wayland window, which xdotool can't find) must not take it for closed.
    let mut had_window = false;
    let mut gone = 0;
    let exit = loop {
        let every = if shown { timings.watch_every } else { SHOW_EVERY.min(timings.watch_every) };
        tokio::select! {
            exit = wait(&mut followed), if first.is_none() => {
                first = Some(exit);
            }
            _ = &mut stop => break close(&sessions, &mut followed, root, first, env.clone()).await,
            () = clock.sleep(every) => {}
        }
        let root_exited = first.is_some();
        if !shown {
            // Until the game shows, only its processes and one try to show them: each call
            // may run a helper program.
            let waited = clock.now() >= deadline;
            let env_now = env.clone();
            let (ids, showing, front) = sessions
                .blocking(move |desktop, processes| {
                    let ids = members(processes, root, root_exited, env_now.as_ref());
                    let showing = !ids.is_empty() && desktop.show_game(&ids);
                    let front = if !showing && waited { desktop.front() } else { Front::Unknown };
                    (ids, showing, front)
                })
                .await;
            ended_by_itself(&followed, &ids, &mut first);
            if let Some(exit) = first
                && ids.is_empty()
            {
                break exit;
            }
            if showing || waited {
                shown = true;
                had_window = showing;
                // A desktop that can't say (Windows, or macOS without the Accessibility
                // permission) can't show the game either; the game's own window took the
                // screen as it opened, so the pad is the game's until the PS button.
                sessions.shown(session, showing || front == Front::Unknown);
            }
            continue;
        }
        // The game stays on screen with the windows it opened since it showed: Proton opens the
        // game's own after a helper's, and Gamescope shows only the windows it was told about.
        // Until the player brings the launcher, even when the desktop says the launcher is in
        // front: Gamescope shows the launcher when the helper's window closes.
        let keep = sessions.keeps_game();
        let env_now = env.clone();
        let look = sessions
            .blocking(move |desktop, processes| {
                let ids = members(processes, root, root_exited, env_now.as_ref());
                if keep && !ids.is_empty() {
                    desktop.keep_game(&ids);
                }
                let front = desktop.front();
                let window = if ids.is_empty() { Some(false) } else { desktop.has_window(&ids) };
                Look { ends_with_window: desktop.window_close_ends_game(), front, window, ids }
            })
            .await;
        ended_by_itself(&followed, &look.ids, &mut first);
        if let Some(exit) = first
            && look.ids.is_empty()
        {
            break exit;
        }
        had_window |= look.window == Some(true);
        gone = if had_window && look.window == Some(false) { gone + 1 } else { 0 };
        if look.ends_with_window && gone >= WINDOW_GONE_FOR {
            tracing::info!(%session, "the game's window closed: stopping it");
            if let Err(e) = sessions.stop(session) {
                tracing::warn!(error = %e, "could not stop the game");
            }
            continue;
        }
        let in_front = match look.front {
            Front::Launcher => Some(false),
            Front::Other(front) => Some(look.ids.contains(&front)),
            // The last window the launcher showed counts.
            Front::Unknown => None,
        };
        if let Some(in_front) = in_front {
            sessions.set_in_front(in_front);
        }
    };
    // What the first process left in its group goes with it (on Windows, what it started).
    if let Followed::Started(process) = &mut followed
        && let Err(e) = process.kill().await
    {
        tracing::warn!(error = %e, "could not stop what the game left running");
    }
    exit
}

/// The first process's exit: the launcher's own, waited for. A found one has no handle to wait
/// on: the process list says when it ended ([`ended_by_itself`]).
async fn wait(followed: &mut Followed) -> ProcessExit {
    match followed {
        Followed::Started(process) => process.wait().await.unwrap_or(KILLED),
        Followed::Found(_) => std::future::pending().await,
    }
}

/// A found first process ended when the process list no longer has it.
fn ended_by_itself(followed: &Followed, ids: &[u32], first: &mut Option<ProcessExit>) {
    if let Followed::Found(id) = followed
        && first.is_none()
        && !ids.contains(id)
    {
        *first = Some(ENDED);
    }
}

/// One look at the game: its processes, which window is in front, whether it has a window.
struct Look {
    ids: Vec<u32>,
    front: Front,
    window: Option<bool>,
    ends_with_window: bool,
}

/// Stop: asks the game to close, so the emulator saves its caches, and kills what is left
/// after the host's `stop_grace`.
async fn close(
    sessions: &SessionsSubsystem,
    followed: &mut Followed,
    root: Option<u32>,
    mut first: Option<ProcessExit>,
    env: Option<(String, String)>,
) -> ProcessExit {
    // A launcher's process is waited for, so its id leaves the list once it exited; a found
    // one stays in it while it runs.
    let drop_root = matches!(followed, Followed::Started(_));
    let root_exited = first.is_some();
    let env_now = env.clone();
    let ids = sessions.blocking(move |_, processes| members(processes, root, root_exited, env_now.as_ref())).await;
    if let Followed::Started(process) = followed
        && let Err(e) = process.terminate().await
    {
        tracing::warn!(error = %e, "could not ask the game to close");
    }
    let asked = ids.clone();
    sessions.blocking(move |_, processes| processes.end(&asked, false)).await;
    let clock = sessions.clock();
    let deadline = clock.now() + sessions.timings().stop_grace;
    while clock.now() < deadline {
        let waited = match followed {
            Followed::Started(process) if first.is_none() => clock.timeout(STOP_EVERY, process.wait()).await,
            _ => None,
        };
        match waited {
            Some(Ok(exit)) => first = Some(exit),
            // Timed out: the wait was the pause. Otherwise pause here, or this loop spins.
            None if first.is_none() && drop_root => {}
            _ => clock.sleep(STOP_EVERY).await,
        }
        let env_now = env.clone();
        // A found process that left is no longer the game's, whatever runs under its id now.
        let left_root = drop_root || first.is_some();
        let left = sessions.blocking(move |_, processes| members(processes, root, left_root, env_now.as_ref())).await;
        ended_by_itself(followed, &left, &mut first);
        if let Some(exit) = first
            && left.is_empty()
        {
            return exit;
        }
    }
    tracing::info!("the game didn't close: killing it");
    if let Followed::Started(process) = followed
        && let Err(e) = process.kill().await
    {
        tracing::warn!(error = %e, "could not stop the game");
    }
    let left_root = drop_root || first.is_some();
    let left = sessions.blocking(move |_, processes| members(processes, root, left_root, env.as_ref())).await;
    sessions.blocking(move |_, processes| processes.end(&left, true)).await;
    match (first, followed) {
        (Some(exit), _) => exit,
        (None, Followed::Started(process)) => process.wait().await.unwrap_or(KILLED),
        (None, Followed::Found(_)) => KILLED,
    }
}

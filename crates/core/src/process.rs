//! The real adapters that start and follow programs: [`TokioSpawner`] runs them with
//! `tokio::process`, and [`GroupTree`] follows a game by its process group. The traits they fill
//! are in [`crate::ports`]; the app's `SystemProcesses` is the fuller [`ProcessTree`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use launcher_api::PluginResult;

use crate::ports::{Clock, ProcessExit, ProcessHandle, ProcessSpawner, ProcessTree, SpawnCommand, TokioClock};

/// Spawns real processes with `tokio::process`.
pub struct TokioSpawner;

/// The child, its id kept after the child is reaped (its process group can outlive it), and
/// when it started.
#[cfg_attr(not(windows), allow(dead_code))] // Only Windows' orphan sweep reads the start time.
struct TokioChild(tokio::process::Child, Option<u32>, std::time::SystemTime);

#[async_trait]
impl ProcessSpawner for TokioSpawner {
    async fn spawn(&self, command: SpawnCommand) -> PluginResult<Box<dyn ProcessHandle>> {
        let failed =
            |e: std::io::Error| launcher_api::PluginError::Other(format!("could not start {:?}: {e}", command.program));
        let mut process = tokio::process::Command::new(&command.program);
        process.args(&command.args).envs(&command.env).current_dir(&command.cwd).kill_on_drop(!command.keep_running);
        process.stdin(std::process::Stdio::null());
        if let Some(log) = &command.log {
            if let Some(dir) = log.parent() {
                std::fs::create_dir_all(dir).map_err(failed)?;
            }
            let out = std::fs::File::create(log).map_err(failed)?;
            process.stderr(out.try_clone().map_err(failed)?).stdout(out);
        } else {
            process.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
        #[cfg(unix)]
        process.process_group(0);
        let child = process.spawn().map_err(failed)?;
        let id = child.id();
        Ok(Box::new(TokioChild(child, id, std::time::SystemTime::now())))
    }

    async fn output(&self, command: SpawnCommand, timeout: Duration, limit: usize) -> PluginResult<Option<Vec<u8>>> {
        use tokio::io::AsyncReadExt;
        let mut child = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .envs(&command.env)
            .current_dir(&command.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| launcher_api::PluginError::Other(format!("could not start {:?}: {e}", command.program)))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let run = async {
            let mut out = Vec::new();
            stdout.take(limit as u64).read_to_end(&mut out).await?;
            // The read dropped the pipe: a program with more to say stops on it, not on us.
            child.wait().await?;
            Ok::<_, std::io::Error>(out)
        };
        // A real program runs in real time, whatever clock the core has.
        let clock: &dyn Clock = &TokioClock;
        match clock.timeout(timeout, run).await {
            Some(Ok(out)) => Ok(Some(out)),
            Some(Err(e)) => Err(launcher_api::PluginError::Other(format!("{:?}: {e}", command.program))),
            None => {
                stop(&mut child).await;
                Ok(None)
            }
        }
    }

    async fn stream(
        &self,
        command: SpawnCommand,
        elevated: bool,
        line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> PluginResult<i32> {
        stream_with(command, elevated, line, cancelled, &tokio::process::Child::start_kill).await
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
            vec![format!("{name}.exe"), name.into()]
        } else {
            vec![name.into()]
        };
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).flat_map(|dir| names.iter().map(move |n| dir.join(n))).find(|p| p.is_file())
    }
}

/// What [`TokioSpawner::stream`] writes to an elevated program's input on a cancel it can't do
/// with a kill (docs/contributors/os-contract.md).
const CANCEL_REQUEST: &[u8] = b"cancel\n";

/// [`TokioSpawner::stream`], with the step that kills the program on a cancel: a test makes it
/// fail, as it fails on a program that runs as root.
async fn stream_with(
    command: SpawnCommand,
    elevated: bool,
    line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
    kill: &(dyn Fn(&mut tokio::process::Child) -> std::io::Result<()> + Send + Sync),
) -> PluginResult<i32> {
    use futures::StreamExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let failed = |e: std::io::Error| launcher_api::PluginError::Other(format!("{:?}: {e}", command.program));
    let input = if elevated { std::process::Stdio::piped() } else { std::process::Stdio::null() };
    let mut child = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .envs(&command.env)
        .current_dir(&command.cwd)
        .stdin(input)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| launcher_api::PluginError::Other(format!("could not start {:?}: {e}", command.program)))?;
    let mut input = child.stdin.take();
    let mut out = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    let mut err = BufReader::new(child.stderr.take().expect("stderr is piped")).lines();
    let (mut out_open, mut err_open) = (true, true);
    // Set when a kill failed: the program then runs to its end, and is read to its end.
    let mut unstoppable = false;
    // The cancel request on its way to the program's input. It owns the input and closes it once
    // written: the program gets one request, then the end of its input.
    let mut request: Option<futures::future::BoxFuture<'static, Option<std::io::Result<()>>>> = None;
    let mut tick = TokioClock.interval(Duration::from_millis(250));
    // A program can close its output and run on: the cancel still stops it then.
    loop {
        tokio::select! {
            read = out.next_line(), if out_open => match read.map_err(failed)? {
                Some(text) => line(&text),
                None => out_open = false,
            },
            read = err.next_line(), if err_open => match read.map_err(failed)? {
                Some(text) => line(&text),
                None => err_open = false,
            },
            // Beside the reads: a program that doesn't read its input never holds them up.
            sent = async { request.as_mut().expect("a request").await }, if request.is_some() => {
                request = None;
                let error = match sent {
                    Some(Ok(())) => None,
                    Some(Err(e)) => Some(e.to_string()),
                    None => Some("its input stayed full".to_owned()),
                };
                if let Some(error) = error {
                    tracing::warn!(program = ?command.program, %error, "could not ask the program to stop: it runs to its end");
                }
            },
            status = child.wait(), if !out_open && !err_open => {
                return Ok(status.map_err(failed)?.code().unwrap_or(-1));
            },
            _ = tick.next(), if !unstoppable => if cancelled() {
                match kill(&mut child) {
                    Ok(()) => {
                        wait_for_stop(&mut child).await;
                        // On Unix a killed program has no exit code.
                        return Ok(-1);
                    }
                    // pkexec's program runs as root after the password, and the launcher may
                    // not kill it. A program that stopped reading would block on its full pipe.
                    Err(e) => {
                        unstoppable = true;
                        if let Some(mut input) = input.take() {
                            tracing::warn!(program = ?command.program, error = %e, "could not stop the program on a cancel: asked it to stop on its input");
                            request = Some(Box::pin(async move {
                                let clock: &dyn Clock = &TokioClock;
                                clock.timeout(Duration::from_secs(5), input.write_all(CANCEL_REQUEST)).await
                            }));
                        } else {
                            tracing::warn!(program = ?command.program, error = %e, "could not stop the program on a cancel: it runs to its end");
                        }
                    }
                }
            },
        }
        if !out_open && !err_open {
            // The end of its input, for a program that reads it to its end before it ends.
            (input, request) = (None, None);
        }
    }
}

/// How long a stopped program has to end before its caller stops waiting for it.
const STOP_WAIT: Duration = Duration::from_secs(1);

/// Kills `child`, and waits at most [`STOP_WAIT`] for it to end. For a program that isn't
/// elevated, a failed kill means it has ended.
async fn stop(child: &mut tokio::process::Child) {
    if let Err(e) = child.start_kill() {
        tracing::debug!(error = %e, "the program wasn't killed: it has ended");
    }
    wait_for_stop(child).await;
}

/// Waits at most [`STOP_WAIT`] for a killed `child` to end. A program stuck in the kernel, such
/// as `lsblk` on a dead drive, ends only when the kernel lets it go: the caller doesn't wait for
/// that, and tokio reaps the program later.
async fn wait_for_stop(child: &mut tokio::process::Child) {
    let clock: &dyn Clock = &TokioClock;
    if clock.timeout(STOP_WAIT, child.wait()).await.is_none() {
        tracing::debug!("a killed program hasn't ended yet: it is left to end");
    }
}

/// Sends `signal` to the process group `group` with the system's `kill`: this crate has no
/// unsafe code to call it directly.
#[cfg(unix)]
async fn signal_group(group: u32, signal: &str) -> PluginResult<()> {
    // A failed exit status means the group is already gone, which is what we wanted.
    tokio::process::Command::new("kill")
        .args([&format!("-{signal}"), "--", &format!("-{group}")])
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| launcher_api::PluginError::Other(format!("kill: {e}")))?;
    Ok(())
}

/// Runs `taskkill` on process `id` and the processes it started. Without `force`, it asks
/// their windows to close.
#[cfg(windows)]
async fn taskkill(id: u32, force: bool) -> PluginResult<()> {
    let mut args = vec!["/PID".to_owned(), id.to_string(), "/T".to_owned()];
    if force {
        args.push("/F".into());
    }
    // A failed exit status means the processes are already gone, which is what we wanted.
    tokio::process::Command::new("taskkill")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| launcher_api::PluginError::Other(format!("taskkill: {e}")))?;
    Ok(())
}

/// Kills what process `id` started, and what those started, after `id` ended: `taskkill /T`
/// follows the tree from a running process only. A child keeps its parent's id, so a walk by
/// parent id still finds it. Windows can give a dead process's id to a new one, so the walk
/// takes only processes created since the game `started`: a stranger with a reused parent id
/// is older than that, or not below the game.
#[cfg(windows)]
async fn orphans_of(id: u32, started: std::time::SystemTime) {
    let since = started.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let script = format!(
        "$since = [DateTimeOffset]::FromUnixTimeSeconds({since}).LocalDateTime; \
         $all = Get-CimInstance Win32_Process; $parents = @({id}); $seen = @(); \
         while ($parents.Count -gt 0) {{ \
           $next = @($all | Where-Object {{ $parents -contains $_.ParentProcessId -and $seen -notcontains $_.ProcessId -and $_.CreationDate -ge $since }}); \
           $seen += $next.ProcessId; $parents = @($next.ProcessId) }}; \
         $seen | ForEach-Object {{ Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }}"
    );
    let ran = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    if let Err(e) = ran {
        tracing::debug!(error = %e, "could not look for what the game left running");
    }
}

#[async_trait]
impl ProcessHandle for TokioChild {
    fn id(&self) -> Option<u32> {
        self.0.id()
    }

    async fn wait(&mut self) -> PluginResult<ProcessExit> {
        let status = self.0.wait().await.map_err(|e| launcher_api::PluginError::Other(e.to_string()))?;
        Ok(ProcessExit { code: status.code(), success: status.success() })
    }

    async fn terminate(&mut self) -> PluginResult<()> {
        let Some(id) = self.1 else { return Ok(()) };
        #[cfg(unix)]
        return signal_group(id, "TERM").await;
        #[cfg(windows)]
        return taskkill(id, false).await;
    }

    async fn kill(&mut self) -> PluginResult<()> {
        // By the kept id: after the first process ends, the rest of its group can still run.
        #[cfg(unix)]
        if let Some(id) = self.1 {
            signal_group(id, "KILL").await?;
        }
        #[cfg(windows)]
        if let Some(id) = self.1 {
            taskkill(id, true).await?;
            orphans_of(id, self.2).await;
        }
        match self.0.kill().await {
            // Already gone: the group signal reached it first.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            other => other.map_err(|e| launcher_api::PluginError::Other(e.to_string())),
        }
    }
}

/// The process group alone: `root` while its group has a live process (Unix), nothing on
/// Windows, which has no process groups. The default until the app plugs in a real one. Once
/// the first process exits the core drops `root`, so with this default a game ends with its
/// first process: plug in the app's adapter to follow the group.
pub struct GroupTree;

impl ProcessTree for GroupTree {
    fn members(&self, root: u32) -> Vec<u32> {
        #[cfg(unix)]
        {
            let alive = std::process::Command::new("kill")
                .args(["-0", "--", &format!("-{root}")])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if alive { vec![root] } else { Vec::new() }
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            Vec::new()
        }
    }

    fn end(&self, _ids: &[u32], _force: bool) {}
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Real programs run in real time.
    const REAL: &dyn Clock = &TokioClock;

    fn sh(script: &str) -> SpawnCommand {
        SpawnCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: std::env::temp_dir(),
            log: None,
            keep_running: false,
        }
    }

    /// A program that may become Steam's client outlives its handle, and so Marah Launcher.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_may_outlive_marah_isn_t_stopped_when_its_handle_goes() {
        let mut command = sh("sleep 30");
        command.keep_running = true;
        let process = TokioSpawner.spawn(command).await.unwrap();
        let id = process.id().unwrap().to_string();
        drop(process);
        TokioClock.sleep(Duration::from_millis(200)).await;
        let alive = std::process::Command::new("kill").args(["-0", &id]).status().is_ok_and(|s| s.success());
        let _ = std::process::Command::new("kill").args(["-KILL", &id]).status();
        assert!(alive, "the program was stopped with its handle");
    }

    #[tokio::test]
    async fn output_reads_what_a_program_prints_up_to_the_limit() {
        let out = TokioSpawner.output(sh("echo hello"), Duration::from_secs(5), 1024).await.unwrap();
        assert_eq!(out.as_deref(), Some(&b"hello\n"[..]));
        // More than the limit: the read stops there, and the program stops on its closed pipe.
        let out = TokioSpawner.output(sh("yes"), Duration::from_secs(5), 10).await.unwrap();
        assert_eq!(out.map(|o| o.len()), Some(10));
    }

    #[tokio::test]
    async fn output_stops_a_program_that_runs_past_the_timeout() {
        let started = REAL.now();
        let out = TokioSpawner.output(sh("sleep 30"), Duration::from_millis(200), 1024).await.unwrap();
        assert_eq!(out, None);
        assert!(REAL.now() - started < Duration::from_secs(5));
    }

    async fn alive(pid: &str) -> bool {
        let status =
            tokio::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status();
        status.await.unwrap().success()
    }

    /// A program that writes its process id to `pidfile`, then sleeps as that process.
    fn sleeper(pidfile: &Path) -> SpawnCommand {
        sh(&format!("echo $$ > {}; exec sleep 30", pidfile.display()))
    }

    /// The id `pidfile` holds, once it is written: `None` before then.
    fn written(pidfile: &Path) -> Option<String> {
        std::fs::read_to_string(pidfile).ok().filter(|pid| pid.ends_with('\n')).map(|pid| pid.trim().to_owned())
    }

    /// The id `pidfile` holds. Fails when it isn't written within 5 seconds.
    async fn pid_in(pidfile: &Path) -> String {
        let wait = async {
            loop {
                match written(pidfile) {
                    Some(pid) => break pid,
                    None => TokioClock.sleep(Duration::from_millis(20)).await,
                }
            }
        };
        REAL.timeout(Duration::from_secs(5), wait).await.expect("the program wrote its id")
    }

    #[tokio::test]
    async fn output_kills_the_program_it_stops_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");

        // Long enough for the shell to start and write its id.
        let out = TokioSpawner.output(sleeper(&pidfile), Duration::from_secs(1), 1024).await.unwrap();

        assert_eq!(out, None);
        let pid = pid_in(&pidfile).await;
        assert!(!alive(&pid).await, "the program still runs after the timeout");
    }

    #[tokio::test]
    async fn stream_kills_the_program_when_it_is_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        // Once the program runs as itself.
        let cancelled = || written(&pidfile).is_some();

        let code = TokioSpawner.stream(sleeper(&pidfile), false, &|_| {}, &cancelled).await.unwrap();

        assert_eq!(code, -1);
        let pid = pid_in(&pidfile).await;
        assert!(!alive(&pid).await, "the program still runs after the cancel");
    }

    #[tokio::test]
    async fn stream_kills_a_program_that_closed_its_output_when_it_is_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let quiet = sh(&format!("exec >/dev/null 2>&1; echo $$ > {}; exec sleep 30", pidfile.display()));
        // Once the program runs as itself.
        let cancelled = || written(&pidfile).is_some();

        let run = TokioSpawner.stream(quiet, false, &|_| {}, &cancelled);
        let code = REAL.timeout(Duration::from_secs(5), run).await.expect("the cancel stopped it").unwrap();

        assert_eq!(code, -1);
        let pid = pid_in(&pidfile).await;
        assert!(!alive(&pid).await, "the program still runs after the cancel");
    }

    #[tokio::test]
    async fn terminate_asks_a_program_to_close() {
        let mut process = TokioSpawner.spawn(sh("sleep 30")).await.unwrap();
        assert!(process.id().is_some());

        process.terminate().await.unwrap();

        let exit = REAL.timeout(Duration::from_secs(5), process.wait()).await.unwrap().unwrap();
        assert!(!exit.success);
    }

    #[tokio::test]
    async fn kill_stops_the_programs_a_game_started_too() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child");
        // A start script that ignores the request to close and leaves a program running.
        let script = format!("trap '' TERM; sleep 30 & echo $! > {}; wait", pidfile.display());
        let mut process = TokioSpawner.spawn(sh(&script)).await.unwrap();
        let child = loop {
            match std::fs::read_to_string(&pidfile) {
                Ok(pid) if pid.ends_with('\n') => break pid.trim().to_owned(),
                _ => TokioClock.sleep(Duration::from_millis(20)).await,
            }
        };
        assert!(alive(&child).await);

        process.kill().await.unwrap();
        process.wait().await.unwrap();

        let gone = async {
            while alive(&child).await {
                TokioClock.sleep(Duration::from_millis(20)).await;
            }
        };
        REAL.timeout(Duration::from_secs(5), gone).await.expect("the script's program stopped too");
    }

    #[tokio::test]
    async fn a_program_s_output_goes_to_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("logs/game.log");
        let mut command = sh("echo out; echo err >&2");
        command.log = Some(log.clone());

        let mut process = TokioSpawner.spawn(command).await.unwrap();
        process.wait().await.unwrap();

        assert_eq!(std::fs::read_to_string(log).unwrap(), "out\nerr\n");
    }

    /// Runs `script`, `elevated` or not; the player cancels once it prints `ready`. When
    /// `killable` is false, the kill fails with EPERM, as on pkexec's program after the password.
    /// Its exit code, every line it printed and how often the kill was tried, or `None` when it
    /// didn't end within 20 seconds.
    #[cfg(unix)]
    async fn cancelled_run(script: &str, killable: bool, elevated: bool) -> Option<(i32, Vec<String>, usize)> {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let (lines, cancel, kills) = (Mutex::new(Vec::new()), AtomicBool::new(false), AtomicUsize::new(0));
        let line = |text: &str| {
            if text == "ready" {
                cancel.store(true, Ordering::SeqCst);
            }
            lines.lock().unwrap().push(text.to_owned());
        };
        let cancelled = || cancel.load(Ordering::SeqCst);
        let kill = |child: &mut tokio::process::Child| {
            kills.fetch_add(1, Ordering::SeqCst);
            if killable { child.start_kill() } else { Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)) }
        };
        let run = stream_with(sh(script), elevated, &line, &cancelled, &kill);
        let code = REAL.timeout(Duration::from_secs(20), run).await?.unwrap();
        Some((code, lines.into_inner().unwrap(), kills.load(Ordering::SeqCst)))
    }

    /// The script waits past two of the cancel's checks before it goes on: the kill is tried
    /// once, not at each check.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_can_t_be_killed_is_read_to_its_end_and_its_exit_code_returned() {
        let ended = cancelled_run("echo ready; sleep 0.6; echo last; exit 7", false, false).await;
        let (code, lines, kills) = ended.expect("the run ended");
        assert_eq!((code, lines), (7, vec!["ready".to_owned(), "last".to_owned()]));
        assert_eq!(kills, 1, "one try, one warning");
    }

    /// The installer's side of the request: it reads `cancel` and stops by itself, with its own
    /// last line and exit code.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_elevated_program_that_can_t_be_killed_is_asked_to_stop_on_its_input() {
        let script =
            "echo ready; read -r request; echo \"read $request\"; [ \"$request\" = cancel ] && exit 42; exit 1";
        let (code, lines, kills) = cancelled_run(script, false, true).await.expect("the run ended");
        assert_eq!((code, lines, kills), (42, vec!["ready".to_owned(), "read cancel".to_owned()], 1));
    }

    /// A kill that works is enough: the program's input gets nothing. A program it started
    /// outlives the kill and keeps what came on the input, until the input closes.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_elevated_program_that_can_be_killed_isn_t_asked_on_its_input() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("input");
        let script = format!("exec 3<&0; {{ cat <&3; echo end; }} > '{}' & echo ready; sleep 30", seen.display());
        let (code, lines, kills) = cancelled_run(&script, true, true).await.expect("the run ended");
        assert_eq!((code, lines, kills), (-1, vec!["ready".to_owned()], 1), "killed");
        let read = async {
            loop {
                match std::fs::read_to_string(&seen) {
                    Ok(text) if text.ends_with("end\n") => return text,
                    _ => TokioClock.sleep(Duration::from_millis(20)).await,
                }
            }
        };
        let read = REAL.timeout(Duration::from_secs(10), read).await.expect("the input closed with the run");
        assert_eq!(read, "end\n", "nothing written to the input");
    }

    /// A program that never reads its input, or closed it, still runs to its end and is read
    /// to its end: the request never blocks the run.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_elevated_program_that_ignores_its_input_runs_to_its_end() {
        let script = "echo ready; sleep 0.6; i=0; while [ $i -lt 4000 ]; do \
                      echo \"line $i of what the program prints after the cancel\"; i=$((i+1)); done; exit 7";
        let (code, lines, kills) = cancelled_run(script, false, true).await.expect("the run ended, without blocking");
        assert_eq!((code, lines.len(), kills), (7, 4001, 1));

        let closed = "exec 0<&-; echo ready; sleep 0.6; echo last; exit 7";
        let (code, lines, _) = cancelled_run(closed, false, true).await.expect("the run ended");
        assert_eq!((code, lines), (7, vec!["ready".to_owned(), "last".to_owned()]));
    }

    /// A program that isn't elevated has no input: one that reads it ends at once.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_isn_t_elevated_reads_no_input() {
        let lines = std::sync::Mutex::new(Vec::new());
        let line = |text: &str| lines.lock().unwrap().push(text.to_owned());
        let run = stream_with(sh("cat; echo end"), false, &line, &|| false, &tokio::process::Child::start_kill);
        let code = REAL.timeout(Duration::from_secs(10), run).await.expect("the run ended").unwrap();
        assert_eq!((code, lines.into_inner().unwrap()), (0, vec!["end".to_owned()]));
    }

    /// An elevated program that closed its output and reads its input to its end gets the end
    /// of its input: the run doesn't wait for a program that waits for it.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_elevated_program_that_closed_its_output_gets_the_end_of_its_input() {
        let run = stream_with(
            sh("exec >&- 2>&-; while read -r _; do :; done; exit 3"),
            true,
            &|_| {},
            &|| false,
            &|_| Ok(()),
        );
        let code = REAL.timeout(Duration::from_secs(10), run).await.expect("the run ended").unwrap();
        assert_eq!(code, 3);
    }

    /// More than the pipe holds after the cancel: a run that stopped reading would wait forever.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_can_t_be_killed_and_prints_a_lot_after_the_cancel_doesn_t_block() {
        let script = "echo ready; sleep 0.6; i=0; while [ $i -lt 4000 ]; do \
                      echo \"line $i of what the program prints after the cancel\"; i=$((i+1)); done; \
                      echo last >&2; exit 7";
        let (code, lines, kills) = cancelled_run(script, false, false).await.expect("the run ended, without blocking");
        assert_eq!((code, kills), (7, 1));
        // Its output and its errors come through separate pipes, so their order may mix.
        assert_eq!(lines.len(), 4002, "every line");
        assert!(lines.iter().any(|l| l == "last"), "the last line, from its errors");
    }

    /// A program that can be killed stops at the cancel, as pkexec does at its password prompt.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cancel_stops_a_program_that_can_be_killed() {
        let started = REAL.now();
        let ended = cancelled_run("echo ready; sleep 30; echo never", true, false).await;
        let (code, lines, kills) = ended.expect("the run ended");
        assert_eq!((code, lines, kills), (-1, vec!["ready".to_owned()], 1), "killed, so no exit code");
        assert!(REAL.now() - started < Duration::from_secs(10));
    }

    #[test]
    fn which_finds_a_program_on_the_path() {
        assert!(TokioSpawner.which("sh").is_some_and(|p| p.is_file()));
        assert_eq!(TokioSpawner.which("no-such-program-anywhere"), None);
    }
}

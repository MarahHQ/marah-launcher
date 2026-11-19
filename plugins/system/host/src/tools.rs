//! The tool runner: every area runs its system tools through [`Runner`], on the host's
//! [`Programs`]. Each call has a time limit, and at the limit the host kills the tool before
//! the call returns, so a hung tool doesn't keep running. The errors read the same for every
//! tool:
//!
//! - A missing tool: "NetworkManager (nmcli) isn't installed" ([`PluginError::NotFound`]).
//! - A tool that ran past its limit: "lsblk took too long".
//! - A change that the tool refused: the tool's own message ([`Runner::act`]).

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use launcher_api::{Location, MAX_PROGRAM_OUTPUT, PluginError, PluginResult, Programs, SystemJob};

use crate::parse;

/// A system tool: its program, and the name the player knows it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    pub program: &'static str,
    pub name: &'static str,
}

impl Tool {
    pub const fn new(program: &'static str, name: &'static str) -> Self {
        Self { program, name }
    }

    fn location(self) -> Location {
        Location::System(self.program.into())
    }

    /// Why an area can't be used without it.
    pub fn missing(self) -> String {
        format!("{} ({}) isn't installed", self.name, self.program)
    }

    fn too_long(self) -> PluginError {
        PluginError::Other(format!("{} took too long", self.program))
    }
}

/// How long a read or a quick change may take: the host's own limit for [`Programs::output`].
pub const QUICK: Duration = launcher_api::MAX_PROGRAM_TIME;

/// How long a call may run before the runner has the host kill it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// [`QUICK`].
    Quick,
    After(Duration),
    /// Until the tool ends or the player cancels: for a call that waits on the player, such as
    /// a password prompt, or one that takes as long as its work, such as the OS install.
    Never,
}

/// How a streamed call runs.
pub struct Call<'a> {
    pub limit: Limit,
    /// As the administrator, through polkit.
    pub elevated: bool,
    /// The player's cancel. The runner adds its own at the limit.
    pub cancelled: Box<dyn Fn() -> bool + Send + Sync + 'a>,
}

impl<'a> Call<'a> {
    /// A quick change: [`QUICK`], and no cancel.
    pub fn quick() -> Call<'static> {
        Call { limit: Limit::Quick, elevated: false, cancelled: Box::new(|| false) }
    }

    /// A step of a job, which the player can cancel.
    pub fn job(job: &'a Arc<dyn SystemJob>, limit: Limit) -> Self {
        Call { limit, elevated: false, cancelled: Box::new(move || job.cancelled()) }
    }

    pub fn elevated(self) -> Self {
        Self { elevated: true, ..self }
    }
}

/// How a streamed tool ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ran {
    /// Its exit code: -1 when the player's cancel stopped it.
    pub code: i32,
    /// Every line it wrote, output and errors, up to [`MAX_PROGRAM_OUTPUT`] bytes.
    pub text: String,
    /// Its last line that isn't blank, trimmed.
    pub last: String,
}

/// Runs the system's tools through the host's [`Programs`].
pub struct Runner {
    programs: Arc<dyn Programs>,
    /// [`Limit::Quick`]: [`QUICK`], shorter in tests.
    quick: Duration,
}

impl Runner {
    pub fn new(programs: Arc<dyn Programs>) -> Self {
        Self { programs, quick: QUICK }
    }

    /// Why an area can't be used: the first of `tools` that isn't installed. `None` when all
    /// are.
    pub async fn missing(&self, tools: &[Tool]) -> Option<String> {
        for tool in tools {
            if self.programs.find(&tool.location()).await.ok().flatten().is_none() {
                return Some(tool.missing());
            }
        }
        None
    }

    /// Runs a tool that reads something, for at most [`QUICK`]: what it printed. Its exit code
    /// isn't read; the parsers read what it printed.
    pub async fn read(&self, tool: Tool, args: &[&str]) -> PluginResult<String> {
        let out = self.programs.output(&tool.location(), &owned(args), self.quick).await;
        out.map(|out| String::from_utf8_lossy(&out).into_owned()).map_err(|e| plain(tool, e, false))
    }

    /// Runs a tool line by line, handing each line to `each`, until it ends, `call` cancels it,
    /// or its limit passes. At the limit the host kills it and this fails.
    pub async fn run(
        &self,
        tool: Tool,
        args: &[&str],
        call: Call<'_>,
        each: impl Fn(&str) + Send + Sync,
    ) -> PluginResult<Ran> {
        let limit = match call.limit {
            Limit::Quick => Some(self.quick),
            Limit::After(limit) => Some(limit),
            Limit::Never => None,
        };
        let started = Instant::now();
        let late = AtomicBool::new(false);
        let cancelled = || {
            if limit.is_some_and(|limit| started.elapsed() >= limit) {
                late.store(true, Ordering::Relaxed);
            }
            late.load(Ordering::Relaxed) || (call.cancelled)()
        };
        let printed = Mutex::new(Ran::default());
        let line = |text: &str| {
            {
                let mut printed = printed.lock().expect("not poisoned");
                if !text.trim().is_empty() {
                    printed.last = text.trim().to_owned();
                }
                if printed.text.len() + text.len() < MAX_PROGRAM_OUTPUT {
                    printed.text.push_str(text);
                    printed.text.push('\n');
                }
            }
            each(text);
        };
        let ran = self.programs.stream(&tool.location(), &owned(args), call.elevated, &line, &cancelled).await;
        let code = ran.map_err(|e| plain(tool, e, call.elevated))?;
        if late.load(Ordering::Relaxed) {
            return Err(tool.too_long());
        }
        Ok(Ran { code, ..printed.into_inner().expect("not poisoned") })
    }

    /// Runs a tool that changes something, for at most [`QUICK`]. Fails with the tool's own
    /// message, its last line, when its exit code isn't 0. What it printed.
    pub async fn act(&self, tool: Tool, args: &[&str]) -> PluginResult<String> {
        let ran = self.run(tool, args, Call::quick(), |_| {}).await?;
        if ran.code == 0 { Ok(ran.text) } else { Err(PluginError::Other(parse::error_line(&ran.last))) }
    }
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

/// The host's error for `tool`, in the runner's words. An elevated call's missing program can be
/// pkexec, so its error stays the host's.
fn plain(tool: Tool, error: PluginError, elevated: bool) -> PluginError {
    match error {
        PluginError::NotFound(_) if !elevated => PluginError::NotFound(tool.missing()),
        PluginError::Timeout(_) => tool.too_long(),
        other => other,
    }
}

/// Runs `work` in the background, ending `job` with its outcome.
pub fn job(job: Arc<dyn SystemJob>, work: impl Future<Output = Result<String, String>> + Send + 'static) {
    tokio::spawn(async move {
        match work.await {
            Ok(text) => job.done(&text),
            Err(_) if job.cancelled() => job.failed("Cancelled"),
            Err(reason) => job.failed(&reason),
        }
    });
}

/// An error as a job's plain reason: a missing tool's or a tool's own text, without the kind.
pub fn reason(error: PluginError) -> String {
    match error {
        PluginError::NotFound(text) => text,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSBLK: Tool = Tool::new("lsblk", "util-linux");
    const NMCLI: Tool = Tool::new("nmcli", "NetworkManager");

    /// The host's programs as a test sets them: `installed` are there; `output` prints `out`
    /// with exit code `code`, or never ends when `hangs`. It notes when the host was asked to
    /// stop one, as the real host kills it then.
    #[derive(Default)]
    struct Fake {
        installed: Vec<&'static str>,
        out: &'static str,
        code: i32,
        hangs: bool,
        stopped: AtomicBool,
        ran: Mutex<Vec<String>>,
    }

    impl Fake {
        fn with(installed: &[&'static str]) -> Self {
            Self { installed: installed.to_vec(), ..Self::default() }
        }

        fn has(&self, program: &Location) -> bool {
            self.installed.iter().any(|p| Location::System((*p).into()) == *program)
        }
    }

    #[launcher_api::async_trait]
    impl Programs for Fake {
        async fn find(&self, program: &Location) -> PluginResult<Option<String>> {
            Ok(self.has(program).then(|| format!("/usr/bin/{program}")))
        }

        async fn output(&self, program: &Location, args: &[String], timeout: Duration) -> PluginResult<Vec<u8>> {
            self.ran.lock().unwrap().push(format!("{program} {} in {timeout:?}", args.join(" ")));
            if !self.has(program) {
                return Err(PluginError::NotFound(format!("{program} isn't there")));
            }
            if self.hangs {
                // The real host stops the program at `timeout` and says so.
                self.stopped.store(true, Ordering::SeqCst);
                return Err(PluginError::Timeout(timeout.as_millis() as u64));
            }
            Ok(self.out.as_bytes().to_vec())
        }

        async fn stream(
            &self,
            program: &Location,
            args: &[String],
            elevated: bool,
            line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
            cancelled: &(dyn Fn() -> bool + Send + Sync),
        ) -> PluginResult<i32> {
            self.ran.lock().unwrap().push(format!("{program} {} elevated {elevated}", args.join(" ")));
            if !self.has(program) {
                return Err(PluginError::NotFound(format!("{program} isn't there")));
            }
            self.out.lines().for_each(line);
            if !self.hangs {
                return Ok(self.code);
            }
            // As the real host: it asks `cancelled` every so often, and kills the program then.
            while !cancelled() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.stopped.store(true, Ordering::SeqCst);
            Ok(-1)
        }
    }

    fn runner(fake: Fake) -> (Runner, Arc<Fake>) {
        let fake = Arc::new(fake);
        (Runner::new(fake.clone()), fake)
    }

    #[tokio::test]
    async fn a_tool_that_runs_past_its_limit_is_stopped_and_took_too_long() {
        let (runner, fake) = runner(Fake { hangs: true, ..Fake::with(&["nmcli"]) });
        let call = Call { limit: Limit::After(Duration::from_millis(50)), ..Call::quick() };

        let started = Instant::now();
        let error = runner.run(NMCLI, &["connection", "up", "id", "wg-se"], call, |_| {}).await.unwrap_err();

        assert_eq!(error, PluginError::Other("nmcli took too long".into()));
        assert!(fake.stopped.load(Ordering::SeqCst), "the host wasn't asked to stop it");
        assert!(Instant::now() - started < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_quick_change_has_a_limit() {
        let (mut runner, fake) = runner(Fake { hangs: true, ..Fake::with(&["nmcli"]) });
        assert_eq!(runner.quick, QUICK);
        runner.quick = Duration::from_millis(50);
        let run = runner.act(NMCLI, &["connection", "delete", "id", "home"]);
        let ended = tokio::time::timeout(Duration::from_secs(5), run).await;
        assert_eq!(
            ended.expect("the change ended at its limit"),
            Err(PluginError::Other("nmcli took too long".into()))
        );
        assert!(fake.stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_read_that_runs_past_its_limit_took_too_long() {
        let (runner, fake) = runner(Fake { hangs: true, ..Fake::with(&["lsblk"]) });

        let error = runner.read(LSBLK, &["--json"]).await.unwrap_err();

        assert_eq!(error, PluginError::Other("lsblk took too long".into()));
        assert_eq!(*fake.ran.lock().unwrap(), [format!("lsblk (system) --json in {QUICK:?}")]);
    }

    #[tokio::test]
    async fn a_missing_tool_says_which_one_and_its_name() {
        let (runner, _) = runner(Fake::with(&["udisksctl"]));
        let udisks = Tool::new("udisksctl", "UDisks");

        assert_eq!(runner.missing(&[udisks, LSBLK]).await.as_deref(), Some("util-linux (lsblk) isn't installed"));
        assert_eq!(runner.missing(&[udisks]).await, None);
        let missing = PluginError::NotFound("util-linux (lsblk) isn't installed".into());
        assert_eq!(runner.read(LSBLK, &[]).await, Err(missing.clone()));
        assert_eq!(runner.run(LSBLK, &[], Call::quick(), |_| {}).await, Err(missing.clone()));
        assert_eq!(reason(missing), "util-linux (lsblk) isn't installed");
    }

    #[tokio::test]
    async fn a_refused_change_fails_with_the_tool_s_message() {
        let out = "Connecting\nError: Connection activation failed: no secrets\n";
        let (runner, _) = runner(Fake { out, code: 4, ..Fake::with(&["nmcli"]) });

        let error = runner.act(NMCLI, &["connection", "up", "id", "wg-se"]).await.unwrap_err();

        assert_eq!(error, PluginError::Other("Connection activation failed: no secrets".into()));
        assert_eq!(reason(error), "Connection activation failed: no secrets");
    }

    #[tokio::test]
    async fn a_streamed_tool_hands_back_its_code_its_lines_and_its_last_line() {
        let (runner, fake) = runner(Fake { out: "one\n  two  \n\n", code: 3, ..Fake::with(&["nmcli"]) });
        let seen = Mutex::new(Vec::new());

        let ran = runner.run(NMCLI, &["x"], Call::quick().elevated(), |l| seen.lock().unwrap().push(l.to_owned()));

        assert_eq!(ran.await, Ok(Ran { code: 3, text: "one\n  two  \n\n".into(), last: "two".into() }));
        assert_eq!(*seen.lock().unwrap(), ["one", "  two  ", ""]);
        assert_eq!(*fake.ran.lock().unwrap(), ["nmcli (system) x elevated true"]);
    }

    #[tokio::test]
    async fn the_player_s_cancel_stops_a_tool_with_no_limit() {
        let (runner, fake) = runner(Fake { hangs: true, ..Fake::with(&["nmcli"]) });
        let cancel = AtomicBool::new(false);
        let cancelled = || cancel.load(Ordering::SeqCst);
        let call = Call { limit: Limit::Never, elevated: false, cancelled: Box::new(cancelled) };

        let run = runner.run(NMCLI, &["x"], call, |_| {});
        let stop = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.store(true, Ordering::SeqCst);
        };
        let (ran, ()) = tokio::join!(run, stop);

        // A cancel isn't "took too long": the job says it was cancelled.
        assert_eq!(ran.map(|r| r.code), Ok(-1));
        assert!(fake.stopped.load(Ordering::SeqCst));
    }
}

//! When the fetch of an address (a catalog address or the sources address) and the refresh of
//! a catalog run: the due-time and back-off rules, with no clock of their own. The plugin's
//! task passes the time in, and runs what the plan says.
//!
//! - A copy is used for [`Timing::fresh`] after it was made. Then its work is due again.
//! - A try that failed holds its work: it doesn't run again before the hold's time. The first
//!   hold is [`Timing::retry`], then each one is twice as long, at most the fresh time. A hold
//!   is of one thing (an address, a base copy): work of something else starts again.
//! - A refresh that waits for the VPN is held the same way, but each pass may run it: it asks
//!   the VPN gate again, which asks no forum. While it still waits before its time, the time
//!   stays.
//! - **Refresh now** runs a refresh that is fresh or held.
//!
//! [`Schedule`] keeps these rules for each job of the plugin's task (the fetch of the sources
//! address, and the fetch and the refresh of each console): its hold, and why its last try
//! didn't work. It answers whether a job is due now and when the task wakes next. It reads no
//! copy: the task passes in when each copy was made.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// How long a copy is used, and how long the first hold is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// How long a copy is used before its work is due again. Also the longest hold.
    pub fresh: Duration,
    /// How long the first hold of a try that failed or waits is.
    pub retry: Duration,
}

/// Work that failed or waits for the VPN, and when it tries again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hold {
    /// What the work was of: the address for a fetch, the base copy for a refresh.
    pub of: String,
    /// It waits for the VPN: a pass asks the VPN gate again before the hold's time.
    pub waiting: bool,
    /// When it tries again.
    pub at: SystemTime,
    /// How long it waited this time.
    pub delay: Duration,
}

/// One fetch or refresh, as the schedule sees it.
#[derive(Clone, Copy, Debug)]
struct Work<'a> {
    /// What the work is of: the address for a fetch, the base copy for a refresh.
    pub of: &'a str,
    /// When the work made its copy of `of`. `None` with no such copy.
    pub made: Option<SystemTime>,
    /// The hold of the last try, if any. A hold of something other than `of` doesn't count.
    pub held: Option<&'a Hold>,
    /// **Refresh now** asked for it.
    pub forced: bool,
}

impl Work<'_> {
    /// The hold of this work's `of`.
    fn hold(&self) -> Option<&Hold> {
        self.held.filter(|h| h.of == self.of)
    }
}

/// What to do with one console's work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    /// Run it in this pass.
    pub run: bool,
    /// When it is due with no ask: its hold's time, else when its copy gets too old. `None`
    /// with neither.
    pub wake: Option<SystemTime>,
}

/// How a try ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Done,
    /// It didn't run: the VPN isn't connected.
    Waiting,
    Failed,
}

/// Whether a copy made at `made` is still used at `now`.
fn fresh(now: SystemTime, made: Option<SystemTime>, timing: &Timing) -> bool {
    made.is_some_and(|made| now < made + timing.fresh)
}

/// What to do with `work` at `now`.
fn next(now: SystemTime, work: &Work, timing: &Timing) -> Plan {
    let hold = work.hold();
    let held = hold.is_some_and(|h| now < h.at && !h.waiting);
    let run = work.forced || !(fresh(now, work.made, timing) || held);
    // With no ask, it is due once its copy is too old and its hold is over.
    let stale = work.made.map(|made| made + timing.fresh);
    let wake = match (stale, hold) {
        (Some(stale), Some(hold)) => Some(stale.max(hold.at)),
        (stale, hold) => stale.or(hold.map(|h| h.at)),
    };
    Plan { run, wake }
}

/// The hold after a try of `work` at `now` that ended with `outcome`: none after a try that
/// worked.
fn hold_after(now: SystemTime, work: &Work, outcome: Outcome, timing: &Timing) -> Option<Hold> {
    let waiting = match outcome {
        Outcome::Done => return None,
        Outcome::Waiting => true,
        Outcome::Failed => false,
    };
    let before = work.hold();
    if waiting
        && !work.forced
        && let Some(before) = before.filter(|h| h.waiting && now < h.at)
    {
        return Some(before.clone());
    }
    let delay = match before {
        Some(before) => before.delay.saturating_mul(2).min(timing.fresh),
        None => timing.retry.min(timing.fresh),
    };
    Some(Hold { of: work.of.to_owned(), waiting, at: now + delay, delay })
}

/// How long the task waits for its next pass with no ask: until the soonest of `wakes`, and at
/// least a second. With the refresh on, at most the fresh time. `None` when only an ask wakes
/// it.
fn sleep(
    now: SystemTime,
    wakes: impl IntoIterator<Item = Option<SystemTime>>,
    refresh_on: bool,
    timing: &Timing,
) -> Option<Duration> {
    let fresh = refresh_on.then_some(now + timing.fresh);
    let soonest = wakes.into_iter().flatten().chain(fresh).min()?;
    Some(soonest.duration_since(now).unwrap_or_default().max(Duration::from_secs(1)))
}

/// Which job of the plugin's task: the key [`Schedule`] keeps its hold and its reason under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// The fetch of the sources address.
    Sources,
    /// The fetch of the catalog address of the console with this file name.
    Fetch(String),
    /// The refresh of the catalog of the console with this file name.
    Refresh(String),
}

/// One job, as the plugin's task sees it now.
#[derive(Clone, Copy, Debug)]
pub struct Job<'a> {
    pub key: &'a Key,
    /// What the job is of: the address for a fetch, the base copy for a refresh.
    pub of: &'a str,
    /// When the job made its copy of `of`. `None` with no such copy.
    pub made: Option<SystemTime>,
    /// **Refresh now** asked for it.
    pub forced: bool,
}

/// How a job's try ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    Done,
    /// It didn't run: the VPN isn't connected. With why.
    Waiting(String),
    /// With why.
    Failed(String),
}

/// Why a job's last try didn't work.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reason {
    /// The address of a fetch. `None` for a refresh: its reason is of its console, from any
    /// base copy.
    of: Option<String>,
    words: String,
}

/// What the schedule keeps of a job's last try.
#[derive(Clone, Debug, Default)]
struct Last {
    hold: Option<Hold>,
    reason: Option<Reason>,
}

/// The hold and the reason of each job of the plugin's task, and the rules above over them.
#[derive(Debug)]
pub struct Schedule {
    timing: Timing,
    jobs: HashMap<Key, Last>,
}

impl Schedule {
    pub fn new(timing: Timing) -> Self {
        Self { timing, jobs: HashMap::new() }
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// `job` with the hold of its last try.
    fn work<'a>(&'a self, job: &Job<'a>) -> Work<'a> {
        let held = self.jobs.get(job.key).and_then(|last| last.hold.as_ref());
        Work { of: job.of, made: job.made, held, forced: job.forced }
    }

    /// Whether `job` runs in a pass at `now`.
    pub fn due(&self, now: SystemTime, job: &Job) -> bool {
        next(now, &self.work(job), &self.timing).run
    }

    /// Keeps how a try of `job` ended at `now`: its next hold and its reason. `true` when its
    /// reason changed: new words, a fetch's words for another address, or a reason gone after a
    /// try that worked. A refresh's same words for another base copy are no change.
    pub fn ended(&mut self, now: SystemTime, job: &Job, end: End) -> bool {
        let (outcome, words) = match end {
            End::Done => (Outcome::Done, None),
            End::Waiting(words) => (Outcome::Waiting, Some(words)),
            End::Failed(words) => (Outcome::Failed, Some(words)),
        };
        let hold = hold_after(now, &self.work(job), outcome, &self.timing);
        let of = match job.key {
            Key::Refresh(_) => None,
            Key::Sources | Key::Fetch(_) => Some(job.of.to_owned()),
        };
        let reason = words.map(|words| Reason { of, words });
        let last = self.jobs.entry(job.key.clone()).or_default();
        let changed = last.reason != reason;
        *last = Last { hold, reason };
        if last.hold.is_none() && last.reason.is_none() {
            self.jobs.remove(job.key);
        }
        changed
    }

    /// Why the last try of the job `key` didn't work. For a fetch, only while it is of `of`, its
    /// address now; a refresh's reason is read with `None`.
    pub fn why(&self, key: &Key, of: Option<&str>) -> Option<&str> {
        let reason = self.jobs.get(key)?.reason.as_ref()?;
        (reason.of.as_deref() == of).then_some(reason.words.as_str())
    }

    /// How long the task waits for its next pass with no ask, with `jobs` the work it has with
    /// no ask (see [`sleep`]).
    pub fn sleep<'a>(
        &self,
        now: SystemTime,
        jobs: impl IntoIterator<Item = Job<'a>>,
        refresh_on: bool,
    ) -> Option<Duration> {
        let wakes = jobs.into_iter().map(|job| next(now, &self.work(&job), &self.timing).wake);
        sleep(now, wakes.collect::<Vec<_>>(), refresh_on, &self.timing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    const TIMING: Timing = Timing { fresh: Duration::from_secs(1000), retry: Duration::from_secs(300) };

    /// `secs` seconds after 1970.
    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// A case of [`next`]: what it is, when the copy was made, the hold, Refresh now, and the
    /// plan's `run` and `wake`. Times are in seconds since 1970.
    type DueCase = (&'static str, Option<u64>, Option<Hold>, bool, bool, Option<u64>);

    /// A case of [`hold_after`]: what it is, the hold before, Refresh now, how the try ended,
    /// and the hold after as `(waiting, at, delay)` in seconds.
    type HoldCase = (&'static str, Option<Hold>, bool, Outcome, Option<(bool, u64, u64)>);

    fn hold(of: &str, waiting: bool, secs: u64) -> Hold {
        Hold { of: of.into(), waiting, at: at(secs), delay: Duration::from_secs(300) }
    }

    #[test]
    fn work_runs_when_it_is_due() {
        let now = at(10_000);
        let cases: [DueCase; 12] = [
            ("no copy and no hold", None, None, false, true, None),
            ("a fresh copy", Some(9_500), None, false, false, Some(10_500)),
            ("a copy just the fresh time old", Some(9_000), None, false, true, Some(10_000)),
            ("a copy over the fresh time old", Some(8_000), None, false, true, Some(9_000)),
            ("a fresh copy and Refresh now", Some(9_500), None, true, true, Some(10_500)),
            ("held before its time", None, Some(hold("a", false, 10_200)), false, false, Some(10_200)),
            ("held at its time", None, Some(hold("a", false, 10_000)), false, true, Some(10_000)),
            ("held before its time and Refresh now", None, Some(hold("a", false, 10_200)), true, true, Some(10_200)),
            ("waiting for the VPN before its time", None, Some(hold("a", true, 10_200)), false, true, Some(10_200)),
            ("held for something else", None, Some(hold("b", false, 10_200)), false, true, None),
            (
                "held, with a copy over the fresh time old",
                Some(8_000),
                Some(hold("a", false, 10_200)),
                false,
                false,
                Some(10_200),
            ),
            ("held, with a fresh copy", Some(9_500), Some(hold("a", false, 9_900)), false, false, Some(10_500)),
        ];
        for (what, made, held, forced, run, wake) in cases {
            let work = Work { of: "a", made: made.map(at), held: held.as_ref(), forced };
            assert_eq!(next(now, &work, &TIMING), Plan { run, wake: wake.map(at) }, "{what}");
        }
    }

    #[test]
    fn a_held_try_waits_twice_as_long_each_time_at_most_the_fresh_time() {
        let (first, most, now) = (Duration::from_secs(300), Duration::from_secs(1000), at(10_000));
        let timing = Timing { fresh: most, retry: first };
        fn work<'a>(held: Option<&'a Hold>, of: &'a str) -> Work<'a> {
            Work { of, made: None, held, forced: false }
        }
        let one = hold_after(now, &work(None, "a"), Outcome::Failed, &timing).unwrap();
        assert_eq!((one.delay, one.at), (first, now + first));
        let two = hold_after(now, &work(Some(&one), "a"), Outcome::Failed, &timing).unwrap();
        assert_eq!(two.delay, Duration::from_secs(600));
        let three = hold_after(now, &work(Some(&two), "a"), Outcome::Waiting, &timing).unwrap();
        assert_eq!((three.delay, three.waiting), (most, true), "at most the fresh time");
        let other = hold_after(now, &work(Some(&three), "b"), Outcome::Failed, &timing).unwrap();
        assert_eq!(other.delay, first, "a new base copy starts again");
        let short = Timing { fresh: Duration::from_secs(1), retry: first };
        assert_eq!(hold_after(now, &work(None, "a"), Outcome::Failed, &short).unwrap().delay, Duration::from_secs(1));
    }

    #[test]
    fn a_try_sets_the_next_hold() {
        let now = at(10_000);
        let cases: [HoldCase; 7] = [
            ("a try that worked", Some(hold("a", false, 9_000)), false, Outcome::Done, None),
            ("the first failure", None, false, Outcome::Failed, Some((false, 10_300, 300))),
            (
                "a failure after a hold",
                Some(hold("a", false, 9_900)),
                false,
                Outcome::Failed,
                Some((false, 10_600, 600)),
            ),
            (
                "still waiting before its time",
                Some(hold("a", true, 10_200)),
                false,
                Outcome::Waiting,
                Some((true, 10_200, 300)),
            ),
            (
                "still waiting at its time",
                Some(hold("a", true, 10_000)),
                false,
                Outcome::Waiting,
                Some((true, 10_600, 600)),
            ),
            (
                "waiting after Refresh now",
                Some(hold("a", true, 10_200)),
                true,
                Outcome::Waiting,
                Some((true, 10_600, 600)),
            ),
            (
                "waiting after a failure",
                Some(hold("a", false, 10_200)),
                true,
                Outcome::Waiting,
                Some((true, 10_600, 600)),
            ),
        ];
        for (what, held, forced, outcome, after) in cases {
            let work = Work { of: "a", made: None, held: held.as_ref(), forced };
            let want = after.map(|(waiting, secs, delay)| Hold {
                of: "a".into(),
                waiting,
                at: at(secs),
                delay: Duration::from_secs(delay),
            });
            assert_eq!(hold_after(now, &work, outcome, &TIMING), want, "{what}");
        }
    }

    #[test]
    fn the_task_sleeps_until_the_soonest_wake_and_at_least_a_second() {
        let now = at(10_000);
        let secs = Duration::from_secs;
        assert_eq!(sleep(now, [Some(at(10_500)), None, Some(at(10_200))], false, &TIMING), Some(secs(200)));
        assert_eq!(sleep(now, [Some(at(9_000))], false, &TIMING), Some(secs(1)), "a wake gone by");
        assert_eq!(sleep(now, [Some(at(10_000))], false, &TIMING), Some(secs(1)));
        assert_eq!(sleep(now, [None], false, &TIMING), None, "only an ask wakes it");
        assert_eq!(sleep(now, [], true, &TIMING), Some(secs(1000)), "with the refresh on, the fresh time");
        assert_eq!(sleep(now, [Some(at(12_000))], true, &TIMING), Some(secs(1000)));
        assert!(fresh(now, Some(at(9_001)), &TIMING));
        assert!(!fresh(now, Some(at(9_000)), &TIMING));
        assert!(!fresh(now, None, &TIMING));
    }

    /// The job of `key`, of `of`, with its copy made at `made` seconds and Refresh now `forced`.
    fn job<'a>(key: &'a Key, of: &'a str, made: Option<u64>, forced: bool) -> Job<'a> {
        Job { key, of, made: made.map(at), forced }
    }

    fn failed() -> End {
        End::Failed("HTTP 404".into())
    }

    #[test]
    fn a_job_is_due_with_no_copy_or_a_stale_one_and_a_forced_refresh_always() {
        let schedule = Schedule::new(TIMING);
        let now = at(10_000);
        let refresh = Key::Refresh("ps5".into());
        for key in [Key::Sources, Key::Fetch("ps5".into()), refresh.clone()] {
            assert!(schedule.due(now, &job(&key, "a", None, false)), "{key:?} with no copy");
            assert!(!schedule.due(now, &job(&key, "a", Some(9_500), false)), "{key:?} with a fresh copy");
            assert!(schedule.due(now, &job(&key, "a", Some(9_000), false)), "{key:?} with a stale copy");
        }
        assert!(schedule.due(now, &job(&refresh, "a", Some(9_500), true)), "Refresh now");
    }

    #[test]
    fn refresh_now_runs_a_held_refresh_and_a_forced_wait_takes_a_new_hold() {
        let mut schedule = Schedule::new(TIMING);
        let key = Key::Refresh("ps5".into());
        let (plain, forced) = (job(&key, "kept:1", None, false), job(&key, "kept:1", None, true));
        schedule.ended(at(10_000), &plain, End::Failed("Failed: x".into()));
        assert!(!schedule.due(at(10_100), &plain), "held");
        assert!(schedule.due(at(10_100), &forced), "Refresh now runs it");
        // Waiting after Refresh now, before the hold's time: a new hold, twice as long.
        schedule.ended(at(10_100), &forced, End::Waiting("Waiting for the VPN: down".into()));
        assert_eq!(schedule.sleep(at(10_100), [plain], false), Some(Duration::from_secs(600)));
        // Waiting with no Refresh now, before the hold's time: the hold keeps its time.
        schedule.ended(at(10_200), &plain, End::Waiting("Waiting for the VPN: down".into()));
        assert_eq!(schedule.sleep(at(10_200), [plain], false), Some(Duration::from_secs(500)));
    }

    #[test]
    fn a_failed_job_waits_twice_as_long_each_time_at_most_the_fresh_time() {
        let mut schedule = Schedule::new(TIMING);
        let key = Key::Fetch("ps5".into());
        let fetch = job(&key, "https://a/ps5.json", None, false);
        schedule.ended(at(10_000), &fetch, failed());
        assert!(!schedule.due(at(10_299), &fetch), "held for the first wait");
        assert!(schedule.due(at(10_300), &fetch));
        schedule.ended(at(10_300), &fetch, failed());
        assert!(!schedule.due(at(10_899), &fetch), "held twice as long");
        assert!(schedule.due(at(10_900), &fetch));
        schedule.ended(at(10_900), &fetch, failed());
        assert!(schedule.due(at(11_900), &fetch), "at most the fresh time");
        // Another job's hold is its own.
        assert!(schedule.due(at(10_000), &job(&Key::Fetch("ps4".into()), "https://a/ps4.json", None, false)));
        assert!(schedule.due(at(10_000), &job(&Key::Sources, "https://a/ps5.json", None, false)));
    }

    #[test]
    fn a_hold_of_another_address_or_base_copy_does_not_count_and_one_that_worked_is_gone() {
        let mut schedule = Schedule::new(TIMING);
        let key = Key::Sources;
        schedule.ended(at(10_000), &job(&key, "https://a", None, false), failed());
        assert!(schedule.due(at(10_000), &job(&key, "https://b", None, false)), "a new address");
        let refresh = Key::Refresh("ps5".into());
        schedule.ended(at(10_000), &job(&refresh, "kept:1", None, false), End::Failed("Failed: x".into()));
        assert!(schedule.due(at(10_000), &job(&refresh, "kept:2", None, false)), "a new base copy");
        // A held refresh of the new copy starts again from the first wait.
        schedule.ended(at(10_000), &job(&refresh, "kept:2", None, false), End::Failed("Failed: x".into()));
        assert!(schedule.due(at(10_300), &job(&refresh, "kept:2", None, false)));
        // A try that worked ends the hold.
        schedule.ended(at(10_000), &job(&key, "https://a", None, false), End::Done);
        assert!(schedule.due(at(10_000), &job(&key, "https://a", None, false)));
    }

    #[test]
    fn a_refresh_that_waits_for_the_vpn_runs_each_pass_and_keeps_its_time() {
        let mut schedule = Schedule::new(TIMING);
        let key = Key::Refresh("ps5".into());
        let refresh = job(&key, "kept:1", None, false);
        let waiting = || End::Waiting("Waiting for the VPN: down".into());
        schedule.ended(at(10_000), &refresh, waiting());
        assert!(schedule.due(at(10_010), &refresh), "it asks the gate again");
        schedule.ended(at(10_010), &refresh, waiting());
        // Still before its time: the wake stays at the first hold's time.
        assert_eq!(schedule.sleep(at(10_010), [refresh], false), Some(Duration::from_secs(290)));
        // At its time, the next wait is twice as long.
        schedule.ended(at(10_300), &refresh, waiting());
        assert_eq!(schedule.sleep(at(10_300), [refresh], false), Some(Duration::from_secs(600)));
    }

    #[test]
    fn a_fetch_s_reason_is_of_its_address() {
        let mut schedule = Schedule::new(TIMING);
        let key = Key::Fetch("ps5".into());
        let (a, b) = (job(&key, "https://a", None, false), job(&key, "https://b", None, false));
        assert!(schedule.ended(at(10_000), &a, failed()), "a first reason");
        assert_eq!(schedule.why(&key, Some("https://a")), Some("HTTP 404"));
        assert_eq!(schedule.why(&key, Some("https://b")), None, "not of the address now");
        assert!(!schedule.ended(at(10_300), &a, failed()), "the same words");
        assert!(schedule.ended(at(10_300), &b, failed()), "the same words of another address");
        assert!(schedule.ended(at(10_300), &b, End::Failed("HTTP 500".into())), "new words");
        assert!(schedule.ended(at(10_900), &b, End::Done), "the reason is gone");
        assert_eq!(schedule.why(&key, Some("https://b")), None);
        assert!(!schedule.ended(at(10_900), &b, End::Done), "no reason before");
    }

    #[test]
    fn a_refresh_s_reason_is_of_its_console_from_any_base_copy() {
        let mut schedule = Schedule::new(TIMING);
        let key = Key::Refresh("ps5".into());
        let words = || End::Waiting("Waiting for the VPN: down".into());
        assert!(schedule.ended(at(10_000), &job(&key, "kept:1", None, false), words()));
        assert!(!schedule.ended(at(10_000), &job(&key, "kept:2", None, false), words()), "a new base copy");
        assert_eq!(schedule.why(&key, None), Some("Waiting for the VPN: down"));
        assert_eq!(schedule.why(&Key::Refresh("ps4".into()), None), None);
        assert!(schedule.ended(at(10_000), &job(&key, "kept:2", None, false), End::Failed("Failed: x".into())));
        assert_eq!(schedule.why(&key, None), Some("Failed: x"));
    }

    #[test]
    fn the_task_wakes_for_the_soonest_job() {
        let mut schedule = Schedule::new(TIMING);
        let (sources, fetch) = (Key::Sources, Key::Fetch("ps5".into()));
        let now = at(10_000);
        let kept = [job(&sources, "https://s", Some(9_500), false), job(&fetch, "https://a", Some(9_200), false)];
        assert_eq!(schedule.sleep(now, kept, false), Some(Duration::from_secs(200)), "the older copy");
        schedule.ended(now, &job(&fetch, "https://a", None, false), failed());
        let held = [job(&sources, "https://s", Some(9_500), false), job(&fetch, "https://a", None, false)];
        assert_eq!(schedule.sleep(now, held, false), Some(Duration::from_secs(300)), "the held try");
        assert_eq!(schedule.sleep(now, [], false), None, "only an ask wakes it");
        assert_eq!(schedule.sleep(now, [], true), Some(Duration::from_secs(1000)), "with the refresh on");
    }
}

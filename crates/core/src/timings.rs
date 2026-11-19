//! How long the reads for a render take, and how long the UI thread waits for the core's state
//! lock. The app logs them for the first seconds of a run; the timing tests print them.
//!
//! Each figure is a few atomics, so the counting costs a render next to nothing.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// A count of timed runs: how many, their total time and the longest.
#[derive(Default)]
pub(crate) struct Stat {
    count: AtomicU64,
    total_ns: AtomicU64,
    worst_ns: AtomicU64,
}

impl Stat {
    /// Adds one run that took `took`.
    pub(crate) fn add(&self, took: Duration) {
        let ns = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.worst_ns.fetch_max(ns, Ordering::Relaxed);
    }

    /// Runs `work`, and adds its time.
    pub(crate) fn time<T>(&self, work: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let out = work();
        self.add(started.elapsed());
        out
    }

    fn read(&self) -> Timed {
        Timed {
            count: self.count.load(Ordering::Relaxed),
            total: Duration::from_nanos(self.total_ns.load(Ordering::Relaxed)),
            worst: Duration::from_nanos(self.worst_ns.load(Ordering::Relaxed)),
        }
    }
}

/// The core's timings since it started.
#[derive(Default)]
pub(crate) struct Timings {
    /// [`crate::Core::snapshot`], whole.
    pub snapshots: Stat,
    /// The disk reads of a snapshot ([`crate::snapshot::files`]).
    pub files: Stat,
    /// [`crate::Core::view`]: the snapshot and the build.
    pub views: Stat,
    /// The UI thread's waits for the state lock, when another thread held it.
    pub ui_lock_waits: Stat,
    /// The thread the UI runs on, once the app says.
    pub ui_thread: OnceLock<ThreadId>,
}

impl Timings {
    pub(crate) fn on_ui_thread(&self) -> bool {
        self.ui_thread.get().is_some_and(|ui| *ui == std::thread::current().id())
    }

    pub(crate) fn read(&self) -> CoreTimings {
        CoreTimings {
            snapshots: self.snapshots.read(),
            files: self.files.read(),
            views: self.views.read(),
            ui_lock_waits: self.ui_lock_waits.read(),
        }
    }
}

/// How many timed runs there were, their total time and the longest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timed {
    pub count: u64,
    pub total: Duration,
    pub worst: Duration,
}

impl Timed {
    /// The mean time of a run; zero with none.
    pub fn mean(&self) -> Duration {
        u32::try_from(self.count).ok().filter(|n| *n > 0).map_or(Duration::ZERO, |n| self.total / n)
    }

    /// The runs since `before`: the count and the total. The longest stays the longest of all.
    pub fn since(&self, before: &Timed) -> Timed {
        Timed { count: self.count - before.count, total: self.total.saturating_sub(before.total), worst: self.worst }
    }
}

/// The core's timings since it started ([`crate::Core::timings`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreTimings {
    /// The reads of the whole app for a render.
    pub snapshots: Timed,
    /// The part of those reads that goes to the disk.
    pub files: Timed,
    /// The views built: each a snapshot and a build.
    pub views: Timed,
    /// The UI thread's waits for the core's state lock while another thread held it.
    pub ui_lock_waits: Timed,
}

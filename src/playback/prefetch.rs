//! Work done ahead of the statement that needs it, on a thread of its own.
//!
//! A `.ORS` file is a timeline, so what a script will start in the next
//! couple of seconds is known now. Starting it is not free: opening a movie
//! measured 12-28ms on a Snapdragon 8 Gen 2 and decoding a sound's head
//! 14-20ms, both on the thread that draws, both on the pass that cuts — which
//! left gaps of up to 62ms between passes, over a 24 fps frame's 41.7ms.
//! [`Prefetch`] runs that work on a worker before the statement's frame, and
//! the statement collects it.
//!
//! **A miss costs nothing over not having prefetched.** Whatever the worker
//! has not begun when its statement comes due is cancelled, and the caller
//! does the work itself as it always did. Whatever the worker is part-way
//! through is waited for, since it has already spent some of the time the
//! caller would spend. And whatever comes due without being collected — a
//! seek that jumped past it, a statement that failed before it asked — is
//! dropped, because a prepared movie holds its whole file and a sound can
//! hold a minute of samples.
//!
//! The work itself is the caller's: this module knows keys, frames and a
//! queue, and nothing of movies or sounds. See
//! [`crate::playback::stage::Stage::look_ahead`].

use days_script::Frame;
use std::collections::HashMap;
use std::hash::Hash;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};

type Job = Box<dyn FnOnce() + Send>;

/// Work queued for a worker thread, by key, and collected by key.
pub struct Prefetch<K, T> {
    /// The worker's queue. Started with the first request, so a stage that
    /// never looks ahead — every headless tool — never has a thread.
    jobs: Option<mpsc::Sender<Job>>,
    pending: HashMap<K, Pending<T>>,
}

struct Pending<T> {
    /// The latest frame a statement wanting this starts on. Past it, nothing
    /// will collect the result.
    due: Frame,
    ticket: Arc<Ticket<T>>,
}

struct Ticket<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

enum State<T> {
    Queued,
    Working,
    Ready(T),
    /// Collected, cancelled before the worker reached it, or the work
    /// panicked: either way there is nothing to hand over.
    Gone,
}

impl<T> Ticket<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        // A panic in the work is caught before it can poison this; a poisoned
        // lock would mean a panic in the few lines below, and the state they
        // leave is still a state.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Moves a queued ticket to working, or says it was cancelled.
    fn begin(&self) -> bool {
        let mut state = self.lock();
        if matches!(*state, State::Queued) {
            *state = State::Working;
            true
        } else {
            false
        }
    }

    fn finish(&self, out: Option<T>) {
        *self.lock() = out.map_or(State::Gone, State::Ready);
        self.ready.notify_all();
    }

    /// The result, waiting for it if the worker is on it, or `None` — and the
    /// worker told not to bother — if it has not started.
    fn take(&self) -> Option<T> {
        let mut state = self.lock();
        loop {
            match std::mem::replace(&mut *state, State::Gone) {
                State::Ready(out) => return Some(out),
                State::Queued | State::Gone => return None,
                State::Working => {
                    *state = State::Working;
                    state = self
                        .ready
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }

    /// Blocks until the worker has finished with this, for a test that wants
    /// a result rather than a race with it.
    #[cfg(test)]
    fn settle(&self) {
        let mut state = self.lock();
        while matches!(*state, State::Queued | State::Working) {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Tells the worker not to start this. One it has started runs on, and
    /// its result is dropped with the last reference to the ticket.
    fn cancel(&self) {
        let mut state = self.lock();
        if matches!(*state, State::Queued) {
            *state = State::Gone;
        }
    }
}

impl<K: Hash + Eq, T: Send + 'static> Prefetch<K, T> {
    pub fn new() -> Self {
        Prefetch {
            jobs: None,
            pending: HashMap::new(),
        }
    }

    /// Whether `key` is queued, being worked on, or ready.
    pub fn contains(&self, key: &K) -> bool {
        self.pending.contains_key(key)
    }

    /// Queues `work` under `key` for a statement starting at `due`. A key
    /// already queued keeps its work and is only kept for longer, so two
    /// statements asking for the same thing share one result.
    ///
    /// No thread to be had is a warning and no prefetch: the statement will do
    /// the work itself, as it would have anyway.
    pub fn request(&mut self, key: K, due: Frame, work: impl FnOnce() -> T + Send + 'static) {
        if let Some(pending) = self.pending.get_mut(&key) {
            pending.due = pending.due.max(due);
            return;
        }
        let Some(jobs) = self.worker() else { return };
        let ticket = Arc::new(Ticket {
            state: Mutex::new(State::Queued),
            ready: Condvar::new(),
        });
        let worker_ticket = ticket.clone();
        let job: Job = Box::new(move || {
            if !worker_ticket.begin() {
                return;
            }
            // A panic in the work must still wake a statement waiting on it.
            let out = panic::catch_unwind(AssertUnwindSafe(work)).ok();
            worker_ticket.finish(out);
        });
        if jobs.send(job).is_err() {
            // The worker is gone; it has nothing to report and never will.
            self.jobs = None;
            return;
        }
        self.pending.insert(key, Pending { due, ticket });
    }

    fn worker(&mut self) -> Option<&mpsc::Sender<Job>> {
        if self.jobs.is_none() {
            let (send, receive) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new()
                .name("prefetch".into())
                .spawn(move || {
                    for job in receive {
                        job();
                    }
                });
            match spawned {
                Ok(_) => self.jobs = Some(send),
                Err(err) => log::warn!("no thread to prepare statements ahead on: {err}"),
            }
        }
        self.jobs.as_ref()
    }

    /// The result queued under `key`: waited for if the worker is on it, and
    /// `None` if it was never asked for or the worker has not reached it yet —
    /// in which case it never will, and the caller does the work instead.
    pub fn take(&mut self, key: &K) -> Option<T> {
        self.pending.remove(key)?.ticket.take()
    }

    /// Waits for the worker to finish `key`. See [`Ticket::settle`].
    #[cfg(test)]
    fn settle(&self, key: &K) {
        if let Some(pending) = self.pending.get(key) {
            pending.ticket.settle();
        }
    }

    /// Drops everything due at or before `at`: nothing is going to collect it.
    pub fn forget_until(&mut self, at: Frame) {
        self.pending.retain(|_, pending| {
            let keep = pending.due > at;
            if !keep {
                pending.ticket.cancel();
            }
            keep
        });
    }

    /// Drops everything, for a stage starting over.
    pub fn clear(&mut self) {
        for pending in self.pending.values() {
            pending.ticket.cancel();
        }
        self.pending.clear();
    }
}

impl<K: Hash + Eq, T: Send + 'static> Default for Prefetch<K, T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn a_finished_result_is_collected_once() {
        let mut prefetch = Prefetch::new();
        prefetch.request("a", Frame(10), || 7);
        prefetch.settle(&"a");
        assert_eq!(prefetch.take(&"a"), Some(7));
        assert_eq!(prefetch.take(&"a"), None);
    }

    #[test]
    fn work_the_worker_has_not_reached_is_cancelled_and_never_runs() {
        let mut prefetch = Prefetch::new();
        let (release, gate) = channel::<()>();
        let (started, on_it) = channel::<()>();
        let ran = Arc::new(AtomicU32::new(0));
        // Holds the worker so the second job is still queued when taken.
        prefetch.request("block", Frame(1), move || {
            started.send(()).unwrap();
            gate.recv().ok();
            0
        });
        on_it.recv().unwrap();
        let counter = ran.clone();
        prefetch.request("b", Frame(2), move || {
            counter.fetch_add(1, Ordering::SeqCst);
            1
        });
        assert_eq!(prefetch.take(&"b"), None);
        release.send(()).unwrap();
        assert_eq!(prefetch.take(&"block"), Some(0));
        // The queue is in order, so a job behind "b" finishing means "b" was
        // passed over rather than still to come.
        prefetch.request("c", Frame(3), || 3);
        prefetch.settle(&"c");
        assert_eq!(prefetch.take(&"c"), Some(3));
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn work_in_progress_is_waited_for() {
        let mut prefetch = Prefetch::new();
        let (started, on_it) = channel::<()>();
        prefetch.request("slow", Frame(1), move || {
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            5
        });
        on_it.recv().unwrap();
        assert_eq!(prefetch.take(&"slow"), Some(5));
    }

    #[test]
    fn a_panic_in_the_work_is_a_miss_and_not_a_hang() {
        let mut prefetch = Prefetch::<&str, u32>::new();
        prefetch.request("bad", Frame(1), || panic!("deliberate"));
        prefetch.request("good", Frame(1), || 2);
        prefetch.settle(&"bad");
        prefetch.settle(&"good");
        assert_eq!(prefetch.take(&"bad"), None);
        assert_eq!(prefetch.take(&"good"), Some(2));
    }

    #[test]
    fn a_repeated_key_shares_one_result_and_lives_to_its_last_statement() {
        let mut prefetch = Prefetch::new();
        prefetch.request("s", Frame(10), || 1);
        prefetch.request("s", Frame(30), || 2);
        prefetch.forget_until(Frame(20));
        assert!(prefetch.contains(&"s"));
        prefetch.settle(&"s");
        assert_eq!(prefetch.take(&"s"), Some(1));
    }

    #[test]
    fn whatever_comes_due_uncollected_is_dropped() {
        let mut prefetch = Prefetch::new();
        prefetch.request("early", Frame(10), || 1);
        prefetch.request("late", Frame(40), || 2);
        prefetch.forget_until(Frame(10));
        assert!(!prefetch.contains(&"early"));
        assert!(prefetch.contains(&"late"));
        prefetch.request("again", Frame(50), || 3);
        prefetch.clear();
        assert_eq!(prefetch.take(&"late"), None);
        assert_eq!(prefetch.take(&"again"), None);
    }
}

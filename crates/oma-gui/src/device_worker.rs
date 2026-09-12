//! One bounded reader thread per device. The sampler asks every worker for a
//! reading, then collects what has arrived within a short budget. A device
//! that does not answer keeps exactly one request outstanding: nothing is
//! sent to it again until that returns, so a wedged USB cooler holds one
//! thread, never a growing pile, and every other device keeps its cadence.
//! When the late reading finally arrives it is used, and the next request
//! goes out on the following cycle.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

pub struct Worker<R> {
    name: String,
    requests: Sender<()>,
    results: Receiver<R>,
    /// A request is outstanding: the thread is reading (or stuck).
    busy: bool,
    stalled_since: Option<Instant>,
    last_ok: Option<Instant>,
}

/// What a collection cycle found for one worker.
#[derive(Debug, PartialEq)]
pub enum Poll<R> {
    Ready(R),
    /// Nothing arrived in time; the outstanding request stays out.
    Stalled,
    /// The thread is gone (its reader panicked).
    Dead,
}

impl<R: Send + 'static> Worker<R> {
    /// Start the thread; `read` produces one reading per request.
    pub fn spawn(name: impl Into<String>, mut read: impl FnMut() -> R + Send + 'static) -> Self {
        let name = name.into();
        let (requests, rx_req) = mpsc::channel::<()>();
        let (tx_res, results) = mpsc::channel::<R>();
        let thread_name = format!("oma-read-{name}");
        let spawned = std::thread::Builder::new().name(thread_name).spawn(move || {
            while rx_req.recv().is_ok() {
                if tx_res.send(read()).is_err() {
                    break;
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(device = %name, error = %e, "cannot start a reader thread");
        }
        Self { name, requests, results, busy: false, stalled_since: None, last_ok: None }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ask for a reading unless one is still outstanding.
    pub fn request(&mut self) {
        if !self.busy && self.requests.send(()).is_ok() {
            self.busy = true;
        }
    }

    /// Wait up to `wait` for the outstanding reading.
    pub fn collect(&mut self, wait: Duration, now: Instant) -> Poll<R> {
        if !self.busy {
            return Poll::Stalled;
        }
        match self.results.recv_timeout(wait) {
            Ok(r) => {
                self.busy = false;
                if let Some(since) = self.stalled_since.take() {
                    tracing::info!(device = %self.name, stalled_s = now.duration_since(since).as_secs(), "device answering again");
                }
                self.last_ok = Some(now);
                Poll::Ready(r)
            }
            Err(RecvTimeoutError::Timeout) => {
                if self.stalled_since.is_none() {
                    tracing::warn!(device = %self.name, "device not answering; its readings are held until it does");
                    self.stalled_since = Some(now);
                }
                Poll::Stalled
            }
            Err(RecvTimeoutError::Disconnected) => Poll::Dead,
        }
    }

    pub fn healthy(&self) -> bool {
        self.stalled_since.is_none()
    }

    /// How long the device has been silent, if it is.
    pub fn stalled_for(&self, now: Instant) -> Option<Duration> {
        self.stalled_since.map(|s| now.duration_since(s))
    }

    pub fn last_ok(&self) -> Option<Instant> {
        self.last_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_stuck_device_holds_one_request_and_recovers() {
        // The read blocks until the test releases it.
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let (g, calls) = (gate.clone(), Arc::new(Mutex::new(0u32)));
        let c = calls.clone();
        let mut w = Worker::spawn("aio", move || {
            *c.lock().unwrap() += 1;
            let _held_by_reader = g.lock().unwrap();
            42u32
        });
        let now = Instant::now();
        w.request();
        assert_eq!(w.collect(Duration::from_millis(30), now), Poll::Stalled);
        assert!(!w.healthy());
        // More cycles: no further request goes out while one is outstanding.
        for _ in 0..3 {
            w.request();
            assert_eq!(w.collect(Duration::from_millis(10), now), Poll::Stalled);
        }
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(*calls.lock().unwrap(), 1, "one read in flight, not a pile");
        // The device answers.
        drop(held);
        let later = now + Duration::from_secs(3);
        assert_eq!(w.collect(Duration::from_millis(500), later), Poll::Ready(42));
        assert!(w.healthy());
        assert_eq!(w.last_ok(), Some(later));
        // And the next cycle reads again.
        w.request();
        assert_eq!(w.collect(Duration::from_millis(500), later), Poll::Ready(42));
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[test]
    fn a_healthy_device_answers_every_cycle() {
        let mut n = 0;
        let mut w = Worker::spawn("board", move || {
            n += 1;
            n
        });
        let now = Instant::now();
        for expect in 1..=5 {
            w.request();
            assert_eq!(w.collect(Duration::from_millis(500), now), Poll::Ready(expect));
        }
        assert!(w.stalled_for(now).is_none());
    }

    #[test]
    fn nothing_requested_means_nothing_to_collect() {
        let mut w = Worker::spawn("idle", || 1);
        assert_eq!(w.collect(Duration::from_millis(10), Instant::now()), Poll::Stalled);
        assert!(w.healthy(), "not asked is not stalled");
    }
}

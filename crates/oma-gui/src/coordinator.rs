//! Profile coordination: which profile the user asked for, which apply is
//! running, what is waiting behind it, and what the last apply did.
//!
//! One apply runs at a time. A request that arrives while one runs is queued
//! (only the newest is kept) and the running one is told to stop at its next
//! checkpoint, so conflicting writes never race and the latest request wins.
//! A report from an apply that was replaced is discarded: it must not claim an
//! old profile is in place, and nothing acts on it.

use crate::apply::{Origin, Report};
use std::time::{Duration, Instant};

/// An apply that has not reported back after this long is assumed lost; a new
/// request no longer waits behind it.
pub const LOST_AFTER: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: uuid::Uuid,
    pub name: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlight {
    pub generation: u64,
    pub request: Request,
    pub since: Instant,
}

/// What the last completed apply did.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub request: Request,
    pub report: Report,
    pub at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run it now under this generation.
    Start(u64),
    /// An apply is running; this one starts when it reports back.
    Queued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finish {
    /// The report is for the current apply: act on it.
    Accepted,
    /// A newer request replaced this apply: discard the report.
    Superseded,
    /// Accepted, and the queued request starts now under this generation.
    StartNext(u64, Request),
}

/// What to show about profile application right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// An apply is running, and maybe one waits behind it.
    Applying { name: String, then: Option<String> },
    /// The last apply left something undone; the profile can be sent again.
    Failed { id: uuid::Uuid, name: String, failed: Vec<String>, applied: usize, age: Duration },
    /// The last apply went through.
    Ok { name: String, applied: usize, skipped: Vec<String>, age: Duration },
}

impl Status {
    /// A few words for a pill.
    pub fn short(&self) -> String {
        match self {
            Status::Applying { name, then: Some(next) } => format!("applying {name}, then {next}"),
            Status::Applying { name, then: None } => format!("applying {name}"),
            Status::Failed { failed, .. } => format!("{} failed", failed.len()),
            Status::Ok { skipped, .. } if !skipped.is_empty() => format!("applied, {} skipped", skipped.len()),
            Status::Ok { .. } => "applied".into(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Coordinator {
    next_generation: u64,
    in_flight: Option<InFlight>,
    queued: Option<Request>,
    pub last: Option<Outcome>,
}

impl Coordinator {
    /// The newest generation handed out: an apply whose generation is older
    /// has been replaced and stops at its next checkpoint.
    pub fn latest_generation(&self) -> u64 {
        self.next_generation
    }

    #[cfg(test)]
    pub fn in_flight(&self) -> Option<&InFlight> {
        self.in_flight.as_ref()
    }

    #[cfg(test)]
    pub fn queued(&self) -> Option<&Request> {
        self.queued.as_ref()
    }

    #[cfg(test)]
    /// The profile the user (or automation) most recently asked for.
    pub fn requested(&self) -> Option<&Request> {
        self.queued.as_ref().or(self.in_flight.as_ref().map(|f| &f.request))
    }

    pub fn status(&self, now: Instant) -> Option<Status> {
        if let Some(f) = &self.in_flight {
            return Some(Status::Applying { name: f.request.name.clone(), then: self.queued.as_ref().map(|q| q.name.clone()) });
        }
        let last = self.last.as_ref()?;
        let age = now.saturating_duration_since(last.at);
        Some(if last.report.failed.is_empty() {
            Status::Ok { name: last.request.name.clone(), applied: last.report.applied.len(), skipped: last.report.skipped.clone(), age }
        } else {
            Status::Failed { id: last.request.id, name: last.request.name.clone(), failed: last.report.failed.clone(), applied: last.report.applied.len(), age }
        })
    }

    pub fn request(&mut self, request: Request, now: Instant) -> Action {
        self.next_generation += 1;
        let lost = self.in_flight.as_ref().is_some_and(|f| now.duration_since(f.since) >= LOST_AFTER);
        if lost {
            tracing::warn!(profile = %self.in_flight.as_ref().map(|f| f.request.name.clone()).unwrap_or_default(), "an apply never reported back; not waiting for it");
            self.in_flight = None;
        }
        if self.in_flight.is_some() {
            // Only the newest intent waits; the running apply sees the newer
            // generation and stops.
            self.queued = Some(request);
            return Action::Queued;
        }
        self.in_flight = Some(InFlight { generation: self.next_generation, request, since: now });
        Action::Start(self.next_generation)
    }

    pub fn finished(&mut self, generation: u64, report: Report, now: Instant) -> Finish {
        let Some(flight) = self.in_flight.take_if(|f| f.generation == generation) else {
            return Finish::Superseded;
        };
        self.last = Some(Outcome { request: flight.request, report, at: now });
        match self.queued.take() {
            Some(next) => {
                self.next_generation += 1;
                self.in_flight = Some(InFlight { generation: self.next_generation, request: next.clone(), since: now });
                Finish::StartNext(self.next_generation, next)
            }
            None => Finish::Accepted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str) -> Request {
        Request { id: uuid::Uuid::new_v4(), name: name.into(), origin: Origin::Manual }
    }

    #[test]
    fn the_latest_request_wins_and_old_reports_are_discarded() {
        let mut c = Coordinator::default();
        let t0 = Instant::now();
        let a = req("A");
        let Action::Start(gen_a) = c.request(a.clone(), t0) else { panic!("A runs at once") };
        assert_eq!(c.request(req("B"), t0), Action::Queued);
        let cc = req("C");
        assert_eq!(c.request(cc.clone(), t0), Action::Queued, "C replaces B in the queue");
        assert_eq!(c.queued().map(|r| r.name.as_str()), Some("C"));
        assert!(c.latest_generation() > gen_a, "A has been told to stop");
        assert_eq!(c.requested().map(|r| r.name.as_str()), Some("C"));
        // A reports (stopped early); C starts.
        let Finish::StartNext(gen_c, next) = c.finished(gen_a, Report { superseded: true, ..Default::default() }, t0) else { panic!("C should start") };
        assert_eq!(next, cc);
        assert!(gen_c > gen_a);
        // A late duplicate report for A is ignored.
        assert_eq!(c.finished(gen_a, Report::default(), t0), Finish::Superseded);
        // C completes: it is the outcome shown.
        let mut done = Report::default();
        done.applied.push("power mode performance".into());
        assert_eq!(c.finished(gen_c, done.clone(), t0), Finish::Accepted);
        assert_eq!(c.last.as_ref().map(|o| (&o.request.name, &o.report)), Some((&"C".to_string(), &done)));
        assert!(c.in_flight().is_none());
        assert!(c.queued().is_none());
    }

    #[test]
    fn status_follows_the_apply() {
        let mut c = Coordinator::default();
        let t0 = Instant::now();
        assert_eq!(c.status(t0), None);
        let a = req("A");
        let Action::Start(g) = c.request(a.clone(), t0) else { panic!() };
        c.request(req("B"), t0);
        assert_eq!(c.status(t0).map(|s| s.short()).as_deref(), Some("applying A, then B"));
        let Finish::StartNext(g2, _) = c.finished(g, Report::default(), t0) else { panic!() };
        let mut r = Report::default();
        r.failed.push("NVIDIA: busy".into());
        assert_eq!(c.finished(g2, r, t0), Finish::Accepted);
        match c.status(t0 + Duration::from_secs(5)) {
            Some(Status::Failed { name, failed, age, .. }) => {
                assert_eq!((name.as_str(), failed.len(), age.as_secs()), ("B", 1, 5));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_lost_apply_does_not_block_the_next_request() {
        let mut c = Coordinator::default();
        let t0 = Instant::now();
        let Action::Start(_) = c.request(req("A"), t0) else { panic!() };
        assert_eq!(c.request(req("B"), t0 + LOST_AFTER), Action::Start(2));
        assert_eq!(c.in_flight().map(|f| f.request.name.as_str()), Some("B"));
    }

    #[test]
    fn a_report_from_an_unknown_generation_is_superseded() {
        let mut c = Coordinator::default();
        assert_eq!(c.finished(7, Report::default(), Instant::now()), Finish::Superseded);
        assert!(c.last.is_none());
    }
}

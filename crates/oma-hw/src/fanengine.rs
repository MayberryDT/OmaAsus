//! Software fan engine: evaluates profile fan curves against live
//! temperatures and produces per-target duty commands with hysteresis and
//! ramp limiting. Pure logic (no I/O) so it is unit-testable; the GUI/daemon
//! turns [`Command`]s into writes and reports each outcome back through
//! [`FanEngine::report`], so the engine never assumes a write landed.

use crate::profile::{CoolingSettings, FanCurve, FanMode, FanTarget, TempSource};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Pause before a failed write, or an unconfirmed release, is tried again.
pub const RETRY: Duration = Duration::from_secs(5);
/// A curve whose temperature can't be read holds its duty this long, then
/// runs at its top.
pub const BLIND_LIMIT: Duration = Duration::from_secs(30);

/// Temperatures the engine can read this tick.
#[derive(Debug, Clone, Default)]
pub struct Temps {
    pub cpu_tctl: Option<f64>,
    pub cpu_package: Option<f64>,
    pub gpu: Option<f64>,
    pub coolant: Option<f64>,
    pub vrm: Option<f64>,
    pub board: Option<f64>,
    /// (driver, label) → °C for arbitrary hwmon sources.
    pub hwmon: BTreeMap<(String, String), f64>,
    /// The GPU is there but not being read right now: its temperature is
    /// unknown, not absent, so curves that include it hold instead of
    /// following the rest.
    pub gpu_unread: bool,
}

impl Temps {
    pub fn resolve(&self, src: &TempSource) -> Option<f64> {
        match src {
            TempSource::CpuTctl => self.cpu_tctl.or(self.cpu_package),
            TempSource::CpuPackage => self.cpu_package.or(self.cpu_tctl),
            TempSource::Gpu => self.gpu,
            TempSource::Coolant => self.coolant,
            TempSource::Vrm => self.vrm,
            TempSource::Motherboard => self.board,
            TempSource::CpuGpuMax if self.gpu_unread => None,
            TempSource::CpuGpuMax => match (self.cpu_tctl.or(self.cpu_package), self.gpu) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            },
            TempSource::Hwmon { driver, label } => self.hwmon.get(&(driver.clone(), label.clone())).copied(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub target: FanTarget,
    /// Duty 0..=100, or `None` to hand control back to firmware/driver.
    pub duty: Option<f64>,
    /// Hardware curve to program (nct6775 Smart Fan IV), if the mode asks for it.
    pub hw_curve: Option<FanCurve>,
    /// Issue order per engine: a backend drops a command an older number
    /// than the newest it has seen for the target, and the engine ignores
    /// the result of one, so a slow write can never land after a newer one.
    #[serde(default)]
    pub seq: u64,
}

impl Command {
    fn release(target: &FanTarget) -> Self {
        Self { target: target.clone(), duty: None, hw_curve: None, seq: 0 }
    }

    /// Hands the output back to firmware/driver rather than setting it.
    pub fn is_release(&self) -> bool {
        self.duty.is_none() && self.hw_curve.is_none()
    }
}

#[derive(Debug, Clone)]
struct ChannelState {
    last_duty: f64,
    last_temp: f64,
    /// Send the current target again on the next tick (profile re-applied,
    /// or the last write failed).
    resend: bool,
    /// The hardware curve last programmed, so edits are sent again.
    curve: Option<FanCurve>,
    /// Since when the curve's temperature hasn't been readable.
    blind_since: Option<Instant>,
}

impl ChannelState {
    fn new(duty: f64, temp: f64) -> Self {
        Self { last_duty: duty, last_temp: temp, resend: false, curve: None, blind_since: None }
    }
}

/// Stateful evaluator.
#[derive(Debug, Default)]
pub struct FanEngine {
    /// Outputs the engine is driving.
    state: BTreeMap<FanTarget, ChannelState>,
    /// Releases not yet confirmed, with the time of the next attempt.
    pending_release: BTreeMap<FanTarget, Instant>,
    /// Outputs whose last write failed, left alone until the instant.
    backoff: BTreeMap<FanTarget, Instant>,
    /// Lowest duty an output may ever be given (pumps).
    floors: BTreeMap<FanTarget, f64>,
    /// Assumed time between evaluations until two have happened.
    tick: Duration,
    /// When the last evaluation ran: ramps use the real elapsed time, so they
    /// hold at any telemetry rate.
    last_eval: Option<Instant>,
    /// Issue counter for [`Command::seq`], and the newest number per target.
    next_seq: u64,
    latest: BTreeMap<FanTarget, u64>,
}

impl FanEngine {
    pub fn new(tick: Duration) -> Self {
        Self { tick, ..Default::default() }
    }

    pub fn set_floors(&mut self, floors: BTreeMap<FanTarget, f64>) {
        self.floors = floors;
    }

    /// Minimum duty for an output; enforced on every command it receives.
    pub fn floor(&self, target: &FanTarget) -> f64 {
        self.floors.get(target).copied().unwrap_or(0.0).clamp(0.0, 100.0)
    }

    /// Evaluate one tick. Returns only channels whose duty should change
    /// (or that need a mode change), to keep write traffic low.
    pub fn evaluate(&mut self, cooling: &CoolingSettings, temps: &Temps, now: Instant) -> Vec<Command> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let dt = self.last_eval.map(|t| now.saturating_duration_since(t)).unwrap_or(self.tick);
        self.last_eval = Some(now);
        self.backoff.retain(|_, until| *until > now);
        for fa in &cooling.fans {
            seen.push(fa.target.clone());
            if self.backoff.contains_key(&fa.target) {
                continue;
            }
            let floor = self.floor(&fa.target);
            match &fa.mode {
                FanMode::Auto => {
                    if self.state.remove(&fa.target).is_some() {
                        self.pending_release.insert(fa.target.clone(), now + RETRY);
                        out.push(Command::release(&fa.target));
                    }
                }
                FanMode::Fixed(d) => {
                    let d = d.clamp(0.0, 100.0).max(floor);
                    let changed = self.state.get(&fa.target).is_none_or(|s| s.resend || (s.last_duty - d).abs() > 0.01);
                    if changed {
                        self.state.insert(fa.target.clone(), ChannelState::new(d, 0.0));
                        out.push(Command { target: fa.target.clone(), duty: Some(d), hw_curve: None, seq: 0 });
                    }
                }
                FanMode::HardwareCurve(curve) => {
                    // Programmed once, and again when re-applied or edited.
                    if self.state.get(&fa.target).is_none_or(|s| s.resend || s.curve.as_ref() != Some(curve)) {
                        self.state.insert(fa.target.clone(), ChannelState { curve: Some(curve.clone()), ..ChannelState::new(-1.0, 0.0) });
                        out.push(Command { target: fa.target.clone(), duty: None, hw_curve: Some(curve.clone()), seq: 0 });
                    }
                }
                FanMode::Curve(curve) => {
                    let Some(t) = temps.resolve(&curve.source) else {
                        // No reading: hold the duty a while, then run at the curve's
                        // top, so a fan never idles through heat it can't see.
                        if let Some(st) = self.state.get_mut(&fa.target) {
                            let blind = now.saturating_duration_since(*st.blind_since.get_or_insert(now));
                            let top = curve.points.iter().map(|p| p.1).fold(curve.min_duty, f64::max).clamp(floor, 100.0);
                            // Sent again after a failed write too (`resend`), or a fan that
                            // couldn't be set would stay where it was for the whole blind spell.
                            if blind >= BLIND_LIMIT && (st.resend || (st.last_duty - top).abs() >= 0.5) {
                                st.last_duty = top;
                                st.resend = false;
                                out.push(Command { target: fa.target.clone(), duty: Some(top), hw_curve: None, seq: 0 });
                            }
                        }
                        continue;
                    };
                    // Back from a hold that went to the top: the reading before the hold
                    // means nothing now, so hysteresis mustn't compare against it (that
                    // would pin the top); the ramp still brings the duty down.
                    if let Some(st) = self.state.get_mut(&fa.target)
                        && st.blind_since.take().is_some_and(|since| now.saturating_duration_since(since) >= BLIND_LIMIT)
                    {
                        st.last_temp = f64::INFINITY;
                    }
                    let want = curve.duty_at(t).max(floor);
                    let st = self.state.entry(fa.target.clone()).or_insert(ChannelState::new(-1.0, t));
                    if st.resend || st.last_duty < 0.0 {
                        *st = ChannelState::new(want, t);
                        out.push(Command { target: fa.target.clone(), duty: Some(want), hw_curve: None, seq: 0 });
                        continue;
                    }
                    // Hysteresis: ignore small temperature wiggles when cooling down.
                    let cooling_down = want < st.last_duty;
                    if cooling_down && (st.last_temp - t).abs() < curve.hysteresis_c {
                        continue;
                    }
                    // Ramp limiting: max change per tick given ramp_s for a full sweep.
                    let max_step = if curve.ramp_s > 0.0 { 100.0 * dt.as_secs_f64() / curve.ramp_s } else { 100.0 };
                    let delta = (want - st.last_duty).clamp(-max_step, max_step);
                    let next = (st.last_duty + delta).clamp(floor, 100.0);
                    if (next - st.last_duty).abs() >= 0.5 {
                        st.last_duty = next;
                        st.last_temp = t;
                        out.push(Command { target: fa.target.clone(), duty: Some(next), hw_curve: None, seq: 0 });
                    }
                }
            }
        }
        // Targets that disappeared from the profile: release them.
        let stale: Vec<FanTarget> = self.state.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for t in stale {
            self.state.remove(&t);
            self.pending_release.insert(t.clone(), now + RETRY);
            out.push(Command::release(&t));
        }
        // An output driven again no longer needs releasing; the rest are
        // retried until a write confirms them.
        let state = &self.state;
        self.pending_release.retain(|t, _| !state.contains_key(t));
        for (t, at) in self.pending_release.iter_mut() {
            if *at <= now {
                *at = now + RETRY;
                out.push(Command::release(t));
            }
        }
        self.number(out)
    }

    /// Stamp every command with its issue number.
    fn number(&mut self, cmds: Vec<Command>) -> Vec<Command> {
        cmds.into_iter()
            .map(|mut c| {
                self.next_seq += 1;
                c.seq = self.next_seq;
                self.latest.insert(c.target.clone(), c.seq);
                c
            })
            .collect()
    }

    /// Whether `cmd` is the newest the engine issued for its target.
    pub fn is_current(&self, cmd: &Command) -> bool {
        self.latest.get(&cmd.target).is_none_or(|&n| cmd.seq >= n)
    }

    /// Feed back the outcome of writing `cmd`. The result of a command a
    /// newer one has replaced says nothing about the output now.
    pub fn report(&mut self, cmd: &Command, ok: bool, now: Instant) {
        if !self.is_current(cmd) {
            return;
        }
        if cmd.is_release() {
            // A failed release stays pending and is retried.
            if ok {
                self.pending_release.remove(&cmd.target);
            }
            return;
        }
        if ok {
            return;
        }
        // Don't trust what was sent: drive the output from scratch after a pause.
        if let Some(s) = self.state.get_mut(&cmd.target) {
            s.resend = true;
        }
        self.backoff.insert(cmd.target.clone(), now + RETRY);
    }

    /// Hand every output back to firmware (owner change, shutdown).
    pub fn release_all(&mut self, now: Instant) -> Vec<Command> {
        let targets: BTreeSet<FanTarget> = self.state.keys().chain(self.pending_release.keys()).cloned().collect();
        self.state.clear();
        self.backoff.clear();
        let cmds = targets
            .into_iter()
            .map(|t| {
                self.pending_release.insert(t.clone(), now + RETRY);
                Command::release(&t)
            })
            .collect();
        self.number(cmds)
    }

    /// Send every driven output again on the next tick (profile re-applied).
    pub fn invalidate(&mut self) {
        for s in self.state.values_mut() {
            s.resend = true;
        }
        self.backoff.clear();
    }

    /// Send every driven output again, keeping any retry pause: the helper
    /// went away (a failed write may have taken it down).
    pub fn resend_all(&mut self) {
        for s in self.state.values_mut() {
            s.resend = true;
        }
    }

    /// Nothing driven and nothing waiting to be handed back.
    pub fn is_idle(&self) -> bool {
        self.state.is_empty() && self.pending_release.is_empty()
    }

    pub fn current(&self, target: &FanTarget) -> Option<f64> {
        self.state.get(target).map(|s| s.last_duty).filter(|d| *d >= 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_commands_result_is_ignored_and_numbers_climb() {
        let t = FanTarget::new("hwmon:nct6775:pwm2");
        let mut e = FanEngine::new(Duration::from_millis(500));
        let now = Instant::now();
        let first = e.evaluate(&fixed(t.clone(), 40.0), &Temps::default(), now);
        let second = e.evaluate(&fixed(t.clone(), 60.0), &Temps::default(), now + Duration::from_secs(1));
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert!(second[0].seq > first[0].seq);
        assert!(!e.is_current(&first[0]), "replaced");
        assert!(e.is_current(&second[0]));
        // A late failure of the replaced write must not put the output into backoff.
        e.report(&first[0], false, now + Duration::from_secs(2));
        assert!(e.backoff.is_empty());
        e.report(&second[0], false, now + Duration::from_secs(2));
        assert!(e.backoff.contains_key(&t), "the current write's failure counts");
    }

    fn fixed(target: FanTarget, d: f64) -> CoolingSettings {
        let mut c = CoolingSettings::default();
        c.set(target, FanMode::Fixed(d));
        c
    }

    #[test]
    fn curve_interpolates() {
        let c = FanCurve { points: vec![(30.0, 20.0), (70.0, 100.0)], ..FanCurve::balanced() };
        assert_eq!(c.duty_at(20.0), 25.0); // min_duty
        assert!((c.duty_at(50.0) - 60.0).abs() < 1e-6);
        assert_eq!(c.duty_at(90.0), 100.0);
    }

    #[test]
    fn a_curve_that_cannot_see_holds_then_runs_at_its_top() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let mut cooling = CoolingSettings::default();
        cooling.set(FanTarget::new("superio:pwm2"), FanMode::Curve(FanCurve { points: vec![(30.0, 20.0), (80.0, 90.0)], ramp_s: 0.0, hysteresis_c: 0.0, min_duty: 0.0, source: TempSource::Gpu }));
        let now = Instant::now();
        let c = e.evaluate(&cooling, &Temps { gpu: Some(30.0), ..Default::default() }, now);
        assert_eq!(c[0].duty, Some(20.0));
        e.report(&c[0], true, now);
        let blind = Temps { gpu_unread: true, ..Default::default() };
        assert!(e.evaluate(&cooling, &blind, now + Duration::from_secs(1)).is_empty(), "holds");
        assert!(e.evaluate(&cooling, &blind, now + Duration::from_secs(20)).is_empty(), "still holds");
        let c = e.evaluate(&cooling, &blind, now + Duration::from_secs(32));
        assert_eq!(c.first().and_then(|c| c.duty), Some(90.0), "the curve's top");
        // 30.5 °C on (30, 20)→(80, 90) is 20.7 %.
        let c = e.evaluate(&cooling, &Temps { gpu: Some(30.5), ..Default::default() }, now + Duration::from_secs(33));
        assert!(c.first().and_then(|c| c.duty).is_some_and(|d| (d - 20.7).abs() < 1e-6), "back on the curve at once, not held at the top by hysteresis: {c:?}");
    }

    #[test]
    fn recovery_from_a_blind_hold_ramps_down_despite_hysteresis() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let mut cooling = CoolingSettings::default();
        cooling.set(FanTarget::new("superio:pwm2"), FanMode::Curve(FanCurve { points: vec![(30.0, 20.0), (80.0, 90.0)], ramp_s: 20.0, hysteresis_c: 3.0, min_duty: 0.0, source: TempSource::Gpu }));
        let now = Instant::now();
        let c = e.evaluate(&cooling, &Temps { gpu: Some(50.0), ..Default::default() }, now);
        e.report(&c[0], true, now);
        let blind = Temps { gpu_unread: true, ..Default::default() };
        let _ = e.evaluate(&cooling, &blind, now + Duration::from_secs(1));
        let c = e.evaluate(&cooling, &blind, now + Duration::from_secs(32));
        assert_eq!(c.first().and_then(|c| c.duty), Some(90.0));
        e.report(&c[0], true, now + Duration::from_secs(32));
        // 51 °C is within the 3 °C margin of the 50 °C before the hold: that must
        // not pin the top, and the 20 s ramp still applies (5 % per 1 s tick).
        let c = e.evaluate(&cooling, &Temps { gpu: Some(51.0), ..Default::default() }, now + Duration::from_secs(33));
        assert_eq!(c.first().and_then(|c| c.duty), Some(85.0), "one ramp step down from the top: {c:?}");
    }

    #[test]
    fn a_failed_top_write_while_blind_is_retried() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let mut cooling = CoolingSettings::default();
        cooling.set(FanTarget::new("superio:pwm2"), FanMode::Curve(FanCurve { points: vec![(30.0, 20.0), (80.0, 90.0)], ramp_s: 0.0, hysteresis_c: 0.0, min_duty: 0.0, source: TempSource::Gpu }));
        let now = Instant::now();
        let c = e.evaluate(&cooling, &Temps { gpu: Some(30.0), ..Default::default() }, now);
        e.report(&c[0], true, now);
        let blind = Temps { gpu_unread: true, ..Default::default() };
        let _ = e.evaluate(&cooling, &blind, now + Duration::from_secs(1));
        let c = e.evaluate(&cooling, &blind, now + Duration::from_secs(32));
        assert_eq!(c.first().and_then(|c| c.duty), Some(90.0));
        e.report(&c[0], false, now + Duration::from_secs(32));
        let c = e.evaluate(&cooling, &blind, now + Duration::from_secs(32) + RETRY + Duration::from_secs(1));
        assert_eq!(c.first().and_then(|c| c.duty), Some(90.0), "sent again after the backoff");
    }

    #[test]
    fn a_max_curve_holds_while_its_gpu_is_unread() {
        let unread = Temps { cpu_tctl: Some(50.0), gpu_unread: true, ..Default::default() };
        assert_eq!(unread.resolve(&TempSource::CpuGpuMax), None);
        let no_gpu = Temps { cpu_tctl: Some(50.0), ..Default::default() };
        assert_eq!(no_gpu.resolve(&TempSource::CpuGpuMax), Some(50.0), "a machine with no GPU reading at all");
    }

    #[test]
    fn ramp_limits_change() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let mut cooling = CoolingSettings::default();
        cooling.set(FanTarget::new("ryujin:radiator"), FanMode::Curve(FanCurve { points: vec![(20.0, 0.0), (60.0, 100.0)], ramp_s: 10.0, hysteresis_c: 0.0, min_duty: 0.0, source: TempSource::Coolant }));
        let now = Instant::now();
        let t = Temps { coolant: Some(20.0), ..Default::default() };
        let c = e.evaluate(&cooling, &t, now);
        assert_eq!(c[0].duty, Some(0.0));
        let t = Temps { coolant: Some(60.0), ..Default::default() };
        let c = e.evaluate(&cooling, &t, now + Duration::from_secs(1));
        assert_eq!(c[0].duty, Some(10.0)); // 100/10s per 1s tick
    }

    #[test]
    fn dropped_output_is_released_until_confirmed() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let now = Instant::now();
        let cmds = e.evaluate(&fixed(FanTarget::new("superio:pwm2"), 40.0), &Temps::default(), now);
        e.report(&cmds[0], true, now);

        // The next profile no longer drives the output.
        let cmds = e.evaluate(&CoolingSettings::default(), &Temps::default(), now);
        let releases = |cmds: &[Command]| cmds.iter().map(|c| (c.target.clone(), c.is_release())).collect::<Vec<_>>();
        assert_eq!(releases(&cmds), vec![(FanTarget::new("superio:pwm2"), true)]);
        assert!(!e.is_idle());

        // Unconfirmed: retried after the pause, not before.
        assert!(e.evaluate(&CoolingSettings::default(), &Temps::default(), now + Duration::from_secs(1)).is_empty());
        let retry = e.evaluate(&CoolingSettings::default(), &Temps::default(), now + RETRY);
        assert_eq!(releases(&retry), vec![(FanTarget::new("superio:pwm2"), true)]);
        assert!(retry[0].seq > cmds[0].seq, "the retry is a newer command");
        e.report(&retry[0], true, now + RETRY);
        assert!(e.is_idle());
    }

    #[test]
    fn failed_write_is_resent_after_backoff() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let cooling = fixed(FanTarget::new("superio:pwm1"), 50.0);
        let now = Instant::now();
        let cmds = e.evaluate(&cooling, &Temps::default(), now);
        e.report(&cmds[0], false, now);
        assert!(e.evaluate(&cooling, &Temps::default(), now + Duration::from_secs(1)).is_empty());
        let again = e.evaluate(&cooling, &Temps::default(), now + RETRY);
        assert_eq!(again[0].duty, Some(50.0));
    }

    #[test]
    fn release_all_hands_back_everything() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let mut cooling = fixed(FanTarget::new("superio:pwm1"), 50.0);
        cooling.set(FanTarget::new("nvidia:0:fans"), FanMode::Fixed(70.0));
        let now = Instant::now();
        e.evaluate(&cooling, &Temps::default(), now);
        let released = e.release_all(now);
        assert_eq!(released.len(), 2);
        assert!(released.iter().all(Command::is_release));
        assert!(e.current(&FanTarget::new("superio:pwm1")).is_none());
    }

    #[test]
    fn invalidate_resends_without_releasing() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let cooling = fixed(FanTarget::new("superio:pwm1"), 50.0);
        let now = Instant::now();
        e.evaluate(&cooling, &Temps::default(), now);
        assert!(e.evaluate(&cooling, &Temps::default(), now).is_empty());
        e.invalidate();
        let cmds = e.evaluate(&cooling, &Temps::default(), now);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].duty, Some(50.0));
    }

    #[test]
    fn resend_all_resends_but_keeps_a_retry_pause() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let cooling = fixed(FanTarget::new("superio:pwm1"), 50.0);
        let now = Instant::now();
        let cmds = e.evaluate(&cooling, &Temps::default(), now);
        e.report(&cmds[0], true, now);
        e.resend_all();
        let cmds = e.evaluate(&cooling, &Temps::default(), now);
        assert_eq!(cmds.len(), 1, "sent again");
        e.report(&cmds[0], false, now);
        e.resend_all();
        assert!(e.evaluate(&cooling, &Temps::default(), now + Duration::from_secs(1)).is_empty(), "a failed write still pauses");
        assert_eq!(e.evaluate(&cooling, &Temps::default(), now + RETRY)[0].duty, Some(50.0));
    }

    #[test]
    fn edited_hardware_curves_are_resent() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let target = FanTarget::new("asusd:fan:CPU");
        let mut cooling = CoolingSettings::default();
        cooling.set(target.clone(), FanMode::HardwareCurve(FanCurve::balanced()));
        let now = Instant::now();
        assert_eq!(e.evaluate(&cooling, &Temps::default(), now).len(), 1);
        assert!(e.evaluate(&cooling, &Temps::default(), now).is_empty(), "unchanged curves are not resent");
        cooling.set(target, FanMode::HardwareCurve(FanCurve::performance()));
        let cmds = e.evaluate(&cooling, &Temps::default(), now);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].hw_curve, Some(FanCurve::performance()));
    }

    #[test]
    fn floor_is_enforced() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        e.set_floors(BTreeMap::from([(FanTarget::new("ryujin:pump"), 60.0)]));
        let now = Instant::now();
        let cmds = e.evaluate(&fixed(FanTarget::new("ryujin:pump"), 0.0), &Temps::default(), now);
        assert_eq!(cmds[0].duty, Some(60.0));

        let mut e = FanEngine::new(Duration::from_secs(1));
        e.set_floors(BTreeMap::from([(FanTarget::new("ryujin:pump"), 60.0)]));
        let mut cooling = CoolingSettings::default();
        cooling.set(FanTarget::new("ryujin:pump"), FanMode::Curve(FanCurve { points: vec![(20.0, 0.0), (60.0, 100.0)], ramp_s: 0.0, hysteresis_c: 0.0, min_duty: 0.0, source: TempSource::Coolant }));
        let cmds = e.evaluate(&cooling, &Temps { coolant: Some(20.0), ..Default::default() }, now);
        assert_eq!(cmds[0].duty, Some(60.0));
    }
}

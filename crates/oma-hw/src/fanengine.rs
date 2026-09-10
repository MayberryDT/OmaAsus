//! Software fan engine: evaluates profile fan curves against live
//! temperatures and produces per-target duty commands with hysteresis and
//! ramp limiting. Pure logic (no I/O) so it is unit-testable; the GUI/daemon
//! turns [`Command`]s into writes.

use crate::profile::{CoolingSettings, FanCurve, FanMode, FanTarget, TempSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

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
}

#[derive(Debug, Clone)]
struct ChannelState {
    last_duty: f64,
    last_temp: f64,
    last_change: Instant,
}

/// Stateful evaluator.
#[derive(Debug, Default)]
pub struct FanEngine {
    state: BTreeMap<FanTarget, ChannelState>,
    tick: Duration,
}

impl FanEngine {
    pub fn new(tick: Duration) -> Self {
        Self { state: BTreeMap::new(), tick }
    }

    /// Evaluate one tick. Returns only channels whose duty should change
    /// (or that need a mode change), to keep write traffic low.
    pub fn evaluate(&mut self, cooling: &CoolingSettings, temps: &Temps, now: Instant) -> Vec<Command> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for fa in &cooling.fans {
            seen.push(fa.target.clone());
            match &fa.mode {
                FanMode::Auto => {
                    if self.state.remove(&fa.target).is_some() {
                        out.push(Command { target: fa.target.clone(), duty: None, hw_curve: None });
                    }
                }
                FanMode::Fixed(d) => {
                    let d = d.clamp(0.0, 100.0);
                    let changed = self.state.get(&fa.target).map(|s| (s.last_duty - d).abs() > 0.01).unwrap_or(true);
                    if changed {
                        self.state.insert(fa.target.clone(), ChannelState { last_duty: d, last_temp: 0.0, last_change: now });
                        out.push(Command { target: fa.target.clone(), duty: Some(d), hw_curve: None });
                    }
                }
                FanMode::HardwareCurve(curve) => {
                    if !self.state.contains_key(&fa.target) {
                        self.state.insert(fa.target.clone(), ChannelState { last_duty: -1.0, last_temp: 0.0, last_change: now });
                        out.push(Command { target: fa.target.clone(), duty: None, hw_curve: Some(curve.clone()) });
                    }
                }
                FanMode::Curve(curve) => {
                    let Some(t) = temps.resolve(&curve.source) else { continue };
                    let want = curve.duty_at(t);
                    let st = self.state.entry(fa.target.clone()).or_insert(ChannelState { last_duty: -1.0, last_temp: t, last_change: now - Duration::from_secs(3600) });
                    if st.last_duty < 0.0 {
                        st.last_duty = want;
                        st.last_temp = t;
                        st.last_change = now;
                        out.push(Command { target: fa.target.clone(), duty: Some(want), hw_curve: None });
                        continue;
                    }
                    // Hysteresis: ignore small temperature wiggles when cooling down.
                    let cooling_down = want < st.last_duty;
                    if cooling_down && (st.last_temp - t).abs() < curve.hysteresis_c {
                        continue;
                    }
                    // Ramp limiting: max change per tick given ramp_s for a full sweep.
                    let max_step = if curve.ramp_s > 0.0 { 100.0 * self.tick.as_secs_f64() / curve.ramp_s } else { 100.0 };
                    let delta = (want - st.last_duty).clamp(-max_step, max_step);
                    let next = (st.last_duty + delta).clamp(0.0, 100.0);
                    if (next - st.last_duty).abs() >= 0.5 {
                        st.last_duty = next;
                        st.last_temp = t;
                        st.last_change = now;
                        out.push(Command { target: fa.target.clone(), duty: Some(next), hw_curve: None });
                    }
                }
            }
        }
        // Targets that disappeared from the profile: release them.
        let stale: Vec<FanTarget> = self.state.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for t in stale {
            self.state.remove(&t);
            out.push(Command { target: t, duty: None, hw_curve: None });
        }
        out
    }

    pub fn current(&self, target: &FanTarget) -> Option<f64> {
        self.state.get(target).map(|s| s.last_duty).filter(|d| *d >= 0.0)
    }

    pub fn reset(&mut self) {
        self.state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_interpolates() {
        let c = FanCurve { points: vec![(30.0, 20.0), (70.0, 100.0)], ..FanCurve::balanced() };
        assert_eq!(c.duty_at(20.0), 25.0); // min_duty
        assert!((c.duty_at(50.0) - 60.0).abs() < 1e-6);
        assert_eq!(c.duty_at(90.0), 100.0);
    }

    #[test]
    fn ramp_limits_change() {
        let mut e = FanEngine::new(Duration::from_secs(1));
        let mut cooling = CoolingSettings::default();
        cooling.set(FanTarget::RyujinExternalFans, FanMode::Curve(FanCurve { points: vec![(20.0, 0.0), (60.0, 100.0)], ramp_s: 10.0, hysteresis_c: 0.0, min_duty: 0.0, source: TempSource::Coolant }));
        let now = Instant::now();
        let t = Temps { coolant: Some(20.0), ..Default::default() };
        let c = e.evaluate(&cooling, &t, now);
        assert_eq!(c[0].duty, Some(0.0));
        let t = Temps { coolant: Some(60.0), ..Default::default() };
        let c = e.evaluate(&cooling, &t, now + Duration::from_secs(1));
        assert_eq!(c[0].duty, Some(10.0)); // 100/10s per 1s tick
    }
}

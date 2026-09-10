//! Fan backend: turns [`oma_hw::fanengine::Command`]s into writes for the
//! output each id names in the hardware model (hwmon PWM, Lian Li hub, NVIDIA).

use oma_hw::coolercontrol::CoolerControl;
use oma_hw::fanengine::Command;
use oma_hw::helper::Controller;
use oma_hw::hwmon::{PwmChannel, PwmEnable};
use oma_hw::lianli::LianLiHub;
use oma_hw::model::{FanBackend as Via, FanCaps, FanOutput, HardwareModel, Release};
use oma_hw::profile::{FanCurve, FanTarget};
use oma_hw::SystemInventory;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct FanBackend {
    ctl: Controller,
    model: Arc<HardwareModel>,
    /// Outputs whose last write stalled; skipped until the instant.
    quarantine: Arc<Mutex<HashMap<FanTarget, std::time::Instant>>>,
    /// hwmon PWM channels, by output id.
    channels: HashMap<FanTarget, PwmChannel>,
    /// `pwmN_enable` modes saved before taking an output over, by output id.
    saved_enable: Arc<Mutex<HashMap<FanTarget, PwmEnable>>>,
    lianli: Vec<LianLiHub>,
    pub cc: Option<CoolerControl>,
}

impl std::fmt::Debug for FanBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FanBackend").field("outputs", &self.model.fans.len()).field("pwm", &self.channels.len()).field("lianli", &self.lianli.len()).field("cc", &self.cc.is_some()).finish()
    }
}

/// A fan output that can be assigned in the Cooling page.
#[derive(Debug, Clone, PartialEq)]
pub struct Available {
    pub target: FanTarget,
    pub label: String,
    pub detail: String,
    pub caps: FanCaps,
}

impl FanBackend {
    pub async fn build(model: Arc<HardwareModel>, inv: Arc<SystemInventory>, cc: Option<CoolerControl>) -> Self {
        let ctl = Controller::connect().await;
        let mut channels = HashMap::new();
        for f in &model.fans {
            if let Via::Hwmon { dir, index } = &f.backend {
                if let Some(ch) = inv.hwmon.iter().find(|d| &d.path == dir).and_then(|d| d.pwms.iter().find(|p| p.index == *index)) {
                    channels.insert(f.id.clone(), ch.clone());
                }
            }
        }
        let lianli = if model.fans.iter().any(|f| matches!(f.backend, Via::LianLi { .. })) { tokio::task::spawn_blocking(LianLiHub::enumerate).await.unwrap_or_default() } else { Vec::new() };
        Self { ctl, model, quarantine: Arc::new(Mutex::new(HashMap::new())), channels, saved_enable: Arc::new(Mutex::new(HashMap::new())), lianli, cc }
    }

    /// Every fan output the machine has, with live speed when a tach reads it.
    pub fn available(model: &HardwareModel, snap: Option<&crate::telemetry::Snapshot>) -> Vec<Available> {
        model
            .fans
            .iter()
            .map(|f| {
                let rpm = f.tach.as_ref().and_then(|t| model.sensor(t.as_str())).and_then(|s| snap?.fans.iter().find(|r| r.label == s.label).map(|r| r.rpm));
                let detail = match (rpm, f.caps.duty) {
                    (Some(0), _) => "stopped".into(),
                    (Some(r), _) => format!("{r} rpm"),
                    (None, true) => "no tach signal".into(),
                    (None, false) => "firmware curve".into(),
                };
                Available { target: f.id.clone(), label: f.label.clone(), detail, caps: f.caps.clone() }
            })
            .collect()
    }

    /// Whether this machine has the output, so absent ones are never driven.
    pub fn has(&self, target: &FanTarget) -> bool {
        self.model.fan(target.as_str()).is_some()
    }

    /// Per-output minimum duty the fan engine must respect.
    pub fn floors(&self) -> BTreeMap<FanTarget, f64> {
        self.model.fans.iter().filter(|f| f.caps.min_duty > 0.0).map(|f| (f.id.clone(), f.caps.min_duty)).collect()
    }

    pub async fn apply(&self, cmd: Command) -> Result<(), String> {
        let now = std::time::Instant::now();
        if cmd.is_release() && !self.has(&cmd.target) {
            // Nothing is attached, so there is nothing to hand back.
            return Ok(());
        }
        if self.quarantine.lock().unwrap().get(&cmd.target).is_some_and(|until| *until > now) {
            return Err(format!("{}: device stalled recently; retrying shortly", cmd.target));
        }
        let target = cmd.target.clone();
        let started = std::time::Instant::now();
        let r = self.apply_inner(cmd).await;
        if started.elapsed() > std::time::Duration::from_millis(600) {
            tracing::warn!(%target, ms = started.elapsed().as_millis(), "fan write stalled; quarantining the output for 60s");
            self.quarantine.lock().unwrap().insert(target, started + std::time::Duration::from_secs(60));
        }
        r
    }

    async fn apply_inner(&self, cmd: Command) -> Result<(), String> {
        let out = self.model.fan(cmd.target.as_str()).ok_or("fan output not present")?;
        match &out.backend {
            Via::Hwmon { .. } => self.apply_pwm(out, &cmd).await,
            Via::LianLi { path, channel } => {
                let hub = self.lianli.iter().find(|h| &h.path == path).ok_or("Lian Li hub not present")?;
                if cmd.duty.is_some() {
                    // Manual mode first, then the speed.
                    let _ = self.hid(hub, &hub.report_pwm_sync(*channel, false)).await;
                }
                let report = match cmd.duty {
                    Some(d) => hub.report_set_speed(*channel, d.round() as u8),
                    None => hub.report_pwm_sync(*channel, true),
                };
                self.hid(hub, &report).await
            }
            Via::Nvidia { gpu, .. } => {
                let ctl = oma_hw::nvidia::NvidiaControl::fans_only(cmd.duty.map(|d| d.round() as u32));
                let errs = self.ctl.nvidia_apply(*gpu, &ctl).await.map_err(|e| e.to_string())?;
                errs.first().map(|(s, e)| Err(format!("{s}: {e}"))).unwrap_or(Ok(()))
            }
            // Firmware curves are edited rather than driven; writing them comes
            // with the laptop fan backend.
            Via::AsusdCurve { .. } | Via::AsusCurve { .. } => {
                if cmd.is_release() {
                    Ok(())
                } else {
                    Err(format!("{}: firmware curve editing is not available yet", out.label))
                }
            }
        }
    }

    async fn apply_pwm(&self, out: &FanOutput, cmd: &Command) -> Result<(), String> {
        let ch = self.channels.get(&out.id).ok_or("PWM channel not present")?;
        let duty_value = |d: f64| ((d.clamp(0.0, 100.0) / 100.0) * 255.0).round().to_string();
        match (cmd.duty, &cmd.hw_curve) {
            (Some(d), _) => {
                let mut writes = Vec::new();
                if ch.has_enable {
                    let cur = ch.read().enable.unwrap_or(PwmEnable::SmartFan4);
                    self.saved_enable.lock().unwrap().entry(out.id.clone()).or_insert(cur);
                    if cur != PwmEnable::Manual {
                        writes.push((ch.enable_path(), "1".to_string()));
                    }
                }
                writes.push((ch.path.clone(), duty_value(d)));
                let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
                errs.first().map(|(_, e)| Err(e.clone())).unwrap_or(Ok(()))
            }
            (None, Some(curve)) => self.program_smart_fan(ch, curve).await,
            (None, None) => match out.caps.release {
                // No automatic mode to return to once driven.
                Release::SafeFixed(d) => self.ctl.write(&ch.path, duty_value(d)).await.map_err(|e| e.to_string()),
                Release::RestoreMode | Release::Auto => {
                    let saved = self.saved_enable.lock().unwrap().remove(&out.id);
                    match (ch.has_enable, saved) {
                        (true, Some(mode)) => self.ctl.write(ch.enable_path(), mode.as_u8().to_string()).await.map_err(|e| e.to_string()),
                        // Never taken over: nothing to hand back.
                        _ => Ok(()),
                    }
                }
            },
        }
    }

    async fn hid(&self, hub: &LianLiHub, report: &[u8]) -> Result<(), String> {
        match hub.write(report) {
            Ok(()) => Ok(()),
            Err(_) => {
                let mut buf = report.to_vec();
                buf.resize(65, 0);
                self.ctl.hid_write(&hub.path, &buf).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// Program an nct6775 Smart Fan IV curve (5 points: 4 curve + critical).
    async fn program_smart_fan(&self, ch: &PwmChannel, curve: &FanCurve) -> Result<(), String> {
        let ac = ch.auto_curve.as_ref().ok_or("channel has no hardware curve")?;
        let mut pts: Vec<(f64, f64)> = curve.points.clone();
        pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        // Resample to points-1 curve points + a critical point at 100 %.
        let n = (ac.points.saturating_sub(1)).max(1) as usize;
        let mut writes = Vec::new();
        let base = ch.path.to_string_lossy().into_owned();
        for i in 0..n {
            let f = i as f64 / (n - 1).max(1) as f64;
            let t = pts.first().map(|p| p.0).unwrap_or(30.0) + f * (pts.last().map(|p| p.0).unwrap_or(80.0) - pts.first().map(|p| p.0).unwrap_or(30.0));
            let d = curve.duty_at(t);
            writes.push((PathBuf::from(format!("{base}_auto_point{}_temp", i + 1)), ((t * 1000.0) as i64).to_string()));
            writes.push((PathBuf::from(format!("{base}_auto_point{}_pwm", i + 1)), ((d / 100.0) * 255.0).round().to_string()));
        }
        let crit = (pts.last().map(|p| p.0).unwrap_or(80.0) + 15.0).min(120.0);
        writes.push((PathBuf::from(format!("{base}_auto_point{}_temp", n + 1)), ((crit * 1000.0) as i64).to_string()));
        writes.push((PathBuf::from(format!("{base}_auto_point{}_pwm", n + 1)), "255".into()));
        if ch.has_enable {
            writes.push((ch.enable_path(), "5".into()));
        }
        let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
        errs.first().map(|(p, e)| Err(format!("{p}: {e}"))).unwrap_or(Ok(()))
    }
}

/// Build the engine's temperature view purely from the sampled snapshot
/// (never touches sysfs on the UI thread).
pub fn temps_from(snap: &crate::telemetry::Snapshot) -> oma_hw::fanengine::Temps {
    oma_hw::fanengine::Temps {
        cpu_tctl: snap.cpu.tctl_c,
        cpu_package: snap.cpu.tctl_c,
        gpu: snap.gpu().and_then(|g| g.temp_c),
        coolant: snap.coolant_c,
        vrm: snap.vrm_c,
        board: snap.board_c,
        hwmon: snap.hwmon_temps.iter().map(|((d, l), v)| ((d.clone(), l.clone()), *v)).collect(),
    }
}

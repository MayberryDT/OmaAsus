//! Fan backend: turns [`oma_hw::fanengine::Command`]s into hardware writes
//! (Super I/O PWM, Ryujin hwmon, Lian Li hub, NVIDIA fans, CoolerControl).

use oma_hw::coolercontrol::CoolerControl;
use oma_hw::fanengine::Command;
use oma_hw::helper::Controller;
use oma_hw::hwmon::{HwmonDevice, PwmChannel, PwmEnable};
use oma_hw::lianli::LianLiHub;
use oma_hw::profile::{FanCurve, FanTarget};
use oma_hw::SystemInventory;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct FanBackend {
    ctl: Controller,
    /// Targets whose last write stalled; skipped until the instant.
    quarantine: Arc<Mutex<HashMap<FanTarget, std::time::Instant>>>,
    ryujin: Option<[PathBuf; 3]>,
    superio: HashMap<u32, PwmChannel>,
    saved_enable: Arc<Mutex<HashMap<u32, PwmEnable>>>,
    lianli: Vec<LianLiHub>,
    pub cc: Option<CoolerControl>,
    has_nvidia: bool,
}

impl std::fmt::Debug for FanBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FanBackend").field("ryujin", &self.ryujin.is_some()).field("superio", &self.superio.len()).field("lianli", &self.lianli.len()).field("cc", &self.cc.is_some()).finish()
    }
}

/// A fan output that can be assigned in the Cooling page.
#[derive(Debug, Clone, PartialEq)]
pub struct Available {
    pub target: FanTarget,
    pub label: String,
    pub detail: String,
}

impl FanBackend {
    pub async fn build(inv: Arc<SystemInventory>, cc: Option<CoolerControl>) -> Self {
        let ctl = Controller::connect().await;
        let mut ryujin = None;
        let mut superio = HashMap::new();
        for d in &inv.hwmon {
            if d.name == "rog_ryujin" {
                let get = |i: u32| d.pwms.iter().find(|p| p.index == i).map(|p| p.path.clone());
                if let (Some(a), Some(b), Some(c)) = (get(1), get(2), get(3)) {
                    ryujin = Some([a, b, c]);
                }
            } else if d.is_super_io() {
                for p in &d.pwms {
                    superio.insert(p.index, p.clone());
                }
            }
        }
        Self { ctl, quarantine: Arc::new(Mutex::new(HashMap::new())), ryujin, superio, saved_enable: Arc::new(Mutex::new(HashMap::new())), lianli: LianLiHub::enumerate(), cc, has_nvidia: inv.nvidia_count > 0 }
    }

    pub fn available(inv: &SystemInventory, snap: Option<&crate::telemetry::Snapshot>) -> Vec<Available> {
        let mut v = Vec::new();
        let empty: Vec<crate::telemetry::FanReading> = Vec::new();
        let snapshot_fans: &[crate::telemetry::FanReading] = snap.map(|s| s.fans.as_slice()).unwrap_or(&empty);
        let rpm_of = |label: &str| snapshot_fans.iter().find(|f| f.label == label).map(|f| f.rpm).unwrap_or(0);
        if inv.hwmon.iter().any(|d| d.name == "rog_ryujin") {
            v.push(Available { target: FanTarget::RyujinPump, label: "AIO pump".into(), detail: format!("ROG Ryujin · {} rpm", rpm_of("Pump speed")) });
            v.push(Available { target: FanTarget::RyujinExternalFans, label: "Radiator fans".into(), detail: "ROG Ryujin controller · 4 ports".into() });
            v.push(Available { target: FanTarget::RyujinInternalFan, label: "Pump-block fan".into(), detail: "ROG Ryujin VRM blower".into() });
        }
        for d in inv.hwmon.iter().filter(|d| d.is_super_io()) {
            for p in &d.pwms {
                let rpm = snap.and_then(|s| s.superio_rpm.get(&p.index).copied()).unwrap_or(0);
                v.push(Available { target: FanTarget::SuperIo(p.index), label: format!("Header {}", header_name(p.index)), detail: if rpm > 0 { format!("{rpm} rpm") } else { "no tach signal".into() } });
            }
        }
        if inv.features.lianli_uni_hub {
            for c in 1..=4u8 {
                let rpm = rpm_of(&format!("Lian Li channel {c}"));
                v.push(Available { target: FanTarget::LianLiChannel(c), label: format!("Lian Li channel {c}"), detail: if rpm > 0 { format!("UNI FAN hub · {rpm} rpm") } else { "UNI FAN hub · no fan".into() } });
            }
        }
        if inv.nvidia_count > 0 {
            v.push(Available { target: FanTarget::NvidiaFans, label: "GPU fans".into(), detail: "NVIDIA via NVML".into() });
        }
        v
    }

    pub async fn apply(&self, cmd: Command) -> Result<(), String> {
        let now = std::time::Instant::now();
        if self.quarantine.lock().unwrap().get(&cmd.target).is_some_and(|until| *until > now) {
            return Ok(());
        }
        let target = cmd.target.clone();
        let started = std::time::Instant::now();
        let r = self.apply_inner(cmd).await;
        if started.elapsed() > std::time::Duration::from_millis(600) {
            tracing::warn!(?target, ms = started.elapsed().as_millis(), "fan write stalled; quarantining target for 60s");
            self.quarantine.lock().unwrap().insert(target, started + std::time::Duration::from_secs(60));
        }
        r
    }

    async fn apply_inner(&self, cmd: Command) -> Result<(), String> {
        match &cmd.target {
            FanTarget::RyujinPump | FanTarget::RyujinInternalFan | FanTarget::RyujinExternalFans => {
                let paths = self.ryujin.as_ref().ok_or("Ryujin hwmon not present")?;
                let idx = match cmd.target {
                    FanTarget::RyujinPump => 0,
                    FanTarget::RyujinInternalFan => 1,
                    _ => 2,
                };
                // Releasing: fall back to a safe fixed duty (the AIO has no autonomous curve once driven).
                let duty = cmd.duty.unwrap_or(if idx == 0 { 65.0 } else { 40.0 });
                self.ctl.write(&paths[idx], ((duty / 100.0) * 255.0).round().to_string()).await.map_err(|e| e.to_string())
            }
            FanTarget::SuperIo(n) => {
                let ch = self.superio.get(n).ok_or("PWM channel not present")?;
                match (cmd.duty, cmd.hw_curve) {
                    (Some(d), _) => {
                        let mut writes = Vec::new();
                        if ch.has_enable {
                            let cur = ch.read().enable.unwrap_or(PwmEnable::SmartFan4);
                            let mut saved = self.saved_enable.lock().unwrap();
                            if !saved.contains_key(n) {
                                saved.insert(*n, cur);
                            }
                            if cur != PwmEnable::Manual {
                                writes.push((ch.enable_path(), "1".to_string()));
                            }
                        }
                        writes.push((ch.path.clone(), ((d / 100.0) * 255.0).round().to_string()));
                        let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
                        errs.first().map(|(_, e)| Err(e.clone())).unwrap_or(Ok(()))
                    }
                    (None, Some(curve)) => self.program_smart_fan(ch, &curve).await,
                    (None, None) => {
                        let saved = self.saved_enable.lock().unwrap().remove(n).unwrap_or(PwmEnable::SmartFan4);
                        if ch.has_enable {
                            self.ctl.write(ch.enable_path(), saved.as_u8().to_string()).await.map_err(|e| e.to_string())
                        } else {
                            Ok(())
                        }
                    }
                }
            }
            FanTarget::LianLiChannel(c) => {
                let hub = self.lianli.first().ok_or("Lian Li hub not present")?;
                let report = match cmd.duty {
                    Some(d) => hub.report_set_speed(*c, d.round() as u8),
                    None => hub.report_pwm_sync(*c, true),
                };
                if let Some(d) = cmd.duty {
                    // Make sure manual mode is on before setting speed.
                    let _ = self.hid(hub, &hub.report_pwm_sync(*c, false)).await;
                    let _ = d;
                }
                self.hid(hub, &report).await
            }
            FanTarget::NvidiaFans => {
                if !self.has_nvidia {
                    return Err("no NVIDIA GPU".into());
                }
                let ctl = oma_hw::nvidia::NvidiaControl::fans_only(cmd.duty.map(|d| d.round() as u32));
                let errs = self.ctl.nvidia_apply(0, &ctl).await.map_err(|e| e.to_string())?;
                errs.first().map(|(s, e)| Err(format!("{s}: {e}"))).unwrap_or(Ok(()))
            }
            FanTarget::CoolerControl { device_uid, channel } => {
                let cc = self.cc.as_ref().ok_or("CoolerControl not connected")?;
                match cmd.duty {
                    Some(d) => cc.set_manual(device_uid, channel, d.round() as u8).await.map_err(|e| e.to_string()),
                    None => cc.reset(device_uid, channel).await.map_err(|e| e.to_string()),
                }
            }
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

pub fn header_name(idx: u32) -> String {
    match idx {
        1 => "CPU_FAN (pwm1)".into(),
        2 => "CPU_OPT (pwm2)".into(),
        3 => "CHA_FAN1 (pwm3)".into(),
        4 => "CHA_FAN2 (pwm4)".into(),
        5 => "CHA_FAN3 (pwm5)".into(),
        6 => "AIO_PUMP (pwm6)".into(),
        7 => "W_PUMP+ (pwm7)".into(),
        n => format!("pwm{n}"),
    }
}

/// Build the engine's temperature view purely from the sampled snapshot
/// (never touches sysfs on the UI thread).
pub fn temps_from(snap: &crate::telemetry::Snapshot, _hwmon: &[HwmonDevice]) -> oma_hw::fanengine::Temps {
    oma_hw::fanengine::Temps {
        cpu_tctl: snap.cpu.tctl_c,
        cpu_package: snap.cpu.tctl_c,
        gpu: snap.nvidia.as_ref().and_then(|n| n.temp_c).map(|c| c as f64),
        coolant: snap.coolant_c,
        vrm: snap.vrm_c,
        board: snap.board_c,
        hwmon: snap.hwmon_temps.iter().map(|((d, l), v)| ((d.clone(), l.clone()), *v)).collect(),
    }
}

//! Fan backend: turns [`oma_hw::fanengine::Command`]s into writes for the
//! output each id names in the hardware model (hwmon PWM, Lian Li hub, NVIDIA).

use oma_hw::coolercontrol::CoolerControl;
use oma_hw::fanengine::Command;
use oma_hw::helper::Controller;
use oma_hw::asusd::{self, CurveData, PlatformProfile};
use oma_hw::hwmon::{PwmChannel, PwmEnable, TempUnit};
use oma_hw::lianli::LianLiHub;
use oma_hw::model::{FanBackend as Via, FanCaps, FanOutput, HardwareModel, Release};
use oma_hw::profile::{match_power_mode, FanCurve, FanTarget};
use oma_hw::SystemInventory;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What `apply` answers for a command a newer one for the same output has
/// overtaken: nothing was written, and the engine ignores the result.
pub const SUPERSEDED: &str = "superseded by a newer command";

/// A reading older than this is no longer a temperature to control fans by;
/// curves then follow the missing-source policy instead of a stale number.
pub const CONTROL_FRESH: std::time::Duration = std::time::Duration::from_secs(4);

/// Saved `auto_point` attribute values, by output.
/// What an output held before OmaAsus took it over, by output id: the
/// `pwmN_enable` mode as read, and the board's own curve points when a
/// hardware curve was programmed over them. Saved once, before the first
/// write, so programming again never saves OmaAsus's own values as the
/// board's; forgotten only when the release that wrote them back succeeded,
/// so a refused release is retried with the right values.
#[derive(Debug, Default)]
struct Saved {
    mode: HashMap<FanTarget, String>,
    /// The duty an output ran at, kept only where it was under manual control:
    /// under an automatic mode the driver sets its own.
    duty: HashMap<FanTarget, String>,
    curve: HashMap<FanTarget, Vec<(PathBuf, String)>>,
}

impl Saved {
    /// Keep the mode as read and, under manual control, the duty as read,
    /// unless kept already. A mode that cannot be read is not taken over:
    /// nothing could put it back.
    fn save_found(&mut self, id: &FanTarget, mode: Option<String>, duty: Option<String>) -> Result<(), String> {
        if self.mode.contains_key(id) {
            return Ok(());
        }
        let mode = mode.ok_or_else(|| format!("{id}: the output's mode cannot be read, so it is not taken over"))?;
        if mode.trim().parse::<u64>().ok().map(PwmEnable::from) == Some(PwmEnable::Manual)
            && let Some(d) = duty
        {
            self.duty.insert(id.clone(), d);
        }
        self.mode.insert(id.clone(), mode);
        Ok(())
    }

    fn has_curve(&self, id: &FanTarget) -> bool {
        self.curve.contains_key(id)
    }

    /// Keep the board's curve before it is overwritten. Every one of its
    /// `points` pairs must have been read, or nothing is kept and the caller
    /// must not program: half a curve would be put back as half a curve.
    fn save_curve(&mut self, id: &FanTarget, pwm: &std::path::Path, points: u32, originals: Vec<(PathBuf, String)>) -> Result<(), String> {
        if self.curve.contains_key(id) {
            return Ok(());
        }
        let want = (points * 2) as usize;
        if originals.len() != want {
            return Err(format!("{}: only {} of {want} values of the board's own curve could be read, so it is not overwritten", pwm.display(), originals.len()));
        }
        self.curve.insert(id.clone(), originals);
        Ok(())
    }

    /// The writes that put an output back: the board's own points, the mode
    /// it was found in, then the duty it ran at if that mode was manual. The
    /// mode goes before the duty because a driver refuses a duty under its
    /// automatic modes (the board may be on OmaAsus's curve at this point)
    /// and switching to manual leaves the automatic logic's last duty, which
    /// the duty written after it corrects. Empty when never taken over.
    fn release_writes(&self, id: &FanTarget, enable: Option<PathBuf>, duty: Option<PathBuf>) -> Vec<(PathBuf, String)> {
        let mut writes = self.curve.get(id).cloned().unwrap_or_default();
        if let (Some(path), Some(mode)) = (enable, self.mode.get(id)) {
            writes.push((path, mode.clone()));
        }
        if let (Some(path), Some(d)) = (duty, self.duty.get(id)) {
            writes.push((path, d.clone()));
        }
        writes
    }

    /// The output is back as found: nothing more to put back.
    fn forget(&mut self, id: &FanTarget) {
        self.mode.remove(id);
        self.duty.remove(id);
        self.curve.remove(id);
    }
}

#[derive(Clone)]
pub struct FanBackend {
    ctl: Controller,
    model: Arc<HardwareModel>,
    /// Outputs whose last write stalled; skipped until the instant.
    quarantine: Arc<Mutex<HashMap<FanTarget, std::time::Instant>>>,
    /// One ordered write stream per output, and the newest command number
    /// asked for it: an older command waiting behind a slow write is dropped.
    lanes: Arc<Mutex<HashMap<FanTarget, Arc<tokio::sync::Mutex<()>>>>>,
    /// One write stream per fan hub: its channels share one HID node, and
    /// the hub wants its reports spaced.
    hubs: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    newest: Arc<Mutex<HashMap<FanTarget, u64>>>,
    /// hwmon PWM channels, by output id.
    channels: HashMap<FanTarget, PwmChannel>,
    /// The hwmon driver behind each channel: its curve mode comes from the knowledge base.
    drivers: HashMap<FanTarget, String>,
    /// What each output held before OmaAsus took it over.
    saved: Arc<Mutex<Saved>>,
    lianli: Vec<LianLiHub>,
    pub cc: Option<CoolerControl>,
    /// The active profile's power mode: firmware curves are stored per mode.
    power_mode: Arc<Mutex<Option<String>>>,
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
        let mut drivers = HashMap::new();
        for f in &model.fans {
            if let Via::Hwmon { dir, index } = &f.backend
                && let Some(dev) = inv.hwmon.iter().find(|d| &d.path == dir)
                && let Some(ch) = dev.pwms.iter().find(|p| p.index == *index)
            {
                channels.insert(f.id.clone(), ch.clone());
                drivers.insert(f.id.clone(), dev.name.clone());
            }
        }
        let lianli = if model.fans.iter().any(|f| matches!(f.backend, Via::LianLi { .. })) { tokio::task::spawn_blocking(LianLiHub::enumerate).await.unwrap_or_default() } else { Vec::new() };
        Self { ctl, model, quarantine: Arc::new(Mutex::new(HashMap::new())), lanes: Arc::new(Mutex::new(HashMap::new())), hubs: Arc::new(Mutex::new(HashMap::new())), newest: Arc::new(Mutex::new(HashMap::new())), channels, drivers, saved: Arc::new(Mutex::new(Saved::default())), lianli, cc, power_mode: Arc::new(Mutex::new(None)) }
    }

    /// Tell the backend which power mode the active profile selects.
    pub fn set_power_mode(&self, mode: Option<String>) {
        *self.power_mode.lock().unwrap() = mode;
    }

    /// The platform profile a firmware curve belongs to: the active profile's
    /// power mode, else whatever asusd is in now.
    async fn curve_profile(&self) -> Result<PlatformProfile, String> {
        let wanted = self.power_mode.lock().unwrap().clone();
        if let Some(p) = wanted.as_deref().and_then(|w| match_power_mode(w, &self.model.controls.power_modes)).and_then(PlatformProfile::from_label) {
            return Ok(p);
        }
        let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
        let current = asusd::PlatformProxy::new(&conn).await.map_err(|e| e.to_string())?.platform_profile().await.map_err(|e| e.to_string())?;
        Ok(PlatformProfile::from_u32(current))
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
        if cmd.is_release() && !self.has(&cmd.target) {
            // Nothing is attached, so there is nothing to hand back.
            return Ok(());
        }
        // Note the newest number before waiting for the lane, so whichever
        // command gets the lane next knows whether it has been overtaken.
        {
            let mut n = self.newest.lock().unwrap();
            let e = n.entry(cmd.target.clone()).or_insert(0);
            *e = (*e).max(cmd.seq);
        }
        let lane = self.lanes.lock().unwrap().entry(cmd.target.clone()).or_default().clone();
        let _ordered = lane.lock().await;
        if self.newest.lock().unwrap().get(&cmd.target).copied().unwrap_or(0) > cmd.seq {
            return Err(format!("{}: {SUPERSEDED}", cmd.target));
        }
        let now = std::time::Instant::now();
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
                let hub = self.lianli.iter().find(|h| &h.path == path).ok_or("fan hub not connected")?;
                let hub_lane = self.hubs.lock().unwrap().entry(hub.path.clone()).or_default().clone();
                let _one_channel_at_a_time = hub_lane.lock().await;
                let r = match cmd.duty {
                    Some(d) => {
                        // Manual control first, then the speed. A channel left in
                        // PWM sync ignores the speed, so that write must succeed.
                        self.hid(hub, &hub.report_pwm_sync(*channel, false)).await.map_err(|e| format!("manual mode: {e}"))?;
                        self.hid(hub, &hub.report_set_speed(*channel, d.round() as u8)).await
                    }
                    None => self.hid(hub, &hub.report_pwm_sync(*channel, true)).await,
                };
                // The pause liquidctl leaves after a report before the hub gets another.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                r
            }
            Via::Nvidia { gpu, .. } => {
                // supergfxd kills whatever holds the dGPU while it switches; the engine retries.
                if !crate::telemetry::dgpu_open_ok() {
                    return Err("the dGPU is settling after a graphics change".into());
                }
                let ctl = oma_hw::nvidia::NvidiaControl::fans_only(cmd.duty.map(|d| d.round() as u32));
                let errs = self.ctl.nvidia_apply(*gpu, &ctl).await.map_err(|e| e.to_string())?;
                errs.first().map(|(s, e)| Err(format!("{s}: {e}"))).unwrap_or(Ok(()))
            }
            // The firmware runs these fans: OmaAsus programs their curve.
            Via::AsusdCurve { fan } => {
                let profile = self.curve_profile().await?;
                let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                match &cmd.hw_curve {
                    Some(curve) => {
                        let data = CurveData::from_points(fan, &curve.resample(firmware_points(out)), true);
                        asusd::set_fan_curve(&conn, profile, data).await.map_err(|e| e.to_string())
                    }
                    None if cmd.is_release() => asusd::disable_fan_curve(&conn, profile, fan).await.map_err(|e| e.to_string()),
                    None => Err(format!("{}: the firmware runs this fan; give it a firmware curve instead", out.label)),
                }
            }
            Via::AsusCurve { dir, index, driver } => {
                let (on, off) = oma_hw::knowledge::curve_enable_values(driver).ok_or("this curve driver's modes are unknown")?;
                let enable = dir.join(format!("pwm{index}_enable"));
                let pwm = dir.join(format!("pwm{index}"));
                match &cmd.hw_curve {
                    Some(curve) => {
                        let n = firmware_points(out);
                        // The firmware's own points are saved before they are overwritten.
                        if !self.saved.lock().unwrap().has_curve(&out.id) {
                            let originals = curve_originals(|p| oma_hw::sysfs::read_string(p), &pwm, n as u32);
                            self.saved.lock().unwrap().save_curve(&out.id, &pwm, n as u32, originals)?;
                        }
                        let data = CurveData::from_points("", &curve.resample(n), true);
                        let scale = match oma_hw::knowledge::curve_temp_unit(driver) {
                            TempUnit::Celsius => 1,
                            TempUnit::Milli => 1000,
                        };
                        let mut writes: Vec<(PathBuf, String)> = Vec::new();
                        for (i, (t, p)) in data.temp.iter().zip(data.pwm).enumerate().take(n) {
                            writes.push((dir.join(format!("pwm{index}_auto_point{}_temp", i + 1)), (u32::from(*t) * scale).to_string()));
                            writes.push((dir.join(format!("pwm{index}_auto_point{}_pwm", i + 1)), p.to_string()));
                        }
                        writes.push((enable, on.to_string()));
                        let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
                        errs.first().map(|(p, e)| Err(format!("{p}: {e}"))).unwrap_or(Ok(()))
                    }
                    None if cmd.is_release() => {
                        // The firmware's points go back, then its own mode.
                        let mut writes = self.saved.lock().unwrap().release_writes(&out.id, None, None);
                        writes.push((enable.clone(), off.to_string()));
                        let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
                        if let Some((p, e)) = errs.first() {
                            return Err(format!("{p}: {e}"));
                        }
                        self.saved.lock().unwrap().forget(&out.id);
                        Ok(())
                    }
                    None => Err(format!("{}: the firmware runs this fan; give it a firmware curve instead", out.label)),
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
                    // The mode is kept as read: a value this build has no name
                    // for still goes back as it was.
                    let found = oma_hw::sysfs::read_string(ch.enable_path());
                    let manual = found.as_deref().and_then(|v| v.parse::<u64>().ok()).map(PwmEnable::from) == Some(PwmEnable::Manual);
                    let duty_now = ch.has_duty.then(|| oma_hw::sysfs::read_string(&ch.path)).flatten();
                    self.saved.lock().unwrap().save_found(&out.id, found, duty_now)?;
                    if !manual {
                        writes.push((ch.enable_path(), "1".to_string()));
                    }
                }
                writes.push((ch.path.clone(), duty_value(d)));
                let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
                errs.first().map(|(_, e)| Err(e.clone())).unwrap_or(Ok(()))
            }
            (None, Some(curve)) => self.program_smart_fan(&out.id, ch, curve).await,
            (None, None) => match out.caps.release {
                // No automatic mode to return to once driven.
                Release::SafeFixed(d) => self.ctl.write(&ch.path, duty_value(d)).await.map_err(|e| e.to_string()),
                Release::RestoreMode | Release::Auto => {
                    // What was found before the take-over goes back: the board's own
                    // curve points first, then the mode. Forgotten only once the writes
                    // confirmed it: a refused release is retried and must still know.
                    let writes = self.saved.lock().unwrap().release_writes(&out.id, ch.has_enable.then(|| ch.enable_path()), ch.has_duty.then(|| ch.path.clone()));
                    if writes.is_empty() {
                        // Never taken over: nothing to hand back.
                        return Ok(());
                    }
                    let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
                    if let Some((p, e)) = errs.first() {
                        return Err(format!("{p}: {e}"));
                    }
                    self.saved.lock().unwrap().forget(&out.id);
                    Ok(())
                }
            },
        }
    }

    /// Write a report to the hub: directly where udev grants the user the
    /// node, else through the helper. Off the runtime thread either way.
    async fn hid(&self, hub: &LianLiHub, report: &[u8]) -> Result<(), String> {
        let (h, r) = (hub.clone(), report.to_vec());
        match tokio::task::spawn_blocking(move || h.write(&r)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(direct)) => {
                tracing::debug!(hub = %hub.path, error = %direct, "direct hub write failed; asking the helper");
                let mut buf = report.to_vec();
                buf.resize(65, 0);
                self.ctl.hid_write(&hub.path, &buf).await.map(|_| ()).map_err(|e| e.to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Program an nct6775 Smart Fan IV curve (5 points: 4 curve + critical).
    /// The board's own points and mode are saved first, once, so a release can
    /// put them back.
    async fn program_smart_fan(&self, id: &FanTarget, ch: &PwmChannel, curve: &FanCurve) -> Result<(), String> {
        let ac = ch.auto_curve.as_ref().ok_or("channel has no hardware curve")?;
        let mode = self.drivers.get(id).and_then(|d| oma_hw::knowledge::smart_fan_mode(d)).ok_or_else(|| format!("{}: the mode that runs this driver's own curve is not known", ch.path.display()))?;
        if !self.saved.lock().unwrap().has_curve(id) {
            let originals = curve_originals(|p| oma_hw::sysfs::read_string(p), &ch.path, ac.points);
            self.saved.lock().unwrap().save_curve(id, &ch.path, ac.points, originals)?;
        }
        if ch.has_enable {
            let duty_now = ch.has_duty.then(|| oma_hw::sysfs::read_string(&ch.path)).flatten();
            self.saved.lock().unwrap().save_found(id, oma_hw::sysfs::read_string(ch.enable_path()), duty_now)?;
        }
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
            writes.push((ch.enable_path(), mode.to_string()));
        }
        let errs = self.ctl.write_batch(&writes).await.map_err(|e| e.to_string())?;
        errs.first().map(|(p, e)| Err(format!("{p}: {e}"))).unwrap_or(Ok(()))
    }
}

/// The current values of an output's `auto_point` attributes (the board's own
/// curve), read through `read` so it can be tested without sysfs.
fn curve_originals(read: impl Fn(&std::path::Path) -> Option<String>, pwm: &std::path::Path, points: u32) -> Vec<(PathBuf, String)> {
    let base = pwm.to_string_lossy().into_owned();
    (1..=points)
        .flat_map(|i| [format!("{base}_auto_point{i}_temp"), format!("{base}_auto_point{i}_pwm")])
        .map(PathBuf::from)
        .filter_map(|p| read(&p).map(|v| (p, v)))
        .collect()
}

/// Points in an output's firmware curve (8 on ASUS laptops).
fn firmware_points(out: &FanOutput) -> usize {
    out.caps.firmware_curve.as_ref().map(|c| c.points as usize).unwrap_or(8)
}

/// The temperatures fan control may act on right now: the last frame's, if
/// it is fresh; otherwise none, so a sampler that has stopped delivering
/// cannot leave curves acting on old numbers. The engine's missing-source
/// policy (hold, then a safe duty) takes over from there.
pub fn control_temps(snap: Option<&crate::telemetry::Snapshot>, taken: Option<std::time::Instant>, now: std::time::Instant) -> oma_hw::fanengine::Temps {
    match (snap, taken) {
        (Some(s), Some(t)) if now.saturating_duration_since(t) <= CONTROL_FRESH => temps_from(s),
        _ => oma_hw::fanengine::Temps::default(),
    }
}

/// Build the engine's temperature view purely from the sampled snapshot
/// (never touches sysfs on the UI thread).
pub fn temps_from(snap: &crate::telemetry::Snapshot) -> oma_hw::fanengine::Temps {
    oma_hw::fanengine::Temps {
        cpu_tctl: snap.cpu.tctl_c,
        cpu_package: snap.cpu.tctl_c,
        gpu: snap.curve_gpu_temp(),
        coolant: snap.coolant_c,
        vrm: snap.vrm_c,
        board: snap.board_c,
        hwmon: snap.hwmon_temps.iter().map(|((d, l), v)| ((d.clone(), l.clone()), *v)).collect(),
        gpu_unread: snap.gpu_unread(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boards_own_curve_is_read_before_it_is_overwritten() {
        let pwm = std::path::Path::new("/sys/class/hwmon/hwmon4/pwm1");
        let read = |p: &std::path::Path| {
            let n = p.file_name()?.to_str()?.to_string();
            match n.as_str() {
                "pwm1_auto_point1_temp" => Some("20000".to_string()),
                "pwm1_auto_point1_pwm" => Some("153".to_string()),
                "pwm1_auto_point2_temp" => Some("45000".to_string()),
                "pwm1_auto_point2_pwm" => Some("178".to_string()),
                _ => None,
            }
        };
        let saved = curve_originals(read, pwm, 5);
        let names: Vec<String> = saved.iter().map(|(p, v)| format!("{}={v}", p.file_name().unwrap().to_string_lossy())).collect();
        assert_eq!(names, ["pwm1_auto_point1_temp=20000", "pwm1_auto_point1_pwm=153", "pwm1_auto_point2_temp=45000", "pwm1_auto_point2_pwm=178"], "only what the board reports, in write order");
    }

    #[test]
    fn what_the_board_held_is_saved_once_and_put_back_in_order() {
        let id = FanTarget::new("superio:pwm1");
        let pwm = PathBuf::from("/sys/class/hwmon/hwmon4/pwm1");
        let en = PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_enable");
        let mut saved = Saved::default();
        assert!(saved.release_writes(&id, Some(en.clone()), Some(pwm.clone())).is_empty(), "never taken over: nothing to put back");
        // Half a curve cannot be saved, so it is not programmed.
        let half = vec![(PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_auto_point1_temp"), "20000".to_string())];
        assert!(saved.save_curve(&id, &pwm, 2, half).is_err());
        assert!(!saved.has_curve(&id));
        // Taking over: the board's points and mode, as found.
        let board = vec![
            (PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_auto_point1_temp"), "20000".to_string()),
            (PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_auto_point1_pwm"), "153".to_string()),
            (PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_auto_point2_temp"), "45000".to_string()),
            (PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_auto_point2_pwm"), "178".to_string()),
        ];
        saved.save_curve(&id, &pwm, 2, board.clone()).unwrap();
        saved.save_found(&id, Some("7".into()), Some("90".into())).unwrap();
        // Programming again (now reading OmaAsus's own values) changes nothing.
        let ours = vec![(PathBuf::from("/sys/class/hwmon/hwmon4/pwm1_auto_point1_temp"), "30000".to_string()); 4];
        saved.save_curve(&id, &pwm, 2, ours).unwrap();
        saved.save_found(&id, Some("5".into()), Some("10".into())).unwrap();
        let mut expect = board.clone();
        expect.push((en.clone(), "7".into()));
        assert_eq!(saved.release_writes(&id, Some(en.clone()), Some(pwm.clone())), expect, "the board's points, then the mode it was found in, even one this build has no name for; no duty under an automatic mode");
        // A refused release keeps them; a successful one forgets them.
        assert_eq!(saved.release_writes(&id, Some(en.clone()), Some(pwm.clone())), expect);
        saved.forget(&id);
        assert!(saved.release_writes(&id, Some(en.clone()), Some(pwm.clone())).is_empty());
        // Found under manual control: manual again first, then the duty it ran at.
        saved.save_found(&id, Some("1".into()), Some("200".into())).unwrap();
        assert_eq!(saved.release_writes(&id, Some(en.clone()), Some(pwm.clone())), vec![(en.clone(), "1".to_string()), (pwm.clone(), "200".to_string())]);
        saved.forget(&id);
        // A mode that cannot be read is not taken over.
        assert!(saved.save_found(&id, None, Some("200".into())).is_err());
        assert!(saved.release_writes(&id, Some(en), Some(pwm)).is_empty());
    }

    #[test]
    fn stale_frames_give_fan_control_no_temperatures() {
        let now = std::time::Instant::now();
        let snap = crate::telemetry::Snapshot { cpu: oma_hw::cpu::CpuTelemetry { tctl_c: Some(70.0), ..Default::default() }, coolant_c: Some(31.0), ..Default::default() };
        let fresh = control_temps(Some(&snap), Some(now), now + std::time::Duration::from_secs(1));
        assert_eq!((fresh.cpu_tctl, fresh.coolant), (Some(70.0), Some(31.0)));
        let stale = control_temps(Some(&snap), Some(now), now + CONTROL_FRESH + std::time::Duration::from_secs(1));
        assert_eq!((stale.cpu_tctl, stale.coolant), (None, None), "an old frame is not a temperature");
        assert!(control_temps(None, None, now).cpu_tctl.is_none(), "no frame yet");
    }
}

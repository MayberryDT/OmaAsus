//! Background telemetry sampler → iced subscription.

use crate::device_worker::{Poll, Worker};
use iced::futures::SinkExt;
use iced::futures::Stream;
use oma_hw::cpu::{CpuMonitor, CpuTelemetry};
use oma_hw::hwmon::{self, HwmonDevice};
use oma_hw::nvidia::{DgpuState, NvidiaGpu, NvidiaTelemetry};
use oma_hw::amdgpu::{AmdGpu, AmdGpuTelemetry};
use oma_hw::knowledge::{self, TachRole};
use oma_hw::lianli::LianLiHub;
use oma_hw::model::{HardwareModel, SensorRole};
use std::sync::Arc;
use std::time::Duration;

/// Freshness of a channel in the persistent registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Freshness {
    /// Updated within the last few seconds.
    #[default]
    Live,
    /// Missed a few polls; last value shown dimmed.
    Stale,
    /// Not answering (quarantined / unplugged); last value kept, flagged.
    Offline,
}

#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub label: String,
    pub value: f64,
    pub freshness: Freshness,
}

#[derive(Debug, Clone, Default)]
pub struct FanReading {
    pub label: String,
    pub rpm: u64,
    pub duty: Option<f64>,
    pub device: String,
    pub freshness: Freshness,
    /// Top speed, when known (knowledge or the user's quirks).
    pub max_rpm: Option<u64>,
    /// Fastest seen since start, to scale a fan whose top speed isn't known.
    pub peak_rpm: u64,
}

/// Registry entry: last value plus when it was last refreshed.
#[derive(Debug, Clone)]
struct Tracked<T> {
    value: T,
    last_seen: std::time::Instant,
    order: usize,
}

const STALE_AFTER: Duration = Duration::from_secs(4);
const OFFLINE_AFTER: Duration = Duration::from_secs(30);

fn freshness(last_seen: std::time::Instant, now: std::time::Instant) -> Freshness {
    let age = now.duration_since(last_seen);
    if age < STALE_AFTER { Freshness::Live } else if age < OFFLINE_AFTER { Freshness::Stale } else { Freshness::Offline }
}

/// One telemetry frame.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub seq: u64,
    pub cpu: CpuTelemetry,
    pub nvidia: Option<NvidiaTelemetry>,
    pub amd: Option<AmdGpuTelemetry>,
    /// `amd` comes from an integrated GPU (APU), whose power is the package's.
    pub amd_integrated: bool,
    /// Board / EC / AIO temperatures (label, °C).
    pub temps: Vec<Reading>,
    pub fans: Vec<FanReading>,
    pub coolant_c: Option<f64>,
    pub pump_rpm: Option<u64>,
    pub vrm_c: Option<f64>,
    pub board_c: Option<f64>,
    pub nvme_c: Vec<f64>,
    pub dimm_c: Vec<f64>,
    pub cpu_control: oma_hw::cpu::CpuControlState,
    /// Every hwmon temperature read this tick: (driver, label) → °C.
    pub hwmon_temps: std::collections::BTreeMap<(String, String), f64>,
    /// Runtime power of the NVIDIA GPU as the bus reports it (`Absent` when
    /// switched off or there is none).
    pub dgpu: Option<DgpuState>,
    /// Every sampled device and whether it is answering.
    pub sources: Vec<SourceHealth>,
}

/// One sampled device's health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHealth {
    pub name: String,
    pub healthy: bool,
    /// How long it has been silent, if it is.
    pub stalled_s: Option<u64>,
}

/// GPU figures for glance views.
#[derive(Debug, Clone, Copy, Default)]
pub struct GpuVitals {
    pub temp_c: Option<f64>,
    pub load: Option<f64>,
    pub power_w: Option<f64>,
    pub discrete: bool,
}

impl Snapshot {
    /// The GPU worth showing: an awake NVIDIA card, else the AMD one.
    pub fn gpu(&self) -> Option<GpuVitals> {
        if let Some(n) = &self.nvidia {
            return Some(GpuVitals { temp_c: n.temp_c.map(f64::from), load: n.util_gpu.map(f64::from), power_w: n.power_w, discrete: true });
        }
        self.amd.as_ref().map(|a| GpuVitals { temp_c: a.edge_c.or(a.junction_c), load: a.busy_percent.map(|b| b as f64), power_w: a.power_w, discrete: !self.amd_integrated })
    }

    /// Package power: RAPL when readable, else an APU's SoC power as its
    /// integrated GPU reports it.
    pub fn package_w(&self) -> Option<f64> {
        self.cpu.package_w.or_else(|| if self.amd_integrated { self.amd.as_ref().and_then(|a| a.power_w) } else { None })
    }

    /// The GPU temperature fan curves follow: the dGPU's while it is read.
    /// Unknown while it is awake but not read (settling, a switch running,
    /// resting after idle), so curves hold their duty rather than follow
    /// another GPU's temperature; otherwise the GPU `gpu()` shows.
    pub fn curve_gpu_temp(&self) -> Option<f64> {
        match &self.nvidia {
            Some(n) => n.temp_c.map(f64::from),
            None if self.gpu_unread() => None,
            None => self.gpu().and_then(|g| g.temp_c),
        }
    }

    /// The dGPU is awake but not being read (settling, a switch running,
    /// resting after idle): its temperature is unknown, not absent.
    pub fn gpu_unread(&self) -> bool {
        self.nvidia.is_none() && self.dgpu == Some(DgpuState::Active)
    }
}

/// What to do with NVML this sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NvmlAction {
    Poll,
    /// Keep no handle: the GPU is gone or asleep (NVML would wake it), or it
    /// was idle and gets a chance to suspend.
    Release,
}

/// `keep_away`: resting after idling, settling, or the gate is closed.
fn nvml_action(state: DgpuState, keep_away: bool) -> NvmlAction {
    match state {
        DgpuState::Active if !keep_away => NvmlAction::Poll,
        _ => NvmlAction::Release,
    }
}

/// When the NVIDIA GPU may be opened at all. supergfxd kills any process
/// holding it while it switches modes (it killed OmaAsus when a sleep hook
/// switched modes right after a resume), so it is opened only where the
/// graphics mode means it to be in use, not while a switch runs, and not in
/// the first seconds after it comes onto the bus.
#[derive(Debug, Clone, Copy, Default)]
struct DgpuGate {
    /// What the graphics mode says. `None` until known: where supergfxd runs,
    /// nothing opens the dGPU before its state has been read.
    mode_ok: Option<bool>,
    hold_until: Option<std::time::Instant>,
    /// When the dGPU came back on the bus while OmaAsus ran.
    arrived: Option<std::time::Instant>,
}

/// Why the dGPU may or may not be opened now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DgpuGateState {
    Open,
    /// The graphics mode uses it, but a switch is running or it just came onto the bus.
    Settling,
    /// The graphics mode doesn't use it.
    ModeOff,
    /// supergfxd runs but hasn't said which mode it is in.
    Unknown,
}

impl DgpuGate {
    fn state(&self, now: std::time::Instant) -> DgpuGateState {
        match self.mode_ok {
            None => DgpuGateState::Unknown,
            Some(false) => DgpuGateState::ModeOff,
            Some(true) if self.hold_until.is_none_or(|u| now >= u) && self.arrived.is_none_or(|t| now.duration_since(t) >= NVML_SETTLE) => DgpuGateState::Open,
            Some(true) => DgpuGateState::Settling,
        }
    }

    #[cfg(test)]
    fn open(&self, now: std::time::Instant) -> bool {
        self.state(now) == DgpuGateState::Open
    }
}

static GATE: std::sync::Mutex<DgpuGate> = std::sync::Mutex::new(DgpuGate { mode_ok: None, hold_until: None, arrived: None });

/// Whether the graphics mode means the dGPU to be in use (always, without a
/// graphics manager).
pub fn allow_dgpu(on: bool) {
    if let Ok(mut g) = GATE.lock() {
        if g.mode_ok != Some(on) {
            tracing::info!(allowed = on, "dGPU readings {}", if on { "allowed" } else { "off: the graphics mode doesn't use the dGPU" });
        }
        g.mode_ok = Some(on);
    }
}

/// Keep away from the dGPU for a while: while a graphics switch runs, and
/// before OmaAsus asks for one.
pub fn hold_off_dgpu(d: Duration) {
    if let Ok(mut g) = GATE.lock() {
        let until = std::time::Instant::now() + d;
        g.hold_until = Some(g.hold_until.map_or(until, |u| u.max(until)));
    }
}

/// No graphics manager runs, so nothing will power the dGPU off under OmaAsus.
/// Opens the gate unless the graphics mode has already had its say.
pub fn no_graphics_manager() {
    if let Ok(mut g) = GATE.lock()
        && g.mode_ok.is_none()
    {
        tracing::info!("dGPU readings allowed: no graphics manager runs");
        g.mode_ok = Some(true);
    }
}

/// Whether the graphics mode's answer is known yet.
pub fn dgpu_mode_known() -> bool {
    GATE.lock().is_ok_and(|g| g.mode_ok.is_some())
}

/// Why the dGPU may or may not be opened now (a poisoned lock reads as unknown: closed).
pub fn dgpu_gate() -> DgpuGateState {
    GATE.lock().map(|g| g.state(std::time::Instant::now())).unwrap_or(DgpuGateState::Unknown)
}

/// Whether anything in OmaAsus may open the dGPU now.
pub fn dgpu_open_ok() -> bool {
    dgpu_gate() == DgpuGateState::Open
}

/// When the dGPU counts as newly arrived: it came onto the bus after OmaAsus
/// started. One there from the start (a desktop card) doesn't wait.
fn next_arrival(was_present: Option<bool>, present: bool, arrived: Option<std::time::Instant>, now: std::time::Instant) -> Option<std::time::Instant> {
    match (was_present, present) {
        (Some(false), true) => Some(now),
        (_, false) => None,
        _ => arrived,
    }
}

/// Idle this long with a handle open, and the handle is let go.
const NVML_IDLE: Duration = Duration::from_secs(5);
/// A dGPU that comes onto the bus while OmaAsus runs is left alone this long first.
const NVML_SETTLE: Duration = Duration::from_secs(15);
/// After letting go, stay away this long so runtime PM can suspend the GPU.
const NVML_REST: Duration = Duration::from_secs(20);
/// How often to look for an NVIDIA GPU that isn't on the bus.
const NVML_RESCAN: Duration = Duration::from_secs(10);

/// NVML for the NVIDIA GPU, opened only while the GPU is awake and let go when
/// it idles: an open handle keeps the GPU out of runtime suspend and the
/// driver in use, which would also block supergfxd mode switches.
#[derive(Default)]
struct NvidiaSource {
    gpu: Option<NvidiaGpu>,
    device: Option<std::path::PathBuf>,
    idle_since: Option<std::time::Instant>,
    rest_until: Option<std::time::Instant>,
    next_scan: Option<std::time::Instant>,
    /// What the last sample found.
    state: Option<DgpuState>,
    /// Whether the last sample found the dGPU on the bus (`None` before the first).
    was_present: Option<bool>,
}

impl NvidiaSource {
    fn sample(&mut self, now: std::time::Instant) -> Option<NvidiaTelemetry> {
        if self.device.as_ref().is_none_or(|d| !d.exists()) && self.next_scan.is_none_or(|t| now >= t) {
            self.device = oma_hw::nvidia::pci_device();
            self.next_scan = Some(now + NVML_RESCAN);
        }
        let state = oma_hw::nvidia::power_state(self.device.as_deref());
        self.state = Some(state);
        // A dGPU that comes onto the bus while running (resume, a mode switch) is
        // often about to be switched off again: everything waits it out (the gate).
        let present = !matches!(state, DgpuState::Absent);
        if let Ok(mut g) = GATE.lock() {
            g.arrived = next_arrival(self.was_present, present, g.arrived, now);
        }
        self.was_present = Some(present);
        let keep_away = self.rest_until.is_some_and(|t| now < t) || !dgpu_open_ok();
        if nvml_action(state, keep_away) == NvmlAction::Release {
            self.gpu = None;
            self.idle_since = None;
            return None;
        }
        if self.gpu.is_none() {
            self.gpu = NvidiaGpu::open(0).ok();
        }
        let t = self.gpu.as_ref()?.telemetry().ok()?;
        let busy = t.util_gpu.unwrap_or(0) > 0 || t.process_count.unwrap_or(0) > 0;
        // Idle, it is let go so runtime PM can suspend it; where runtime PM may
        // not (a desktop card), it is read without pause.
        if busy {
            self.idle_since = None;
        } else if now.duration_since(*self.idle_since.get_or_insert(now)) >= NVML_IDLE && oma_hw::nvidia::runtime_pm_allowed(self.device.as_deref()) {
            self.gpu = None;
            self.idle_since = None;
            self.rest_until = Some(now + NVML_REST);
        }
        Some(t)
    }
}

/// What one hwmon device's reader thread returns: indices into the device's
/// inputs, so labels and roles stay with the metadata on the sampler side.
struct HwmonSample {
    temps: Vec<(usize, f64)>,
    fans: Vec<(usize, u64, Option<f64>)>,
}

fn read_hwmon(d: &HwmonDevice, temp_idx: &[usize]) -> HwmonSample {
    let temps = temp_idx.iter().filter_map(|&i| d.temps[i].read().map(|v| (i, v))).collect();
    let fans = d
        .fans
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let rpm = f.read_rpm()?;
            let duty = d.pwms.iter().find(|p| p.index == f.index && p.has_duty).map(|p| p.read().value as f64 / 2.55);
            Some((i, rpm, duty))
        })
        .collect();
    HwmonSample { temps, fans }
}

struct Sampler {
    cpu: CpuMonitor,
    nvidia: NvidiaSource,
    amd: Vec<AmdGpu>,
    hwmon: Vec<HwmonDevice>,
    /// One bounded reader per hwmon device, same order as `hwmon`.
    hwmon_workers: Vec<Worker<HwmonSample>>,
    lianli: Vec<LianLiHub>,
    lianli_workers: Vec<Worker<Option<[u16; 4]>>>,
    /// Devices whose expected channels were registered offline because they
    /// never answered.
    registered_offline: std::collections::HashSet<usize>,
    /// Persistent registries: channels never disappear once discovered.
    fans: std::collections::HashMap<String, Tracked<FanReading>>,
    temps: std::collections::HashMap<String, Tracked<Reading>>,
    /// Some driver names its fans, so firmware duplicates of them can go.
    named_tachs: bool,
    seq: u64,
}

/// The hardware model, for names and limits, once detection has built it.
static MODEL: std::sync::RwLock<Option<Arc<HardwareModel>>> = std::sync::RwLock::new(None);

pub fn set_model(m: Arc<HardwareModel>) {
    if let Ok(mut g) = MODEL.write() {
        *g = Some(m);
    }
}

fn model() -> Option<Arc<HardwareModel>> {
    MODEL.read().ok()?.clone()
}

/// A fan's name and top speed from the model output it belongs to, by the
/// output's id or its tachometer's.
fn fan_identity(model: Option<&HardwareModel>, id: &str, raw: &str) -> (String, Option<u64>) {
    match model.and_then(|m| m.fans.iter().find(|f| f.id.as_str() == id || f.tach.as_ref().is_some_and(|t| t.as_str() == id))) {
        Some(f) => (f.label.clone(), f.caps.max_rpm.map(u64::from)),
        None => (raw.to_string(), None),
    }
}

/// A readable name for a temperature the driver only numbers ("temp1").
fn temp_label(d: &HwmonDevice, label: &str) -> String {
    match label.strip_prefix("temp").filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        Some(n) => format!("{} {n}", d.friendly_name()),
        None => label.replace('_', " "),
    }
}

/// How long one sample waits for every device's reading; a device that
/// misses it keeps its request out and is collected on a later cycle.
const COLLECT_BUDGET: Duration = Duration::from_millis(500);

impl Sampler {
    fn new() -> Self {
        let hwmon = hwmon::enumerate();
        let hwmon_workers = hwmon
            .iter()
            .map(|d| {
                let dev = d.clone();
                let temp_idx: Vec<usize> = d.temps.iter().enumerate().filter(|(_, t)| !knowledge::sensor_hidden(&d.name, &t.label)).map(|(i, _)| i).collect();
                Worker::spawn(d.friendly_name().to_string(), move || read_hwmon(&dev, &temp_idx))
            })
            .collect();
        let lianli = LianLiHub::enumerate();
        let lianli_workers = lianli
            .iter()
            .map(|h| {
                let hub = h.clone();
                let mut warned = false;
                Worker::spawn(h.kind.label().to_string(), move || match hub.read_rpm() {
                    Ok(r) => Some(r),
                    Err(e) => {
                        // Usually the node is root-only (no udev rule): say so once
                        // instead of showing a hub with no fans.
                        if !warned {
                            tracing::warn!(hub = %hub.path, error = %e, "fan hub not readable; its speeds are not shown");
                            warned = true;
                        }
                        None
                    }
                })
            })
            .collect();
        let named_tachs = hwmon.iter().any(|d| d.fans.iter().any(|f| knowledge::tach_role(&d.name, &f.label) == TachRole::Fan && f.label != format!("fan{}", f.index)));
        Self { cpu: CpuMonitor::new(), nvidia: NvidiaSource::default(), amd: AmdGpu::enumerate(), hwmon, hwmon_workers, lianli, lianli_workers, registered_offline: Default::default(), fans: Default::default(), temps: Default::default(), named_tachs, seq: 0 }
    }

    /// A device that has never answered: register what it *should* expose,
    /// flagged offline, so it stays visible rather than absent.
    fn register_offline(&mut self, i: usize) {
        if !self.registered_offline.insert(i) {
            return;
        }
        let d = &self.hwmon[i];
        let long_ago = std::time::Instant::now().checked_sub(OFFLINE_AFTER).unwrap_or_else(std::time::Instant::now);
        for f in d.fans.iter().filter(|f| matches!(knowledge::tach_role(&d.name, &f.label), TachRole::Fan | TachRole::Pump)) {
            let key = format!("{}:{}", d.name, f.label);
            if !self.fans.contains_key(&key) {
                let order = self.fans.len();
                self.fans.insert(key, Tracked { value: FanReading { label: f.label.clone(), device: d.friendly_name().to_string(), freshness: Freshness::Offline, ..Default::default() }, last_seen: long_ago, order });
            }
        }
        // Coolant is what a stalled cooler most needs watching for.
        for t in d.temps.iter().filter(|t| knowledge::sensor_role(&d.name, &t.label, false) == SensorRole::Coolant) {
            let key = format!("{}:{}", d.name, t.label);
            if !self.temps.contains_key(&key) {
                let order = self.temps.len();
                self.temps.insert(key, Tracked { value: Reading { label: temp_label(d, &t.label), value: 0.0, freshness: Freshness::Offline }, last_seen: long_ago, order });
            }
        }
    }

    fn track_fan(&mut self, key: String, mut r: FanReading, now: std::time::Instant) {
        let order = self.fans.len();
        r.freshness = Freshness::Live;
        let e = self.fans.entry(key).or_insert_with(|| Tracked { value: r.clone(), last_seen: now, order });
        r.peak_rpm = e.value.peak_rpm.max(r.rpm);
        e.value = r;
        e.last_seen = now;
    }

    fn track_temp(&mut self, key: String, label: String, v: f64, now: std::time::Instant) {
        let order = self.temps.len();
        let e = self.temps.entry(key).or_insert_with(|| Tracked { value: Reading { label: label.clone(), value: v, freshness: Freshness::Live }, last_seen: now, order });
        e.value.value = v;
        e.value.label = label;
        e.last_seen = now;
    }

    fn sample(&mut self) -> Snapshot {
        self.seq += 1;
        let model = model();
        let mut s = Snapshot { seq: self.seq, cpu: self.cpu.sample(), ..Default::default() };
        s.nvidia = self.nvidia.sample(std::time::Instant::now());
        s.dgpu = self.nvidia.state;
        let amd = self.amd.iter().find(|g| !g.is_integrated).or(self.amd.first());
        s.amd = amd.map(|g| g.telemetry());
        s.amd_integrated = amd.is_some_and(|g| g.is_integrated);
        s.cpu_control = oma_hw::cpu::control_state();
        let now = std::time::Instant::now();
        let mut pending_temps: Vec<(String, String, f64, std::time::Instant)> = Vec::new();
        let mut pending_fans: Vec<(String, FanReading, std::time::Instant)> = Vec::new();
        // Ask every device at once, then collect within one budget: a device
        // that does not answer costs the rest nothing beyond that budget.
        for w in &mut self.hwmon_workers {
            w.request(now);
        }
        for w in &mut self.lianli_workers {
            w.request(now);
        }
        let deadline = now + COLLECT_BUDGET;
        // Each reading is dated when it was asked for: one that took long to
        // arrive goes into the registry with that age and never into this
        // frame's control inputs, so a 30-second-old coolant temperature is
        // not what a curve acts on.
        let mut samples: Vec<Option<(HwmonSample, std::time::Instant)>> = Vec::with_capacity(self.hwmon.len());
        for w in &mut self.hwmon_workers {
            let wait = deadline.saturating_duration_since(std::time::Instant::now());
            samples.push(match w.collect(wait, now) {
                Poll::Ready(r, taken) => Some((r, taken)),
                Poll::Stalled | Poll::Dead => None,
            });
        }
        let never_answered: Vec<usize> = self.hwmon_workers.iter().enumerate().filter(|(_, w)| !w.healthy() && w.last_ok().is_none()).map(|(i, _)| i).collect();
        for i in never_answered {
            self.register_offline(i);
        }
        for (i, sample) in samples.into_iter().enumerate() {
            let Some((sample, taken)) = sample else { continue };
            let fresh = now.duration_since(taken) < STALE_AFTER;
            let d = &self.hwmon[i];
            let igpu = self.amd.iter().any(|g| g.is_integrated && g.device_path == d.device_path);
            let (mut storage, mut memory) = (false, false);
            for (ti, v) in sample.temps {
                let t = &d.temps[ti];
                let key = format!("{}:{}", d.name, t.label);
                if !fresh {
                    // Old news: shown with its age, not fed to anything that acts.
                    pending_temps.push((key, temp_label(d, &t.label), v, taken));
                    continue;
                }
                s.hwmon_temps.insert((d.name.clone(), t.label.clone()), v);
                match knowledge::sensor_role(&d.name, &t.label, igpu) {
                    // Shown with the CPU and GPU figures, or not a component's temperature.
                    SensorRole::CpuTemp | SensorRole::CpuCore | SensorRole::IgpuTemp | SensorRole::DgpuTemp | SensorRole::GpuHotspot | SensorRole::Wireless => {}
                    // One reading per drive and memory module: the first is the overall one.
                    SensorRole::Storage => {
                        if !std::mem::replace(&mut storage, true) {
                            s.nvme_c.push(v);
                        }
                    }
                    SensorRole::Memory => {
                        if !std::mem::replace(&mut memory, true) {
                            s.dimm_c.push(v);
                        }
                    }
                    SensorRole::Vrm => s.vrm_c = s.vrm_c.or(Some(v)),
                    SensorRole::Board => s.board_c = s.board_c.or(Some(v)),
                    SensorRole::Coolant => {
                        s.coolant_c = s.coolant_c.or(Some(v));
                        pending_temps.push((key, temp_label(d, &t.label), v, taken));
                    }
                    _ => pending_temps.push((key, temp_label(d, &t.label), v, taken)),
                }
            }
            for (fi, rpm, duty) in sample.fans {
                let f = &d.fans[fi];
                let role = knowledge::tach_role(&d.name, &f.label);
                if role == TachRole::Duplicate && self.named_tachs {
                    continue;
                }
                if role == TachRole::Pump && fresh {
                    s.pump_rpm = Some(rpm);
                }
                let key = format!("{}:{}", d.name, f.label);
                // 0 rpm is a stopped fan, not a missing one, once the fan is known:
                // it has spun before, or the driver names it. Unnamed inputs that
                // never spun are unused headers and stay hidden.
                let named = f.label != format!("fan{}", f.index);
                if rpm == 0 && (role == TachRole::WhenSpinning || (!named && !self.fans.contains_key(&key))) {
                    continue;
                }
                let (label, max_rpm) = fan_identity(model.as_deref(), &format!("hwmon:{}:{}", d.name, f.label), &f.label);
                pending_fans.push((key, FanReading { label, rpm, duty, device: d.friendly_name().to_string(), freshness: Freshness::Live, max_rpm, peak_rpm: 0 }, taken));
            }
        }
        // Lian Li hub tachometers (HID input report).
        for (i, w) in self.lianli_workers.iter_mut().enumerate() {
            let wait = deadline.saturating_duration_since(std::time::Instant::now());
            let Poll::Ready(Some(rpm), taken) = w.collect(wait, now) else { continue };
            let h = &self.lianli[i];
            for (c, r) in rpm.iter().enumerate() {
                if *r > 0 {
                    let (label, max_rpm) = fan_identity(model.as_deref(), &format!("lianli:{i}:ch{}", c + 1), &format!("{} channel {}", h.kind.label(), c + 1));
                    pending_fans.push((format!("lianli{i}:{}", c + 1), FanReading { label, rpm: *r as u64, duty: None, device: h.kind.label().to_string(), freshness: Freshness::Live, max_rpm, peak_rpm: 0 }, taken));
                }
            }
        }
        s.sources = self
            .hwmon_workers
            .iter()
            .map(|w| SourceHealth { name: w.name().to_string(), healthy: w.healthy(), stalled_s: w.stalled_for(now).map(|d| d.as_secs()) })
            .chain(self.lianli_workers.iter().map(|w| SourceHealth { name: w.name().to_string(), healthy: w.healthy(), stalled_s: w.stalled_for(now).map(|d| d.as_secs()) }))
            .collect();
        for (key, label, v, taken) in pending_temps {
            self.track_temp(key, label, v, taken);
        }
        for (key, r, taken) in pending_fans {
            self.track_fan(key, r, taken);
        }
        // Emit the registries in discovery order with freshness flags.
        let mut fans: Vec<&Tracked<FanReading>> = self.fans.values().collect();
        fans.sort_by_key(|t| t.order);
        s.fans = fans.into_iter().map(|t| FanReading { freshness: freshness(t.last_seen, now), ..t.value.clone() }).collect();
        let mut temps: Vec<&Tracked<Reading>> = self.temps.values().collect();
        temps.sort_by_key(|t| t.order);
        s.temps = temps.into_iter().map(|t| Reading { freshness: freshness(t.last_seen, now), ..t.value.clone() }).collect();
        s
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Frame(std::sync::Arc<Snapshot>),
}

/// Samples per second, from the Settings page (1..=5).
static RATE_HZ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(2);

pub fn set_rate(hz: u32) {
    RATE_HZ.store(hz.clamp(1, 5), std::sync::atomic::Ordering::Relaxed);
}

fn period() -> Duration {
    Duration::from_millis(1000 / u64::from(RATE_HZ.load(std::sync::atomic::Ordering::Relaxed).max(1)))
}

pub fn stream() -> impl Stream<Item = Event> {
    iced::stream::channel(8, async move |mut out| {
        let mut sampler = tokio::task::spawn_blocking(Sampler::new).await.expect("sampler");
        let mut next = tokio::time::Instant::now();
        loop {
            // The rate can change while running; a late sample doesn't cause a burst.
            next = (next + period()).max(tokio::time::Instant::now());
            tokio::time::sleep_until(next).await;
            let (snap, s) = tokio::task::spawn_blocking(move || {
                let snap = sampler.sample();
                (snap, sampler)
            })
            .await
            .expect("sampler task");
            sampler = s;
            if out.send(Event::Frame(std::sync::Arc::new(snap))).await.is_err() {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvml_is_only_used_while_the_gpu_is_awake() {
        assert_eq!(nvml_action(DgpuState::Active, false), NvmlAction::Poll);
        assert_eq!(nvml_action(DgpuState::Active, true), NvmlAction::Release, "resting after idling");
        assert_eq!(nvml_action(DgpuState::Suspended, false), NvmlAction::Release, "NVML would wake it");
        assert_eq!(nvml_action(DgpuState::Absent, false), NvmlAction::Release);
    }

    #[test]
    fn the_dgpu_gate_opens_only_when_everything_agrees() {
        let now = std::time::Instant::now();
        let ago = |s| now.checked_sub(Duration::from_secs(s)).expect("uptime");
        let known = DgpuGate { mode_ok: Some(true), ..Default::default() };
        assert!(known.open(now));
        assert!(!DgpuGate::default().open(now), "supergfxd's state not read yet");
        assert!(!DgpuGate { mode_ok: Some(false), ..known }.open(now), "the graphics mode doesn't use it");
        assert!(!DgpuGate { hold_until: Some(now + Duration::from_secs(5)), ..known }.open(now), "a switch is running");
        assert!(DgpuGate { hold_until: Some(ago(1)), ..known }.open(now), "the switch is over");
        assert!(!DgpuGate { arrived: Some(ago(3)), ..known }.open(now), "just came onto the bus");
        assert!(DgpuGate { arrived: Some(ago(20)), ..known }.open(now), "settled");
        assert_eq!(DgpuGate::default().state(now), DgpuGateState::Unknown);
        assert_eq!(DgpuGate { mode_ok: Some(false), ..known }.state(now), DgpuGateState::ModeOff);
        assert_eq!(DgpuGate { arrived: Some(ago(3)), ..known }.state(now), DgpuGateState::Settling);
    }

    #[test]
    fn curves_never_follow_another_gpu_for_an_unread_dgpu() {
        let igpu = AmdGpuTelemetry { edge_c: Some(45.0), ..Default::default() };
        let awake_unread = Snapshot { amd: Some(igpu), amd_integrated: true, dgpu: Some(DgpuState::Active), ..Default::default() };
        assert!(awake_unread.gpu_unread());
        assert_eq!(awake_unread.curve_gpu_temp(), None);
        let off = Snapshot { dgpu: Some(DgpuState::Absent), ..awake_unread.clone() };
        assert_eq!(off.curve_gpu_temp(), Some(45.0), "with the dGPU off the iGPU is the GPU");
    }

    #[test]
    fn only_a_dgpu_that_comes_back_waits() {
        let now = std::time::Instant::now();
        assert_eq!(next_arrival(None, true, None, now), None, "there from the start");
        assert_eq!(next_arrival(Some(false), true, None, now), Some(now), "came back on the bus");
        assert_eq!(next_arrival(Some(true), true, Some(now), now), Some(now), "still settling");
        assert_eq!(next_arrival(Some(true), false, Some(now), now), None, "gone again");
    }

    #[test]
    fn telemetry_rate_is_clamped() {
        set_rate(0);
        assert_eq!(period(), Duration::from_millis(1000));
        set_rate(50);
        assert_eq!(period(), Duration::from_millis(200));
        set_rate(2);
        assert_eq!(period(), Duration::from_millis(500));
    }
}

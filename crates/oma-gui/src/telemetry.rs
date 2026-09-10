//! Background telemetry sampler → iced subscription.

use iced::futures::SinkExt;
use iced::futures::Stream;
use oma_hw::cpu::{CpuMonitor, CpuTelemetry};
use oma_hw::hwmon::{self, HwmonDevice};
use oma_hw::nvidia::{NvidiaGpu, NvidiaTelemetry};
use oma_hw::amdgpu::{AmdGpu, AmdGpuTelemetry};
use oma_hw::lianli::LianLiHub;
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
    /// Super I/O tachometers by pwm index (rpm), for the Cooling page.
    pub superio_rpm: std::collections::BTreeMap<u32, u64>,
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
}

struct Sampler {
    cpu: CpuMonitor,
    nvidia: Option<NvidiaGpu>,
    amd: Vec<AmdGpu>,
    hwmon: Vec<HwmonDevice>,
    lianli: Vec<LianLiHub>,
    /// Devices whose sysfs reads stalled (e.g. a wedged USB AIO): skipped until the instant.
    quarantine: std::collections::HashMap<String, std::time::Instant>,
    /// Persistent registries: channels never disappear once discovered.
    fans: std::collections::HashMap<String, Tracked<FanReading>>,
    temps: std::collections::HashMap<String, Tracked<Reading>>,
    seq: u64,
}

/// A sysfs read that takes longer than this is a stalled device, not a sensor.
const STALL: Duration = Duration::from_millis(250);
const QUARANTINE: Duration = Duration::from_secs(60);

/// Probe a device's first input on a helper thread; `false` if it does not answer in time.
fn responsive(d: &HwmonDevice) -> bool {
    let path = d.temps.first().map(|t| t.input.clone()).or_else(|| d.fans.first().map(|f| f.input.clone()));
    let Some(path) = path else { return true };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = std::fs::read_to_string(&path);
        let _ = tx.send(());
    });
    rx.recv_timeout(STALL * 4).is_ok()
}

impl Sampler {
    fn new() -> Self {
        let hwmon = hwmon::enumerate();
        let mut quarantine = std::collections::HashMap::new();
        let mut fans: std::collections::HashMap<String, Tracked<FanReading>> = Default::default();
        let mut temps: std::collections::HashMap<String, Tracked<Reading>> = Default::default();
        let long_ago = std::time::Instant::now() - OFFLINE_AFTER;
        for d in &hwmon {
            if !responsive(d) {
                tracing::warn!(device = %d.name, "hwmon device not answering; quarantined for {}s", QUARANTINE.as_secs());
                quarantine.insert(d.path.to_string_lossy().into_owned(), std::time::Instant::now() + QUARANTINE);
                // Register what the device *should* expose, flagged offline, so it stays visible.
                for f in &d.fans {
                    let essential = d.name != "rog_ryujin" || f.label.starts_with("Pump");
                    if essential {
                        let order = fans.len();
                        fans.insert(format!("{}:{}", d.name, f.label), Tracked { value: FanReading { label: f.label.clone(), rpm: 0, duty: None, device: d.friendly_name().to_string(), freshness: Freshness::Offline }, last_seen: long_ago, order });
                    }
                }
                if d.name == "rog_ryujin" {
                    let order = temps.len();
                    temps.insert("rog_ryujin:Coolant".into(), Tracked { value: Reading { label: "Coolant".into(), value: 0.0, freshness: Freshness::Offline }, last_seen: long_ago, order });
                }
            }
        }
        Self { cpu: CpuMonitor::new(), nvidia: NvidiaGpu::open(0).ok(), amd: AmdGpu::enumerate(), hwmon, lianli: LianLiHub::enumerate(), quarantine, fans, temps, seq: 0 }
    }

    fn track_fan(&mut self, key: String, mut r: FanReading, now: std::time::Instant) {
        let order = self.fans.len();
        r.freshness = Freshness::Live;
        let e = self.fans.entry(key).or_insert_with(|| Tracked { value: r.clone(), last_seen: now, order });
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
        let mut s = Snapshot { seq: self.seq, cpu: self.cpu.sample(), ..Default::default() };
        s.nvidia = self.nvidia.as_ref().and_then(|g| g.telemetry().ok());
        let amd = self.amd.iter().find(|g| !g.is_integrated).or(self.amd.first());
        s.amd = amd.map(|g| g.telemetry());
        s.amd_integrated = amd.is_some_and(|g| g.is_integrated);
        s.cpu_control = oma_hw::cpu::control_state();
        let now = std::time::Instant::now();
        let mut stalled: Vec<String> = Vec::new();
        let mut pending_temps: Vec<(String, String, f64)> = Vec::new();
        let mut pending_fans: Vec<(String, FanReading)> = Vec::new();
        for d in &self.hwmon {
            if self.quarantine.get(&d.path.to_string_lossy().into_owned()).is_some_and(|until| *until > now) {
                continue;
            }
            let t_dev = std::time::Instant::now();
            match d.name.as_str() {
                "nvme" => {
                    if let Some(t) = d.temps.first().and_then(|t| t.read()) {
                        s.nvme_c.push(t);
                    }
                }
                "spd5118" => {
                    if let Some(t) = d.temps.first().and_then(|t| t.read()) {
                        s.dimm_c.push(t);
                    }
                }
                "k10temp" | "zenpower" | "coretemp" | "amdgpu" | "iwlwifi_1" | "iwlwifi" => {}
                _ => {
                    for t in &d.temps {
                        if t_dev.elapsed() > STALL {
                            break;
                        }
                        if let Some(v) = t.read() {
                            s.hwmon_temps.insert((d.name.clone(), t.label.clone()), v);
                            match (d.name.as_str(), t.label.as_str()) {
                                ("rog_ryujin", _) => s.coolant_c = Some(v),
                                ("asusec", "VRM") => s.vrm_c = Some(v),
                                ("asusec", "Motherboard") => s.board_c = Some(v),
                                ("asusec", "Water_In") | ("asusec", "Water_Out") => pending_temps.push((format!("{}:{}", d.name, t.label), t.label.replace('_', " "), v)),
                                ("asusec", "CPU") | ("asusec", "CPU Package") | ("asusec", "T_Sensor") => {}
                                (n, l) if n.starts_with("nct6") => {
                                    if !l.starts_with("PCH") && !l.starts_with("AUXTIN") && !l.starts_with("PECI") && !l.starts_with("TSI") && !l.starts_with("CPUTIN") && !l.starts_with("SYSTIN") {
                                        pending_temps.push((format!("{}:{}", d.name, l), l.to_string(), v));
                                    }
                                }
                                (_, l) => pending_temps.push((format!("{}:{}", d.name, l), l.to_string(), v)),
                            }
                        }
                    }
                    for f in &d.fans {
                        if t_dev.elapsed() > STALL {
                            break;
                        }
                        // A failed read is no reading: the fan goes stale, then offline.
                        let Some(rpm) = f.read_rpm() else { continue };
                        if d.is_super_io() {
                            s.superio_rpm.insert(f.index, rpm);
                        }
                        if d.name == "rog_ryujin" && f.label.starts_with("Pump") {
                            s.pump_rpm = Some(rpm);
                        }
                        let key = format!("{}:{}", d.name, f.label);
                        // 0 rpm is a stopped fan, not a missing one, once the fan is known:
                        // it has spun before, or the driver names it. Unnamed inputs that
                        // never spun are unused headers and stay hidden.
                        let named = f.label != format!("fan{}", f.index);
                        if rpm == 0 && !named && !self.fans.contains_key(&key) {
                            continue;
                        }
                        if d.name == "rog_ryujin" && rpm == 0 && f.label.starts_with("Controller fan") {
                            continue;
                        }
                        let duty = d.pwms.iter().find(|p| p.index == f.index && p.has_duty).map(|p| p.read().value as f64 / 2.55);
                        pending_fans.push((key, FanReading { label: f.label.clone(), rpm, duty, device: d.friendly_name().to_string(), freshness: Freshness::Live }));
                    }
                }
            }
            if t_dev.elapsed() > STALL {
                stalled.push(d.path.to_string_lossy().into_owned());
                tracing::warn!(device = %d.name, ms = t_dev.elapsed().as_millis(), "hwmon reads stalled; quarantining for {}s", QUARANTINE.as_secs());
            }
        }
        for key in stalled {
            self.quarantine.insert(key, now + QUARANTINE);
        }
        // Lian Li hub tachometers (HID input report).
        for (i, h) in self.lianli.iter().enumerate() {
            let key = format!("lianli{i}");
            if self.quarantine.get(&key).is_some_and(|u| *u > now) {
                continue;
            }
            let t = std::time::Instant::now();
            if let Ok(rpm) = h.read_rpm() {
                for (c, r) in rpm.iter().enumerate() {
                    if *r > 0 {
                        pending_fans.push((format!("lianli{i}:{}", c + 1), FanReading { label: format!("Lian Li channel {}", c + 1), rpm: *r as u64, duty: None, device: h.kind.label().to_string(), freshness: Freshness::Live }));
                    }
                }
            }
            if t.elapsed() > STALL {
                self.quarantine.insert(key, now + QUARANTINE);
            }
        }
        for (key, label, v) in pending_temps {
            self.track_temp(key, label, v, now);
        }
        for (key, r) in pending_fans {
            self.track_fan(key, r, now);
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

pub fn stream() -> impl Stream<Item = Event> {
    iced::stream::channel(8, async move |mut out| {
        let mut sampler = tokio::task::spawn_blocking(Sampler::new).await.expect("sampler");
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
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

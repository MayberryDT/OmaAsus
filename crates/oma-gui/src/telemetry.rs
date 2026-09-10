//! Background telemetry sampler → iced subscription.

use iced::futures::SinkExt;
use iced::futures::Stream;
use oma_hw::cpu::{CpuMonitor, CpuTelemetry};
use oma_hw::hwmon::{self, HwmonDevice};
use oma_hw::nvidia::{NvidiaGpu, NvidiaTelemetry};
use oma_hw::amdgpu::{AmdGpu, AmdGpuTelemetry};
use oma_hw::lianli::LianLiHub;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, Default)]
pub struct FanReading {
    pub label: String,
    pub rpm: u64,
    pub duty: Option<f64>,
    pub device: String,
}

/// One telemetry frame.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub seq: u64,
    pub cpu: CpuTelemetry,
    pub nvidia: Option<NvidiaTelemetry>,
    pub amd: Option<AmdGpuTelemetry>,
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

struct Sampler {
    cpu: CpuMonitor,
    nvidia: Option<NvidiaGpu>,
    amd: Vec<AmdGpu>,
    hwmon: Vec<HwmonDevice>,
    lianli: Vec<LianLiHub>,
    /// Devices whose sysfs reads stalled (e.g. a wedged USB AIO): skipped until the instant.
    quarantine: std::collections::HashMap<String, std::time::Instant>,
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
        for d in &hwmon {
            if !responsive(d) {
                tracing::warn!(device = %d.name, "hwmon device not answering; quarantined for {}s", QUARANTINE.as_secs());
                quarantine.insert(d.path.to_string_lossy().into_owned(), std::time::Instant::now() + QUARANTINE);
            }
        }
        Self { cpu: CpuMonitor::new(), nvidia: NvidiaGpu::open(0).ok(), amd: AmdGpu::enumerate(), hwmon, lianli: LianLiHub::enumerate(), quarantine, seq: 0 }
    }

    fn sample(&mut self) -> Snapshot {
        self.seq += 1;
        let mut s = Snapshot { seq: self.seq, cpu: self.cpu.sample(), ..Default::default() };
        s.nvidia = self.nvidia.as_ref().and_then(|g| g.telemetry().ok());
        s.amd = self.amd.iter().find(|g| !g.is_integrated).or(self.amd.first()).map(|g| g.telemetry());
        s.cpu_control = oma_hw::cpu::control_state();
        let now = std::time::Instant::now();
        let mut stalled: Vec<String> = Vec::new();
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
                                ("asusec", "Water_In") | ("asusec", "Water_Out") => s.temps.push(Reading { label: t.label.replace('_', " "), value: v }),
                                ("asusec", "CPU") | ("asusec", "CPU Package") | ("asusec", "T_Sensor") => {}
                                (n, l) if n.starts_with("nct6") => {
                                    if !l.starts_with("PCH") && !l.starts_with("AUXTIN") && !l.starts_with("PECI") && !l.starts_with("TSI") && !l.starts_with("CPUTIN") && !l.starts_with("SYSTIN") {
                                        s.temps.push(Reading { label: l.to_string(), value: v });
                                    }
                                }
                                (_, l) => s.temps.push(Reading { label: l.to_string(), value: v }),
                            }
                        }
                    }
                    for f in &d.fans {
                        if t_dev.elapsed() > STALL {
                            break;
                        }
                        let rpm = f.read_rpm().unwrap_or(0);
                        if d.is_super_io() {
                            s.superio_rpm.insert(f.index, rpm);
                        }
                        if d.name == "rog_ryujin" && f.label.starts_with("Pump") {
                            s.pump_rpm = Some(rpm);
                        }
                        if rpm == 0 && !d.name.starts_with("rog_") {
                            continue;
                        }
                        if d.name == "rog_ryujin" && rpm == 0 && f.label.starts_with("Controller fan") {
                            continue;
                        }
                        let duty = d.pwms.iter().find(|p| p.index == f.index).map(|p| p.read().value as f64 / 2.55);
                        s.fans.push(FanReading { label: f.label.clone(), rpm, duty, device: d.friendly_name().to_string() });
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
        // Lian Li hub tachometers (HID input report), every other tick.
        if self.seq % 2 == 0 {
            for (i, h) in self.lianli.iter().enumerate() {
                let key = format!("lianli{i}");
                if self.quarantine.get(&key).is_some_and(|u| *u > now) {
                    continue;
                }
                let t = std::time::Instant::now();
                if let Ok(rpm) = h.read_rpm() {
                    for (c, r) in rpm.iter().enumerate() {
                        if *r > 0 {
                            s.fans.push(FanReading { label: format!("Lian Li channel {}", c + 1), rpm: *r as u64, duty: None, device: h.kind.label().to_string() });
                        }
                    }
                }
                if t.elapsed() > STALL {
                    self.quarantine.insert(key, now + QUARANTINE);
                }
            }
        }
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

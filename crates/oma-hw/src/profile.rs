//! Gaming profile model: a named bundle of CPU, GPU, cooling, lighting and
//! platform settings, plus the automation rules that select profiles.

use crate::cpu::CpuControlState;
use crate::nvidia::NvidiaControl;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A fan curve in device-agnostic terms: (°C, duty %) points.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FanCurve {
    pub points: Vec<(f64, f64)>,
    /// Which temperature drives the curve.
    pub source: TempSource,
    /// Hysteresis / smoothing in °C.
    pub hysteresis_c: f64,
    /// Minimum duty ever applied (protects pumps).
    pub min_duty: f64,
    /// Seconds over which duty ramps (0 = instant).
    pub ramp_s: f64,
}

impl FanCurve {
    /// Interpolate duty (0..=100) for a temperature.
    pub fn duty_at(&self, temp: f64) -> f64 {
        let pts = &self.points;
        if pts.is_empty() {
            return self.min_duty.max(0.0);
        }
        let mut sorted: Vec<(f64, f64)> = pts.clone();
        sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if temp <= sorted[0].0 {
            return sorted[0].1.max(self.min_duty);
        }
        for w in sorted.windows(2) {
            let (t0, d0) = w[0];
            let (t1, d1) = w[1];
            if temp <= t1 {
                let f = if (t1 - t0).abs() < f64::EPSILON { 1.0 } else { (temp - t0) / (t1 - t0) };
                return (d0 + f * (d1 - d0)).max(self.min_duty).clamp(0.0, 100.0);
            }
        }
        sorted.last().map(|p| p.1).unwrap_or(100.0).max(self.min_duty).clamp(0.0, 100.0)
    }

    pub fn silent() -> Self {
        Self { points: vec![(30.0, 20.0), (50.0, 30.0), (65.0, 45.0), (75.0, 70.0), (85.0, 100.0)], source: TempSource::CpuTctl, hysteresis_c: 2.0, min_duty: 20.0, ramp_s: 4.0 }
    }
    pub fn balanced() -> Self {
        Self { points: vec![(30.0, 30.0), (50.0, 40.0), (60.0, 55.0), (70.0, 75.0), (80.0, 100.0)], source: TempSource::CpuTctl, hysteresis_c: 1.5, min_duty: 25.0, ramp_s: 3.0 }
    }
    pub fn performance() -> Self {
        Self { points: vec![(30.0, 40.0), (45.0, 55.0), (55.0, 70.0), (65.0, 85.0), (75.0, 100.0)], source: TempSource::CpuTctl, hysteresis_c: 1.0, min_duty: 35.0, ramp_s: 2.0 }
    }
    pub fn coolant() -> Self {
        Self { points: vec![(25.0, 30.0), (30.0, 40.0), (35.0, 60.0), (40.0, 85.0), (45.0, 100.0)], source: TempSource::Coolant, hysteresis_c: 0.5, min_duty: 30.0, ramp_s: 5.0 }
    }
    pub fn pump() -> Self {
        Self { points: vec![(25.0, 60.0), (32.0, 70.0), (38.0, 85.0), (42.0, 100.0)], source: TempSource::Coolant, hysteresis_c: 0.5, min_duty: 60.0, ramp_s: 6.0 }
    }
}

/// Where a curve reads its temperature from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TempSource {
    #[default]
    CpuTctl,
    CpuPackage,
    Gpu,
    Coolant,
    Vrm,
    Motherboard,
    /// Max of CPU and GPU: the usual case-fan driver.
    CpuGpuMax,
    /// Arbitrary hwmon input: (driver name, temp label).
    Hwmon { driver: String, label: String },
}

impl TempSource {
    pub fn label(&self) -> String {
        match self {
            Self::CpuTctl => "CPU (Tctl)".into(),
            Self::CpuPackage => "CPU package".into(),
            Self::Gpu => "GPU".into(),
            Self::Coolant => "Coolant".into(),
            Self::Vrm => "VRM".into(),
            Self::Motherboard => "Motherboard".into(),
            Self::CpuGpuMax => "CPU / GPU max".into(),
            Self::Hwmon { driver, label } => format!("{driver}: {label}"),
        }
    }
}

/// A fan output that a curve can drive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FanTarget {
    /// nct6775 `pwmN`.
    SuperIo(u32),
    RyujinPump,
    RyujinInternalFan,
    RyujinExternalFans,
    NvidiaFans,
    LianLiChannel(u8),
    /// A CoolerControl device UID + channel name.
    CoolerControl { device_uid: String, channel: String },
}

impl FanTarget {
    pub fn label(&self) -> String {
        match self {
            Self::SuperIo(n) => format!("Board header pwm{n}"),
            Self::RyujinPump => "Ryujin pump".into(),
            Self::RyujinInternalFan => "Ryujin VRM fan".into(),
            Self::RyujinExternalFans => "Ryujin radiator fans".into(),
            Self::NvidiaFans => "GPU fans".into(),
            Self::LianLiChannel(c) => format!("Lian Li channel {c}"),
            Self::CoolerControl { channel, .. } => format!("CoolerControl {channel}"),
        }
    }
}

/// How a fan is controlled inside a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FanMode {
    /// Leave to firmware / driver default.
    Auto,
    /// Fixed duty percent.
    Fixed(f64),
    /// Software curve evaluated by OmaAsus (or handed to CoolerControl).
    Curve(FanCurve),
    /// Hardware Smart Fan IV curve written into the Super I/O (nct6775 only).
    HardwareCurve(FanCurve),
}

/// A fan output with the mode assigned to it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FanAssignment {
    pub target: FanTarget,
    pub mode: FanMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CoolingSettings {
    pub fans: Vec<FanAssignment>,
    /// Zero-RPM allowed for GPU fans (NVIDIA default policy).
    pub gpu_zero_rpm: bool,
}

impl CoolingSettings {
    pub fn set(&mut self, target: FanTarget, mode: FanMode) {
        match self.fans.iter_mut().find(|f| f.target == target) {
            Some(f) => f.mode = mode,
            None => self.fans.push(FanAssignment { target, mode }),
        }
    }
    pub fn get(&self, target: &FanTarget) -> Option<&FanMode> {
        self.fans.iter().find(|f| &f.target == target).map(|f| &f.mode)
    }
    pub fn remove(&mut self, target: &FanTarget) {
        self.fans.retain(|f| &f.target != target);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CpuSettings {
    pub control: Option<CpuControlState>,
    /// Pin the game to physical cores of CCD0 (Zen 4 gaming trick) via cgroup/cpuset.
    pub prefer_ccd0: bool,
    /// power-profiles-daemon profile to activate.
    pub ppd_profile: Option<String>,
    /// `/sys/firmware/acpi/platform_profile` value (laptops).
    pub platform_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GpuSettings {
    pub nvidia: Option<NvidiaControl>,
    /// amdgpu `power_dpm_force_performance_level`.
    pub amd_perf_level: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LightingMode {
    Off,
    Static(Rgb),
    Breathing(Rgb),
    Rainbow,
    /// Colour follows a temperature source between two colours.
    Thermal { source: TempSource, cool: Rgb, hot: Rgb, min_c: f64, max_c: f64 },
    /// Direct per-LED colours (OpenRGB direct mode).
    Direct(Vec<Rgb>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LightingSettings {
    /// Keyed by OpenRGB device name (or asusd Aura zone).
    pub zones: BTreeMap<String, LightingMode>,
    pub brightness: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LcdSettings {
    /// Ryujin LCD / LiveDash content.
    pub content: LcdContent,
    pub brightness: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum LcdContent {
    #[default]
    Firmware,
    Temps,
    Clocks,
    Image(String),
    Gif(String),
    Text(String),
}

/// A full gaming profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: uuid::Uuid,
    pub name: String,
    pub icon: String,
    pub accent: Rgb,
    pub cpu: CpuSettings,
    pub gpu: GpuSettings,
    pub cooling: CoolingSettings,
    pub lighting: LightingSettings,
    pub lcd: LcdSettings,
    /// Laptop-only knobs applied through asusd, kept generic.
    pub asusd: BTreeMap<String, String>,
    /// supergfxctl mode name to request (laptops).
    pub gfx_mode: Option<String>,
    /// CoolerControl Mode to activate with this profile (when CoolerControl owns fans).
    #[serde(default)]
    pub cc_mode: Option<String>,
    pub builtin: bool,
}

impl Profile {
    pub fn new(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name: name.into(),
            icon: "bolt".into(),
            accent: Rgb::new(255, 64, 96),
            cpu: CpuSettings::default(),
            gpu: GpuSettings::default(),
            cooling: CoolingSettings::default(),
            lighting: LightingSettings { zones: BTreeMap::new(), brightness: 100 },
            lcd: LcdSettings::default(),
            asusd: BTreeMap::new(),
            gfx_mode: None,
            cc_mode: None,
            builtin: false,
        }
    }
}

/// When the automatic engine should switch to a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Trigger {
    /// A GameMode client registered.
    GameMode,
    /// A fullscreen window that looks like a game is focused.
    FullscreenGame,
    /// A window with this class (exact or glob) is focused.
    WindowClass(String),
    /// A process with this executable name is running.
    Process(String),
    /// CPU package temperature above a threshold for N seconds.
    CpuHot { above_c: f64, for_s: u32 },
    /// GPU above a temperature.
    GpuHot { above_c: f64, for_s: u32 },
    /// Time window (24h, local), e.g. quiet at night.
    Time { from: (u8, u8), to: (u8, u8) },
    /// System idle for N seconds (no input, via Hyprland idle / swayidle hook).
    Idle { for_s: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: uuid::Uuid,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub profile: uuid::Uuid,
    /// Higher wins when several rules match.
    pub priority: i32,
    /// Seconds to keep the profile after the trigger clears (debounce).
    pub hold_s: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Mode {
    /// The user picks the profile.
    #[default]
    Manual,
    /// Rules pick the profile; `default_profile` otherwise.
    Automatic,
}

/// Who drives the fans.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FanOwner {
    /// CoolerControl when its daemon is running, otherwise OmaAsus.
    #[default]
    Auto,
    OmaAsus,
    CoolerControl,
    /// Leave fans to firmware / other tools.
    None,
}

/// Everything persisted for the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub mode: Mode,
    #[serde(default)]
    pub fan_owner: FanOwner,
    pub active_profile: uuid::Uuid,
    pub default_profile: uuid::Uuid,
    pub profiles: Vec<Profile>,
    pub rules: Vec<Rule>,
    pub coolercontrol: CoolerControlAuth,
    pub overlay: OverlaySettings,
    pub telemetry_hz: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CoolerControlAuth {
    pub enabled: bool,
    pub url: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlaySettings {
    pub anchor: String,
    pub width: u32,
    pub height: u32,
    pub margin: u32,
    pub opacity: f32,
    pub hud_enabled: bool,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self { anchor: "right".into(), width: 560, height: 1040, margin: 16, opacity: 0.97, hud_enabled: true }
    }
}

impl Config {
    pub fn with_builtin_profiles() -> Self {
        let mut silent = Profile::new("Silent");
        silent.icon = "moon".into();
        silent.accent = Rgb::new(96, 140, 255);
        silent.builtin = true;
        silent.cpu.control = Some(CpuControlState { governor: "powersave".into(), epp: Some("balance_power".into()), boost: Some(false), smt: None, scaling_min_khz: 0, scaling_max_khz: 0 });
        silent.cpu.ppd_profile = Some("power-saver".into());
        silent.gpu.nvidia = Some(NvidiaControl { power_limit_w: Some(300), ..Default::default() });
        silent.cooling.set(FanTarget::RyujinPump, FanMode::Curve(FanCurve::pump()));
        silent.cooling.set(FanTarget::RyujinExternalFans, FanMode::Curve(FanCurve::coolant()));
        silent.cooling.set(FanTarget::LianLiChannel(1), FanMode::Curve(FanCurve::silent()));
        silent.cooling.gpu_zero_rpm = true;

        let mut balanced = Profile::new("Balanced");
        balanced.icon = "scale".into();
        balanced.accent = Rgb::new(64, 224, 180);
        balanced.builtin = true;
        balanced.cpu.control = Some(CpuControlState { governor: "powersave".into(), epp: Some("balance_performance".into()), boost: Some(true), smt: None, scaling_min_khz: 0, scaling_max_khz: 0 });
        balanced.cpu.ppd_profile = Some("balanced".into());
        balanced.gpu.nvidia = Some(NvidiaControl { reset_power_limit: true, unlock_clocks: true, ..Default::default() });
        balanced.cooling.set(FanTarget::RyujinPump, FanMode::Curve(FanCurve::pump()));
        balanced.cooling.set(FanTarget::RyujinExternalFans, FanMode::Curve(FanCurve::coolant()));
        balanced.cooling.set(FanTarget::LianLiChannel(1), FanMode::Curve(FanCurve::balanced()));
        balanced.cooling.gpu_zero_rpm = true;

        let mut gaming = Profile::new("Gaming");
        gaming.icon = "gamepad".into();
        gaming.accent = Rgb::new(255, 64, 96);
        gaming.builtin = true;
        gaming.cpu.control = Some(CpuControlState { governor: "performance".into(), epp: Some("performance".into()), boost: Some(true), smt: None, scaling_min_khz: 0, scaling_max_khz: 0 });
        gaming.cpu.ppd_profile = Some("performance".into());
        gaming.gpu.nvidia = Some(NvidiaControl { reset_power_limit: true, unlock_clocks: true, persistence: Some(true), ..Default::default() });
        gaming.cooling.set(FanTarget::RyujinPump, FanMode::Fixed(100.0));
        gaming.cooling.set(FanTarget::RyujinExternalFans, FanMode::Curve(FanCurve::coolant()));
        gaming.cooling.set(FanTarget::LianLiChannel(1), FanMode::Curve(FanCurve::performance()));
        gaming.cooling.gpu_zero_rpm = false;

        let mut turbo = Profile::new("Turbo");
        turbo.icon = "flame".into();
        turbo.accent = Rgb::new(255, 160, 32);
        turbo.builtin = true;
        turbo.cpu.control = gaming.cpu.control.clone();
        turbo.cpu.ppd_profile = Some("performance".into());
        turbo.gpu.nvidia = Some(NvidiaControl { power_limit_w: Some(600), persistence: Some(true), ..Default::default() });
        turbo.cooling.set(FanTarget::RyujinPump, FanMode::Fixed(100.0));
        turbo.cooling.set(FanTarget::RyujinExternalFans, FanMode::Fixed(100.0));
        turbo.cooling.set(FanTarget::LianLiChannel(1), FanMode::Curve(FanCurve::performance()));
        turbo.cooling.set(FanTarget::NvidiaFans, FanMode::Curve(FanCurve { source: TempSource::Gpu, ..FanCurve::performance() }));
        turbo.cooling.gpu_zero_rpm = false;

        let balanced_id = balanced.id;
        let gaming_id = gaming.id;
        let rules = vec![
            Rule { id: uuid::Uuid::new_v4(), name: "GameMode active".into(), enabled: true, trigger: Trigger::GameMode, profile: gaming_id, priority: 100, hold_s: 20 },
            Rule { id: uuid::Uuid::new_v4(), name: "Fullscreen game".into(), enabled: true, trigger: Trigger::FullscreenGame, profile: gaming_id, priority: 50, hold_s: 20 },
        ];
        Self {
            mode: Mode::Manual,
            fan_owner: FanOwner::Auto,
            active_profile: balanced_id,
            default_profile: balanced_id,
            profiles: vec![silent, balanced, gaming, turbo],
            rules,
            coolercontrol: CoolerControlAuth { enabled: false, url: "http://localhost:11987".into(), password: None },
            overlay: OverlaySettings::default(),
            telemetry_hz: 2,
        }
    }

    pub fn profile(&self, id: uuid::Uuid) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }
}

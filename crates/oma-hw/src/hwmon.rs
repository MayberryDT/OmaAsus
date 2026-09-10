//! Enumeration of Linux hwmon devices: temperatures, fans, PWM outputs,
//! voltages and power sensors, with full description of the PWM automatic
//! curve capabilities exposed by drivers such as `nct6775` and `rog_ryujin`.

use crate::sysfs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A temperature sensor input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TempSensor {
    /// `tempN` index.
    pub index: u32,
    /// Driver label or a generated `tempN` name.
    pub label: String,
    /// Path to `tempN_input`.
    pub input: PathBuf,
    /// Critical threshold in °C, if reported.
    pub crit: Option<f64>,
    /// Max threshold in °C, if reported.
    pub max: Option<f64>,
}

impl TempSensor {
    /// Current reading in °C. Drivers report `-60`/`-40` style sentinels for
    /// unconnected headers (ASUS EC `T_Sensor`, `Water_In`), which are mapped to `None`.
    pub fn read(&self) -> Option<f64> {
        let v = sysfs::read_milli(&self.input)?;
        if v <= -30.0 || v > 250.0 { None } else { Some(v) }
    }
}

/// A fan tachometer input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FanSensor {
    pub index: u32,
    pub label: String,
    pub input: PathBuf,
    /// Optional `fanN_min` alarm threshold.
    pub min: Option<u64>,
}

impl FanSensor {
    pub fn read_rpm(&self) -> Option<u64> {
        sysfs::read_u64(&self.input)
    }
}

/// Meaning of the `pwmN_enable` attribute as documented for `nct6775`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u8)]
pub enum PwmEnable {
    /// Fan control disabled; output is full speed.
    Off = 0,
    /// Manual: `pwmN` value is applied directly.
    Manual = 1,
    /// Thermal cruise: target temperature mode.
    ThermalCruise = 2,
    /// Fan speed cruise: target RPM mode.
    SpeedCruise = 3,
    /// "Smart Fan III" (older chips).
    SmartFan3 = 4,
    /// "Smart Fan IV": the 5-point auto curve in `pwmN_auto_pointM_*`.
    SmartFan4 = 5,
    /// Any other driver specific value.
    Other = 255,
}

impl From<u64> for PwmEnable {
    fn from(v: u64) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::Manual,
            2 => Self::ThermalCruise,
            3 => Self::SpeedCruise,
            4 => Self::SmartFan3,
            5 => Self::SmartFan4,
            _ => Self::Other,
        }
    }
}

impl PwmEnable {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Full speed",
            Self::Manual => "Manual",
            Self::ThermalCruise => "Thermal cruise",
            Self::SpeedCruise => "Speed cruise",
            Self::SmartFan3 => "Smart Fan III",
            Self::SmartFan4 => "Smart Fan IV (auto curve)",
            Self::Other => "Driver specific",
        }
    }
}

/// One point of a hardware auto-curve (temperature °C -> pwm 0..=255).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub temp_c: f64,
    pub pwm: u8,
}

/// Serde default for fields older captures don't have.
fn yes() -> bool {
    true
}

/// Unit of `pwmN_auto_pointM_temp`. The hwmon ABI says millidegrees; some
/// drivers use plain °C (see [`crate::knowledge::curve_temp_unit`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TempUnit {
    #[default]
    Milli,
    Celsius,
}

/// Auto-curve capability of a PWM output as advertised by the driver.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoCurve {
    /// Number of `pwmN_auto_pointM_*` pairs (5 for nct6775, 8 for asus_custom_fan_curve).
    pub points: u32,
    #[serde(default)]
    pub temp_unit: TempUnit,
    /// Whether `pwmN_temp_sel` exists (choose the driving temperature).
    pub has_temp_sel: bool,
    /// Whether `pwmN_step_up_time` / `pwmN_step_down_time` exist.
    pub has_step_times: bool,
    /// Whether `pwmN_floor` / `pwmN_start` / `pwmN_stop_time` exist.
    pub has_floor_start: bool,
}

/// A PWM output channel and its live state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PwmChannel {
    pub index: u32,
    /// Path to `pwmN`.
    pub path: PathBuf,
    /// Whether `pwmN`, the duty value, exists. Firmware-curve outputs such as
    /// `asus_custom_fan_curve` have only a mode and curve points.
    #[serde(default = "yes")]
    pub has_duty: bool,
    pub has_enable: bool,
    pub has_mode: bool,
    pub auto_curve: Option<AutoCurve>,
}

/// Live snapshot of a PWM channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PwmState {
    pub value: u8,
    pub enable: Option<PwmEnable>,
    /// `true` = PWM, `false` = DC voltage control (nct6775 `pwmN_mode`).
    pub pwm_mode: Option<bool>,
    pub temp_sel: Option<u32>,
    pub curve: Vec<CurvePoint>,
    pub step_up_ms: Option<u64>,
    pub step_down_ms: Option<u64>,
    pub floor: Option<u8>,
    pub start: Option<u8>,
}

impl PwmChannel {
    fn attr(&self, suffix: &str) -> PathBuf {
        let mut s = self.path.as_os_str().to_owned();
        s.push(suffix);
        PathBuf::from(s)
    }

    pub fn enable_path(&self) -> PathBuf {
        self.attr("_enable")
    }

    pub fn read(&self) -> PwmState {
        let curve = self
            .auto_curve
            .as_ref()
            .map(|ac| {
                (1..=ac.points)
                    .filter_map(|p| {
                        let temp = self.attr(&format!("_auto_point{p}_temp"));
                        let t = match ac.temp_unit {
                            TempUnit::Milli => sysfs::read_milli(temp)?,
                            TempUnit::Celsius => sysfs::read_u64(temp)? as f64,
                        };
                        let pwm = sysfs::read_u64(self.attr(&format!("_auto_point{p}_pwm")))? as u8;
                        Some(CurvePoint { temp_c: t, pwm })
                    })
                    .collect()
            })
            .unwrap_or_default();
        PwmState {
            value: sysfs::read_u64(&self.path).unwrap_or(0).min(255) as u8,
            enable: sysfs::read_u64(self.attr("_enable")).map(PwmEnable::from),
            pwm_mode: sysfs::read_u64(self.attr("_mode")).map(|m| m == 1),
            temp_sel: sysfs::read_u64(self.attr("_temp_sel")).map(|v| v as u32),
            curve,
            step_up_ms: sysfs::read_u64(self.attr("_step_up_time")),
            step_down_ms: sysfs::read_u64(self.attr("_step_down_time")),
            floor: sysfs::read_u64(self.attr("_floor")).map(|v| v.min(255) as u8),
            start: sysfs::read_u64(self.attr("_start")).map(|v| v.min(255) as u8),
        }
    }
}

/// A voltage input in millivolts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoltageSensor {
    pub index: u32,
    pub label: String,
    pub input: PathBuf,
}

impl VoltageSensor {
    pub fn read_mv(&self) -> Option<i64> {
        sysfs::read_i64(&self.input)
    }
}

/// A power sensor in microwatts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PowerSensor {
    pub index: u32,
    pub label: String,
    pub input: PathBuf,
}

impl PowerSensor {
    pub fn read_w(&self) -> Option<f64> {
        sysfs::read_u64(&self.input).map(|uw| uw as f64 / 1e6)
    }
}

/// A single hwmon device such as `nct6799`, `k10temp`, `asusec`, `rog_ryujin`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HwmonDevice {
    /// Driver name from the `name` attribute.
    pub name: String,
    /// `/sys/class/hwmon/hwmonN`.
    pub path: PathBuf,
    /// Canonical device path (stable across reboots).
    pub device_path: PathBuf,
    pub temps: Vec<TempSensor>,
    pub fans: Vec<FanSensor>,
    pub pwms: Vec<PwmChannel>,
    pub voltages: Vec<VoltageSensor>,
    pub powers: Vec<PowerSensor>,
}

impl HwmonDevice {
    /// Human-friendly label for a well-known driver.
    pub fn friendly_name(&self) -> &str {
        match self.name.as_str() {
            "nct6799" | "nct6798" | "nct6797" | "nct6796" | "nct6795" | "nct6793" | "nct6792" | "nct6791" | "nct6779" | "nct6776" | "nct6775" => "Motherboard Super I/O",
            "asusec" => "ASUS Embedded Controller",
            "asus" | "asus_wmi_sensors" => "ASUS WMI Sensors",
            "rog_ryujin" => "ROG Ryujin AIO",
            "k10temp" => "CPU (k10temp)",
            "zenpower" => "CPU (zenpower)",
            "coretemp" => "CPU (coretemp)",
            "amdgpu" => "AMD GPU",
            "nvme" => "NVMe SSD",
            "spd5118" => "DDR5 DIMM",
            "drivetemp" => "SATA drive",
            "iwlwifi_1" | "iwlwifi" => "Wi-Fi",
            "acpitz" => "ACPI thermal zone",
            _ => self.name.as_str(),
        }
    }

    pub fn is_super_io(&self) -> bool {
        self.name.starts_with("nct6") || self.name.starts_with("it87") || self.name.starts_with("f71")
    }

    fn probe(dir: &Path) -> Option<Self> {
        let name = sysfs::read_string(dir.join("name"))?;
        let entries = sysfs::list_dir(dir);
        let names: Vec<String> = entries
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();

        let indices = |prefix: &str, suffix: &str| -> Vec<u32> {
            let mut v: Vec<u32> = names
                .iter()
                .filter_map(|n| n.strip_prefix(prefix)?.strip_suffix(suffix)?.parse().ok())
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };

        let temps = indices("temp", "_input")
            .into_iter()
            .map(|i| TempSensor {
                index: i,
                label: sysfs::read_string(dir.join(format!("temp{i}_label"))).unwrap_or_else(|| format!("temp{i}")),
                input: dir.join(format!("temp{i}_input")),
                crit: sysfs::read_milli(dir.join(format!("temp{i}_crit"))),
                max: sysfs::read_milli(dir.join(format!("temp{i}_max"))),
            })
            .collect();

        let fans = indices("fan", "_input")
            .into_iter()
            .map(|i| FanSensor {
                index: i,
                label: sysfs::read_string(dir.join(format!("fan{i}_label"))).unwrap_or_else(|| format!("fan{i}")),
                input: dir.join(format!("fan{i}_input")),
                min: sysfs::read_u64(dir.join(format!("fan{i}_min"))),
            })
            .collect();

        // Any `pwmN…` attribute names a channel; `pwmN` itself (the duty) may be
        // absent on firmware-curve outputs, which still have a mode and points.
        let pwms = names
            .iter()
            .filter_map(|n| {
                let rest = n.strip_prefix("pwm")?;
                let digits = &rest[..rest.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(rest.len())];
                digits.parse::<u32>().ok()
            })
            .collect::<std::collections::BTreeSet<u32>>()
            .into_iter()
            .filter_map(|i| {
                let base = format!("pwm{i}");
                let points = (1..=32u32).take_while(|p| names.contains(&format!("{base}_auto_point{p}_pwm"))).count() as u32;
                let auto_curve = (points > 0).then(|| AutoCurve {
                    points,
                    temp_unit: crate::knowledge::curve_temp_unit(&name),
                    has_temp_sel: names.contains(&format!("{base}_temp_sel")),
                    has_step_times: names.contains(&format!("{base}_step_up_time")),
                    has_floor_start: names.contains(&format!("{base}_floor")),
                });
                let has_duty = names.contains(&base);
                let has_enable = names.contains(&format!("{base}_enable"));
                (has_duty || has_enable || auto_curve.is_some()).then(|| PwmChannel {
                    index: i,
                    path: dir.join(&base),
                    has_duty,
                    has_enable,
                    has_mode: names.contains(&format!("{base}_mode")),
                    auto_curve,
                })
            })
            .collect();

        let voltages = indices("in", "_input")
            .into_iter()
            .map(|i| VoltageSensor {
                index: i,
                label: sysfs::read_string(dir.join(format!("in{i}_label"))).unwrap_or_else(|| format!("in{i}")),
                input: dir.join(format!("in{i}_input")),
            })
            .collect();

        let powers = indices("power", "_input")
            .into_iter()
            .map(|i| PowerSensor {
                index: i,
                label: sysfs::read_string(dir.join(format!("power{i}_label"))).unwrap_or_else(|| format!("power{i}")),
                input: dir.join(format!("power{i}_input")),
            })
            .collect();

        Some(Self {
            name,
            path: dir.to_owned(),
            device_path: sysfs::canonical(dir.join("device")),
            temps,
            fans,
            pwms,
            voltages,
            powers,
        })
    }
}

/// Enumerate every hwmon device on the system.
pub fn enumerate() -> Vec<HwmonDevice> {
    sysfs::hwmon_dirs().iter().filter_map(|d| HwmonDevice::probe(d)).collect()
}

/// Find a device by driver name.
pub fn find<'a>(devices: &'a [HwmonDevice], name: &str) -> Option<&'a HwmonDevice> {
    devices.iter().find(|d| d.name == name)
}

/// The nct6775 family maps `pwmN_temp_sel` to temperature *sources*, whose
/// numbering matches the `tempN` inputs of the same device for the first
/// entries. This helper resolves a selector to a label when possible.
pub fn temp_sel_label(dev: &HwmonDevice, sel: u32) -> String {
    dev.temps
        .iter()
        .find(|t| t.index == sel)
        .map(|t| t.label.clone())
        .unwrap_or_else(|| format!("source {sel}"))
}

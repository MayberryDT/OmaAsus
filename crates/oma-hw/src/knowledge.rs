//! Hardware knowledge: facts detection can't read from the machine, keyed by
//! what it did detect (driver names, DMI identity, USB IDs). This is the one
//! place specific hardware is named; everything else derives from detection.
//!
//! Identity facts match `board_name` exactly or by model-code prefix, or the
//! model family; the most specific match wins, and the user's `quirks.toml`
//! wins over all of them. Every fact says where it comes from: verified on
//! named hardware, or taken from a reference (see `research/`) and not yet
//! verified.

use crate::detect::DmiInfo;
use crate::hwmon::TempUnit;
use crate::lianli::HubKind;
use crate::model::{CurveInput, Release, SensorRole};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Who stands behind a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Source {
    /// Checked on this hardware.
    Verified(&'static str),
    /// From a reference (see `research/`), not yet verified on hardware.
    Reference(&'static str),
    /// Set in the user's `quirks.toml`.
    User,
}

/// The machine as knowledge sees it: DMI identity, corrected by the user if asked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub vendor: String,
    pub product: String,
    pub family: String,
    pub board: String,
    pub bios: String,
}

impl Identity {
    pub fn from_dmi(d: &DmiInfo, o: &Overrides) -> Self {
        Self {
            vendor: d.sys_vendor.clone(),
            product: d.product_name.clone(),
            family: o.family.clone().unwrap_or_else(|| d.product_family.clone()),
            board: o.board.clone().unwrap_or_else(|| d.board_name.clone()),
            bios: d.bios_version.clone(),
        }
    }
}

/// How an identity fact applies to a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// `board_name` is exactly this, e.g. "GA403WR".
    Board(&'static str),
    /// `board_name` starts with this model code, e.g. "GA403".
    BoardPrefix(&'static str),
    /// `product_family` contains these whole words, e.g. "Zephyrus".
    Family(&'static str),
}

impl Match {
    /// How specific the match is (higher wins), or `None` when it doesn't apply.
    fn rank(self, id: &Identity) -> Option<usize> {
        let board = id.board.to_ascii_uppercase();
        match self {
            Match::Board(b) => (board == b.to_ascii_uppercase()).then_some(3000),
            Match::BoardPrefix(p) => board.starts_with(&p.to_ascii_uppercase()).then_some(2000 + p.len()),
            Match::Family(f) => {
                let family = format!(" {} ", id.family.to_ascii_uppercase());
                (!id.family.is_empty() && family.contains(&format!(" {} ", f.to_ascii_uppercase()))).then_some(1000)
            }
        }
    }
}

struct Fact<T: 'static> {
    when: Match,
    value: T,
    source: Source,
}

/// The most specific fact that applies.
fn best<T>(facts: &'static [Fact<T>], id: &Identity) -> Option<&'static Fact<T>> {
    facts.iter().filter_map(|f| f.when.rank(id).map(|r| (r, f))).max_by_key(|(r, _)| *r).map(|(_, f)| f)
}

/// The user's corrections, from `quirks.toml`. They win over every table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    /// Match knowledge as if the board were this (misreported DMI, testing).
    pub board: Option<String>,
    /// Match knowledge as if the model family were this.
    pub family: Option<String>,
    /// Maximum speed per fan name ("CPU", "GPU", "MID"), in rpm.
    pub fan_max_rpm: BTreeMap<String, u32>,
    /// Minimum duty per fan output id, in percent.
    pub min_duty: BTreeMap<String, f64>,
    /// Fan, sensor, GPU or lighting ids to leave out of the model.
    pub hide: Vec<String>,
}

impl Overrides {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

// ---- identity facts --------------------------------------------------------

/// Maximum speed per fan (asusd fan names), for showing speed as a share of it.
static FAN_MAX_RPM: &[Fact<&[(&str, u32)]>] = &[Fact { when: Match::BoardPrefix("GA403"), value: &[("CPU", 6800), ("GPU", 6800), ("MID", 8000)], source: Source::Reference("G-Helper FanSensorControl.cs") }];

/// Maximum speed of a named fan ("CPU", "GPU", "MID"), if known.
pub fn fan_max_rpm(id: &Identity, fan: &str, o: &Overrides) -> Option<(u32, Source)> {
    if let Some(v) = o.fan_max_rpm.iter().find(|(k, _)| k.eq_ignore_ascii_case(fan)).map(|(_, v)| *v) {
        return Some((v, Source::User));
    }
    let f = best(FAN_MAX_RPM, id)?;
    f.value.iter().find(|(name, _)| name.eq_ignore_ascii_case(fan)).map(|(_, rpm)| (*rpm, f.source))
}

/// NVIDIA power ceiling to assume when the driver can't report one.
static DGPU_MAX_POWER_W: &[Fact<u32>] = &[Fact { when: Match::BoardPrefix("GA403"), value: 90, source: Source::Reference("G-Helper NvidiaSmi.cs") }];

pub fn dgpu_max_power_w(id: &Identity) -> Option<(u32, Source)> {
    best(DGPU_MAX_POWER_W, id).map(|f| (f.value, f.source))
}

/// LEDs on the Slash bar.
static SLASH_LEDS: &[Fact<u8>] = &[Fact { when: Match::BoardPrefix("GA403"), value: 7, source: Source::Reference("G-Helper SlashDevice.cs") }];

pub fn slash_leds(id: &Identity) -> Option<(u8, Source)> {
    best(SLASH_LEDS, id).map(|f| (f.value, f.source))
}

/// Silk-screen names of Super I/O fan headers, by pwm index.
static FAN_HEADERS: &[Fact<&[(u32, &str)]>] = &[Fact {
    when: Match::BoardPrefix("ROG CROSSHAIR X670E"),
    value: &[(1, "CPU_FAN"), (2, "CPU_OPT"), (3, "CHA_FAN1"), (4, "CHA_FAN2"), (5, "CHA_FAN3"), (6, "AIO_PUMP"), (7, "W_PUMP+")],
    source: Source::Reference("OmaAsus 0.1, written on a ROG Crosshair X670E Extreme"),
}];

pub fn fan_header(id: &Identity, index: u32) -> Option<(&'static str, Source)> {
    let f = best(FAN_HEADERS, id)?;
    f.value.iter().find(|(i, _)| *i == index).map(|(_, name)| (*name, f.source))
}

// ---- driver and device facts -------------------------------------------------

/// Unit of a driver's `pwmN_auto_pointM_temp` attributes.
pub fn curve_temp_unit(driver: &str) -> TempUnit {
    match driver {
        // Verified on a ROG Zephyrus G14 GA403WR (kernel 7.2): plain °C, e.g. 47, 69 … 255.
        "asus_custom_fan_curve" => TempUnit::Celsius,
        _ => TempUnit::Milli,
    }
}

/// asusd's name for the fan behind a firmware-curve output (its `CurveData.fan`).
/// Verified for CPU and GPU on a GA403WR; MID from `research/asusctl.md`.
pub fn firmware_fan_name(driver: &str, index: u32) -> Option<&'static str> {
    match (driver, index) {
        ("asus_custom_fan_curve", 1) => Some("CPU"),
        ("asus_custom_fan_curve", 2) => Some("GPU"),
        ("asus_custom_fan_curve", 3) => Some("MID"),
        _ => None,
    }
}

/// The hwmon device whose `fanN_input` reads the fan a firmware curve's `pwmN`
/// drives (verified on a GA403WR: `asus` fan1/fan2 are cpu_fan/gpu_fan).
pub fn curve_tach_driver(driver: &str) -> Option<&'static str> {
    match driver {
        "asus_custom_fan_curve" => Some("asus"),
        _ => None,
    }
}

/// `pwmN_enable` values that switch a firmware curve on and off.
pub fn curve_enable_values(driver: &str) -> Option<(&'static str, &'static str)> {
    match driver {
        // 1 = the custom curve, 2 = the firmware's own (research/asusctl.md).
        "asus_custom_fan_curve" => Some(("1", "2")),
        _ => None,
    }
}

/// Naming and safety limits for a specific PWM output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputQuirk {
    pub id: &'static str,
    pub label: &'static str,
    pub min_duty: f64,
    pub release: Release,
    /// What a new software curve for it should follow.
    pub curve_input: CurveInput,
    pub source: Source,
}

pub fn output_quirk(driver: &str, index: u32) -> Option<OutputQuirk> {
    const RYUJIN: Source = Source::Verified("ROG Ryujin II 360 (OmaAsus 0.1)");
    match (driver, index) {
        // The pump has no curve of its own once driven: never below 60 %, and
        // released to a safe fixed duty rather than left where it was.
        ("rog_ryujin", 1) => Some(OutputQuirk { id: "ryujin:pump", label: "AIO pump", min_duty: 60.0, release: Release::SafeFixed(65.0), curve_input: CurveInput::Coolant, source: RYUJIN }),
        ("rog_ryujin", 2) => Some(OutputQuirk { id: "ryujin:block-fan", label: "Pump-block fan", min_duty: 0.0, release: Release::SafeFixed(40.0), curve_input: CurveInput::Cpu, source: RYUJIN }),
        ("rog_ryujin", 3) => Some(OutputQuirk { id: "ryujin:radiator", label: "Radiator fans", min_duty: 0.0, release: Release::SafeFixed(40.0), curve_input: CurveInput::Coolant, source: RYUJIN }),
        _ => None,
    }
}

/// Fan channels on a Lian Li UNI FAN hub (the protocol in `lianli.rs` addresses 4 groups).
pub fn lianli_channels(kind: &HubKind) -> u8 {
    match kind {
        HubKind::Sl | HubKind::SlInfinity | HubKind::SlV2 | HubKind::AlV2 | HubKind::Al => 4,
    }
}

/// What a hwmon reading measures.
pub fn sensor_role(driver: &str, label: &str, integrated_gpu: bool) -> SensorRole {
    use SensorRole::*;
    match driver {
        "k10temp" | "zenpower" => match label {
            "Tctl" | "Tdie" => CpuTemp,
            l if l.starts_with("Tccd") => CpuCore,
            _ => Other,
        },
        "coretemp" => {
            if label.starts_with("Package") {
                CpuTemp
            } else {
                CpuCore
            }
        }
        "amdgpu" => match label {
            "edge" => {
                if integrated_gpu {
                    IgpuTemp
                } else {
                    DgpuTemp
                }
            }
            "junction" | "mem" => GpuHotspot,
            // On an APU the SoC's power is the package's.
            "PPT" | "slowPPT" | "fastPPT" => {
                if integrated_gpu {
                    PackagePower
                } else {
                    GpuPower
                }
            }
            _ => Other,
        },
        "nvme" | "drivetemp" => Storage,
        "spd5118" | "jc42" => Memory,
        "rog_ryujin" => Coolant,
        "asusec" => match label {
            "VRM" => Vrm,
            "Motherboard" | "Chipset" => Board,
            "Water_In" | "Water_Out" => Coolant,
            _ => Other,
        },
        n if n.starts_with("iwlwifi") || n.starts_with("mt79") || n.starts_with("ath1") => Wireless,
        _ => Other,
    }
}

/// What a tachometer reads, where that isn't simply a fan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TachRole {
    Fan,
    Pump,
    /// Shown only while spinning: a header that is often empty.
    WhenSpinning,
    /// Firmware's own copy of a fan another driver reports by name.
    Duplicate,
}

pub fn tach_role(driver: &str, label: &str) -> TachRole {
    match (driver, label) {
        ("rog_ryujin", l) if l.starts_with("Pump") => TachRole::Pump,
        ("rog_ryujin", l) if l.starts_with("Controller fan") => TachRole::WhenSpinning,
        // The ACPI fan object mirrors a fan the platform driver names
        // (GA403WR: acpi_fan fan1 tracks asus cpu_fan, verified).
        ("acpi_fan", _) => TachRole::Duplicate,
        _ => TachRole::Fan,
    }
}

/// Temperature inputs that read nothing useful: unconnected Super I/O inputs,
/// and EC copies of what the CPU's own sensor reports.
pub fn sensor_hidden(driver: &str, label: &str) -> bool {
    match driver {
        n if n.starts_with("nct6") => ["PCH", "AUXTIN", "PECI", "TSI", "CPUTIN", "SYSTIN"].iter().any(|p| label.starts_with(p)),
        "asusec" => matches!(label, "CPU" | "CPU Package" | "T_Sensor"),
        _ => false,
    }
}

/// The energy-performance preferences the cpufreq EPP interface defines
/// (amd-pstate-epp and intel_pstate take the same strings; kernel docs).
pub const EPP_STANDARD: [&str; 5] = ["default", "performance", "balance_performance", "balance_power", "power"];

/// Under the `performance` governor both EPP drivers hold EPP at
/// `performance` and reject other values; amd-pstate-epp then also lists only
/// `performance` as available (kernel amd-pstate and intel_pstate docs;
/// seen on the GA403WR).
pub fn epp_pinned_by_governor(governor: &str) -> bool {
    governor == "performance"
}

/// Firmware attributes that switch or power off GPUs. supergfxd owns them when it runs.
pub fn is_gpu_switch(attr: &str) -> bool {
    matches!(attr, "dgpu_disable" | "gpu_mux_mode" | "egpu_enable")
}

/// What an asusd Aura device lights, from its `DeviceType` (asusd's positional enum).
pub fn aura_device_label(device_type: Option<u32>) -> &'static str {
    match device_type {
        Some(0..=2) => "Keyboard",
        Some(4) => "Ally lighting",
        _ => "Lighting",
    }
}

/// asusd lists Aura modes by enum value, and `LedMode`/`LedModeData` read and
/// write that value, but `AllModeData` numbers them by position, and there is
/// no mode 9: Pulse is 10 everywhere except `AllModeData`, where it is 9
/// (verified on a GA403WR, asusd 6.4: writing 9 fails "incorrect type");
/// Comet and Flash follow the same shift (asusctl notes).
pub fn aura_mode_data_key(listed: u32) -> u32 {
    if listed >= 10 { listed - 1 } else { listed }
}

/// Name of an Aura mode, by asusd's listed number.
pub fn aura_mode_name(listed: u32) -> &'static str {
    crate::asusd::aura_mode::label(aura_mode_data_key(listed))
}

/// Slash bar animations: asusd `Mode` value and name (G-Helper SlashDevice.cs
/// and asusctl; the GA403WR reports 16 = Bounce, verified).
pub const SLASH_MODES: &[(u32, &str)] = &[
    (0x06, "Static"),
    (0x10, "Bounce"),
    (0x12, "Slash"),
    (0x13, "Loading"),
    (0x1D, "Bit stream"),
    (0x1A, "Transmission"),
    (0x19, "Flow"),
    (0x25, "Flux"),
    (0x24, "Phantom"),
    (0x26, "Spectrum"),
    (0x32, "Hazard"),
    (0x33, "Interfacing"),
    (0x34, "Ramp"),
    (0x42, "Game over"),
    (0x43, "Start"),
    (0x44, "Buzzer"),
];

/// The Slash animation that shows a steady bar.
pub const SLASH_STATIC: u32 = 0x06;

pub fn slash_mode_name(mode: u32) -> Option<&'static str> {
    SLASH_MODES.iter().find(|(m, _)| *m == mode).map(|(_, n)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(board: &str, family: &str) -> Identity {
        Identity { board: board.into(), family: family.into(), ..Default::default() }
    }

    #[test]
    fn curve_units_follow_the_driver() {
        assert_eq!(curve_temp_unit("asus_custom_fan_curve"), TempUnit::Celsius);
        assert_eq!(curve_temp_unit("nct6799"), TempUnit::Milli);
    }

    #[test]
    fn the_most_specific_match_wins() {
        let g14 = id("GA403WR", "ROG Zephyrus G14");
        assert_eq!(Match::Board("GA403WR").rank(&g14), Some(3000));
        assert!(Match::BoardPrefix("GA403").rank(&g14) > Match::BoardPrefix("GA4").rank(&g14));
        assert!(Match::BoardPrefix("GA4").rank(&g14) > Match::Family("Zephyrus").rank(&g14));
        assert_eq!(Match::Family("Zephyrus G14").rank(&g14), Some(1000));
        // Whole words only: "G1" is not a word of the family, and prefixes don't match mid-code.
        assert_eq!(Match::Family("G1").rank(&g14), None);
        assert_eq!(Match::BoardPrefix("403").rank(&g14), None);
        assert_eq!(Match::Family("Zephyrus").rank(&id("GA403WR", "")), None);
    }

    #[test]
    fn facts_come_from_tables_unless_the_user_overrides() {
        let o = Overrides::default();
        assert_eq!(fan_max_rpm(&id("GA403WR", ""), "CPU", &o).map(|f| f.0), Some(6800));
        assert_eq!(fan_max_rpm(&id("GA402X", ""), "CPU", &o), None);
        let o = Overrides::parse("[fan_max_rpm]\ncpu = 7100\n").expect("valid");
        assert_eq!(fan_max_rpm(&id("GA403WR", ""), "CPU", &o), Some((7100, Source::User)));
        assert!(Overrides::parse("fan_max = 3").is_err(), "typos are reported, not ignored");
    }

    #[test]
    fn overrides_can_correct_the_identity() {
        let dmi = DmiInfo { sys_vendor: String::new(), product_name: String::new(), product_family: "Wrong".into(), board_vendor: String::new(), board_name: "GA403WR".into(), bios_version: String::new(), bios_date: String::new() };
        let o = Overrides { family: Some("ROG Zephyrus G14".into()), ..Default::default() };
        let identity = Identity::from_dmi(&dmi, &o);
        assert_eq!((identity.board.as_str(), identity.family.as_str()), ("GA403WR", "ROG Zephyrus G14"));
    }

    #[test]
    fn sensor_roles_depend_on_the_gpu_kind() {
        assert_eq!(sensor_role("amdgpu", "PPT", true), SensorRole::PackagePower);
        assert_eq!(sensor_role("amdgpu", "PPT", false), SensorRole::GpuPower);
        assert_eq!(sensor_role("amdgpu", "edge", true), SensorRole::IgpuTemp);
        assert_eq!(sensor_role("k10temp", "Tctl", false), SensorRole::CpuTemp);
        assert_eq!(sensor_role("mt7925_phy0", "temp1", false), SensorRole::Wireless);
    }

    #[test]
    fn aura_mode_keys_skip_the_missing_nine() {
        assert_eq!(aura_mode_data_key(3), 3);
        assert_eq!(aura_mode_data_key(10), 9);
    }

    #[test]
    fn tachs_and_inputs_that_need_care() {
        assert_eq!(tach_role("rog_ryujin", "Pump speed"), TachRole::Pump);
        assert_eq!(tach_role("rog_ryujin", "Controller fan 1 speed"), TachRole::WhenSpinning);
        assert_eq!(tach_role("acpi_fan", "fan1"), TachRole::Duplicate);
        assert_eq!(tach_role("asus", "cpu_fan"), TachRole::Fan);
        assert!(sensor_hidden("nct6799", "AUXTIN3") && sensor_hidden("asusec", "T_Sensor"));
        assert!(!sensor_hidden("asusec", "VRM") && !sensor_hidden("k10temp", "Tctl"));
    }
}

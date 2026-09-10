//! asusd client (`xyz.ljones.Asusd`, asusctl ≥ 6.1) — laptops and ROG Ally.
//!
//! The daemon never starts on desktop boards, so everything here is optional.
//! Enum wire encodings follow `research/asusctl.md` §2.2 exactly.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Type};

pub const BUS_NAME: &str = "xyz.ljones.Asusd";
pub const LEGACY_BUS_NAME: &str = "org.asuslinux.Daemon";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[repr(u32)]
pub enum PlatformProfile {
    Balanced = 0,
    Performance = 1,
    Quiet = 2,
    LowPower = 3,
    Custom = 4,
}

impl PlatformProfile {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Performance,
            2 => Self::Quiet,
            3 => Self::LowPower,
            4 => Self::Custom,
            _ => Self::Balanced,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::Performance => "Performance",
            Self::Quiet => "Quiet",
            Self::LowPower => "Low power",
            Self::Custom => "Custom",
        }
    }
    pub fn sysfs_name(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::Quiet => "quiet",
            Self::LowPower => "low-power",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[repr(u32)]
pub enum CpuEpp {
    Default = 0,
    Performance = 1,
    BalancePerformance = 2,
    BalancePower = 3,
    Power = 4,
}

/// `xyz.ljones.Platform` at `/xyz/ljones`.
#[zbus::proxy(interface = "xyz.ljones.Platform", default_service = "xyz.ljones.Asusd", default_path = "/xyz/ljones")]
pub trait Platform {
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
    fn supported_properties(&self) -> zbus::Result<Vec<String>>;
    fn next_platform_profile(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn platform_profile_choices(&self) -> zbus::Result<Vec<u32>>;
    #[zbus(property)]
    fn platform_profile(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_platform_profile(&self, p: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn platform_profile_on_ac(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_platform_profile_on_ac(&self, p: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn platform_profile_on_battery(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_platform_profile_on_battery(&self, p: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn change_platform_profile_on_ac(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_change_platform_profile_on_ac(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn change_platform_profile_on_battery(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_change_platform_profile_on_battery(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn platform_profile_linked_epp(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_platform_profile_linked_epp(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn profile_balanced_epp(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_profile_balanced_epp(&self, v: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn profile_performance_epp(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_profile_performance_epp(&self, v: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn profile_quiet_epp(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_profile_quiet_epp(&self, v: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn charge_control_end_threshold(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn set_charge_control_end_threshold(&self, v: u8) -> zbus::Result<()>;
    fn one_shot_full_charge(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn enable_ppt_group(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_enable_ppt_group(&self, v: bool) -> zbus::Result<()>;
    /// asusd 6.4+: stop nvidia-powerd while on battery.
    #[zbus(property)]
    fn disable_nvidia_powerd_on_battery(&self) -> zbus::Result<bool>;
}

/// `xyz.ljones.AsusArmoury` at `/xyz/ljones/asus_armoury/<attr>`.
#[zbus::proxy(interface = "xyz.ljones.AsusArmoury", default_service = "xyz.ljones.Asusd")]
pub trait AsusArmoury {
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn available_attrs(&self) -> zbus::Result<Vec<String>>;
    #[zbus(property)]
    fn current_value(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn set_current_value(&self, v: i32) -> zbus::Result<()>;
    #[zbus(property)]
    fn default_value(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn min_value(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn max_value(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn scalar_increment(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn possible_values(&self) -> zbus::Result<Vec<i32>>;
    #[zbus(property)]
    fn queued_gpu_value(&self) -> zbus::Result<i32>;
    fn restore_default(&self) -> zbus::Result<()>;
}

/// One fan's firmware curve: 8 points of (°C, pwm 0..=255).
///
/// The arrays are fixed-size on the wire, `(s(yyyyyyyy)(yyyyyyyy)b)`, as
/// asusd 6.4 reports in its introspection data; `Vec<u8>` (`ay`) would not
/// decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CurveData {
    pub fan: String,
    pub pwm: [u8; 8],
    pub temp: [u8; 8],
    pub enabled: bool,
}

/// `xyz.ljones.FanCurves` at `/xyz/ljones`.
#[zbus::proxy(interface = "xyz.ljones.FanCurves", default_service = "xyz.ljones.Asusd", default_path = "/xyz/ljones")]
pub trait FanCurves {
    fn fan_curve_data(&self, profile: u32) -> zbus::Result<Vec<CurveData>>;
    fn set_fan_curve(&self, profile: u32, curve: CurveData) -> zbus::Result<()>;
    fn set_fan_curves_enabled(&self, profile: u32, enabled: bool) -> zbus::Result<()>;
    fn set_profile_fan_curve_enabled(&self, profile: u32, fan: &str, enabled: bool) -> zbus::Result<()>;
    fn set_curves_to_defaults(&self, profile: u32) -> zbus::Result<()>;
}

/// `(uu(yyy)(yyy)ss)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, zbus::zvariant::Value, zbus::zvariant::OwnedValue)]
pub struct AuraEffect {
    pub mode: u32,
    pub zone: u32,
    pub colour1: (u8, u8, u8),
    pub colour2: (u8, u8, u8),
    pub speed: String,
    pub direction: String,
}

pub mod aura_mode {
    pub const STATIC: u32 = 0;
    pub const BREATHE: u32 = 1;
    pub const RAINBOW_CYCLE: u32 = 2;
    pub const RAINBOW_WAVE: u32 = 3;
    pub const STAR: u32 = 4;
    pub const RAIN: u32 = 5;
    pub const HIGHLIGHT: u32 = 6;
    pub const LASER: u32 = 7;
    pub const RIPPLE: u32 = 8;
    /// Wire index (positional serde), *not* the Rust discriminant 10.
    pub const PULSE: u32 = 9;
    pub const COMET: u32 = 10;
    pub const FLASH: u32 = 11;
    pub fn label(m: u32) -> &'static str {
        match m {
            STATIC => "Static",
            BREATHE => "Breathe",
            RAINBOW_CYCLE => "Rainbow cycle",
            RAINBOW_WAVE => "Rainbow wave",
            STAR => "Star",
            RAIN => "Rain",
            HIGHLIGHT => "Highlight",
            LASER => "Laser",
            RIPPLE => "Ripple",
            PULSE => "Pulse",
            COMET => "Comet",
            FLASH => "Flash",
            _ => "Unknown",
        }
    }
}

/// `xyz.ljones.Aura` at `/xyz/ljones/aura/<dev>`.
#[zbus::proxy(interface = "xyz.ljones.Aura", default_service = "xyz.ljones.Asusd")]
pub trait Aura {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn brightness(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_brightness(&self, v: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn supported_basic_modes(&self) -> zbus::Result<Vec<u32>>;
    #[zbus(property)]
    fn supported_basic_zones(&self) -> zbus::Result<Vec<u32>>;
    #[zbus(property)]
    fn led_mode(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_led_mode(&self, v: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn led_mode_data(&self) -> zbus::Result<AuraEffect>;
    #[zbus(property)]
    fn set_led_mode_data(&self, v: AuraEffect) -> zbus::Result<()>;
    #[zbus(property)]
    fn supported_brightness(&self) -> zbus::Result<Vec<u32>>;
    #[zbus(property)]
    fn supported_power_zones(&self) -> zbus::Result<Vec<u32>>;
    #[zbus(property)]
    fn led_power(&self) -> zbus::Result<LedPower>;
    #[zbus(property)]
    fn set_led_power(&self, v: LedPower) -> zbus::Result<()>;
    fn all_mode_data(&self) -> zbus::Result<HashMap<u32, AuraEffect>>;
    fn direct_addressing_raw(&self, packets: Vec<Vec<u8>>) -> zbus::Result<()>;
}

/// When one lighting zone is lit, `(ubbbb)`: zone, boot, awake, sleep, shutdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, zbus::zvariant::Value, zbus::zvariant::OwnedValue)]
pub struct PowerZoneState {
    pub zone: u32,
    pub boot: bool,
    pub awake: bool,
    pub sleep: bool,
    pub shutdown: bool,
}

/// `(a(ubbbb))`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, zbus::zvariant::Value, zbus::zvariant::OwnedValue)]
pub struct LedPower {
    pub states: Vec<PowerZoneState>,
}

/// `xyz.ljones.Slash`: the LED bar on the lid of recent Zephyrus models.
#[zbus::proxy(interface = "xyz.ljones.Slash", default_service = "xyz.ljones.Asusd")]
pub trait Slash {
    #[zbus(property)]
    fn enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_enabled(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn brightness(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn set_brightness(&self, v: u8) -> zbus::Result<()>;
    #[zbus(property)]
    fn interval(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn set_interval(&self, v: u8) -> zbus::Result<()>;
    /// `y` on asusd 6.4 (6.3.8's getter and setter disagreed on the type).
    #[zbus(property)]
    fn mode(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn set_mode(&self, v: u8) -> zbus::Result<()>;
    #[zbus(property)]
    fn show_on_boot(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_show_on_boot(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn show_on_sleep(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_show_on_sleep(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn show_on_shutdown(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_show_on_shutdown(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn show_on_battery(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_show_on_battery(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn show_battery_warning(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_show_battery_warning(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn show_on_lid_closed(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_show_on_lid_closed(&self, v: bool) -> zbus::Result<()>;
    /// `(byyu)`: enabled, brightness, interval, mode.
    fn device_state(&self) -> zbus::Result<(bool, u8, u8, u32)>;
}

#[zbus::proxy(interface = "org.freedesktop.DBus.ObjectManager", default_service = "xyz.ljones.Asusd", default_path = "/")]
trait ObjectManager {
    fn get_managed_objects(&self) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

/// Discovered asusd objects.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AsusdObjects {
    pub version: String,
    pub has_platform: bool,
    pub has_fan_curves: bool,
    /// sysfs attribute names, e.g. `ppt_pl1_spl`, `gpu_mux_mode`.
    pub armoury_attrs: Vec<String>,
    pub aura_paths: Vec<String>,
    pub anime: bool,
    pub slash: bool,
    pub backlight: bool,
    /// Object paths of the hot-plugged Slash / AniMe devices.
    #[serde(default)]
    pub slash_path: Option<String>,
    #[serde(default)]
    pub anime_path: Option<String>,
}

/// Describe an armoury attribute with its limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArmouryAttr {
    pub attr: String,
    pub name: String,
    pub current: i32,
    pub default: i32,
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub possible: Vec<i32>,
    pub queued: i32,
}

pub async fn discover(conn: &zbus::Connection) -> zbus::Result<AsusdObjects> {
    let om = ObjectManagerProxy::new(conn).await?;
    let objs = om.get_managed_objects().await?;
    let mut out = AsusdObjects::default();
    for (path, ifaces) in objs {
        let p = path.as_str().to_string();
        for iface in ifaces.keys() {
            match iface.as_str() {
                "xyz.ljones.Platform" => out.has_platform = true,
                "xyz.ljones.FanCurves" => out.has_fan_curves = true,
                "xyz.ljones.AsusArmoury" => {
                    if let Some(attr) = p.rsplit('/').next() {
                        out.armoury_attrs.push(attr.to_string());
                    }
                }
                "xyz.ljones.Aura" => out.aura_paths.push(p.clone()),
                "xyz.ljones.Anime" => {
                    out.anime = true;
                    out.anime_path = Some(p.clone());
                }
                "xyz.ljones.Slash" => {
                    out.slash = true;
                    out.slash_path = Some(p.clone());
                }
                "xyz.ljones.Backlight" => out.backlight = true,
                _ => {}
            }
        }
    }
    if out.has_platform {
        out.version = PlatformProxy::new(conn).await?.version().await.unwrap_or_default();
    }
    out.armoury_attrs.sort();
    Ok(out)
}

pub async fn armoury_attr(conn: &zbus::Connection, attr: &str) -> zbus::Result<ArmouryAttr> {
    let path = format!("/xyz/ljones/asus_armoury/{attr}");
    let p = AsusArmouryProxy::builder(conn).path(path)?.build().await?;
    Ok(ArmouryAttr {
        attr: attr.to_string(),
        name: p.name().await.unwrap_or_default(),
        current: p.current_value().await.unwrap_or(-1),
        default: p.default_value().await.unwrap_or(-1),
        min: p.min_value().await.unwrap_or(-1),
        max: p.max_value().await.unwrap_or(-1),
        step: p.scalar_increment().await.unwrap_or(-1),
        possible: p.possible_values().await.unwrap_or_default(),
        queued: p.queued_gpu_value().await.unwrap_or(-1),
    })
}

pub async fn set_armoury_attr(conn: &zbus::Connection, attr: &str, value: i32) -> zbus::Result<()> {
    let path = format!("/xyz/ljones/asus_armoury/{attr}");
    AsusArmouryProxy::builder(conn).path(path)?.build().await?.set_current_value(value).await
}

/// Human labels for well-known armoury attributes.
pub fn attr_label(attr: &str) -> &str {
    match attr {
        "ppt_pl1_spl" => "CPU sustained power (PL1 / SPL)",
        "ppt_pl2_sppt" => "CPU slow boost power (PL2 / SPPT)",
        "ppt_pl3_fppt" | "ppt_fppt" => "CPU fast boost power (FPPT)",
        "ppt_apu_sppt" => "APU slow boost power",
        "ppt_platform_sppt" => "Platform slow boost power",
        "nv_dynamic_boost" => "NVIDIA Dynamic Boost",
        "nv_temp_target" => "NVIDIA temperature target",
        "nv_tgp" => "NVIDIA TGP",
        "nv_base_tgp" => "NVIDIA base TGP",
        "gpu_mux_mode" => "GPU MUX (0 = dGPU only)",
        "dgpu_disable" => "Disable dGPU",
        "egpu_enable" => "Enable eGPU",
        "panel_od" | "panel_overdrive" => "Panel overdrive",
        "mini_led_mode" => "Mini-LED mode",
        "boot_sound" => "Boot sound",
        "mcu_powersave" => "MCU power save",
        "charge_mode" => "Charge mode",
        "panel_hd_mode" => "Panel HD mode",
        "screen_auto_brightness" => "Auto brightness",
        "cores_performance" => "Performance cores",
        "cores_efficiency" => "Efficiency cores",
        "apu_mem" => "APU memory",
        _ => attr,
    }
}

//! System detection: DMI identity, ASUS platform class, present daemons,
//! and the full capability map that the UI adapts to.

use crate::{amdgpu, cpu, hwmon, sysfs};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Platform {
    /// ROG / TUF / Zephyrus laptop or ROG Ally (asus-nb-wmi present).
    AsusLaptop,
    /// ASUS desktop board (ROG Crosshair / Strix / TUF Gaming / ProArt).
    AsusDesktop,
    /// Non-ASUS system; generic sensors only.
    Generic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DmiInfo {
    pub sys_vendor: String,
    pub product_name: String,
    /// Model family, e.g. "ROG Zephyrus G14" (empty on boards that don't set it).
    #[serde(default)]
    pub product_family: String,
    pub board_vendor: String,
    pub board_name: String,
    pub bios_version: String,
    pub bios_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Daemons {
    pub asusd: bool,
    pub supergfxd: bool,
    pub coolercontrold: bool,
    pub power_profiles_daemon: bool,
    pub gamemode: bool,
    pub openrgb_server: bool,
    pub oma_helper: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HidDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub path: String,
    pub interface: i32,
    pub usage_page: u16,
    /// Whether the current user can open it.
    pub accessible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemInventory {
    pub platform: Platform,
    pub dmi: DmiInfo,
    pub kernel: String,
    pub cpu: cpu::CpuInfo,
    pub hwmon: Vec<hwmon::HwmonDevice>,
    pub amd_gpus: Vec<amdgpu::AmdGpu>,
    pub nvidia_count: u32,
    pub hid: Vec<HidDevice>,
    pub daemons: Daemons,
    pub asus_armoury_attrs: Vec<String>,
    pub platform_profile_choices: Vec<String>,
    pub features: Features,
}

/// Coarse-grained feature flags consumed by the UI to show/hide pages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Features {
    pub cpu_epp: bool,
    pub cpu_boost: bool,
    pub cpu_smt: bool,
    pub super_io_fans: bool,
    pub ryujin_aio: bool,
    pub livedash_oled: bool,
    pub aura_usb: bool,
    pub lianli_uni_hub: bool,
    pub nvidia: bool,
    pub amd_gpu: bool,
    pub asus_ec_sensors: bool,
    pub platform_profile: bool,
    pub asusd: bool,
    pub supergfxd: bool,
    pub coolercontrol: bool,
    pub openrgb: bool,
    pub liquidctl: bool,
}

fn dmi(attr: &str) -> String {
    sysfs::read_string(Path::new("/sys/class/dmi/id").join(attr)).unwrap_or_default()
}

pub fn dmi_info() -> DmiInfo {
    DmiInfo {
        sys_vendor: dmi("sys_vendor"),
        product_name: dmi("product_name"),
        product_family: dmi("product_family"),
        board_vendor: dmi("board_vendor"),
        board_name: dmi("board_name"),
        bios_version: dmi("bios_version"),
        bios_date: dmi("bios_date"),
    }
}

pub fn platform(d: &DmiInfo) -> Platform {
    let asus = d.sys_vendor.to_ascii_uppercase().contains("ASUS") || d.board_vendor.to_ascii_uppercase().contains("ASUS");
    if !asus {
        return Platform::Generic;
    }
    let laptop = Path::new("/sys/devices/platform/asus-nb-wmi").exists()
        || sysfs::read_string("/sys/class/dmi/id/chassis_type").map(|t| matches!(t.as_str(), "9" | "10" | "14" | "31" | "32")).unwrap_or(false);
    if laptop { Platform::AsusLaptop } else { Platform::AsusDesktop }
}

fn unit_active(unit: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tcp_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), std::time::Duration::from_millis(150)).is_ok()
}

fn user_bus_has(name: &str) -> bool {
    std::process::Command::new("busctl")
        .args(["--user", "--no-pager", "list"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

pub fn daemons() -> Daemons {
    Daemons {
        asusd: unit_active("asusd"),
        supergfxd: unit_active("supergfxd"),
        coolercontrold: unit_active("coolercontrold") || tcp_open(11987),
        power_profiles_daemon: unit_active("power-profiles-daemon") || unit_active("tuned-ppd"),
        gamemode: user_bus_has("com.feralinteractive.GameMode"),
        openrgb_server: tcp_open(6742),
        oma_helper: unit_active("oma-helper"),
    }
}

pub fn hid_devices() -> Vec<HidDevice> {
    let Ok(api) = hidapi::HidApi::new() else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    api.device_list()
        .filter(|d| d.vendor_id() == 0x0b05 || d.vendor_id() == 0x0cf2)
        .filter_map(|d| {
            let path = d.path().to_string_lossy().into_owned();
            if !seen.insert((path.clone(), d.usage_page())) {
                return None;
            }
            Some(HidDevice {
                vendor_id: d.vendor_id(),
                product_id: d.product_id(),
                product: d.product_string().unwrap_or("").to_owned(),
                path: path.clone(),
                interface: d.interface_number(),
                usage_page: d.usage_page(),
                accessible: std::fs::OpenOptions::new().read(true).write(true).open(&path).is_ok(),
            })
        })
        .collect()
}

pub fn armoury_attributes() -> Vec<String> {
    sysfs::list_dir("/sys/class/firmware-attributes/asus-armoury/attributes")
        .into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect()
}

pub fn platform_profile_choices() -> Vec<String> {
    sysfs::read_string("/sys/firmware/acpi/platform_profile_choices")
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Known ASUS / partner USB product IDs.
pub mod pid {
    pub const ASUS: u16 = 0x0b05;
    pub const ENE: u16 = 0x0cf2;
    pub const AURA_LED_CONTROLLER: u16 = 0x18f3;
    pub const AURA_LED_CONTROLLER_ALT: u16 = 0x19af;
    pub const RYUJIN_II_360: u16 = 0x1a60;
    pub const RYUJIN_II_360_ALT: u16 = 0x1988;
    pub const RYUJIN_III: u16 = 0x1a45;
    /// Motherboard LiveDash OLED / AniMe Matrix controller (ROG Extreme boards).
    pub const LIVEDASH_OLED: u16 = 0x1a21;
    pub const LIANLI_UNI_SL: u16 = 0xa100;
    pub const LIANLI_UNI_AL: u16 = 0x7750;
    pub const LIANLI_UNI_SL_INF: u16 = 0xa101;
    pub const LIANLI_UNI_SL_V2: u16 = 0xa102;
}

pub fn inventory() -> SystemInventory {
    let dmi = dmi_info();
    let platform = platform(&dmi);
    let cpu = cpu::cpu_info();
    let hwmon = hwmon::enumerate();
    let amd_gpus = amdgpu::AmdGpu::enumerate();
    // NVML wakes a runtime-suspended GPU, so only count when it's already awake.
    let nvidia_count = if crate::nvidia::awake() { crate::nvidia::NvidiaGpu::count() } else { 0 };
    let hid = hid_devices();
    let daemons = daemons();
    let asus_armoury_attrs = armoury_attributes();
    let platform_profile_choices = platform_profile_choices();
    let has_hid = |pid_: u16| hid.iter().any(|h| h.product_id == pid_);
    let features = Features {
        cpu_epp: cpu.has_epp,
        cpu_boost: cpu.has_boost,
        cpu_smt: cpu.has_smt_control,
        super_io_fans: hwmon.iter().any(|d| d.is_super_io() && d.pwms.iter().any(|p| p.has_duty)),
        ryujin_aio: hwmon.iter().any(|d| d.name == "rog_ryujin") || has_hid(pid::RYUJIN_II_360) || has_hid(pid::RYUJIN_III),
        livedash_oled: has_hid(pid::LIVEDASH_OLED),
        aura_usb: has_hid(pid::AURA_LED_CONTROLLER) || has_hid(pid::AURA_LED_CONTROLLER_ALT),
        lianli_uni_hub: hid.iter().any(|h| h.vendor_id == pid::ENE && (0xa100..=0xa102).contains(&h.product_id) || h.product_id == pid::LIANLI_UNI_AL),
        nvidia: nvidia_count > 0,
        amd_gpu: !amd_gpus.is_empty(),
        asus_ec_sensors: hwmon.iter().any(|d| d.name == "asusec"),
        platform_profile: !platform_profile_choices.is_empty(),
        asusd: daemons.asusd,
        supergfxd: daemons.supergfxd,
        coolercontrol: daemons.coolercontrold,
        openrgb: which("openrgb"),
        liquidctl: which("liquidctl"),
    };
    SystemInventory {
        platform,
        dmi,
        kernel: sysfs::read_string("/proc/sys/kernel/osrelease").unwrap_or_default(),
        cpu,
        hwmon,
        amd_gpus,
        nvidia_count,
        hid,
        daemons,
        asus_armoury_attrs,
        platform_profile_choices,
        features,
    }
}

pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

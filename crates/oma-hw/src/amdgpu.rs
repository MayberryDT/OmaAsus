//! AMD GPU (amdgpu) sysfs telemetry and DPM control; covers both the Raphael
//! iGPU on AM5 boards and discrete Radeon cards.

use crate::sysfs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AmdGpu {
    /// `/sys/bus/pci/devices/0000:12:00.0`.
    pub device_path: PathBuf,
    pub pci_slot: String,
    pub hwmon_path: Option<PathBuf>,
    pub name: String,
    pub is_integrated: bool,
    pub has_od: bool,
    pub dpm_levels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AmdGpuTelemetry {
    pub busy_percent: Option<u64>,
    pub mem_busy_percent: Option<u64>,
    pub sclk_mhz: Option<f64>,
    pub mclk_mhz: Option<f64>,
    pub edge_c: Option<f64>,
    pub junction_c: Option<f64>,
    pub power_w: Option<f64>,
    pub vddgfx_mv: Option<i64>,
    pub perf_level: Option<String>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
}

pub const PERF_LEVELS: &[&str] = &["auto", "low", "high", "manual", "profile_standard", "profile_min_sclk", "profile_min_mclk", "profile_peak"];

impl AmdGpu {
    pub fn enumerate() -> Vec<Self> {
        let mut out = Vec::new();
        for card in sysfs::list_dir("/sys/class/drm") {
            let fname = card.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !fname.starts_with("card") || fname.contains('-') {
                continue;
            }
            let dev = card.join("device");
            let driver = sysfs::canonical(dev.join("driver"));
            if driver.file_name().map(|n| n != "amdgpu").unwrap_or(true) {
                continue;
            }
            let device_path = sysfs::canonical(&dev);
            let pci_slot = device_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let hwmon_path = sysfs::list_dir(dev.join("hwmon")).into_iter().next();
            let vram_total = sysfs::read_u64(dev.join("mem_info_vram_total")).unwrap_or(0);
            let is_integrated = vram_total <= 1024 * 1024 * 1024 || sysfs::read_string(dev.join("boot_vga")).is_none();
            let name = std::process::Command::new("lspci")
                .args(["-s", &pci_slot])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.split(':').skip(2).collect::<Vec<_>>().join(":").trim().to_string().into())
                .filter(|s: &String| !s.is_empty())
                .unwrap_or_else(|| "AMD GPU".into());
            out.push(Self {
                device_path,
                pci_slot,
                hwmon_path,
                name,
                is_integrated,
                has_od: sysfs::exists(dev.join("pp_od_clk_voltage")),
                dpm_levels: PERF_LEVELS.iter().map(|s| s.to_string()).collect(),
            });
        }
        out
    }

    pub fn perf_level_path(&self) -> PathBuf {
        self.device_path.join("power_dpm_force_performance_level")
    }

    fn hw(&self, attr: &str) -> Option<PathBuf> {
        self.hwmon_path.as_ref().map(|h| h.join(attr))
    }

    fn labelled_temp(&self, label: &str) -> Option<f64> {
        let h = self.hwmon_path.as_ref()?;
        for i in 1..=4 {
            if sysfs::read_string(h.join(format!("temp{i}_label"))).as_deref() == Some(label) {
                return sysfs::read_milli(h.join(format!("temp{i}_input")));
            }
        }
        None
    }

    pub fn telemetry(&self) -> AmdGpuTelemetry {
        let d = &self.device_path;
        AmdGpuTelemetry {
            busy_percent: sysfs::read_u64(d.join("gpu_busy_percent")),
            mem_busy_percent: sysfs::read_u64(d.join("mem_busy_percent")),
            sclk_mhz: self.hw("freq1_input").and_then(sysfs::read_u64).map(|hz| hz as f64 / 1e6),
            mclk_mhz: self.hw("freq2_input").and_then(sysfs::read_u64).map(|hz| hz as f64 / 1e6),
            edge_c: self.labelled_temp("edge"),
            junction_c: self.labelled_temp("junction"),
            power_w: self.hw("power1_average").or_else(|| self.hw("power1_input")).and_then(sysfs::read_u64).map(|uw| uw as f64 / 1e6),
            vddgfx_mv: self.hw("in0_input").and_then(sysfs::read_i64),
            perf_level: sysfs::read_string(self.perf_level_path()),
            vram_used_mb: sysfs::read_u64(d.join("mem_info_vram_used")).map(|b| b / 1024 / 1024),
            vram_total_mb: sysfs::read_u64(d.join("mem_info_vram_total")).map(|b| b / 1024 / 1024),
        }
    }
}

pub fn is_amdgpu_device(p: &Path) -> bool {
    sysfs::canonical(p.join("driver")).file_name().map(|n| n == "amdgpu").unwrap_or(false)
}

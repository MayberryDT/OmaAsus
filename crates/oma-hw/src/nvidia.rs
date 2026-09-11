//! NVIDIA GPU telemetry and control through NVML (`libnvidia-ml`).
//!
//! Reads never need privileges. Writes (power limit, locked clocks, clock
//! offsets, fan speeds) need root and are performed by the OmaAsus helper,
//! which uses the same code paths.

use nvml_wrapper::enum_wrappers::device::{Clock, PerformanceState, TemperatureSensor};
use nvml_wrapper::enums::device::FanControlPolicy;
use nvml_wrapper::{Device, Nvml};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NvidiaInfo {
    pub index: u32,
    pub name: String,
    pub uuid: String,
    pub pci_bus_id: String,
    pub driver_version: String,
    pub vram_total_mb: u64,
    pub num_fans: u32,
    pub power_min_mw: u32,
    pub power_max_mw: u32,
    pub power_default_mw: u32,
    pub max_graphics_mhz: u32,
    pub max_memory_mhz: u32,
    /// Distinct supported graphics clocks (MHz), descending.
    pub supported_graphics_mhz: Vec<u32>,
    pub supported_memory_mhz: Vec<u32>,
    pub pcie_max_gen: u32,
    pub pcie_max_width: u32,
    /// Whether NVML reports clock-offset ranges (driver 555+ style API).
    pub supports_clock_offsets: bool,
    pub gpc_offset_range_mhz: Option<(i32, i32)>,
    pub mem_offset_range_mhz: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NvidiaTelemetry {
    pub temp_c: Option<u32>,
    pub temp_mem_c: Option<u32>,
    pub power_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub graphics_mhz: Option<u32>,
    pub sm_mhz: Option<u32>,
    pub memory_mhz: Option<u32>,
    pub video_mhz: Option<u32>,
    pub util_gpu: Option<u32>,
    pub util_mem: Option<u32>,
    pub util_encoder: Option<u32>,
    pub util_decoder: Option<u32>,
    pub vram_used_mb: Option<u64>,
    pub fan_percent: Vec<u32>,
    pub fan_rpm: Vec<u32>,
    pub pstate: Option<String>,
    pub throttle_reasons: Vec<String>,
    pub pcie_gen: Option<u32>,
    pub pcie_width: Option<u32>,
    pub pcie_tx_kbps: Option<u32>,
    pub pcie_rx_kbps: Option<u32>,
    pub locked_graphics_mhz: Option<(u32, u32)>,
    pub gpc_offset_mhz: Option<i32>,
    pub mem_offset_mhz: Option<i32>,
    pub fan_policy_manual: Vec<bool>,
    pub persistence: Option<bool>,
    pub process_count: Option<u32>,
}

/// Desired control state for an NVIDIA GPU. `None` fields are left untouched;
/// the explicit `reset_*` / `unlock_*` / `fan_auto` flags restore defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NvidiaControl {
    /// Power limit in watts.
    pub power_limit_w: Option<u32>,
    #[serde(default)]
    pub reset_power_limit: bool,
    /// Locked graphics clock range (min, max) MHz.
    pub locked_graphics_mhz: Option<(u32, u32)>,
    pub locked_memory_mhz: Option<(u32, u32)>,
    #[serde(default)]
    pub unlock_clocks: bool,
    pub gpc_offset_mhz: Option<i32>,
    pub mem_offset_mhz: Option<i32>,
    /// Manual fan duty per fan (percent).
    pub fan_percent: Option<Vec<u32>>,
    #[serde(default)]
    pub fan_auto: bool,
    pub persistence: Option<bool>,
}

impl NvidiaControl {
    /// A control that only touches the fans.
    pub fn fans_only(percent: Option<u32>) -> Self {
        Self { fan_percent: percent.map(|p| vec![p]), fan_auto: percent.is_none(), ..Default::default() }
    }
    /// Everything back to driver defaults.
    pub fn reset_all() -> Self {
        Self { reset_power_limit: true, unlock_clocks: true, fan_auto: true, gpc_offset_mhz: Some(0), mem_offset_mhz: Some(0), ..Default::default() }
    }
    pub fn touches_advanced(&self) -> bool {
        self.gpc_offset_mhz.is_some_and(|o| o != 0) || self.mem_offset_mhz.is_some_and(|o| o != 0)
    }
}

pub struct NvidiaGpu {
    nvml: Nvml,
    index: u32,
}

fn pstate_label(p: PerformanceState) -> String {
    format!("{p:?}")
}

impl NvidiaGpu {
    pub fn open(index: u32) -> anyhow::Result<Self> {
        let nvml = Nvml::init()?;
        let _ = nvml.device_by_index(index)?;
        Ok(Self { nvml, index })
    }

    pub fn count() -> u32 {
        Nvml::init().and_then(|n| n.device_count()).unwrap_or(0)
    }

    fn dev(&self) -> anyhow::Result<Device<'_>> {
        Ok(self.nvml.device_by_index(self.index)?)
    }

    pub fn info(&self) -> anyhow::Result<NvidiaInfo> {
        let d = self.dev()?;
        let constraints = d.power_management_limit_constraints().ok();
        let mut supported_memory_mhz: Vec<u32> = d.supported_memory_clocks().unwrap_or_default();
        supported_memory_mhz.sort_unstable_by(|a, b| b.cmp(a));
        let mut supported_graphics_mhz: Vec<u32> = supported_memory_mhz
            .first()
            .and_then(|&m| d.supported_graphics_clocks(m).ok())
            .unwrap_or_default();
        supported_graphics_mhz.sort_unstable_by(|a, b| b.cmp(a));
        supported_graphics_mhz.dedup();
        let (gpc_range, mem_range) = clock_offset_ranges(&d);
        Ok(NvidiaInfo {
            index: self.index,
            name: d.name()?,
            uuid: d.uuid().unwrap_or_default(),
            pci_bus_id: d.pci_info().map(|p| p.bus_id).unwrap_or_default(),
            driver_version: self.nvml.sys_driver_version().unwrap_or_default(),
            vram_total_mb: d.memory_info().map(|m| m.total / 1024 / 1024).unwrap_or(0),
            num_fans: d.num_fans().unwrap_or(0),
            power_min_mw: constraints.as_ref().map(|c| c.min_limit).unwrap_or(0),
            power_max_mw: constraints.as_ref().map(|c| c.max_limit).unwrap_or(0),
            power_default_mw: d.power_management_limit_default().unwrap_or(0),
            max_graphics_mhz: d.max_clock_info(Clock::Graphics).unwrap_or(0),
            max_memory_mhz: d.max_clock_info(Clock::Memory).unwrap_or(0),
            supported_graphics_mhz,
            supported_memory_mhz,
            pcie_max_gen: d.max_pcie_link_gen().unwrap_or(0),
            pcie_max_width: d.max_pcie_link_width().unwrap_or(0),
            supports_clock_offsets: gpc_range.is_some(),
            gpc_offset_range_mhz: gpc_range,
            mem_offset_range_mhz: mem_range,
        })
    }

    pub fn telemetry(&self) -> anyhow::Result<NvidiaTelemetry> {
        let d = self.dev()?;
        let num_fans = d.num_fans().unwrap_or(0);
        let util = d.utilization_rates().ok();
        let mem = d.memory_info().ok();
        let throttle = d
            .current_throttle_reasons()
            .map(|r| {
                let mut v = Vec::new();
                use nvml_wrapper::bitmasks::device::ThrottleReasons as T;
                for (flag, name) in [
                    (T::GPU_IDLE, "Idle"),
                    (T::APPLICATIONS_CLOCKS_SETTING, "App clocks"),
                    (T::SW_POWER_CAP, "Power cap"),
                    (T::HW_SLOWDOWN, "HW slowdown"),
                    (T::SYNC_BOOST, "Sync boost"),
                    (T::SW_THERMAL_SLOWDOWN, "Thermal (SW)"),
                    (T::HW_THERMAL_SLOWDOWN, "Thermal (HW)"),
                    (T::HW_POWER_BRAKE_SLOWDOWN, "Power brake"),
                    (T::DISPLAY_CLOCK_SETTING, "Display clock"),
                ] {
                    if r.contains(flag) {
                        v.push(name.to_string());
                    }
                }
                v
            })
            .unwrap_or_default();
        let (gpc_offset_mhz, mem_offset_mhz) = current_clock_offsets(&d);
        Ok(NvidiaTelemetry {
            temp_c: d.temperature(TemperatureSensor::Gpu).ok(),
            temp_mem_c: None,
            power_w: d.power_usage().ok().map(|mw| mw as f64 / 1000.0),
            power_limit_w: d.power_management_limit().ok().map(|mw| mw as f64 / 1000.0),
            graphics_mhz: d.clock_info(Clock::Graphics).ok(),
            sm_mhz: d.clock_info(Clock::SM).ok(),
            memory_mhz: d.clock_info(Clock::Memory).ok(),
            video_mhz: d.clock_info(Clock::Video).ok(),
            util_gpu: util.as_ref().map(|u| u.gpu),
            util_mem: util.as_ref().map(|u| u.memory),
            util_encoder: d.encoder_utilization().ok().map(|u| u.utilization),
            util_decoder: d.decoder_utilization().ok().map(|u| u.utilization),
            vram_used_mb: mem.map(|m| m.used / 1024 / 1024),
            fan_percent: (0..num_fans).filter_map(|i| d.fan_speed(i).ok()).collect(),
            fan_rpm: Vec::new(),
            pstate: d.performance_state().ok().map(pstate_label),
            throttle_reasons: throttle,
            pcie_gen: d.current_pcie_link_gen().ok(),
            pcie_width: d.current_pcie_link_width().ok(),
            pcie_tx_kbps: d.pcie_throughput(nvml_wrapper::enum_wrappers::device::PcieUtilCounter::Send).ok(),
            pcie_rx_kbps: d.pcie_throughput(nvml_wrapper::enum_wrappers::device::PcieUtilCounter::Receive).ok(),
            locked_graphics_mhz: None,
            gpc_offset_mhz,
            mem_offset_mhz,
            fan_policy_manual: (0..num_fans)
                .filter_map(|i| d.fan_control_policy(i).ok())
                .map(|p| matches!(p, FanControlPolicy::Manual))
                .collect(),
            persistence: d.is_in_persistent_mode().ok(),
            process_count: d.running_graphics_processes_count().ok(),
        })
    }

    /// Apply a control state. Requires root; errors are reported per step.
    pub fn apply(&self, ctl: &NvidiaControl) -> Vec<(String, Result<(), String>)> {
        let mut out = Vec::new();
        let d = match self.dev() {
            Ok(d) => d,
            Err(e) => return vec![("open".into(), Err(e.to_string()))],
        };
        let mut d = d;
        if let Some(p) = ctl.persistence {
            out.push(("persistence".into(), d.set_persistent(p).map_err(|e| e.to_string())));
        }
        if ctl.reset_power_limit {
            let range = d.power_management_limit_constraints().ok().map(|c| (c.min_limit, c.max_limit));
            if let Some(def) = stock_limit_to_write(d.power_management_limit_default().ok(), d.power_management_limit().ok(), range) {
                out.push(("power_limit_reset".into(), d.set_power_management_limit(def).map_err(|e| e.to_string())));
            }
        } else if let Some(w) = ctl.power_limit_w {
            out.push(("power_limit".into(), d.set_power_management_limit(w * 1000).map_err(|e| e.to_string())));
        }
        if ctl.unlock_clocks {
            out.push(("reset_gpu_clocks".into(), d.reset_gpu_locked_clocks().map_err(|e| e.to_string())));
            out.push(("reset_mem_clocks".into(), d.reset_mem_locked_clocks().map_err(|e| e.to_string())));
        }
        if let Some((lo, hi)) = ctl.locked_graphics_mhz {
            out.push(("lock_gpu_clocks".into(), d.set_gpu_locked_clocks(nvml_wrapper::enums::device::GpuLockedClocksSetting::Numeric { min_clock_mhz: lo, max_clock_mhz: hi }).map_err(|e| e.to_string())));
        }
        if let Some((lo, hi)) = ctl.locked_memory_mhz {
            out.push(("lock_mem_clocks".into(), d.set_mem_locked_clocks(lo, hi).map_err(|e| e.to_string())));
        }
        if let Some(off) = ctl.gpc_offset_mhz {
            out.push(("gpc_offset".into(), set_clock_offset(&d, ClockKind::Graphics, off)));
        }
        if let Some(off) = ctl.mem_offset_mhz {
            out.push(("mem_offset".into(), set_clock_offset(&d, ClockKind::Memory, off)));
        }
        let num_fans = d.num_fans().unwrap_or(0);
        if ctl.fan_auto {
            for i in 0..num_fans {
                out.push((format!("fan{i}_auto"), d.set_default_fan_speed(i).map_err(|e| e.to_string())));
            }
        } else if let Some(v) = &ctl.fan_percent {
            for i in 0..num_fans {
                let pct = v.get(i as usize).or(v.first()).copied().unwrap_or(50).clamp(0, 100);
                out.push((format!("fan{i}"), d.set_fan_speed(i, pct).map_err(|e| e.to_string())));
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ClockKind {
    Graphics,
    Memory,
}

/// A clock offset range `(min, max)` in MHz, where the driver reports one.
type OffsetRange = Option<(i32, i32)>;

/// Clock offset support through the raw NVML symbols (`nvmlDeviceGetClockOffsets`,
/// driver 555+). Returns `(gpc_range, mem_range)`.
fn clock_offset_ranges(d: &Device<'_>) -> (OffsetRange, OffsetRange) {
    let gpc = raw::get_clock_offsets(d, ClockKind::Graphics).map(|o| (o.min, o.max));
    let mem = raw::get_clock_offsets(d, ClockKind::Memory).map(|o| (o.min, o.max));
    (gpc, mem)
}

fn current_clock_offsets(d: &Device<'_>) -> (Option<i32>, Option<i32>) {
    (
        raw::get_clock_offsets(d, ClockKind::Graphics).map(|o| o.current),
        raw::get_clock_offsets(d, ClockKind::Memory).map(|o| o.current),
    )
}

fn set_clock_offset(d: &Device<'_>, kind: ClockKind, mhz: i32) -> Result<(), String> {
    raw::set_clock_offset(d, kind, mhz)
}

/// Raw NVML calls not wrapped by `nvml-wrapper` (clock offsets, driver 555+).
///
/// `nvml-wrapper` keeps its library handle private, so we open our own
/// `NvmlLib` on the already-loaded `libnvidia-ml.so.1`; NVML initialisation
/// is reference counted so this is safe alongside the wrapper.
mod raw {
    use super::{ClockKind, Device};
    use nvml_wrapper_sys::bindings::*;
    use std::sync::OnceLock;

    #[derive(Debug, Clone, Copy)]
    pub struct Offsets {
        pub current: i32,
        pub min: i32,
        pub max: i32,
    }

    fn lib() -> Option<&'static NvmlLib> {
        static LIB: OnceLock<Option<NvmlLib>> = OnceLock::new();
        LIB.get_or_init(|| unsafe {
            let l = NvmlLib::new("libnvidia-ml.so.1").ok()?;
            if l.nvmlInit_v2() != nvmlReturn_enum_NVML_SUCCESS {
                return None;
            }
            Some(l)
        })
        .as_ref()
    }

    fn clock_type(kind: ClockKind) -> nvmlClockType_t {
        match kind {
            ClockKind::Graphics => nvmlClockType_enum_NVML_CLOCK_GRAPHICS,
            ClockKind::Memory => nvmlClockType_enum_NVML_CLOCK_MEM,
        }
    }

    fn pstate_raw(d: &Device<'_>) -> nvmlPstates_t {
        d.performance_state().map(|p| p.as_c()).unwrap_or(0)
    }

    pub fn get_clock_offsets(d: &Device<'_>, kind: ClockKind) -> Option<Offsets> {
        let l = lib()?;
        let sym = l.nvmlDeviceGetClockOffsets.as_ref().ok()?;
        let mut info = nvmlClockOffset_v1_t {
            version: nvml_clock_offset_v1(),
            type_: clock_type(kind),
            pstate: 0,
            clockOffsetMHz: 0,
            minClockOffsetMHz: 0,
            maxClockOffsetMHz: 0,
        };
        let r = unsafe { sym(d.handle(), &mut info) };
        if r != nvmlReturn_enum_NVML_SUCCESS {
            return None;
        }
        let _ = pstate_raw(d);
        Some(Offsets { current: info.clockOffsetMHz, min: info.minClockOffsetMHz, max: info.maxClockOffsetMHz })
    }

    pub fn set_clock_offset(d: &Device<'_>, kind: ClockKind, mhz: i32) -> Result<(), String> {
        let l = lib().ok_or("NVML library unavailable")?;
        let sym = l.nvmlDeviceSetClockOffsets.as_ref().map_err(|e| e.to_string())?;
        let mut info = nvmlClockOffset_v1_t {
            version: nvml_clock_offset_v1(),
            type_: clock_type(kind),
            pstate: 0,
            clockOffsetMHz: mhz,
            minClockOffsetMHz: 0,
            maxClockOffsetMHz: 0,
        };
        let r = unsafe { sym(d.handle(), &mut info) };
        if r == nvmlReturn_enum_NVML_SUCCESS { Ok(()) } else { Err(format!("nvmlDeviceSetClockOffsets failed with NVML code {r}")) }
    }

    /// `NVML_STRUCT_VERSION(ClockOffset, 1)` = sizeof(struct) | (1 << 24).
    fn nvml_clock_offset_v1() -> u32 {
        (std::mem::size_of::<nvmlClockOffset_v1_t>() as u32) | (1 << 24)
    }
}

/// Lightweight probe: is an NVIDIA GPU present and NVML loadable?
pub fn available() -> bool {
    Nvml::init().is_ok()
}

/// Runtime power state of the NVIDIA GPU, read from sysfs, which never wakes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DgpuState {
    /// Not on the bus (supergfxd Integrated mode, dGPU disabled, desktop without one).
    Absent,
    /// Runtime-suspended: NVML would wake it.
    Suspended,
    Active,
}

/// The stock limit (mW) to write when a profile names none: only where the
/// limit can be set at all (laptop GPUs refuse any; some report min == max),
/// where it isn't at stock already, and not when the current limit can't be
/// read, so an apply never fails on a write that couldn't work.
pub fn stock_limit_to_write(default_mw: Option<u32>, current_mw: Option<u32>, range_mw: Option<(u32, u32)>) -> Option<u32> {
    match (default_mw, current_mw, range_mw) {
        (Some(def), Some(cur), Some((lo, hi))) if lo < hi && cur != def => Some(def),
        _ => None,
    }
}

/// Whether runtime power management may suspend the dGPU (`power/control` is
/// `auto`, as supergfxd's udev rule sets it). Where it may not, letting go of
/// NVML so the GPU can sleep gains nothing.
pub fn runtime_pm_allowed(device: Option<&std::path::Path>) -> bool {
    device.is_some_and(|d| crate::sysfs::read_string(d.join("power/control")).as_deref() == Some("auto"))
}

/// The NVIDIA display device's PCI directory, if one is on the bus.
pub fn pci_device() -> Option<std::path::PathBuf> {
    crate::sysfs::list_dir("/sys/bus/pci/devices")
        .into_iter()
        .find(|d| crate::sysfs::read_string(d.join("vendor")).as_deref() == Some("0x10de") && crate::sysfs::read_string(d.join("class")).is_some_and(|c| c.starts_with("0x03")))
}

pub fn power_state(device: Option<&std::path::Path>) -> DgpuState {
    match device {
        Some(d) if d.exists() => match crate::sysfs::read_string(d.join("power/runtime_status")).as_deref() {
            Some("suspended" | "suspending") => DgpuState::Suspended,
            _ => DgpuState::Active,
        },
        _ => DgpuState::Absent,
    }
}

/// Whether NVML can be used without waking a sleeping GPU: an NVIDIA GPU is on
/// the bus and awake. Every NVML entry point that runs unattended checks this.
pub fn awake() -> bool {
    power_state(pci_device().as_deref()) == DgpuState::Active
}

#[cfg(test)]
mod power_tests {
    use super::*;

    #[test]
    fn power_state_reads_runtime_pm() {
        let dir = std::env::temp_dir().join(format!("omaasus-dgpu-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("power")).unwrap();
        std::fs::write(dir.join("power/runtime_status"), "suspended\n").unwrap();
        assert_eq!(power_state(Some(&dir)), DgpuState::Suspended);
        std::fs::write(dir.join("power/runtime_status"), "active\n").unwrap();
        assert_eq!(power_state(Some(&dir)), DgpuState::Active);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(power_state(Some(&dir)), DgpuState::Absent, "gone from the bus");
        assert_eq!(power_state(None), DgpuState::Absent);
    }

    #[test]
    fn stock_limit_is_written_only_where_it_can_and_must_be() {
        assert_eq!(stock_limit_to_write(Some(115_000), Some(80_000), Some((5_000, 150_000))), Some(115_000));
        assert_eq!(stock_limit_to_write(Some(115_000), Some(115_000), Some((5_000, 150_000))), None, "already stock");
        assert_eq!(stock_limit_to_write(Some(80_000), None, Some((5_000, 150_000))), None, "current limit unreadable");
        assert_eq!(stock_limit_to_write(Some(80_000), Some(95_000), Some((80_000, 80_000))), None, "no range to set");
        assert_eq!(stock_limit_to_write(Some(80_000), Some(95_000), None), None, "constraints unreadable");
    }
}


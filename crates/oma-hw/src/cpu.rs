//! CPU frequency scaling (cpufreq / amd-pstate-epp / intel_pstate), SMT,
//! per-core telemetry and package power.

use crate::sysfs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const CPU_ROOT: &str = "/sys/devices/system/cpu";

/// Static description of the processor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpuInfo {
    pub model: String,
    pub vendor: String,
    pub logical_cpus: u32,
    pub physical_cores: u32,
    pub threads_per_core: u32,
    pub scaling_driver: Option<String>,
    /// `active` / `passive` / `guided` for amd-pstate, or `None`.
    pub amd_pstate_status: Option<String>,
    pub available_governors: Vec<String>,
    pub available_epp: Vec<String>,
    /// kHz.
    pub cpuinfo_min_khz: u64,
    /// kHz.
    pub cpuinfo_max_khz: u64,
    pub has_boost: bool,
    pub has_smt_control: bool,
    pub has_epp: bool,
    /// Preferred-core ranking per logical CPU, if amd-pstate exposes it.
    pub prefcore_ranking: Vec<u32>,
    /// Per-logical-CPU max frequency in kHz (differs per CCD/core on Zen 4).
    pub per_cpu_max_khz: Vec<u64>,
}

/// Tunable state applied to all policies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CpuControlState {
    pub governor: String,
    pub epp: Option<String>,
    pub boost: Option<bool>,
    pub smt: Option<bool>,
    pub scaling_min_khz: u64,
    pub scaling_max_khz: u64,
}

/// Live telemetry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CpuTelemetry {
    /// Per-logical-CPU current frequency in MHz.
    pub freq_mhz: Vec<f64>,
    /// Per-logical-CPU utilisation 0..=100.
    pub util: Vec<f64>,
    /// Overall utilisation 0..=100.
    pub util_total: f64,
    /// Package power in watts, if readable (RAPL needs root).
    pub package_w: Option<f64>,
    /// Tctl (control temperature) °C.
    pub tctl_c: Option<f64>,
    /// Per-CCD temperatures °C.
    pub ccd_c: Vec<f64>,
    /// Highest per-core frequency in MHz.
    pub max_core_mhz: f64,
    pub avg_core_mhz: f64,
}

fn cpu_dir(cpu: u32) -> PathBuf {
    Path::new(CPU_ROOT).join(format!("cpu{cpu}"))
}

fn policy_attr(cpu: u32, attr: &str) -> PathBuf {
    cpu_dir(cpu).join("cpufreq").join(attr)
}

pub fn boost_path() -> PathBuf {
    Path::new(CPU_ROOT).join("cpufreq/boost")
}

pub fn smt_path() -> PathBuf {
    Path::new(CPU_ROOT).join("smt/control")
}

pub fn governor_path(cpu: u32) -> PathBuf {
    policy_attr(cpu, "scaling_governor")
}

pub fn epp_path(cpu: u32) -> PathBuf {
    policy_attr(cpu, "energy_performance_preference")
}

pub fn min_freq_path(cpu: u32) -> PathBuf {
    policy_attr(cpu, "scaling_min_freq")
}

pub fn max_freq_path(cpu: u32) -> PathBuf {
    policy_attr(cpu, "scaling_max_freq")
}

fn online_cpus() -> Vec<u32> {
    let s = sysfs::read_string(Path::new(CPU_ROOT).join("online")).unwrap_or_default();
    parse_cpu_list(&s)
}

/// The EPP values the CPU accepts. Under the `performance` governor,
/// amd-pstate-epp pins EPP and reports only `performance` as available, so a
/// list read then would make every other preference look unsupported for the
/// rest of the session. That forced single entry means "unknown": fall back to
/// the fixed set the EPP drivers define.
pub fn epp_choices(reported: Vec<String>, governor: Option<&str>) -> Vec<String> {
    let forced = governor.is_some_and(crate::knowledge::epp_pinned_by_governor) && reported.len() == 1 && reported[0] == "performance";
    if forced {
        crate::knowledge::EPP_STANDARD.iter().map(|s| s.to_string()).collect()
    } else {
        reported
    }
}

/// Where boost goes. Per policy when the kernel offers it (amd-pstate on
/// 6.11+): power-profiles-daemon restores boost per policy when it leaves
/// power-saver, and those writes fail with EINVAL while the global knob is 0,
/// which made every Quiet → Balanced switch error out. The global knob is only
/// used where per-policy boost is missing, or to lift a global 0 left by an
/// older version before the per-policy values can take.
fn boost_plan(per_policy: &[u32], has_global: bool, global_off: bool, on: bool) -> Vec<(PathBuf, String)> {
    let val = || if on { "1" } else { "0" }.to_string();
    let mut w = Vec::new();
    if per_policy.is_empty() {
        if has_global {
            w.push((boost_path(), val()));
        }
        return w;
    }
    if on && global_off {
        w.push((boost_path(), "1".into()));
    }
    for &c in per_policy {
        w.push((policy_attr(c, "boost"), val()));
    }
    w
}

/// Parse kernel CPU lists like `0-3,8,10-11`.
pub fn parse_cpu_list(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                out.extend(a..=b);
            }
        } else if let Ok(v) = part.parse::<u32>() {
            out.push(v);
        }
    }
    out
}

pub fn cpu_info() -> CpuInfo {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let field = |key: &str| -> Option<String> {
        cpuinfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
    };
    let model = field("model name").unwrap_or_else(|| "Unknown CPU".into());
    let vendor = field("vendor_id").unwrap_or_default();
    let cpus = online_cpus();
    let logical_cpus = cpus.len() as u32;
    let siblings: u32 = field("siblings").and_then(|s| s.parse().ok()).unwrap_or(logical_cpus);
    let cores: u32 = field("cpu cores").and_then(|s| s.parse().ok()).unwrap_or(logical_cpus);
    let threads_per_core = if cores > 0 { (siblings / cores).max(1) } else { 1 };
    let split = |p: PathBuf| -> Vec<String> {
        sysfs::read_string(p)
            .map(|s| s.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let per_cpu_max_khz = cpus.iter().map(|&c| sysfs::read_u64(policy_attr(c, "cpuinfo_max_freq")).unwrap_or(0)).collect();
    let prefcore_ranking = cpus.iter().filter_map(|&c| sysfs::read_u64(policy_attr(c, "amd_pstate_prefcore_ranking")).map(|v| v as u32)).collect();
    CpuInfo {
        model,
        vendor,
        logical_cpus,
        physical_cores: cores,
        threads_per_core,
        scaling_driver: sysfs::read_string(policy_attr(0, "scaling_driver")),
        amd_pstate_status: sysfs::read_string(Path::new(CPU_ROOT).join("amd_pstate/status")),
        available_governors: split(policy_attr(0, "scaling_available_governors")),
        available_epp: epp_choices(split(policy_attr(0, "energy_performance_available_preferences")), sysfs::read_string(policy_attr(0, "scaling_governor")).as_deref()),
        cpuinfo_min_khz: sysfs::read_u64(policy_attr(0, "cpuinfo_min_freq")).unwrap_or(0),
        cpuinfo_max_khz: sysfs::read_u64(policy_attr(0, "cpuinfo_max_freq")).unwrap_or(0),
        has_boost: sysfs::exists(boost_path()),
        has_smt_control: sysfs::exists(smt_path()),
        has_epp: sysfs::exists(epp_path(0)),
        prefcore_ranking,
        per_cpu_max_khz,
    }
}

pub fn control_state() -> CpuControlState {
    CpuControlState {
        governor: sysfs::read_string(governor_path(0)).unwrap_or_default(),
        epp: sysfs::read_string(epp_path(0)),
        boost: sysfs::read_u64(boost_path()).map(|v| v == 1),
        smt: sysfs::read_string(smt_path()).map(|s| s == "on"),
        scaling_min_khz: sysfs::read_u64(min_freq_path(0)).unwrap_or(0),
        scaling_max_khz: sysfs::read_u64(max_freq_path(0)).unwrap_or(0),
    }
}

/// Incremental `/proc/stat` sampler for per-core utilisation.
#[derive(Debug, Default)]
pub struct StatSampler {
    prev: HashMap<u32, (u64, u64)>, // (idle, total)
    prev_total: (u64, u64),
    last: Option<Instant>,
}

impl StatSampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sample and return `(per_cpu_util, total_util)`.
    pub fn sample(&mut self) -> (Vec<f64>, f64) {
        let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
        let mut per = Vec::new();
        let mut total_util = 0.0;
        for line in stat.lines().filter(|l| l.starts_with("cpu")) {
            let mut it = line.split_whitespace();
            let name = it.next().unwrap_or("");
            let vals: Vec<u64> = it.filter_map(|v| v.parse().ok()).collect();
            if vals.len() < 4 {
                continue;
            }
            let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
            let total: u64 = vals.iter().sum();
            if name == "cpu" {
                let (pi, pt) = self.prev_total;
                let dt = total.saturating_sub(pt);
                total_util = if dt > 0 { 100.0 * (1.0 - idle.saturating_sub(pi) as f64 / dt as f64) } else { 0.0 };
                self.prev_total = (idle, total);
            } else if let Ok(n) = name[3..].parse::<u32>() {
                let (pi, pt) = self.prev.get(&n).copied().unwrap_or((0, 0));
                let dt = total.saturating_sub(pt);
                let u = if dt > 0 { 100.0 * (1.0 - idle.saturating_sub(pi) as f64 / dt as f64) } else { 0.0 };
                if per.len() <= n as usize {
                    per.resize(n as usize + 1, 0.0);
                }
                per[n as usize] = u.clamp(0.0, 100.0);
                self.prev.insert(n, (idle, total));
            }
        }
        self.last = Some(Instant::now());
        (per, total_util.clamp(0.0, 100.0))
    }
}

/// RAPL energy counter sampler (`/sys/class/powercap/*-rapl:0/energy_uj`).
/// Readable only as root on most kernels; returns `None` otherwise.
#[derive(Debug, Default)]
pub struct RaplSampler {
    path: Option<PathBuf>,
    prev: Option<(u64, Instant)>,
    max_uj: u64,
}

impl RaplSampler {
    pub fn new() -> Self {
        let path = sysfs::list_dir("/sys/class/powercap")
            .into_iter()
            .find(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (n == "intel-rapl:0" || n == "amd-rapl:0") && sysfs::read_u64(p.join("energy_uj")).is_some()
            })
            .map(|p| p.join("energy_uj"));
        let max_uj = path
            .as_ref()
            .and_then(|p| sysfs::read_u64(p.with_file_name("max_energy_range_uj")))
            .unwrap_or(u64::MAX);
        Self { path, prev: None, max_uj }
    }

    pub fn available(&self) -> bool {
        self.path.is_some()
    }

    pub fn sample_w(&mut self) -> Option<f64> {
        let p = self.path.as_ref()?;
        let now = Instant::now();
        let e = sysfs::read_u64(p)?;
        let out = self.prev.map(|(pe, pt)| {
            let de = if e >= pe { e - pe } else { self.max_uj - pe + e };
            let dt = now.duration_since(pt).as_secs_f64();
            if dt > 0.0 { de as f64 / 1e6 / dt } else { 0.0 }
        });
        self.prev = Some((e, now));
        out
    }
}

/// Stateful telemetry reader.
#[derive(Debug)]
pub struct CpuMonitor {
    cpus: Vec<u32>,
    stat: StatSampler,
    rapl: RaplSampler,
    tctl: Option<PathBuf>,
    ccds: Vec<PathBuf>,
}

impl Default for CpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuMonitor {
    pub fn new() -> Self {
        let mut tctl = None;
        let mut ccds = Vec::new();
        for dev in crate::hwmon::enumerate() {
            if matches!(dev.name.as_str(), "k10temp" | "zenpower" | "coretemp") {
                for t in &dev.temps {
                    if t.label == "Tctl" || t.label == "Package id 0" {
                        tctl = Some(t.input.clone());
                    } else if t.label.starts_with("Tccd") {
                        ccds.push(t.input.clone());
                    }
                }
                if tctl.is_none() {
                    tctl = dev.temps.first().map(|t| t.input.clone());
                }
            }
        }
        Self { cpus: online_cpus(), stat: StatSampler::new(), rapl: RaplSampler::new(), tctl, ccds }
    }

    pub fn sample(&mut self) -> CpuTelemetry {
        let freq_mhz: Vec<f64> = self
            .cpus
            .iter()
            .map(|&c| sysfs::read_u64(policy_attr(c, "scaling_cur_freq")).unwrap_or(0) as f64 / 1000.0)
            .collect();
        let (util, util_total) = self.stat.sample();
        let max_core_mhz = freq_mhz.iter().cloned().fold(0.0, f64::max);
        let avg_core_mhz = if freq_mhz.is_empty() { 0.0 } else { freq_mhz.iter().sum::<f64>() / freq_mhz.len() as f64 };
        CpuTelemetry {
            freq_mhz,
            util,
            util_total,
            package_w: self.rapl.sample_w(),
            tctl_c: self.tctl.as_ref().and_then(sysfs::read_milli),
            ccd_c: self.ccds.iter().filter_map(sysfs::read_milli).collect(),
            max_core_mhz,
            avg_core_mhz,
        }
    }

    pub fn logical_cpus(&self) -> &[u32] {
        &self.cpus
    }
}

/// The set of sysfs writes needed to apply a governor/EPP/limits change.
/// Returned as (path, value) pairs so the privileged helper can apply them.
pub fn plan_writes(info: &CpuInfo, target: &CpuControlState) -> Vec<(PathBuf, String)> {
    let mut w = Vec::new();
    let cpus = online_cpus();
    for &c in &cpus {
        if !target.governor.is_empty() {
            w.push((governor_path(c), target.governor.clone()));
        }
        if let (true, Some(epp)) = (info.has_epp, &target.epp) {
            w.push((epp_path(c), epp.clone()));
        }
        if target.scaling_min_khz > 0 {
            w.push((min_freq_path(c), target.scaling_min_khz.to_string()));
        }
        if target.scaling_max_khz > 0 {
            w.push((max_freq_path(c), target.scaling_max_khz.to_string()));
        }
    }
    if let Some(b) = target.boost {
        let per_policy: Vec<u32> = cpus.iter().copied().filter(|&c| sysfs::exists(policy_attr(c, "boost"))).collect();
        let global_off = info.has_boost && sysfs::read_string(boost_path()).as_deref() == Some("0");
        w.extend(boost_plan(&per_policy, info.has_boost, global_off, b));
    }
    if let (true, Some(s)) = (info.has_smt_control, target.smt) {
        w.push((smt_path(), if s { "on" } else { "off" }.into()));
    }
    w
}

#[cfg(test)]
mod boost_tests {
    use super::*;

    #[test]
    fn forced_single_epp_under_performance_governor_means_unknown() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(epp_choices(v(&["performance"]), Some("performance")).len(), 5);
        // Any real list, or the same entry under another governor, is taken as reported.
        assert_eq!(epp_choices(v(&["performance"]), Some("powersave")), v(&["performance"]));
        let full = v(&["default", "performance", "balance_performance", "balance_power", "power", "custom"]);
        assert_eq!(epp_choices(full.clone(), Some("performance")), full);
        assert!(epp_choices(vec![], None).is_empty());
    }

    #[test]
    fn global_knob_only_without_per_policy_boost() {
        let w = boost_plan(&[], true, false, false);
        assert_eq!(w, vec![(boost_path(), "0".to_string())]);
        assert!(boost_plan(&[], false, false, true).is_empty(), "no boost control at all");
    }

    #[test]
    fn per_policy_boost_never_touches_the_global_knob_to_disable() {
        let w = boost_plan(&[0, 1], true, false, false);
        assert_eq!(w, vec![(policy_attr(0, "boost"), "0".to_string()), (policy_attr(1, "boost"), "0".to_string())]);
    }

    #[test]
    fn a_global_zero_is_lifted_before_per_policy_enables() {
        let w = boost_plan(&[0], true, true, true);
        assert_eq!(w[0], (boost_path(), "1".to_string()));
        assert_eq!(w[1], (policy_attr(0, "boost"), "1".to_string()));
        let w = boost_plan(&[0], true, false, true);
        assert_eq!(w, vec![(policy_attr(0, "boost"), "1".to_string())], "already lifted: leave the global knob alone");
    }
}

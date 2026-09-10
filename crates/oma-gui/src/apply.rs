//! Profile application engine: turns a `Profile` into concrete writes via the
//! helper (or direct writes when root) and the daemons.

use oma_hw::asusd::{PlatformProfile, PlatformProxy};
use oma_hw::helper::Controller;
use oma_hw::model::{HardwareModel, Owner};
use oma_hw::profile::{match_power_mode, Profile};
use oma_hw::SystemInventory;
use std::sync::Arc;

pub async fn apply_profile(p: Profile, inv: Option<Arc<SystemInventory>>, model: Option<Arc<HardwareModel>>) -> Result<String, String> {
    let ctl = Controller::connect().await;
    let mut done: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    // Power mode, through whichever component owns it on this machine.
    if let (Some(wanted), Some(m)) = (&p.cpu.power_mode, &model) {
        match match_power_mode(wanted, &m.controls.power_modes) {
            Some(mode) => match set_power_mode(m.controls.power_owner, mode, &ctl).await {
                Ok(()) => done.push(format!("power mode {mode}")),
                Err(e) => failed.push(format!("power mode: {e}")),
            },
            None => failed.push(format!("power mode {wanted}: this machine has no such mode")),
        }
    }

    // CPU governor / EPP / boost.
    if let Some(cs) = &p.cpu.control {
        let info = inv.as_ref().map(|i| i.cpu.clone()).unwrap_or_else(oma_hw::cpu::cpu_info);
        let mut target = cs.clone();
        if !info.available_governors.contains(&target.governor) {
            target.governor = info.available_governors.first().cloned().unwrap_or_default();
        }
        if let Some(epp) = &target.epp {
            if !info.available_epp.is_empty() && !info.available_epp.contains(epp) {
                target.epp = None;
            }
        }
        let plan = oma_hw::cpu::plan_writes(&info, &target);
        match ctl.write_batch(&plan).await {
            Ok(errs) if errs.is_empty() => done.push(format!("CPU {}{}", target.governor, target.epp.as_ref().map(|e| format!("/{e}")).unwrap_or_default())),
            Ok(errs) => failed.push(format!("CPU: {}", errs.first().map(|(_, e)| e.clone()).unwrap_or_default())),
            Err(e) => failed.push(format!("CPU: {e}")),
        }
    }

    // NVIDIA.
    if let Some(nv) = &p.gpu.nvidia {
        if !oma_hw::nvidia::awake() {
            // Waking the dGPU just to set limits would cost battery; they apply when a profile is next applied with it awake.
            done.push("NVIDIA skipped (asleep or off)".into());
        } else if oma_hw::nvidia::available() {
            match ctl.nvidia_apply(0, nv).await {
                Ok(errs) if errs.is_empty() => done.push("GPU".into()),
                Ok(errs) => failed.push(format!("GPU: {}", errs.iter().map(|(s, e)| format!("{s} ({e})")).collect::<Vec<_>>().join(", "))),
                Err(e) => failed.push(format!("GPU: {e}")),
            }
        }
    }

    // amdgpu perf level.
    if let Some(level) = &p.gpu.amd_perf_level {
        for g in oma_hw::amdgpu::AmdGpu::enumerate() {
            if let Err(e) = ctl.write(g.perf_level_path(), level).await {
                failed.push(format!("AMD GPU: {e}"));
            }
        }
    }

    // Cooling is handled by the fan engine; record intent here.
    if !p.cooling.fans.is_empty() {
        done.push(format!("{} fan targets", p.cooling.fans.len()));
    }

    if failed.is_empty() {
        Ok(format!("{} applied: {}", p.name, done.join(", ")))
    } else if done.is_empty() {
        Err(format!("{} failed: {}", p.name, failed.join("; ")))
    } else {
        Err(format!("{} partly applied ({}); failed: {}", p.name, done.join(", "), failed.join("; ")))
    }
}

async fn set_power_mode(owner: Option<Owner>, mode: &str, ctl: &Controller) -> Result<(), String> {
    let system = || async { zbus::Connection::system().await.map_err(|e| e.to_string()) };
    match owner {
        Some(Owner::Asusd) => {
            let profile = (0..=4).map(PlatformProfile::from_u32).find(|p| p.label().eq_ignore_ascii_case(mode)).ok_or_else(|| format!("asusd has no {mode} mode"))?;
            let c = system().await?;
            PlatformProxy::new(&c).await.map_err(|e| e.to_string())?.set_platform_profile(profile as u32).await.map_err(|e| e.to_string())
        }
        Some(Owner::PowerProfilesDaemon) => oma_hw::ppd::set_active(&system().await?, mode).await.map_err(|e| e.to_string()),
        Some(Owner::Sysfs) => ctl.write("/sys/firmware/acpi/platform_profile", mode).await.map_err(|e| e.to_string()),
        _ => Err("nothing controls power modes here".into()),
    }
}

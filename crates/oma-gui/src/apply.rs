//! Profile application engine: turns a `Profile` into concrete writes via the
//! helper (or direct writes when root) and the daemons.

use oma_hw::helper::Controller;
use oma_hw::profile::Profile;
use oma_hw::SystemInventory;
use std::sync::Arc;

pub async fn apply_profile(p: Profile, inv: Option<Arc<SystemInventory>>) -> Result<String, String> {
    let ctl = Controller::connect().await;
    let mut done: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    // power-profiles-daemon (no privileges needed).
    if let Some(ppd) = &p.cpu.ppd_profile {
        if let Ok(conn) = zbus::Connection::system().await {
            match oma_hw::ppd::set_active(&conn, ppd).await {
                Ok(()) => done.push(format!("power profile {ppd}")),
                Err(e) => failed.push(format!("power profile: {e}")),
            }
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
        if oma_hw::nvidia::available() {
            let mut nv = nv.clone();
            if !p.cooling.gpu_zero_rpm && nv.fan_percent.is_none() {
                // keep automatic policy; zero-rpm is firmware default
            }
            if let Some(w) = nv.power_limit_w {
                nv.power_limit_w = Some(w);
            }
            match ctl.nvidia_apply(0, &nv).await {
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

    // Cooling is handled by the fan engine (next stage); record intent here.
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

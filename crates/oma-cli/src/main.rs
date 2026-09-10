//! `oma` — command-line companion for OmaAsus.

use oma_hw::{amdgpu, cpu, detect, hwmon, nvidia};
use std::time::Duration;

fn usage() -> ! {
    eprintln!("usage: oma <inventory [--json]|sensors|watch [secs]|nvidia|cpu|daemons|rgb [--set index]|capture [dir]>");
    std::process::exit(2)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("inventory") => {
            let inv = detect::inventory();
            if args.get(1).map(String::as_str) == Some("--json") {
                println!("{}", serde_json::to_string_pretty(&inv)?);
            } else {
                print_inventory(&inv);
            }
        }
        Some("sensors") => print_sensors(),
        Some("watch") => {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            let mut mon = cpu::CpuMonitor::new();
            let nv = nvidia::NvidiaGpu::open(0).ok();
            loop {
                let t = mon.sample();
                let top = t.freq_mhz.iter().cloned().fold(0.0, f64::max);
                print!(
                    "\x1b[2K\rCPU {:5.1}% {:4.0} MHz max {:4.0} MHz Tctl {:5.1}°C",
                    t.util_total,
                    t.avg_core_mhz,
                    top,
                    t.tctl_c.unwrap_or(f64::NAN)
                );
                if let Some(pw) = t.package_w {
                    print!(" {pw:5.1} W");
                }
                if let Some(nv) = &nv {
                    if let Ok(g) = nv.telemetry() {
                        print!(
                            " | GPU {:3}% {:4} MHz {:3}°C {:5.1} W fans {:?} {}",
                            g.util_gpu.unwrap_or(0),
                            g.graphics_mhz.unwrap_or(0),
                            g.temp_c.unwrap_or(0),
                            g.power_w.unwrap_or(0.0),
                            g.fan_percent,
                            g.throttle_reasons.join(",")
                        );
                    }
                }
                use std::io::Write;
                std::io::stdout().flush().ok();
                std::thread::sleep(Duration::from_secs(secs));
            }
        }
        Some("nvidia") => {
            let n = nvidia::NvidiaGpu::open(0)?;
            println!("{}", serde_json::to_string_pretty(&n.info()?)?);
            println!("{}", serde_json::to_string_pretty(&n.telemetry()?)?);
        }
        Some("daemons") => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let sys = zbus::Connection::system().await;
                if let Ok(c) = &sys {
                    match oma_hw::ppd::state(c).await {
                        Ok(s) => println!("power-profiles-daemon {} active={} profiles={:?}", s.version, s.active, s.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>()),
                        Err(e) => println!("power-profiles-daemon: {e}"),
                    }
                    match oma_hw::supergfx::state(c).await {
                        Ok(s) => println!("supergfxd {:?}", s),
                        Err(e) => println!("supergfxd: {e}"),
                    }
                    match oma_hw::asusd::discover(c).await {
                        Ok(s) => println!("asusd {:?}", s),
                        Err(e) => println!("asusd: {e}"),
                    }
                    let ctl = oma_hw::helper::Controller::connect().await;
                    println!("oma-helper: {}", if ctl.has_helper() { "connected" } else { "not running" });
                }
                if let Ok(c) = zbus::Connection::session().await {
                    println!("gamemode clients: {:?}", oma_hw::gamemode::client_count(&c).await);
                }
                if oma_hw::hypr::available() {
                    println!("hyprland active: {:?}", oma_hw::hypr::active_window().await.map(|w| (w.class, w.fullscreen)));
                }
            });
        }
        Some("rgb") => {
            let rt = tokio::runtime::Runtime::new()?;
            println!("server_running={}", oma_hw::rgb::server_running());
            rt.block_on(async {
                match oma_hw::rgb::devices().await {
                    Ok(d) => {
                        for x in &d {
                            println!("[{}] {} · {} · {} LEDs · modes={:?} active={}", x.index, x.name, x.kind, x.leds, x.modes.iter().map(|m| m.name.clone()).collect::<Vec<_>>(), x.active_mode);
                        }
                        // Listing is read-only; lighting changes only on request.
                        if args.get(1).map(String::as_str) == Some("--set") {
                            let idx: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0);
                            println!("set_static({idx}) -> {:?}", oma_hw::rgb::set_static(idx, (255, 61, 104)).await.map_err(|e| e.to_string()));
                        }
                    }
                    Err(e) => println!("devices error: {e:#}"),
                }
            });
        }
        Some("capture") => {
            let dir = std::path::PathBuf::from(args.get(1).map(String::as_str).unwrap_or("."));
            let rt = tokio::runtime::Runtime::new()?;
            let raw = rt.block_on(oma_hw::capture::gather());
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("raw-inventory.json");
            std::fs::write(&path, serde_json::to_string_pretty(&raw)? + "\n")?;
            let yes = |b: bool| if b { "yes" } else { "no" };
            println!(
                "captured {} (BIOS {}): {} hwmon devices, {} sysfs attributes, asusd {}, supergfxd {}, NVIDIA GPUs {}",
                raw.system.dmi.product_name,
                raw.system.dmi.bios_version,
                raw.system.hwmon.len(),
                raw.sysfs.len(),
                yes(raw.asusd.is_some()),
                yes(raw.supergfx.is_some()),
                raw.nvidia.len()
            );
            println!("wrote {}", path.display());
            println!("contains model, BIOS and kernel versions and live readings; no serial numbers (GPU UUIDs are redacted)");
        }
        Some("cpu") => {
            println!("{}", serde_json::to_string_pretty(&cpu::cpu_info())?);
            println!("{}", serde_json::to_string_pretty(&cpu::control_state())?);
        }
        _ => usage(),
    }
    Ok(())
}

fn print_inventory(inv: &detect::SystemInventory) {
    println!("Platform : {:?}", inv.platform);
    println!("Board    : {} {} (BIOS {} {})", inv.dmi.board_vendor, inv.dmi.board_name, inv.dmi.bios_version, inv.dmi.bios_date);
    println!("Kernel   : {}", inv.kernel);
    println!(
        "CPU      : {} ({}C/{}T) driver={} status={} governors={:?} epp={:?} boost={} smt={}",
        inv.cpu.model,
        inv.cpu.physical_cores,
        inv.cpu.logical_cpus,
        inv.cpu.scaling_driver.as_deref().unwrap_or("?"),
        inv.cpu.amd_pstate_status.as_deref().unwrap_or("-"),
        inv.cpu.available_governors,
        inv.cpu.available_epp,
        inv.cpu.has_boost,
        inv.cpu.has_smt_control
    );
    println!("NVIDIA   : {} device(s)", inv.nvidia_count);
    for g in &inv.amd_gpus {
        println!("AMD GPU  : {} [{}] integrated={} od={}", g.name, g.pci_slot, g.is_integrated, g.has_od);
    }
    println!("Daemons  : {:?}", inv.daemons);
    println!("Armoury  : {:?}", inv.asus_armoury_attrs);
    println!("Profiles : {:?}", inv.platform_profile_choices);
    println!("Features : {:?}", inv.features);
    println!("HID      :");
    for h in &inv.hid {
        println!("  {:04x}:{:04x} {:<40} if{} up={:04x} {} {}", h.vendor_id, h.product_id, h.product, h.interface, h.usage_page, h.path, if h.accessible { "rw" } else { "--" });
    }
    println!("hwmon    :");
    for d in &inv.hwmon {
        println!(
            "  {:<12} {:<28} temps={} fans={} pwm={} volts={} power={}",
            d.name,
            d.friendly_name(),
            d.temps.len(),
            d.fans.len(),
            d.pwms.len(),
            d.voltages.len(),
            d.powers.len()
        );
    }
}

fn print_sensors() {
    for d in hwmon::enumerate() {
        println!("== {} ({})", d.friendly_name(), d.name);
        for t in &d.temps {
            if let Some(v) = t.read() {
                println!("   {:<26} {:6.1} °C", t.label, v);
            }
        }
        for f in &d.fans {
            if let Some(v) = f.read_rpm() {
                println!("   {:<26} {:6} rpm", f.label, v);
            }
        }
        for p in &d.pwms {
            let s = p.read();
            // Enable values are driver specific, so show the raw number; outputs
            // driven only by a firmware curve have no duty.
            println!(
                "   pwm{:<23} {:>6} (enable {}) {}{}",
                p.index,
                if p.has_duty { s.value.to_string() } else { "-".into() },
                oma_hw::sysfs::read_string(p.enable_path()).unwrap_or_else(|| "-".into()),
                s.temp_sel.map(|t| format!("src={} ", hwmon::temp_sel_label(&d, t))).unwrap_or_default(),
                if s.curve.is_empty() { String::new() } else { format!("curve={:?}", s.curve.iter().map(|c| (c.temp_c, c.pwm)).collect::<Vec<_>>()) }
            );
        }
        for v in &d.voltages {
            if let Some(mv) = v.read_mv() {
                println!("   {:<26} {:6} mV", v.label, mv);
            }
        }
        for p in &d.powers {
            if let Some(w) = p.read_w() {
                println!("   {:<26} {:6.2} W", p.label, w);
            }
        }
    }
    for g in amdgpu::AmdGpu::enumerate() {
        println!("== {} ({})", g.name, g.pci_slot);
        println!("   {:?}", g.telemetry());
    }
}

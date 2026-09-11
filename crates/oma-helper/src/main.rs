//! `oma-helper` — the only privileged component of OmaAsus.
//!
//! Runs as root on the system bus as `com.omaasus.Helper1` and exposes a
//! *narrow* set of operations, each gated by polkit:
//!
//! * `com.omaasus.helper.control`   — routine tuning (fans, governor, EPP,
//!   power limits, platform profile). Active local sessions are allowed
//!   without a password by default, like power-profiles-daemon.
//! * `com.omaasus.helper.advanced`  — overclocking offsets, raw HID writes,
//!   SMT toggling, GPU MUX switches. Requires admin authentication (cached).
//!
//! Every sysfs path is resolved (symlinks included) and the *resolved* path is
//! matched against named attribute sets, so a compromised client cannot turn
//! this into a generic root file writer through class or device links.
//!
//! Fan outputs a client puts under manual control are remembered; if that
//! client disappears from the bus without handing them back (crash, kill),
//! their original modes are restored.

mod polkit;

use oma_hw::nvidia::{NvidiaControl, NvidiaGpu};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use zbus::interface;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

const BUS_NAME: &str = "com.omaasus.Helper1";
const OBJ_PATH: &str = "/com/omaasus/Helper1";
const ACTION_CONTROL: &str = "com.omaasus.helper.control";
const ACTION_ADVANCED: &str = "com.omaasus.helper.advanced";
/// Idle this long, with no client's fans to guard, and the helper exits.
const IDLE_EXIT: std::time::Duration = std::time::Duration::from_secs(300);
const IDLE_CHECK: std::time::Duration = std::time::Duration::from_secs(30);

/// Classify a canonical (symlink-resolved) sysfs path into the polkit action
/// guarding writes to it, or `None` if it may not be written at all.
///
/// Matching the resolved path means class links (`/sys/class/hwmon/…`) and
/// device links (`…/subsystem`, `…/driver`) can't reach attributes outside
/// these sets, such as driver `bind`/`unbind` or module parameters.
fn classify_canonical(path: &Path) -> Option<&'static str> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return None;
    }
    let s = path.to_str()?;
    let name = path.file_name()?.to_str()?;
    let parent = path.parent()?;
    let parent_name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if s.starts_with("/sys/devices/") && is_hwmon_dir(parent_name) {
        return is_fan_attr(name).then_some(ACTION_CONTROL);
    }
    match s {
        "/sys/devices/system/cpu/smt/control" => return Some(ACTION_ADVANCED),
        "/sys/devices/system/cpu/cpufreq/boost" | "/sys/firmware/acpi/platform_profile" => return Some(ACTION_CONTROL),
        _ => {}
    }
    if parent.parent() == Some(Path::new("/sys/devices/system/cpu/cpufreq")) && parent_name.starts_with("policy") {
        let ok = matches!(name, "scaling_governor" | "energy_performance_preference" | "scaling_min_freq" | "scaling_max_freq" | "scaling_setspeed" | "boost");
        return ok.then_some(ACTION_CONTROL);
    }
    if s.starts_with("/sys/devices/pci") {
        return match name {
            "power_dpm_force_performance_level" | "power_dpm_state" | "pp_power_profile_mode" => Some(ACTION_CONTROL),
            "pp_od_clk_voltage" => Some(ACTION_ADVANCED),
            _ => None,
        };
    }
    if let Some(attr) = s.strip_prefix("/sys/devices/virtual/firmware-attributes/asus-armoury/attributes/").and_then(|r| r.strip_suffix("/current_value")) {
        if attr.is_empty() || attr.contains('/') {
            return None;
        }
        return Some(if is_gpu_switch(attr) { ACTION_ADVANCED } else { ACTION_CONTROL });
    }
    if parent == Path::new("/sys/devices/platform/asus-nb-wmi") || parent == Path::new("/sys/devices/platform/eeepc-wmi") {
        return match name {
            n if is_gpu_switch(n) => Some(ACTION_ADVANCED),
            "throttle_thermal_policy" | "panel_od" | "boot_sound" | "mcu_powersave" | "mini_led_mode" | "cpufv" | "ppt_pl1_spl" | "ppt_pl2_sppt" | "ppt_fppt" | "ppt_apu_sppt" | "ppt_platform_sppt" | "nv_dynamic_boost" | "nv_temp_target" => Some(ACTION_CONTROL),
            _ => None,
        };
    }
    if name == "brightness" && s.starts_with("/sys/devices/") && parent.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "leds") {
        return Some(ACTION_CONTROL);
    }
    None
}

/// Switches that can unbind or power off the GPU driving the display.
fn is_gpu_switch(attr: &str) -> bool {
    matches!(attr, "dgpu_disable" | "gpu_mux_mode" | "egpu_enable")
}

fn is_hwmon_dir(name: &str) -> bool {
    name.strip_prefix("hwmon").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Split `123rest` into (`123`, `rest`).
fn split_digits(s: &str) -> (&str, &str) {
    s.split_at(s.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(s.len()))
}

/// Fan-control attributes of a hwmon device: PWM outputs, their modes and
/// curve points, and tachometer alarm floors.
fn is_fan_attr(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("fan") {
        let (n, tail) = split_digits(rest);
        return !n.is_empty() && tail == "_min";
    }
    let Some(rest) = name.strip_prefix("pwm") else { return false };
    let (n, tail) = split_digits(rest);
    if n.is_empty() {
        return false;
    }
    match tail {
        "" | "_enable" | "_mode" | "_temp_sel" | "_step_up_time" | "_step_down_time" | "_floor" | "_start" | "_stop_time" => true,
        t => t.strip_prefix("_auto_point").is_some_and(|r| {
            let (p, kind) = split_digits(r);
            !p.is_empty() && matches!(kind, "_pwm" | "_temp")
        }),
    }
}

/// A PWM duty (`pwmN`) or mode (`pwmN_enable`) attribute: the state the
/// watchdog restores when a client vanishes.
fn is_fan_output(path: &Path) -> bool {
    let in_hwmon = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).is_some_and(is_hwmon_dir);
    let Some(rest) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("pwm")) else { return false };
    let (n, tail) = split_digits(rest);
    in_hwmon && !n.is_empty() && (tail.is_empty() || tail == "_enable")
}

/// Resolve a client path and classify it; returns the canonical path to write.
fn resolve_write(path: &str) -> Option<(PathBuf, &'static str)> {
    let canon = std::fs::canonicalize(path).ok()?;
    let action = classify_canonical(&canon)?;
    // DPM knobs exist on other PCI drivers too; only amdgpu's are ours to touch.
    if canon.starts_with("/sys/devices/pci") && !oma_hw::amdgpu::is_amdgpu_device(canon.parent()?) {
        return None;
    }
    Some((canon, action))
}

/// Resolve a client path for reading root-only telemetry (RAPL, fan state,
/// cpufreq, firmware attributes).
fn resolve_read(path: &str) -> Option<PathBuf> {
    let canon = std::fs::canonicalize(path).ok()?;
    let s = canon.to_str()?;
    let in_hwmon = canon.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).is_some_and(is_hwmon_dir);
    let ok = s.starts_with("/sys/devices/virtual/powercap/")
        || s.starts_with("/sys/devices/system/cpu/")
        || s.starts_with("/sys/devices/virtual/firmware-attributes/")
        || (s.starts_with("/sys/devices/") && in_hwmon);
    ok.then_some(canon)
}

/// Open a HID device only if it belongs to a vendor OmaAsus drives.
fn open_vendor_hid(path: &str) -> zbus::fdo::Result<hidapi::HidDevice> {
    use oma_hw::detect::pid;
    let api = hidapi::HidApi::new().map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
    let cpath = std::ffi::CString::new(path).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
    let vendor = api.device_list().find(|d| d.path() == cpath.as_c_str()).map(|d| d.vendor_id());
    if !matches!(vendor, Some(v) if v == pid::ASUS || v == pid::ENE) {
        return Err(zbus::fdo::Error::AccessDenied("only ASUS/ENE HID devices may be written".into()));
    }
    api.open_path(&cpath).map_err(|e| zbus::fdo::Error::Failed(format!("open {path}: {e}")))
}

/// Fan state a client changed, kept so it can be restored if the client dies.
#[derive(Debug, Default)]
struct Claims {
    /// Canonical PWM attribute → its value before this client first wrote it.
    sysfs: BTreeMap<PathBuf, String>,
    /// NVIDIA GPUs whose fans this client switched to manual.
    nvidia_fans: BTreeSet<u32>,
}

type ClaimMap = Arc<Mutex<HashMap<String, Claims>>>;

/// Work in progress (a call, a password prompt, a restore), counted while it
/// lives so the idle exit waits for it.
struct Busy(Arc<AtomicUsize>);

impl Busy {
    fn enter(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count.clone())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whether the helper may exit: idle long enough, guarding no client's fans,
/// and nothing in progress.
fn may_exit(idle: std::time::Duration, guarding: bool, busy: usize) -> bool {
    idle >= IDLE_EXIT && !guarding && busy == 0
}

fn sender(hdr: &Header<'_>) -> String {
    hdr.sender().map(|s| s.to_string()).unwrap_or_default()
}

struct Helper {
    conn: zbus::Connection,
    claims: ClaimMap,
    /// When a client last asked for anything, for the idle exit.
    last_active: Arc<Mutex<std::time::Instant>>,
    /// Calls in progress (polkit may be asking for a password).
    in_flight: Arc<AtomicUsize>,
    /// Going idle: calls are refused from here on.
    closing: Arc<AtomicBool>,
}

impl Helper {
    async fn authorize(&self, hdr: &Header<'_>, action: &str) -> zbus::fdo::Result<()> {
        // Refused before anything is written or claimed; the caller tries once
        // more and reaches a fresh helper.
        if self.closing.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed(oma_hw::helper::RESTARTING.into()));
        }
        let checked = {
            let _busy = Busy::enter(&self.in_flight);
            polkit::check(&self.conn, &sender(hdr), action, true).await
        };
        // Idle time counts from the answer, not the question.
        *self.last_active.lock().unwrap() = std::time::Instant::now();
        // The helper began stopping while polkit answered, and may have handed
        // the claims back already: claim nothing new. Nothing awaits between
        // here and a call's write, so a call that gets past this finishes first.
        if self.closing.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed(oma_hw::helper::RESTARTING.into()));
        }
        match checked {
            Ok(true) => Ok(()),
            Ok(false) => Err(zbus::fdo::Error::AccessDenied(format!("polkit denied {action}"))),
            Err(e) => Err(zbus::fdo::Error::Failed(format!("polkit check failed: {e}"))),
        }
    }

    /// Remember the pre-write state of a fan output this client is about to change.
    fn claim_before_write(&self, client: &str, path: &Path, value: &str) {
        if !is_fan_output(path) {
            return;
        }
        let path_str = path.to_string_lossy();
        let (duty, enable) = match path_str.strip_suffix("_enable") {
            Some(base) => (PathBuf::from(base), path.to_path_buf()),
            None => (path.to_path_buf(), PathBuf::from(format!("{path_str}_enable"))),
        };
        let mut all = self.claims.lock().unwrap();
        let c = all.entry(client.to_string()).or_default();
        // Writing back the original mode hands the output back: nothing left to restore.
        if path == enable && c.sysfs.get(&enable).is_some_and(|orig| orig == value.trim()) {
            c.sysfs.remove(&enable);
            c.sysfs.remove(&duty);
            return;
        }
        for p in [duty, enable] {
            if !c.sysfs.contains_key(&p)
                && let Some(v) = oma_hw::sysfs::read_string(&p)
            {
                c.sysfs.insert(p, v);
            }
        }
    }
}

/// The writes that put a client's fan settings back: duties first, then the
/// modes that hand control back to firmware.
fn restore_order(sysfs: BTreeMap<PathBuf, String>) -> Vec<(PathBuf, String)> {
    let (modes, duties): (Vec<_>, Vec<_>) = sysfs.into_iter().partition(|(p, _)| p.to_string_lossy().ends_with("_enable"));
    duties.into_iter().chain(modes).collect()
}

/// Put back every fan setting a client changed: when the client vanishes, so
/// a crashed GUI never leaves fans at a fixed manual duty, and when the
/// helper stops.
async fn restore(conn: zbus::Connection, claims: ClaimMap, client: String, reason: &str) {
    let client = client.as_str();
    let Some(c) = claims.lock().unwrap().remove(client) else { return };
    if c.sysfs.is_empty() && c.nvidia_fans.is_empty() {
        return;
    }
    warn!(client, reason, "restoring the fans this client left under manual control");
    for (p, v) in restore_order(c.sysfs) {
        match oma_hw::sysfs::write(&p, &v) {
            Ok(()) => info!(path = %p.display(), value = %v, "restored"),
            Err(e) => warn!(path = %p.display(), error = %e, "restore failed"),
        }
    }
    if !c.nvidia_fans.is_empty() {
        // supergfxd kills whatever holds the dGPU while it switches: wait for a
        // running switch to finish (bounded: a refused one leaves its mode set).
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(90) {
            match oma_hw::supergfx::switch_state(&conn).await {
                Ok((mode, pending)) if oma_hw::supergfx::switch_done(mode, pending) => break,
                // Not there at all: nothing to wait for.
                Err(_) if !oma_hw::supergfx::present(&conn).await => break,
                // Switching, or there but not answering (blocked mid-switch): wait.
                _ => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
            }
        }
        // NVML would wake a sleeping GPU; one that is off has had its driver
        // unloaded, and the driver starts with its fans on automatic.
        if !oma_hw::nvidia::awake() {
            info!("NVIDIA fans left to the driver: the GPU is asleep or off");
            return;
        }
    }
    for index in c.nvidia_fans {
        match NvidiaGpu::open(index) {
            Ok(gpu) => {
                for (step, r) in gpu.apply(&NvidiaControl::fans_only(None)) {
                    if let Err(e) = r {
                        warn!(%step, error = %e, "NVIDIA fan restore failed");
                    }
                }
            }
            Err(e) => warn!(index, error = %e, "cannot open GPU to restore fans"),
        }
    }
}

/// Restore claims of any client that drops off the bus.
async fn watch_clients(conn: zbus::Connection, claims: ClaimMap, restoring: Arc<AtomicUsize>) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.new_owner().is_none() {
            // Its own task (restoring NVIDIA fans may wait for a graphics switch),
            // counted from now so the idle exit waits for it.
            let busy = Busy::enter(&restoring);
            let (conn, claims, client) = (conn.clone(), claims.clone(), args.name().to_string());
            tokio::spawn(async move {
                restore(conn, claims, client, "client vanished").await;
                drop(busy);
            });
        }
    }
    Ok(())
}

#[interface(name = "com.omaasus.Helper1")]
impl Helper {
    /// Helper version.
    #[zbus(property)]
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Write one sysfs attribute. Returns an empty string on success or an error text.
    async fn write_sysfs(&self, #[zbus(header)] hdr: Header<'_>, path: String, value: String) -> zbus::fdo::Result<String> {
        let Some((canon, action)) = resolve_write(&path) else {
            return Err(zbus::fdo::Error::AccessDenied(format!("{path} is not an allowed control attribute")));
        };
        self.authorize(&hdr, action).await?;
        info!(path = %canon.display(), %value, "write");
        self.claim_before_write(&sender(&hdr), &canon, &value);
        Ok(match oma_hw::sysfs::write(&canon, &value) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        })
    }

    /// Write many attributes atomically-ish; returns per-entry error strings ("" = ok).
    async fn write_sysfs_batch(&self, #[zbus(header)] hdr: Header<'_>, entries: Vec<(String, String)>) -> zbus::fdo::Result<Vec<String>> {
        let mut needed = ACTION_CONTROL;
        let mut resolved = Vec::with_capacity(entries.len());
        for (path, value) in entries {
            let Some((canon, action)) = resolve_write(&path) else {
                return Err(zbus::fdo::Error::AccessDenied(format!("{path} is not an allowed control attribute")));
            };
            if action == ACTION_ADVANCED {
                needed = ACTION_ADVANCED;
            }
            resolved.push((canon, value));
        }
        self.authorize(&hdr, needed).await?;
        let client = sender(&hdr);
        let mut out = Vec::with_capacity(resolved.len());
        for (path, value) in &resolved {
            self.claim_before_write(&client, path, value);
            let r = oma_hw::sysfs::write(path, value);
            if let Err(e) = &r {
                warn!(path = %path.display(), %value, error = %e, "write failed");
            }
            out.push(r.err().map(|e| e.to_string()).unwrap_or_default());
        }
        Ok(out)
    }

    /// Read a root-only attribute (RAPL energy counters etc.).
    async fn read_sysfs(&self, #[zbus(header)] hdr: Header<'_>, path: String) -> zbus::fdo::Result<String> {
        let Some(canon) = resolve_read(&path) else {
            return Err(zbus::fdo::Error::AccessDenied(format!("{path} is not readable through the helper")));
        };
        self.authorize(&hdr, ACTION_CONTROL).await?;
        oma_hw::sysfs::read_string(&canon).ok_or_else(|| zbus::fdo::Error::FileNotFound(path))
    }

    /// Apply an NVIDIA control state (JSON-encoded `NvidiaControl`).
    /// Returns a JSON array of `[step, error-or-null]`.
    async fn nvidia_apply(&self, #[zbus(header)] hdr: Header<'_>, index: u32, control_json: String) -> zbus::fdo::Result<String> {
        let ctl: NvidiaControl = serde_json::from_str(&control_json).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let advanced = ctl.touches_advanced();
        self.authorize(&hdr, if advanced { ACTION_ADVANCED } else { ACTION_CONTROL }).await?;
        let gpu = NvidiaGpu::open(index).map_err(|e| {
            warn!(error = %e, "cannot open the GPU through NVML");
            zbus::fdo::Error::Failed(format!("cannot open the GPU through NVML: {e} (if the helper started before the NVIDIA driver, `systemctl restart oma-helper`)"))
        })?;
        {
            let mut all = self.claims.lock().unwrap();
            let c = all.entry(sender(&hdr)).or_default();
            if ctl.fan_auto {
                c.nvidia_fans.remove(&index);
            } else if ctl.fan_percent.is_some() {
                c.nvidia_fans.insert(index);
            }
        }
        let results = gpu.apply(&ctl);
        for (step, r) in &results {
            match r {
                Ok(()) => info!(step, "nvidia ok"),
                Err(e) => warn!(step, error = %e, "nvidia failed"),
            }
        }
        let arr: Vec<(String, Option<String>)> = results.into_iter().map(|(s, r)| (s, r.err())).collect();
        Ok(serde_json::to_string(&arr).unwrap_or_default())
    }

    /// Write a raw HID output report to an ASUS / ENE device (LCD, Aura, Ryujin).
    async fn hid_write(&self, #[zbus(header)] hdr: Header<'_>, path: String, report: Vec<u8>) -> zbus::fdo::Result<u32> {
        self.authorize(&hdr, ACTION_ADVANCED).await?;
        let dev = open_vendor_hid(&path)?;
        dev.write(&report).map(|n| n as u32).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Send a HID feature report to an ASUS / ENE device.
    async fn hid_send_feature(&self, #[zbus(header)] hdr: Header<'_>, path: String, report: Vec<u8>) -> zbus::fdo::Result<()> {
        self.authorize(&hdr, ACTION_ADVANCED).await?;
        let dev = open_vendor_hid(&path)?;
        dev.send_feature_report(&report).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Emitted after any successful write so clients can refresh.
    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>, what: String) -> zbus::Result<()>;
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).init();
    // Before the name is taken, so any stop from then on hands the fans back.
    use tokio::signal::unix::{SignalKind, signal};
    let (mut term, mut int) = (signal(SignalKind::terminate())?, signal(SignalKind::interrupt())?);
    let conn = zbus::Connection::system().await?;
    let claims = ClaimMap::default();
    let last_active = Arc::new(Mutex::new(std::time::Instant::now()));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let restoring = Arc::new(AtomicUsize::new(0));
    let closing = Arc::new(AtomicBool::new(false));
    conn.object_server().at(OBJ_PATH, Helper { conn: conn.clone(), claims: claims.clone(), last_active: last_active.clone(), in_flight: in_flight.clone(), closing: closing.clone() }).await?;
    conn.request_name(BUS_NAME).await?;
    let (watch_conn, watch_claims, watch_restoring) = (conn.clone(), claims.clone(), restoring.clone());
    tokio::spawn(async move {
        if let Err(e) = watch_clients(watch_conn, watch_claims, watch_restoring).await {
            warn!(error = %e, "client watchdog stopped; crashed clients' fans will not be restored");
        }
    });
    // Exit when idle, so D-Bus starts a fresh helper next time: systemd works
    // out device access (the NVIDIA device groups) when the helper starts, and
    // one started before the NVIDIA driver loaded would never get it. Never
    // while a client holds fans: the watchdog has to outlive that client.
    let (idle_conn, idle_claims, idle_restoring, idle_closing) = (conn.clone(), claims.clone(), restoring.clone(), closing.clone());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(IDLE_CHECK).await;
            let idle = last_active.lock().unwrap().elapsed();
            let guarding = idle_claims.lock().unwrap().values().any(|c| !c.sysfs.is_empty() || !c.nvidia_fans.is_empty());
            if may_exit(idle, guarding, in_flight.load(Ordering::SeqCst) + idle_restoring.load(Ordering::SeqCst)) {
                // Refuse calls from here on. Nothing awaits between the check and
                // this, so none has started; a later one is refused before it
                // writes or claims anything, and the caller's retry starts a
                // fresh helper.
                idle_closing.store(true, Ordering::SeqCst);
                info!(idle_s = idle.as_secs(), "idle; exiting (D-Bus starts the helper again when needed)");
                // systemd stops a Type=dbus service once it gives up its name.
                let _ = idle_conn.release_name(BUS_NAME).await;
                std::process::exit(0);
            }
        }
    });
    info!("oma-helper {} ready on {BUS_NAME}", env!("CARGO_PKG_VERSION"));
    tokio::select! {
        _ = term.recv() => info!("SIGTERM: stopping"),
        _ = int.recv() => info!("SIGINT: stopping"),
    }
    // A package upgrade or removal, `systemctl stop`, shutdown. Refuse calls,
    // then hand every client's fans back, so the helper D-Bus starts next
    // records the state they were handed back in rather than a manual duty.
    closing.store(true, Ordering::SeqCst);
    let clients: Vec<String> = claims.lock().unwrap().keys().cloned().collect();
    for client in clients {
        restore(conn.clone(), claims.clone(), client, "helper stopping").await;
    }
    // And any restore the watchdog had already started.
    while restoring.load(Ordering::SeqCst) > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let _ = conn.release_name(BUS_NAME).await;
    info!("stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_sets_duties_before_it_hands_modes_back() {
        let hwmon = Path::new("/sys/class/hwmon/hwmon3");
        let sysfs = BTreeMap::from([(hwmon.join("pwm1_enable"), "2".to_string()), (hwmon.join("pwm1"), "128".to_string()), (hwmon.join("pwm2"), "90".to_string())]);
        let order: Vec<String> = restore_order(sysfs).into_iter().map(|(p, v)| format!("{}={v}", p.file_name().unwrap().to_string_lossy())).collect();
        assert_eq!(order, ["pwm1=128", "pwm2=90", "pwm1_enable=2"]);
    }

    #[test]
    fn the_helper_exits_only_when_nothing_needs_it() {
        let long = IDLE_EXIT + std::time::Duration::from_secs(1);
        assert!(may_exit(long, false, 0));
        assert!(!may_exit(long, true, 0), "a client's fans to guard");
        assert!(!may_exit(long, false, 1), "a call, prompt or restore in progress");
        assert!(!may_exit(IDLE_EXIT / 2, false, 0), "not idle long enough");
    }

    #[test]
    fn busy_counts_while_it_lives() {
        let count = Arc::new(AtomicUsize::new(0));
        let a = Busy::enter(&count);
        let b = Busy::enter(&count);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        drop(a);
        drop(b);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    fn class(p: &str) -> Option<&'static str> {
        classify_canonical(Path::new(p))
    }

    #[test]
    fn allows_named_control_attributes() {
        let cases = [
            ("/sys/devices/platform/asus-nb-wmi/hwmon/hwmon8/pwm1_enable", ACTION_CONTROL),
            ("/sys/devices/platform/asus-nb-wmi/hwmon/hwmon8/pwm2_auto_point8_temp", ACTION_CONTROL),
            ("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2", ACTION_CONTROL),
            ("/sys/devices/platform/nct6775.656/hwmon/hwmon3/fan3_min", ACTION_CONTROL),
            ("/sys/devices/system/cpu/cpufreq/policy5/energy_performance_preference", ACTION_CONTROL),
            ("/sys/devices/system/cpu/cpufreq/boost", ACTION_CONTROL),
            ("/sys/devices/system/cpu/smt/control", ACTION_ADVANCED),
            ("/sys/firmware/acpi/platform_profile", ACTION_CONTROL),
            ("/sys/devices/virtual/firmware-attributes/asus-armoury/attributes/ppt_pl1_spl/current_value", ACTION_CONTROL),
            ("/sys/devices/virtual/firmware-attributes/asus-armoury/attributes/gpu_mux_mode/current_value", ACTION_ADVANCED),
            ("/sys/devices/platform/asus-nb-wmi/throttle_thermal_policy", ACTION_CONTROL),
            ("/sys/devices/platform/asus-nb-wmi/dgpu_disable", ACTION_ADVANCED),
            ("/sys/devices/platform/asus-nb-wmi/leds/asus::kbd_backlight/brightness", ACTION_CONTROL),
            ("/sys/devices/pci0000:00/0000:00:08.1/0000:65:00.0/power_dpm_force_performance_level", ACTION_CONTROL),
            ("/sys/devices/pci0000:00/0000:00:08.1/0000:65:00.0/pp_od_clk_voltage", ACTION_ADVANCED),
        ];
        for (p, want) in cases {
            assert_eq!(class(p), Some(want), "{p}");
        }
    }

    #[test]
    fn rejects_everything_else() {
        // Includes what `…/asus-nb-wmi/subsystem/…`, `…/driver/…` and module links resolve to.
        let cases = [
            "/sys/bus/platform/drivers/acpi-ec/unbind",
            "/sys/bus/platform/drivers_probe",
            "/sys/module/asus_wmi/parameters/fnlock_default",
            "/sys/devices/platform/asus-nb-wmi/driver_override",
            "/sys/devices/platform/asus-nb-wmi/uevent",
            "/sys/devices/platform/asus-nb-wmi/hwmon/hwmon8/uevent",
            "/sys/devices/platform/asus-nb-wmi/hwmon/hwmon8/name",
            "/sys/devices/platform/asus-nb-wmi/hwmon/hwmon8/pwm1_auto_point_temp",
            "/sys/devices/system/cpu/cpu0/online",
            "/sys/devices/system/cpu/cpufreq/policy0/../../smt/control",
            "/sys/devices/virtual/firmware-attributes/asus-armoury/attributes/ppt_pl1_spl/min_value",
            "/sys/devices/pci0000:00/0000:00:08.1/0000:65:00.0/remove",
            "/sys/devices/platform/asus-nb-wmi/leds/asus::kbd_backlight/trigger",
            "/etc/shadow",
            "relative/hwmon1/pwm1",
        ];
        for p in cases {
            assert_eq!(class(p), None, "{p}");
        }
    }

    #[test]
    fn watchdog_tracks_duty_and_mode_only() {
        assert!(is_fan_output(Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2")));
        assert!(is_fan_output(Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_enable")));
        assert!(!is_fan_output(Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_auto_point1_pwm")));
        assert!(!is_fan_output(Path::new("/sys/devices/system/cpu/cpufreq/policy0/pwm1")));
    }
}

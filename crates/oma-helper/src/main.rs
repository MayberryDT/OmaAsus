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
//! their original modes are restored. A restore that fails is kept and
//! retried (`claims`), and every device operation runs in its device's lane
//! on a blocking thread (`lanes`), so one wedged device blocks neither the
//! bus nor other devices.

mod claims;
mod lanes;
mod polkit;

use claims::{Claims, Ledger, Writer};
use std::sync::MutexGuard;
use lanes::Lanes;
use oma_hw::nvidia::{NvidiaControl, NvidiaGpu};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use oma_hw::nvidia::DgpuState;
use tracing::{debug, info, warn};
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
/// How long a restore waits out a running graphics switch before it touches
/// NVIDIA fans (supergfxd kills whatever holds the dGPU while it switches).
const SWITCH_WAIT: std::time::Duration = std::time::Duration::from_secs(90);
/// The same while the helper stops, which has to fit the unit's TimeoutStopSec.
const STOP_SWITCH_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
/// How long a call waits for its device's lane before it is refused as busy.
const LANE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
/// A restore waits longer: it is what hands a fan back.
const RESTORE_LANE_WAIT: std::time::Duration = std::time::Duration::from_secs(8);
/// How often restores that failed are looked at again.
const RETRY_CHECK: std::time::Duration = std::time::Duration::from_secs(2);
/// The whole stop path (hand-backs, retries, in-flight work) fits in this,
/// inside the unit's TimeoutStopSec so systemd never has to SIGKILL a
/// helper mid-restore.
const STOP_BUDGET: std::time::Duration = std::time::Duration::from_secs(12);
/// More GPUs than any supported machine has: bounds the lane map.
const MAX_GPUS: u32 = 8;

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
        "/sys/devices/system/cpu/cpufreq/boost" | "/sys/firmware/acpi/platform_profile" => {
            return Some(ACTION_CONTROL);
        }
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
            "throttle_thermal_policy"
            | "panel_od"
            | "boot_sound"
            | "mcu_powersave"
            | "mini_led_mode"
            | "cpufv"
            | "ppt_pl1_spl"
            | "ppt_pl2_sppt"
            | "ppt_fppt"
            | "ppt_apu_sppt"
            | "ppt_platform_sppt"
            | "nv_dynamic_boost"
            | "nv_temp_target" => Some(ACTION_CONTROL),
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
    let Some(rest) = name.strip_prefix("pwm") else {
        return false;
    };
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

/// A PWM duty (`pwmN`), mode (`pwmN_enable`) or board curve point
/// (`pwmN_auto_pointK_pwm|_temp`) attribute: the state the watchdog restores
/// when a client vanishes.
fn is_fan_output(path: &Path) -> bool {
    let in_hwmon = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).is_some_and(is_hwmon_dir);
    let Some(rest) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("pwm")) else {
        return false;
    };
    let (n, tail) = split_digits(rest);
    let point = tail.strip_prefix("_auto_point").is_some_and(|r| {
        let (k, kind) = split_digits(r);
        !k.is_empty() && matches!(kind, "_pwm" | "_temp")
    });
    in_hwmon && !n.is_empty() && (tail.is_empty() || tail == "_enable" || point)
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
    let ok = s.starts_with("/sys/devices/virtual/powercap/") || s.starts_with("/sys/devices/system/cpu/") || s.starts_with("/sys/devices/virtual/firmware-attributes/") || (s.starts_with("/sys/devices/") && in_hwmon);
    ok.then_some(canon)
}

/// A client's HID path, canonical and under /dev/hidraw, before it names a
/// lane: the lane map must not grow with arbitrary strings.
fn hidraw_path(path: &str) -> zbus::fdo::Result<String> {
    // One answer for "missing" and "elsewhere": not a path-existence oracle.
    let denied = || zbus::fdo::Error::AccessDenied(format!("{path} is not a hidraw device"));
    let canon = std::fs::canonicalize(path).map_err(|_| denied())?;
    let s = canon.to_string_lossy();
    if !s.starts_with("/dev/hidraw") {
        return Err(denied());
    }
    Ok(s.into_owned())
}

/// Open a HID device only if it belongs to a vendor OmaAsus drives.
/// The vendor of the HID device at `path`, from enumeration.
fn hid_vendor(path: &str) -> Option<u16> {
    let api = hidapi::HidApi::new().ok()?;
    let cpath = std::ffi::CString::new(path).ok()?;
    api.device_list().find(|d| d.path() == cpath.as_c_str()).map(|d| d.vendor_id())
}

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

/// sysfs as the ledger sees it.
struct Sysfs;

impl Writer for Sysfs {
    fn write(&self, path: &Path, value: &str) -> Result<(), String> {
        oma_hw::sysfs::write(path, value).map_err(|e| e.to_string())
    }
    fn read(&self, path: &Path) -> Option<String> {
        oma_hw::sysfs::read_string(path)
    }
}

type LedgerRef = Arc<Mutex<Ledger>>;

/// The ledger, poisoned or not: a panic inside one lane must not take the
/// stop path's hand-back down with it.
fn lock_ledger(l: &LedgerRef) -> MutexGuard<'_, Ledger> {
    l.lock().unwrap_or_else(|e| e.into_inner())
}

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
    ledger: LedgerRef,
    lanes: Arc<Lanes>,
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
        // the claims back already: claim nothing new. The lane checks again
        // right before the write, since the lane wait is an await.
        if self.closing.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed(oma_hw::helper::RESTARTING.into()));
        }
        match checked {
            Ok(true) => Ok(()),
            Ok(false) => Err(zbus::fdo::Error::AccessDenied(format!("polkit denied {action}"))),
            Err(e) => Err(zbus::fdo::Error::Failed(format!("polkit check failed: {e}"))),
        }
    }
}

/// One attribute write on a blocking thread: the originals of a fan output
/// are read first (no lock held: a blocking device must not freeze the
/// helper), the write goes in, and only a write that *succeeded* is recorded.
/// Refused once the helper is stopping, so nothing is taken over after the
/// hand-back.
fn write_recorded(ledger: &LedgerRef, closing: &AtomicBool, client: &str, path: &Path, value: &str) -> Result<(), String> {
    if closing.load(Ordering::SeqCst) {
        return Err(oma_hw::helper::RESTARTING.into());
    }
    let fan = is_fan_output(path);
    let originals = fan.then(|| claims::read_originals(&Sysfs, path));
    let r = oma_hw::sysfs::write(path, value).map_err(|e| e.to_string());
    if let (Ok(()), Some(originals)) = (&r, originals) {
        lock_ledger(ledger).record_write(client, path, value, &originals);
    }
    if let Err(e) = &r {
        warn!(path = %path.display(), %value, error = %e, "write failed");
    }
    r
}

/// Write one attribute in its device's lane.
async fn write_in_lane(lanes: &Lanes, ledger: &LedgerRef, closing: &Arc<AtomicBool>, client: String, path: PathBuf, value: String) -> Result<(), String> {
    let (ledger, closing) = (ledger.clone(), closing.clone());
    lanes.run(&lanes::sysfs_lane(&path), LANE_WAIT, move || write_recorded(&ledger, &closing, &client, &path, &value)).await.map_err(|b| b.to_string())?
}

/// What a restore needs: the bus (to wait out a graphics switch), the ledger
/// (to keep what fails), the lanes, and whether the helper is stopping.
#[derive(Clone)]
struct Restorer {
    conn: zbus::Connection,
    ledger: LedgerRef,
    lanes: Arc<Lanes>,
    closing: Arc<AtomicBool>,
}

/// Per-step results of an NVML apply.
type NvidiaSteps = Vec<(String, Result<(), String>)>;

impl Restorer {
    /// A client's claims, taken out of the ledger and put back.
    async fn restore_client(&self, client: String, reason: &str) {
        let Some(c) = lock_ledger(&self.ledger).take(&client) else {
            return;
        };
        self.restore(client, c, 1, reason, None).await;
    }

    /// Restores whose next attempt is due (`all`: everything waiting, for a
    /// stop). Side by side: each waits on its own devices.
    async fn retry_due(&self, all: bool) {
        let due = {
            let mut l = lock_ledger(&self.ledger);
            if all { l.drain_pending() } else { l.due(std::time::Instant::now()) }
        };
        futures_util::future::join_all(due.into_iter().map(|p| self.restore(p.client, p.claims, p.attempts + 1, "retrying an incomplete restore", p.deferred_since))).await;
    }

    /// `deferred_since`: when this restore first waited for a sleeping GPU.
    async fn restore(&self, client: String, c: Claims, attempts: u32, reason: &str, deferred_since: Option<std::time::Instant>) {
        let (conn, ledger, lanes, closing) = (&self.conn, &self.ledger, &self.lanes, &self.closing);
        if c.is_empty() {
            return;
        }
        if deferred_since.is_some() {
            debug!(client, reason, "restore due again");
        } else {
            warn!(client, reason, attempts, "restoring the fans this client left under manual control");
        }
        let mut failed = Claims::default();
        let mut errors = Vec::new();
        // Group by device so each lane's writes keep their duty-before-mode order.
        let mut by_lane: std::collections::BTreeMap<String, std::collections::BTreeMap<PathBuf, String>> = Default::default();
        for (p, v) in c.sysfs {
            by_lane.entry(lanes::sysfs_lane(&p)).or_default().insert(p, v);
        }
        // Devices side by side: a stuck one delays only its own outputs.
        let per_lane = by_lane.into_iter().map(|(lane, sysfs)| async move {
            let snapshot = sysfs.clone();
            let r = lanes.run(&lane, RESTORE_LANE_WAIT, move || claims::restore_sysfs(&Sysfs, &snapshot)).await;
            (sysfs, r)
        });
        for (sysfs, r) in futures_util::future::join_all(per_lane).await {
            match r {
                Ok(out) => {
                    for p in &out.restored {
                        info!(path = %p.display(), "restored");
                    }
                    errors.extend(out.errors);
                    failed.sysfs.extend(out.failed);
                }
                Err(busy) => {
                    warn!(%busy, "restore deferred: device busy");
                    errors.push(busy.to_string());
                    failed.sysfs.extend(sysfs);
                }
            }
        }
        if !c.nvidia_fans.is_empty() {
            // supergfxd kills whatever holds the dGPU while it switches: wait for a
            // running switch to finish (bounded: a refused one leaves its mode set).
            // The bound shrinks once the helper is stopping.
            let started = std::time::Instant::now();
            let mut settled = false;
            while started.elapsed() < if closing.load(Ordering::SeqCst) { STOP_SWITCH_WAIT } else { SWITCH_WAIT } {
                match oma_hw::supergfx::switch_state(conn).await {
                    Ok((mode, pending)) if oma_hw::supergfx::switch_done(mode, pending) => {
                        settled = true;
                        break;
                    }
                    // Not there at all: nothing to wait for.
                    Err(_) if !oma_hw::supergfx::present(conn).await => {
                        settled = true;
                        break;
                    }
                    // Switching, or there but not answering (blocked mid-switch): wait.
                    _ => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
                }
            }
            // Stopping mid-switch: stay off NVML; the switch reloads the driver,
            // which starts with its fans on automatic. So does a GPU that is off
            // the bus. One that is asleep keeps a manual fan policy, and NVML
            // would wake it: the claim waits (spending no attempt) for the GPU
            // to wake on its own, unless the helper is stopping or has waited
            // long enough, when it wakes the GPU once.
            let stopping = closing.load(Ordering::SeqCst);
            let now = std::time::Instant::now();
            let state = oma_hw::nvidia::power_state(oma_hw::nvidia::pci_device().as_deref());
            let wait = state == DgpuState::Suspended && !stopping && deferred_since.is_none_or(|s| now.duration_since(s) < claims::DEFER_MAX);
            if !settled && stopping {
                info!("NVIDIA fans left to the driver: a graphics switch is running");
            } else if state == DgpuState::Absent {
                info!(client, "NVIDIA fans left to the driver: the GPU is off the bus");
            } else if wait {
                if deferred_since.is_none() {
                    info!(client, "NVIDIA fans: GPU asleep; the restore waits for it to wake");
                }
                lock_ledger(ledger).defer(&client, Claims { nvidia_fans: c.nvidia_fans.clone(), ..Default::default() }, attempts, "GPU asleep".into(), now, deferred_since);
            } else {
                if state == DgpuState::Suspended {
                    info!(client, stopping, "NVIDIA fans: waking the GPU once to hand them back");
                }
                for index in c.nvidia_fans {
                    let r = lanes
                        .run(&lanes::nvidia_lane(index), RESTORE_LANE_WAIT, move || {
                            let gpu = NvidiaGpu::open(index).map_err(|e| format!("cannot open GPU {index}: {e}"))?;
                            let errs: Vec<String> = gpu.apply(&NvidiaControl::fans_only(None)).into_iter().filter_map(|(step, r)| r.err().map(|e| format!("{step}: {e}"))).collect();
                            if errs.is_empty() { Ok(()) } else { Err(errs.join(", ")) }
                        })
                        .await
                        .map_err(|b| b.to_string())
                        .and_then(|r| r);
                    match r {
                        Ok(()) => info!(index, "NVIDIA fans restored to automatic"),
                        Err(e) => {
                            warn!(index, error = %e, "NVIDIA fan restore failed");
                            errors.push(e);
                            failed.nvidia_fans.insert(index);
                        }
                    }
                }
            }
        }
        if !failed.is_empty() {
            let error = errors.join("; ");
            let outputs: Vec<String> = failed.sysfs.keys().map(|p| p.to_string_lossy().into_owned()).chain(failed.nvidia_fans.iter().map(|i| format!("nvidia:{i}"))).collect();
            // Stale curve points under a mode that went back are the board's
            // problem, not an uncontrolled fan.
            let uncontrolled = failed.sysfs.keys().any(|p| !claims::is_curve_point(p)) || !failed.nvidia_fans.is_empty();
            let gave_up = lock_ledger(ledger).retain_failed(&client, failed, attempts, error.clone(), std::time::Instant::now());
            if gave_up && !uncontrolled {
                warn!(client, attempts, outputs = ?outputs, error = %error, "the board's own curve points could not all be put back; its fan runs on the mode it had, with a stale point");
            } else if gave_up {
                // Nothing drives these fans now and the helper has run out of
                // attempts: say so where it will be seen, and tell clients.
                tracing::error!(client, attempts, outputs = ?outputs, error = %error, "fan recovery abandoned: these outputs could not be handed back");
                if let Ok(emitter) = SignalEmitter::new(conn, OBJ_PATH) {
                    let _ = Helper::changed(&emitter, format!("{}: {}", oma_hw::helper::RECOVERY_ABANDONED, outputs.join(", "))).await;
                }
            } else {
                warn!(client, attempts, error = %error, "restore incomplete; will retry");
            }
        }
    }
}

/// Restore claims of any client that drops off the bus.
async fn watch_clients(restorer: Restorer, restoring: Arc<AtomicUsize>) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let dbus = zbus::fdo::DBusProxy::new(&restorer.conn).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.new_owner().is_none() {
            // Its own task (restoring NVIDIA fans may wait for a graphics switch),
            // counted from now so the idle exit waits for it.
            let busy = Busy::enter(&restoring);
            let (restorer, client) = (restorer.clone(), args.name().to_string());
            tokio::spawn(async move {
                restorer.restore_client(client, "client vanished").await;
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
        // Counted until the device work is done, so a stop waits for it.
        let _work = Busy::enter(&self.in_flight);
        info!(path = %canon.display(), %value, "write");
        match write_in_lane(&self.lanes, &self.ledger, &self.closing, sender(&hdr), canon, value).await {
            Ok(()) => Ok(String::new()),
            // Refused because the helper is stopping: a bus error, so the client's
            // one retry reaches the fresh helper.
            Err(e) if e == oma_hw::helper::RESTARTING => Err(zbus::fdo::Error::Failed(e)),
            Err(e) => Ok(e),
        }
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
        let _work = Busy::enter(&self.in_flight);
        let client = sender(&hdr);
        // In the caller's order throughout: a batch is built so earlier entries
        // make later ones valid (the global boost knob before per-policy boost,
        // curve points before the mode that enables them). Consecutive entries
        // for one device share a lane run; a stuck device fails its own entries
        // and the ones after it, which must not go in without them.
        let mut out = vec![String::new(); resolved.len()];
        let mut i = 0;
        let mut stuck: Option<String> = None;
        while i < resolved.len() {
            let lane = lanes::sysfs_lane(&resolved[i].0);
            let mut j = i;
            while j < resolved.len() && lanes::sysfs_lane(&resolved[j].0) == lane {
                j += 1;
            }
            if let Some(why) = &stuck {
                for slot in out.iter_mut().take(j).skip(i) {
                    *slot = format!("not written: an earlier entry failed ({why})");
                }
                i = j;
                continue;
            }
            let entries: Vec<(PathBuf, String)> = resolved[i..j].to_vec();
            let (ledger, closing, client) = (self.ledger.clone(), self.closing.clone(), client.clone());
            match self.lanes.run(&lane, LANE_WAIT, move || entries.iter().map(|(path, value)| write_recorded(&ledger, &closing, &client, path, value).err().unwrap_or_default()).collect::<Vec<_>>()).await {
                Ok(results) => {
                    for (k, e) in results.into_iter().enumerate() {
                        out[i + k] = e;
                    }
                }
                Err(busy) => {
                    for slot in out.iter_mut().take(j).skip(i) {
                        *slot = busy.to_string();
                    }
                    stuck = Some(busy.to_string());
                }
            }
            i = j;
        }
        if out.iter().any(|e| e == oma_hw::helper::RESTARTING) {
            return Err(zbus::fdo::Error::Failed(oma_hw::helper::RESTARTING.into()));
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
        // A lane is named after the index: keep the map to plausible GPUs.
        if index >= MAX_GPUS {
            return Err(zbus::fdo::Error::InvalidArgs(format!("GPU index {index} out of range")));
        }
        let advanced = ctl.touches_advanced();
        self.authorize(&hdr, if advanced { ACTION_ADVANCED } else { ACTION_CONTROL }).await?;
        let _work = Busy::enter(&self.in_flight);
        let (client, ledger, closing) = (sender(&hdr), self.ledger.clone(), self.closing.clone());
        let results = self
            .lanes
            .run(&lanes::nvidia_lane(index), LANE_WAIT, move || -> Result<NvidiaSteps, String> {
                if closing.load(Ordering::SeqCst) {
                    return Err(oma_hw::helper::RESTARTING.into());
                }
                let gpu = NvidiaGpu::open(index).map_err(|e| {
                    warn!(error = %e, "cannot open the GPU through NVML");
                    format!("cannot open the GPU through NVML: {e} (if the helper started before the NVIDIA driver, `systemctl restart oma-helper`)")
                })?;
                let results = gpu.apply(&ctl);
                // The claim follows what actually happened to the fans.
                let fans_ok = results.iter().any(|(step, r)| step.starts_with("fan") && r.is_ok());
                if fans_ok {
                    lock_ledger(&ledger).note_nvidia(&client, index, !ctl.fan_auto && ctl.fan_percent.is_some());
                }
                Ok(results)
            })
            .await
            .map_err(|b| zbus::fdo::Error::Failed(b.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
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
        let path = hidraw_path(&path)?;
        // A fan hub's speed and control-mode reports are fan tuning, the same
        // right as a fan header; anything else on a HID device (lighting, an
        // LCD) is advanced control. The cheaper right is checked first, so an
        // unauthorised caller costs no device walk.
        self.authorize(&hdr, ACTION_CONTROL).await?;
        let p = path.clone();
        let vendor = tokio::task::spawn_blocking(move || hid_vendor(&p)).await.ok().flatten();
        if !(vendor == Some(oma_hw::detect::pid::ENE) && oma_hw::lianli::is_fan_report(&report)) {
            self.authorize(&hdr, ACTION_ADVANCED).await?;
        }
        let _work = Busy::enter(&self.in_flight);
        self.lanes
            .run(&lanes::hid_lane(&path), LANE_WAIT, move || {
                let dev = open_vendor_hid(&path)?;
                dev.write(&report).map(|n| n as u32).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
            })
            .await
            .map_err(|b| zbus::fdo::Error::Failed(b.to_string()))?
    }

    /// Send a HID feature report to an ASUS / ENE device.
    async fn hid_send_feature(&self, #[zbus(header)] hdr: Header<'_>, path: String, report: Vec<u8>) -> zbus::fdo::Result<()> {
        self.authorize(&hdr, ACTION_ADVANCED).await?;
        let path = hidraw_path(&path)?;
        let _work = Busy::enter(&self.in_flight);
        self.lanes
            .run(&lanes::hid_lane(&path), LANE_WAIT, move || {
                let dev = open_vendor_hid(&path)?;
                dev.send_feature_report(&report).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
            })
            .await
            .map_err(|b| zbus::fdo::Error::Failed(b.to_string()))?
    }

    /// What the helper still has to put back: clients guarded, restores
    /// waiting for another attempt, and ones it gave up on (JSON).
    async fn recovery_report(&self, #[zbus(header)] hdr: Header<'_>) -> zbus::fdo::Result<String> {
        // Which clients hold which outputs is the active session's business.
        self.authorize(&hdr, ACTION_CONTROL).await?;
        let mut report = lock_ledger(&self.ledger).report();
        report.busy_devices = self.lanes.held();
        Ok(serde_json::to_string(&report).unwrap_or_default())
    }

    /// Emitted as the helper stops having handed the fans back
    /// (`oma_hw::helper::HANDED_BACK`): clients still running send theirs again.
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
    let ledger = LedgerRef::default();
    let lanes = Arc::new(Lanes::default());
    let last_active = Arc::new(Mutex::new(std::time::Instant::now()));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let restoring = Arc::new(AtomicUsize::new(0));
    let closing = Arc::new(AtomicBool::new(false));
    conn.object_server()
        .at(OBJ_PATH, Helper { conn: conn.clone(), ledger: ledger.clone(), lanes: lanes.clone(), last_active: last_active.clone(), in_flight: in_flight.clone(), closing: closing.clone() })
        .await?;
    conn.request_name(BUS_NAME).await?;
    let restorer = Restorer { conn: conn.clone(), ledger: ledger.clone(), lanes: lanes.clone(), closing: closing.clone() };
    let (watch_restorer, watch_restoring) = (restorer.clone(), restoring.clone());
    tokio::spawn(async move {
        if let Err(e) = watch_clients(watch_restorer, watch_restoring).await {
            warn!(error = %e, "client watchdog stopped; crashed clients' fans will not be restored");
        }
    });
    // Restores that failed are tried again on a bounded backoff.
    let (retry_restorer, retry_restoring, retry_closing) = (restorer.clone(), restoring.clone(), closing.clone());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(RETRY_CHECK).await;
            if retry_closing.load(Ordering::SeqCst) {
                return;
            }
            let busy = Busy::enter(&retry_restoring);
            retry_restorer.retry_due(false).await;
            drop(busy);
        }
    });
    // Exit when idle, so D-Bus starts a fresh helper next time: systemd works
    // out device access (the NVIDIA device groups) when the helper starts, and
    // one started before the NVIDIA driver loaded would never get it. Never
    // while a client holds fans: the watchdog has to outlive that client.
    let (idle_conn, idle_ledger, idle_restoring, idle_closing, idle_in_flight) = (conn.clone(), ledger.clone(), restoring.clone(), closing.clone(), in_flight.clone());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(IDLE_CHECK).await;
            // Stopping: the stop path hands the fans back and exits; exiting
            // here could cut one of its restores short.
            if idle_closing.load(Ordering::SeqCst) {
                return;
            }
            let idle = last_active.lock().unwrap().elapsed();
            let guarding = lock_ledger(&idle_ledger).has_work();
            if may_exit(idle, guarding, idle_in_flight.load(Ordering::SeqCst) + idle_restoring.load(Ordering::SeqCst)) {
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
    // Everything within one budget under the unit's TimeoutStopSec: clients'
    // hand-backs side by side (each may wait out a graphics switch), one more
    // go at retries, then the work already in flight.
    let clients: Vec<String> = lock_ledger(&ledger).guarding();
    let handed_back = !clients.is_empty() || lock_ledger(&ledger).has_work();
    let stop = async {
        futures_util::future::join_all(clients.into_iter().map(|client| restorer.restore_client(client, "helper stopping"))).await;
        restorer.retry_due(true).await;
        // Restores the watchdog had started, and calls that got past authorize.
        while restoring.load(Ordering::SeqCst) > 0 || in_flight.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // A call that passed the closing check before it was set may have
        // recorded a claim after the list above was taken: put those back too.
        loop {
            let late: Vec<String> = lock_ledger(&ledger).guarding();
            if late.is_empty() {
                break;
            }
            futures_util::future::join_all(late.into_iter().map(|client| restorer.restore_client(client, "helper stopping (late write)"))).await;
        }
    };
    if tokio::time::timeout(STOP_BUDGET, stop).await.is_err() {
        warn!(budget_s = STOP_BUDGET.as_secs(), "stop budget spent; leaving what has not returned");
    }
    let left = lock_ledger(&ledger).report();
    if !left.pending.is_empty() || !left.given_up.is_empty() {
        tracing::error!(report = %serde_json::to_string(&left).unwrap_or_default(), "stopping with fans not put back");
    }
    // Clients still running send their fans again, which starts a fresh helper
    // once this one has exited (a call that lands sooner is refused and tried
    // again). Said while the name is still ours: anyone may send a signal on
    // the system bus, so clients only take this one from the name's owner.
    if handed_back && let Ok(emitter) = SignalEmitter::new(&conn, OBJ_PATH) {
        let _ = Helper::changed(&emitter, oma_hw::helper::HANDED_BACK.into()).await;
    }
    let _ = conn.release_name(BUS_NAME).await;
    info!("stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(is_fan_output(Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_auto_point1_pwm")), "the board's curve points are put back too");
        assert!(!is_fan_output(Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_auto_point_temp")));
        assert!(!is_fan_output(Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_mode")));
        assert!(!is_fan_output(Path::new("/sys/devices/system/cpu/cpufreq/policy0/pwm1")));
    }
}

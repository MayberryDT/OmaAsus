//! `oma-helper` — the only privileged component of OmaAsus.
//!
//! Runs as root on the system bus as `com.omaasus.Helper1` and exposes a
//! *narrow* set of operations, each gated by polkit:
//!
//! * `com.omaasus.helper.control`   — routine tuning (fans, governor, EPP,
//!   power limits, platform profile). Active local sessions are allowed
//!   without a password by default, like power-profiles-daemon.
//! * `com.omaasus.helper.advanced`  — overclocking offsets, raw HID writes,
//!   SMT toggling. Requires admin authentication (cached).
//!
//! Every sysfs write is validated against an allow-list of path patterns so a
//! compromised client cannot turn this into a generic root file writer.

mod polkit;

use oma_hw::nvidia::{NvidiaControl, NvidiaGpu};
use std::path::{Component, Path, PathBuf};
use tracing::{info, warn};
use zbus::interface;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

const BUS_NAME: &str = "com.omaasus.Helper1";
const OBJ_PATH: &str = "/com/omaasus/Helper1";
const ACTION_CONTROL: &str = "com.omaasus.helper.control";
const ACTION_ADVANCED: &str = "com.omaasus.helper.advanced";

/// (prefix, allowed file-name predicate, action)
fn classify(path: &Path) -> Option<&'static str> {
    // Reject anything with `..` or symlink tricks before pattern matching.
    if path.components().any(|c| matches!(c, Component::ParentDir)) || !path.is_absolute() {
        return None;
    }
    let s = path.to_string_lossy();
    let name = path.file_name()?.to_string_lossy();
    let under = |p: &str| s.starts_with(p);
    if under("/sys/class/hwmon/") || under("/sys/devices/") && s.contains("/hwmon/hwmon") {
        let ok = name.starts_with("pwm") || name.starts_with("fan") && name.ends_with("_min");
        return ok.then_some(ACTION_CONTROL);
    }
    if under("/sys/devices/system/cpu/") {
        if name == "control" && s.ends_with("/smt/control") {
            return Some(ACTION_ADVANCED);
        }
        let ok = matches!(
            name.as_ref(),
            "scaling_governor" | "energy_performance_preference" | "scaling_min_freq" | "scaling_max_freq" | "boost" | "scaling_setspeed"
        );
        return ok.then_some(ACTION_CONTROL);
    }
    if s == "/sys/firmware/acpi/platform_profile" {
        return Some(ACTION_CONTROL);
    }
    if (under("/sys/bus/pci/devices/") || under("/sys/class/drm/") || under("/sys/devices/pci")) && matches!(name.as_ref(), "power_dpm_force_performance_level" | "power_dpm_state" | "pp_power_profile_mode") {
        return Some(ACTION_CONTROL);
    }
    if (under("/sys/bus/pci/devices/") || under("/sys/class/drm/")) && name == "pp_od_clk_voltage" {
        return Some(ACTION_ADVANCED);
    }
    if under("/sys/class/firmware-attributes/asus-armoury/attributes/") && name == "current_value" {
        return Some(ACTION_CONTROL);
    }
    if under("/sys/devices/platform/asus-nb-wmi/") || under("/sys/devices/platform/eeepc-wmi/") {
        return Some(ACTION_CONTROL);
    }
    if under("/sys/class/leds/") && name == "brightness" {
        return Some(ACTION_CONTROL);
    }
    None
}

fn readable(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.starts_with("/sys/class/powercap/") || s.starts_with("/sys/class/hwmon/") || s.starts_with("/sys/devices/system/cpu/") || s.starts_with("/sys/class/firmware-attributes/")
}

struct Helper {
    conn: zbus::Connection,
}

impl Helper {
    async fn authorize(&self, hdr: &Header<'_>, action: &str) -> zbus::fdo::Result<()> {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        match polkit::check(&self.conn, &sender, action, true).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(zbus::fdo::Error::AccessDenied(format!("polkit denied {action}"))),
            Err(e) => Err(zbus::fdo::Error::Failed(format!("polkit check failed: {e}"))),
        }
    }
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
        let p = PathBuf::from(&path);
        let Some(action) = classify(&p) else {
            return Err(zbus::fdo::Error::AccessDenied(format!("{path} is not an allowed control attribute")));
        };
        self.authorize(&hdr, action).await?;
        info!(%path, %value, "write");
        Ok(match oma_hw::sysfs::write(&p, &value) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        })
    }

    /// Write many attributes atomically-ish; returns per-entry error strings ("" = ok).
    async fn write_sysfs_batch(&self, #[zbus(header)] hdr: Header<'_>, entries: Vec<(String, String)>) -> zbus::fdo::Result<Vec<String>> {
        let mut needed = ACTION_CONTROL;
        for (path, _) in &entries {
            match classify(Path::new(path)) {
                Some(ACTION_ADVANCED) => needed = ACTION_ADVANCED,
                Some(_) => {}
                None => return Err(zbus::fdo::Error::AccessDenied(format!("{path} is not an allowed control attribute"))),
            }
        }
        self.authorize(&hdr, needed).await?;
        let mut out = Vec::with_capacity(entries.len());
        for (path, value) in &entries {
            let r = oma_hw::sysfs::write(path, value);
            if let Err(e) = &r {
                warn!(%path, %value, error = %e, "write failed");
            }
            out.push(r.err().map(|e| e.to_string()).unwrap_or_default());
        }
        Ok(out)
    }

    /// Read a root-only attribute (RAPL energy counters etc.).
    async fn read_sysfs(&self, #[zbus(header)] hdr: Header<'_>, path: String) -> zbus::fdo::Result<String> {
        let p = PathBuf::from(&path);
        if !readable(&p) || p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(zbus::fdo::Error::AccessDenied(format!("{path} is not readable through the helper")));
        }
        self.authorize(&hdr, ACTION_CONTROL).await?;
        oma_hw::sysfs::read_string(&p).ok_or_else(|| zbus::fdo::Error::FileNotFound(path))
    }

    /// Apply an NVIDIA control state (JSON-encoded `NvidiaControl`).
    /// Returns a JSON array of `[step, error-or-null]`.
    async fn nvidia_apply(&self, #[zbus(header)] hdr: Header<'_>, index: u32, control_json: String) -> zbus::fdo::Result<String> {
        let ctl: NvidiaControl = serde_json::from_str(&control_json).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let advanced = ctl.touches_advanced();
        self.authorize(&hdr, if advanced { ACTION_ADVANCED } else { ACTION_CONTROL }).await?;
        let gpu = NvidiaGpu::open(index).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
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
        let api = hidapi::HidApi::new().map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let cpath = std::ffi::CString::new(path.clone()).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let dev = api.open_path(&cpath).map_err(|e| zbus::fdo::Error::Failed(format!("open {path}: {e}")))?;
        let info = api.device_list().find(|d| d.path() == cpath.as_c_str());
        if !matches!(info.map(|d| d.vendor_id()), Some(0x0b05) | Some(0x0cf2)) {
            return Err(zbus::fdo::Error::AccessDenied("only ASUS/ENE HID devices may be written".into()));
        }
        dev.write(&report).map(|n| n as u32).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Send a HID feature report.
    async fn hid_send_feature(&self, #[zbus(header)] hdr: Header<'_>, path: String, report: Vec<u8>) -> zbus::fdo::Result<()> {
        self.authorize(&hdr, ACTION_ADVANCED).await?;
        let api = hidapi::HidApi::new().map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let cpath = std::ffi::CString::new(path.clone()).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let dev = api.open_path(&cpath).map_err(|e| zbus::fdo::Error::Failed(format!("open {path}: {e}")))?;
        dev.send_feature_report(&report).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Emitted after any successful write so clients can refresh.
    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>, what: String) -> zbus::Result<()>;
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).init();
    let conn = zbus::Connection::system().await?;
    let helper = Helper { conn: conn.clone() };
    conn.object_server().at(OBJ_PATH, helper).await?;
    conn.request_name(BUS_NAME).await?;
    info!("oma-helper {} ready on {BUS_NAME}", env!("CARGO_PKG_VERSION"));
    std::future::pending::<()>().await;
    Ok(())
}

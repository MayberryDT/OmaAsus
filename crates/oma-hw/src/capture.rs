//! Hardware capture: everything detection reads, gathered once into a
//! serializable [`RawInventory`]. `oma capture` saves it as JSON; fixtures
//! captured on real machines drive the model tests, so hardware can be
//! supported and regression-tested without owning it. Nothing here writes
//! to hardware.

use crate::{asusd, detect, nvidia, ppd, supergfx, sysfs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Bumped when the capture format changes incompatibly.
pub const FORMAT: u32 = 1;

/// Longest attribute value kept; larger files are tables nothing models.
const MAX_VALUE: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawInventory {
    pub format: u32,
    pub captured_at: String,
    /// What [`detect::inventory`] returned.
    pub system: detect::SystemInventory,
    pub asusd: Option<AsusdSnapshot>,
    pub supergfx: Option<supergfx::GfxState>,
    pub ppd: Option<ppd::PpdState>,
    /// GPUs NVML could open (none while a laptop dGPU is powered off).
    pub nvidia: Vec<nvidia::NvidiaInfo>,
    pub pci_display: Vec<PciDevice>,
    /// DRM connector names, e.g. `card1-eDP-1`.
    pub drm_connectors: Vec<String>,
    /// Relevant sysfs attributes by path.
    pub sysfs: BTreeMap<String, SysfsAttr>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysfsAttr {
    /// `None` when the capturing user can't read it (root-only attributes).
    pub value: Option<String>,
    /// Permission bits, e.g. `0o644`: tells writable from read-only attributes.
    pub mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PciDevice {
    pub slot: String,
    pub vendor: String,
    pub device: String,
    pub class: String,
    pub driver: Option<String>,
    pub boot_vga: Option<String>,
    pub runtime_status: Option<String>,
    pub power_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AsusdSnapshot {
    pub objects: asusd::AsusdObjects,
    pub platform: Option<PlatformSnapshot>,
    pub armoury: Vec<asusd::ArmouryAttr>,
    /// Stored firmware fan curves per platform profile (asusd enum value).
    pub fan_curves: BTreeMap<u32, Vec<asusd::CurveData>>,
    pub aura: Vec<AuraSnapshot>,
    pub slash: Option<SlashSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformSnapshot {
    pub version: String,
    pub profile: Option<u32>,
    pub choices: Vec<u32>,
    pub profile_on_ac: Option<u32>,
    pub profile_on_battery: Option<u32>,
    pub change_on_ac: Option<bool>,
    pub change_on_battery: Option<bool>,
    pub linked_epp: Option<bool>,
    pub charge_limit: Option<u8>,
    pub ppt_group: Option<bool>,
    pub disable_nvidia_powerd_on_battery: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuraSnapshot {
    pub path: String,
    pub device_type: Option<u32>,
    pub brightness: Option<u32>,
    pub supported_brightness: Vec<u32>,
    pub supported_basic_modes: Vec<u32>,
    pub supported_basic_zones: Vec<u32>,
    pub supported_power_zones: Vec<u32>,
    pub led_mode: Option<u32>,
    pub led_mode_data: Option<asusd::AuraEffect>,
    /// Keyed by wire mode index, which can differ from `supported_basic_modes`
    /// (asusd 6.4 lists Pulse as 10 there and keys it as 9 here).
    pub all_mode_data: BTreeMap<u32, asusd::AuraEffect>,
    pub led_power: Option<asusd::LedPower>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlashSnapshot {
    pub path: String,
    pub enabled: Option<bool>,
    pub brightness: Option<u8>,
    pub interval: Option<u8>,
    pub mode: Option<u8>,
    pub show_on_boot: Option<bool>,
    pub show_on_sleep: Option<bool>,
    pub show_on_shutdown: Option<bool>,
    pub show_on_battery: Option<bool>,
    pub show_battery_warning: Option<bool>,
    pub show_on_lid_closed: Option<bool>,
}

/// Gather everything detection reads. Blocking sysfs/HID/NVML work runs on a
/// blocking thread; D-Bus services that aren't running are simply absent.
pub async fn gather() -> RawInventory {
    let (system, sysfs, pci_display, drm_connectors, nvidia) =
        tokio::task::spawn_blocking(|| (detect::inventory(), snapshot_sysfs(), pci_display(), drm_connectors(), nvidia_infos())).await.expect("capture thread");
    let (asusd, supergfx, ppd) = match zbus::Connection::system().await {
        Ok(c) => (asusd_snapshot(&c).await, supergfx::state(&c).await.ok(), ppd::state(&c).await.ok()),
        Err(_) => (None, None, None),
    };
    RawInventory { format: FORMAT, captured_at: chrono::Local::now().to_rfc3339(), system, asusd, supergfx, ppd, nvidia, pci_display, drm_connectors, sysfs }
}

/// NVML info per GPU, with the UUID redacted: fixtures are shared. Skipped
/// while the GPU sleeps, since NVML would wake it.
fn nvidia_infos() -> Vec<nvidia::NvidiaInfo> {
    if !nvidia::awake() {
        return Vec::new();
    }
    (0..nvidia::NvidiaGpu::count())
        .filter_map(|i| nvidia::NvidiaGpu::open(i).and_then(|g| g.info()).ok())
        .map(|mut info| {
            info.uuid = format!("GPU-redacted-{}", info.index);
            info
        })
        .collect()
}

async fn asusd_snapshot(conn: &zbus::Connection) -> Option<AsusdSnapshot> {
    let objects = asusd::discover(conn).await.ok()?;
    let mut snap = AsusdSnapshot::default();
    if objects.has_platform {
        if let Ok(p) = asusd::PlatformProxy::builder(conn).cache_properties(zbus::proxy::CacheProperties::No).build().await {
            snap.platform = Some(PlatformSnapshot {
                version: p.version().await.unwrap_or_default(),
                profile: p.platform_profile().await.ok(),
                choices: p.platform_profile_choices().await.unwrap_or_default(),
                profile_on_ac: p.platform_profile_on_ac().await.ok(),
                profile_on_battery: p.platform_profile_on_battery().await.ok(),
                change_on_ac: p.change_platform_profile_on_ac().await.ok(),
                change_on_battery: p.change_platform_profile_on_battery().await.ok(),
                linked_epp: p.platform_profile_linked_epp().await.ok(),
                charge_limit: p.charge_control_end_threshold().await.ok(),
                ppt_group: p.enable_ppt_group().await.ok(),
                disable_nvidia_powerd_on_battery: p.disable_nvidia_powerd_on_battery().await.ok(),
            });
        }
    }
    if objects.has_fan_curves {
        if let Ok(f) = asusd::FanCurvesProxy::new(conn).await {
            for profile in snap.platform.as_ref().map(|p| p.choices.clone()).unwrap_or_default() {
                match f.fan_curve_data(profile).await {
                    Ok(curves) => {
                        snap.fan_curves.insert(profile, curves);
                    }
                    Err(e) => tracing::warn!(profile, error = %e, "cannot read asusd fan curves"),
                }
            }
        }
    }
    for attr in &objects.armoury_attrs {
        if let Ok(a) = asusd::armoury_attr(conn, attr).await {
            snap.armoury.push(a);
        }
    }
    for path in &objects.aura_paths {
        if let Some(a) = aura_snapshot(conn, path).await {
            snap.aura.push(a);
        }
    }
    if let Some(path) = &objects.slash_path {
        snap.slash = slash_snapshot(conn, path).await;
    }
    snap.objects = objects;
    Some(snap)
}

async fn aura_snapshot(conn: &zbus::Connection, path: &str) -> Option<AuraSnapshot> {
    let p = asusd::AuraProxy::builder(conn).path(path).ok()?.cache_properties(zbus::proxy::CacheProperties::No).build().await.ok()?;
    Some(AuraSnapshot {
        path: path.to_string(),
        device_type: p.device_type().await.ok(),
        brightness: p.brightness().await.ok(),
        supported_brightness: p.supported_brightness().await.unwrap_or_default(),
        supported_basic_modes: p.supported_basic_modes().await.unwrap_or_default(),
        supported_basic_zones: p.supported_basic_zones().await.unwrap_or_default(),
        supported_power_zones: p.supported_power_zones().await.unwrap_or_default(),
        led_mode: p.led_mode().await.ok(),
        led_mode_data: p.led_mode_data().await.ok(),
        all_mode_data: p.all_mode_data().await.map(|m| m.into_iter().collect()).unwrap_or_default(),
        led_power: p.led_power().await.ok(),
    })
}

async fn slash_snapshot(conn: &zbus::Connection, path: &str) -> Option<SlashSnapshot> {
    let p = asusd::SlashProxy::builder(conn).path(path).ok()?.cache_properties(zbus::proxy::CacheProperties::No).build().await.ok()?;
    Some(SlashSnapshot {
        path: path.to_string(),
        enabled: p.enabled().await.ok(),
        brightness: p.brightness().await.ok(),
        interval: p.interval().await.ok(),
        mode: p.mode().await.ok(),
        show_on_boot: p.show_on_boot().await.ok(),
        show_on_sleep: p.show_on_sleep().await.ok(),
        show_on_shutdown: p.show_on_shutdown().await.ok(),
        show_on_battery: p.show_on_battery().await.ok(),
        show_battery_warning: p.show_battery_warning().await.ok(),
        show_on_lid_closed: p.show_on_lid_closed().await.ok(),
    })
}

/// Display-class PCI devices with their driver and runtime power state.
fn pci_display() -> Vec<PciDevice> {
    sysfs::list_dir("/sys/bus/pci/devices")
        .into_iter()
        .filter_map(|d| {
            let class = sysfs::read_string(d.join("class"))?;
            if !class.starts_with("0x03") {
                return None;
            }
            Some(PciDevice {
                slot: d.file_name()?.to_string_lossy().into_owned(),
                vendor: sysfs::read_string(d.join("vendor")).unwrap_or_default(),
                device: sysfs::read_string(d.join("device")).unwrap_or_default(),
                class,
                driver: std::fs::read_link(d.join("driver")).ok().and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned())),
                boot_vga: sysfs::read_string(d.join("boot_vga")),
                runtime_status: sysfs::read_string(d.join("power/runtime_status")),
                power_state: sysfs::read_string(d.join("power_state")),
            })
        })
        .collect()
}

fn drm_connectors() -> Vec<String> {
    sysfs::list_dir("/sys/class/drm")
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_owned))
        .filter(|n| n.starts_with("card") && n.contains('-'))
        .collect()
}

fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn files(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    sysfs::list_dir(dir).into_iter().filter(|p| p.is_file()).collect()
}

/// Values of the attributes detection and control care about. Attributes the
/// capturing user can't read are kept with `value: None`, so their presence
/// and permissions still show. Serial numbers are never read.
fn snapshot_sysfs() -> BTreeMap<String, SysfsAttr> {
    let mut out = BTreeMap::new();
    let mut record = |p: &Path| record(&mut out, p);

    for dir in sysfs::hwmon_dirs() {
        files(&dir).iter().filter(|f| file_name(f) != "uevent").for_each(|f| record(f));
    }
    for class in sysfs::list_dir("/sys/class/firmware-attributes") {
        for attr in sysfs::list_dir(class.join("attributes")) {
            if attr.is_dir() {
                files(&attr).iter().for_each(|f| record(f));
            } else {
                record(&attr);
            }
        }
    }
    for f in ["/sys/firmware/acpi/platform_profile", "/sys/firmware/acpi/platform_profile_choices"] {
        record(Path::new(f));
    }
    for d in sysfs::list_dir("/sys/class/platform-profile") {
        ["name", "profile", "choices"].iter().for_each(|n| record(&d.join(n)));
    }
    files("/sys/devices/system/cpu/cpufreq/policy0").iter().for_each(|f| record(f));
    for f in [
        "/sys/devices/system/cpu/cpufreq/boost",
        "/sys/devices/system/cpu/smt/control",
        "/sys/devices/system/cpu/smt/active",
        "/sys/devices/system/cpu/amd_pstate/status",
        "/sys/devices/system/cpu/amd_pstate/prefcore",
        "/sys/devices/system/cpu/intel_pstate/status",
        "/sys/devices/system/cpu/intel_pstate/no_turbo",
    ] {
        record(Path::new(f));
    }
    for d in sysfs::list_dir("/sys/class/power_supply") {
        ["type", "online", "status", "capacity", "charge_control_end_threshold", "charge_control_start_threshold", "charge_behaviour"].iter().for_each(|n| record(&d.join(n)));
    }
    for d in sysfs::list_dir("/sys/class/leds") {
        ["brightness", "max_brightness"].iter().for_each(|n| record(&d.join(n)));
    }
    files("/sys/devices/platform/asus-nb-wmi").iter().filter(|f| !matches!(file_name(f), "uevent" | "modalias" | "driver_override")).for_each(|f| record(f));
    for card in sysfs::list_dir("/sys/class/drm") {
        let n = file_name(&card);
        if n.starts_with("card") && !n.contains('-') {
            ["power_dpm_force_performance_level", "power_dpm_state", "pp_power_profile_mode", "mem_info_vram_total", "boot_vga"].iter().for_each(|a| record(&card.join("device").join(a)));
        }
    }
    for d in sysfs::list_dir("/sys/class/powercap") {
        ["name", "energy_uj", "max_energy_range_uj"].iter().for_each(|n| record(&d.join(n)));
    }
    out
}

fn record(out: &mut BTreeMap<String, SysfsAttr>, path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if !meta.is_file() {
        return;
    }
    let value = std::fs::read(path).ok().filter(|b| b.len() <= MAX_VALUE).and_then(|b| String::from_utf8(b).ok()).map(|s| s.trim_end().to_owned());
    out.insert(path.to_string_lossy().into_owned(), SysfsAttr { value, mode: meta.permissions().mode() & 0o777 });
}

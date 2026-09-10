//! Machine fixtures captured with `oma capture`: every one must load, and each
//! reference machine's capture must show what detection relies on.

use oma_hw::Platform;
use oma_hw::capture::{FORMAT, RawInventory};
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load(machine: &str) -> RawInventory {
    let path = fixtures().join(machine).join("raw-inventory.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_fixture_loads() {
    let mut loaded = 0;
    for entry in std::fs::read_dir(fixtures()).expect("fixtures dir") {
        let dir = entry.expect("fixture entry").path();
        if dir.join("raw-inventory.json").exists() {
            let raw = load(dir.file_name().and_then(|n| n.to_str()).expect("fixture name"));
            assert_eq!(raw.format, FORMAT, "{}", dir.display());
            loaded += 1;
        }
    }
    assert!(loaded > 0, "no fixtures found");
}

/// ROG Zephyrus G14 GA403WR (2025), captured in supergfxd Integrated mode.
#[test]
fn ga403wr_shows_the_laptop_stack() {
    let raw = load("ga403wr");
    assert_eq!(raw.system.platform, Platform::AsusLaptop);

    let asusd = raw.asusd.expect("asusd snapshot");
    assert!(asusd.objects.has_fan_curves);
    // Firmware curves for CPU and GPU fans, stored per platform profile.
    let platform = asusd.platform.expect("platform snapshot");
    assert_eq!(asusd.fan_curves.len(), platform.choices.len());
    for curves in asusd.fan_curves.values() {
        let fans: Vec<&str> = curves.iter().map(|c| c.fan.as_str()).collect();
        assert_eq!(fans, ["CPU", "GPU"]);
    }
    assert_eq!(asusd.aura.len(), 1);
    assert!(!asusd.aura[0].supported_basic_modes.is_empty());
    assert!(asusd.slash.is_some());

    // The fan outputs have curve points and a mode, but no plain `pwmN` duty file.
    let curve_hwmon = raw.system.hwmon.iter().find(|d| d.name == "asus_custom_fan_curve").expect("fan curve hwmon");
    let has = |attr: &str| raw.sysfs.contains_key(curve_hwmon.path.join(attr).to_str().expect("utf-8 path"));
    assert!(has("pwm1_enable") && has("pwm1_auto_point8_temp"));
    assert!(!has("pwm1"));

    // dGPU powered off: only the integrated GPU is on the bus; supergfxd owns switching.
    assert!(raw.nvidia.is_empty());
    assert!(raw.pci_display.iter().all(|d| d.driver.as_deref() == Some("amdgpu")));
    assert!(raw.supergfx.is_some());
    assert!(raw.drm_connectors.iter().any(|c| c.contains("-eDP-")));

    // The root-only RAPL counter is recorded without a value.
    assert!(raw.sysfs.iter().any(|(p, a)| p.ends_with("energy_uj") && a.value.is_none()));
}

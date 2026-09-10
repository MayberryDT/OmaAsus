//! The hardware model built from machine captures.

use oma_hw::capture::RawInventory;
use oma_hw::hwmon::{AutoCurve, FanSensor, HwmonDevice, PwmChannel, TempUnit};
use oma_hw::knowledge::{Overrides, Source};
use oma_hw::model::{CurveTemp, FanBackend, GpuPower, GpuVendor, HardwareModel, Owner, Release, SensorRole};
use std::path::{Path, PathBuf};

fn load(machine: &str) -> RawInventory {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(machine).join("raw-inventory.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("fixture parses")
}

fn ids<T>(items: &[T], id: impl Fn(&T) -> String) -> Vec<String> {
    items.iter().map(id).collect()
}

/// ROG Zephyrus G14 GA403WR, captured in supergfxd Integrated mode.
#[test]
fn ga403wr_fans_are_asusd_firmware_curves() {
    let m = HardwareModel::build(&load("ga403wr"), &Overrides::default());
    assert_eq!(ids(&m.fans, |f| f.id.0.clone()), ["asusd:fan:CPU", "asusd:fan:GPU"]);
    for (f, tach) in m.fans.iter().zip(["hwmon:asus:cpu_fan", "hwmon:asus:gpu_fan"]) {
        assert!(!f.caps.duty, "{}: firmware drives it, OmaAsus only edits the curve", f.id);
        let curve = f.caps.firmware_curve.as_ref().expect("firmware curve");
        assert_eq!((curve.points, curve.temp), (8, CurveTemp::Firmware));
        assert_eq!(f.tach.as_ref().map(|t| t.0.as_str()), Some(tach));
        assert_eq!(f.caps.max_rpm, Some(6800));
        assert_eq!(f.caps.release, Release::Auto);
        assert!(matches!(f.backend, FanBackend::AsusdCurve { .. }));
    }
    assert_eq!(m.fan_owner, Owner::Asusd);
    let cpu = m.fan("asusd:fan:CPU").expect("CPU fan");
    assert_eq!(cpu.firmware_curves.keys().map(String::as_str).collect::<Vec<_>>(), ["Balanced", "Performance", "Quiet"]);
    assert!(cpu.firmware_curves.values().all(|c| c.points.len() == 8));
    assert_eq!(cpu.firmware_curves["Quiet"].points.last().map(|p| p.0), Some(120.0), "the firmware's 255 'never' marker reads as 120");
    assert!(m.notes.iter().any(|n| n.what.contains("asusd:fan:CPU") && matches!(n.source, Source::Reference(_))));
}

#[test]
fn ga403wr_sensors_have_roles() {
    let m = HardwareModel::build(&load("ga403wr"), &Overrides::default());
    let role = |r| m.sensor_for(r).map(|s| s.id.0.clone());
    assert_eq!(role(SensorRole::CpuTemp).as_deref(), Some("hwmon:k10temp:Tctl"));
    assert_eq!(role(SensorRole::IgpuTemp).as_deref(), Some("hwmon:amdgpu:edge"));
    assert_eq!(role(SensorRole::PackagePower).as_deref(), Some("hwmon:amdgpu:PPT"));
    assert!(role(SensorRole::Storage).is_some());
    assert_eq!(role(SensorRole::Coolant), None, "a laptop has no coolant sensor");
    assert_eq!(role(SensorRole::DgpuTemp), None, "the dGPU is off");
}

#[test]
fn ga403wr_gpus_lighting_and_controls() {
    let m = HardwareModel::build(&load("ga403wr"), &Overrides::default());

    let igpu = m.gpus.iter().find(|g| g.vendor == GpuVendor::Amd).expect("iGPU");
    assert!(igpu.integrated && igpu.power == GpuPower::Active);
    let dgpu = m.gpus.iter().find(|g| g.vendor == GpuVendor::Nvidia).expect("dGPU known although powered off");
    assert_eq!((dgpu.power, dgpu.pci_slot.as_deref()), (GpuPower::Off, None));

    let kb = m.lighting.iter().find(|l| l.label == "Keyboard").expect("keyboard");
    assert_eq!(kb.modes, [0, 1, 2, 3, 10]);
    assert_eq!(kb.brightness_levels, [0, 1, 2, 3]);
    let slash = m.lighting.iter().find(|l| l.label == "Slash").expect("Slash");
    assert_eq!(slash.leds, Some(7));
    assert!(slash.modes.contains(&16), "the animation it's showing (Bounce) is one of the known ones");

    let c = &m.controls;
    assert_eq!(c.power_modes, ["Quiet", "Balanced", "Performance"]);
    assert_eq!(c.power_owner, Some(Owner::Asusd));
    assert_eq!(c.charge_limit, Some(100));
    assert_eq!(c.gpu_owner, Some(Owner::Supergfxd));
    let attr = |n: &str| c.attributes.iter().find(|a| a.name == n).expect(n);
    assert_eq!(attr("gpu_mux_mode").owned_by, Some(Owner::Supergfxd));
    assert_eq!(attr("dgpu_disable").owned_by, Some(Owner::Supergfxd));
    assert!(!attr("charge_mode").writable && !attr("nv_base_tgp").writable);
    let pl1 = attr("ppt_pl1_spl");
    assert!(pl1.writable && pl1.owned_by.is_none());
    assert!(pl1.min.is_some() && pl1.max.is_some() && pl1.current.is_some(), "value and range come from sysfs");
}

#[test]
fn user_overrides_win() {
    let raw = load("ga403wr");
    let o = Overrides::parse(
        r#"
        hide = ["hwmon:mt7925_phy0:temp1"]
        [fan_max_rpm]
        CPU = 7100
        [min_duty]
        "asusd:fan:GPU" = 20
        "#,
    )
    .expect("valid quirks");
    let m = HardwareModel::build(&raw, &o);
    assert_eq!(m.fan("asusd:fan:CPU").and_then(|f| f.caps.max_rpm), Some(7100));
    assert_eq!(m.fan("asusd:fan:GPU").map(|f| f.caps.min_duty), Some(20.0));
    assert!(m.sensor("hwmon:mt7925_phy0:temp1").is_none());

    // Matching as another board drops the GA403 facts.
    let other = HardwareModel::build(&raw, &Overrides { board: Some("GA402X".into()), ..Default::default() });
    assert_eq!(other.fan("asusd:fan:CPU").and_then(|f| f.caps.max_rpm), None);
}

/// Synthetic desktop pieces added to the G14 capture, until a real desktop
/// capture is committed: a Super I/O chip on a ROG Crosshair X670E board and a
/// Ryujin AIO.
#[test]
fn desktop_outputs_get_names_and_limits_from_knowledge() {
    let mut raw = load("ga403wr");
    raw.system.dmi.board_name = "ROG CROSSHAIR X670E EXTREME".into();
    raw.system.dmi.product_family = String::new();
    let pwm = |dir: &str, i: u32, curve: Option<AutoCurve>| PwmChannel { index: i, path: PathBuf::from(format!("{dir}/pwm{i}")), has_duty: true, has_enable: true, has_mode: false, auto_curve: curve };
    let smart_fan = AutoCurve { points: 5, temp_unit: TempUnit::Milli, has_temp_sel: true, has_step_times: true, has_floor_start: true };
    let device = |name: &str, dir: &str, pwms: Vec<PwmChannel>| HwmonDevice {
        name: name.into(),
        path: dir.into(),
        device_path: dir.into(),
        temps: Vec::new(),
        fans: vec![FanSensor { index: 1, label: "fan1".into(), input: PathBuf::from(format!("{dir}/fan1_input")), min: None }],
        pwms,
        voltages: Vec::new(),
        powers: Vec::new(),
    };
    raw.system.hwmon.push(device("nct6799", "/sys/class/hwmon/hwmon90", vec![pwm("/sys/class/hwmon/hwmon90", 1, Some(smart_fan)), pwm("/sys/class/hwmon/hwmon90", 6, None)]));
    let ryujin = "/sys/class/hwmon/hwmon91";
    let mut aio = device("rog_ryujin", ryujin, (1..=3).map(|i| pwm(ryujin, i, None)).collect());
    aio.pwms.iter_mut().for_each(|p| p.has_enable = false);
    raw.system.hwmon.push(aio);

    let m = HardwareModel::build(&raw, &Overrides::default());
    let cpu_header = m.fan("superio:pwm1").expect("header");
    assert_eq!(cpu_header.label, "CPU_FAN (pwm1)");
    assert_eq!(cpu_header.caps.firmware_curve.as_ref().map(|c| (c.points, c.temp)), Some((5, CurveTemp::Selectable)));
    assert_eq!(cpu_header.caps.release, Release::RestoreMode);
    assert_eq!(cpu_header.tach.as_ref().map(|t| t.0.as_str()), Some("hwmon:nct6799:fan1"));
    assert_eq!(m.fan("superio:pwm6").map(|f| f.label.as_str()), Some("AIO_PUMP (pwm6)"));

    let pump = m.fan("ryujin:pump").expect("pump");
    assert_eq!((pump.caps.min_duty, pump.caps.release), (60.0, Release::SafeFixed(65.0)));
    assert_eq!(pump.caps.curve_input, oma_hw::model::CurveInput::Coolant);
    assert!(m.fan("ryujin:radiator").is_some() && m.fan("ryujin:block-fan").is_some());
}

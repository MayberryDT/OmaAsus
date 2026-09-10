//! Turning profile curves into firmware curves.

use oma_hw::asusd::{CurveData, PlatformProfile};
use oma_hw::profile::FanCurve;

#[test]
fn firmware_points_rise_and_never_drop() {
    // Out of order, a repeated temperature, a falling duty and an out-of-range point.
    let c = CurveData::from_points("CPU", &[(40.0, 30.0), (40.0, 20.0), (60.0, 50.0), (55.0, 45.0), (70.0, 80.0), (80.0, 90.0), (90.0, 100.0), (300.0, 100.0)], true);
    assert_eq!(c.temp, [40, 41, 55, 60, 70, 80, 90, 120]);
    assert_eq!(c.pwm, [77, 77, 115, 128, 204, 230, 255, 255]);
    assert!(c.enabled && c.fan == "CPU");

    // Fewer points repeat the last one, still rising.
    let short = CurveData::from_points("GPU", &[(50.0, 20.0), (70.0, 60.0)], true);
    assert_eq!(short.temp, [50, 70, 71, 72, 73, 74, 75, 76]);
    assert_eq!(short.pwm[7], 153);
}

#[test]
fn stored_curves_read_back_as_percent() {
    // The GA403WR's stored Quiet CPU curve: the last point uses 255 °C as "never".
    let c = CurveData { fan: "CPU".into(), temp: [47, 69, 70, 72, 74, 76, 78, 255], pwm: [2, 12, 22, 30, 40, 51, 71, 71], enabled: false };
    let pts = c.points();
    assert_eq!(pts.last().map(|p| p.0), Some(120.0));
    // Duties round-trip through the firmware encoding.
    assert_eq!(CurveData::from_points("CPU", &pts, false).pwm, c.pwm);
}

#[test]
fn curves_resample_to_the_firmware_point_count() {
    let five = FanCurve::balanced();
    let eight = five.resample(8);
    assert_eq!(eight.len(), 8);
    assert_eq!((eight[0].0, eight[7].0), (five.points[0].0, five.points[4].0));
    assert!(eight.windows(2).all(|w| w[1].0 > w[0].0 && w[1].1 >= w[0].1));
    // An 8-point curve keeps its temperatures.
    let same = FanCurve { points: eight.clone(), ..FanCurve::balanced() }.resample(8);
    assert_eq!(same.iter().map(|p| p.0).collect::<Vec<_>>(), eight.iter().map(|p| p.0).collect::<Vec<_>>());
}

#[test]
fn platform_profiles_by_label() {
    assert_eq!(PlatformProfile::from_label("quiet"), Some(PlatformProfile::Quiet));
    assert_eq!(PlatformProfile::from_label("Low power"), Some(PlatformProfile::LowPower));
    assert_eq!(PlatformProfile::from_label("Turbo"), None);
}

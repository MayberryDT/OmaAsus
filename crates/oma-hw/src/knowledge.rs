//! Hardware knowledge: facts detection can't read from the machine, keyed by
//! what it did detect (driver names, DMI identity, USB IDs). This is the one
//! place specific hardware is named; everything else derives from detection.
//!
//! Each fact says where it comes from: verified on a named machine, or taken
//! from a reference (see `research/`) and not yet verified here.

use crate::hwmon::TempUnit;

/// Unit of a driver's `pwmN_auto_pointM_temp` attributes.
pub fn curve_temp_unit(driver: &str) -> TempUnit {
    match driver {
        // Verified on a ROG Zephyrus G14 GA403WR (kernel 7.2): plain °C, e.g. 47, 69 … 255.
        "asus_custom_fan_curve" => TempUnit::Celsius,
        _ => TempUnit::Milli,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_units_follow_the_driver() {
        assert_eq!(curve_temp_unit("asus_custom_fan_curve"), TempUnit::Celsius);
        assert_eq!(curve_temp_unit("nct6799"), TempUnit::Milli);
    }
}

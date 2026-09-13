//! Lian Li UNI FAN hub (SL / SL-Infinity / SL v2 / AL v2) over hidraw.
//!
//! Protocol from liquidctl `lianli_uni.py` (1.16), uni-sync and OpenRGB. Each
//! hub has 4 channels, one tachometer per channel, and a "PWM sync" toggle
//! that makes the channel follow the motherboard fan header it is cabled to.
//! Product ids, the mode command (it differs per family) and the speed bytes
//! are liquidctl's.

use serde::{Deserialize, Serialize};

pub const VID_ENE: u16 = 0x0cf2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HubKind {
    Sl,
    SlInfinity,
    SlV2,
    AlV2,
    Al,
}

impl HubKind {
    /// As liquidctl 1.16 lists them (`lianli_uni.py` `_MATCHES`).
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0x7750 | 0xa100 => Some(Self::Sl),
            0xa101 => Some(Self::Al),
            0xa102 => Some(Self::SlInfinity),
            0xa103 | 0xa105 => Some(Self::SlV2),
            0xa104 => Some(Self::AlV2),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sl => "Lian Li UNI FAN SL",
            Self::SlInfinity => "Lian Li UNI FAN SL-Infinity",
            Self::SlV2 => "Lian Li UNI FAN SL v2",
            Self::AlV2 => "Lian Li UNI FAN AL v2",
            Self::Al => "Lian Li UNI FAN AL",
        }
    }

    /// Duty percent → speed byte (liquidctl's formulas, and its bytes for a
    /// stopped fan).
    pub fn speed_byte(self, duty: u8) -> u8 {
        let d = duty.min(100) as f64;
        let v = match self {
            Self::Sl | Self::Al if duty == 0 => 40.0,
            Self::SlInfinity if duty == 0 => 10.0,
            Self::SlV2 | Self::AlV2 if duty == 0 => 7.0,
            Self::Sl | Self::Al => (800.0 + 11.0 * d) / 19.0,
            Self::SlInfinity => (250.0 + 17.5 * d) / 20.0,
            Self::SlV2 | Self::AlV2 => (200.0 + 19.0 * d) / 21.0,
        };
        (v as u32 & 0xff) as u8
    }
    /// The command that sets a channel's control mode: it differs per family
    /// (liquidctl `_PWM_COMMANDS`).
    fn mode_command(self) -> [u8; 3] {
        match self {
            Self::Sl => [0xE0, 0x10, 0x31],
            Self::Al => [0xE0, 0x10, 0x42],
            Self::SlInfinity | Self::SlV2 | Self::AlV2 => [0xE0, 0x10, 0x62],
        }
    }

    fn rpm_offset(self) -> usize {
        match self {
            Self::SlV2 | Self::AlV2 => 2,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LianLiHub {
    pub kind: HubKind,
    pub path: String,
    pub product_id: u16,
}

impl LianLiHub {
    pub fn enumerate() -> Vec<Self> {
        let Ok(api) = hidapi::HidApi::new() else { return Vec::new() };
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for d in api.device_list() {
            if d.vendor_id() != VID_ENE {
                continue;
            }
            let Some(kind) = HubKind::from_pid(d.product_id()) else { continue };
            let path = d.path().to_string_lossy().into_owned();
            if seen.insert(path.clone()) {
                out.push(Self { kind, path, product_id: d.product_id() });
            }
        }
        out
    }

    /// Report for manual speed on channel `c` (1..=4).
    pub fn report_set_speed(&self, c: u8, duty: u8) -> Vec<u8> {
        vec![0xE0, 0x20 + (c.clamp(1, 4) - 1), 0x00, self.kind.speed_byte(duty)]
    }

    /// Report toggling PWM sync (follow the motherboard header) for one
    /// channel: one bit pair per channel, as liquidctl sends it.
    pub fn report_pwm_sync(&self, c: u8, sync: bool) -> Vec<u8> {
        let base: u8 = if sync { 0x11 } else { 0x10 };
        let mut v = self.kind.mode_command().to_vec();
        v.push(base << (c.clamp(1, 4) - 1));
        v
    }

    /// Report for an RGB effect on channel `c`.
    pub fn report_rgb_mode(&self, c: u8, mode: u8, speed: u8, direction: u8, brightness: u8) -> Vec<u8> {
        vec![0xE0, 0x10 + c.clamp(1, 4), mode, speed, direction, brightness]
    }

    pub fn report_rgb_colors(&self, c: u8, colors: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut v = vec![0xE0, 0x30 + c.clamp(1, 4)];
        for (r, g, b) in colors {
            v.extend_from_slice(&[*r, *g, *b]);
        }
        v
    }

    fn open(&self) -> anyhow::Result<hidapi::HidDevice> {
        let api = hidapi::HidApi::new()?;
        let c = std::ffi::CString::new(self.path.clone())?;
        Ok(api.open_path(&c)?)
    }

    /// Direct write (works when hidraw is user accessible via udev `uaccess`):
    /// a plain `write(2)` on the node, which is all hidapi's Linux backend
    /// does, without its device walk first.
    pub fn write(&self, report: &[u8]) -> anyhow::Result<()> {
        use std::io::Write;
        let mut buf = report.to_vec();
        buf.resize(65, 0);
        let mut node = std::fs::OpenOptions::new().write(true).open(&self.path)?;
        node.write_all(&buf)?;
        Ok(())
    }

    /// Tachometer reading per channel (rpm), via the 0xE0 input report.
    pub fn read_rpm(&self) -> anyhow::Result<[u16; 4]> {
        let dev = self.open()?;
        let mut buf = [0u8; 65];
        buf[0] = 0xE0;
        let n = dev.get_input_report(&mut buf)?;
        let off = self.kind.rpm_offset();
        let mut out = [0u16; 4];
        for (i, o) in out.iter_mut().enumerate() {
            let idx = off + 2 * i;
            if idx + 1 < n {
                *o = u16::from_be_bytes([buf[idx], buf[idx + 1]]);
            }
        }
        Ok(out)
    }
}

/// Whether a report to a UNI hub is exactly a fan speed or a channel's
/// control mode (as opposed to its lighting, or anything else): `E0 20..23
/// 00 speed` or one of the families' mode commands, four bytes, padded with
/// zeros to the 65-byte report at most.
pub fn is_fan_report(report: &[u8]) -> bool {
    let shape = matches!(report, [0xE0, 0x20..=0x23, 0x00, _, ..]) || matches!(report, [0xE0, 0x10, 0x31 | 0x42 | 0x62, _, ..]);
    shape && report.len() <= 65 && report[4..].iter().all(|b| *b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub(pid: u16) -> LianLiHub {
        LianLiHub { kind: HubKind::from_pid(pid).unwrap(), path: "/dev/hidraw15".into(), product_id: pid }
    }

    #[test]
    fn reports_match_liquidctl() {
        // SL (0xa100): mode command E0 10 31, one bit pair per channel; speed E0 (0x1F + ch) 00 byte.
        let sl = hub(0xa100);
        assert_eq!(sl.report_pwm_sync(1, true), [0xE0, 0x10, 0x31, 0x11]);
        assert_eq!(sl.report_pwm_sync(2, false), [0xE0, 0x10, 0x31, 0x20]);
        assert_eq!(sl.report_pwm_sync(4, true), [0xE0, 0x10, 0x31, 0x88]);
        assert_eq!(sl.report_set_speed(1, 50), [0xE0, 0x20, 0x00, ((800 + 11 * 50) / 19) as u8]);
        assert_eq!(sl.report_set_speed(3, 0), [0xE0, 0x22, 0x00, 40], "liquidctl's byte for a stopped SL fan");
        // The families liquidctl tells apart by product id.
        assert_eq!((HubKind::from_pid(0xa101), HubKind::from_pid(0xa102), HubKind::from_pid(0xa103), HubKind::from_pid(0xa104), HubKind::from_pid(0xa105)), (Some(HubKind::Al), Some(HubKind::SlInfinity), Some(HubKind::SlV2), Some(HubKind::AlV2), Some(HubKind::SlV2)));
        assert_eq!(hub(0xa101).report_pwm_sync(1, false), [0xE0, 0x10, 0x42, 0x10], "AL uses its own mode command");
        assert_eq!(hub(0xa102).report_pwm_sync(1, false), [0xE0, 0x10, 0x62, 0x10]);
        assert_eq!(hub(0xa102).report_set_speed(1, 0)[3], 10);
        assert_eq!(hub(0xa104).report_set_speed(1, 0)[3], 7);
        assert_eq!(hub(0xa104).report_set_speed(1, 100)[3], ((200 + 19 * 100) / 21) as u8);
        assert_eq!((HubKind::Sl.rpm_offset(), HubKind::SlInfinity.rpm_offset(), HubKind::SlV2.rpm_offset()), (1, 1, 2));
        assert!(is_fan_report(&sl.report_set_speed(2, 40)) && is_fan_report(&hub(0xa101).report_pwm_sync(1, true)));
        assert!(!is_fan_report(&sl.report_rgb_mode(1, 1, 2, 0, 4)) && !is_fan_report(&sl.report_rgb_colors(1, &[(1, 2, 3)])));
        let mut padded = sl.report_set_speed(1, 50);
        padded.resize(65, 0);
        assert!(is_fan_report(&padded));
        padded[10] = 1;
        assert!(!is_fan_report(&padded), "anything beyond the four bytes is not a fan report");
        assert!(!is_fan_report(&[0xE0, 0x20]), "too short");
    }
}

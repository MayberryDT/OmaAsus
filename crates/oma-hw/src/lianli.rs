//! Lian Li UNI FAN hub (SL / SL-Infinity / SL v2 / AL v2) over hidraw.
//!
//! Protocol from liquidctl `lianli_uni.py`, uni-sync and OpenRGB. Each hub
//! has 4 channels, one tachometer per channel, and a "PWM sync" toggle that
//! makes the channel follow the motherboard fan header it is cabled to.

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
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0x7750 | 0xa100 => Some(Self::Sl),
            0xa101 => Some(Self::SlInfinity),
            0xa102 => Some(Self::SlV2),
            0xa103 => Some(Self::AlV2),
            0xa104 => Some(Self::Al),
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

    /// Duty percent → speed byte.
    pub fn speed_byte(self, duty: u8) -> u8 {
        let d = duty.min(100) as f64;
        let v = match self {
            Self::Sl | Self::Al => (800.0 + 11.0 * d) / 19.0,
            Self::SlInfinity => (250.0 + 17.5 * d) / 20.0,
            Self::SlV2 | Self::AlV2 => (200.0 + 19.0 * d) / 21.0,
        };
        (v as u32 & 0xff) as u8
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

    /// Report toggling PWM sync (follow the motherboard header).
    pub fn report_pwm_sync(&self, c: u8, sync: bool) -> Vec<u8> {
        let base: u8 = if sync { 0x11 } else { 0x10 };
        vec![0xE0, 0x10, 0x31, base << (c.clamp(1, 4) - 1)]
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

    /// Direct write (works when hidraw is user accessible via udev `uaccess`).
    pub fn write(&self, report: &[u8]) -> anyhow::Result<()> {
        let mut buf = report.to_vec();
        buf.resize(65, 0);
        self.open()?.write(&buf)?;
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

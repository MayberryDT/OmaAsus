//! ASUS LiveDash OLED / AniMe Matrix controller (USB `0b05:1a21`) found on
//! ROG Extreme boards. Protocol reverse engineered by the Polylux project.
//!
//! Only the low-risk *text mode* is implemented (two lines: label + value).
//! Every packet is a 65-byte HID output report starting with `0xEC`.

pub const PID: u16 = 0x1a21;

pub fn report(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[..bytes.len().min(65)].copy_from_slice(&bytes[..bytes.len().min(65)]);
    v
}

/// `EC DC 00` keep-alive.
pub fn heartbeat() -> Vec<u8> {
    report(&[0xEC, 0xDC, 0x00])
}

/// Switch the chip to OLED text mode (send once before `text`).
pub fn mode_text() -> Vec<u8> {
    report(&[0xEC, 0x51, 0x09])
}

/// Back to the firmware hardware monitor page.
pub fn mode_hw_monitor() -> Vec<u8> {
    report(&[0xEC, 0x51, 0x00])
}

/// Exit preset animation → data input mode (needed before uploads).
pub fn mode_data_input() -> Vec<u8> {
    report(&[0xEC, 0x51, 0x15])
}

/// Two-line text: label (≤18 ASCII bytes at 3..21) and value (≤44 UTF-8 bytes at 21..65).
pub fn text(label: &str, value: &str) -> Vec<u8> {
    let mut v = report(&[0xEC, 0x53, 0x00]);
    let l: Vec<u8> = label.bytes().filter(u8::is_ascii).take(18).collect();
    v[3..3 + l.len()].copy_from_slice(&l);
    let mut vb = value.as_bytes().to_vec();
    while vb.len() > 44 {
        vb.pop();
    }
    while !value.is_char_boundary(vb.len()) {
        vb.pop();
    }
    v[21..21 + vb.len()].copy_from_slice(&vb);
    v
}

/// Find the hidraw path of the LiveDash HID interface.
pub fn hid_path() -> Option<String> {
    let api = hidapi::HidApi::new().ok()?;
    api.device_list()
        .find(|d| d.vendor_id() == 0x0b05 && d.product_id() == PID)
        .map(|d| d.path().to_string_lossy().into_owned())
}

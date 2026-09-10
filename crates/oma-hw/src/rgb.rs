//! OpenRGB SDK client (server on `127.0.0.1:6742`). OpenRGB already knows the
//! ASUS Aura motherboard controller, DRAM, GPU and Lian Li hub on this class of
//! machine, so OmaAsus drives lighting through it rather than re-implementing
//! every HID protocol. The server is started on demand (`openrgb --server`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RgbMode {
    pub index: usize,
    pub name: String,
    pub has_speed: bool,
    pub has_brightness: bool,
    pub per_led: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RgbDevice {
    pub index: usize,
    pub name: String,
    pub kind: String,
    pub vendor: String,
    pub leds: usize,
    pub zones: Vec<(String, usize)>,
    pub modes: Vec<RgbMode>,
    pub active_mode: usize,
    pub colors: Vec<(u8, u8, u8)>,
}

pub fn server_running() -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], 6742)), std::time::Duration::from_millis(150)).is_ok()
}

/// Spawn `openrgb --server` detached (user session). Returns immediately.
pub fn start_server() -> anyhow::Result<()> {
    std::process::Command::new("openrgb")
        .args(["--server", "--noautoconnect"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[allow(deprecated)]
pub async fn devices() -> anyhow::Result<Vec<RgbDevice>> {
    let mut client = openrgb2::OpenRgbClient::connect().await?;
    let mut out = Vec::new();
    let n = client.get_controller_count().await? as usize;
    for i in 0..n {
        let Ok(c) = client.get_controller(i).await else { continue };
        let modes = c
            .modes()
            .iter()
            .enumerate()
            .map(|(mi, m)| RgbMode { index: mi, name: m.name().to_string(), has_speed: m.speed_min().is_some(), has_brightness: m.brightness_min().is_some(), per_led: m.name().eq_ignore_ascii_case("direct") || m.name().eq_ignore_ascii_case("custom") })
            .collect();
        let zones = c.get_all_zones().map(|z| (z.name().to_string(), z.num_leds())).collect();
        out.push(RgbDevice {
            index: i,
            name: c.name().to_string(),
            kind: format!("{:?}", c.device_type()),
            vendor: c.vendor().to_string(),
            leds: c.num_leds(),
            zones,
            modes,
            active_mode: c.mode_iter().position(|m| m.is_active()).unwrap_or(0),
            colors: c.colors().iter().map(|col| (col.r, col.g, col.b)).collect(),
        });
    }
    Ok(out)
}

/// Set every LED of a device to one colour (switches it to direct/custom mode).
#[allow(deprecated)]
pub async fn set_static(index: usize, rgb: (u8, u8, u8)) -> anyhow::Result<()> {
    let client = openrgb2::OpenRgbClient::connect().await?;
    let c = client.get_controller(index).await?;
    c.set_controllable_mode().await?;
    c.set_all_leds(openrgb2::Color::new(rgb.0, rgb.1, rgb.2)).await?;
    Ok(())
}

/// Set per-LED colours (direct mode).
#[allow(deprecated)]
pub async fn set_leds(index: usize, colors: &[(u8, u8, u8)]) -> anyhow::Result<()> {
    let client = openrgb2::OpenRgbClient::connect().await?;
    let c = client.get_controller(index).await?;
    c.set_controllable_mode().await?;
    let cols: Vec<openrgb2::Color> = colors.iter().map(|(r, g, b)| openrgb2::Color::new(*r, *g, *b)).collect();
    c.set_leds(cols).await?;
    Ok(())
}

/// Activate a built-in effect mode by index (speed/brightness at their maxima).
#[allow(deprecated)]
pub async fn set_mode(index: usize, mode: usize) -> anyhow::Result<()> {
    let client = openrgb2::OpenRgbClient::connect().await?;
    let c = client.get_controller(index).await?;
    let m = c.mode_iter().nth(mode).ok_or_else(|| anyhow::anyhow!("mode {mode} not found"))?;
    let mut b = m.builder();
    let _ = b.set_max_brightness();
    b.execute(&c).await?;
    Ok(())
}

#[allow(deprecated)]
pub async fn turn_off(index: usize) -> anyhow::Result<()> {
    let client = openrgb2::OpenRgbClient::connect().await?;
    let c = client.get_controller(index).await?;
    c.turn_off_leds().await?;
    Ok(())
}

pub async fn load_profile(name: &str) -> anyhow::Result<()> {
    let client = openrgb2::OpenRgbClient::connect().await?;
    client.load_profile(name).await?;
    Ok(())
}

pub async fn save_profile(name: &str) -> anyhow::Result<()> {
    let client = openrgb2::OpenRgbClient::connect().await?;
    client.save_profile(name).await?;
    Ok(())
}

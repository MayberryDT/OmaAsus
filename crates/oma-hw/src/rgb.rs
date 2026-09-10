//! Native OpenRGB SDK client (TCP `127.0.0.1:6742`, protocol ≤ 5).
//!
//! OpenRGB already knows the ASUS Aura motherboard controller, DRAM, GPU and
//! fan hubs, so OmaAsus drives lighting through its SDK server. The protocol
//! is tiny: a 16-byte header (`ORGB`, device index, packet id, payload size)
//! followed by a little-endian payload. Written against OpenRGB 1.0rc3 after
//! third-party crates proved unreliable against protocol-5 servers.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const ADDR: &str = "127.0.0.1:6742";
const OUR_PROTOCOL: u32 = 5;
const CLIENT_NAME: &str = "OmaAsus";
const IO_TIMEOUT: Duration = Duration::from_secs(4);

mod id {
    pub const REQUEST_CONTROLLER_COUNT: u32 = 0;
    pub const REQUEST_CONTROLLER_DATA: u32 = 1;
    pub const REQUEST_PROTOCOL_VERSION: u32 = 40;
    pub const SET_CLIENT_NAME: u32 = 50;
    pub const PROFILE_LOAD: u32 = 152;
    pub const PROFILE_SAVE: u32 = 153;
    pub const UPDATE_LEDS: u32 = 1050;
    pub const SET_CUSTOM_MODE: u32 = 1100;
    pub const UPDATE_MODE: u32 = 1101;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RgbMode {
    pub index: usize,
    pub name: String,
    pub has_speed: bool,
    pub has_brightness: bool,
    /// Direct / Custom modes accept per-LED colours.
    pub per_led: bool,
    /// Mode takes its own colour(s) (flags bit MODE_SPECIFIC_COLOR).
    pub mode_color: bool,
    #[serde(skip)]
    raw: ModeRaw,
}

/// Every field of a serialized mode so it can be re-sent with edits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ModeRaw {
    name: String,
    value: i32,
    flags: u32,
    speed_min: u32,
    speed_max: u32,
    brightness_min: u32,
    brightness_max: u32,
    colors_min: u32,
    colors_max: u32,
    speed: u32,
    brightness: u32,
    direction: u32,
    color_mode: u32,
    colors: Vec<u32>,
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
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], 6742)), Duration::from_millis(150)).is_ok()
}

/// Spawn `openrgb --server` detached (user session). Returns immediately.
/// Whether the `openrgb` program is on `PATH`.
pub fn installed() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("openrgb").is_file()))
}

pub fn start_server() -> anyhow::Result<()> {
    std::process::Command::new("openrgb")
        .args(["--server", "--noautoconnect"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

// ---------------------------------------------------------------- wire helpers

struct Reader<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    fn u16(&mut self) -> anyhow::Result<u16> {
        let v = u16::from_le_bytes(self.b.get(self.o..self.o + 2).ok_or_else(|| anyhow::anyhow!("short packet"))?.try_into()?);
        self.o += 2;
        Ok(v)
    }
    fn u32(&mut self) -> anyhow::Result<u32> {
        let v = u32::from_le_bytes(self.b.get(self.o..self.o + 4).ok_or_else(|| anyhow::anyhow!("short packet"))?.try_into()?);
        self.o += 4;
        Ok(v)
    }
    fn i32(&mut self) -> anyhow::Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn str(&mut self) -> anyhow::Result<String> {
        let n = self.u16()? as usize;
        let s = self.b.get(self.o..self.o + n).ok_or_else(|| anyhow::anyhow!("short string"))?;
        self.o += n;
        Ok(String::from_utf8_lossy(s).trim_end_matches('\0').to_string())
    }
    fn skip(&mut self, n: usize) -> anyhow::Result<()> {
        if self.o + n > self.b.len() {
            anyhow::bail!("short packet");
        }
        self.o += n;
        Ok(())
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&((bytes.len() + 1) as u16).to_le_bytes());
    out.extend_from_slice(bytes);
    out.push(0);
}

fn parse_mode(r: &mut Reader<'_>, protocol: u32) -> anyhow::Result<ModeRaw> {
    let name = r.str()?;
    let value = r.i32()?;
    let flags = r.u32()?;
    let speed_min = r.u32()?;
    let speed_max = r.u32()?;
    let (brightness_min, brightness_max) = if protocol >= 3 { (r.u32()?, r.u32()?) } else { (0, 0) };
    let colors_min = r.u32()?;
    let colors_max = r.u32()?;
    let speed = r.u32()?;
    let brightness = if protocol >= 3 { r.u32()? } else { 0 };
    let direction = r.u32()?;
    let color_mode = r.u32()?;
    let n = r.u16()? as usize;
    let mut colors = Vec::with_capacity(n);
    for _ in 0..n {
        colors.push(r.u32()?);
    }
    Ok(ModeRaw { name, value, flags, speed_min, speed_max, brightness_min, brightness_max, colors_min, colors_max, speed, brightness, direction, color_mode, colors })
}

fn serialize_mode(m: &ModeRaw, protocol: u32) -> Vec<u8> {
    let mut o = Vec::new();
    put_str(&mut o, &m.name);
    o.extend_from_slice(&m.value.to_le_bytes());
    o.extend_from_slice(&m.flags.to_le_bytes());
    o.extend_from_slice(&m.speed_min.to_le_bytes());
    o.extend_from_slice(&m.speed_max.to_le_bytes());
    if protocol >= 3 {
        o.extend_from_slice(&m.brightness_min.to_le_bytes());
        o.extend_from_slice(&m.brightness_max.to_le_bytes());
    }
    o.extend_from_slice(&m.colors_min.to_le_bytes());
    o.extend_from_slice(&m.colors_max.to_le_bytes());
    o.extend_from_slice(&m.speed.to_le_bytes());
    if protocol >= 3 {
        o.extend_from_slice(&m.brightness.to_le_bytes());
    }
    o.extend_from_slice(&m.direction.to_le_bytes());
    o.extend_from_slice(&m.color_mode.to_le_bytes());
    o.extend_from_slice(&(m.colors.len() as u16).to_le_bytes());
    for c in &m.colors {
        o.extend_from_slice(&c.to_le_bytes());
    }
    o
}

fn color_u32(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

const FLAG_HAS_SPEED: u32 = 1 << 0;
const FLAG_HAS_BRIGHTNESS: u32 = 1 << 8;
const FLAG_PER_LED_COLOR: u32 = 1 << 5;
const FLAG_MODE_SPECIFIC_COLOR: u32 = 1 << 6;
const COLOR_MODE_PER_LED: u32 = 1;
const COLOR_MODE_MODE_SPECIFIC: u32 = 2;

fn device_type_name(t: i32) -> &'static str {
    match t {
        0 => "Motherboard",
        1 => "DRAM",
        2 => "GPU",
        3 => "Cooler",
        4 => "LED strip",
        5 => "Keyboard",
        6 => "Mouse",
        7 => "Mouse mat",
        8 => "Headset",
        9 => "Headset stand",
        10 => "Gamepad",
        11 => "Light",
        12 => "Speaker",
        13 => "Virtual",
        14 => "Storage",
        15 => "Case",
        16 => "Microphone",
        17 => "Accessory",
        18 => "Keypad",
        _ => "Device",
    }
}

fn parse_controller(index: usize, b: &[u8], protocol: u32) -> anyhow::Result<RgbDevice> {
    let mut r = Reader { b, o: 0 };
    let _size = r.u32()?;
    let kind = r.i32()?;
    let name = r.str()?;
    let vendor = if protocol >= 1 { r.str()? } else { String::new() };
    let _desc = r.str()?;
    let _version = r.str()?;
    let _serial = r.str()?;
    let _location = r.str()?;
    let n_modes = r.u16()? as usize;
    let active_mode = r.i32()?.max(0) as usize;
    let mut modes = Vec::with_capacity(n_modes);
    for i in 0..n_modes {
        let raw = parse_mode(&mut r, protocol)?;
        modes.push(RgbMode {
            index: i,
            name: raw.name.clone(),
            has_speed: raw.flags & FLAG_HAS_SPEED != 0,
            has_brightness: raw.flags & FLAG_HAS_BRIGHTNESS != 0,
            per_led: raw.flags & FLAG_PER_LED_COLOR != 0,
            mode_color: raw.flags & FLAG_MODE_SPECIFIC_COLOR != 0,
            raw,
        });
    }
    let n_zones = r.u16()? as usize;
    let mut zones = Vec::with_capacity(n_zones);
    for _ in 0..n_zones {
        let zname = r.str()?;
        let _ztype = r.i32()?;
        let _min = r.u32()?;
        let _max = r.u32()?;
        let count = r.u32()? as usize;
        let matrix_len = r.u16()? as usize;
        r.skip(matrix_len)?;
        if protocol >= 4 {
            let n_seg = r.u16()? as usize;
            for _ in 0..n_seg {
                let _ = r.str()?;
                r.skip(12)?;
            }
        }
        if protocol >= 5 {
            let _flags = r.u32()?;
        }
        zones.push((zname, count));
    }
    let n_leds = r.u16()? as usize;
    for _ in 0..n_leds {
        let _ = r.str()?;
        let _ = r.u32()?;
    }
    let n_colors = r.u16()? as usize;
    let mut colors = Vec::with_capacity(n_colors);
    for _ in 0..n_colors {
        let c = r.u32()?;
        colors.push(((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8));
    }
    Ok(RgbDevice { index, name, kind: device_type_name(kind).into(), vendor, leds: n_leds, zones, modes, active_mode, colors })
}

// ---------------------------------------------------------------- client

pub struct Client {
    stream: TcpStream,
    pub protocol: u32,
}

impl Client {
    pub async fn connect() -> anyhow::Result<Self> {
        let stream = tokio::time::timeout(IO_TIMEOUT, TcpStream::connect(ADDR)).await.map_err(|_| anyhow::anyhow!("OpenRGB connect timed out"))??;
        stream.set_nodelay(true)?;
        let mut c = Self { stream, protocol: OUR_PROTOCOL };
        c.send(0, id::REQUEST_PROTOCOL_VERSION, &OUR_PROTOCOL.to_le_bytes()).await?;
        let (_, payload) = c.recv(id::REQUEST_PROTOCOL_VERSION).await?;
        let server = if payload.len() >= 4 { u32::from_le_bytes(payload[..4].try_into()?) } else { 0 };
        c.protocol = OUR_PROTOCOL.min(server);
        let mut name = CLIENT_NAME.as_bytes().to_vec();
        name.push(0);
        c.send(0, id::SET_CLIENT_NAME, &name).await?;
        Ok(c)
    }

    async fn send(&mut self, device: u32, packet: u32, payload: &[u8]) -> anyhow::Result<()> {
        let mut buf = Vec::with_capacity(16 + payload.len());
        buf.extend_from_slice(b"ORGB");
        buf.extend_from_slice(&device.to_le_bytes());
        buf.extend_from_slice(&packet.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        tokio::time::timeout(IO_TIMEOUT, self.stream.write_all(&buf)).await.map_err(|_| anyhow::anyhow!("OpenRGB write timed out"))??;
        Ok(())
    }

    /// Receive the next packet with the expected id (skipping unsolicited ones).
    async fn recv(&mut self, expect: u32) -> anyhow::Result<(u32, Vec<u8>)> {
        loop {
            let mut hdr = [0u8; 16];
            tokio::time::timeout(IO_TIMEOUT, self.stream.read_exact(&mut hdr)).await.map_err(|_| anyhow::anyhow!("OpenRGB read timed out"))??;
            if &hdr[..4] != b"ORGB" {
                anyhow::bail!("bad OpenRGB packet magic");
            }
            let device = u32::from_le_bytes(hdr[4..8].try_into()?);
            let packet = u32::from_le_bytes(hdr[8..12].try_into()?);
            let size = u32::from_le_bytes(hdr[12..16].try_into()?) as usize;
            let mut payload = vec![0u8; size];
            if size > 0 {
                tokio::time::timeout(IO_TIMEOUT, self.stream.read_exact(&mut payload)).await.map_err(|_| anyhow::anyhow!("OpenRGB read timed out"))??;
            }
            if packet == expect {
                return Ok((device, payload));
            }
        }
    }

    pub async fn controller_count(&mut self) -> anyhow::Result<u32> {
        self.send(0, id::REQUEST_CONTROLLER_COUNT, &[]).await?;
        let (_, p) = self.recv(id::REQUEST_CONTROLLER_COUNT).await?;
        Ok(u32::from_le_bytes(p.get(..4).ok_or_else(|| anyhow::anyhow!("short count"))?.try_into()?))
    }

    pub async fn controller(&mut self, index: u32) -> anyhow::Result<RgbDevice> {
        self.send(index, id::REQUEST_CONTROLLER_DATA, &self.protocol.to_le_bytes()).await?;
        let (_, p) = self.recv(id::REQUEST_CONTROLLER_DATA).await?;
        parse_controller(index as usize, &p, self.protocol)
    }

    pub async fn set_custom_mode(&mut self, index: u32) -> anyhow::Result<()> {
        self.send(index, id::SET_CUSTOM_MODE, &[]).await
    }

    pub async fn update_leds(&mut self, index: u32, colors: &[(u8, u8, u8)]) -> anyhow::Result<()> {
        let mut p = Vec::with_capacity(6 + colors.len() * 4);
        p.extend_from_slice(&((6 + colors.len() * 4) as u32).to_le_bytes());
        p.extend_from_slice(&(colors.len() as u16).to_le_bytes());
        for (r, g, b) in colors {
            p.extend_from_slice(&color_u32(*r, *g, *b).to_le_bytes());
        }
        self.send(index, id::UPDATE_LEDS, &p).await
    }

    async fn update_mode(&mut self, index: u32, mode_index: usize, raw: &ModeRaw) -> anyhow::Result<()> {
        let body = serialize_mode(raw, self.protocol);
        let mut p = Vec::with_capacity(8 + body.len());
        p.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        p.extend_from_slice(&(mode_index as i32).to_le_bytes());
        p.extend_from_slice(&body);
        self.send(index, id::UPDATE_MODE, &p).await
    }
}

// ---------------------------------------------------------------- high level API

pub async fn devices() -> anyhow::Result<Vec<RgbDevice>> {
    let mut c = Client::connect().await?;
    let n = c.controller_count().await?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        match c.controller(i).await {
            Ok(d) => out.push(d),
            Err(e) => tracing::warn!(index = i, error = %e, "OpenRGB controller parse failed"),
        }
    }
    Ok(out)
}

/// Set every LED of a device to one colour: direct mode when the device
/// exposes LEDs, otherwise its `Static` mode with a mode-specific colour.
pub async fn set_static(index: usize, rgb: (u8, u8, u8)) -> anyhow::Result<()> {
    let mut c = Client::connect().await?;
    let d = c.controller(index as u32).await?;
    if d.leds > 0 && d.modes.iter().any(|m| m.per_led) {
        c.set_custom_mode(index as u32).await?;
        let colors = vec![rgb; d.leds];
        c.update_leds(index as u32, &colors).await?;
        return Ok(());
    }
    let m = d
        .modes
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case("static") && m.mode_color)
        .or_else(|| d.modes.iter().find(|m| m.mode_color))
        .ok_or_else(|| anyhow::anyhow!("{} has no colour-capable mode", d.name))?;
    let mut raw = m.raw.clone();
    raw.color_mode = COLOR_MODE_MODE_SPECIFIC;
    let n = raw.colors_max.max(1) as usize;
    raw.colors = vec![color_u32(rgb.0, rgb.1, rgb.2); n];
    c.update_mode(index as u32, m.index, &raw).await
}

/// Set per-LED colours (direct mode).
pub async fn set_leds(index: usize, colors: &[(u8, u8, u8)]) -> anyhow::Result<()> {
    let mut c = Client::connect().await?;
    c.set_custom_mode(index as u32).await?;
    c.update_leds(index as u32, colors).await
}

/// Activate a built-in effect mode by index, keeping its current colours.
pub async fn set_mode(index: usize, mode: usize) -> anyhow::Result<()> {
    let mut c = Client::connect().await?;
    let d = c.controller(index as u32).await?;
    let m = d.modes.get(mode).ok_or_else(|| anyhow::anyhow!("mode {mode} not found"))?;
    let mut raw = m.raw.clone();
    if raw.flags & FLAG_HAS_BRIGHTNESS != 0 && raw.brightness == 0 {
        raw.brightness = raw.brightness_max;
    }
    if raw.color_mode == 0 {
        raw.color_mode = if m.per_led { COLOR_MODE_PER_LED } else if m.mode_color { COLOR_MODE_MODE_SPECIFIC } else { 0 };
    }
    c.update_mode(index as u32, mode, &raw).await
}

pub async fn turn_off(index: usize) -> anyhow::Result<()> {
    let mut c = Client::connect().await?;
    let d = c.controller(index as u32).await?;
    if let Some(m) = d.modes.iter().find(|m| m.name.eq_ignore_ascii_case("off")) {
        return c.update_mode(index as u32, m.index, &m.raw).await;
    }
    if d.leds > 0 {
        c.set_custom_mode(index as u32).await?;
        return c.update_leds(index as u32, &vec![(0, 0, 0); d.leds]).await;
    }
    anyhow::bail!("{} has no off mode", d.name)
}

pub async fn load_profile(name: &str) -> anyhow::Result<()> {
    let mut c = Client::connect().await?;
    let mut p = name.as_bytes().to_vec();
    p.push(0);
    c.send(0, id::PROFILE_LOAD, &p).await
}

pub async fn save_profile(name: &str) -> anyhow::Result<()> {
    let mut c = Client::connect().await?;
    let mut p = name.as_bytes().to_vec();
    p.push(0);
    c.send(0, id::PROFILE_SAVE, &p).await
}

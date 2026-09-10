//! supergfxd client (`org.supergfxctl.Daemon` at `/org/supergfxctl/Gfx`).
//!
//! Notes from the research (see `research/supergfxctl.md`):
//! * no D-Bus properties, everything is a method; enums travel as `u32`;
//! * `NotifyGfx`/`NotifyAction` fire *before* a switch completes, poll
//!   `PendingMode` until it returns `None`;
//! * `Power` and a second `SetMode` block while a switch is in flight, so all
//!   calls are wrapped in timeouts;
//! * on desktops with a dGPU and no eDP panel the daemon still offers
//!   `Integrated`, which would unbind the primary GPU. [`is_safe_to_switch`]
//!   guards against that.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use zbus::zvariant::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[repr(u32)]
pub enum GfxMode {
    Hybrid = 0,
    Integrated = 1,
    NvidiaNoModeset = 2,
    Vfio = 3,
    AsusEgpu = 4,
    AsusMuxDgpu = 5,
    None = 6,
}

impl GfxMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hybrid => "Hybrid",
            Self::Integrated => "Integrated",
            Self::NvidiaNoModeset => "NVIDIA (no modeset)",
            Self::Vfio => "VFIO passthrough",
            Self::AsusEgpu => "ASUS eGPU",
            Self::AsusMuxDgpu => "MUX: dGPU only",
            Self::None => "None",
        }
    }
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Hybrid,
            1 => Self::Integrated,
            2 => Self::NvidiaNoModeset,
            3 => Self::Vfio,
            4 => Self::AsusEgpu,
            5 => Self::AsusMuxDgpu,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[repr(u32)]
pub enum GfxPower {
    Active = 0,
    Suspended = 1,
    Off = 2,
    AsusDisabled = 3,
    AsusMuxDiscreet = 4,
    Unknown = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[repr(u32)]
pub enum UserActionRequired {
    Logout = 0,
    Reboot = 1,
    SwitchToIntegrated = 2,
    AsusEgpuDisable = 3,
    Nothing = 4,
}

impl UserActionRequired {
    pub fn label(self) -> &'static str {
        match self {
            Self::Logout => "Log out to finish switching",
            Self::Reboot => "Reboot required",
            Self::SwitchToIntegrated => "Switch to Integrated first",
            Self::AsusEgpuDisable => "Disable the eGPU first",
            Self::Nothing => "Applied",
        }
    }
}

#[zbus::proxy(
    interface = "org.supergfxctl.Daemon",
    default_service = "org.supergfxctl.Daemon",
    default_path = "/org/supergfxctl/Gfx"
)]
pub trait SuperGfx {
    fn version(&self) -> zbus::Result<String>;
    fn mode(&self) -> zbus::Result<u32>;
    fn supported(&self) -> zbus::Result<Vec<u32>>;
    fn vendor(&self) -> zbus::Result<String>;
    fn power(&self) -> zbus::Result<u32>;
    fn set_mode(&self, mode: u32) -> zbus::Result<u32>;
    fn pending_mode(&self) -> zbus::Result<u32>;
    fn pending_user_action(&self) -> zbus::Result<u32>;
    #[zbus(signal)]
    fn notify_gfx_status(&self, status: u32) -> zbus::Result<()>;
    #[zbus(signal)]
    fn notify_gfx(&self, mode: u32) -> zbus::Result<()>;
    #[zbus(signal)]
    fn notify_action(&self, action: u32) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GfxState {
    pub version: String,
    pub mode: GfxMode,
    pub supported: Vec<GfxMode>,
    pub vendor: String,
    pub power: Option<GfxPower>,
    pub pending_mode: GfxMode,
    pub pending_action: UserActionRequired,
}

const T: Duration = Duration::from_secs(3);

async fn timed<F, R>(f: F) -> zbus::Result<R>
where
    F: std::future::Future<Output = zbus::Result<R>>,
{
    tokio::time::timeout(T, f).await.map_err(|_| zbus::Error::Failure("supergfxd timed out".into()))?
}

pub async fn state(conn: &zbus::Connection) -> zbus::Result<GfxState> {
    let p = SuperGfxProxy::builder(conn).cache_properties(zbus::proxy::CacheProperties::No).build().await?;
    let power = timed(p.power()).await.ok().map(|v| match v {
        0 => GfxPower::Active,
        1 => GfxPower::Suspended,
        2 => GfxPower::Off,
        3 => GfxPower::AsusDisabled,
        4 => GfxPower::AsusMuxDiscreet,
        _ => GfxPower::Unknown,
    });
    Ok(GfxState {
        version: timed(p.version()).await?,
        mode: GfxMode::from_u32(timed(p.mode()).await?),
        supported: timed(p.supported()).await?.into_iter().map(GfxMode::from_u32).collect(),
        vendor: timed(p.vendor()).await?,
        power,
        pending_mode: GfxMode::from_u32(timed(p.pending_mode()).await?),
        pending_action: match timed(p.pending_user_action()).await? {
            0 => UserActionRequired::Logout,
            1 => UserActionRequired::Reboot,
            2 => UserActionRequired::SwitchToIntegrated,
            3 => UserActionRequired::AsusEgpuDisable,
            _ => UserActionRequired::Nothing,
        },
    })
}

/// Request a mode switch. This can take a long time (the daemon waits for the
/// display manager and may kill GPU users), so give it a generous timeout.
pub async fn set_mode(conn: &zbus::Connection, mode: GfxMode) -> zbus::Result<UserActionRequired> {
    let p = SuperGfxProxy::builder(conn).cache_properties(zbus::proxy::CacheProperties::No).build().await?;
    let r = tokio::time::timeout(Duration::from_secs(90), p.set_mode(mode as u32)).await.map_err(|_| zbus::Error::Failure("supergfxd SetMode timed out".into()))??;
    Ok(match r {
        0 => UserActionRequired::Logout,
        1 => UserActionRequired::Reboot,
        2 => UserActionRequired::SwitchToIntegrated,
        3 => UserActionRequired::AsusEgpuDisable,
        _ => UserActionRequired::Nothing,
    })
}

/// A laptop has an internal panel connector; desktops don't. Switching a
/// desktop to `Integrated` would tear down its only display GPU.
pub fn is_safe_to_switch() -> bool {
    crate::sysfs::list_dir("/sys/class/drm").iter().any(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        n.contains("-eDP-") || n.contains("-LVDS-") || n.contains("-DSI-")
    })
}

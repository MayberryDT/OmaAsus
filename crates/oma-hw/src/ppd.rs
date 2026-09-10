//! `power-profiles-daemon` (and `tuned-ppd`) client over the system bus.
//! Interface `net.hadess.PowerProfiles` on `/net/hadess/PowerProfiles`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zbus::zvariant::OwnedValue;

#[zbus::proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
pub trait PowerProfiles {
    /// Hold a profile (`performance` or `power-saver`) with a reason; returns a cookie.
    fn hold_profile(&self, profile: &str, reason: &str, application_id: &str) -> zbus::Result<u32>;
    fn release_profile(&self, cookie: u32) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
    #[zbus(property)]
    fn performance_degraded(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn performance_inhibited(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn active_profile_holds(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    #[zbus(signal)]
    fn profile_released(&self, cookie: u32) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PpdProfile {
    pub name: String,
    pub driver: String,
    pub cpu_driver: Option<String>,
    pub platform_driver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PpdState {
    pub active: String,
    pub profiles: Vec<PpdProfile>,
    pub degraded: String,
    pub version: String,
}

fn str_of(m: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    m.get(k).and_then(|v| <&str>::try_from(v).ok().map(str::to_owned))
}

pub async fn state(conn: &zbus::Connection) -> zbus::Result<PpdState> {
    let p = PowerProfilesProxy::new(conn).await?;
    let profiles = p
        .profiles()
        .await?
        .iter()
        .map(|m| PpdProfile {
            name: str_of(m, "Profile").unwrap_or_default(),
            driver: str_of(m, "Driver").unwrap_or_default(),
            cpu_driver: str_of(m, "CpuDriver"),
            platform_driver: str_of(m, "PlatformDriver"),
        })
        .collect();
    Ok(PpdState {
        active: p.active_profile().await?,
        profiles,
        degraded: p.performance_degraded().await.unwrap_or_default(),
        version: p.version().await.unwrap_or_default(),
    })
}

pub async fn set_active(conn: &zbus::Connection, profile: &str) -> zbus::Result<()> {
    PowerProfilesProxy::new(conn).await?.set_active_profile(profile).await
}

//! Feral GameMode client (session bus): detect when games register so the
//! automatic profile engine can react.

#[zbus::proxy(
    interface = "com.feralinteractive.GameMode",
    default_service = "com.feralinteractive.GameMode",
    default_path = "/com/feralinteractive/GameMode"
)]
pub trait GameMode {
    #[zbus(property)]
    fn client_count(&self) -> zbus::Result<i32>;
    fn list_games(&self) -> zbus::Result<Vec<(i32, zbus::zvariant::OwnedObjectPath)>>;
    #[zbus(signal)]
    fn game_registered(&self, pid: i32, path: zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;
    #[zbus(signal)]
    fn game_unregistered(&self, pid: i32, path: zbus::zvariant::OwnedObjectPath) -> zbus::Result<()>;
}

/// Number of GameMode clients, or `None` when GameMode is not running.
pub async fn client_count(conn: &zbus::Connection) -> Option<i32> {
    let p = GameModeProxy::builder(conn).cache_properties(zbus::proxy::CacheProperties::No).build().await.ok()?;
    p.client_count().await.ok()
}

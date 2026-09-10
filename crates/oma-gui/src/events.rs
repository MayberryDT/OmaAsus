//! System events that call for re-applying the active profile: resume from
//! sleep (logind), the charger connecting or disconnecting (limits differ on
//! battery), and graphics mode switches (supergfxd).

use iced::futures::stream::{self, BoxStream, Stream, StreamExt};
use iced::futures::SinkExt;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Event {
    Resumed,
    /// Mains power connected (`true`) or disconnected.
    Power(bool),
    Graphics,
}

#[zbus::proxy(interface = "org.freedesktop.login1.Manager", default_service = "org.freedesktop.login1", default_path = "/org/freedesktop/login1")]
trait Login1 {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

const AC_POLL: Duration = Duration::from_secs(2);

/// Whether mains power is connected; `None` without a mains supply (desktops).
fn on_ac() -> Option<bool> {
    let mains: Vec<_> = oma_hw::sysfs::list_dir("/sys/class/power_supply").into_iter().filter(|p| oma_hw::sysfs::read_string(p.join("type")).as_deref() == Some("Mains")).collect();
    (!mains.is_empty()).then(|| mains.iter().any(|p| oma_hw::sysfs::read_string(p.join("online")).as_deref() == Some("1")))
}

pub fn stream() -> impl Stream<Item = Event> {
    iced::stream::channel(8, async move |mut out| {
        let power = stream::unfold(on_ac(), |last| async move {
            loop {
                tokio::time::sleep(AC_POLL).await;
                let now = on_ac();
                if now != last {
                    return Some((now.map(Event::Power), now));
                }
            }
        })
        .filter_map(|e| async move { e });
        let mut sources: Vec<BoxStream<'static, Event>> = vec![power.boxed()];
        let mut watching = vec![if on_ac().is_some() { "charger" } else { "no charger (no mains supply)" }];
        if let Ok(conn) = zbus::Connection::system().await {
            if let Ok(sleep) = async { Login1Proxy::new(&conn).await?.receive_prepare_for_sleep().await }.await {
                // `start = false` is the wake-up half.
                sources.push(sleep.filter_map(|s| async move { s.args().ok().filter(|a| !*a.start()).map(|_| Event::Resumed) }).boxed());
                watching.push("resume");
            }
            if let Ok(gfx) = async { oma_hw::supergfx::SuperGfxProxy::new(&conn).await?.receive_notify_gfx_status().await }.await {
                sources.push(gfx.map(|_| Event::Graphics).boxed());
                watching.push("graphics switches");
            }
        }
        tracing::info!(events = %watching.join(", "), "re-applying the active profile on system events");
        let mut events = stream::select_all(sources);
        while let Some(e) = events.next().await {
            if out.send(e).await.is_err() {
                break;
            }
        }
    })
}

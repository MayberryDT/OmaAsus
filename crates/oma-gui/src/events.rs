//! System events: resume from sleep (logind) and the charger connecting or
//! disconnecting (limits differ on battery) call for re-applying the active
//! profile; a graphics switch starting and the dGPU coming back on the bus
//! (supergfxd) call for staying off the dGPU first.

use iced::futures::stream::{self, BoxStream, Stream, StreamExt};
use iced::futures::SinkExt;
use oma_hw::supergfx::{dgpu_arrived, GfxPower, SuperGfxProxy, UserActionRequired};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Event {
    Resumed,
    /// Mains power connected (`true`) or disconnected.
    Power(bool),
    /// supergfxd is switching modes now (not a switch waiting for a logout, or
    /// refused): it kills whatever holds the dGPU meanwhile.
    GraphicsSwitch,
    /// The dGPU came back on the bus.
    DgpuArrived,
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
            if let Ok(gfx) = SuperGfxProxy::new(&conn).await {
                // supergfxd announces a switch before it touches the dGPU, with what
                // the user must do first; `Nothing` means it runs now.
                if let Ok(actions) = gfx.receive_notify_action().await {
                    sources.push(actions.filter_map(|s| async move { s.args().ok().filter(|a| *a.action() == UserActionRequired::Nothing as u32).map(|_| Event::GraphicsSwitch) }).boxed());
                    watching.push("graphics switches");
                }
                // Its status signal also reports every runtime-PM wake and sleep;
                // only the dGPU coming back on the bus matters.
                if let Ok(status) = gfx.receive_notify_gfx_status().await {
                    let arrivals = status
                        .scan(None, |last: &mut Option<GfxPower>, s| {
                            let now = s.args().ok().map(|a| GfxPower::from_u32(*a.status()));
                            let arrived = now.is_some_and(|n| dgpu_arrived(*last, n));
                            if now.is_some() {
                                *last = now;
                            }
                            std::future::ready(Some(arrived))
                        })
                        .filter_map(|arrived| async move { arrived.then_some(Event::DgpuArrived) });
                    sources.push(arrivals.boxed());
                    watching.push("dGPU arrivals");
                }
            }
        }
        tracing::info!(events = %watching.join(", "), "watching system events");
        let mut events = stream::select_all(sources);
        while let Some(e) = events.next().await {
            if out.send(e).await.is_err() {
                break;
            }
        }
    })
}

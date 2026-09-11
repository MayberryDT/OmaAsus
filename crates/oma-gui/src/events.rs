//! System events: resume from sleep (logind) and the charger connecting or
//! disconnecting (limits differ on battery) call for re-applying the active
//! profile; a graphics switch starting and the dGPU coming back on the bus
//! (supergfxd) call for staying off the dGPU first; the helper giving up its
//! bus name calls for sending the fans again.

use iced::futures::stream::{self, BoxStream, Stream, StreamExt};
use iced::futures::SinkExt;
use oma_hw::supergfx::{dgpu_arrived, GfxPower, SuperGfxProxy, UserActionRequired};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Event {
    Resumed,
    /// Mains power connected (`true`) or disconnected.
    Power(bool),
    /// supergfxd has started a switch: anything but a refusal (a switch that
    /// waits for a logout has started too, and runs at once without a display
    /// manager). It kills whatever holds the dGPU meanwhile.
    GraphicsSwitch,
    /// The dGPU came back on the bus.
    DgpuArrived,
    /// The helper gave up its bus name (stopped, restarted or went idle) and
    /// handed back any fans it guarded.
    HelperGone,
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
                // supergfxd announces a switch right after starting it, with what the
                // user must do; only its refusals mean nothing happens. The signal can
                // lose a race with an early KillNvidia: the mode gate and the settle
                // after the dGPU arrives are the main protection.
                if let Ok(actions) = gfx.receive_notify_action().await {
                    sources.push(actions.filter_map(|s| async move { s.args().ok().filter(|a| UserActionRequired::from_u32(*a.action()).switch_runs()).map(|_| Event::GraphicsSwitch) }).boxed());
                    watching.push("graphics switches");
                }
                // Its status signal also reports every runtime-PM wake and sleep;
                // only the dGPU coming back on the bus matters.
                if let Ok(status) = gfx.receive_notify_gfx_status().await {
                    // It sends changes only: start from what it says now, so the first change counts.
                    let initial = oma_hw::supergfx::state(&conn).await.ok().and_then(|s| s.power);
                    let arrivals = status
                        .scan(initial, |last: &mut Option<GfxPower>, s| {
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
            // The name is released, never handed over: D-Bus starts the next
            // helper only when something calls it.
            if let Ok(owners) = async { zbus::fdo::DBusProxy::new(&conn).await?.receive_name_owner_changed_with_args(&[(0, oma_hw::helper::BUS_NAME)]).await }.await {
                sources.push(owners.filter_map(|s| async move { s.args().ok().filter(|a| a.new_owner().is_none()).map(|_| Event::HelperGone) }).boxed());
                watching.push("helper restarts");
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

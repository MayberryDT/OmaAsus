//! Session-bus control interface (`com.omaasus.App`) so hotkeys and scripts
//! can toggle the overlay or switch profiles: `omaasus toggle`.

use iced::futures::SinkExt;
use iced::futures::Stream;
use tokio::sync::mpsc;

pub const NAME: &str = "com.omaasus.App";
pub const PATH: &str = "/com/omaasus/App";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Show,
    Hide,
    Window,
    Profile(String),
    Page(String),
}

struct Service {
    tx: mpsc::Sender<Command>,
}

#[zbus::interface(name = "com.omaasus.App")]
impl Service {
    async fn toggle(&self) {
        let _ = self.tx.send(Command::Toggle).await;
    }
    async fn show(&self) {
        let _ = self.tx.send(Command::Show).await;
    }
    async fn hide(&self) {
        let _ = self.tx.send(Command::Hide).await;
    }
    async fn window(&self) {
        let _ = self.tx.send(Command::Window).await;
    }
    async fn apply_profile(&self, name: String) {
        let _ = self.tx.send(Command::Profile(name)).await;
    }
    async fn navigate(&self, page: String) {
        let _ = self.tx.send(Command::Page(page)).await;
    }
    #[zbus(property)]
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }
}

#[zbus::proxy(interface = "com.omaasus.App", default_service = "com.omaasus.App", default_path = "/com/omaasus/App")]
trait App {
    fn toggle(&self) -> zbus::Result<()>;
    fn show(&self) -> zbus::Result<()>;
    fn hide(&self) -> zbus::Result<()>;
    fn window(&self) -> zbus::Result<()>;
    fn apply_profile(&self, name: &str) -> zbus::Result<()>;
    fn navigate(&self, page: &str) -> zbus::Result<()>;
}

/// Subscription stream: owns the bus name for the lifetime of the app.
pub fn stream() -> impl Stream<Item = Command> {
    iced::stream::channel(8, async move |mut out| {
        let (tx, mut rx) = mpsc::channel(8);
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "session bus unavailable; IPC disabled");
                std::future::pending::<()>().await;
                unreachable!()
            }
        };
        if let Err(e) = conn.object_server().at(PATH, Service { tx }).await {
            tracing::error!(error = %e, "cannot register IPC object");
        }
        match conn.request_name(NAME).await {
            Ok(_) => tracing::info!("IPC ready on {NAME}"),
            Err(e) => tracing::warn!(error = %e, "another OmaAsus instance owns {NAME}; IPC disabled here"),
        }
        while let Some(cmd) = rx.recv().await {
            if out.send(cmd).await.is_err() {
                break;
            }
        }
        drop(conn);
    })
}

/// Client side: forward a CLI verb to the running instance.
pub async fn send(args: &[String]) -> anyhow::Result<()> {
    let conn = zbus::Connection::session().await?;
    let p = AppProxy::new(&conn).await?;
    let r = match args.first().map(String::as_str) {
        Some("toggle") => p.toggle().await,
        Some("show") => p.show().await,
        Some("hide") => p.hide().await,
        Some("window") => p.window().await,
        Some("profile") => p.apply_profile(args.get(1).map(String::as_str).unwrap_or("")).await,
        Some("page") => p.navigate(args.get(1).map(String::as_str).unwrap_or("")).await,
        _ => return Ok(()),
    };
    r.map_err(|e| anyhow::anyhow!("OmaAsus is not running ({e}). Start it with `omaasus --overlay`."))
}

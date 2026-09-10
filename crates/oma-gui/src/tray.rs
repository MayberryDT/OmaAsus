//! StatusNotifierItem for the bar (`org.kde.StatusNotifierItem`, served through
//! `ksni`). Omarchy's Quickshell bar hosts it in the tray on the right: a left
//! click drops the overlay panel down under the bar, a middle click opens the
//! full window, a right click shows the menu (window, panel, profiles, quit).
//!
//! The tray is what lets the daemon outlive its main window: with it present,
//! closing the window keeps automation, the fan engine and the overlay alive.

use iced::futures::{SinkExt, Stream};
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};
use ksni::{Category, Status, ToolTip, Tray, TrayMethods};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// What the user asked for through the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    /// Left click: drop the panel down (or fold it back up).
    ToggleOverlay,
    OpenWindow,
    Profile(uuid::Uuid),
    Quit,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// The item is registered with the watcher; the app keeps the handle to
    /// push state (profiles, open surfaces) into the menu.
    Ready(TrayHandle),
    Command(TrayCommand),
    /// The watcher came (true) or went (false). Without a host nobody can see
    /// the item, so the window must not close into thin air.
    Hosted(bool),
    /// No StatusNotifierWatcher answered: the bar has no tray, so closing the
    /// window must keep exiting the app.
    Unavailable,
}

/// Everything the tray reflects about the app. Compared after every update so
/// the menu is only rebuilt when something the user can see changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayState {
    pub profiles: Vec<(uuid::Uuid, String)>,
    pub active: Option<uuid::Uuid>,
    pub overlay_open: bool,
    pub window_open: bool,
    pub helper: bool,
}

#[derive(Clone)]
pub struct TrayHandle(pub Arc<ksni::Handle<OmaTray>>);

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrayHandle")
    }
}

impl TrayHandle {
    /// Push a fresh state into the item; the host is told about the changed
    /// properties and re-reads the menu.
    pub async fn sync(&self, state: TrayState) {
        self.0.update(|t| t.state = state).await;
    }

    pub async fn shutdown(&self) {
        self.0.shutdown().await;
    }
}

enum Outbound {
    Command(TrayCommand),
    Hosted(bool),
}

pub struct OmaTray {
    tx: mpsc::UnboundedSender<Outbound>,
    pub state: TrayState,
    icon_theme_path: String,
}

impl OmaTray {
    fn send(&self, c: TrayCommand) {
        let _ = self.tx.send(Outbound::Command(c));
    }

    fn active_name(&self) -> Option<&str> {
        let id = self.state.active?;
        self.state.profiles.iter().find(|(pid, _)| *pid == id).map(|(_, n)| n.as_str())
    }
}

impl Tray for OmaTray {
    fn id(&self) -> String {
        "omaasus".into()
    }

    fn title(&self) -> String {
        "OmaAsus".into()
    }

    fn category(&self) -> Category {
        Category::Hardware
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_name(&self) -> String {
        ICON_SYMBOLIC.into()
    }

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        let profile = self.active_name().unwrap_or("no profile");
        let helper = if self.state.helper { "helper connected" } else { "read-only" };
        ToolTip { icon_name: ICON_SYMBOLIC.into(), title: "OmaAsus".into(), description: format!("{profile} · {helper}\nClick for the panel, middle-click for the window"), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ToggleOverlay);
    }

    fn watcher_online(&self) {
        let _ = self.tx.send(Outbound::Hosted(true));
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        tracing::warn!(?reason, "StatusNotifier host went away; keeping the item for when it returns");
        let _ = self.tx.send(Outbound::Hosted(false));
        true
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::OpenWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let selected = self.state.active.and_then(|id| self.state.profiles.iter().position(|(pid, _)| *pid == id)).unwrap_or(0);
        let options = self.state.profiles.iter().map(|(_, name)| RadioItem { label: name.clone(), ..Default::default() }).collect::<Vec<_>>();
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: if self.state.window_open { "Focus OmaAsus".into() } else { "Open OmaAsus".into() },
                icon_name: ICON_SYMBOLIC.into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::OpenWindow)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: if self.state.overlay_open { "Hide panel".into() } else { "Show panel".into() },
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::ToggleOverlay)),
                ..Default::default()
            }
            .into(),
        ];
        if !options.is_empty() {
            items.push(MenuItem::Separator);
            items.push(
                RadioGroup {
                    selected,
                    select: Box::new(|t: &mut Self, i| {
                        if let Some((id, _)) = t.state.profiles.get(i) {
                            t.send(TrayCommand::Profile(*id));
                        }
                    }),
                    options,
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(StandardItem { label: "Quit OmaAsus".into(), activate: Box::new(|t: &mut Self| t.send(TrayCommand::Quit)), ..Default::default() }.into());
        items
    }
}

pub const ICON_SYMBOLIC: &str = "omaasus-symbolic";
pub const ICON_APP: &str = "omaasus";
const SVG_SYMBOLIC: &str = include_str!("../assets/icons/omaasus-symbolic.svg");
const SVG_APP: &str = include_str!("../assets/icons/omaasus.svg");

/// Where the user's icon theme lives (`$XDG_DATA_HOME/icons`).
pub fn user_icon_root() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.data_dir().join("icons"))
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).map(|c| c == content).unwrap_or(false) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// Make sure the tray and app icons resolve through the icon theme. A package
/// installs them under `/usr/share/icons`; a `cargo install` build drops user
/// copies into `$XDG_DATA_HOME/icons/hicolor` instead. Returns the directory
/// handed to the host as the item's extra icon search path.
pub fn install_icons() -> String {
    let system = Path::new("/usr/share/icons/hicolor/scalable/apps");
    if system.join(format!("{ICON_SYMBOLIC}.svg")).exists() {
        return String::new();
    }
    let Some(root) = user_icon_root() else { return String::new() };
    let apps = root.join("hicolor/scalable/apps");
    for (name, svg) in [(ICON_SYMBOLIC, SVG_SYMBOLIC), (ICON_APP, SVG_APP)] {
        match write_if_changed(&apps.join(format!("{name}.svg")), svg) {
            Ok(true) => tracing::info!(icon = name, dir = %apps.display(), "installed user icon"),
            Ok(false) => {}
            Err(e) => tracing::warn!(icon = name, error = %e, "cannot install user icon; the tray may show a placeholder"),
        }
    }
    root.to_string_lossy().into_owned()
}

/// Subscription stream: registers the item and forwards clicks.
pub fn stream() -> impl Stream<Item = Event> {
    iced::stream::channel(8, async move |mut out| {
        let icon_theme_path = tokio::task::spawn_blocking(install_icons).await.unwrap_or_default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let tray = OmaTray { tx, state: TrayState::default(), icon_theme_path };
        // The daemon may start before the shell: keep the item around and let it
        // register when a watcher shows up, instead of failing at boot.
        let handle = match tray.assume_sni_available(true).spawn().await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "no StatusNotifier tray; closing the window will exit the app");
                let _ = out.send(Event::Unavailable).await;
                std::future::pending::<()>().await;
                unreachable!()
            }
        };
        tracing::info!("tray item registered");
        let _ = out.send(Event::Ready(TrayHandle(Arc::new(handle)))).await;
        // Registration succeeded, so a watcher is there; if it was in fact
        // missing, the `Hosted(false)` queued by `watcher_offline` follows.
        let _ = out.send(Event::Hosted(true)).await;
        while let Some(msg) = rx.recv().await {
            let ev = match msg {
                Outbound::Command(c) => Event::Command(c),
                Outbound::Hosted(on) => Event::Hosted(on),
            };
            if out.send(ev).await.is_err() {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_well_formed_svg() {
        for svg in [SVG_SYMBOLIC, SVG_APP] {
            assert!(svg.contains("<svg"), "missing root element");
            assert!(svg.contains("viewBox=\"0 0 800 800\""), "mark paths are authored in the 800-unit space");
            assert_eq!(svg.matches("<path").count(), 3, "lobe, body and diamond");
        }
        assert!(SVG_SYMBOLIC.contains("width=\"16\""));
    }

    #[test]
    fn write_if_changed_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("omaasus-icons-{}", std::process::id()));
        let path = dir.join("a/b/icon.svg");
        assert!(write_if_changed(&path, "one").unwrap());
        assert!(!write_if_changed(&path, "one").unwrap());
        assert!(write_if_changed(&path, "two").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn menu_reflects_state() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let tray = OmaTray { tx, state: TrayState { profiles: vec![(a, "Silent".into()), (b, "Gaming".into())], active: Some(b), overlay_open: true, window_open: false, helper: true }, icon_theme_path: String::new() };
        let menu = tray.menu();
        // open, panel, separator, profiles, separator, quit
        assert_eq!(menu.len(), 6);
        match &menu[3] {
            MenuItem::RadioGroup(g) => {
                assert_eq!(g.selected, 1);
                assert_eq!(g.options.len(), 2);
            }
            _ => panic!("profiles should be a radio group"),
        }
        assert!(tray.tool_tip().description.starts_with("Gaming"));
    }
}

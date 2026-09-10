//! Hyprland IPC: read the active window and subscribe to compositor events
//! (fullscreen, window open/close, active window) via the `.socket2.sock`
//! event stream. Used by the automatic gaming-mode engine and by the overlay
//! to know when a game is in the foreground.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HyprEvent {
    ActiveWindow { class: String, title: String },
    Fullscreen(bool),
    OpenWindow { address: String, workspace: String, class: String, title: String },
    CloseWindow { address: String },
    Workspace(String),
    MonitorFocused(String),
    Other { name: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HyprClient {
    pub address: String,
    pub class: String,
    pub title: String,
    pub pid: i64,
    pub fullscreen: i64,
    pub workspace: i64,
    pub monitor: i64,
}

fn runtime_dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let base = std::env::var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp"));
    Some(base.join("hypr").join(sig))
}

pub fn available() -> bool {
    runtime_dir().map(|d| d.join(".socket2.sock").exists()).unwrap_or(false)
}

pub fn parse_event(line: &str) -> Option<HyprEvent> {
    let (name, data) = line.split_once(">>")?;
    Some(match name {
        "activewindow" => {
            let (class, title) = data.split_once(',').unwrap_or((data, ""));
            HyprEvent::ActiveWindow { class: class.into(), title: title.into() }
        }
        "fullscreen" => HyprEvent::Fullscreen(data.trim() == "1"),
        "openwindow" => {
            let mut it = data.splitn(4, ',');
            HyprEvent::OpenWindow {
                address: it.next().unwrap_or("").into(),
                workspace: it.next().unwrap_or("").into(),
                class: it.next().unwrap_or("").into(),
                title: it.next().unwrap_or("").into(),
            }
        }
        "closewindow" => HyprEvent::CloseWindow { address: data.into() },
        "workspace" => HyprEvent::Workspace(data.into()),
        "focusedmon" => HyprEvent::MonitorFocused(data.into()),
        // v2 duplicates and everything else are surfaced generically.
        _ => HyprEvent::Other { name: name.into(), data: data.into() },
    })
}

/// Stream Hyprland events into `tx` until the socket closes.
pub async fn subscribe(tx: tokio::sync::mpsc::Sender<HyprEvent>) -> anyhow::Result<()> {
    let dir = runtime_dir().ok_or_else(|| anyhow::anyhow!("not running under Hyprland"))?;
    let stream = UnixStream::connect(dir.join(".socket2.sock")).await?;
    let mut lines = BufReader::new(stream).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some(ev) = parse_event(&line) {
            if tx.send(ev).await.is_err() {
                break;
            }
        }
    }
    Ok(())
}

/// One-shot request on `.socket.sock` (same as `hyprctl -j <cmd>`).
pub async fn request_json(cmd: &str) -> anyhow::Result<serde_json::Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = runtime_dir().ok_or_else(|| anyhow::anyhow!("not running under Hyprland"))?;
    let mut s = UnixStream::connect(dir.join(".socket.sock")).await?;
    s.write_all(format!("j/{cmd}").as_bytes()).await?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

fn client_from(v: &serde_json::Value) -> HyprClient {
    let fullscreen = match &v["fullscreen"] {
        serde_json::Value::Bool(b) => *b as i64,
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    HyprClient {
        address: v["address"].as_str().unwrap_or("").into(),
        class: v["class"].as_str().unwrap_or("").into(),
        title: v["title"].as_str().unwrap_or("").into(),
        pid: v["pid"].as_i64().unwrap_or(0),
        fullscreen,
        workspace: v["workspace"]["id"].as_i64().unwrap_or(0),
        monitor: v["monitor"].as_i64().unwrap_or(0),
    }
}

pub async fn active_window() -> anyhow::Result<HyprClient> {
    Ok(client_from(&request_json("activewindow").await?))
}

pub async fn clients() -> anyhow::Result<Vec<HyprClient>> {
    Ok(request_json("clients").await?.as_array().map(|a| a.iter().map(client_from).collect()).unwrap_or_default())
}

/// Send a dispatcher. Hyprland ≥ 0.56 uses Lua dispatchers, e.g.
/// `dispatch("hl.dsp.window.float({ action = 'on' })")`; older releases take
/// the classic `name args` form. Pass the string appropriate for the running
/// compositor (see [`lua_ipc`]).
pub async fn dispatch(expr: &str) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = runtime_dir().ok_or_else(|| anyhow::anyhow!("not running under Hyprland"))?;
    let mut s = UnixStream::connect(dir.join(".socket.sock")).await?;
    s.write_all(format!("dispatch {expr}").as_bytes()).await?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).await?;
    Ok(buf)
}

/// Whether the compositor speaks the Lua IPC dialect (Hyprland ≥ 0.56).
pub async fn lua_ipc() -> bool {
    request_json("version").await.ok().and_then(|v| v["tag"].as_str().map(|t| t.trim_start_matches('v').split('.').take(2).filter_map(|n| n.parse::<u32>().ok()).collect::<Vec<_>>())).map(|v| v.len() == 2 && (v[0] > 0 || v[1] >= 56)).unwrap_or(true)
}

/// Heuristic: does the window look like a game? Combined with GameMode and
/// fullscreen state by the automation engine.
pub fn looks_like_game(c: &HyprClient) -> bool {
    let class = c.class.to_ascii_lowercase();
    let title = c.title.to_ascii_lowercase();
    if class.starts_with("steam_app_") || class.starts_with("steam_proton") {
        return true;
    }
    if matches!(class.as_str(), "gamescope" | "lutris" | "heroic" | "net.lutris.lutris") && c.fullscreen > 0 {
        return true;
    }
    let launchers = ["steam", "lutris", "heroic", "bottles", "com.usebottles.bottles", "net.lutris.lutris"];
    if launchers.contains(&class.as_str()) {
        return false;
    }
    c.fullscreen > 0 && !title.contains("youtube") && !class.contains("firefox") && !class.contains("chrom") && !class.contains("mpv") && !class.contains("vlc")
}

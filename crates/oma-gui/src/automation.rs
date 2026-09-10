//! Automatic mode: detection sources and the rule decision function.

use iced::futures::SinkExt;
use iced::futures::Stream;
use oma_hw::hypr::{self, HyprEvent};
use oma_hw::profile::{Config, Trigger};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum AutoEvent {
    Hypr(HyprEvent),
    ActiveWindow(hypr::HyprClient),
    GameMode(Option<i32>),
    Processes(Vec<String>),
}

#[derive(Debug, Default)]
pub struct AutoState {
    pub gamemode_clients: Option<i32>,
    pub window_class: String,
    pub window_title: String,
    pub fullscreen: bool,
    pub looks_like_game: bool,
    pub processes: Vec<String>,
    pub matched_rule: Option<String>,
    /// Rule id → when its condition first became true (for `for_s` triggers).
    pub since: HashMap<uuid::Uuid, Instant>,
    /// Last time any rule matched, and the hold to honour.
    pub last_match: Option<(Instant, u32)>,
}

impl AutoState {
    pub fn absorb(&mut self, ev: AutoEvent) {
        match ev {
            AutoEvent::Hypr(HyprEvent::Fullscreen(f)) => self.fullscreen = f,
            AutoEvent::Hypr(HyprEvent::ActiveWindow { class, title }) => {
                self.window_class = class;
                self.window_title = title;
            }
            AutoEvent::Hypr(_) => {}
            AutoEvent::ActiveWindow(c) => {
                self.fullscreen = c.fullscreen > 0;
                self.looks_like_game = hypr::looks_like_game(&c);
                self.window_class = c.class;
                self.window_title = c.title;
            }
            AutoEvent::GameMode(n) => self.gamemode_clients = n,
            AutoEvent::Processes(p) => self.processes = p,
        }
    }
}

fn glob_match(pattern: &str, s: &str) -> bool {
    let pat = pattern.to_ascii_lowercase();
    let s = s.to_ascii_lowercase();
    if pat.is_empty() {
        return false;
    }
    match (pat.strip_prefix('*'), pat.strip_suffix('*')) {
        (Some(p), _) if p.ends_with('*') => s.contains(p.trim_end_matches('*')),
        (Some(p), None) => s.ends_with(p),
        (None, Some(p)) => s.starts_with(p),
        _ => s == pat,
    }
}

/// Pick the profile that should be active, or `None` for "no change".
pub fn decide(cfg: &Config, st: &mut AutoState, snap: Option<&crate::telemetry::Snapshot>, now: Instant) -> Option<uuid::Uuid> {
    let cpu_t = snap.and_then(|s| s.cpu.tctl_c).unwrap_or(0.0);
    let gpu_t = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.temp_c)).unwrap_or(0) as f64;
    let local = chrono::Local::now();
    use chrono::Timelike;
    let minutes_now = local.hour() * 60 + local.minute();
    let mut best: Option<(&oma_hw::profile::Rule, i32)> = None;
    for r in cfg.rules.iter().filter(|r| r.enabled) {
        let raw = match &r.trigger {
            Trigger::GameMode => st.gamemode_clients.unwrap_or(0) > 0,
            Trigger::FullscreenGame => st.fullscreen && st.looks_like_game,
            Trigger::WindowClass(pat) => glob_match(pat, &st.window_class),
            Trigger::Process(name) => st.processes.iter().any(|p| glob_match(name, p)),
            Trigger::CpuHot { above_c, .. } => cpu_t >= *above_c,
            Trigger::GpuHot { above_c, .. } => gpu_t >= *above_c,
            Trigger::Time { from, to } => {
                let a = from.0 as u32 * 60 + from.1 as u32;
                let b = to.0 as u32 * 60 + to.1 as u32;
                if a <= b { (a..b).contains(&minutes_now) } else { minutes_now >= a || minutes_now < b }
            }
            Trigger::Idle { .. } => false,
        };
        let needs = match &r.trigger {
            Trigger::CpuHot { for_s, .. } | Trigger::GpuHot { for_s, .. } | Trigger::Idle { for_s } => *for_s,
            _ => 0,
        };
        let matched = if raw {
            let since = *st.since.entry(r.id).or_insert(now);
            now.duration_since(since) >= Duration::from_secs(needs as u64)
        } else {
            st.since.remove(&r.id);
            false
        };
        if matched && best.map(|(_, p)| r.priority > p).unwrap_or(true) {
            best = Some((r, r.priority));
        }
    }
    match best {
        Some((r, _)) => {
            st.matched_rule = Some(r.name.clone());
            st.last_match = Some((now, r.hold_s));
            Some(r.profile)
        }
        None => {
            st.matched_rule = None;
            match st.last_match {
                Some((t, hold)) if now.duration_since(t) < Duration::from_secs(hold as u64) => None,
                Some(_) => {
                    st.last_match = None;
                    Some(cfg.default_profile)
                }
                None => {
                    if cfg.active_profile != cfg.default_profile { Some(cfg.default_profile) } else { None }
                }
            }
        }
    }
}

fn process_names() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let name = e.file_name();
            if !name.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(c) = std::fs::read_to_string(e.path().join("comm")) {
                v.push(c.trim().to_string());
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

pub fn stream() -> impl Stream<Item = AutoEvent> {
    iced::stream::channel(32, async move |mut out| {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AutoEvent>(32);
        // Hyprland events
        if hypr::available() {
            let tx1 = tx.clone();
            tokio::spawn(async move {
                loop {
                    let (htx, mut hrx) = tokio::sync::mpsc::channel(32);
                    let fwd = tx1.clone();
                    let reader = tokio::spawn(async move {
                        while let Some(ev) = hrx.recv().await {
                            let refresh = matches!(ev, HyprEvent::ActiveWindow { .. } | HyprEvent::Fullscreen(_) | HyprEvent::CloseWindow { .. } | HyprEvent::Workspace(_));
                            let _ = fwd.send(AutoEvent::Hypr(ev)).await;
                            if refresh {
                                if let Ok(w) = hypr::active_window().await {
                                    let _ = fwd.send(AutoEvent::ActiveWindow(w)).await;
                                }
                            }
                        }
                    });
                    let _ = hypr::subscribe(htx).await;
                    reader.abort();
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
            if let Ok(w) = hypr::active_window().await {
                let _ = tx.send(AutoEvent::ActiveWindow(w)).await;
            }
        }
        // GameMode + processes polling
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let session = zbus::Connection::session().await.ok();
            let mut i: u32 = 0;
            loop {
                if let Some(c) = &session {
                    let n = oma_hw::gamemode::client_count(c).await;
                    let _ = tx2.send(AutoEvent::GameMode(n)).await;
                }
                if i % 3 == 0 {
                    let procs = tokio::task::spawn_blocking(process_names).await.unwrap_or_default();
                    let _ = tx2.send(AutoEvent::Processes(procs)).await;
                }
                i = i.wrapping_add(1);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        drop(tx);
        while let Some(ev) = rx.recv().await {
            if out.send(ev).await.is_err() {
                break;
            }
        }
    })
}

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

/// The temperatures rules may act on: the CPU's, and the GPU the machine is
/// using (an awake discrete card, else the integrated one), so AMD-only
/// machines have GPU rules too. Missing is missing: a rule on a temperature
/// nobody read does not fire.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Readings {
    pub cpu_c: Option<f64>,
    pub gpu_c: Option<f64>,
}

impl Readings {
    pub fn from_snapshot(snap: Option<&crate::telemetry::Snapshot>) -> Self {
        Self { cpu_c: snap.and_then(|s| s.cpu.tctl_c), gpu_c: snap.and_then(|s| s.gpu()).and_then(|g| g.temp_c) }
    }
}

/// Pick the profile that should be active, or `None` for "no change".
pub fn decide(cfg: &Config, st: &mut AutoState, snap: Option<&crate::telemetry::Snapshot>, now: Instant) -> Option<uuid::Uuid> {
    let local = chrono::Local::now();
    use chrono::Timelike;
    decide_at(cfg, st, Readings::from_snapshot(snap), now, local.hour() * 60 + local.minute())
}

/// `decide` with the clock passed in: `minutes_now` is the local time of day.
pub fn decide_at(cfg: &Config, st: &mut AutoState, temps: Readings, now: Instant, minutes_now: u32) -> Option<uuid::Uuid> {
    let mut best: Option<(&oma_hw::profile::Rule, i32)> = None;
    // A rule that names a profile that no longer exists can't be honoured.
    for r in cfg.rules.iter().filter(|r| r.enabled && cfg.profile(r.profile).is_some()) {
        let raw = match &r.trigger {
            Trigger::GameMode => st.gamemode_clients.unwrap_or(0) > 0,
            Trigger::FullscreenGame => st.fullscreen && st.looks_like_game,
            Trigger::WindowClass(pat) => glob_match(pat, &st.window_class),
            Trigger::Process(name) => st.processes.iter().any(|p| glob_match(name, p)),
            Trigger::CpuHot { above_c, .. } => temps.cpu_c.is_some_and(|t| t >= *above_c),
            Trigger::GpuHot { above_c, .. } => temps.gpu_c.is_some_and(|t| t >= *above_c),
            Trigger::Time { from, to } => {
                let a = from.0 as u32 * 60 + from.1 as u32;
                let b = to.0 as u32 * 60 + to.1 as u32;
                if a <= b { (a..b).contains(&minutes_now) } else { minutes_now >= a || minutes_now < b }
            }
            // No idle source exists yet: the rule is shown as unavailable and never fires.
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
                            if refresh
                                && let Ok(w) = hypr::active_window().await
                            {
                                let _ = fwd.send(AutoEvent::ActiveWindow(w)).await;
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
                if i.is_multiple_of(3) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use oma_hw::profile::{Profile, Rule};

    fn cfg_with(rules: Vec<Rule>) -> (Config, uuid::Uuid, uuid::Uuid) {
        let mut cfg = Config::empty();
        let quiet = Profile::new("Quiet");
        let hot = Profile::new("Cool down");
        let (q, h) = (quiet.id, hot.id);
        cfg.profiles = vec![quiet, hot];
        cfg.active_profile = q;
        cfg.default_profile = q;
        cfg.rules = rules;
        (cfg, q, h)
    }

    fn rule(name: &str, trigger: Trigger, profile: uuid::Uuid, priority: i32) -> Rule {
        Rule { id: uuid::Uuid::new_v4(), name: name.into(), enabled: true, trigger, profile, priority, hold_s: 0 }
    }

    #[test]
    fn a_gpu_rule_fires_on_an_amd_only_machine_and_not_without_a_reading() {
        let (mut cfg, _q, hot) = cfg_with(vec![]);
        cfg.rules = vec![rule("gpu", Trigger::GpuHot { above_c: 80.0, for_s: 0 }, hot, 10)];
        let mut st = AutoState::default();
        let now = Instant::now();
        // The integrated AMD GPU is the machine's GPU: its temperature counts.
        let amd = crate::telemetry::Snapshot { amd: Some(oma_hw::amdgpu::AmdGpuTelemetry { edge_c: Some(85.0), ..Default::default() }), amd_integrated: true, ..Default::default() };
        assert_eq!(Readings::from_snapshot(Some(&amd)).gpu_c, Some(85.0));
        assert_eq!(decide_at(&cfg, &mut st, Readings::from_snapshot(Some(&amd)), now, 600), Some(hot));
        // No GPU reading at all: the rule stops firing (the default comes back).
        assert_ne!(decide_at(&cfg, &mut st, Readings::default(), now, 600), Some(hot), "missing is not 0 °C, and not a trigger");
        assert_eq!(st.matched_rule, None);
    }

    #[test]
    fn rules_for_deleted_profiles_and_idle_never_fire() {
        let (mut cfg, _q, hot) = cfg_with(vec![]);
        cfg.rules = vec![rule("gone", Trigger::GameMode, uuid::Uuid::new_v4(), 100), rule("idle", Trigger::Idle { for_s: 0 }, hot, 50)];
        let mut st = AutoState { gamemode_clients: Some(1), ..Default::default() };
        assert_eq!(decide_at(&cfg, &mut st, Readings::default(), Instant::now(), 600), None);
        assert_eq!(st.matched_rule, None);
    }

    #[test]
    fn priority_then_hold_then_default() {
        let (mut cfg, quiet, hot) = cfg_with(vec![]);
        cfg.rules = vec![rule("game", Trigger::GameMode, hot, 10), rule("cpu", Trigger::CpuHot { above_c: 90.0, for_s: 0 }, quiet, 5)];
        let mut hold = cfg.rules[0].clone();
        hold.hold_s = 20;
        cfg.rules[0] = hold;
        let mut st = AutoState { gamemode_clients: Some(1), ..Default::default() };
        let t0 = Instant::now();
        let both = Readings { cpu_c: Some(95.0), gpu_c: None };
        assert_eq!(decide_at(&cfg, &mut st, both, t0, 600), Some(hot), "higher priority wins");
        assert_eq!(st.matched_rule.as_deref(), Some("game"));
        // Game over, CPU cool: held for hold_s, then back to the default.
        st.gamemode_clients = Some(0);
        assert_eq!(decide_at(&cfg, &mut st, Readings::default(), t0 + Duration::from_secs(5), 600), None, "within the hold");
        assert_eq!(decide_at(&cfg, &mut st, Readings::default(), t0 + Duration::from_secs(25), 600), Some(quiet), "hold over: default");
    }

    #[test]
    fn a_window_across_midnight_matches_late_and_early() {
        let (mut cfg, quiet, hot) = cfg_with(vec![]);
        cfg.rules = vec![rule("night", Trigger::Time { from: (22, 0), to: (6, 0) }, hot, 1)];
        let mut st = AutoState::default();
        let now = Instant::now();
        assert_eq!(decide_at(&cfg, &mut st, Readings::default(), now, 23 * 60), Some(hot));
        assert_eq!(decide_at(&cfg, &mut st, Readings::default(), now, 3 * 60), Some(hot));
        assert_eq!(decide_at(&cfg, &mut st, Readings::default(), now, 12 * 60), Some(quiet), "midday: back to the default");
    }

    #[test]
    fn a_for_s_trigger_needs_the_condition_to_last() {
        let (mut cfg, _q, hot) = cfg_with(vec![]);
        cfg.rules = vec![rule("cpu", Trigger::CpuHot { above_c: 90.0, for_s: 10 }, hot, 1)];
        let mut st = AutoState::default();
        let t0 = Instant::now();
        let hotc = Readings { cpu_c: Some(95.0), gpu_c: None };
        assert_eq!(decide_at(&cfg, &mut st, hotc, t0, 600), None);
        assert_eq!(decide_at(&cfg, &mut st, hotc, t0 + Duration::from_secs(11), 600), Some(hot));
        // A missing reading resets the timer (and ends the match: the default comes back).
        assert_ne!(decide_at(&cfg, &mut st, Readings::default(), t0 + Duration::from_secs(12), 600), Some(hot));
        assert_ne!(decide_at(&cfg, &mut st, hotc, t0 + Duration::from_secs(13), 600), Some(hot), "timer restarted");
    }
}

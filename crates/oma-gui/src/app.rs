//! Application state, message loop, and multi-surface routing
//! (overlay layer-shell panel + normal window from one process).

use crate::pages::cooling::CoolingMsg;
use crate::pages::cpu::CpuMsg;
use crate::pages::gpu::GpuMsg;
use crate::pages::lighting::LightingMsg;
use crate::pages::profiles::ProfilesMsg;
use crate::pages::settings::SettingsMsg;
use crate::pages::asus::{AsusMsg, AsusState};
use crate::pages::automation::AutomationMsg;
use crate::automation::{AutoEvent, AutoState};
use crate::pages::Page;
use crate::theme::{self, Palette, size, space};
use crate::widgets;
use crate::{ipc, telemetry};
use iced::widget::{column, container, row, Column};
use iced::window::Id;
use iced::{Background, Element, Length, Subscription, Task, Theme};
use iced_exwlshell::actions::IcedXdgWindowSettings;
use iced_exwlshell::daemon;
use iced_exwlshell::reexport::{Anchor, BlurOption, KeyboardInteractivity, Layer, LayerSize, NewLayerShellSettings, OutputOption, PixelSize};
use iced_exwlshell::settings::{LayerShellSettings, Settings, StartMode};
use iced_exwlshell::to_exwlshell_message;
use oma_hw::profile::{Config, FanMode, FanOwner, FanTarget};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub const HISTORY: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Overlay,
    Window,
}

#[derive(Default)]
pub struct History {
    pub cpu_load: VecDeque<f32>,
    pub cpu_temp: VecDeque<f32>,
    pub gpu_load: VecDeque<f32>,
    pub gpu_temp: VecDeque<f32>,
    pub gpu_power: VecDeque<f32>,
    pub coolant: VecDeque<f32>,
}

fn push(v: &mut VecDeque<f32>, x: f32) {
    if v.len() >= HISTORY {
        v.pop_front();
    }
    v.push_back(x);
}

pub struct App {
    pub palette: Palette,
    pub page: Page,
    pub config: Config,
    pub snapshot: Option<Arc<telemetry::Snapshot>>,
    pub hist: History,
    pub inventory: Option<Arc<oma_hw::SystemInventory>>,
    pub inventory_title: String,
    pub controller_ready: bool,
    pub surfaces: HashMap<Id, Surface>,
    pub overlay_only: bool,
    pub toast: Option<(String, bool)>,
    pub cpu_edit: oma_hw::cpu::CpuControlState,
    pub cpu_synced: bool,
    pub gpu_edit: oma_hw::nvidia::NvidiaControl,
    pub gpu_dirty: bool,
    pub nvidia_info: Option<Arc<oma_hw::nvidia::NvidiaInfo>>,
    pub cooling_sel: Option<FanTarget>,
    pub cc: Option<oma_hw::coolercontrol::CoolerControl>,
    pub cc_connected: bool,
    pub cc_modes: Vec<oma_hw::coolercontrol::CcMode>,
    pub fan_backend: Option<Arc<crate::fans::FanBackend>>,
    pub fan_engine: oma_hw::fanengine::FanEngine,
    pub fan_errors: u32,
    pub rgb_server: bool,
    pub rgb_devices: Vec<oma_hw::rgb::RgbDevice>,
    pub rgb_sel: usize,
    pub rgb_hex: String,
    pub rgb_thermal: bool,
    pub profile_sel: Option<uuid::Uuid>,
    pub auto_state: AutoState,
    pub helper_log: String,
    pub asus: AsusState,
}

#[to_exwlshell_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Telemetry(telemetry::Event),
    Ipc(ipc::Command),
    Inventory(Arc<oma_hw::SystemInventory>),
    Controller(bool),
    Navigate(Page),
    Cpu(CpuMsg),
    Gpu(GpuMsg),
    NvidiaInfo(Option<Arc<oma_hw::nvidia::NvidiaInfo>>),
    Cooling(CoolingMsg),
    CcReady(bool, Vec<oma_hw::coolercontrol::CcMode>),
    FanBackend(Arc<crate::fans::FanBackend>),
    FanResult(Result<(), String>),
    Lighting(LightingMsg),
    Profiles(ProfilesMsg),
    Automation(AutomationMsg),
    Auto(AutoEvent),
    Settings(SettingsMsg),
    Asus(AsusMsg),
    AsusLoaded(AsusState),
    HelperInstalled(Result<String, String>),
    RgbDevices(Result<Vec<oma_hw::rgb::RgbDevice>, String>),
    ApplyProfile(uuid::Uuid),
    Applied(Result<String, String>),
    ToggleOverlay,
    OpenWindow,
    DismissToastNoop,
    Close(Id),
    DismissToast,
}

pub fn run(overlay_only: bool) -> Result<(), iced_exwlshell::Error> {
    daemon(move || App::boot(overlay_only), App::namespace, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .style(App::style)
        .subscription(App::subscription)
        .font(theme::font::BODY_BYTES)
        .font(theme::font::DISPLAY_BYTES)
        .font(theme::font::MONO_BYTES)
        .default_font(theme::font::BODY)
        .settings(Settings {
            layer_settings: LayerShellSettings { start_mode: StartMode::Background, ..Default::default() },
            ..Default::default()
        })
        .run()
}

impl App {
    fn boot(overlay_only: bool) -> (Self, Task<Message>) {
        let config = crate::config_store::load();
        let accent = config.profile(config.active_profile).map(|p| (p.accent.r, p.accent.g, p.accent.b)).unwrap_or((255, 61, 104));
        let app = Self {
            palette: theme::OBSIDIAN.with_accent(accent),
            page: Page::Dashboard,
            config,
            snapshot: None,
            hist: History::default(),
            inventory: None,
            inventory_title: "detecting hardware…".into(),
            controller_ready: false,
            surfaces: HashMap::new(),
            overlay_only,
            toast: None,
            cpu_edit: oma_hw::cpu::control_state(),
            cpu_synced: false,
            gpu_edit: oma_hw::nvidia::NvidiaControl::default(),
            gpu_dirty: false,
            nvidia_info: None,
            cooling_sel: None,
            cc: None,
            cc_connected: false,
            cc_modes: Vec::new(),
            fan_backend: None,
            fan_engine: oma_hw::fanengine::FanEngine::new(std::time::Duration::from_millis(500)),
            fan_errors: 0,
            rgb_server: oma_hw::rgb::server_running(),
            rgb_devices: Vec::new(),
            rgb_sel: 0,
            rgb_hex: String::new(),
            rgb_thermal: false,
            profile_sel: None,
            auto_state: AutoState::default(),
            helper_log: String::new(),
            asus: AsusState::default(),
        };
        let inv = Task::perform(async { Arc::new(tokio::task::spawn_blocking(oma_hw::detect::inventory).await.expect("inventory")) }, Message::Inventory);
        let ctl = Task::perform(async { oma_hw::helper::Controller::connect().await.has_helper() }, Message::Controller);
        let nvi = Task::perform(async { tokio::task::spawn_blocking(|| oma_hw::nvidia::NvidiaGpu::open(0).and_then(|g| g.info()).ok().map(Arc::new)).await.unwrap_or(None) }, Message::NvidiaInfo);
        let open = if overlay_only { Task::none() } else { Task::done(Message::OpenWindow) };
        let cc_cfg = app.config.coolercontrol.clone();
        let cc = Task::perform(
            async move {
                let cc = oma_hw::coolercontrol::CoolerControl::new(&cc_cfg.url, cc_cfg.password.clone());
                if !cc.handshake().await {
                    return (false, Vec::new());
                }
                match cc.modes().await {
                    Ok(m) => (true, m),
                    Err(_) => (false, Vec::new()),
                }
            },
            |(ok, modes)| Message::CcReady(ok, modes),
        );
        let rgb = if app.rgb_server { Task::perform(async { oma_hw::rgb::devices().await.map_err(|e| e.to_string()) }, Message::RgbDevices) } else { Task::none() };
        let asus = Task::perform(crate::pages::asus::load(), Message::AsusLoaded);
        (app, Task::batch([inv, ctl, nvi, cc, rgb, asus, open]))
    }

    fn namespace() -> String {
        "omaasus".into()
    }

    fn title(&self, id: Id) -> Option<String> {
        Some(match self.surfaces.get(&id) {
            Some(Surface::Overlay) => "OmaAsus overlay".into(),
            _ => "OmaAsus".into(),
        })
    }

    fn theme(&self, _id: Id) -> Theme {
        theme::iced_theme(&self.palette)
    }

    fn style(&self, _t: &Theme) -> iced::theme::Style {
        iced::theme::Style { background_color: iced::Color::TRANSPARENT, text_color: self.palette.text }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(telemetry::stream).map(Message::Telemetry),
            Subscription::run(ipc::stream).map(Message::Ipc),
            Subscription::run(crate::automation::stream).map(Message::Auto),
        ])
    }

    pub fn active_profile(&self) -> Option<&oma_hw::Profile> {
        self.config.profile(self.config.active_profile)
    }

    pub fn effective_fan_owner(&self) -> FanOwner {
        let cc_running = self.inventory.as_ref().map(|i| i.daemons.coolercontrold).unwrap_or(false);
        match self.config.fan_owner {
            FanOwner::Auto => {
                if cc_running { FanOwner::CoolerControl } else { FanOwner::OmaAsus }
            }
            o => o,
        }
    }

    pub fn fan_engine_duty(&self, t: &FanTarget) -> Option<f64> {
        self.fan_engine.current(t)
    }

    fn cc_client(&self) -> oma_hw::coolercontrol::CoolerControl {
        self.cc.clone().unwrap_or_else(|| oma_hw::coolercontrol::CoolerControl::new(&self.config.coolercontrol.url, self.config.coolercontrol.password.clone()))
    }

    fn edit_cooling(&mut self, f: impl FnOnce(&mut oma_hw::profile::CoolingSettings)) {
        let id = self.config.active_profile;
        if let Some(pr) = self.config.profiles.iter_mut().find(|p| p.id == id) {
            f(&mut pr.cooling);
        }
        crate::config_store::save(&self.config);
    }

    fn selected_target(&self) -> Option<FanTarget> {
        self.cooling_sel.clone().or_else(|| {
            let inv = self.inventory.as_ref()?;
            let fans = self.snapshot.as_ref().map(|s| s.fans.clone()).unwrap_or_default();
            crate::fans::FanBackend::available(inv, &fans).first().map(|a| a.target.clone())
        })
    }

    fn update_cooling(&mut self, m: CoolingMsg) -> Task<Message> {
        let Some(target) = self.selected_target() else { return Task::none() };
        let default_source = match target {
            FanTarget::RyujinPump | FanTarget::RyujinExternalFans => oma_hw::profile::TempSource::Coolant,
            FanTarget::NvidiaFans => oma_hw::profile::TempSource::Gpu,
            FanTarget::LianLiChannel(_) | FanTarget::SuperIo(_) => oma_hw::profile::TempSource::CpuGpuMax,
            _ => oma_hw::profile::TempSource::CpuTctl,
        };
        match m {
            CoolingMsg::Select(t) => self.cooling_sel = Some(t),
            CoolingMsg::Owner(o) => {
                self.config.fan_owner = o;
                self.fan_engine.reset();
                crate::config_store::save(&self.config);
            }
            CoolingMsg::Mode(t, kind) => {
                let src = default_source.clone();
                self.edit_cooling(|c| {
                    let existing_curve = match c.get(&t) {
                        Some(FanMode::Curve(cv)) | Some(FanMode::HardwareCurve(cv)) => Some(cv.clone()),
                        _ => None,
                    };
                    let mode = match kind {
                        "fixed" => FanMode::Fixed(50.0),
                        "curve" => FanMode::Curve(existing_curve.clone().unwrap_or_else(|| crate::pages::cooling::preset("balanced", src.clone()))),
                        "hw" => FanMode::HardwareCurve(existing_curve.unwrap_or_else(|| crate::pages::cooling::preset("balanced", src))),
                        _ => FanMode::Auto,
                    };
                    c.set(t, mode);
                });
            }
            CoolingMsg::Fixed(v) => self.edit_cooling(|c| c.set(target, FanMode::Fixed(v))),
            CoolingMsg::Curve(ev) => {
                use crate::widgets::curve::CurveEvent;
                self.edit_cooling(|c| {
                    if let Some(FanMode::Curve(cv)) | Some(FanMode::HardwareCurve(cv)) = c.fans.iter_mut().find(|f| f.target == target).map(|f| &mut f.mode) {
                        match ev {
                            CurveEvent::Move(i, t, d) => {
                                if let Some(pt) = cv.points.get_mut(i) {
                                    *pt = (t, d);
                                }
                            }
                            CurveEvent::Add(t, d) => {
                                if cv.points.len() < 12 {
                                    cv.points.push((t, d));
                                    cv.points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                                }
                            }
                            CurveEvent::Remove(i) => {
                                if cv.points.len() > 2 && i < cv.points.len() {
                                    cv.points.remove(i);
                                }
                            }
                            CurveEvent::Commit => cv.points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap()),
                        }
                    }
                });
            }
            CoolingMsg::Source(src) => self.edit_cooling(|c| {
                if let Some(FanMode::Curve(cv)) | Some(FanMode::HardwareCurve(cv)) = c.fans.iter_mut().find(|f| f.target == target).map(|f| &mut f.mode) {
                    cv.source = src;
                }
            }),
            CoolingMsg::MinDuty(v) => self.edit_cooling(|c| {
                if let Some(FanMode::Curve(cv)) | Some(FanMode::HardwareCurve(cv)) = c.fans.iter_mut().find(|f| f.target == target).map(|f| &mut f.mode) {
                    cv.min_duty = v;
                }
            }),
            CoolingMsg::Ramp(v) => self.edit_cooling(|c| {
                if let Some(FanMode::Curve(cv)) | Some(FanMode::HardwareCurve(cv)) = c.fans.iter_mut().find(|f| f.target == target).map(|f| &mut f.mode) {
                    cv.ramp_s = v;
                }
            }),
            CoolingMsg::Hysteresis(v) => self.edit_cooling(|c| {
                if let Some(FanMode::Curve(cv)) | Some(FanMode::HardwareCurve(cv)) = c.fans.iter_mut().find(|f| f.target == target).map(|f| &mut f.mode) {
                    cv.hysteresis_c = v;
                }
            }),
            CoolingMsg::Preset(name) => {
                let src = default_source;
                self.edit_cooling(|c| {
                    let hw = matches!(c.get(&target), Some(FanMode::HardwareCurve(_)));
                    let mut cv = crate::pages::cooling::preset(name, src);
                    if let Some(FanMode::Curve(old)) | Some(FanMode::HardwareCurve(old)) = c.get(&target) {
                        if !matches!(name, "coolant" | "pump") {
                            cv.source = old.source.clone();
                        }
                    }
                    c.set(target, if hw { FanMode::HardwareCurve(cv) } else { FanMode::Curve(cv) });
                });
            }
            CoolingMsg::CcMode(uid) => {
                let id = self.config.active_profile;
                if let Some(pr) = self.config.profiles.iter_mut().find(|p| p.id == id) {
                    pr.cc_mode = if pr.cc_mode.as_deref() == Some(uid.as_str()) { None } else { Some(uid.clone()) };
                }
                crate::config_store::save(&self.config);
                let cc = self.cc_client();
                return Task::perform(async move { cc.activate_mode(&uid).await.map(|_| "CoolerControl mode activated".to_string()).map_err(|e| e.to_string()) }, Message::Applied);
            }
            CoolingMsg::CcRefresh => {
                let cc = self.cc_client();
                return Task::perform(async move { match cc.modes().await { Ok(m) => (true, m), Err(_) => (false, Vec::new()) } }, |(ok, m)| Message::CcReady(ok, m));
            }
        }
        Task::none()
    }

    fn update_asus(&mut self, m: AsusMsg) -> Task<Message> {
        use oma_hw::asusd::PlatformProxy;
        let reload = || Task::perform(crate::pages::asus::load(), Message::AsusLoaded);
        let run = |f: std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>| Task::perform(f, Message::Applied).chain(Task::perform(crate::pages::asus::load(), Message::AsusLoaded));
        match m {
            AsusMsg::Refresh => reload(),
            AsusMsg::Profile(pp) => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                PlatformProxy::new(&c).await.map_err(|e| e.to_string())?.set_platform_profile(pp as u32).await.map_err(|e| e.to_string())?;
                Ok(format!("Platform profile: {}", pp.label()))
            })),
            AsusMsg::NextProfile => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                PlatformProxy::new(&c).await.map_err(|e| e.to_string())?.next_platform_profile().await.map_err(|e| e.to_string())?;
                Ok("Platform profile cycled".into())
            })),
            AsusMsg::PptGroup(b) => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                PlatformProxy::new(&c).await.map_err(|e| e.to_string())?.set_enable_ppt_group(b).await.map_err(|e| e.to_string())?;
                Ok(format!("PPT group {}", if b { "enabled" } else { "disabled" }))
            })),
            AsusMsg::ChargeLimit(v) => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                PlatformProxy::new(&c).await.map_err(|e| e.to_string())?.set_charge_control_end_threshold(v as u8).await.map_err(|e| e.to_string())?;
                Ok(format!("Charge limit {v:.0}%"))
            })),
            AsusMsg::Attr(name, v) => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                oma_hw::asusd::set_armoury_attr(&c, &name, v as i32).await.map_err(|e| e.to_string())?;
                Ok(format!("{} = {v:.0}", oma_hw::asusd::attr_label(&name)))
            })),
            AsusMsg::AttrRestore(name) => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                let path = format!("/xyz/ljones/asus_armoury/{name}");
                oma_hw::asusd::AsusArmouryProxy::builder(&c).path(path).map_err(|e| e.to_string())?.build().await.map_err(|e| e.to_string())?.restore_default().await.map_err(|e| e.to_string())?;
                Ok(format!("{} restored", oma_hw::asusd::attr_label(&name)))
            })),
            AsusMsg::GfxMode(mode) => run(Box::pin(async move {
                let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                let action = oma_hw::supergfx::set_mode(&c, mode).await.map_err(|e| e.to_string())?;
                Ok(format!("Graphics → {}: {}", mode.label(), action.label()))
            })),
        }
    }

    fn update_settings(&mut self, m: SettingsMsg) -> Task<Message> {
        match m {
            SettingsMsg::InstallHelper => {
                return Task::perform(
                    async {
                        let (bin, data, script) = crate::install::locate().map_err(|e| e.to_string())?;
                        let out = tokio::process::Command::new("pkexec").arg("bash").arg(&script).arg(&bin).arg(&data).output().await.map_err(|e| e.to_string())?;
                        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                        if out.status.success() { Ok(text) } else { Err(text) }
                    },
                    Message::HelperInstalled,
                );
            }
            SettingsMsg::RecheckHelper => return Task::perform(async { oma_hw::helper::Controller::connect().await.has_helper() }, Message::Controller),
            SettingsMsg::CcUrl(u) => self.config.coolercontrol.url = u,
            SettingsMsg::CcPassword(pw) => self.config.coolercontrol.password = if pw.is_empty() { None } else { Some(pw) },
            SettingsMsg::CcTest => {
                crate::config_store::save(&self.config);
                self.cc = None;
                let cc = self.cc_client();
                return Task::perform(
                    async move {
                        if !cc.handshake().await {
                            return (false, Vec::new());
                        }
                        match cc.login().await.and(cc.modes().await.map_err(|e| e)) {
                            Ok(m) => (true, m),
                            Err(_) => (false, Vec::new()),
                        }
                    },
                    |(ok, m)| Message::CcReady(ok, m),
                );
            }
            SettingsMsg::OverlayAnchor(a) => self.config.overlay.anchor = a,
            SettingsMsg::OverlayWidth(w) => self.config.overlay.width = w as u32,
            SettingsMsg::OverlayHeight(h) => self.config.overlay.height = if h < 400.0 { 0 } else { h as u32 },
            SettingsMsg::OverlayMargin(v) => self.config.overlay.margin = v as u32,
            SettingsMsg::OverlayOpacity(v) => self.config.overlay.opacity = v as f32,
            SettingsMsg::HudToggle(b) => self.config.overlay.hud_enabled = b,
            SettingsMsg::TelemetryHz(v) => self.config.telemetry_hz = v as u32,
            SettingsMsg::OledText => {
                let snap = self.snapshot.clone();
                return Task::perform(
                    async move {
                        let path = oma_hw::livedash::hid_path().ok_or("LiveDash OLED not found")?;
                        let ctl = oma_hw::helper::Controller::connect().await;
                        let cpu = snap.as_ref().and_then(|s| s.cpu.tctl_c).unwrap_or(0.0);
                        let gpu = snap.as_ref().and_then(|s| s.nvidia.as_ref().and_then(|n| n.temp_c)).unwrap_or(0);
                        ctl.hid_write(&path, &oma_hw::livedash::mode_text()).await.map_err(|e| e.to_string())?;
                        ctl.hid_write(&path, &oma_hw::livedash::text("CPU / GPU", &format!("{cpu:.0}C  {gpu}C"))).await.map_err(|e| e.to_string())?;
                        Ok("OLED text sent".to_string())
                    },
                    Message::Applied,
                );
            }
            SettingsMsg::OledHwMonitor => {
                return Task::perform(
                    async move {
                        let path = oma_hw::livedash::hid_path().ok_or("LiveDash OLED not found")?;
                        let ctl = oma_hw::helper::Controller::connect().await;
                        ctl.hid_write(&path, &oma_hw::livedash::mode_hw_monitor()).await.map_err(|e| e.to_string())?;
                        Ok("OLED back to hardware monitor".to_string())
                    },
                    Message::Applied,
                );
            }
        }
        crate::config_store::save(&self.config);
        Task::none()
    }

    fn update_profiles(&mut self, m: ProfilesMsg) -> Task<Message> {
        match m {
            ProfilesMsg::Select(id) => self.profile_sel = Some(id),
            ProfilesMsg::Apply(id) => return Task::done(Message::ApplyProfile(id)),
            ProfilesMsg::New => {
                let mut pr = oma_hw::Profile::new("New profile");
                if let Some(base) = self.active_profile() {
                    pr.accent = base.accent;
                }
                self.profile_sel = Some(pr.id);
                self.config.profiles.push(pr);
                crate::config_store::save(&self.config);
            }
            ProfilesMsg::Duplicate(id) => {
                if let Some(src) = self.config.profile(id).cloned() {
                    let mut pr = src;
                    pr.id = uuid::Uuid::new_v4();
                    pr.name = format!("{} copy", pr.name);
                    pr.builtin = false;
                    self.profile_sel = Some(pr.id);
                    self.config.profiles.push(pr);
                    crate::config_store::save(&self.config);
                }
            }
            ProfilesMsg::Delete(id) => {
                if self.config.profiles.len() > 1 {
                    self.config.profiles.retain(|p| p.id != id);
                    let fallback = self.config.profiles[0].id;
                    if self.config.active_profile == id {
                        self.config.active_profile = fallback;
                    }
                    if self.config.default_profile == id {
                        self.config.default_profile = fallback;
                    }
                    self.config.rules.retain(|r| r.profile != id);
                    self.profile_sel = Some(fallback);
                    crate::config_store::save(&self.config);
                }
            }
            ProfilesMsg::SetDefault(id) => {
                self.config.default_profile = id;
                crate::config_store::save(&self.config);
            }
            ProfilesMsg::Rename(name) => {
                let id = self.profile_sel.unwrap_or(self.config.active_profile);
                if let Some(pr) = self.config.profiles.iter_mut().find(|p| p.id == id) {
                    pr.name = name;
                }
                crate::config_store::save(&self.config);
            }
            ProfilesMsg::Accent(c) => {
                let id = self.profile_sel.unwrap_or(self.config.active_profile);
                if let Some(pr) = self.config.profiles.iter_mut().find(|p| p.id == id) {
                    pr.accent = c;
                }
                if id == self.config.active_profile {
                    self.palette = theme::OBSIDIAN.with_accent((c.r, c.g, c.b));
                }
                crate::config_store::save(&self.config);
            }
        }
        Task::none()
    }

    fn update_automation(&mut self, m: AutomationMsg) -> Task<Message> {
        use oma_hw::profile::{Rule, Trigger};
        match m {
            AutomationMsg::Mode(mode) => {
                self.config.mode = mode;
                crate::config_store::save(&self.config);
                return self.auto_evaluate();
            }
            AutomationMsg::DefaultProfile(id) => self.config.default_profile = id,
            AutomationMsg::Toggle(id, on) => {
                if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    r.enabled = on;
                }
            }
            AutomationMsg::Delete(id) => self.config.rules.retain(|r| r.id != id),
            AutomationMsg::Add(kind) => {
                let gaming = self.config.profiles.iter().find(|p| p.name.eq_ignore_ascii_case("gaming")).map(|p| p.id).unwrap_or(self.config.active_profile);
                let silent = self.config.profiles.iter().find(|p| p.name.eq_ignore_ascii_case("silent")).map(|p| p.id).unwrap_or(self.config.default_profile);
                let (name, trigger, profile, prio) = match kind {
                    "gamemode" => ("GameMode active", Trigger::GameMode, gaming, 100),
                    "fullscreen" => ("Fullscreen game", Trigger::FullscreenGame, gaming, 50),
                    "class" => ("Window class", Trigger::WindowClass("steam_app_*".into()), gaming, 60),
                    "process" => ("Process", Trigger::Process("gamescope".into()), gaming, 60),
                    "cpuhot" => ("CPU hot", Trigger::CpuHot { above_c: 88.0, for_s: 20 }, silent, 150),
                    "gpuhot" => ("GPU hot", Trigger::GpuHot { above_c: 83.0, for_s: 20 }, silent, 150),
                    _ => ("Night hours", Trigger::Time { from: (23, 0), to: (7, 0) }, silent, 10),
                };
                self.config.rules.push(Rule { id: uuid::Uuid::new_v4(), name: name.into(), enabled: true, trigger, profile, priority: prio, hold_s: 20 });
            }
            AutomationMsg::SetProfile(id, pid) => {
                if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    r.profile = pid;
                }
            }
            AutomationMsg::Priority(id, v) => {
                if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    r.priority = v as i32;
                }
            }
            AutomationMsg::Hold(id, v) => {
                if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    r.hold_s = v as u32;
                }
            }
            AutomationMsg::Text(id, s) => {
                if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    match &mut r.trigger {
                        Trigger::WindowClass(v) | Trigger::Process(v) => *v = s,
                        _ => {}
                    }
                }
            }
            AutomationMsg::Threshold(id, v) => {
                if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    match &mut r.trigger {
                        Trigger::CpuHot { above_c, .. } | Trigger::GpuHot { above_c, .. } => *above_c = v,
                        _ => {}
                    }
                }
            }
        }
        crate::config_store::save(&self.config);
        Task::none()
    }

    /// Evaluate automation rules against the current state; may apply a profile.
    fn auto_evaluate(&mut self) -> Task<Message> {
        if self.config.mode != oma_hw::profile::Mode::Automatic {
            self.auto_state.matched_rule = None;
            return Task::none();
        }
        let snap = self.snapshot.clone();
        let now = std::time::Instant::now();
        let decision = crate::automation::decide(&self.config, &mut self.auto_state, snap.as_deref(), now);
        match decision {
            Some(target) if target != self.config.active_profile => {
                tracing::info!(?target, "automation switching profile");
                Task::done(Message::ApplyProfile(target))
            }
            _ => Task::none(),
        }
    }

    fn rgb_refresh() -> Task<Message> {
        Task::perform(async { oma_hw::rgb::devices().await.map_err(|e| e.to_string()) }, Message::RgbDevices)
    }

    fn update_lighting(&mut self, m: LightingMsg) -> Task<Message> {
        use oma_hw::profile::{LightingMode, Rgb};
        let sel = self.rgb_sel.min(self.rgb_devices.len().saturating_sub(1));
        let dev_index = self.rgb_devices.get(sel).map(|d| d.index);
        let dev_name = self.rgb_devices.get(sel).map(|d| d.name.clone());
        let remember = |app: &mut App, name: Option<String>, mode: LightingMode| {
            if let Some(n) = name {
                let id = app.config.active_profile;
                if let Some(pr) = app.config.profiles.iter_mut().find(|p| p.id == id) {
                    pr.lighting.zones.insert(n, mode);
                }
            }
        };
        match m {
            LightingMsg::Refresh => return Self::rgb_refresh(),
            LightingMsg::StartServer => {
                if let Err(e) = oma_hw::rgb::start_server() {
                    self.toast = Some((format!("Cannot start OpenRGB: {e}"), false));
                    return Task::none();
                }
                return Task::perform(
                    async {
                        for _ in 0..40 {
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                            if oma_hw::rgb::server_running() {
                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                return oma_hw::rgb::devices().await.map_err(|e| e.to_string());
                            }
                        }
                        Err("OpenRGB server did not come up".into())
                    },
                    Message::RgbDevices,
                );
            }
            LightingMsg::Select(i) => self.rgb_sel = i,
            LightingMsg::Hex(s) => {
                self.rgb_hex = s.clone();
                if let Some(c) = crate::pages::lighting::parse_hex(&s) {
                    return self.update_lighting(LightingMsg::Color(c));
                }
            }
            LightingMsg::Color(c) => {
                let Some(idx) = dev_index else { return Task::none() };
                remember(self, dev_name, LightingMode::Static(c));
                self.rgb_hex = c.hex();
                return Task::batch([Task::perform(async move { oma_hw::rgb::set_static(idx, (c.r, c.g, c.b)).await.map(|_| String::new()).map_err(|e| e.to_string()) }, |r| match r { Ok(_) => Message::Lighting(LightingMsg::Refresh), Err(e) => Message::Applied(Err(e)) })]);
            }
            LightingMsg::Mode(idx, mode) => {
                let name = self.rgb_devices.iter().find(|d| d.index == idx).map(|d| d.name.clone());
                let is_rainbow = self.rgb_devices.iter().find(|d| d.index == idx).and_then(|d| d.modes.get(mode)).map(|m| m.name.to_ascii_lowercase().contains("rainbow") || m.name.to_ascii_lowercase().contains("spectrum")).unwrap_or(false);
                if is_rainbow {
                    remember(self, name, LightingMode::Rainbow);
                }
                return Task::perform(async move { oma_hw::rgb::set_mode(idx, mode).await.map_err(|e| e.to_string()) }, |r| match r { Ok(_) => Message::Lighting(LightingMsg::Refresh), Err(e) => Message::Applied(Err(e)) });
            }
            LightingMsg::Off(idx) => {
                let name = self.rgb_devices.iter().find(|d| d.index == idx).map(|d| d.name.clone());
                remember(self, name, LightingMode::Off);
                return Task::perform(async move { oma_hw::rgb::turn_off(idx).await.map_err(|e| e.to_string()) }, |r| match r { Ok(_) => Message::Lighting(LightingMsg::Refresh), Err(e) => Message::Applied(Err(e)) });
            }
            LightingMsg::AllOff => {
                let idxs: Vec<usize> = self.rgb_devices.iter().map(|d| d.index).collect();
                let names: Vec<String> = self.rgb_devices.iter().map(|d| d.name.clone()).collect();
                for n in names {
                    remember(self, Some(n), LightingMode::Off);
                }
                return Task::perform(async move { for i in idxs { let _ = oma_hw::rgb::turn_off(i).await; } Ok::<_, String>(()) }, |_| Message::Lighting(LightingMsg::Refresh));
            }
            LightingMsg::SyncAccent => {
                let accent = self.active_profile().map(|p| p.accent).unwrap_or(Rgb::new(255, 61, 104));
                let idxs: Vec<usize> = self.rgb_devices.iter().map(|d| d.index).collect();
                let names: Vec<String> = self.rgb_devices.iter().map(|d| d.name.clone()).collect();
                for n in names {
                    remember(self, Some(n), LightingMode::Static(accent));
                }
                return Task::perform(async move { for i in idxs { let _ = oma_hw::rgb::set_static(i, (accent.r, accent.g, accent.b)).await; } Ok::<_, String>(()) }, |_| Message::Lighting(LightingMsg::Refresh));
            }
            LightingMsg::ThermalToggle => self.rgb_thermal = !self.rgb_thermal,
            LightingMsg::SaveToProfile => {
                crate::config_store::save(&self.config);
                self.toast = Some(("Lighting saved into the active profile".into(), true));
            }
        }
        Task::none()
    }

    /// Thermal glow: tint all RGB devices from cool→hot by the CPU/GPU max temperature.
    fn rgb_thermal_tick(&mut self, snap: &telemetry::Snapshot) -> Task<Message> {
        if !self.rgb_thermal || !self.rgb_server || snap.seq % 4 != 0 {
            return Task::none();
        }
        let t = snap.cpu.tctl_c.unwrap_or(0.0).max(snap.nvidia.as_ref().and_then(|n| n.temp_c).unwrap_or(0) as f64);
        let c = theme::thermal(&self.palette, t, 40.0, 90.0);
        let rgb = ((c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8);
        let idxs: Vec<usize> = self.rgb_devices.iter().map(|d| d.index).collect();
        Task::perform(async move { for i in idxs { let _ = oma_hw::rgb::set_static(i, rgb).await; } }, |_| Message::DismissToastNoop)
    }

    /// Run the software fan engine for one telemetry frame.
    fn fan_tick(&mut self, snap: &telemetry::Snapshot) -> Task<Message> {
        if self.effective_fan_owner() != FanOwner::OmaAsus {
            if self.fan_engine.current(&FanTarget::RyujinPump).is_some() || !self.fan_engine_is_idle() {
                // Owner changed away from us: release everything once.
                let cmds = self.fan_engine.evaluate(&Default::default(), &Default::default(), std::time::Instant::now());
                return self.dispatch_fan_cmds(cmds);
            }
            return Task::none();
        }
        let (Some(inv), Some(pr)) = (self.inventory.clone(), self.active_profile().cloned()) else { return Task::none() };
        let temps = crate::fans::temps_from(snap, &inv.hwmon);
        let cmds = self.fan_engine.evaluate(&pr.cooling, &temps, std::time::Instant::now());
        self.dispatch_fan_cmds(cmds)
    }

    fn fan_engine_is_idle(&self) -> bool {
        // cheap proxy: no known targets are being driven
        [FanTarget::RyujinPump, FanTarget::RyujinExternalFans, FanTarget::RyujinInternalFan, FanTarget::NvidiaFans, FanTarget::LianLiChannel(1), FanTarget::LianLiChannel(2), FanTarget::LianLiChannel(3), FanTarget::LianLiChannel(4)]
            .iter()
            .chain((1..=7).map(|i| FanTarget::SuperIo(i)).collect::<Vec<_>>().iter())
            .all(|t| self.fan_engine.current(t).is_none())
    }

    fn dispatch_fan_cmds(&mut self, cmds: Vec<oma_hw::fanengine::Command>) -> Task<Message> {
        if cmds.is_empty() {
            return Task::none();
        }
        let Some(be) = self.fan_backend.clone() else { return Task::none() };
        Task::batch(cmds.into_iter().map(move |c| {
            let be = be.clone();
            Task::perform(async move { be.apply(c).await }, Message::FanResult)
        }))
    }


    fn overlay_id(&self) -> Option<Id> {
        self.surfaces.iter().find(|(_, s)| **s == Surface::Overlay).map(|(id, _)| *id)
    }

    fn window_id(&self) -> Option<Id> {
        self.surfaces.iter().find(|(_, s)| **s == Surface::Window).map(|(id, _)| *id)
    }

    fn open_overlay(&mut self) -> Task<Message> {
        let o = &self.config.overlay;
        let anchor = match (o.anchor.as_str(), o.height == 0) {
            ("left", true) => Anchor::Left | Anchor::Top | Anchor::Bottom,
            ("left", false) => Anchor::Left | Anchor::Top,
            ("center", _) => Anchor::Top,
            (_, true) => Anchor::Right | Anchor::Top | Anchor::Bottom,
            (_, false) => Anchor::Right | Anchor::Top,
        };
        let (id, task) = Message::layershell_open(NewLayerShellSettings {
            size: if o.height == 0 { LayerSize::fill_height(o.width.max(1)) } else { LayerSize::px(o.width.max(1), o.height.max(1)) },
            layer: Layer::Overlay,
            anchor,
            margin: Some((o.margin as i32, o.margin as i32, o.margin as i32, o.margin as i32)),
            exclusive_zone: Some(0),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            output_option: OutputOption::LastOutput,
            blur_option: BlurOption::FullRegion,
            namespace: Some("omaasus-overlay".into()),
            ..Default::default()
        });
        self.surfaces.insert(id, Surface::Overlay);
        task
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Telemetry(telemetry::Event::Frame(snap)) => {
                push(&mut self.hist.cpu_load, snap.cpu.util_total as f32);
                push(&mut self.hist.cpu_temp, snap.cpu.tctl_c.unwrap_or(0.0) as f32);
                push(&mut self.hist.gpu_load, snap.nvidia.as_ref().and_then(|n| n.util_gpu).unwrap_or(0) as f32);
                push(&mut self.hist.gpu_temp, snap.nvidia.as_ref().and_then(|n| n.temp_c).unwrap_or(0) as f32);
                push(&mut self.hist.gpu_power, snap.nvidia.as_ref().and_then(|n| n.power_w).unwrap_or(0.0) as f32);
                push(&mut self.hist.coolant, snap.coolant_c.unwrap_or(0.0) as f32);
                if !self.cpu_synced {
                    self.cpu_edit = snap.cpu_control.clone();
                    self.cpu_synced = true;
                }
                if !self.gpu_dirty {
                    if let Some(n) = &snap.nvidia {
                        self.gpu_edit.gpc_offset_mhz = n.gpc_offset_mhz;
                        self.gpu_edit.mem_offset_mhz = n.mem_offset_mhz;
                        self.gpu_edit.persistence = n.persistence;
                        self.gpu_edit.power_limit_w = n.power_limit_w.map(|w| w.round() as u32);
                        self.gpu_edit.fan_percent = if n.fan_policy_manual.iter().any(|m| *m) { Some(n.fan_percent.clone()) } else { None };
                    }
                }
                let task = Task::batch([self.fan_tick(&snap), self.rgb_thermal_tick(&snap)]);
                self.snapshot = Some(snap);
                let auto = if self.snapshot.as_ref().map(|s| s.seq % 2 == 0).unwrap_or(false) { self.auto_evaluate() } else { Task::none() };
                Task::batch([task, auto])
            }
            Message::Inventory(inv) => {
                self.inventory_title = format!("{} · {}", inv.dmi.board_name, inv.cpu.model.split(" Processor").next().unwrap_or(&inv.cpu.model));
                self.inventory = Some(inv.clone());
                let cc = self.cc.clone();
                Task::perform(async move { Arc::new(crate::fans::FanBackend::build(inv, cc).await) }, Message::FanBackend)
            }
            Message::Controller(ok) => {
                self.controller_ready = ok;
                Task::none()
            }
            Message::Navigate(p) => {
                self.page = p;
                Task::none()
            }
            Message::NvidiaInfo(i) => {
                self.nvidia_info = i;
                Task::none()
            }
            Message::Cpu(m) => self.update_cpu(m),
            Message::Gpu(m) => self.update_gpu(m),
            Message::Cooling(m) => self.update_cooling(m),
            Message::Lighting(m) => self.update_lighting(m),
            Message::Profiles(m) => self.update_profiles(m),
            Message::Automation(m) => self.update_automation(m),
            Message::Settings(m) => self.update_settings(m),
            Message::AsusLoaded(st) => {
                self.asus = st;
                Task::none()
            }
            Message::Asus(m) => self.update_asus(m),
            Message::HelperInstalled(r) => {
                match r {
                    Ok(out) => {
                        self.helper_log = out;
                        self.toast = Some(("Helper installed".into(), true));
                    }
                    Err(e) => {
                        self.helper_log = e.clone();
                        self.toast = Some((format!("Helper install failed: {}", e.lines().last().unwrap_or("")), false));
                    }
                }
                Task::perform(async { oma_hw::helper::Controller::connect().await.has_helper() }, Message::Controller)
            }
            Message::Auto(ev) => {
                self.auto_state.absorb(ev);
                self.auto_evaluate()
            }
            Message::RgbDevices(r) => {
                match r {
                    Ok(d) => {
                        self.rgb_server = true;
                        self.rgb_devices = d;
                    }
                    Err(e) => {
                        self.rgb_server = oma_hw::rgb::server_running();
                        self.toast = Some((format!("OpenRGB: {e}"), false));
                    }
                }
                Task::none()
            }
            Message::CcReady(ok, modes) => {
                self.cc_connected = ok;
                self.cc_modes = modes;
                self.cc = Some(self.cc_client());
                Task::none()
            }
            Message::FanBackend(b) => {
                self.fan_backend = Some(b);
                Task::none()
            }
            Message::FanResult(r) => {
                if let Err(e) = r {
                    self.fan_errors += 1;
                    if self.fan_errors <= 3 || self.fan_errors % 60 == 0 {
                        self.toast = Some((format!("Fan control: {e}"), false));
                    }
                }
                Task::none()
            }
            Message::ApplyProfile(id) => {
                if let Some(pr) = self.config.profile(id).cloned() {
                    self.config.active_profile = id;
                    self.palette = theme::OBSIDIAN.with_accent((pr.accent.r, pr.accent.g, pr.accent.b));
                    crate::config_store::save(&self.config);
                    self.fan_engine.reset();
                    let inv = self.inventory.clone();
                    let cc = (self.effective_fan_owner() == FanOwner::CoolerControl).then(|| (self.cc_client(), pr.cc_mode.clone()));
                    return Task::perform(
                        async move {
                            let r = crate::apply::apply_profile(pr, inv).await;
                            if let Some((cc, Some(mode))) = cc {
                                if let Err(e) = cc.activate_mode(&mode).await {
                                    return Err(format!("{}; CoolerControl: {e}", r.unwrap_or_else(|e| e)));
                                }
                            }
                            r
                        },
                        Message::Applied,
                    );
                }
                Task::none()
            }
            Message::Applied(r) => {
                self.toast = Some(match r {
                    Ok(s) => (s, true),
                    Err(e) => (e, false),
                });
                Task::none()
            }
            Message::DismissToast => {
                self.toast = None;
                Task::none()
            }
            Message::DismissToastNoop => Task::none(),
            Message::Ipc(cmd) => match cmd {
                ipc::Command::Toggle => Task::done(Message::ToggleOverlay),
                ipc::Command::Show => {
                    if self.overlay_id().is_none() { self.open_overlay() } else { Task::none() }
                }
                ipc::Command::Hide => match self.overlay_id() {
                    Some(id) => Task::done(Message::Close(id)),
                    None => Task::none(),
                },
                ipc::Command::Window => Task::done(Message::OpenWindow),
                ipc::Command::Profile(name) => match self.config.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(&name)) {
                    Some(p) => Task::done(Message::ApplyProfile(p.id)),
                    None => Task::none(),
                },
                ipc::Command::Page(name) => match Page::ALL.iter().find(|p| p.label().eq_ignore_ascii_case(&name) || format!("{p:?}").eq_ignore_ascii_case(&name)) {
                    Some(p) => Task::done(Message::Navigate(*p)),
                    None => Task::none(),
                },
            },
            Message::ToggleOverlay => match self.overlay_id() {
                Some(id) => Task::done(Message::Close(id)),
                None => self.open_overlay(),
            },
            Message::OpenWindow => {
                if self.window_id().is_some() {
                    return Task::none();
                }
                let (id, task) = Message::base_window_open(IcedXdgWindowSettings { size: Some(PixelSize::px(1280, 820)), client_side_decorations: false });
                self.surfaces.insert(id, Surface::Window);
                task
            }
            Message::Close(id) => {
                let kind = self.surfaces.remove(&id);
                let task = Task::done(Message::RemoveWindow(id));
                if kind == Some(Surface::Window) && !self.overlay_only && self.overlay_id().is_none() {
                    // Closing the main window in window mode exits the app.
                    return Task::batch([task, iced::exit()]);
                }
                task
            }
            _ => Task::none(),
        }
    }

    fn update_cpu(&mut self, m: CpuMsg) -> Task<Message> {
        match m {
            CpuMsg::Governor(g) => self.cpu_edit.governor = g,
            CpuMsg::Epp(e) => self.cpu_edit.epp = Some(e),
            CpuMsg::Boost(b) => self.cpu_edit.boost = Some(b),
            CpuMsg::Smt(b) => self.cpu_edit.smt = Some(b),
            CpuMsg::MaxMhz(v) => self.cpu_edit.scaling_max_khz = (v * 1000.0) as u64,
            CpuMsg::MinMhz(v) => self.cpu_edit.scaling_min_khz = (v * 1000.0) as u64,
            CpuMsg::Revert => {
                self.cpu_synced = false;
            }
            CpuMsg::SaveToProfile => {
                let id = self.config.active_profile;
                let edit = self.cpu_edit.clone();
                if let Some(name) = self.config.profiles.iter_mut().find(|p| p.id == id).map(|pr| {
                    pr.cpu.control = Some(edit);
                    pr.name.clone()
                }) {
                    crate::config_store::save(&self.config);
                    self.toast = Some((format!("CPU settings saved into {name}"), true));
                }
            }
            CpuMsg::Apply => {
                let info = self.inventory.as_ref().map(|i| i.cpu.clone()).unwrap_or_else(oma_hw::cpu::cpu_info);
                let target = self.cpu_edit.clone();
                self.cpu_synced = false;
                return Task::perform(
                    async move {
                        let ctl = oma_hw::helper::Controller::connect().await;
                        let plan = oma_hw::cpu::plan_writes(&info, &target);
                        match ctl.write_batch(&plan).await {
                            Ok(errs) if errs.is_empty() => Ok("CPU settings applied".to_string()),
                            Ok(errs) => Err(format!("CPU: {}", errs[0].1)),
                            Err(e) => Err(format!("CPU: {e}")),
                        }
                    },
                    Message::Applied,
                );
            }
        }
        Task::none()
    }

    fn update_gpu(&mut self, m: GpuMsg) -> Task<Message> {
        let info = self.nvidia_info.clone();
        match m {
            GpuMsg::PowerLimit(w) => {
                self.gpu_edit.power_limit_w = Some(w.round() as u32);
                self.gpu_dirty = true;
            }
            GpuMsg::LockClocks(on) => {
                self.gpu_edit.locked_graphics_mhz = on.then(|| (210, info.as_ref().map(|i| i.max_graphics_mhz).unwrap_or(3000)));
                self.gpu_edit.unlock_clocks = !on;
                self.gpu_dirty = true;
            }
            GpuMsg::LockMax(v) => {
                if let Some((lo, _)) = self.gpu_edit.locked_graphics_mhz {
                    self.gpu_edit.locked_graphics_mhz = Some((lo, v.round() as u32));
                    self.gpu_dirty = true;
                }
            }
            GpuMsg::GpcOffset(v) => {
                self.gpu_edit.gpc_offset_mhz = Some(v.round() as i32);
                self.gpu_dirty = true;
            }
            GpuMsg::MemOffset(v) => {
                self.gpu_edit.mem_offset_mhz = Some(v.round() as i32);
                self.gpu_dirty = true;
            }
            GpuMsg::FanManual(on) => {
                self.gpu_edit.fan_percent = on.then(|| vec![50]);
                self.gpu_edit.fan_auto = !on;
                self.gpu_dirty = true;
            }
            GpuMsg::FanPercent(v) => {
                self.gpu_edit.fan_percent = Some(vec![v.round() as u32]);
                self.gpu_dirty = true;
            }
            GpuMsg::Persistence(b) => {
                self.gpu_edit.persistence = Some(b);
                self.gpu_dirty = true;
            }
            GpuMsg::Revert => {
                self.gpu_dirty = false;
            }
            GpuMsg::SaveToProfile => {
                let id = self.config.active_profile;
                let edit = self.gpu_edit.clone();
                if let Some(name) = self.config.profiles.iter_mut().find(|p| p.id == id).map(|pr| {
                    pr.gpu.nvidia = Some(edit);
                    pr.name.clone()
                }) {
                    crate::config_store::save(&self.config);
                    self.toast = Some((format!("GPU settings saved into {name}"), true));
                }
            }
            GpuMsg::AmdLevel(level) => {
                let gpus = self.inventory.as_ref().map(|i| i.amd_gpus.clone()).unwrap_or_default();
                return Task::perform(
                    async move {
                        let ctl = oma_hw::helper::Controller::connect().await;
                        for g in gpus {
                            ctl.write(g.perf_level_path(), &level).await.map_err(|e| e.to_string())?;
                        }
                        Ok(format!("AMD GPU performance level: {level}"))
                    },
                    Message::Applied,
                );
            }
            GpuMsg::Apply => {
                let ctl_state = self.gpu_edit.clone();
                self.gpu_dirty = false;
                return Task::perform(
                    async move {
                        let ctl = oma_hw::helper::Controller::connect().await;
                        match ctl.nvidia_apply(0, &ctl_state).await {
                            Ok(errs) if errs.is_empty() => Ok("GPU settings applied".to_string()),
                            Ok(errs) => Err(format!("GPU: {}", errs.iter().map(|(s, e)| format!("{s}: {e}")).collect::<Vec<_>>().join("; "))),
                            Err(e) => Err(format!("GPU: {e}")),
                        }
                    },
                    Message::Applied,
                );
            }
        }
        Task::none()
    }

    fn view(&self, id: Id) -> Element<'_, Message> {
        let p = self.palette;
        let is_overlay = self.surfaces.get(&id) == Some(&Surface::Overlay);
        let content: Element<Message> = match self.page {
            Page::Dashboard if is_overlay => crate::pages::dashboard::view_compact(self),
            Page::Dashboard => crate::pages::dashboard::view(self),
            Page::Cpu => crate::pages::cpu::view(self),
            Page::Gpu => crate::pages::gpu::view(self),
            Page::Cooling => crate::pages::cooling::view(self),
            Page::Lighting => crate::pages::lighting::view(self),
            Page::Profiles => crate::pages::profiles::view(self),
            Page::Automation => crate::pages::automation::view(self),
            Page::Settings => crate::pages::settings::view(self),
            Page::Asus => crate::pages::asus::view(self),
        };
        let show_asus = self.asus.asusd.is_some() || self.asus.gfx.is_some() || self.inventory.as_ref().map(|i| i.platform == oma_hw::Platform::AsusLaptop).unwrap_or(false);
        let pages: Vec<Page> = Page::ALL.iter().copied().filter(|pg| *pg != Page::Asus || show_asus).collect();
        let nav = Column::with_children(
            pages
                .iter()
                .map(|pg| {
                    let active = *pg == self.page;
                    let label = row![
                        container(iced::widget::text(pg.glyph()).size(size::LEAD).color(if active { p.accent } else { p.text_faint })).width(Length::Fixed(22.0)).align_x(iced::Alignment::Center),
                        iced::widget::text(pg.label()).size(size::BODY).font(theme::font::BODY),
                    ]
                    .spacing(space::MD)
                    .align_y(iced::Alignment::Center);
                    iced::widget::button(label).width(Length::Fill).padding([10, 14]).style(widgets::button_style(p, widgets::ButtonKind::Nav { active })).on_press(Message::Navigate(*pg)).into()
                })
                .collect::<Vec<_>>(),
        )
        .spacing(space::XS)
        .width(Length::Fixed(200.0));

        let brand = column![
            iced::widget::text("OMA").size(size::TITLE).font(theme::font::DISPLAY).color(p.text),
            iced::widget::text("ASUS control").size(size::CAPTION).font(theme::font::BODY).color(p.text_faint),
        ]
        .spacing(0.0);

        let toast: Element<Message> = match &self.toast {
            Some((msg, ok)) => container(row![widgets::body(p, msg), widgets::hfill(), widgets::btn(p, "✕", widgets::ButtonKind::Ghost, Some(Message::DismissToast))].align_y(iced::Alignment::Center))
                .padding([8, 12])
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(Background::Color(theme::alpha(if *ok { p.ok } else { p.danger }, 0.14))),
                    border: iced::Border { color: theme::alpha(if *ok { p.ok } else { p.danger }, 0.4), width: 1.0, radius: theme::radius::MD.into() },
                    ..Default::default()
                })
                .into(),
            None => iced::widget::Space::new().height(0.0).into(),
        };

        let shell: Element<Message> = if is_overlay {
            let tabs = row(pages.iter().map(|pg| {
                let active = *pg == self.page;
                iced::widget::button(iced::widget::text(pg.glyph()).size(size::LEAD)).padding([6, 10]).style(widgets::button_style(p, widgets::ButtonKind::Nav { active })).on_press(Message::Navigate(*pg)).into()
            }))
            .spacing(space::XS);
            column![
                row![brand, widgets::hfill(), widgets::btn(p, "Open window", widgets::ButtonKind::Ghost, Some(Message::OpenWindow)), widgets::btn(p, "✕", widgets::ButtonKind::Ghost, Some(Message::Close(id)))].spacing(space::SM).align_y(iced::Alignment::Center),
                tabs,
                toast,
                content
            ]
            .spacing(space::LG)
            .into()
        } else {
            row![
                column![brand, nav, widgets::vfill(), widgets::dim(p, format!("v{}", env!("CARGO_PKG_VERSION")))].spacing(space::XL).height(Length::Fill),
                column![toast, content].spacing(space::MD).width(Length::Fill),
            ]
            .spacing(space::XL)
            .into()
        };

        container(shell)
            .padding(space::XL)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(if is_overlay { iced::Color { a: self.config.overlay.opacity.clamp(0.5, 1.0), ..p.bg } } else { iced::Color { a: 1.0, ..p.bg } })),
                border: iced::Border { color: p.line, width: if is_overlay { 1.0 } else { 0.0 }, radius: if is_overlay { theme::radius::LG.into() } else { 0.0.into() } },
                ..Default::default()
            })
            .into()
    }
}

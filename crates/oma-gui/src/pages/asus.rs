//! ASUS laptop / ROG Ally page: asusd platform profile, Armoury firmware
//! attributes (PPT limits, MUX, panel), fan curves, and supergfxd graphics
//! mode. Shown only when `asusd` or `supergfxd` is present.

use crate::app::{App, Message};
use crate::pages::cpu::slider_style;
use crate::theme::{size, space};
use crate::widgets;
use iced::widget::{column, row, scrollable, slider, Column, Row};
use iced::{Element, Length};
use oma_hw::asusd::{attr_label, attr_unit, ArmouryAttr, PlatformProfile};
use oma_hw::model::FirmwareAttr;
use oma_hw::supergfx::{GfxMode, GfxState};

#[derive(Debug, Clone)]
pub enum AsusMsg {
    Refresh,
    Profile(PlatformProfile),
    NextProfile,
    Attr(String, f64),
    AttrRestore(String),
    PptGroup(bool),
    ChargeLimit(f64),
    GfxMode(GfxMode),
    KbdBrightness(u32),
    /// A firmware setting's slider moved; it is applied on release.
    AttrDrag(String, f64),
    AttrRelease,
    /// Go ahead with the graphics mode switch that is waiting to be confirmed.
    GfxConfirm,
    GfxCancel,
}

/// Keyboard backlight through asusd's Aura device.
#[derive(Debug, Clone)]
pub struct KbdLight {
    pub path: String,
    pub brightness: u32,
    pub levels: Vec<u32>,
}

/// A firmware setting as the page shows it: the live value, range and
/// writability from sysfs (asusd's copy is per power mode and charger state),
/// with the default, step and any queued value from asusd.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub current: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub step: i64,
    pub choices: Vec<i64>,
    pub default: Option<i64>,
    pub writable: bool,
    /// A GPU switch supergfxd runs through the graphics mode.
    pub graphics_switch: bool,
    /// A GPU value asusd holds until shutdown applies it.
    pub queued: Option<i64>,
}

impl Attr {
    fn new(name: &str, cached: Option<&ArmouryAttr>, live: Option<&FirmwareAttr>) -> Self {
        // asusd reports -1 for what it doesn't have.
        let known = |v: i32| (v >= 0).then_some(i64::from(v));
        Self {
            name: name.to_string(),
            current: live.and_then(|l| l.current).or_else(|| cached.and_then(|a| known(a.current))),
            min: live.and_then(|l| l.min).or_else(|| cached.and_then(|a| known(a.min))),
            max: live.and_then(|l| l.max).or_else(|| cached.and_then(|a| known(a.max))),
            step: cached.map(|a| i64::from(a.step)).filter(|s| *s > 0).unwrap_or(1),
            choices: live.map(|l| l.choices.clone()).filter(|c| !c.is_empty()).or_else(|| cached.map(|a| a.possible.iter().map(|v| i64::from(*v)).collect())).unwrap_or_default(),
            default: cached.and_then(|a| known(a.default)),
            // With no sysfs file to look at, asusd decides.
            writable: live.is_none_or(|l| l.writable),
            graphics_switch: false,
            queued: cached.and_then(|a| known(a.queued)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AsusState {
    pub asusd: Option<oma_hw::asusd::AsusdObjects>,
    pub profile: Option<PlatformProfile>,
    pub choices: Vec<PlatformProfile>,
    pub ppt_group: Option<bool>,
    pub charge_limit: Option<u8>,
    pub attrs: Vec<Attr>,
    pub gfx: Option<GfxState>,
    pub kbd: Option<KbdLight>,
    pub error: Option<String>,
}

pub async fn load() -> AsusState {
    let mut st = AsusState::default();
    let Ok(conn) = zbus::Connection::system().await else {
        st.error = Some("system bus unavailable".into());
        return st;
    };
    if let Ok(objs) = oma_hw::asusd::discover(&conn).await {
        if objs.has_platform {
            if let Ok(p) = oma_hw::asusd::PlatformProxy::new(&conn).await {
                st.profile = p.platform_profile().await.ok().map(PlatformProfile::from_u32);
                st.choices = p.platform_profile_choices().await.unwrap_or_default().into_iter().map(PlatformProfile::from_u32).collect();
                st.ppt_group = p.enable_ppt_group().await.ok();
                st.charge_limit = p.charge_control_end_threshold().await.ok();
            }
        }
        for a in &objs.armoury_attrs {
            let cached = oma_hw::asusd::armoury_attr(&conn, a).await.ok();
            let live = FirmwareAttr::read_live(a);
            if cached.is_some() || live.is_some() {
                st.attrs.push(Attr::new(a, cached.as_ref(), live.as_ref()));
            }
        }
        if let Some(path) = objs.aura_paths.first() {
            if let Ok(b) = oma_hw::asusd::AuraProxy::builder(&conn).path(path.as_str()) {
                if let Ok(a) = b.cache_properties(zbus::proxy::CacheProperties::No).build().await {
                    if let (Ok(brightness), Ok(levels)) = (a.brightness().await, a.supported_brightness().await) {
                        if !levels.is_empty() {
                            st.kbd = Some(KbdLight { path: path.clone(), brightness, levels });
                        }
                    }
                }
            }
        }
        st.asusd = Some(objs);
    }
    st.gfx = oma_hw::supergfx::state(&conn).await.ok();
    let switched_by_mode = st.gfx.is_some();
    for a in &mut st.attrs {
        a.graphics_switch = switched_by_mode && oma_hw::knowledge::is_gpu_switch(&a.name);
    }
    st
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let st = &app.asus;
    let header = row![widgets::headline(p, "ASUS platform"), widgets::hfill(), widgets::btn(p, "Refresh", widgets::ButtonKind::Ghost, Some(Message::Asus(AsusMsg::Refresh)))].align_y(iced::Alignment::Center);
    let mut col = Column::new().spacing(space::LG).push(header);

    match &st.asusd {
        None => {
            col = col.push(widgets::card(p, column![
                widgets::title(p, "asusd not running"),
                widgets::dim(p, "asusctl's daemon provides platform profiles, PPT tuning, MUX and keyboard lighting on ROG laptops and the ROG Ally. It does not start on desktop boards, where OmaAsus manages hardware directly."),
            ].spacing(space::SM)).width(Length::Fill));
        }
        Some(objs) => {
            let chips = Row::with_children(st.choices.iter().map(|c| widgets::btn(p, c.label(), if Some(*c) == st.profile { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Asus(AsusMsg::Profile(*c))))).collect::<Vec<_>>()).spacing(space::SM).wrap();
            let mut platform = column![
                row![widgets::title(p, "Platform profile"), widgets::hfill(), widgets::pill(p, format!("asusd {}", objs.version), p.ok)].align_y(iced::Alignment::Center),
                chips,
                row![
                    widgets::btn(p, "Cycle (Fn+F5)", widgets::ButtonKind::Ghost, Some(Message::Asus(AsusMsg::NextProfile))),
                    crate::pages::cpu::toggle(p, "Custom PPT group enabled", st.ppt_group.unwrap_or(false), st.ppt_group.is_some(), |b| Message::Asus(AsusMsg::PptGroup(b))),
                ]
                .spacing(space::LG)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(space::MD);
            if let Some(cl) = st.charge_limit {
                // Shares its drag state with the tray panel's slider; applied on release.
                let shown = app.quick_charge.unwrap_or(cl as f64);
                platform = platform.push(column![
                    row![widgets::eyebrow(p, "Battery charge limit"), widgets::hfill(), widgets::mono(p, format!("{shown:.0}%"), size::SMALL)],
                    slider(20.0..=100.0, shown, |v| Message::Quick(crate::pages::quick::QuickMsg::ChargeDrag(v))).on_release(Message::Quick(crate::pages::quick::QuickMsg::ChargeCommit)).step(5.0).style(slider_style(p)),
                ].spacing(space::XS));
            }
            col = col.push(widgets::card(p, platform).width(Length::Fill));

            if !st.attrs.is_empty() {
                let mut attrs = Column::new().spacing(space::MD).push(widgets::title(p, "Firmware settings"));
                for a in st.attrs.iter().filter(|a| !a.graphics_switch) {
                    attrs = attrs.push(attr_row(app, a));
                }
                if st.ppt_group.is_some() {
                    attrs = attrs.push(widgets::dim(p, "Power limits are kept per power mode and charger state, and reach the firmware while the custom PPT group is enabled."));
                }
                if st.attrs.iter().any(|a| a.graphics_switch) {
                    attrs = attrs.push(widgets::dim(p, "GPU switching (MUX, dGPU power) goes through the graphics mode below."));
                }
                col = col.push(widgets::card(p, attrs).width(Length::Fill));
            }
        }
    }

    match &st.gfx {
        None => col = col.push(widgets::card(p, column![widgets::title(p, "supergfxd not running"), widgets::dim(p, "Graphics mode switching (Hybrid / Integrated / VFIO / MUX) needs supergfxctl on hybrid laptops.")].spacing(space::SM)).width(Length::Fill)),
        Some(g) => {
            let safe = oma_hw::supergfx::is_safe_to_switch();
            let asking = app.gfx_confirm;
            let chips = Row::with_children(g.supported.iter().map(|m| {
                let allowed = safe || *m == GfxMode::Hybrid || *m == g.mode;
                let kind = if *m == g.mode || Some(*m) == asking { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost };
                widgets::btn(p, m.label(), kind, allowed.then_some(Message::Asus(AsusMsg::GfxMode(*m))))
            }).collect::<Vec<_>>()).spacing(space::SM).wrap();
            let power = g.power.map(|pw| format!(" · dGPU {}", power_label(pw))).unwrap_or_default();
            let mut card = column![
                row![widgets::title(p, "Graphics mode"), widgets::hfill(), widgets::pill(p, format!("supergfxd {}{power}", g.version), p.text_dim)].align_y(iced::Alignment::Center),
                chips,
            ]
            .spacing(space::MD);
            // Nothing switches until the user has read what it involves.
            if let Some(m) = asking {
                let question = column![
                    widgets::body(p, format!("Switch the graphics mode to {}?", m.label())),
                    widgets::dim(p, oma_hw::supergfx::switch_note(g.mode, m)),
                    row![
                        widgets::btn(p, format!("Switch to {}", m.label()), widgets::ButtonKind::Primary, Some(Message::Asus(AsusMsg::GfxConfirm))),
                        widgets::btn(p, "Cancel", widgets::ButtonKind::Ghost, Some(Message::Asus(AsusMsg::GfxCancel))),
                    ]
                    .spacing(space::SM),
                ]
                .spacing(space::SM);
                card = card.push(
                    iced::widget::container(question)
                        .padding(space::MD)
                        .width(Length::Fill)
                        .style(move |_| iced::widget::container::Style { border: iced::Border { color: p.warn, width: 1.0, radius: crate::theme::radius::SM.into() }, ..Default::default() }),
                );
            }
            if g.pending_mode != GfxMode::None {
                card = card.push(widgets::pill(p, format!("Pending: {} · {}", g.pending_mode.label(), g.pending_action.label()), p.warn));
            }
            if !safe {
                card = card.push(widgets::pill(p, "Desktop detected (no internal panel): only Hybrid is offered, Integrated would unbind your display GPU.", p.danger));
            }
            col = col.push(widgets::card(p, card).width(Length::Fill));
        }
    }
    if let Some(e) = &st.error {
        col = col.push(widgets::pill(p, e, p.danger));
    }
    scrollable(col).height(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(current: i32) -> ArmouryAttr {
        ArmouryAttr { attr: "ppt_pl1_spl".into(), name: String::new(), current, default: 80, min: 15, max: 80, step: 1, possible: Vec::new(), queued: -1 }
    }

    fn live(current: i64, writable: bool) -> FirmwareAttr {
        FirmwareAttr { name: "ppt_pl1_spl".into(), current: Some(current), min: Some(15), max: Some(80), choices: Vec::new(), writable, owned_by: None }
    }

    #[test]
    fn the_page_shows_what_the_firmware_runs() {
        // asusd keeps a copy per power mode and charger state (80 W) while
        // the firmware runs 35 W on battery: sysfs wins.
        let a = Attr::new("ppt_pl1_spl", Some(&cached(80)), Some(&live(35, true)));
        assert_eq!((a.current, a.min, a.max, a.default, a.queued), (Some(35), Some(15), Some(80), Some(80), None));
        assert!(a.writable);
        assert!(!Attr::new("nv_base_tgp", Some(&cached(80)), Some(&live(80, false))).writable, "read-only in sysfs stays read-only");
        // Without sysfs, asusd's values and word stand.
        let b = Attr::new("ppt_pl1_spl", Some(&cached(45)), None);
        assert_eq!((b.current, b.writable), (Some(45), true));
        assert_eq!(Attr::new("ppt_pl1_spl", Some(&cached(-1)), None).current, None, "-1 is asusd for unknown");
    }
}

fn power_label(p: oma_hw::supergfx::GfxPower) -> &'static str {
    use oma_hw::supergfx::GfxPower;
    match p {
        GfxPower::Active => "active",
        GfxPower::Suspended => "asleep",
        GfxPower::Off => "off",
        GfxPower::AsusDisabled => "disabled",
        GfxPower::AsusMuxDiscreet => "driving the display",
        GfxPower::Unknown => "unknown",
    }
}

/// One firmware setting: a slider, choices or a switch when it can be
/// changed, its value when it can't.
fn attr_row<'a>(app: &'a App, a: &'a Attr) -> Element<'a, Message> {
    let p = app.palette;
    let unit = attr_unit(&a.name);
    let fmt = move |v: i64| match unit {
        Some(u) => format!("{v} {u}"),
        None => v.to_string(),
    };
    let label = attr_label(&a.name);
    let queued = a.queued.filter(|q| Some(*q) != a.current).map(|q| widgets::pill(p, format!("{} after restart", fmt(q)), p.warn));
    let with_queued = |r: Row<'a, Message>| -> Element<'a, Message> {
        match queued {
            Some(q) => r.push(q).into(),
            None => r.into(),
        }
    };
    let name = a.name.clone();
    match (a.writable, a.current) {
        (true, Some(cur)) if a.choices == [0, 1] => with_queued(row![crate::pages::cpu::toggle(p, label, cur == 1, true, move |on| Message::Asus(AsusMsg::Attr(name.clone(), if on { 1.0 } else { 0.0 })))].spacing(space::SM).align_y(iced::Alignment::Center)),
        (true, Some(cur)) if !a.choices.is_empty() => {
            let opts = Row::with_children(a.choices.iter().map(|v| widgets::btn(p, fmt(*v), if *v == cur { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Asus(AsusMsg::Attr(name.clone(), *v as f64))))).collect::<Vec<_>>()).spacing(space::XS).wrap();
            with_queued(row![widgets::body(p, label), widgets::hfill(), opts].spacing(space::MD).align_y(iced::Alignment::Center))
        }
        (true, Some(cur)) => match (a.min, a.max) {
            (Some(lo), Some(hi)) if hi > lo => {
                let shown = app.attr_drag.as_ref().filter(|(n, _)| *n == a.name).map_or(cur as f64, |(_, v)| *v);
                let mut head = row![widgets::body(p, label), widgets::hfill(), widgets::mono(p, fmt(shown.round() as i64), size::SMALL)].spacing(space::SM).align_y(iced::Alignment::Center);
                if let Some(d) = a.default.filter(|d| *d != cur) {
                    head = head.push(widgets::btn(p, format!("Default {}", fmt(d)), widgets::ButtonKind::Ghost, Some(Message::Asus(AsusMsg::AttrRestore(name.clone())))));
                }
                let drag = name.clone();
                column![
                    with_queued(head),
                    slider(lo as f64..=hi as f64, shown, move |v| Message::Asus(AsusMsg::AttrDrag(drag.clone(), v))).step(a.step as f64).on_release(Message::Asus(AsusMsg::AttrRelease)).style(slider_style(p)),
                ]
                .spacing(space::XS)
                .into()
            }
            _ => with_queued(row![widgets::body(p, label), widgets::hfill(), widgets::mono(p, fmt(cur), size::SMALL)].spacing(space::SM).align_y(iced::Alignment::Center)),
        },
        (_, cur) => with_queued(row![widgets::body(p, label), widgets::hfill(), widgets::mono(p, cur.map_or_else(|| "—".to_string(), fmt), size::SMALL), widgets::pill(p, "read-only", p.text_dim)].spacing(space::SM).align_y(iced::Alignment::Center)),
    }
}

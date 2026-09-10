//! ASUS laptop / ROG Ally page: asusd platform profile, Armoury firmware
//! attributes (PPT limits, MUX, panel), fan curves, and supergfxd graphics
//! mode. Shown only when `asusd` or `supergfxd` is present.

use crate::app::{App, Message};
use crate::pages::cpu::slider_style;
use crate::theme::{size, space};
use crate::widgets;
use iced::widget::{column, row, scrollable, slider, Column, Row};
use iced::{Element, Length};
use oma_hw::asusd::{attr_label, ArmouryAttr, PlatformProfile};
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
}

/// Keyboard backlight through asusd's Aura device.
#[derive(Debug, Clone)]
pub struct KbdLight {
    pub path: String,
    pub brightness: u32,
    pub levels: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct AsusState {
    pub asusd: Option<oma_hw::asusd::AsusdObjects>,
    pub profile: Option<PlatformProfile>,
    pub choices: Vec<PlatformProfile>,
    pub ppt_group: Option<bool>,
    pub charge_limit: Option<u8>,
    pub attrs: Vec<ArmouryAttr>,
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
            if let Ok(x) = oma_hw::asusd::armoury_attr(&conn, a).await {
                st.attrs.push(x);
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
                platform = platform.push(column![
                    row![widgets::eyebrow(p, "Battery charge limit"), widgets::hfill(), widgets::mono(p, format!("{cl}%"), size::SMALL)],
                    slider(20.0..=100.0, cl as f64, |v| Message::Asus(AsusMsg::ChargeLimit(v))).step(5.0).style(slider_style(p)),
                ].spacing(space::XS));
            }
            col = col.push(widgets::card(p, platform).width(Length::Fill));

            if !st.attrs.is_empty() {
                let mut attrs = Column::new().spacing(space::MD).push(widgets::title(p, "Armoury firmware attributes"));
                for a in &st.attrs {
                    let name = a.attr.clone();
                    let el: Element<Message> = if !a.possible.is_empty() {
                        let opts = Row::with_children(a.possible.iter().map(|v| widgets::btn(p, v.to_string(), if *v == a.current { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Asus(AsusMsg::Attr(name.clone(), *v as f64))))).collect::<Vec<_>>()).spacing(space::XS).wrap();
                        row![widgets::body(p, attr_label(&a.attr)), widgets::hfill(), opts].spacing(space::MD).align_y(iced::Alignment::Center).into()
                    } else if a.min >= 0 && a.max > a.min {
                        let n2 = name.clone();
                        column![
                            row![widgets::body(p, attr_label(&a.attr)), widgets::hfill(), widgets::mono(p, format!("{} (default {}, {}..{})", a.current, a.default, a.min, a.max), size::SMALL), widgets::btn(p, "↺", widgets::ButtonKind::Ghost, Some(Message::Asus(AsusMsg::AttrRestore(n2))))].spacing(space::SM).align_y(iced::Alignment::Center),
                            slider(a.min as f64..=a.max as f64, a.current as f64, move |v| Message::Asus(AsusMsg::Attr(name.clone(), v))).step(a.step.max(1) as f64).style(slider_style(p)),
                        ]
                        .spacing(space::XS)
                        .into()
                    } else {
                        row![widgets::body(p, attr_label(&a.attr)), widgets::hfill(), widgets::mono(p, a.current.to_string(), size::SMALL), if a.queued >= 0 { widgets::pill(p, format!("queued {} · reboot", a.queued), p.warn) } else { iced::widget::Space::new().into() }].spacing(space::SM).align_y(iced::Alignment::Center).into()
                    };
                    attrs = attrs.push(el);
                }
                attrs = attrs.push(widgets::dim(p, "PPT values are stored per profile and AC/DC state and only reach the firmware while the custom PPT group is enabled. GPU MUX / dGPU changes are queued and applied at shutdown."));
                col = col.push(widgets::card(p, attrs).width(Length::Fill));
            }
        }
    }

    match &st.gfx {
        None => col = col.push(widgets::card(p, column![widgets::title(p, "supergfxd not running"), widgets::dim(p, "Graphics mode switching (Hybrid / Integrated / VFIO / MUX) needs supergfxctl on hybrid laptops.")].spacing(space::SM)).width(Length::Fill)),
        Some(g) => {
            let safe = oma_hw::supergfx::is_safe_to_switch();
            let chips = Row::with_children(g.supported.iter().map(|m| {
                let allowed = safe || *m == GfxMode::Hybrid || *m == g.mode;
                widgets::btn(p, m.label(), if *m == g.mode { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, allowed.then_some(Message::Asus(AsusMsg::GfxMode(*m))))
            }).collect::<Vec<_>>()).spacing(space::SM).wrap();
            col = col.push(widgets::card(p, column![
                row![widgets::title(p, "Graphics mode"), widgets::hfill(), widgets::pill(p, format!("supergfxd {} · {} · power {:?}", g.version, g.vendor, g.power), p.text_dim)].align_y(iced::Alignment::Center),
                chips,
                if g.pending_mode != GfxMode::None { widgets::pill(p, format!("pending: {} — {}", g.pending_mode.label(), g.pending_action.label()), p.warn) } else { iced::widget::Space::new().into() },
                if safe { widgets::dim(p, "Switching may log you out or require a reboot; the daemon tells you which.") } else { widgets::pill(p, "Desktop detected (no internal panel): only Hybrid is offered, Integrated would unbind your display GPU.", p.danger) },
            ].spacing(space::MD)).width(Length::Fill));
        }
    }
    if let Some(e) = &st.error {
        col = col.push(widgets::pill(p, e, p.danger));
    }
    scrollable(col).height(Length::Fill).into()
}

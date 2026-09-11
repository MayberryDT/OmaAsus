//! Profiles page: create, duplicate, rename, recolour, delete, set default;
//! summary of what each profile changes.

use crate::app::{App, Message};
use crate::pages::lighting::{swatch, SWATCHES};
use crate::theme::{self, size, space};
use crate::widgets;
use iced::widget::{column, row, scrollable, Column, Row};
use iced::{Element, Length};
use oma_hw::profile::{FanMode, Profile, Rgb};

#[derive(Debug, Clone)]
pub enum ProfilesMsg {
    Select(uuid::Uuid),
    New,
    Duplicate(uuid::Uuid),
    Delete(uuid::Uuid),
    SetDefault(uuid::Uuid),
    Rename(String),
    Accent(Rgb),
    Apply(uuid::Uuid),
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let sel = app.profile_sel.unwrap_or(app.config.active_profile);
    let list: Vec<Element<Message>> = app
        .config
        .profiles
        .iter()
        .map(|pr| {
            let active = pr.id == app.config.active_profile;
            let selected = pr.id == sel;
            let col = iced::Color::from_rgb8(pr.accent.r, pr.accent.g, pr.accent.b);
            let content = row![
                swatch(col, 14.0),
                column![widgets::body(p, &pr.name), widgets::dim(p, summary(pr))].spacing(2.0).width(Length::Fill),
                if active { widgets::pill(p, "active", p.ok) } else if pr.id == app.config.default_profile { widgets::pill(p, "default", p.text_dim) } else { iced::widget::Space::new().into() },
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center);
            iced::widget::button(content).width(Length::Fill).padding([8, 10]).style(widgets::button_style(p, widgets::ButtonKind::Nav { active: selected })).on_press(Message::Profiles(ProfilesMsg::Select(pr.id))).into()
        })
        .collect();
    let list_card = widgets::card(
        p,
        column![
            row![widgets::eyebrow(p, "Profiles"), widgets::hfill(), widgets::btn(p, "+ New", widgets::ButtonKind::Ghost, Some(Message::Profiles(ProfilesMsg::New)))].align_y(iced::Alignment::Center),
            scrollable(Column::with_children(list).spacing(space::XS)).height(Length::Fill),
        ]
        .spacing(space::MD)
        .height(Length::Fill),
    )
    .width(Length::Fixed(340.0))
    .height(Length::Fill);

    let editor: Element<Message> = match app.config.profile(sel) {
        None => widgets::card(p, widgets::dim(p, "Select a profile.")).height(Length::Fill).into(),
        Some(pr) => {
            let name = iced::widget::text_input("Profile name", &pr.name)
                .on_input(|s| Message::Profiles(ProfilesMsg::Rename(s)))
                .font(theme::font::DISPLAY)
                .size(size::TITLE)
                .style(move |_, _| iced::widget::text_input::Style {
                    background: iced::Background::Color(p.glass),
                    border: iced::Border { color: p.line_strong, width: 1.0, radius: theme::radius::SM.into() },
                    icon: p.text_dim,
                    placeholder: p.text_faint,
                    value: p.text,
                    selection: p.accent_soft,
                });
            let accents = Row::with_children(
                SWATCHES
                    .iter()
                    .map(|(_, c)| {
                        let col = iced::Color::from_rgb8(c.r, c.g, c.b);
                        let chosen = *c == pr.accent;
                        iced::widget::button(swatch(col, 24.0))
                            .padding(3)
                            .style(move |_, status| iced::widget::button::Style {
                                background: Some(iced::Background::Color(if chosen || matches!(status, iced::widget::button::Status::Hovered) { theme::alpha(col, 0.3) } else { iced::Color::TRANSPARENT })),
                                border: iced::Border { color: if chosen { col } else { iced::Color::TRANSPARENT }, width: 1.5, radius: 10.0.into() },
                                ..Default::default()
                            })
                            .on_press(Message::Profiles(ProfilesMsg::Accent(*c)))
                            .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(space::XS)
            .wrap();

            let detail = |label: &str, value: String| -> Element<Message> { row![widgets::dim(p, label.to_string()), widgets::hfill(), widgets::mono(p, value, size::SMALL)].align_y(iced::Alignment::Center).into() };
            let cpu = pr.cpu.control.as_ref().map(|c| format!("{}{}{}", c.governor, c.epp.as_ref().map(|e| format!(" · {e}")).unwrap_or_default(), c.boost.map(|b| if b { " · boost" } else { " · no boost" }).unwrap_or(""))).unwrap_or_else(|| "unchanged".into());
            let gpu = pr.gpu.nvidia.as_ref().map(|n| {
                let mut v = Vec::new();
                if let Some(w) = n.power_limit_w { v.push(format!("{w} W")); }
                if n.reset_power_limit { v.push("default power".into()); }
                if let Some((_, hi)) = n.locked_graphics_mhz { v.push(format!("lock ≤{hi} MHz")); }
                if let Some(o) = n.gpc_offset_mhz.filter(|o| *o != 0) { v.push(format!("core {o:+}")); }
                if let Some(o) = n.mem_offset_mhz.filter(|o| *o != 0) { v.push(format!("mem {o:+}")); }
                if n.persistence == Some(true) { v.push("persistence".into()); }
                if v.is_empty() { "unchanged".into() } else { v.join(" · ") }
            }).unwrap_or_else(|| "unchanged".into());
            let fans = pr.cooling.fans.iter().map(|f| format!("{}: {}", app.model.as_ref().and_then(|m| m.fan(f.target.as_str())).map(|o| o.label.clone()).unwrap_or_else(|| format!("{} (not on this machine)", f.target)), match &f.mode { FanMode::Auto => "auto".into(), FanMode::Fixed(d) => format!("{d:.0}%"), FanMode::Curve(c) => format!("curve on {}", c.source.label()), FanMode::HardwareCurve(c) => format!("hw curve on {}", c.source.label()) })).collect::<Vec<_>>();
            let fans_el: Element<Message> = if fans.is_empty() { widgets::dim(p, "no fan overrides") } else { Column::with_children(fans.into_iter().map(|s| widgets::mono(p, s, size::SMALL)).collect::<Vec<_>>()).spacing(2.0).into() };
            let actions = row![
                widgets::btn(p, "Activate", widgets::ButtonKind::Primary, Some(Message::Profiles(ProfilesMsg::Apply(pr.id)))),
                widgets::btn(p, "Duplicate", widgets::ButtonKind::Ghost, Some(Message::Profiles(ProfilesMsg::Duplicate(pr.id)))),
                widgets::btn(p, "Make default", widgets::ButtonKind::Ghost, (pr.id != app.config.default_profile).then_some(Message::Profiles(ProfilesMsg::SetDefault(pr.id)))),
                widgets::hfill(),
                widgets::btn(p, "Delete", widgets::ButtonKind::Danger, (!pr.builtin && app.config.profiles.len() > 1).then_some(Message::Profiles(ProfilesMsg::Delete(pr.id)))),
            ]
            .spacing(space::SM)
            .wrap();
            widgets::card(
                p,
                scrollable(column![
                    row![name, widgets::hfill(), if pr.builtin { widgets::pill(p, "built-in", p.text_dim) } else { iced::widget::Space::new().into() }].spacing(space::MD).align_y(iced::Alignment::Center),
                    widgets::eyebrow(p, "Accent"),
                    accents,
                    widgets::rule(p),
                    widgets::eyebrow(p, "What this profile changes"),
                    detail("Power mode", pr.cpu.power_mode.clone().unwrap_or_else(|| "unchanged".into())),
                    detail("CPU", cpu),
                    detail("GPU", gpu),
                    detail("CoolerControl mode", pr.cc_mode.clone().map(|m| app.cc_modes.iter().find(|x| x.uid == m).map(|x| x.name.clone()).unwrap_or(m)).unwrap_or_else(|| "none".into())),
                    detail("Lighting", format!("{} device(s)", pr.lighting.zones.len())),
                    row![widgets::dim(p, "Fans"), widgets::hfill(), fans_el].align_y(iced::Alignment::Start),
                    widgets::dim(p, "Edit CPU, GPU, cooling and lighting on their pages and use “Save into active profile”."),
                    widgets::rule(p),
                    actions,
                ]
                .spacing(space::MD))
                .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        }
    };

    column![widgets::headline(p, "Profiles"), row![list_card, editor].spacing(space::LG).height(Length::Fill)].spacing(space::LG).height(Length::Fill).into()
}

fn summary(pr: &Profile) -> String {
    let mut parts = Vec::new();
    if let Some(c) = &pr.cpu.control {
        parts.push(c.epp.clone().unwrap_or_else(|| c.governor.clone()));
    }
    if let Some(n) = &pr.gpu.nvidia
        && let Some(w) = n.power_limit_w
    {
        parts.push(format!("{w} W"));
    }
    if !pr.cooling.fans.is_empty() {
        parts.push(format!("{} fans", pr.cooling.fans.len()));
    }
    if parts.is_empty() { "no overrides".into() } else { parts.join(" · ") }
}

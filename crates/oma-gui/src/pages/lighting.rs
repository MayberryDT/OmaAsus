//! Lighting page: OpenRGB-backed control of every RGB device, profile accent
//! sync, and per-profile lighting memory.

use crate::app::{App, Message};
use crate::theme::{self, size, space};
use crate::widgets;
use iced::widget::{column, container, row, scrollable, Column, Row};
use iced::{Background, Border, Element, Length};
use oma_hw::profile::{LightingMode, Rgb};

#[derive(Debug, Clone)]
pub enum LightingMsg {
    Refresh,
    StartServer,
    Select(usize),
    Color(Rgb),
    Hex(String),
    Mode(usize, usize),
    Off(usize),
    SyncAccent,
    AllOff,
    SaveToProfile,
    ThermalToggle,
}

pub const SWATCHES: [(&str, Rgb); 12] = [
    ("ROG red", Rgb::new(255, 30, 60)),
    ("Coral", Rgb::new(255, 61, 104)),
    ("Amber", Rgb::new(255, 177, 61)),
    ("Lime", Rgb::new(120, 255, 90)),
    ("Mint", Rgb::new(61, 255, 177)),
    ("Cyan", Rgb::new(61, 214, 255)),
    ("Azure", Rgb::new(70, 130, 255)),
    ("Violet", Rgb::new(150, 90, 255)),
    ("Magenta", Rgb::new(255, 61, 220)),
    ("White", Rgb::new(255, 255, 255)),
    ("Warm", Rgb::new(255, 200, 150)),
    ("Ice", Rgb::new(190, 230, 255)),
];

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let status: Element<Message> = if app.rgb_server {
        row![widgets::pill(p, format!("OpenRGB · {} devices", app.rgb_devices.len()), p.ok), widgets::btn(p, "Rescan", widgets::ButtonKind::Ghost, Some(Message::Lighting(LightingMsg::Refresh)))].spacing(space::SM).align_y(iced::Alignment::Center).into()
    } else {
        row![widgets::pill(p, "OpenRGB server not running", p.warn), widgets::btn(p, "Start OpenRGB server", widgets::ButtonKind::Primary, Some(Message::Lighting(LightingMsg::StartServer)))].spacing(space::SM).align_y(iced::Alignment::Center).into()
    };
    let header = widgets::card(
        p,
        column![
            row![widgets::title(p, "Lighting"), widgets::hfill(), status].align_y(iced::Alignment::Center),
            widgets::dim(p, "Aura motherboard zones, DRAM, GPU and fan hubs are driven through the OpenRGB SDK. Colours here are saved into the active profile so each profile can carry its own look."),
            row![
                widgets::btn(p, "Sync everything to profile accent", widgets::ButtonKind::Primary, app.rgb_server.then_some(Message::Lighting(LightingMsg::SyncAccent))),
                widgets::btn(p, "All off", widgets::ButtonKind::Ghost, app.rgb_server.then_some(Message::Lighting(LightingMsg::AllOff))),
                widgets::btn(p, if app.rgb_thermal { "Thermal glow: on" } else { "Thermal glow: off" }, if app.rgb_thermal { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, app.rgb_server.then_some(Message::Lighting(LightingMsg::ThermalToggle))),
                widgets::hfill(),
                widgets::btn(p, "Save into active profile", widgets::ButtonKind::Ghost, Some(Message::Lighting(LightingMsg::SaveToProfile))),
            ]
            .spacing(space::SM)
            .wrap(),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let sel = app.rgb_sel.min(app.rgb_devices.len().saturating_sub(1));
    let list: Vec<Element<Message>> = app
        .rgb_devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let active = i == sel;
            let sw = d.colors.first().copied().unwrap_or((0, 0, 0));
            let content = row![
                swatch(iced::Color::from_rgb8(sw.0, sw.1, sw.2), 16.0),
                column![widgets::body(p, &d.name), widgets::dim(p, format!("{} · {} LEDs · {}", d.kind, d.leds, d.modes.get(d.active_mode).map(|m| m.name.as_str()).unwrap_or("?")))].spacing(2.0).width(Length::Fill),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center);
            iced::widget::button(content).width(Length::Fill).padding([8, 10]).style(widgets::button_style(p, widgets::ButtonKind::Nav { active })).on_press(Message::Lighting(LightingMsg::Select(i))).into()
        })
        .collect();
    let devices = widgets::card(p, column![widgets::eyebrow(p, "Devices"), if list.is_empty() { widgets::dim(p, if app.rgb_server { "No RGB devices reported." } else { "Start the server to enumerate devices." }) } else { Column::with_children(list).spacing(space::XS).into() }].spacing(space::MD)).width(Length::Fixed(320.0));

    let editor: Element<Message> = match app.rgb_devices.get(sel) {
        None => widgets::card(p, widgets::dim(p, "Select a device.")).into(),
        Some(d) => {
            let swatches = Row::with_children(
                SWATCHES
                    .iter()
                    .map(|(_name, c)| {
                        let col = iced::Color::from_rgb8(c.r, c.g, c.b);
                        iced::widget::button(swatch(col, 28.0))
                            .padding(3)
                            .style(move |_, status| iced::widget::button::Style {
                                background: Some(Background::Color(if matches!(status, iced::widget::button::Status::Hovered) { theme::alpha(col, 0.3) } else { iced::Color::TRANSPARENT })),
                                border: Border { radius: 10.0.into(), ..Default::default() },
                                ..Default::default()
                            })
                            .on_press(Message::Lighting(LightingMsg::Color(*c)))
                            .into()
                    })
                    .chain(std::iter::once(widgets::dim(p, "")))
                    .collect::<Vec<_>>(),
            )
            .spacing(space::XS)
            .wrap();
            let hex = iced::widget::text_input("#RRGGBB", &app.rgb_hex)
                .on_input(|s| Message::Lighting(LightingMsg::Hex(s)))
                .on_submit(Message::Lighting(LightingMsg::Hex(app.rgb_hex.clone())))
                .font(theme::font::MONO)
                .size(size::BODY)
                .width(Length::Fixed(120.0))
                .style(move |_, _| iced::widget::text_input::Style {
                    background: Background::Color(p.glass),
                    border: Border { color: p.line_strong, width: 1.0, radius: theme::radius::SM.into() },
                    icon: p.text_dim,
                    placeholder: p.text_faint,
                    value: p.text,
                    selection: p.accent_soft,
                });
            let modes = Row::with_children(
                d.modes
                    .iter()
                    .map(|m| widgets::btn(p, &m.name, if m.index == d.active_mode { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Lighting(LightingMsg::Mode(d.index, m.index)))))
                    .collect::<Vec<_>>(),
            )
            .spacing(space::XS)
            .wrap();
            let leds = Row::with_children(d.colors.iter().take(96).map(|c| swatch(iced::Color::from_rgb8(c.0, c.1, c.2), 12.0)).collect::<Vec<_>>()).spacing(3.0).wrap();
            widgets::card(
                p,
                column![
                    row![widgets::title(p, &d.name), widgets::hfill(), widgets::dim(p, format!("{} · {}", d.vendor, d.zones.iter().map(|(n, c)| format!("{n} ({c})")).collect::<Vec<_>>().join(", ")))].align_y(iced::Alignment::Center),
                    widgets::eyebrow(p, "Static colour"),
                    row![swatches, hex, widgets::btn(p, "Off", widgets::ButtonKind::Ghost, Some(Message::Lighting(LightingMsg::Off(d.index))))].spacing(space::MD).align_y(iced::Alignment::Center),
                    widgets::eyebrow(p, "Effects"),
                    modes,
                    widgets::eyebrow(p, "LEDs"),
                    leds,
                ]
                .spacing(space::MD),
            )
            .width(Length::Fill)
            .into()
        }
    };

    let profile_summary: Element<Message> = {
        let pr = app.active_profile();
        let zones = pr.map(|x| x.lighting.zones.len()).unwrap_or(0);
        widgets::card(
            p,
            column![
                widgets::eyebrow(p, "Profile lighting"),
                widgets::dim(p, format!("“{}” stores {} device colour(s). They are re-applied whenever the profile is activated.", pr.map(|x| x.name.as_str()).unwrap_or("—"), zones)),
                Column::with_children(pr.map(|x| x.lighting.zones.iter().map(|(name, mode)| {
                    let (label, col) = match mode {
                        LightingMode::Off => ("off".to_string(), p.text_faint),
                        LightingMode::Static(c) => (c.hex(), iced::Color::from_rgb8(c.r, c.g, c.b)),
                        LightingMode::Breathing(c) => (format!("breathing {}", c.hex()), iced::Color::from_rgb8(c.r, c.g, c.b)),
                        LightingMode::Rainbow => ("rainbow".into(), p.accent),
                        LightingMode::Thermal { .. } => ("thermal".into(), p.warn),
                        LightingMode::Direct(_) => ("custom".into(), p.text_dim),
                    };
                    row![widgets::body(p, name), widgets::hfill(), widgets::pill(p, label, col)].align_y(iced::Alignment::Center).into()
                }).collect::<Vec<_>>()).unwrap_or_default()).spacing(space::XS),
            ]
            .spacing(space::MD),
        )
        .width(Length::Fill)
        .into()
    };

    scrollable(column![header, row![devices, editor].spacing(space::LG), profile_summary].spacing(space::LG).padding(iced::Padding::from([0.0, space::XS]))).into()
}

pub fn swatch<'a, M: 'a>(c: iced::Color, size: f32) -> Element<'a, M> {
    container(iced::widget::Space::new().width(size).height(size))
        .style(move |_| container::Style {
            background: Some(Background::Color(c)),
            border: Border { color: iced::Color::from_rgba(1.0, 1.0, 1.0, 0.25), width: 1.0, radius: (size / 3.0).into() },
            shadow: iced::Shadow { color: theme::alpha(c, 0.5), offset: iced::Vector::ZERO, blur_radius: size / 2.0 },
            ..Default::default()
        })
        .into()
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Rgb::new((v >> 16) as u8, (v >> 8 & 0xff) as u8, (v & 0xff) as u8))
}

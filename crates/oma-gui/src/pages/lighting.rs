//! Lighting page: every lighting device this machine has, whether asusd
//! drives it (keyboard Aura, Slash bar) or OpenRGB does, remembered per profile.

use crate::app::{App, Message};
use crate::theme::{self, size, space, Palette};
use crate::widgets::{self, ButtonKind};
use iced::widget::{column, container, row, scrollable, slider, Column, Row};
use iced::{Background, Border, Element, Length};
use oma_hw::lighting::{self as light, LightState, SlashOption};
use oma_hw::model::{LightingBackend, LightingDevice};
use oma_hw::profile::{LightingMode, Rgb};
use oma_hw::rgb::RgbDevice;

#[derive(Debug, Clone)]
pub enum LightingMsg {
    /// Look again: OpenRGB server and devices, and the model's devices.
    Refresh,
    /// Read the model's devices again.
    Reload,
    Loaded(Vec<(String, LightState)>),
    StartServer,
    /// An OpenRGB device, by list position.
    Select(usize),
    /// A device from the model, by id.
    SelectDevice(String),
    Color(Rgb),
    Hex(String),
    /// The typed hex colour, once it has stayed the same for a moment.
    HexSettled(String),
    /// OpenRGB device index, mode index.
    Mode(usize, usize),
    Off(usize),
    /// Show a mode on a device from the model.
    Show(String, LightingMode),
    /// Brightness in percent for a device from the model.
    Level(String, u8),
    SlashDrag(u8),
    SlashRelease(String),
    SlashOption(String, SlashOption, bool),
    /// Stop the active profile setting this device.
    Forget(String),
    SyncAccent,
    AllOff,
    ThermalToggle,
}

pub const SWATCHES: [(&str, Rgb); 12] = [
    ("Red", Rgb::new(255, 30, 60)),
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

fn msg(m: LightingMsg) -> Message {
    Message::Lighting(m)
}

fn colour(c: Rgb) -> iced::Color {
    iced::Color::from_rgb8(c.r, c.g, c.b)
}

fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn model_lights(app: &App) -> &[LightingDevice] {
    app.model.as_ref().map(|m| m.lighting.as_slice()).unwrap_or(&[])
}

/// The model device picked on the page: the one selected, else the first when
/// there is no OpenRGB device to show instead.
pub fn selected_model(app: &App) -> Option<&LightingDevice> {
    let devices = model_lights(app);
    match &app.light_sel {
        Some(id) => devices.iter().find(|d| d.id.as_str() == id),
        None if app.rgb_devices.is_empty() => devices.first(),
        None => None,
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
    iced::widget::responsive(move |size| layout(app, size)).into()
}

fn layout(app: &App, size: iced::Size) -> Element<'_, Message> {
    let p = app.palette;
    let devices = model_lights(app);
    let profile = app.active_profile().map(|x| x.name.as_str()).unwrap_or("—");

    let mut status = Row::new().spacing(space::SM).align_y(iced::Alignment::Center);
    if !devices.is_empty() {
        status = status.push(widgets::pill(p, format!("asusd · {}", count(devices.len(), "device")), p.ok));
    }
    if app.rgb_server {
        status = status.push(widgets::pill(p, format!("OpenRGB · {}", count(app.rgb_devices.len(), "device")), p.ok)).push(widgets::btn(p, "Rescan", ButtonKind::Ghost, Some(msg(LightingMsg::Refresh))));
    } else if app.rgb_installed {
        status = status.push(widgets::btn(p, "Start OpenRGB", if devices.is_empty() { ButtonKind::Primary } else { ButtonKind::Ghost }, Some(msg(LightingMsg::StartServer))));
    }

    let mut reach = Vec::new();
    if !devices.is_empty() {
        reach.push(format!("{} through asusd", devices.iter().map(|d| d.label.as_str()).collect::<Vec<_>>().join(" and ")));
    }
    if !app.rgb_devices.is_empty() {
        reach.push(format!("{} through OpenRGB", count(app.rgb_devices.len(), "device")));
    }
    let about = match (reach.is_empty(), app.rgb_installed) {
        (false, _) => format!("{}. Changes are kept in “{profile}” and shown again whenever it's activated.", reach.join("; ")),
        (true, true) => "No lighting found yet. Start the OpenRGB server to reach motherboard, memory, GPU and fan-hub RGB.".into(),
        (true, false) => "No lighting found on this machine. Installing OpenRGB adds motherboard, memory, GPU and fan-hub RGB.".into(),
    };

    let colour_capable = devices.iter().any(|d| matches!(d.backend, LightingBackend::AsusdAura { .. }) && d.modes.contains(&0)) || !app.rgb_devices.is_empty();
    let any = !devices.is_empty() || !app.rgb_devices.is_empty();
    let mut actions = Row::new()
        .spacing(space::SM)
        .push(widgets::btn(p, "Match profile accent", ButtonKind::Primary, colour_capable.then_some(msg(LightingMsg::SyncAccent))))
        .push(widgets::btn(p, "All off", ButtonKind::Ghost, any.then_some(msg(LightingMsg::AllOff))));
    if !app.rgb_devices.is_empty() {
        actions = actions.push(widgets::btn(p, if app.rgb_thermal { "Thermal glow: on" } else { "Thermal glow: off" }, if app.rgb_thermal { ButtonKind::Primary } else { ButtonKind::Ghost }, Some(msg(LightingMsg::ThermalToggle))));
    }
    let header = widgets::card(p, column![row![widgets::title(p, "Lighting"), widgets::hfill(), status].align_y(iced::Alignment::Center), widgets::dim(p, about), actions.wrap()].spacing(space::MD)).width(Length::Fill);

    // Devices: the model's first, then OpenRGB's.
    let chosen = selected_model(app);
    let rgb_sel = app.rgb_sel.min(app.rgb_devices.len().saturating_sub(1));
    let both = !devices.is_empty() && !app.rgb_devices.is_empty();
    let mut list: Vec<Element<Message>> = Vec::new();
    if both {
        list.push(widgets::eyebrow(p, "asusd"));
    }
    for d in devices {
        let (swatch_colour, detail) = device_line(p, d, app.lights.get(d.id.as_str()));
        list.push(entry(p, swatch_colour, &d.label, detail, chosen.is_some_and(|c| c.id == d.id), msg(LightingMsg::SelectDevice(d.id.to_string()))));
    }
    if both {
        list.push(widgets::eyebrow(p, "OpenRGB"));
    }
    for (i, d) in app.rgb_devices.iter().enumerate() {
        let sw = d.colors.first().copied().unwrap_or((0, 0, 0));
        let detail = format!("{} · {} LEDs · {}", d.kind, d.leds, d.modes.get(d.active_mode).map(|m| m.name.as_str()).unwrap_or("?"));
        list.push(entry(p, iced::Color::from_rgb8(sw.0, sw.1, sw.2), &d.name, detail, chosen.is_none() && i == rgb_sel, msg(LightingMsg::Select(i))));
    }
    let editor: Element<Message> = match (chosen, app.rgb_devices.get(rgb_sel)) {
        (Some(d), _) => model_editor(app, d),
        (None, Some(d)) => rgb_editor(app, d),
        (None, None) => widgets::dim(p, if any { "Select a device." } else { "Lighting devices appear here when asusd or OpenRGB reports them." }),
    };

    if size.width >= 900.0 && size.height >= 620.0 {
        let device_list: Element<Message> = if list.is_empty() { widgets::dim(p, "Nothing to control yet.") } else { scrollable(container(Column::with_children(list).spacing(space::XS)).padding(iced::Padding { right: 14.0, ..iced::Padding::ZERO })).height(Length::Fill).into() };
        let device_card = widgets::card(p, column![widgets::eyebrow(p, "Devices"), device_list].spacing(space::MD).height(Length::Fill)).width(Length::Fixed(320.0)).height(Length::FillPortion(3));
        let editor = widgets::card(p, scrollable(editor).height(Length::Fill)).width(Length::Fill).height(Length::Fill);
        let summary = container(profile_summary(app, profile, true)).height(Length::FillPortion(2));
        let left = column![device_card, summary].spacing(space::LG).width(Length::Fixed(320.0)).height(Length::Fill);
        column![header, row![left, editor].spacing(space::LG).height(Length::Fill)].spacing(space::LG).height(Length::Fill).into()
    } else {
        // A narrow tile: one scrolling column, every card at its natural height.
        let device_list: Element<Message> = if list.is_empty() { widgets::dim(p, "Nothing to control yet.") } else { Column::with_children(list).spacing(space::XS).into() };
        let devices = widgets::card(p, column![widgets::eyebrow(p, "Devices"), device_list].spacing(space::MD)).width(Length::Fill);
        scrollable(column![header, devices, widgets::card(p, editor).width(Length::Fill), profile_summary(app, profile, false)].spacing(space::LG)).height(Length::Fill).into()
    }
}

/// Swatch colour and a one-line summary for a device from the model.
fn device_line(p: Palette, d: &LightingDevice, state: Option<&LightState>) -> (iced::Color, String) {
    let dark = theme::alpha(p.text_faint, 0.4);
    let Some(s) = state else { return (dark, "Not answering".into()) };
    match (light::mode_of(s), s) {
        (LightingMode::Off, _) => (dark, "Off".into()),
        (_, LightState::Aura { listed, colour: c, level }) => {
            let lit = if *listed <= 1 { colour(*c) } else { p.accent };
            let level = d.brightness_levels.iter().zip(light::level_names(&d.brightness_levels)).find(|(l, _)| **l == *level).map(|(_, n)| format!(" · {n}")).unwrap_or_default();
            (lit, format!("{}{level}", light::mode_name(d, *listed)))
        }
        (_, LightState::Slash { mode, .. }) => (p.accent, format!("{}{}", light::mode_name(d, u32::from(*mode)), d.leds.map(|n| format!(" · {n} LEDs")).unwrap_or_default())),
    }
}

fn entry<'a>(p: Palette, swatch_colour: iced::Color, name: &str, detail: String, active: bool, on: Message) -> Element<'a, Message> {
    let content = row![swatch(swatch_colour, 16.0), column![widgets::body(p, name), widgets::dim(p, detail)].spacing(2.0).width(Length::Fill)].spacing(space::SM).align_y(iced::Alignment::Center);
    iced::widget::button(content).width(Length::Fill).padding([8, 10]).style(widgets::button_style(p, ButtonKind::Nav { active })).on_press(on).into()
}

fn swatch_row<'a>(p: Palette) -> Element<'a, Message> {
    let _ = p;
    Row::with_children(
        SWATCHES
            .iter()
            .map(|(_name, c)| {
                let col = colour(*c);
                iced::widget::button(swatch(col, 28.0))
                    .padding(3)
                    .style(move |_, status| iced::widget::button::Style {
                        background: Some(Background::Color(if matches!(status, iced::widget::button::Status::Hovered) { theme::alpha(col, 0.3) } else { iced::Color::TRANSPARENT })),
                        border: Border { radius: 10.0.into(), ..Default::default() },
                        ..Default::default()
                    })
                    .on_press(msg(LightingMsg::Color(*c)))
                    .into()
            })
            .collect::<Vec<_>>(),
    )
    .spacing(space::XS)
    .wrap()
    .into()
}

fn hex_input(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    iced::widget::text_input("#RRGGBB", &app.rgb_hex)
        .on_input(|s| msg(LightingMsg::Hex(s)))
        .on_submit(msg(LightingMsg::Hex(app.rgb_hex.clone())))
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
        })
        .into()
}

fn kind(on: bool) -> ButtonKind {
    if on {
        ButtonKind::Primary
    } else {
        ButtonKind::Ghost
    }
}

/// Controls for a device from the model, built from what it reports.
fn model_editor<'a>(app: &'a App, d: &'a LightingDevice) -> Element<'a, Message> {
    let p = app.palette;
    let id = d.id.to_string();
    let (what, body): (String, Column<'a, Message>) = match (&d.backend, app.lights.get(d.id.as_str())) {
        (_, None) => ("asusd".into(), column![widgets::dim(p, "asusd isn't answering for this device."), widgets::btn(p, "Read again", ButtonKind::Ghost, Some(msg(LightingMsg::Reload)))].spacing(space::MD)),
        (LightingBackend::AsusdAura { .. }, Some(LightState::Aura { listed, colour: c, level })) => (format!("asusd Aura · {}", count(d.modes.len(), "effect")), aura_editor(app, d, id, *listed, *c, *level)),
        (LightingBackend::AsusdSlash { .. }, Some(s @ LightState::Slash { enabled, mode, options, .. })) => (format!("asusd{}", d.leds.map(|n| format!(" · {n} LEDs")).unwrap_or_default()), slash_editor(app, d, id, s, *enabled, *mode, options)),
        _ => ("asusd".into(), column![widgets::dim(p, "This device reported something unexpected.")]),
    };
    column![column![widgets::title(p, &d.label), widgets::dim(p, what)].spacing(2.0), body].spacing(space::MD).into()
}

fn aura_editor<'a>(app: &'a App, d: &'a LightingDevice, id: String, listed: u32, c: Rgb, level: u32) -> Column<'a, Message> {
    let p = app.palette;
    let on = level > 0;
    let mut col = Column::new().spacing(space::MD);
    if d.modes.contains(&0) || d.modes.contains(&1) {
        col = col.push(widgets::eyebrow(p, "Colour")).push(swatch_row(p)).push(hex_input(app));
    }
    let effect = |m: u32| match m {
        0 => LightingMode::Static(c),
        1 => LightingMode::Breathing(c),
        m => LightingMode::Firmware(m),
    };
    let effects = Row::with_children(
        d.modes
            .iter()
            .map(|m| widgets::btn(p, light::mode_name(d, *m), kind(on && listed == *m), Some(msg(LightingMsg::Show(id.clone(), effect(*m))))))
            .chain(std::iter::once(widgets::btn(p, "Off", kind(!on), Some(msg(LightingMsg::Show(id.clone(), LightingMode::Off))))))
            .collect::<Vec<_>>(),
    )
    .spacing(space::XS)
    .wrap();
    col = col.push(widgets::eyebrow(p, "Effect")).push(effects);
    if !d.brightness_levels.is_empty() {
        let levels = Row::with_children(
            d.brightness_levels
                .iter()
                .zip(light::level_names(&d.brightness_levels))
                .map(|(l, name)| widgets::btn(p, name, kind(*l == level), Some(msg(LightingMsg::Level(id.clone(), light::aura_percent(&d.brightness_levels, *l))))))
                .collect::<Vec<_>>(),
        )
        .spacing(space::XS)
        .wrap();
        col = col.push(widgets::eyebrow(p, "Brightness")).push(levels);
    }
    if d.modes.iter().any(|m| *m > 3) {
        col = col.push(widgets::dim(p, "Effects with no colour of their own use the profile accent."));
    }
    col
}

fn slash_editor<'a>(app: &'a App, d: &'a LightingDevice, id: String, s: &LightState, enabled: bool, mode: u8, options: &'a [(SlashOption, bool)]) -> Column<'a, Message> {
    let p = app.palette;
    let pct = app.slash_drag.unwrap_or_else(|| light::percent_of(d, s));
    let toggle_id = id.clone();
    let mut col = Column::new().spacing(space::MD).push(crate::pages::cpu::toggle(p, "Lit", enabled, true, move |b| msg(LightingMsg::Show(toggle_id.clone(), if b { LightingMode::Firmware(u32::from(mode)) } else { LightingMode::Off }))));
    let animations = Row::with_children(d.modes.iter().map(|m| widgets::btn(p, light::mode_name(d, *m), kind(enabled && u32::from(mode) == *m), Some(msg(LightingMsg::Show(id.clone(), LightingMode::Firmware(*m)))))).collect::<Vec<_>>()).spacing(space::XS).wrap();
    col = col.push(widgets::eyebrow(p, "Animation")).push(animations);
    let release_id = id.clone();
    col = col.push(widgets::eyebrow(p, "Brightness")).push(
        row![
            slider(1.0..=100.0, f64::from(pct.max(1)), |v| msg(LightingMsg::SlashDrag(v.round() as u8))).step(1.0).on_release(msg(LightingMsg::SlashRelease(release_id))).style(crate::pages::cpu::slider_style(p)),
            widgets::mono(p, format!("{pct:>3}%"), size::BODY),
        ]
        .spacing(space::MD)
        .align_y(iced::Alignment::Center),
    );
    if !options.is_empty() {
        let toggles = Row::with_children(
            options
                .iter()
                .map(|(o, v)| {
                    let (id, o) = (id.clone(), *o);
                    crate::pages::cpu::toggle(p, o.label(), *v, true, move |b| msg(LightingMsg::SlashOption(id.clone(), o, b)))
                })
                .collect::<Vec<_>>(),
        )
        .spacing(space::LG)
        .wrap();
        col = col.push(widgets::eyebrow(p, "Also show it")).push(toggles);
    }
    col
}

fn rgb_editor<'a>(app: &'a App, d: &'a RgbDevice) -> Element<'a, Message> {
    let p = app.palette;
    let modes = Row::with_children(d.modes.iter().map(|m| widgets::btn(p, &m.name, kind(m.index == d.active_mode), Some(msg(LightingMsg::Mode(d.index, m.index))))).collect::<Vec<_>>()).spacing(space::XS).wrap();
    let leds = Row::with_children(d.colors.iter().take(96).map(|c| swatch(iced::Color::from_rgb8(c.0, c.1, c.2), 12.0)).collect::<Vec<_>>()).spacing(3.0).wrap();
    column![
        column![widgets::title(p, &d.name), widgets::dim(p, format!("{} · {}", d.vendor, d.zones.iter().map(|(n, c)| format!("{n} ({c})")).collect::<Vec<_>>().join(", ")))].spacing(2.0),
        widgets::eyebrow(p, "Static colour"),
        swatch_row(p),
        row![hex_input(app), widgets::btn(p, "Off", ButtonKind::Ghost, Some(msg(LightingMsg::Off(d.index))))].spacing(space::MD).align_y(iced::Alignment::Center),
        widgets::eyebrow(p, "Effects"),
        modes,
        widgets::eyebrow(p, "LEDs"),
        leds,
    ]
    .spacing(space::MD)
    .into()
}

/// What the active profile shows on each device.
/// `fill`: take the height given and scroll the list inside (the wide
/// layout); otherwise the natural height (inside the narrow tile's scroll).
fn profile_summary<'a>(app: &'a App, profile: &str, fill: bool) -> Element<'a, Message> {
    let p = app.palette;
    let devices = model_lights(app);
    let rows: Vec<Element<Message>> = app
        .active_profile()
        .map(|x| {
            x.lighting
                .zones
                .iter()
                .map(|(key, mode)| {
                    let device = devices.iter().find(|d| d.id.as_str() == key);
                    let rgb_name = key.strip_prefix("openrgb:");
                    let name = device.map(|d| d.label.clone()).unwrap_or_else(|| rgb_name.unwrap_or(key).to_string());
                    let absent = match (device, rgb_name) {
                        (Some(_), _) => None,
                        (None, Some(_)) if !app.rgb_server => Some("OpenRGB isn't running"),
                        (None, Some(n)) if !app.rgb_devices.iter().any(|d| d.name == n) => Some("not connected"),
                        (None, Some(_)) => None,
                        (None, None) => Some("not on this machine"),
                    };
                    let mut label = device.map(|d| light::describe_on(d, mode)).unwrap_or_else(|| light::describe(mode));
                    if let (Some(b), false) = (x.lighting.device_brightness.get(key), matches!(mode, LightingMode::Off)) {
                        label = format!("{label} · {b}%");
                    }
                    let tint = match mode {
                        LightingMode::Static(c) | LightingMode::Breathing(c) => colour(*c),
                        LightingMode::Off => p.text_faint,
                        LightingMode::Thermal { .. } => p.warn,
                        _ => p.accent,
                    };
                    // The card is narrow and device names are long: the name takes the
                    // width and wraps, the pill keeps its size, and the second line
                    // carries the note and a small "forget" so nothing can run past
                    // the card's edge.
                    let name = iced::widget::text(name).size(size::BODY).font(theme::font::SANS).color(p.text).width(Length::Fill);
                    let forget = iced::widget::button(iced::widget::text("forget").size(size::CAPTION).font(theme::font::MONO).color(p.text_secondary)).padding([2, 6]).style(widgets::button_style(p, ButtonKind::Ghost)).on_press(msg(LightingMsg::Forget(key.clone())));
                    let note: Element<Message> = match absent {
                        Some(why) => widgets::dim(p, why),
                        None => iced::widget::Space::new().width(Length::Fill).into(),
                    };
                    column![
                        row![name, widgets::pill(p, label, tint)].spacing(space::SM).align_y(iced::Alignment::Center),
                        row![container(note).width(Length::Fill), forget].spacing(space::SM).align_y(iced::Alignment::Center),
                    ]
                    .spacing(2.0)
                    .into()
                })
                .collect()
        })
        .unwrap_or_default();
    let blurb = if rows.is_empty() { format!("“{profile}” leaves lighting as it is. Pick a colour or effect and it's kept here.") } else { format!("Shown whenever “{profile}” is activated.") };
    let list = Column::with_children(rows).spacing(space::SM);
    // Room on the right for the scrollbar, so it never sits on a pill.
    let list: Element<Message> = if fill { scrollable(container(list).padding(iced::Padding { right: 14.0, ..iced::Padding::ZERO })).height(Length::Fill).into() } else { list.into() };
    let body = column![widgets::eyebrow(p, "Profile lighting"), widgets::dim(p, blurb), list].spacing(space::MD);
    let card = if fill { widgets::card(p, body.height(Length::Fill)).height(Length::Fill) } else { widgets::card(p, body) };
    card.width(Length::Fill).into()
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

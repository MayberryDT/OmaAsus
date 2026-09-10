//! The tray drop-down: a quick panel sized to its content. It carries what a
//! glance from the bar needs (profile, vitals, fans) and the few controls this
//! machine actually has; everything deeper opens the full window.
//!
//! Every section has a fixed height, so [`layout`] knows the panel's height as
//! it builds it and the layer surface can be sized to fit.

use crate::app::{App, Message};
use crate::pages::Page;
use crate::pages::asus::AsusMsg;
use crate::pages::cpu::slider_style;
use crate::telemetry::Freshness;
use crate::theme::{self, Palette, size, space};
use crate::widgets::{self, icons::{self, Icon}, sparkline::Sparkline};
use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, Space, button, canvas, column, container, row, scrollable, slider, text, tooltip};
use iced::{Background, Border, Color, Element, Length};
use oma_hw::supergfx::{GfxMode, GfxPower};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub enum QuickMsg {
    /// Open the full window on a page and fold the panel away.
    OpenPage(Page),
    ChargeDrag(f64),
    ChargeCommit,
    /// power-profiles-daemon profile, when asusd doesn't own the platform profile.
    PowerProfile(String),
    KbdBrightness(u32),
}

const PAD: f32 = 16.0;
const GAP: f32 = 14.0;
const HEADER: f32 = 36.0;
const TITLE: f32 = 16.0;
const TITLE_GAP: f32 = 6.0;
const CHIP: f32 = 30.0;
const CHIP_GAP: f32 = 6.0;
const VITAL: f32 = 40.0;
const CONTROL: f32 = 36.0;
const ROW_GAP: f32 = 4.0;
const FAN: f32 = 22.0;
const FOOTER: f32 = 40.0;
const TOAST: f32 = 38.0;
const LABEL_W: f32 = 96.0;
/// JetBrains Mono advances 0.6 em; a little over keeps chip-row estimates safe.
const MONO_ADVANCE: f32 = 0.62;

/// A section and the height it occupies.
type Section<'a> = (Element<'a, Message>, f32);

/// Build the panel for `width` and at most `max_h` logical pixels. Returns the
/// element and the height it needs, never more than `max_h`: when content is
/// taller, the middle scrolls while header and footer stay put.
pub fn layout(app: &App, width: f32, max_h: f32) -> (Element<'_, Message>, f32) {
    let inner = width - 2.0 * PAD;
    let mut body: Vec<Section> = Vec::new();
    if let Some((msg, ok)) = &app.toast {
        body.push(toast(app.palette, msg, *ok));
    }
    body.push(profiles(app, inner));
    body.extend(vitals(app));
    body.extend(controls(app, inner));
    body.extend(fans(app));

    let body_h = body.iter().map(|(_, h)| *h).sum::<f32>() + GAP * body.len().saturating_sub(1) as f32;
    let natural = 2.0 * PAD + HEADER + FOOTER + 2.0 * GAP + body_h;
    let sections = Column::with_children(body.into_iter().map(|(el, h)| container(el).width(Length::Fill).height(h).into())).spacing(GAP);
    let middle: Element<Message> = if natural > max_h { scrollable(sections).height(Length::Fill).into() } else { sections.into() };
    let panel = column![header(app), middle, footer(app)].spacing(GAP).width(Length::Fill).height(Length::Fill);
    (container(panel).padding(PAD).width(Length::Fill).height(Length::Fill).into(), natural.min(max_h).ceil())
}

fn caption<'a>(s: impl ToString, color: Color) -> Element<'a, Message> {
    text(s.to_string()).size(size::CAPTION).font(theme::font::MONO).color(color).wrapping(Wrapping::None).into()
}

fn title<'a>(p: Palette, label: &str, right: Option<Element<'a, Message>>) -> Element<'a, Message> {
    let mut r = row![widgets::eyebrow(p, label), widgets::hfill()].align_y(iced::Alignment::Center);
    if let Some(e) = right {
        r = r.push(e);
    }
    container(r).height(TITLE).into()
}

fn tip<'a>(p: Palette, s: &str) -> Element<'a, Message> {
    container(caption(s, p.text))
        .padding([4, 8])
        .style(move |_| container::Style { background: Some(Background::Color(p.surface_2)), border: Border { color: p.border_strong, width: 1.0, radius: 0.0.into() }, ..Default::default() })
        .into()
}

fn chip<'a>(p: Palette, label: impl ToString, active: bool, msg: Option<Message>) -> Element<'a, Message> {
    let t = text(label.to_string()).size(size::CAPTION).font(theme::font::MONO_MEDIUM).wrapping(Wrapping::None);
    let kind = if active { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost };
    let mut b = button(container(t).center_y(Length::Fill)).height(CHIP).padding([0, 10]).style(widgets::button_style(p, kind));
    if let Some(m) = msg {
        b = b.on_press(m);
    }
    b.into()
}

fn chip_width(label: &str) -> f32 {
    label.chars().count() as f32 * size::CAPTION * MONO_ADVANCE + 20.0
}

/// Lines a wrapped strip of chips needs at `width`.
fn chip_rows<'s>(labels: impl IntoIterator<Item = &'s str>, width: f32) -> usize {
    let (mut rows, mut x) = (1, 0.0);
    for l in labels {
        let w = chip_width(l);
        if x > 0.0 && x + w > width {
            rows += 1;
            x = 0.0;
        }
        x += w + CHIP_GAP;
    }
    rows
}

fn strip_height(rows: usize) -> f32 {
    rows as f32 * CHIP + rows.saturating_sub(1) as f32 * CHIP_GAP
}

fn strip<'a>(chips: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    Row::with_children(chips).spacing(CHIP_GAP).wrap().into()
}

fn header(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let (state, color) = if app.controller_ready { ("helper", p.ok) } else { ("read-only", p.warn) };
    let dot = container(Space::new().width(6.0).height(6.0)).style(move |_| container::Style { background: Some(Background::Color(color)), ..Default::default() });
    let icon_btn = |ic: Icon, hint: &str, msg: Message| {
        tooltip(button(icons::icon(ic, p.text_secondary, 15.0)).padding(7).style(widgets::button_style(p, widgets::ButtonKind::Ghost)).on_press(msg), tip(p, hint), tooltip::Position::Bottom)
    };
    container(
        row![
            widgets::pixel::oma_mark(p, 22.0),
            text("omaasus").size(size::SMALL).font(theme::font::MONO_MEDIUM).color(p.text),
            widgets::hfill(),
            dot,
            caption(state, p.text_secondary),
            icon_btn(Icon::Window, "Open window", Message::Quick(QuickMsg::OpenPage(app.page))),
            icon_btn(Icon::Close, "Close", Message::ToggleOverlay),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center),
    )
    .height(HEADER)
    .into()
}

fn toast<'a>(p: Palette, msg: &'a str, ok: bool) -> Section<'a> {
    let close = button(icons::icon(Icon::Close, p.text_secondary, 12.0)).padding(4).style(widgets::button_style(p, widgets::ButtonKind::Nav { active: false })).on_press(Message::DismissToast);
    let el = container(row![caption(msg, p.text), widgets::hfill(), close].spacing(space::SM).align_y(iced::Alignment::Center))
        .padding([0, 10])
        .center_y(Length::Fill)
        .width(Length::Fill)
        .clip(true)
        .style(move |_| container::Style { background: Some(Background::Color(p.surface)), border: Border { color: if ok { p.brand } else { p.red }, width: 1.0, radius: 0.0.into() }, ..Default::default() });
    (el.into(), TOAST)
}

fn profiles(app: &App, width: f32) -> Section<'_> {
    let p = app.palette;
    let rows = chip_rows(app.config.profiles.iter().map(|pr| pr.name.as_str()), width);
    let chips = app.config.profiles.iter().map(|pr| chip(p, &pr.name, pr.id == app.config.active_profile, Some(Message::ApplyProfile(pr.id)))).collect();
    let auto = (app.config.mode == oma_hw::profile::Mode::Automatic).then(|| widgets::pill(p, "automatic", p.accent));
    (column![title(p, "Profile", auto), strip(chips)].spacing(TITLE_GAP).into(), TITLE + TITLE_GAP + strip_height(rows))
}

#[allow(clippy::too_many_arguments)]
fn vital<'a>(p: Palette, label: &str, value: String, value_color: Color, second: String, hist: &'a VecDeque<f32>, max: f32, color: Color, note: String) -> Element<'a, Message> {
    row![
        container(caption(label.to_uppercase(), p.text_secondary)).width(56.0),
        container(text(value).size(size::TITLE).font(theme::font::MONO_MEDIUM).color(value_color).wrapping(Wrapping::None)).width(64.0),
        container(caption(second, p.text_secondary)).width(44.0),
        canvas(Sparkline { palette: p, data: hist, min: 0.0, max, color, capacity: crate::app::HISTORY }).width(Length::FillPortion(1)).height(22.0),
        container(caption(note, p.text_dim)).width(Length::FillPortion(1)).align_right(Length::FillPortion(1)).clip(true),
    ]
    .spacing(space::SM)
    .height(VITAL)
    .align_y(iced::Alignment::Center)
    .into()
}

fn vitals(app: &App) -> Option<Section<'_>> {
    let p = app.palette;
    let s = app.snapshot.as_deref()?;
    let inv = app.inventory.as_deref();
    let mut rows: Vec<Element<Message>> = Vec::new();
    if let Some(t) = s.cpu.tctl_c {
        let name = inv.map(|i| short_cpu(&i.cpu.model)).unwrap_or_default();
        rows.push(vital(p, "CPU", format!("{t:.0}°"), theme::thermal(&p, t, 35.0, 95.0), format!("{:.0}%", s.cpu.util_total), &app.hist.cpu_load, 100.0, p.cpu, name));
    }
    if let Some(g) = s.gpu() {
        let discrete_name = if g.discrete { app.nvidia_info.as_ref().map(|n| short_gpu(&n.name)) } else { None };
        let name = discrete_name.or_else(|| inv.and_then(|i| i.amd_gpus.iter().find(|a| a.is_integrated != g.discrete)).map(|a| short_gpu(&a.name))).unwrap_or_default();
        let note = match g.power_w {
            Some(w) if g.discrete => format!("{name} · {w:.0} W"),
            _ => name,
        };
        let t = g.temp_c.unwrap_or(0.0);
        rows.push(vital(p, "GPU", format!("{t:.0}°"), theme::thermal(&p, t, 30.0, 90.0), g.load.map(|l| format!("{l:.0}%")).unwrap_or_default(), &app.hist.gpu_load, 100.0, p.gpu, note));
    }
    if let Some(w) = s.package_w() {
        let peak = app.hist.power.iter().copied().fold(10.0_f32, f32::max);
        rows.push(vital(p, "Power", format!("{w:.0} W"), p.power, String::new(), &app.hist.power, peak * 1.15, p.power, "package".into()));
    }
    if rows.is_empty() {
        return None;
    }
    let n = rows.len() as f32;
    Some((column![title(p, "Vitals", None), Column::with_children(rows).spacing(ROW_GAP)].spacing(TITLE_GAP).into(), TITLE + TITLE_GAP + n * VITAL + (n - 1.0) * ROW_GAP))
}

fn control<'a>(p: Palette, label: &str, content: Element<'a, Message>, content_h: f32) -> Section<'a> {
    let h = content_h.max(CONTROL);
    (row![container(caption(label.to_uppercase(), p.text_secondary)).width(LABEL_W), container(content).width(Length::Fill)].height(h).align_y(iced::Alignment::Center).into(), h)
}

/// The quick controls this machine has; none of the rows exist elsewhere.
fn controls(app: &App, width: f32) -> Option<Section<'_>> {
    let p = app.palette;
    let strip_w = width - LABEL_W;
    let mut rows: Vec<Section> = Vec::new();

    // Power mode: asusd owns the platform profile when it runs, else power-profiles-daemon.
    if !app.asus.choices.is_empty() {
        let n = chip_rows(app.asus.choices.iter().map(|c| c.label()), strip_w);
        let chips = app.asus.choices.iter().map(|c| chip(p, c.label(), Some(*c) == app.asus.profile, Some(Message::Asus(AsusMsg::Profile(*c))))).collect();
        rows.push(control(p, "Power mode", strip(chips), strip_height(n)));
    } else if let Some(ppd) = &app.ppd {
        let n = chip_rows(ppd.profiles.iter().map(|x| x.name.as_str()), strip_w);
        let chips = ppd.profiles.iter().map(|x| chip(p, &x.name, x.name == ppd.active, Some(Message::Quick(QuickMsg::PowerProfile(x.name.clone()))))).collect();
        rows.push(control(p, "Power mode", strip(chips), strip_height(n)));
    }

    if let Some(g) = &app.asus.gfx {
        let power = match g.power {
            Some(GfxPower::Active) => " · dGPU on",
            Some(GfxPower::Suspended) => " · dGPU asleep",
            Some(GfxPower::Off) | Some(GfxPower::AsusDisabled) => " · dGPU off",
            Some(GfxPower::AsusMuxDiscreet) => " · dGPU only",
            _ => "",
        };
        let status = if g.pending_mode != GfxMode::None { caption(format!("→ {} · {}", g.pending_mode.label(), g.pending_action.label()), p.warn) } else { caption(format!("{}{power}", g.mode.label()), p.text) };
        // Switching can mean a logout or reboot, so it happens in the full window.
        let open = chip(p, "›", false, Some(Message::Quick(QuickMsg::OpenPage(Page::Asus))));
        rows.push(control(p, "Graphics", row![status, widgets::hfill(), open].align_y(iced::Alignment::Center).into(), CHIP));
    }

    if let Some(limit) = app.asus.charge_limit {
        let v = app.quick_charge.unwrap_or(limit as f64);
        let el = row![
            container(caption(format!("{v:.0}%"), p.text)).width(40.0),
            slider(20.0..=100.0, v, |x| Message::Quick(QuickMsg::ChargeDrag(x))).on_release(Message::Quick(QuickMsg::ChargeCommit)).step(5.0).style(slider_style(p)),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center);
        rows.push(control(p, "Charge limit", el.into(), CHIP));
    }

    if let Some(k) = &app.asus.kbd {
        let labels = oma_hw::lighting::level_names(&k.levels);
        let n = chip_rows(labels.iter().map(String::as_str), strip_w);
        let chips = k.levels.iter().zip(&labels).map(|(l, name)| chip(p, name, *l == k.brightness, Some(Message::Quick(QuickMsg::KbdBrightness(*l))))).collect();
        rows.push(control(p, "Keyboard", strip(chips), strip_height(n)));
    }

    if rows.is_empty() {
        return None;
    }
    let h = TITLE + TITLE_GAP + rows.iter().map(|(_, h)| *h).sum::<f32>() + ROW_GAP * (rows.len() - 1) as f32;
    let list = Column::with_children(rows.into_iter().map(|(el, _)| el)).spacing(ROW_GAP);
    Some((column![title(p, "Controls", None), list].spacing(TITLE_GAP).into(), h))
}

fn fans(app: &App) -> Option<Section<'_>> {
    let p = app.palette;
    let s = app.snapshot.as_deref()?;
    if s.fans.is_empty() {
        return None;
    }
    let mut items = s.fans.iter().map(|f| {
        let (status, color) = match f.freshness {
            Freshness::Offline => ("offline".to_string(), p.red),
            Freshness::Stale => (format!("{} rpm", f.rpm), p.text_faint),
            Freshness::Live if f.rpm == 0 => ("stopped".to_string(), p.text_dim),
            Freshness::Live => (format!("{} rpm", f.rpm), p.text),
        };
        row![caption(&f.label, p.text_secondary), widgets::hfill(), caption(status, color)].width(Length::Fill).into()
    });
    // Two fans per line.
    let mut lines = Column::new();
    let mut n = 0;
    while let Some(a) = items.next() {
        let b: Element<Message> = items.next().unwrap_or_else(|| Space::new().width(Length::Fill).into());
        lines = lines.push(container(row![container(a).width(Length::FillPortion(1)), container(b).width(Length::FillPortion(1))].spacing(space::LG)).center_y(FAN));
        n += 1;
    }
    Some((column![title(p, "Fans", None), lines].spacing(TITLE_GAP).into(), TITLE + TITLE_GAP + n as f32 * FAN))
}

fn footer(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let links = app.visible_pages().into_iter().map(|pg| {
        let b = button(icons::icon(pg.icon(), p.text_secondary, 16.0)).padding(8).style(widgets::button_style(p, widgets::ButtonKind::Nav { active: false })).on_press(Message::Quick(QuickMsg::OpenPage(pg)));
        tooltip(b, tip(p, pg.label()), tooltip::Position::Top).into()
    });
    container(row(links).spacing(space::XS).align_y(iced::Alignment::Center)).center_x(Length::Fill).center_y(FOOTER).into()
}

/// "AMD Ryzen AI 9 HX 370 w/ Radeon 890M" → "Ryzen AI 9 HX 370".
fn short_cpu(model: &str) -> String {
    let m = model.split(" w/ ").next().unwrap_or(model);
    let m = m.split(" with ").next().unwrap_or(m);
    let m = m.split(" Processor").next().unwrap_or(m);
    m.trim_start_matches("AMD ").trim_start_matches("Intel(R) Core(TM) ").trim_start_matches("Intel ").trim().to_string()
}

/// lspci puts the marketing name last in brackets ("… [Radeon 880M / 890M] (rev c1)");
/// NVML names start with the vendor ("NVIDIA GeForce RTX …").
fn short_gpu(name: &str) -> String {
    if let Some((_, rest)) = name.rsplit_once('[') {
        if let Some((n, _)) = rest.split_once(']') {
            return n.to_string();
        }
    }
    name.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_shortened_for_the_panel() {
        assert_eq!(short_cpu("AMD Ryzen AI 9 HX 370 w/ Radeon 890M"), "Ryzen AI 9 HX 370");
        assert_eq!(short_cpu("AMD Ryzen 9 7950X 16-Core Processor"), "Ryzen 9 7950X 16-Core");
        assert_eq!(short_gpu("Advanced Micro Devices, Inc. [AMD/ATI] Strix [Radeon 880M / 890M] (rev c1)"), "Radeon 880M / 890M");
        assert_eq!(short_gpu("NVIDIA GeForce RTX 4090"), "RTX 4090");
    }

    #[test]
    fn chip_strips_wrap_by_width() {
        assert_eq!(chip_rows(["Quiet", "Balanced", "Performance"], 400.0), 1);
        assert_eq!(chip_rows(["Quiet", "Balanced", "Performance"], 150.0), 2);
        assert_eq!(chip_rows(["Quiet", "Balanced", "Performance"], 100.0), 3);
        assert_eq!(strip_height(2), 2.0 * CHIP + CHIP_GAP);
    }
}

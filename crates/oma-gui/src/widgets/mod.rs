//! Reusable, styled building blocks.

use crate::theme::{self, Palette, size, space};
use iced::widget::{button, column, container, row, text, Column};
use iced::{Background, Border, Color, Element, Length, Shadow};
use std::sync::atomic::{AtomicU32, Ordering};

pub mod ambient;
pub mod curve;
pub mod gauge;
pub mod icons;
pub mod pixel;
pub mod reveal;
pub mod ridge;
pub mod shaders;
pub mod sparkline;

#[allow(dead_code)]
static PHASE_BITS: AtomicU32 = AtomicU32::new(0);
static HEAT_BITS: AtomicU32 = AtomicU32::new(0);
static LOAD_BITS: AtomicU32 = AtomicU32::new(0);

pub fn set_phase(seconds: f32) {
    PHASE_BITS.store(seconds.to_bits(), Ordering::Relaxed);
}
pub fn set_thermal(heat: f32, load: f32) {
    HEAT_BITS.store(heat.to_bits(), Ordering::Relaxed);
    LOAD_BITS.store(load.to_bits(), Ordering::Relaxed);
}
pub fn begin_frame() {}
pub fn thermal() -> (f32, f32) {
    (f32::from_bits(HEAT_BITS.load(Ordering::Relaxed)), f32::from_bits(LOAD_BITS.load(Ordering::Relaxed)))
}

/// Surface card: opaque `surface` with the site's 1px elevation ring. Square.
pub fn card<'a, M: 'a>(p: Palette, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content).padding(space::XL).width(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Color(p.surface)),
        border: theme::ring(&p),
        ..Default::default()
    })
}

/// Brand-tinted hero surface (bg-deep with a brand ring).
pub fn glow_card<'a, M: 'a>(p: Palette, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content).padding(space::XL).width(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Color(p.bg_deep)),
        border: Border { color: p.border_subtle, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    })
}

/// Section label: the site's `page-subtitle` — mono, uppercase, tracked, brand.
pub fn eyebrow<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(tracked(&s.to_string())).size(size::MICRO).font(theme::font::MONO_MEDIUM).color(p.brand).into()
}

pub fn tracked(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.to_uppercase().chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out
}

/// h3-like: Geist medium 18.
pub fn title<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::TITLE).font(theme::font::SANS_MEDIUM).color(p.text).into()
}

/// Section heading: Geist semibold 22, tight.
pub fn headline<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::HEADLINE).font(theme::font::SANS_SEMIBOLD).color(p.text).into()
}

pub fn body<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::BODY).font(theme::font::SANS).color(p.text).into()
}

/// Secondary prose in mono, as the site's homepage copy.
pub fn dim<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::SMALL).font(theme::font::MONO).color(p.text_secondary).into()
}

pub fn mono<'a, M: 'a>(p: Palette, s: impl ToString, sz: f32) -> Element<'a, M> {
    text(s.to_string()).size(sz).font(theme::font::MONO).color(p.text).into()
}

/// A metric: mono numeral with unit and a mono eyebrow.
pub fn metric<'a, M: 'a>(p: Palette, label: &str, value: String, unit: &str, color: Color) -> Element<'a, M> {
    Column::new()
        .spacing(space::XS)
        .push(eyebrow(p, label))
        .push(
            row![
                text(value).size(size::DISPLAY).font(theme::font::MONO_MEDIUM).color(color).line_height(1.05),
                text(unit.to_string()).size(size::SMALL).font(theme::font::MONO).color(p.text_secondary),
            ]
            .spacing(space::XS)
            .align_y(iced::Alignment::End),
        )
        .into()
}

/// Badge: square, surface-2, mono caption.
pub fn pill<'a, M: 'a>(p: Palette, s: impl ToString, color: Color) -> Element<'a, M> {
    container(text(s.to_string()).size(size::CAPTION).font(theme::font::MONO).color(color))
        .padding([3, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(p.surface_2)),
            border: Border { color: theme::alpha(color, 0.35), width: 1.0, radius: 0.0.into() },
            ..Default::default()
        })
        .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Ghost,
    Nav { active: bool },
    Danger,
}

pub fn button_style(p: Palette, kind: ButtonKind) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let base = button::Style { text_color: p.text, border: Border { radius: 0.0.into(), ..Default::default() }, shadow: Shadow::default(), ..Default::default() };
        match kind {
            ButtonKind::Primary => button::Style {
                background: Some(Background::Color(if hovered { theme::mix(p.brand, Color::WHITE, 0.14) } else { p.brand })),
                text_color: p.brand_ink,
                border: Border { radius: 0.0.into(), ..Default::default() },
                ..base
            },
            ButtonKind::Ghost => button::Style {
                background: Some(Background::Color(if hovered { p.surface_2 } else { p.surface })),
                border: Border { color: p.border_strong, width: 1.0, radius: 0.0.into() },
                text_color: p.text,
                ..base
            },
            ButtonKind::Nav { active } => button::Style {
                background: Some(Background::Color(if active { p.surface_2 } else if hovered { p.surface } else { Color::TRANSPARENT })),
                text_color: if active { p.text } else { p.text_secondary },
                border: Border { radius: 0.0.into(), ..Default::default() },
                ..base
            },
            ButtonKind::Danger => button::Style {
                background: Some(Background::Color(theme::alpha(p.red, if hovered { 0.2 } else { 0.1 }))),
                text_color: p.red,
                border: Border { radius: 0.0.into(), ..Default::default() },
                ..base
            },
        }
    }
}

pub fn btn<'a, M: Clone + 'a>(p: Palette, label: impl ToString, kind: ButtonKind, on_press: Option<M>) -> Element<'a, M> {
    let t = text(label.to_string()).size(size::SMALL).font(theme::font::MONO_MEDIUM);
    let mut b = button(t).padding([8, 12]).style(button_style(p, kind));
    if let Some(m) = on_press {
        b = b.on_press(m);
    }
    b.into()
}

/// Horizontal spacer that fills.
pub fn hfill<'a, M: 'a>() -> Element<'a, M> {
    iced::widget::Space::new().width(Length::Fill).into()
}

pub fn vfill<'a, M: 'a>() -> Element<'a, M> {
    iced::widget::Space::new().height(Length::Fill).into()
}

/// Thin separator line.
pub fn rule<'a, M: 'a>(p: Palette) -> Element<'a, M> {
    container(iced::widget::Space::new().width(Length::Fill).height(1.0))
        .height(1.0)
        .width(Length::Fill)
        .style(move |_| container::Style { background: Some(Background::Color(p.border_subtle)), ..Default::default() })
        .into()
}

/// A labelled horizontal bar 0..=1.
pub fn bar<'a, M: 'a>(p: Palette, frac: f32, color: Color) -> Element<'a, M> {
    let frac = frac.clamp(0.0, 1.0);
    let fill = container(iced::widget::Space::new().width(Length::FillPortion(((frac * 1000.0) as u16).max(1))).height(6.0))
        .style(move |_| container::Style { background: Some(Background::Color(color)), ..Default::default() });
    let rest = iced::widget::Space::new().width(Length::FillPortion((((1.0 - frac) * 1000.0) as u16).max(1))).height(6.0);
    container(row![fill, rest])
        .width(Length::Fill)
        .style(move |_| container::Style { background: Some(Background::Color(p.surface_2)), ..Default::default() })
        .into()
}

/// One fan/pump line: label, device, duty bar, rpm, duty; dimmed when stale, flagged when offline.
pub fn fan_row<'a, M: 'a>(p: Palette, f: &crate::telemetry::FanReading) -> Element<'a, M> {
    use crate::telemetry::Freshness;
    let live = f.freshness == Freshness::Live;
    let tc = if live { p.text } else { p.text_faint };
    let duty = f.duty.unwrap_or(0.0) as f32 / 100.0;
    let frac = if f.duty.is_some() { duty } else { (f.rpm as f32 / 2400.0).min(1.0) };
    let status: Element<M> = match f.freshness {
        Freshness::Live => iced::widget::Space::new().into(),
        Freshness::Stale => pill(p, "stale", p.warn),
        Freshness::Offline => pill(p, "offline", p.danger),
    };
    row![
        column![text(f.label.clone()).size(size::BODY).font(theme::font::BODY).color(tc), dim(p, f.device.clone())].spacing(1.0).width(Length::FillPortion(3)),
        status,
        bar(p, frac, if live { p.fan } else { theme::alpha(p.fan, 0.35) }),
        text(format!("{:>5} rpm", f.rpm)).size(size::SMALL).font(theme::font::MONO).color(tc),
        text(f.duty.map(|d| format!("{d:>3.0}%")).unwrap_or_else(|| "  — ".into())).size(size::SMALL).font(theme::font::MONO).color(tc),
    ]
    .spacing(space::MD)
    .align_y(iced::Alignment::Center)
    .into()
}

/// One temperature line from the registry, dimmed when stale.
pub fn temp_row<'a, M: 'a>(p: Palette, r: &crate::telemetry::Reading) -> Element<'a, M> {
    use crate::telemetry::Freshness;
    let live = r.freshness == Freshness::Live;
    let col = if live { theme::thermal(&p, r.value, 30.0, 90.0) } else { p.text_faint };
    let value = if r.freshness == Freshness::Offline && r.value == 0.0 { "—".to_string() } else { format!("{:.0}°", r.value) };
    row![dim(p, r.label.clone()), hfill(), text(value).size(size::BODY).font(theme::font::MONO).color(col)].align_y(iced::Alignment::Center).into()
}

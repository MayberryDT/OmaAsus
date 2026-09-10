//! Reusable, styled building blocks.

pub mod ambient;
pub mod curve;
pub mod gauge;
pub mod icons;
pub mod ridge;
pub mod sparkline;

use crate::theme::{self, Palette, radius, size, space};
use iced::widget::{button, container, row, text, Column};
use iced::{Background, Border, Color, Element, Length, Shadow};

/// Layered glass card: a gradient hairline (outer) around a gradient glass
/// body (inner) with a deep soft shadow.
pub fn card<'a, M: 'a>(p: Palette, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    let inner = container(content).padding(space::LG).width(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(iced::Radians(2.6)).add_stop(0.0, p.glass_strong).add_stop(0.45, p.glass).add_stop(1.0, theme::alpha(p.bg, 0.55)),
        ))),
        border: Border { radius: (radius::LG - 1.0).into(), ..Default::default() },
        ..Default::default()
    });
    container(inner).padding(1).style(move |_| container::Style {
        background: Some(Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(iced::Radians(2.6)).add_stop(0.0, theme::alpha(Color::WHITE, 0.18)).add_stop(0.5, theme::alpha(Color::WHITE, 0.05)).add_stop(1.0, theme::alpha(p.accent, 0.08)),
        ))),
        border: Border { radius: radius::LG.into(), ..Default::default() },
        shadow: theme::card_shadow(),
        ..Default::default()
    })
}

/// Accent-lit hero card.
pub fn glow_card<'a, M: 'a>(p: Palette, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    let inner = container(content).padding(space::XL).width(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(iced::Radians(2.3)).add_stop(0.0, theme::alpha(p.accent, 0.16)).add_stop(0.5, theme::alpha(p.accent, 0.05)).add_stop(1.0, theme::alpha(p.bg, 0.55)),
        ))),
        border: Border { radius: (radius::LG - 1.0).into(), ..Default::default() },
        ..Default::default()
    });
    container(inner).padding(1).style(move |_| container::Style {
        background: Some(Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(iced::Radians(2.3)).add_stop(0.0, theme::alpha(p.accent, 0.65)).add_stop(0.6, theme::alpha(Color::WHITE, 0.08)).add_stop(1.0, theme::alpha(p.accent_2, 0.25)),
        ))),
        border: Border { radius: radius::LG.into(), ..Default::default() },
        shadow: Shadow { color: theme::alpha(p.accent, 0.28), offset: iced::Vector::new(0.0, 16.0), blur_radius: 48.0 },
        ..Default::default()
    })
}

/// Section label: accent tick + tracked micro caps.
pub fn eyebrow<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    let tick = container(iced::widget::Space::new().width(10.0).height(2.0)).style(move |_| container::Style { background: Some(Background::Color(theme::alpha(p.accent, 0.9))), border: Border { radius: 1.0.into(), ..Default::default() }, ..Default::default() });
    row![tick, text(tracked(&s.to_string())).size(size::MICRO).font(theme::font::BODY_MEDIUM).color(p.text_faint)].spacing(6.0).align_y(iced::Alignment::Center).into()
}

/// Fake letter-spacing: thin spaces between upper-case characters.
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

pub fn title<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::TITLE).font(theme::font::DISPLAY_MEDIUM).color(p.text).into()
}

pub fn headline<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::HEADLINE).font(theme::font::DISPLAY_LIGHT).color(p.text).into()
}


pub fn body<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::BODY).font(theme::font::BODY).color(p.text).into()
}

pub fn dim<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::SMALL).font(theme::font::BODY).color(p.text_dim).into()
}

pub fn mono<'a, M: 'a>(p: Palette, s: impl ToString, sz: f32) -> Element<'a, M> {
    text(s.to_string()).size(sz).font(theme::font::MONO).color(p.text).into()
}

/// A metric: big light numeral with unit and label.
pub fn metric<'a, M: 'a>(p: Palette, label: &str, value: String, unit: &str, color: Color) -> Element<'a, M> {
    Column::new()
        .spacing(space::XS)
        .push(eyebrow(p, label))
        .push(
            row![
                text(value).size(size::DISPLAY).font(theme::font::DISPLAY_LIGHT).color(color).line_height(1.0),
                text(unit.to_string()).size(size::SMALL).font(theme::font::BODY_MEDIUM).color(p.text_dim),
            ]
            .spacing(space::XS)
            .align_y(iced::Alignment::End),
        )
        .into()
}

/// Pill tag.
pub fn pill<'a, M: 'a>(_p: Palette, s: impl ToString, color: Color) -> Element<'a, M> {
    container(text(s.to_string()).size(size::CAPTION).font(theme::font::BODY).color(color))
        .padding([3, 9])
        .style(move |_| container::Style {
            background: Some(Background::Color(theme::alpha(color, 0.14))),
            border: Border { color: theme::alpha(color, 0.35), width: 1.0, radius: radius::PILL.into() },
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
        let base = button::Style { text_color: p.text, border: Border { radius: radius::MD.into(), ..Default::default() }, shadow: Shadow::default(), ..Default::default() };
        match kind {
            ButtonKind::Primary => button::Style {
                background: Some(Background::Gradient(iced::Gradient::Linear(
                    iced::gradient::Linear::new(iced::Radians(2.2)).add_stop(0.0, theme::mix(p.accent, Color::WHITE, if hovered { 0.28 } else { 0.14 })).add_stop(1.0, if hovered { theme::mix(p.accent, Color::WHITE, 0.08) } else { p.accent }),
                ))),
                text_color: Color::WHITE,
                border: Border { color: theme::alpha(Color::WHITE, 0.25), width: 1.0, radius: radius::PILL.into() },
                shadow: theme::glow(p.accent),
                ..base
            },
            ButtonKind::Ghost => button::Style {
                background: Some(Background::Color(if hovered { p.glass_strong } else { theme::alpha(p.glass, 0.7) })),
                border: Border { color: if hovered { p.line_strong } else { p.line }, width: 1.0, radius: radius::PILL.into() },
                text_color: if hovered { p.text } else { p.text_dim },
                ..base
            },
            ButtonKind::Nav { active } => button::Style {
                background: Some(if active {
                    Background::Gradient(iced::Gradient::Linear(iced::gradient::Linear::new(iced::Radians(1.57)).add_stop(0.0, theme::alpha(p.accent, 0.22)).add_stop(1.0, theme::alpha(p.accent, 0.04))))
                } else {
                    Background::Color(if hovered { p.glass } else { Color::TRANSPARENT })
                }),
                text_color: if active { p.text } else { p.text_dim },
                border: Border { color: if active { theme::alpha(p.accent, 0.35) } else { Color::TRANSPARENT }, width: 1.0, radius: radius::MD.into() },
                shadow: if active { theme::glow(p.accent) } else { Shadow::default() },
                ..base
            },
            ButtonKind::Danger => button::Style {
                background: Some(Background::Color(if hovered { p.danger } else { theme::alpha(p.danger, 0.18) })),
                text_color: if hovered { Color::WHITE } else { p.danger },
                border: Border { color: theme::alpha(p.danger, 0.4), width: 1.0, radius: radius::MD.into() },
                ..base
            },
        }
    }
}

pub fn btn<'a, M: Clone + 'a>(p: Palette, label: impl ToString, kind: ButtonKind, on_press: Option<M>) -> Element<'a, M> {
    let t = text(label.to_string()).size(size::BODY).font(theme::font::BODY_MEDIUM);
    let mut b = button(t).padding([8, 16]).style(button_style(p, kind));
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
        .style(move |_| container::Style { background: Some(Background::Color(p.line)), ..Default::default() })
        .into()
}

/// A labelled horizontal bar 0..=1.
pub fn bar<'a, M: 'a>(p: Palette, frac: f32, color: Color) -> Element<'a, M> {
    let frac = frac.clamp(0.0, 1.0);
    let fill = container(iced::widget::Space::new().width(Length::FillPortion(((frac * 1000.0) as u16).max(1))).height(6.0))
        .style(move |_| container::Style { background: Some(Background::Color(color)), border: Border { radius: radius::PILL.into(), ..Default::default() }, ..Default::default() });
    let rest = iced::widget::Space::new().width(Length::FillPortion((((1.0 - frac) * 1000.0) as u16).max(1))).height(6.0);
    container(row![fill, rest])
        .width(Length::Fill)
        .style(move |_| container::Style { background: Some(Background::Color(p.glass_strong)), border: Border { radius: radius::PILL.into(), ..Default::default() }, ..Default::default() })
        .into()
}

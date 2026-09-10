//! Reusable, styled building blocks.

pub mod curve;
pub mod gauge;
pub mod sparkline;

use crate::theme::{self, Palette, radius, size, space};
use iced::widget::{button, container, row, text, Column};
use iced::{Background, Border, Color, Element, Length, Shadow};

/// A glass card with padding.
pub fn card<'a, M: 'a>(p: Palette, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content)
        .padding(space::LG)
        .style(move |_| container::Style {
            background: Some(Background::Color(p.glass)),
            border: theme::card_border(&p),
            shadow: theme::card_shadow(),
            ..Default::default()
        })
}

/// A card with an accent glow (used for the active profile / hero).
pub fn glow_card<'a, M: 'a>(p: Palette, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content)
        .padding(space::XL)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(iced::Gradient::Linear(
                iced::gradient::Linear::new(iced::Radians(2.4))
                    .add_stop(0.0, theme::alpha(p.accent, 0.22))
                    .add_stop(1.0, theme::alpha(p.accent_2, 0.06)),
            ))),
            border: Border { color: theme::alpha(p.accent, 0.45), width: 1.0, radius: radius::LG.into() },
            shadow: theme::glow(p.accent),
            ..Default::default()
        })
}

/// Section label in small caps style.
pub fn eyebrow<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string().to_uppercase()).size(size::CAPTION).font(theme::font::BODY).color(p.text_faint).into()
}

pub fn title<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::TITLE).font(theme::font::DISPLAY).color(p.text).into()
}

pub fn headline<'a, M: 'a>(p: Palette, s: impl ToString) -> Element<'a, M> {
    text(s.to_string()).size(size::HEADLINE).font(theme::font::DISPLAY).color(p.text).into()
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

/// A metric: big number with unit and label.
pub fn metric<'a, M: 'a>(p: Palette, label: &str, value: String, unit: &str, color: Color) -> Element<'a, M> {
    Column::new()
        .spacing(space::XS)
        .push(eyebrow(p, label))
        .push(
            row![
                text(value).size(size::HEADLINE).font(theme::font::DISPLAY).color(color),
                text(unit.to_string()).size(size::SMALL).font(theme::font::BODY).color(p.text_dim),
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
                background: Some(Background::Color(if hovered { theme::mix(p.accent, Color::WHITE, 0.12) } else { p.accent })),
                text_color: Color::WHITE,
                shadow: if hovered { theme::glow(p.accent) } else { Shadow::default() },
                ..base
            },
            ButtonKind::Ghost => button::Style {
                background: Some(Background::Color(if hovered { p.glass_strong } else { p.glass })),
                border: Border { color: if hovered { p.line_strong } else { p.line }, width: 1.0, radius: radius::MD.into() },
                ..base
            },
            ButtonKind::Nav { active } => button::Style {
                background: Some(Background::Color(if active { p.accent_soft } else if hovered { p.glass } else { Color::TRANSPARENT })),
                text_color: if active { p.text } else { p.text_dim },
                border: Border { radius: radius::MD.into(), ..Default::default() },
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
    let t = text(label.to_string()).size(size::BODY).font(theme::font::BODY);
    let mut b = button(t).padding([8, 14]).style(button_style(p, kind));
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

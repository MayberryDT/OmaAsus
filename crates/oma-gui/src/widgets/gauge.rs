//! Arc gauge drawn on a canvas: value with unit, thermal-aware colour.

use crate::theme::{self, Palette};
use iced::widget::canvas::{self, Frame, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};
use std::f32::consts::PI;

pub struct Gauge {
    pub palette: Palette,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub label: String,
    pub unit: String,
    pub color: Color,
    pub decimals: usize,
    /// Optional second, thinner ring (e.g. load %) 0..=1.
    pub inner: Option<(f32, Color)>,
}

impl Gauge {
    fn frac(&self) -> f32 {
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }
}

impl<M> canvas::Program<M> for Gauge {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        let p = self.palette;
        let mut frame = Frame::new(renderer, bounds.size());
        let c = frame.center();
        let r = bounds.width.min(bounds.height) / 2.0 - 10.0;
        let start = 0.75 * PI;
        let span = 1.5 * PI;
        let arc = |radius: f32, end: f32| {
            Path::new(|b| b.arc(canvas::path::Arc { center: c, radius, start_angle: start.into(), end_angle: end.into() }))
        };
        let w = (r * 0.13).max(6.0);
        frame.stroke(&arc(r, start + span), Stroke::default().with_width(w).with_color(p.glass_strong).with_line_cap(canvas::LineCap::Round));
        // glow underlay
        frame.stroke(&arc(r, start + span * self.frac()), Stroke::default().with_width(w + 6.0).with_color(theme::alpha(self.color, 0.18)).with_line_cap(canvas::LineCap::Round));
        frame.stroke(&arc(r, start + span * self.frac()), Stroke::default().with_width(w).with_color(self.color).with_line_cap(canvas::LineCap::Round));
        if let Some((f, col)) = self.inner {
            let ri = r - w * 1.4;
            frame.stroke(&arc(ri, start + span), Stroke::default().with_width(w * 0.45).with_color(p.glass).with_line_cap(canvas::LineCap::Round));
            frame.stroke(&arc(ri, start + span * f.clamp(0.0, 1.0)), Stroke::default().with_width(w * 0.45).with_color(col).with_line_cap(canvas::LineCap::Round));
        }
        let value = format!("{:.*}", self.decimals, self.value);
        frame.fill_text(Text {
            content: value,
            position: Point::new(c.x, c.y - r * 0.08),
            color: p.text,
            size: (r * 0.5).into(),
            font: theme::font::DISPLAY,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        frame.fill_text(Text {
            content: self.unit.clone(),
            position: Point::new(c.x, c.y + r * 0.28),
            color: p.text_dim,
            size: (r * 0.18).into(),
            font: theme::font::BODY,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        frame.fill_text(Text {
            content: self.label.to_uppercase(),
            position: Point::new(c.x, c.y + r * 0.78),
            color: p.text_faint,
            size: (r * 0.15).max(9.0).into(),
            font: theme::font::BODY,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}

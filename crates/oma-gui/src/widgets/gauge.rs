//! Arc gauge: gradient sweep built from segments, tick ring, soft bloom,
//! big light-weight numeral. Values are pre-smoothed by the app.

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
    /// Optional inner ring (e.g. load) 0..=1.
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
        let w = (r * 0.06).clamp(3.0, 7.0);
        let arc = |radius: f32, a0: f32, a1: f32| Path::new(|b| b.arc(canvas::path::Arc { center: c, radius, start_angle: a0.into(), end_angle: a1.into() }));
        // Tick ring: square ticks, lit ones in the brand tone.
        for i in 0..=30 {
            let a = start + span * i as f32 / 30.0;
            let major = i % 5 == 0;
            let r0 = r + w * 1.6;
            let r1 = r0 + if major { 6.0 } else { 3.0 };
            let lit = (i as f32 / 30.0) <= self.frac();
            frame.stroke(
                &Path::line(Point::new(c.x + r0 * a.cos(), c.y + r0 * a.sin()), Point::new(c.x + r1 * a.cos(), c.y + r1 * a.sin())),
                Stroke::default().with_width(if major { 2.0 } else { 1.0 }).with_color(if lit { self.color } else { p.border_strong }),
            );
        }
        // Track and sweep: flat, square-capped, no bloom.
        frame.stroke(&arc(r, start, start + span), Stroke::default().with_width(w).with_color(p.surface_2));
        let f = self.frac();
        if f > 0.004 {
            frame.stroke(&arc(r, start, start + span * f), Stroke::default().with_width(w).with_color(self.color));
        }
        if let Some((fi, col)) = self.inner {
            let ri = r - w * 2.2;
            frame.stroke(&arc(ri, start, start + span), Stroke::default().with_width(w * 0.5).with_color(p.surface_2));
            frame.stroke(&arc(ri, start, start + span * fi.clamp(0.001, 1.0)), Stroke::default().with_width(w * 0.5).with_color(col));
        }
        frame.fill_text(Text {
            content: format!("{:.*}", self.decimals, self.value),
            position: Point::new(c.x, c.y - r * 0.05),
            color: p.text,
            size: (r * 0.5).into(),
            font: theme::font::MONO_MEDIUM,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        frame.fill_text(Text {
            content: self.unit.clone(),
            position: Point::new(c.x, c.y + r * 0.32),
            color: p.text_secondary,
            size: (r * 0.15).into(),
            font: theme::font::MONO,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        frame.fill_text(Text {
            content: crate::widgets::tracked(&self.label),
            position: Point::new(c.x, c.y + r * 0.92),
            color: p.brand,
            size: (r * 0.11).max(8.5).into(),
            font: theme::font::MONO_MEDIUM,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}

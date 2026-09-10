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
        let r = bounds.width.min(bounds.height) / 2.0 - 12.0;
        let start = 0.75 * PI;
        let span = 1.5 * PI;
        let w = (r * 0.11).max(5.0);
        let arc = |radius: f32, a0: f32, a1: f32| Path::new(|b| b.arc(canvas::path::Arc { center: c, radius, start_angle: a0.into(), end_angle: a1.into() }));

        // Tick ring.
        for i in 0..=30 {
            let a = start + span * i as f32 / 30.0;
            let major = i % 5 == 0;
            let r0 = r + w * 1.05;
            let r1 = r0 + if major { w * 0.75 } else { w * 0.35 };
            let lit = (i as f32 / 30.0) <= self.frac();
            frame.stroke(
                &Path::line(Point::new(c.x + r0 * a.cos(), c.y + r0 * a.sin()), Point::new(c.x + r1 * a.cos(), c.y + r1 * a.sin())),
                Stroke::default().with_width(if major { 1.5 } else { 1.0 }).with_color(if lit { theme::alpha(self.color, 0.85) } else { p.line_strong }),
            );
        }
        // Track.
        frame.stroke(&arc(r, start, start + span), Stroke::default().with_width(w).with_color(theme::alpha(Color::WHITE, 0.06)).with_line_cap(canvas::LineCap::Round));
        // Bloom + gradient sweep (segments interpolate from dim accent to full).
        let f = self.frac();
        if f > 0.005 {
            frame.stroke(&arc(r, start, start + span * f), Stroke::default().with_width(w * 2.6).with_color(theme::alpha(self.color, 0.10)).with_line_cap(canvas::LineCap::Round));
            frame.stroke(&arc(r, start, start + span * f), Stroke::default().with_width(w * 1.5).with_color(theme::alpha(self.color, 0.18)).with_line_cap(canvas::LineCap::Round));
            let segs = 28;
            let n = ((segs as f32 * f).ceil() as usize).max(1);
            for i in 0..n {
                let t0 = i as f32 / segs as f32;
                let t1 = ((i + 1) as f32 / segs as f32).min(f);
                let col = theme::mix(theme::alpha(self.color, 0.55), self.color, t1 / f.max(0.01));
                frame.stroke(&arc(r, start + span * t0, start + span * t1 + 0.012), Stroke::default().with_width(w).with_color(col).with_line_cap(if i == 0 { canvas::LineCap::Round } else { canvas::LineCap::Butt }));
            }
            let a = start + span * f;
            let tip = Point::new(c.x + r * a.cos(), c.y + r * a.sin());
            frame.fill(&Path::circle(tip, w * 0.9), theme::alpha(Color::WHITE, 0.9));
            frame.fill(&Path::circle(tip, w * 1.8), theme::alpha(self.color, 0.25));
        }
        if let Some((fi, col)) = self.inner {
            let ri = r - w * 1.6;
            frame.stroke(&arc(ri, start, start + span), Stroke::default().with_width(w * 0.4).with_color(theme::alpha(Color::WHITE, 0.05)).with_line_cap(canvas::LineCap::Round));
            frame.stroke(&arc(ri, start, start + span * fi.clamp(0.001, 1.0)), Stroke::default().with_width(w * 0.4).with_color(col).with_line_cap(canvas::LineCap::Round));
        }
        frame.fill_text(Text {
            content: format!("{:.*}", self.decimals, self.value),
            position: Point::new(c.x, c.y - r * 0.05),
            color: p.text,
            size: (r * 0.62).into(),
            font: theme::font::DISPLAY_LIGHT,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        frame.fill_text(Text {
            content: self.unit.clone(),
            position: Point::new(c.x, c.y + r * 0.36),
            color: theme::alpha(self.color, 0.9),
            size: (r * 0.17).into(),
            font: theme::font::BODY_MEDIUM,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        frame.fill_text(Text {
            content: self.label.to_uppercase(),
            position: Point::new(c.x, c.y + r * 0.92),
            color: p.text_faint,
            size: (r * 0.13).max(8.5).into(),
            font: theme::font::BODY_MEDIUM,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}

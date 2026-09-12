//! Smooth sparkline with gradient fill for time series.

use crate::theme::{self, Palette};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};
use std::collections::VecDeque;

pub struct Sparkline<'a> {
    #[allow(dead_code)]
    pub palette: Palette,
    pub data: &'a VecDeque<f32>,
    pub min: f32,
    pub max: f32,
    pub color: Color,
    pub capacity: usize,
}

impl<M> canvas::Program<M> for Sparkline<'_> {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        let n = self.data.len();
        if n < 2 {
            return vec![frame.into_geometry()];
        }
        let _ = self.capacity;
        let dx = w / (n as f32 - 1.0);
        let x0 = 0.0;
        let y = |v: f32| h - 3.0 - ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0) * (h - 6.0);
        // A missing reading is a gap in the line, not a dip to zero.
        let raw: Vec<(f32, f32)> = self.data.iter().enumerate().map(|(i, &v)| (x0 + dx * i as f32, if v.is_finite() { y(v) } else { f32::NAN })).collect();
        let runs = crate::widgets::finite_runs(&raw);
        if runs.is_empty() {
            return vec![frame.into_geometry()];
        }
        let to_pts = |run: &[(f32, f32)]| run.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>();
        let line = Path::new(|b| {
            for run in &runs {
                let pts = to_pts(run);
                b.move_to(pts[0]);
                for wnd in pts.windows(2) {
                    let (a, c) = (wnd[0], wnd[1]);
                    let mid = Point::new((a.x + c.x) / 2.0, (a.y + c.y) / 2.0);
                    b.quadratic_curve_to(a, mid);
                }
                b.line_to(*pts.last().unwrap());
            }
        });
        let area = Path::new(|b| {
            for run in &runs {
                let pts = to_pts(run);
                b.move_to(Point::new(pts[0].x, h));
                b.line_to(pts[0]);
                for wnd in pts.windows(2) {
                    let (a, c) = (wnd[0], wnd[1]);
                    let mid = Point::new((a.x + c.x) / 2.0, (a.y + c.y) / 2.0);
                    b.quadratic_curve_to(a, mid);
                }
                b.line_to(*pts.last().unwrap());
                b.line_to(Point::new(pts.last().unwrap().x, h));
                b.close();
            }
        });
        frame.fill(
            &area,
            canvas::Fill {
                style: canvas::Style::Gradient(canvas::Gradient::Linear(
                    canvas::gradient::Linear::new(Point::new(0.0, 0.0), Point::new(0.0, h))
                        .add_stop(0.0, theme::alpha(self.color, 0.35))
                        .add_stop(1.0, theme::alpha(self.color, 0.0)),
                )),
                rule: canvas::fill::Rule::NonZero,
            },
        );
        frame.stroke(&line, Stroke::default().with_width(2.0).with_color(self.color).with_line_cap(canvas::LineCap::Round).with_line_join(canvas::LineJoin::Round));
        // The marker sits on the newest reading only if it is the current one;
        // a series that ends in a gap has no "now" to mark.
        if let Some(&(x, y)) = raw.last().filter(|(_, y)| y.is_finite()) {
            let last = Point::new(x, y);
            frame.fill(&Path::circle(last, 3.0), self.color);
            frame.fill(&Path::circle(last, 6.0), theme::alpha(self.color, 0.25));
        }
        vec![frame.into_geometry()]
    }
}

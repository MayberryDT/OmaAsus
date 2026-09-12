//! "Thermal signature": a ridge plot of recent history — several offset,
//! stacked series drawn back to front with translucent fills. Decorative
//! and data-driven at once.

use crate::theme::{self, Palette};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};
use std::collections::VecDeque;

pub struct Ridge<'a> {
    pub palette: Palette,
    /// (series, min, max, colour) — first is drawn at the back.
    pub series: Vec<(&'a VecDeque<f32>, f32, f32, Color)>,
    pub capacity: usize,
    pub phase: f32,
}

impl<M> canvas::Program<M> for Ridge<'_> {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _t: &Theme, bounds: Rectangle, _c: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        let n_series = self.series.len().max(1) as f32;
        let band = h * 0.55;
        for (k, (data, lo, hi, col)) in self.series.iter().enumerate() {
            let base = h - 4.0 - (n_series - 1.0 - k as f32) * (h * 0.32 / n_series);
            let n = data.len();
            if n < 2 {
                continue;
            }
            // Stretch the available history across the full width.
            let _ = self.capacity;
            let dx = w / (n as f32 - 1.0);
            let x0 = 0.0;
            let y = |v: f32| base - ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * band;
            // A missing reading is a gap, not a dip to the baseline.
            let raw: Vec<(f32, f32)> = data.iter().enumerate().map(|(i, &v)| (x0 + dx * i as f32, if v.is_finite() { y(v) } else { f32::NAN })).collect();
            let runs: Vec<Vec<Point>> = crate::widgets::finite_runs(&raw).into_iter().map(|run| run.into_iter().map(|(x, y)| Point::new(x, y)).collect()).collect();
            if runs.is_empty() {
                continue;
            }
            let curve = |b: &mut canvas::path::Builder, pts: &[Point]| {
                for wnd in pts.windows(2) {
                    let (a, c) = (wnd[0], wnd[1]);
                    let mid = Point::new((a.x + c.x) / 2.0, (a.y + c.y) / 2.0);
                    b.quadratic_curve_to(a, mid);
                }
                b.line_to(*pts.last().unwrap());
            };
            let area = Path::new(|b| {
                for pts in &runs {
                    b.move_to(Point::new(pts[0].x, base));
                    b.line_to(pts[0]);
                    curve(b, pts);
                    b.line_to(Point::new(pts.last().unwrap().x, base));
                    b.close();
                }
            });
            let line = Path::new(|b| {
                for pts in &runs {
                    b.move_to(pts[0]);
                    curve(b, pts);
                }
            });
            // Fill knocks out what is behind it (dark), then tints.
            f.fill(&area, theme::alpha(self.palette.bg, 0.85));
            f.fill(
                &area,
                canvas::Fill {
                    style: canvas::Style::Gradient(canvas::Gradient::Linear(canvas::gradient::Linear::new(Point::new(0.0, base - band), Point::new(0.0, base)).add_stop(0.0, theme::alpha(*col, 0.42)).add_stop(1.0, theme::alpha(*col, 0.02)))),
                    rule: canvas::fill::Rule::NonZero,
                },
            );
            f.stroke(&line, Stroke::default().with_width(1.6).with_color(theme::alpha(*col, 0.95)).with_line_join(canvas::LineJoin::Round));
        }
        // Scanline shimmer.
        let sx = (self.phase.fract()) * (w + 120.0) - 60.0;
        f.fill_rectangle(Point::new(sx, 0.0), iced::Size::new(1.0, h), theme::alpha(Color::WHITE, 0.10));
        vec![f.into_geometry()]
    }
}

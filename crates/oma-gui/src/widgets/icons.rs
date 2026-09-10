//! Vector line icons drawn on a canvas (no icon font, no Unicode glyphs).

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};
use std::f32::consts::PI;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Dashboard,
    Cpu,
    Gpu,
    Fan,
    Light,
    Layers,
    Loop,
    Rog,
    Gear,
    Close,
    Window,
    Logo,
}

pub struct IconView {
    pub icon: Icon,
    pub color: Color,
    pub glow: Option<Color>,
}

pub fn icon<'a, M: 'a>(icon: Icon, color: Color, size: f32) -> iced::Element<'a, M> {
    canvas(IconView { icon, color, glow: None }).width(size).height(size).into()
}

impl<M> canvas::Program<M> for IconView {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _t: &Theme, bounds: Rectangle, _c: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let s = bounds.width.min(bounds.height);
        let c = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let w = (s * 0.075).max(1.2);
        let st = Stroke::default().with_width(w).with_color(self.color).with_line_cap(canvas::LineCap::Round).with_line_join(canvas::LineJoin::Round);
        let r = s * 0.36;
        if let Some(g) = self.glow {
            f.fill(&Path::circle(c, s * 0.55), Color { a: 0.18, ..g });
        }
        let pt = |x: f32, y: f32| Point::new(c.x + x * r, c.y + y * r);
        match self.icon {
            Icon::Dashboard => {
                f.stroke(&Path::new(|b| b.arc(canvas::path::Arc { center: c, radius: r, start_angle: (0.8 * PI).into(), end_angle: (2.2 * PI).into() })), st.clone());
                f.stroke(&Path::line(c, pt(0.55, -0.55)), st.clone());
                f.fill(&Path::circle(c, w * 0.9), self.color);
            }
            Icon::Cpu => {
                f.stroke(&Path::rectangle(pt(-0.6, -0.6), iced::Size::new(1.2 * r, 1.2 * r)), st.clone());
                f.stroke(&Path::rectangle(pt(-0.25, -0.25), iced::Size::new(0.5 * r, 0.5 * r)), st.clone());
                for k in [-0.35f32, 0.0, 0.35] {
                    f.stroke(&Path::line(pt(k, -0.6), pt(k, -0.95)), st.clone());
                    f.stroke(&Path::line(pt(k, 0.6), pt(k, 0.95)), st.clone());
                    f.stroke(&Path::line(pt(-0.6, k), pt(-0.95, k)), st.clone());
                    f.stroke(&Path::line(pt(0.6, k), pt(0.95, k)), st.clone());
                }
            }
            Icon::Gpu => {
                f.stroke(&Path::new(|b| { b.move_to(pt(-1.0, -0.55)); b.line_to(pt(0.9, -0.55)); b.line_to(pt(0.9, 0.45)); b.line_to(pt(-0.7, 0.45)); b.line_to(pt(-0.7, 0.8)); b.line_to(pt(-1.0, 0.8)); b.close(); }), st.clone());
                f.stroke(&Path::circle(pt(0.2, -0.05), 0.32 * r), st.clone());
                f.fill(&Path::circle(pt(0.2, -0.05), w * 0.8), self.color);
            }
            Icon::Fan => {
                f.fill(&Path::circle(c, w * 1.1), self.color);
                for k in 0..3 {
                    let a = k as f32 * 2.0 * PI / 3.0;
                    let blade = Path::new(|b| {
                        b.move_to(c);
                        b.bezier_curve_to(Point::new(c.x + r * 0.9 * (a + 0.9).cos(), c.y + r * 0.9 * (a + 0.9).sin()), Point::new(c.x + r * 1.05 * (a - 0.15).cos(), c.y + r * 1.05 * (a - 0.15).sin()), Point::new(c.x + r * 0.55 * (a - 0.55).cos(), c.y + r * 0.55 * (a - 0.55).sin()));
                        b.close();
                    });
                    f.stroke(&blade, st.clone());
                }
            }
            Icon::Light => {
                f.stroke(&Path::new(|b| b.arc(canvas::path::Arc { center: pt(0.0, -0.15), radius: r * 0.7, start_angle: (0.75 * PI).into(), end_angle: (2.25 * PI).into() })), st.clone());
                f.stroke(&Path::line(pt(-0.5, 0.35), pt(-0.3, 0.75)), st.clone());
                f.stroke(&Path::line(pt(0.5, 0.35), pt(0.3, 0.75)), st.clone());
                f.stroke(&Path::line(pt(-0.3, 0.75), pt(0.3, 0.75)), st.clone());
                f.stroke(&Path::line(pt(-0.22, 1.0), pt(0.22, 1.0)), st.clone());
            }
            Icon::Layers => {
                for (i, y) in [-0.45f32, 0.0, 0.45].iter().enumerate() {
                    let p = Path::new(|b| { b.move_to(pt(-0.95, *y)); b.line_to(pt(0.0, y - 0.5)); b.line_to(pt(0.95, *y)); b.line_to(pt(0.0, y + 0.5)); b.close(); });
                    if i == 0 { f.fill(&p, Color { a: 0.35, ..self.color }); }
                    f.stroke(&p, st.clone());
                }
            }
            Icon::Loop => {
                f.stroke(&Path::new(|b| b.arc(canvas::path::Arc { center: c, radius: r * 0.85, start_angle: (-0.35 * PI).into(), end_angle: (1.25 * PI).into() })), st.clone());
                f.stroke(&Path::new(|b| { b.move_to(pt(0.95, -0.85)); b.line_to(pt(0.95, -0.35)); b.line_to(pt(0.45, -0.35)); }), st.clone());
            }
            Icon::Rog => {
                // stylised eye
                f.stroke(&Path::new(|b| { b.move_to(pt(-1.0, 0.0)); b.quadratic_curve_to(pt(0.0, -1.0), pt(1.0, 0.0)); b.quadratic_curve_to(pt(0.0, 0.85), pt(-1.0, 0.0)); }), st.clone());
                f.fill(&Path::circle(c, r * 0.3), self.color);
            }
            Icon::Gear => {
                let path = Path::new(|b| {
                    let n = 8;
                    for k in 0..(n * 2) {
                        let a = k as f32 * PI / n as f32 - PI / (2.0 * n as f32);
                        let rad = if k % 2 == 0 { r } else { r * 0.72 };
                        let p = Point::new(c.x + rad * a.cos(), c.y + rad * a.sin());
                        if k == 0 { b.move_to(p) } else { b.line_to(p) }
                    }
                    b.close();
                });
                f.stroke(&path, st.clone());
                f.stroke(&Path::circle(c, r * 0.3), st.clone());
            }
            Icon::Close => {
                f.stroke(&Path::line(pt(-0.6, -0.6), pt(0.6, 0.6)), st.clone());
                f.stroke(&Path::line(pt(0.6, -0.6), pt(-0.6, 0.6)), st.clone());
            }
            Icon::Window => {
                f.stroke(&Path::rectangle(pt(-0.9, -0.7), iced::Size::new(1.8 * r, 1.4 * r)), st.clone());
                f.stroke(&Path::line(pt(-0.9, -0.3), pt(0.9, -0.3)), st.clone());
            }
            Icon::Logo => {
                // OmaAsus mark: a glossy liquid drop-orb with an iris ring and a blade of light.
                let rr = s * 0.30;
                for (k, a) in [(1.6, 0.05), (1.32, 0.09), (1.12, 0.15)] {
                    f.fill(&Path::circle(c, rr * k), Color { a, ..self.color });
                }
                // Drop body: circle with a tail toward the top-right. Angles: 0 = right, π/2 = down.
                let a0: f32 = 0.20; // tangent on the right
                let a1: f32 = -1.75 + 2.0 * PI; // tangent near the top (≈ -100°)
                let tip = Point::new(c.x + rr * 1.02, c.y - rr * 1.02);
                let drop = Path::new(|b| {
                    b.move_to(tip);
                    b.quadratic_curve_to(Point::new(c.x + rr * 1.05, c.y - rr * 0.25), Point::new(c.x + rr * a0.cos(), c.y + rr * a0.sin()));
                    b.arc(canvas::path::Arc { center: c, radius: rr, start_angle: a0.into(), end_angle: a1.into() });
                    b.quadratic_curve_to(Point::new(c.x + rr * 0.25, c.y - rr * 1.05), tip);
                    b.close();
                });
                f.fill(
                    &drop,
                    canvas::Fill {
                        style: canvas::Style::Gradient(canvas::Gradient::Linear(canvas::gradient::Linear::new(Point::new(c.x - rr, c.y - rr), Point::new(c.x + rr * 0.7, c.y + rr)).add_stop(0.0, lighten(self.color, 0.55)).add_stop(0.5, self.color).add_stop(1.0, darken(self.color, 0.5)))),
                        rule: canvas::fill::Rule::NonZero,
                    },
                );
                // Iris ring.
                f.stroke(&Path::circle(c, rr * 0.5), Stroke::default().with_width(s * 0.075).with_color(Color::from_rgba(0.02, 0.02, 0.05, 0.85)));
                f.stroke(&Path::circle(c, rr * 0.5), Stroke::default().with_width(s * 0.028).with_color(Color { a: 0.95, ..lighten(self.color, 0.65) }));
                // Blade of light.
                let (b0, b1) = (Point::new(c.x - rr * 0.45, c.y + rr * 0.72), Point::new(c.x + rr * 0.5, c.y - rr * 0.82));
                f.stroke(&Path::line(b0, b1), Stroke::default().with_width(s * 0.10).with_color(Color::from_rgba(0.02, 0.02, 0.05, 0.9)).with_line_cap(canvas::LineCap::Round));
                f.stroke(&Path::line(b0, b1), Stroke::default().with_width(s * 0.038).with_color(Color::WHITE).with_line_cap(canvas::LineCap::Round));
                // Specular highlight + sparkle.
                f.fill(&ellipse(Point::new(c.x - rr * 0.40, c.y - rr * 0.42), rr * 0.30, rr * 0.15, -0.7), Color::from_rgba(1.0, 1.0, 1.0, 0.9));
                f.fill(&Path::circle(Point::new(c.x + rr * 0.3, c.y + rr * 0.5), rr * 0.07), Color::from_rgba(1.0, 1.0, 1.0, 0.7));
            }
        }
        vec![f.into_geometry()]
    }
}

fn canvas<M, P: canvas::Program<M>>(p: P) -> canvas::Canvas<P, M> {
    canvas::Canvas::new(p)
}

fn lighten(c: Color, t: f32) -> Color {
    Color::from_rgba(c.r + (1.0 - c.r) * t, c.g + (1.0 - c.g) * t, c.b + (1.0 - c.b) * t, c.a)
}

fn darken(c: Color, t: f32) -> Color {
    Color::from_rgba(c.r * (1.0 - t), c.g * (1.0 - t), c.b * (1.0 - t), c.a)
}

/// Ellipse path (four cubic segments), rotated by `rot` radians about its centre.
pub fn ellipse(center: Point, rx: f32, ry: f32, rot: f32) -> Path {
    let k = 0.5523;
    let (cs, sn) = (rot.cos(), rot.sin());
    let tr = |x: f32, y: f32| Point::new(center.x + x * cs - y * sn, center.y + x * sn + y * cs);
    Path::new(|b| {
        b.move_to(tr(rx, 0.0));
        b.bezier_curve_to(tr(rx, ry * k), tr(rx * k, ry), tr(0.0, ry));
        b.bezier_curve_to(tr(-rx * k, ry), tr(-rx, ry * k), tr(-rx, 0.0));
        b.bezier_curve_to(tr(-rx, -ry * k), tr(-rx * k, -ry), tr(0.0, -ry));
        b.bezier_curve_to(tr(rx * k, -ry), tr(rx, -ry * k), tr(rx, 0.0));
        b.close();
    })
}

//! Interactive fan-curve editor on a canvas: drag points, click empty space
//! to add, right-click a point to remove. Shows the live operating point.

use crate::theme::{self, Palette};
use iced::widget::canvas::{self, Frame, Path, Stroke, Text};
use iced::{Color, Event, Point, Rectangle, Renderer, Theme, mouse};

pub const T_MIN: f64 = 0.0;
pub const T_MAX: f64 = 110.0;

#[derive(Debug, Clone)]
pub enum CurveEvent {
    Move(usize, f64, f64),
    Add(f64, f64),
    Remove(usize),
    Commit,
}

pub struct CurveEditor<'a, M> {
    pub palette: Palette,
    pub points: &'a [(f64, f64)],
    pub color: Color,
    pub live: Option<(f64, f64)>,
    pub min_duty: f64,
    pub on_event: fn(CurveEvent) -> M,
    pub editable: bool,
}

#[derive(Debug, Default)]
pub struct State {
    drag: Option<usize>,
    hover: Option<usize>,
}

const PAD_L: f32 = 34.0;
const PAD_B: f32 = 22.0;
const PAD_T: f32 = 10.0;
const PAD_R: f32 = 12.0;

fn plot_rect(b: Rectangle) -> Rectangle {
    Rectangle { x: PAD_L, y: PAD_T, width: (b.width - PAD_L - PAD_R).max(1.0), height: (b.height - PAD_T - PAD_B).max(1.0) }
}

fn to_px(r: Rectangle, t: f64, d: f64) -> Point {
    let x = r.x + ((t - T_MIN) / (T_MAX - T_MIN)) as f32 * r.width;
    let y = r.y + (1.0 - (d / 100.0) as f32) * r.height;
    Point::new(x, y)
}

fn to_data(r: Rectangle, p: Point) -> (f64, f64) {
    let t = T_MIN + ((p.x - r.x) / r.width).clamp(0.0, 1.0) as f64 * (T_MAX - T_MIN);
    let d = (1.0 - ((p.y - r.y) / r.height).clamp(0.0, 1.0)) as f64 * 100.0;
    ((t * 2.0).round() / 2.0, d.round())
}

impl<M: Clone> canvas::Program<M> for CurveEditor<'_, M> {
    type State = State;

    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<M>> {
        let r = plot_rect(bounds);
        let pos = cursor.position_in(bounds);
        let hit = |p: Point| self.points.iter().position(|&(t, d)| to_px(r, t, d).distance(p) < 12.0);
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(p) = pos else { return None };
                if let Some(i) = state.drag {
                    if !self.editable {
                        return None;
                    }
                    let (t, d) = to_data(r, p);
                    // Keep temperatures monotonic between neighbours.
                    let lo = if i > 0 { self.points[i - 1].0 + 1.0 } else { T_MIN };
                    let hi = if i + 1 < self.points.len() { self.points[i + 1].0 - 1.0 } else { T_MAX };
                    let t = t.clamp(lo.min(hi), hi.max(lo));
                    return Some(canvas::Action::publish((self.on_event)(CurveEvent::Move(i, t, d))).and_capture());
                }
                let h = hit(p);
                if h != state.hover {
                    state.hover = h;
                    return Some(canvas::Action::request_redraw());
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = pos?;
                if !self.editable {
                    return None;
                }
                match hit(p) {
                    Some(i) => {
                        state.drag = Some(i);
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    None => {
                        if r.contains(p) {
                            let (t, d) = to_data(r, p);
                            Some(canvas::Action::publish((self.on_event)(CurveEvent::Add(t, d))).and_capture())
                        } else {
                            None
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let p = pos?;
                if !self.editable {
                    return None;
                }
                hit(p).filter(|_| self.points.len() > 2).map(|i| canvas::Action::publish((self.on_event)(CurveEvent::Remove(i))).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.drag.take().is_some() {
                    return Some(canvas::Action::publish((self.on_event)(CurveEvent::Commit)).and_capture());
                }
                None
            }
            _ => None,
        }
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if !self.editable {
            return mouse::Interaction::default();
        }
        if state.drag.is_some() {
            return mouse::Interaction::Grabbing;
        }
        if state.hover.is_some() {
            return mouse::Interaction::Grab;
        }
        if cursor.position_in(bounds).is_some_and(|p| plot_rect(bounds).contains(p)) { mouse::Interaction::Crosshair } else { mouse::Interaction::default() }
    }

    fn draw(&self, state: &State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        let p = self.palette;
        let mut frame = Frame::new(renderer, bounds.size());
        let r = plot_rect(bounds);
        // grid
        for i in 0..=10 {
            let y = r.y + r.height * i as f32 / 10.0;
            frame.stroke(&Path::line(Point::new(r.x, y), Point::new(r.x + r.width, y)), Stroke::default().with_width(1.0).with_color(if i % 5 == 0 { p.line_strong } else { p.line }));
            if i % 2 == 0 {
                frame.fill_text(Text { content: format!("{}%", 100 - i * 10), position: Point::new(r.x - 6.0, y), color: p.text_faint, size: 10.0.into(), font: theme::font::MONO, align_x: iced::alignment::Horizontal::Right.into(), align_y: iced::alignment::Vertical::Center, ..Default::default() });
            }
        }
        for t in (0..=110).step_by(10) {
            let x = to_px(r, t as f64, 0.0).x;
            frame.stroke(&Path::line(Point::new(x, r.y), Point::new(x, r.y + r.height)), Stroke::default().with_width(1.0).with_color(if t % 50 == 0 { p.line_strong } else { p.line }));
            if t % 20 == 0 {
                frame.fill_text(Text { content: format!("{t}°"), position: Point::new(x, r.y + r.height + 6.0), color: p.text_faint, size: 10.0.into(), font: theme::font::MONO, align_x: iced::alignment::Horizontal::Center.into(), align_y: iced::alignment::Vertical::Top, ..Default::default() });
            }
        }
        // min duty band
        if self.min_duty > 0.0 {
            let y = to_px(r, 0.0, self.min_duty).y;
            frame.fill_rectangle(Point::new(r.x, y), iced::Size::new(r.width, r.y + r.height - y), theme::alpha(p.warn, 0.05));
            frame.stroke(&Path::line(Point::new(r.x, y), Point::new(r.x + r.width, y)), Stroke::default().with_width(1.0).with_color(theme::alpha(p.warn, 0.5)));
        }
        if self.points.len() >= 2 {
            let mut pts: Vec<(f64, f64)> = self.points.to_vec();
            pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let first = to_px(r, T_MIN, pts[0].1.max(self.min_duty));
            let last = to_px(r, T_MAX, pts.last().unwrap().1.max(self.min_duty));
            let line = Path::new(|b| {
                b.move_to(first);
                for &(t, d) in &pts {
                    b.line_to(to_px(r, t, d.max(self.min_duty)));
                }
                b.line_to(last);
            });
            let area = Path::new(|b| {
                b.move_to(Point::new(first.x, r.y + r.height));
                b.line_to(first);
                for &(t, d) in &pts {
                    b.line_to(to_px(r, t, d.max(self.min_duty)));
                }
                b.line_to(last);
                b.line_to(Point::new(last.x, r.y + r.height));
                b.close();
            });
            frame.fill(
                &area,
                canvas::Fill {
                    style: canvas::Style::Gradient(canvas::Gradient::Linear(canvas::gradient::Linear::new(Point::new(0.0, r.y), Point::new(0.0, r.y + r.height)).add_stop(0.0, theme::alpha(self.color, 0.35)).add_stop(1.0, theme::alpha(self.color, 0.02)))),
                    rule: canvas::fill::Rule::NonZero,
                },
            );
            frame.stroke(&line, Stroke::default().with_width(2.5).with_color(self.color).with_line_join(canvas::LineJoin::Round).with_line_cap(canvas::LineCap::Round));
        }
        // points
        for (i, &(t, d)) in self.points.iter().enumerate() {
            let c = to_px(r, t, d);
            let active = state.drag == Some(i) || state.hover == Some(i);
            if active {
                frame.fill(&Path::circle(c, 12.0), theme::alpha(self.color, 0.25));
            }
            frame.fill(&Path::circle(c, if active { 7.0 } else { 5.5 }), Color::WHITE);
            frame.fill(&Path::circle(c, if active { 4.5 } else { 3.0 }), self.color);
            if active {
                frame.fill_text(Text { content: format!("{t:.0}° → {d:.0}%"), position: Point::new(c.x, c.y - 16.0), color: p.text, size: 11.0.into(), font: theme::font::MONO, align_x: iced::alignment::Horizontal::Center.into(), align_y: iced::alignment::Vertical::Bottom, ..Default::default() });
            }
        }
        // live operating point
        if let Some((t, d)) = self.live {
            let c = to_px(r, t.clamp(T_MIN, T_MAX), d.clamp(0.0, 100.0));
            frame.stroke(&Path::line(Point::new(c.x, r.y), Point::new(c.x, r.y + r.height)), Stroke::default().with_width(1.0).with_color(theme::alpha(p.text, 0.25)));
            frame.fill(&Path::circle(c, 9.0), theme::alpha(p.accent_2, 0.3));
            frame.fill(&Path::circle(c, 4.5), p.accent_2);
            frame.fill_text(Text { content: format!("now {t:.0}° · {d:.0}%"), position: Point::new(c.x + 10.0, r.y + 4.0), color: p.accent_2, size: 11.0.into(), font: theme::font::MONO, align_x: iced::alignment::Horizontal::Left.into(), align_y: iced::alignment::Vertical::Top, ..Default::default() });
        }
        vec![frame.into_geometry()]
    }
}

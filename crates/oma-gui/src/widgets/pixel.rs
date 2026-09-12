//! The site's 3x5 pixel glyph font (`PixelLabel`) with the five brand bands
//! (crest → hover → lit → mid → dim, top to bottom), and the original OmaAsus mark.

use crate::theme::Palette;
use iced::widget::canvas::{self, Frame};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};

fn glyph(c: char) -> Option<&'static str> {
    Some(match c {
        'A' => "111 101 111 101 101",
        'B' => "110 101 110 101 110",
        'C' => "111 100 100 100 111",
        'D' => "110 101 101 101 110",
        'E' => "111 100 111 100 111",
        'F' => "111 100 111 100 100",
        'G' => "111 100 101 101 111",
        'H' => "101 101 111 101 101",
        'I' => "111 010 010 010 111",
        'J' => "001 001 001 101 111",
        'K' => "101 101 110 101 101",
        'L' => "100 100 100 100 111",
        'M' => "101 111 111 101 101",
        'N' => "110 101 101 101 101",
        'O' => "111 101 101 101 111",
        'P' => "111 101 111 100 100",
        'Q' => "111 101 101 111 001",
        'R' => "111 101 110 101 101",
        'S' => "111 100 111 001 111",
        'T' => "111 010 010 010 010",
        'U' => "101 101 101 101 111",
        'V' => "101 101 101 101 010",
        'W' => "101 101 111 111 101",
        'X' => "101 101 010 101 101",
        'Y' => "101 101 010 010 010",
        'Z' => "111 001 010 100 111",
        '0' => "111 101 101 101 111",
        '1' => "010 110 010 010 111",
        '2' => "111 001 111 100 111",
        '3' => "111 001 111 001 111",
        '4' => "101 101 111 001 001",
        '5' => "111 100 111 001 111",
        '6' => "111 100 111 101 111",
        '7' => "111 001 001 001 001",
        '8' => "111 101 111 101 111",
        '9' => "111 101 111 001 111",
        '.' => "000 000 000 000 010",
        '/' => "001 001 010 100 100",
        '-' => "000 000 111 000 000",
        _ => return None,
    })
}

/// Width in cells of a label.
pub fn label_width(text: &str) -> usize {
    let mut x: usize = 0;
    for ch in text.to_uppercase().chars() {
        x += if ch == ' ' { 2 + 1 } else { 3 + 1 };
    }
    x.saturating_sub(1)
}

/// Band colour for a row 0..5 of the glyph (top to bottom), as the site's
/// wordmark: crest, crest, hover, lit, mid/dim.
pub fn band(p: &Palette, row: usize) -> Color {
    match row {
        0 => p.field_crest,
        1 => p.field_hover,
        2 => p.field_lit,
        3 => p.field_mid,
        _ => p.field_dim,
    }
}

pub struct PixelLabel {
    pub palette: Palette,
    pub text: String,
    /// `None` = banded brand gradient; `Some` = flat colour.
    pub color: Option<Color>,
}

impl<M> canvas::Program<M> for PixelLabel {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _t: &Theme, bounds: Rectangle, _c: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let up = self.text.to_uppercase();
        let wcells = label_width(&up).max(1) as f32;
        let cell = (bounds.width / wcells).min(bounds.height / 5.0).floor().max(1.0);
        let x0 = ((bounds.width - wcells * cell) / 2.0).floor();
        let y0 = ((bounds.height - 5.0 * cell) / 2.0).floor();
        let mut x = 0usize;
        for ch in up.chars() {
            if ch == ' ' {
                x += 3;
                continue;
            }
            if let Some(g) = glyph(ch) {
                for (row, bits) in g.split(' ').enumerate() {
                    for (col, b) in bits.chars().enumerate() {
                        if b == '1' {
                            let color = self.color.unwrap_or_else(|| band(&self.palette, row));
                            f.fill_rectangle(Point::new(x0 + (x + col) as f32 * cell, y0 + row as f32 * cell), Size::new(cell, cell), color);
                        }
                    }
                }
            }
            x += 4;
        }
        vec![f.into_geometry()]
    }
}

pub fn pixel_label<'a, M: 'a>(p: Palette, text: impl Into<String>, cell: f32) -> iced::Element<'a, M> {
    let text = text.into();
    let w = label_width(&text) as f32 * cell;
    canvas::Canvas::new(PixelLabel { palette: p, text, color: None }).width(w).height(cell * 5.0).into()
}

/// Original OA monogram, shared with the tray and packaged application icons.
/// A single SVG source keeps the small panel and main-window branding identical.
pub fn oma_mark<'a, M: 'a>(p: Palette, size: f32) -> iced::Element<'a, M> {
    use iced::widget::svg;
    svg(svg::Handle::from_memory(include_bytes!("../../assets/icons/omaasus-symbolic.svg").as_slice()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(p.brand) })
        .into()
}

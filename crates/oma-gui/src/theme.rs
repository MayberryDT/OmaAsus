//! OmaAsus design system.
//!
//! One dark "obsidian glass" theme with a single electric accent that the
//! active profile recolours. Every token lives here; widgets never hard-code
//! colours.

use iced::{Border, Color, Shadow, Vector, color};

/// Font handles. The families are bundled from `assets/fonts` (OFL).
pub mod font {
    use iced::Font;
    pub const DISPLAY: Font = Font::with_name("Space Grotesk");
    pub const BODY: Font = Font::with_name("Inter Variable");
    pub const MONO: Font = Font::with_name("JetBrains Mono");
    pub const BODY_BYTES: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
    pub const DISPLAY_BYTES: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk.ttf");
    pub const MONO_BYTES: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono.ttf");
}

/// Spacing scale (px).
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Type scale (px).
pub mod size {
    pub const CAPTION: f32 = 11.0;
    pub const SMALL: f32 = 12.5;
    pub const BODY: f32 = 14.0;
    pub const LEAD: f32 = 16.0;
    pub const TITLE: f32 = 20.0;
    pub const HEADLINE: f32 = 28.0;
}

pub mod radius {
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 14.0;
    pub const LG: f32 = 20.0;
    pub const PILL: f32 = 999.0;
}

/// Colour tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub bg_elev: Color,
    pub glass: Color,
    pub glass_strong: Color,
    pub line: Color,
    pub line_strong: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub accent_glow: Color,
    pub accent_2: Color,
    pub ok: Color,
    pub warn: Color,
    pub danger: Color,
    pub cpu: Color,
    pub gpu: Color,
    pub coolant: Color,
    pub fan: Color,
    pub power: Color,
}

pub const OBSIDIAN: Palette = Palette {
    bg: Color::from_rgba(0.043, 0.047, 0.063, 0.90),
    bg_elev: Color::from_rgba(0.075, 0.082, 0.106, 0.92),
    glass: Color::from_rgba(1.0, 1.0, 1.0, 0.035),
    glass_strong: Color::from_rgba(1.0, 1.0, 1.0, 0.07),
    line: Color::from_rgba(1.0, 1.0, 1.0, 0.07),
    line_strong: Color::from_rgba(1.0, 1.0, 1.0, 0.16),
    text: color!(0xF3F4F8),
    text_dim: Color::from_rgba(0.953, 0.957, 0.973, 0.62),
    text_faint: Color::from_rgba(0.953, 0.957, 0.973, 0.38),
    accent: color!(0xFF3D68),
    accent_soft: Color::from_rgba(1.0, 0.239, 0.408, 0.18),
    accent_glow: Color::from_rgba(1.0, 0.239, 0.408, 0.45),
    accent_2: color!(0xFFB13D),
    ok: color!(0x3DFFB1),
    warn: color!(0xFFB13D),
    danger: color!(0xFF4D4D),
    cpu: color!(0x7C8CFF),
    gpu: color!(0x3DD6FF),
    coolant: color!(0x5EE6C8),
    fan: color!(0xB58CFF),
    power: color!(0xFFB13D),
};

impl Palette {
    /// Recolour the accent (profile accents).
    pub fn with_accent(mut self, rgb: (u8, u8, u8)) -> Self {
        let a = Color::from_rgb8(rgb.0, rgb.1, rgb.2);
        self.accent = a;
        self.accent_soft = Color { a: 0.18, ..a };
        self.accent_glow = Color { a: 0.45, ..a };
        self
    }
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, a.a + (b.a - a.a) * t)
}

/// Thermal colour ramp: cool → warm → hot.
pub fn thermal(p: &Palette, t: f64, lo: f64, hi: f64) -> Color {
    let f = (((t - lo) / (hi - lo)).clamp(0.0, 1.0)) as f32;
    if f < 0.5 { mix(p.coolant, p.warn, f * 2.0) } else { mix(p.warn, p.danger, (f - 0.5) * 2.0) }
}

pub fn card_border(p: &Palette) -> Border {
    Border { color: p.line, width: 1.0, radius: radius::LG.into() }
}

pub fn card_shadow() -> Shadow {
    Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 }
}

pub fn glow(c: Color) -> Shadow {
    Shadow { color: alpha(c, 0.35), offset: Vector::new(0.0, 0.0), blur_radius: 18.0 }
}

/// Build the iced theme so built-in widgets (text input, scrollbars) match.
pub fn iced_theme(p: &Palette) -> iced::Theme {
    iced::Theme::custom(
        String::from("OmaAsus"),
        iced::theme::Palette { background: p.bg, text: p.text, primary: p.accent, success: p.ok, warning: p.warn, danger: p.danger },
    )
}

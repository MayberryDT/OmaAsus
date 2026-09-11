//! OmaAsus design system — a faithful port of the omarchy-site token model.
//!
//! Colours come from the user's *active Omarchy theme* (`omarchy-theme-current`
//! → `themes/<slug>/colors.toml`), mixed into the same roles the site uses:
//! `bg-deep`, `bg`, `surface`, `surface-2`, `border-subtle`, `border-strong`,
//! `text`, `text-secondary`, `text-muted`, `brand`, `brand-soft`, `brand-ink`,
//! and the pixel-field bands `field-{bg,dim,mid,lit,hover,crest}`.
//! Corner radius is zero everywhere, as on the site.

use iced::{Border, Color};
use std::path::PathBuf;

#[allow(dead_code)]
pub mod font {
    use iced::font::{Family, Weight};
    use iced::Font;
    /// Geist Variable — prose, headings, controls.
    pub const SANS: Font = Font::with_name("Geist");
    pub const SANS_MEDIUM: Font = Font { family: Family::Name("Geist"), weight: Weight::Medium, ..Font::DEFAULT };
    pub const SANS_SEMIBOLD: Font = Font { family: Family::Name("Geist"), weight: Weight::Semibold, ..Font::DEFAULT };
    /// JetBrains Mono Variable — nav, labels, values, eyebrows.
    pub const MONO: Font = Font::with_name("JetBrains Mono");
    pub const MONO_MEDIUM: Font = Font { family: Family::Name("JetBrains Mono"), weight: Weight::Medium, ..Font::DEFAULT };
    pub const SANS_BYTES: &[u8] = include_bytes!("../assets/fonts/Geist.ttf");
    pub const MONO_BYTES: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono.ttf");
    // Kept for the display numerals on gauges.
    pub const BODY: Font = SANS;
    pub const BODY_MEDIUM: Font = SANS_MEDIUM;
    pub const DISPLAY: Font = SANS_SEMIBOLD;
    pub const DISPLAY_MEDIUM: Font = SANS_MEDIUM;
    pub const DISPLAY_LIGHT: Font = Font { family: Family::Name("Geist"), weight: Weight::Light, ..Font::DEFAULT };
}

/// Spacing scale (px), 4-based like Tailwind.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Type scale (px) mirroring the site's Tailwind sizes.
pub mod size {
    pub const MICRO: f32 = 11.0;
    pub const CAPTION: f32 = 12.0;
    pub const SMALL: f32 = 13.0;
    pub const BODY: f32 = 14.0;
    pub const LEAD: f32 = 15.0;
    pub const TITLE: f32 = 18.0;
    pub const HEADLINE: f32 = 22.0;
    pub const DISPLAY: f32 = 28.0;
}

#[allow(dead_code)]
pub mod radius {
    pub const SM: f32 = 0.0;
    pub const MD: f32 = 0.0;
    pub const LG: f32 = 0.0;
    pub const PILL: f32 = 0.0;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg_deep: Color,
    pub bg: Color,
    pub surface: Color,
    pub surface_2: Color,
    pub border_subtle: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub brand: Color,
    pub brand_soft: Color,
    pub brand_ink: Color,
    pub field_bg: Color,
    pub field_dim: Color,
    pub field_mid: Color,
    pub field_lit: Color,
    pub field_hover: Color,
    pub field_crest: Color,
    pub selection: Color,
    // semantic
    pub red: Color,
    pub yellow: Color,
    pub green: Color,
    pub cyan: Color,
    pub blue: Color,
    pub magenta: Color,
    pub orange: Color,
    // --- compatibility aliases used across pages ---
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
    pub glass: Color,
    pub glass_strong: Color,
    pub line: Color,
    pub line_strong: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub bg_elev: Color,
}

fn hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let v = u32::from_str_radix(&h[..6], 16).ok()?;
    Some(Color::from_rgb8((v >> 16) as u8, (v >> 8 & 0xff) as u8, (v & 0xff) as u8))
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, a.a + (b.a - a.a) * t)
}

/// Thermal ramp on the theme's own semantic colours.
pub fn thermal(p: &Palette, t: f64, lo: f64, hi: f64) -> Color {
    let f = (((t - lo) / (hi - lo)).clamp(0.0, 1.0)) as f32;
    if f < 0.5 { mix(p.brand, p.yellow, f * 2.0) } else { mix(p.yellow, p.red, (f - 0.5) * 2.0) }
}

/// Raw Omarchy `colors.toml`.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct OmarchyColors {
    pub mode: Option<String>,
    pub accent: Option<String>,
    pub selection: Option<String>,
    pub muted: Option<String>,
    pub background: Option<String>,
    pub dark_background: Option<String>,
    pub darker_background: Option<String>,
    pub lighter_background: Option<String>,
    pub foreground: Option<String>,
    pub dark_foreground: Option<String>,
    pub light_foreground: Option<String>,
    pub bright_foreground: Option<String>,
    pub red: Option<String>,
    pub yellow: Option<String>,
    pub orange: Option<String>,
    pub green: Option<String>,
    pub cyan: Option<String>,
    pub blue: Option<String>,
    pub magenta: Option<String>,
}

/// The theme the desktop is running, as `omarchy-theme-current` reports it.
pub fn current_theme_name() -> Option<String> {
    let out = std::process::Command::new("omarchy-theme-current").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub fn theme_slug(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

pub fn theme_dir(slug: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".config/omarchy/themes").join(slug));
    }
    candidates.push(PathBuf::from("/usr/share/omarchy/themes").join(slug));
    candidates.into_iter().find(|p| p.join("colors.toml").exists())
}

pub fn load_colors(slug: &str) -> Option<OmarchyColors> {
    let dir = theme_dir(slug)?;
    let s = std::fs::read_to_string(dir.join("colors.toml")).ok()?;
    toml::from_str(&s).ok()
}

/// Osaka Jade, the site's values, used when no Omarchy theme is available.
pub fn fallback() -> Palette {
    palette_from(&OmarchyColors {
        mode: Some("dark".into()),
        accent: Some("#509475".into()),
        selection: Some("#32473B".into()),
        muted: Some("#53685B".into()),
        background: Some("#111c18".into()),
        dark_background: Some("#0c1512".into()),
        darker_background: Some("#090f0d".into()),
        lighter_background: Some("#23372B".into()),
        foreground: Some("#C1C497".into()),
        dark_foreground: Some("#81B8A8".into()),
        light_foreground: Some("#D6D5BC".into()),
        bright_foreground: Some("#F7E8B2".into()),
        red: Some("#FF5345".into()),
        yellow: Some("#E5C736".into()),
        orange: Some("#a2734b".into()),
        green: Some("#63b07a".into()),
        cyan: Some("#2DD5B7".into()),
        blue: Some("#509475".into()),
        magenta: Some("#D2689C".into()),
    }, "osaka-jade")
}

/// Build the palette the way the site mixes its intermediate shades.
pub fn palette_from(c: &OmarchyColors, slug: &str) -> Palette {
    let dark = c.mode.as_deref().unwrap_or("dark") != "light";
    let get = |v: &Option<String>, d: &str| v.as_deref().and_then(hex).unwrap_or_else(|| hex(d).unwrap());
    let bg = get(&c.background, "#1a1b26");
    let bg_deep = get(&c.dark_background, "#13141c");
    let field_bg = get(&c.darker_background, "#0e0e14");
    let surface_2 = get(&c.lighter_background, "#24283b");
    let surface = mix(bg, surface_2, 0.5);
    let border_strong = get(&c.muted, "#414868");
    let text = get(&c.bright_foreground, "#c0caf5");
    let text_secondary = get(&c.foreground, "#a9b1d6");
    let text_muted = mix(text_secondary, bg, 0.22);
    // The site keys some brands to a semantic colour rather than the accent.
    let brand = match slug {
        "tokyo-night" => get(&c.green, "#9ece6a"),
        _ => get(&c.accent, "#9ece6a"),
    };
    let white = Color::WHITE;
    let field_dim = mix(brand, field_bg, 0.68);
    let field_mid = mix(brand, field_bg, 0.36);
    let field_hover = mix(brand, white, 0.35);
    let field_crest = mix(brand, white, 0.70);
    let red = get(&c.red, "#f7768e");
    let yellow = get(&c.yellow, "#e0af68");
    let green = get(&c.green, "#9ece6a");
    let cyan = get(&c.cyan, "#7dcfff");
    let blue = get(&c.blue, "#7aa2f7");
    let magenta = get(&c.magenta, "#bb9af7");
    let orange = get(&c.orange, "#ff9e64");
    let ring = if dark { alpha(white, 0.07) } else { alpha(Color::BLACK, 0.06) };
    let ring_strong = if dark { alpha(white, 0.13) } else { alpha(Color::BLACK, 0.09) };
    Palette {
        dark,
        bg_deep,
        bg,
        surface,
        surface_2,
        border_subtle: surface_2,
        border_strong,
        text,
        text_secondary,
        text_muted,
        brand,
        brand_soft: alpha(brand, 0.12),
        brand_ink: if dark { hex("#0c0e10").unwrap() } else { white },
        field_bg,
        field_dim,
        field_mid,
        field_lit: brand,
        field_hover,
        field_crest,
        selection: get(&c.selection, "#292e42"),
        red,
        yellow,
        green,
        cyan,
        blue,
        magenta,
        orange,
        accent: brand,
        accent_soft: alpha(brand, 0.12),
        accent_glow: alpha(brand, 0.35),
        accent_2: cyan,
        ok: green,
        warn: yellow,
        danger: red,
        cpu: blue,
        gpu: cyan,
        coolant: cyan,
        fan: magenta,
        power: orange,
        glass: surface,
        glass_strong: surface_2,
        line: ring,
        line_strong: ring_strong,
        text_dim: text_secondary,
        text_faint: text_muted,
        bg_elev: surface,
    }
}

/// Load the desktop's theme (or the Osaka Jade fallback).
pub fn load() -> (Palette, String) {
    if let Some(name) = current_theme_name() {
        let slug = theme_slug(&name);
        if let Some(c) = load_colors(&slug) {
            return (palette_from(&c, &slug), name);
        }
    }
    (fallback(), "Osaka Jade".into())
}

pub fn ring(p: &Palette) -> Border {
    Border { color: p.line, width: 1.0, radius: 0.0.into() }
}

pub fn iced_theme(p: &Palette) -> iced::Theme {
    iced::Theme::custom(
        String::from("Omarchy"),
        iced::theme::Palette { background: p.bg, text: p.text, primary: p.brand, success: p.green, warning: p.yellow, danger: p.red },
    )
}

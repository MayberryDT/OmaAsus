//! Pages of the control center. Each page renders from `&App` state and
//! returns `Element<Message>`.

pub mod asus;
pub mod automation;
pub mod cooling;
pub mod cpu;
pub mod dashboard;
pub mod gpu;
pub mod lighting;
pub mod profiles;
pub mod quick;
pub mod settings;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Page {
    #[default]
    Dashboard,
    Cpu,
    Gpu,
    Cooling,
    Lighting,
    Profiles,
    Automation,
    Asus,
    Settings,
}

impl Page {
    pub const ALL: [Page; 9] = [Page::Dashboard, Page::Cpu, Page::Gpu, Page::Cooling, Page::Lighting, Page::Profiles, Page::Automation, Page::Asus, Page::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Page::Dashboard => "Dashboard",
            Page::Cpu => "Processor",
            Page::Gpu => "Graphics",
            Page::Cooling => "Cooling",
            Page::Lighting => "Lighting",
            Page::Profiles => "Profiles",
            Page::Automation => "Automation",
            Page::Asus => "ASUS",
            Page::Settings => "Settings",
        }
    }

    /// The page's icon, for narrow navigation and the tray panel.
    pub fn icon(self) -> crate::widgets::icons::Icon {
        use crate::widgets::icons::Icon;
        match self {
            Page::Dashboard => Icon::Dashboard,
            Page::Cpu => Icon::Cpu,
            Page::Gpu => Icon::Gpu,
            Page::Cooling => Icon::Fan,
            Page::Lighting => Icon::Light,
            Page::Profiles => Icon::Layers,
            Page::Automation => Icon::Loop,
            Page::Asus => Icon::Rog,
            Page::Settings => Icon::Gear,
        }
    }
}

/// How much of the window header fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTier {
    /// Wordmark, labelled links and the status pills.
    Full,
    /// Labelled links beside the mark alone.
    Labels,
    /// Icons with tooltips.
    Icons,
}

impl NavTier {
    /// The most of the header that fits `width`, given the pages, the header
    /// button's label and the status pills' labels.
    pub fn for_width(width: f32, pages: &[Page], button: &str, pills: &[&str]) -> Self {
        use crate::theme::{size, space};
        // JetBrains Mono advances 0.6 em, so widths can be sized from the text.
        let mono = |s: &str| s.chars().count() as f32 * size::SMALL * 0.6;
        let links = pages.iter().map(|pg| mono(pg.label()) + 20.0).sum::<f32>() + pages.len().saturating_sub(1) as f32 * space::XS;
        let pills: f32 = pills.iter().map(|s| mono(s) + 24.0 + space::SM).sum();
        let (mark, word) = (26.0, space::SM + mono("omaasus"));
        // Less the header's padding, the gaps around links and filler, and the button.
        let room = width - 5.0 * space::XL - (button.chars().count() as f32 * size::BODY * 0.6 + 32.0) - 16.0;
        if room >= mark + word + links + pills {
            Self::Full
        } else if room >= mark + links {
            Self::Labels
        } else {
            Self::Icons
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_gives_way_in_order() {
        let fit = |w| NavTier::for_width(w, &Page::ALL, "Performance", &["Osaka Jade", "helper"]);
        assert_eq!(fit(1920.0), NavTier::Full);
        assert_eq!(fit(1100.0), NavTier::Labels);
        // A half-width tile on a laptop: every page stays reachable as an icon.
        assert_eq!(fit(710.0), NavTier::Icons);
        assert_eq!(NavTier::for_width(1100.0, &Page::ALL[..5], "Performance", &["Osaka Jade", "helper"]), NavTier::Full, "fewer pages, more room");
    }
}

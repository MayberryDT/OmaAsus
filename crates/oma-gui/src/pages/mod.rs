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

}

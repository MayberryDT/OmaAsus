//! # oma-hw
//!
//! Hardware abstraction layer for OmaAsus. Everything here is safe to call
//! from an unprivileged process; write paths are described as data
//! (`(path, value)` plans, [`nvidia::NvidiaControl`] structs, ...) so that
//! they can be shipped to the privileged helper over D-Bus.

pub mod amdgpu;
pub mod asusd;
pub mod capture;
pub mod coolercontrol;
pub mod cpu;
pub mod detect;
pub mod fanengine;
pub mod gamemode;
pub mod helper;
pub mod hwmon;
pub mod hypr;
pub mod knowledge;
pub mod lianli;
pub mod lighting;
pub mod livedash;
pub mod model;
pub mod nvidia;
pub mod ppd;
pub mod profile;
pub mod rgb;
pub mod supergfx;
pub mod sysfs;

pub use detect::{Features, Platform, SystemInventory};
pub use profile::{Config, Profile};

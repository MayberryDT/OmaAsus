//! Load/save the user config (`~/.config/omaasus/config.toml`).

use oma_hw::profile::Config;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    directories::ProjectDirs::from("com", "omaasus", "omaasus")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("omaasus.toml"))
}

pub fn load() -> Config {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(s) => match toml::from_str::<Config>(&s) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, path = %p.display(), "config unreadable; using defaults");
                Config::with_builtin_profiles()
            }
        },
        Err(_) => {
            let c = Config::with_builtin_profiles();
            save(&c);
            c
        }
    }
}

pub fn save(c: &Config) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match toml::to_string_pretty(c) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&p, s) {
                tracing::error!(error = %e, "cannot save config");
            }
        }
        Err(e) => tracing::error!(error = %e, "cannot serialise config"),
    }
}

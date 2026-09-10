//! `omaasus` — control center + overlay for ASUS gaming hardware.
//!
//! Entry points:
//! * `omaasus`            — start the daemon in the background (tray-less) and
//!                          open the main window.
//! * `omaasus --overlay`  — start in the background; the overlay is toggled
//!                          with `omaasus toggle` (bind it to a Hyprland key).
//! * `omaasus toggle|show|hide|window` — talk to a running instance.

mod app;
mod apply;
mod automation;
mod config_store;
mod fans;
mod install;
mod ipc;
mod theme;
mod widgets;
mod pages;
mod telemetry;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("toggle") | Some("show") | Some("hide") | Some("window") | Some("profile") | Some("page") => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(ipc::send(&args))?;
            Ok(())
        }
        _ => {
            let overlay_only = args.iter().any(|a| a == "--overlay");
            app::run(overlay_only).map_err(|e| anyhow::anyhow!("{e}"))
        }
    }
}

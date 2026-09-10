//! `omaasus` — control center + overlay for ASUS gaming hardware.
//!
//! Entry points:
//! * `omaasus`            — start the daemon (with a tray item in the bar) and
//!                          open the main window.
//! * `omaasus --overlay`  — start in the background; the overlay is toggled
//!                          with `omaasus toggle` (bind it to a Hyprland key).
//! * `omaasus toggle|show|hide|window|quit` — talk to a running instance.
//!   `omaasus window` starts the app when nothing is running (desktop entry).

mod app;
mod apply;
mod automation;
mod config_store;
mod events;
mod fans;
mod install;
mod ipc;
mod theme;
mod tray;
mod widgets;
mod pages;
mod telemetry;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("window") => {
            // The desktop entry: raise the running instance, or become it.
            let rt = tokio::runtime::Runtime::new()?;
            if rt.block_on(ipc::running()) {
                rt.block_on(ipc::send(&args))?;
                return Ok(());
            }
            drop(rt);
            app::run(false).map_err(|e| anyhow::anyhow!("{e}"))
        }
        Some("toggle") | Some("show") | Some("hide") | Some("profile") | Some("page") | Some("quit") => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(ipc::send(&args))?;
            Ok(())
        }
        _ => {
            let overlay_only = args.iter().any(|a| a == "--overlay");
            let rt = tokio::runtime::Runtime::new()?;
            if rt.block_on(ipc::running()) {
                // One fan engine per session: hand the request to the running instance.
                if !overlay_only {
                    rt.block_on(ipc::send(&["window".to_string()]))?;
                }
                tracing::info!("OmaAsus is already running");
                return Ok(());
            }
            drop(rt);
            app::run(overlay_only).map_err(|e| anyhow::anyhow!("{e}"))
        }
    }
}

//! Load/save the user config (`~/.config/omaasus/config.toml`).
//!
//! Saves are debounced onto a background thread and written atomically, so
//! slider drags never block the UI and a crash can't leave a half-written
//! file. A config that fails to parse is moved aside, never overwritten.

use oma_hw::profile::Config;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Quiet period before a burst of saves is written.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// Set when the existing file could not be read or moved aside: never write over it.
static READ_ONLY: AtomicBool = AtomicBool::new(false);

enum Job {
    Save(Box<Config>),
    Flush(mpsc::Sender<()>),
}

pub fn path() -> PathBuf {
    directories::ProjectDirs::from("com", "omaasus", "omaasus")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("omaasus.toml"))
}

/// Load the config. The second value is a notice for the user when the file
/// on disk could not be used.
pub fn load() -> (Config, Option<String>) {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(s) => match toml::from_str::<Config>(&s) {
            Ok(c) => (c, None),
            Err(e) => {
                tracing::error!(error = %e, path = %p.display(), "config unreadable");
                let aside = p.with_file_name(format!("config.toml.broken-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
                let c = Config::with_builtin_profiles();
                match std::fs::rename(&p, &aside) {
                    Ok(()) => {
                        write_atomic(&p, &c);
                        (c, Some(format!("Config could not be read; kept it as {} and started with defaults", aside.display())))
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "cannot move unreadable config aside; saving disabled");
                        READ_ONLY.store(true, Ordering::Relaxed);
                        (c, Some(format!("Config could not be read ({e}); running with defaults and not saving")))
                    }
                }
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let c = Config::with_builtin_profiles();
            write_atomic(&p, &c);
            (c, None)
        }
        Err(e) => {
            tracing::error!(error = %e, path = %p.display(), "cannot read config; saving disabled");
            READ_ONLY.store(true, Ordering::Relaxed);
            (Config::with_builtin_profiles(), Some(format!("Cannot read {} ({e}); running with defaults and not saving", p.display())))
        }
    }
}

/// Queue a save; bursts are coalesced and written off the UI thread.
pub fn save(c: &Config) {
    if READ_ONLY.load(Ordering::Relaxed) {
        return;
    }
    let _ = saver().send(Job::Save(Box::new(c.clone())));
}

/// Write any queued save now. Call before exiting.
pub fn flush() {
    let (tx, rx) = mpsc::channel();
    if saver().send(Job::Flush(tx)).is_ok() {
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

fn saver() -> &'static mpsc::Sender<Job> {
    static SAVER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    SAVER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new().name("config-save".into()).spawn(move || {
            let p = path();
            let mut pending: Option<Box<Config>> = None;
            loop {
                let job = if pending.is_some() {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(j) => Some(j),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(j) => Some(j),
                        Err(_) => break,
                    }
                };
                match job {
                    Some(Job::Save(c)) => pending = Some(c),
                    Some(Job::Flush(done)) => {
                        if let Some(c) = pending.take() {
                            write_atomic(&p, &c);
                        }
                        let _ = done.send(());
                    }
                    None => {
                        if let Some(c) = pending.take() {
                            write_atomic(&p, &c);
                        }
                    }
                }
            }
            if let Some(c) = pending.take() {
                write_atomic(&p, &c);
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "cannot start the config saver; changes will not be saved");
        }
        tx
    })
}

/// Write through a temp file and rename, so a crash never leaves a truncated config.
fn write_atomic(p: &Path, c: &Config) {
    use std::io::Write;
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let s = match toml::to_string_pretty(c) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "cannot serialise config");
            return;
        }
    };
    let tmp = p.with_extension("toml.tmp");
    let written = std::fs::File::create(&tmp)
        .and_then(|mut f| {
            f.write_all(s.as_bytes())?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, p));
    if let Err(e) = written {
        tracing::error!(error = %e, path = %p.display(), "cannot save config");
        let _ = std::fs::remove_file(&tmp);
    }
}

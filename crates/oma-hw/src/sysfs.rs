//! Thin, careful helpers for reading and writing sysfs attributes.
//!
//! Every read is tolerant (a missing or unreadable attribute yields `None`),
//! every write is explicit and reports the exact path in its error so that the
//! privileged helper can surface meaningful messages to the UI.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Read a sysfs attribute as a trimmed string.
pub fn read_string(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// Read a sysfs attribute and parse it as an integer.
pub fn read_i64(path: impl AsRef<Path>) -> Option<i64> {
    read_string(path)?.parse().ok()
}

/// Read a sysfs attribute and parse it as an unsigned integer.
pub fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_string(path)?.parse().ok()
}

/// Read a sysfs attribute holding a millidegree/millivolt style value and
/// scale it into a float (e.g. `52000` -> `52.0`).
pub fn read_milli(path: impl AsRef<Path>) -> Option<f64> {
    read_i64(path).map(|v| v as f64 / 1000.0)
}

/// Whether a sysfs attribute exists.
pub fn exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().exists()
}

/// Whether the attribute is writable by the *current* process.
pub fn writable(path: impl AsRef<Path>) -> bool {
    fs::OpenOptions::new().write(true).open(path).is_ok()
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("attribute {path} does not exist")]
    Missing { path: PathBuf },
    #[error("permission denied writing {path} (needs the OmaAsus helper or root)")]
    Denied { path: PathBuf },
    #[error("failed writing `{value}` to {path}: {source}")]
    Io {
        path: PathBuf,
        value: String,
        #[source]
        source: io::Error,
    },
}

/// Write a value to a sysfs attribute, translating the usual failure modes.
pub fn write(path: impl AsRef<Path>, value: impl AsRef<str>) -> Result<(), WriteError> {
    let path = path.as_ref();
    let value = value.as_ref();
    if !path.exists() {
        return Err(WriteError::Missing { path: path.to_owned() });
    }
    match fs::write(path, value) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(WriteError::Denied { path: path.to_owned() }),
        Err(source) => Err(WriteError::Io { path: path.to_owned(), value: value.to_owned(), source }),
    }
}

/// List the entries of a directory, sorted, ignoring errors.
pub fn list_dir(path: impl AsRef<Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(path)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Resolve `/sys/class/hwmon/hwmonN` style directories.
pub fn hwmon_dirs() -> Vec<PathBuf> {
    list_dir("/sys/class/hwmon")
}

/// Canonicalise a sysfs path so that hwmon numbers, which can change between
/// boots, are replaced by stable device paths where possible.
pub fn canonical(path: impl AsRef<Path>) -> PathBuf {
    fs::canonicalize(&path).unwrap_or_else(|_| path.as_ref().to_owned())
}

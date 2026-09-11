//! Client proxy for the OmaAsus privileged helper (`com.omaasus.Helper1`).
//!
//! When the helper is unavailable the [`Controller`] falls back to direct
//! writes, which succeed only when running as root.

use crate::nvidia::NvidiaControl;
use std::path::Path;
use std::time::Duration;

/// What the helper answers while it goes idle and exits: nothing was written
/// or claimed, and D-Bus starts a fresh helper for the next call.
pub const RESTARTING: &str = "oma-helper is restarting; try again";

/// The helper's name on the system bus.
pub const BUS_NAME: &str = "com.omaasus.Helper1";

/// A call, tried once more when an exiting helper refused it.
async fn again<T, F, Fut>(call: F) -> zbus::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = zbus::Result<T>>,
{
    match call().await {
        Err(e) if e.to_string().contains(RESTARTING) => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            call().await
        }
        other => other,
    }
}

#[zbus::proxy(
    interface = "com.omaasus.Helper1",
    default_service = "com.omaasus.Helper1",
    default_path = "/com/omaasus/Helper1"
)]
pub trait Helper {
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
    fn write_sysfs(&self, path: &str, value: &str) -> zbus::Result<String>;
    fn write_sysfs_batch(&self, entries: Vec<(String, String)>) -> zbus::Result<Vec<String>>;
    fn read_sysfs(&self, path: &str) -> zbus::Result<String>;
    fn nvidia_apply(&self, index: u32, control_json: &str) -> zbus::Result<String>;
    fn hid_write(&self, path: &str, report: Vec<u8>) -> zbus::Result<u32>;
    fn hid_send_feature(&self, path: &str, report: Vec<u8>) -> zbus::Result<()>;
    #[zbus(signal)]
    fn changed(&self, what: String) -> zbus::Result<()>;
}

/// Unified write path: helper over D-Bus, or direct when root.
#[derive(Clone)]
pub struct Controller {
    proxy: Option<HelperProxy<'static>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("{0}")]
    Sysfs(#[from] crate::sysfs::WriteError),
    #[error("helper: {0}")]
    Bus(#[from] zbus::Error),
    #[error("{0}")]
    Remote(String),
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl Controller {
    /// Connect to the helper if present.
    pub async fn connect() -> Self {
        let proxy = match zbus::Connection::system().await {
            Ok(conn) => match HelperProxy::builder(&conn).cache_properties(zbus::proxy::CacheProperties::No).build().await {
                Ok(p) => match p.version().await {
                    Ok(v) => {
                        tracing::info!(version = %v, "connected to oma-helper");
                        Some(p)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "oma-helper not reachable; direct writes only");
                        None
                    }
                },
                Err(_) => None,
            },
            Err(_) => None,
        };
        Self { proxy }
    }

    pub fn direct() -> Self {
        Self { proxy: None }
    }

    pub fn has_helper(&self) -> bool {
        self.proxy.is_some()
    }

    pub fn is_root() -> bool {
        // Avoid libc dependency: euid 0 owns /proc/self with uid 0.
        std::fs::metadata("/proc/self").map(|m| {
            use std::os::unix::fs::MetadataExt;
            m.uid() == 0
        }).unwrap_or(false)
    }

    /// Direct sysfs/HID writes can block for seconds on a wedged device, so
    /// they always run on a blocking thread and never stall the async runtime.
    pub async fn write(&self, path: impl AsRef<Path>, value: impl AsRef<str>) -> Result<(), ControlError> {
        let path = path.as_ref().to_path_buf();
        let value = value.as_ref().to_string();
        if let Some(p) = &self.proxy {
            let path_s = path.to_string_lossy().into_owned();
            let err = again(|| p.write_sysfs(&path_s, &value)).await?;
            if err.is_empty() { Ok(()) } else { Err(ControlError::Remote(err)) }
        } else {
            Ok(tokio::task::spawn_blocking(move || crate::sysfs::write(&path, &value)).await.map_err(|e| anyhow::anyhow!(e))??)
        }
    }

    /// Write many attributes; returns the list of failures (path, error).
    pub async fn write_batch(&self, entries: &[(std::path::PathBuf, String)]) -> Result<Vec<(String, String)>, ControlError> {
        if let Some(p) = &self.proxy {
            let req: Vec<(String, String)> = entries.iter().map(|(a, b)| (a.to_string_lossy().into_owned(), b.clone())).collect();
            let res = again(|| p.write_sysfs_batch(req.clone())).await?;
            Ok(req.into_iter().zip(res).filter(|(_, e)| !e.is_empty()).map(|((path, _), e)| (path, e)).collect())
        } else {
            let entries = entries.to_vec();
            Ok(tokio::task::spawn_blocking(move || {
                entries
                    .iter()
                    .filter_map(|(path, v)| crate::sysfs::write(path, v).err().map(|e| (path.to_string_lossy().into_owned(), e.to_string())))
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))?)
        }
    }

    pub async fn read(&self, path: impl AsRef<Path>) -> Option<String> {
        let path = path.as_ref();
        if let Some(v) = crate::sysfs::read_string(path) {
            return Some(v);
        }
        let p = self.proxy.as_ref()?;
        let path_s = path.to_string_lossy().into_owned();
        again(|| p.read_sysfs(&path_s)).await.ok()
    }

    /// Apply NVIDIA controls; returns per-step failures.
    pub async fn nvidia_apply(&self, index: u32, ctl: &NvidiaControl) -> Result<Vec<(String, String)>, ControlError> {
        if let Some(p) = &self.proxy {
            let json = serde_json::to_string(ctl).map_err(|e| anyhow::anyhow!(e))?;
            let res = again(|| p.nvidia_apply(index, &json)).await?;
            let arr: Vec<(String, Option<String>)> = serde_json::from_str(&res).map_err(|e| anyhow::anyhow!(e))?;
            Ok(arr.into_iter().filter_map(|(s, e)| e.map(|e| (s, e))).collect())
        } else {
            let ctl = ctl.clone();
            Ok(tokio::task::spawn_blocking(move || -> Result<Vec<(String, String)>, anyhow::Error> {
                let gpu = crate::nvidia::NvidiaGpu::open(index)?;
                Ok(gpu.apply(&ctl).into_iter().filter_map(|(s, r)| r.err().map(|e| (s, e))).collect())
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))??)
        }
    }

    pub async fn hid_write(&self, path: &str, report: &[u8]) -> Result<usize, ControlError> {
        if let Some(p) = &self.proxy {
            Ok(again(|| p.hid_write(path, report.to_vec())).await? as usize)
        } else {
            let path = path.to_string();
            let report = report.to_vec();
            Ok(tokio::task::spawn_blocking(move || -> Result<usize, anyhow::Error> {
                let api = hidapi::HidApi::new()?;
                let c = std::ffi::CString::new(path)?;
                let dev = api.open_path(&c)?;
                Ok(dev.write(&report)?)
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))??)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_from_an_exiting_helper_is_tried_again_once() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        let calls = std::cell::Cell::new(0);
        let r: zbus::Result<u32> = rt.block_on(again(|| {
            calls.set(calls.get() + 1);
            let first = calls.get() == 1;
            async move { if first { Err(zbus::Error::Failure(RESTARTING.into())) } else { Ok(7) } }
        }));
        assert_eq!((r.ok(), calls.get()), (Some(7), 2));

        let calls = std::cell::Cell::new(0);
        let r: zbus::Result<u32> = rt.block_on(again(|| {
            calls.set(calls.get() + 1);
            async { Err(zbus::Error::Failure("polkit denied".into())) }
        }));
        assert!(r.is_err());
        assert_eq!(calls.get(), 1, "other errors aren't retried");
    }
}

//! One lane per physical device. Every sysfs, NVML or HID operation runs on
//! a blocking thread inside its device's lane, so:
//!
//! * the helper's single-threaded async runtime never blocks on a wedged
//!   device (polkit answers, the watchdog and the idle exit keep running);
//! * operations on one device are serialised, so a duty and the mode that
//!   follows it land in order;
//! * a device that stops answering holds exactly one thread. Later calls for
//!   it wait a bounded time, then are refused as busy; nothing piles up
//!   behind it, and other devices' lanes are unaffected.
//!
//! A caller that gives up (D-Bus timeout) does not cancel the kernel call:
//! the worker finishes on its own and the lane frees itself then.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
pub struct Lanes {
    lanes: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// The lane's device has an earlier operation that has not returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy(pub String);

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: an earlier operation on this device has not returned", self.0)
    }
}

impl Lanes {
    fn lane(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.lanes.lock().unwrap().entry(key.to_string()).or_default().clone()
    }

    /// Run `f` on a blocking thread inside the lane `key`, waiting at most
    /// `wait` for the lane to be free.
    pub async fn run<T, F>(&self, key: &str, wait: Duration, f: F) -> Result<T, Busy>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let lane = self.lane(key);
        let guard = tokio::time::timeout(wait, lane.lock_owned()).await.map_err(|_| Busy(key.to_string()))?;
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let _held = guard;
            f()
        })
        .await
        .map_err(|_| Busy(key))
    }

    /// Lanes currently held: devices with an operation in progress.
    pub fn held(&self) -> Vec<String> {
        self.lanes
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, l)| l.try_lock().is_err())
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// The lane a sysfs attribute belongs to: its device directory.
pub fn sysfs_lane(path: &std::path::Path) -> String {
    path.parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn nvidia_lane(index: u32) -> String {
    format!("nvidia:{index}")
}

pub fn hid_lane(path: &str) -> String {
    format!("hid:{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_stuck_device_blocks_only_its_own_lane() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        rt.block_on(async {
            let lanes = Arc::new(Lanes::default());
            let (l1, l2, l3) = (lanes.clone(), lanes.clone(), lanes.clone());
            // Device a is wedged for a while.
            let stuck = tokio::spawn(async move { l1.run("a", Duration::from_millis(50), || std::thread::sleep(Duration::from_millis(400))).await });
            tokio::time::sleep(Duration::from_millis(20)).await;
            // Device b answers at once.
            let t = Instant::now();
            assert_eq!(l2.run("b", Duration::from_millis(50), || 7).await, Ok(7));
            assert!(t.elapsed() < Duration::from_millis(200), "b did not wait for a");
            // A second call for a is refused rather than queued behind the stuck one.
            let again = l3.run("a", Duration::from_millis(50), || 1).await;
            assert_eq!(again, Err(Busy("a".into())));
            assert_eq!(lanes.held(), vec!["a".to_string()]);
            assert!(stuck.await.unwrap().is_ok());
            assert!(lanes.held().is_empty(), "freed when the worker returned");
            assert_eq!(lanes.run("a", Duration::from_millis(50), || 2).await, Ok(2));
        });
    }

    #[test]
    fn lane_keys() {
        assert_eq!(sysfs_lane(std::path::Path::new("/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2")), "/sys/devices/platform/nct6775.656/hwmon/hwmon3");
        assert_eq!(nvidia_lane(0), "nvidia:0");
        assert_eq!(hid_lane("/dev/hidraw4"), "hid:/dev/hidraw4");
    }
}

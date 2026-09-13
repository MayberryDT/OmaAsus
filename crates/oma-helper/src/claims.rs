//! The helper's memory of what it must put back: fan outputs a client took
//! over, keyed by client, with the state each output was in before the first
//! write. Pure over a [`Writer`], so restores and their retries are tested
//! without hardware.
//!
//! Rules:
//! * An output has one owner. When a second client writes an output another
//!   client holds, the claim moves to the new client and keeps the *original*
//!   state, so a late restore by the old client can never leave the fan in the
//!   manual state the new client found it in.
//! * A restore that fails keeps its entries: they are retried with a bounded
//!   backoff and stay reported until they succeed or the retries run out.
//! * Duties are written before the modes that hand control back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How sysfs is written and read; production uses `oma_hw::sysfs`.
pub trait Writer {
    fn write(&self, path: &Path, value: &str) -> Result<(), String>;
    fn read(&self, path: &Path) -> Option<String>;
}

/// Fan state one client changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Claims {
    /// Canonical PWM attribute → its value before the owning client first wrote it.
    pub sysfs: BTreeMap<PathBuf, String>,
    /// NVIDIA GPUs whose fans the client switched to manual.
    pub nvidia_fans: BTreeSet<u32>,
}

impl Claims {
    pub fn is_empty(&self) -> bool {
        self.sysfs.is_empty() && self.nvidia_fans.is_empty()
    }
}

/// A restore that did not fully succeed and is waiting for its next attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub client: String,
    pub claims: Claims,
    /// Attempts made so far (the first restore counts as one).
    pub attempts: u32,
    pub due: Instant,
    pub last_error: String,
}

/// Delay before attempt number `attempts + 1`, or `None` once the retries are
/// spent. Short first, so a device that was merely busy recovers in seconds;
/// then spaced out, so a wedged one is not hammered.
pub fn retry_delay(attempts: u32) -> Option<Duration> {
    match attempts {
        0 | 1 => Some(Duration::from_secs(2)),
        2 => Some(Duration::from_secs(5)),
        3 => Some(Duration::from_secs(15)),
        4 => Some(Duration::from_secs(30)),
        5 => Some(Duration::from_secs(60)),
        _ => None,
    }
}

/// What a restore attempt left behind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub restored: Vec<PathBuf>,
    /// Entries that must be tried again, with their original values.
    pub failed: BTreeMap<PathBuf, String>,
    pub errors: Vec<String>,
}

/// Put the sysfs part of a claim back: the board's curve points, then duties,
/// then the modes that hand control back. Failed entries are returned for a
/// later attempt; a mode whose duty failed is still attempted, since handing
/// the fan back matters more than the duty it hands back at.
pub fn restore_sysfs(w: &dyn Writer, sysfs: &BTreeMap<PathBuf, String>) -> RestoreOutcome {
    let mut out = RestoreOutcome::default();
    for (p, v) in restore_order(sysfs) {
        match w.write(&p, &v) {
            Ok(()) => out.restored.push(p),
            Err(e) => {
                out.errors.push(format!("{}: {e}", p.display()));
                out.failed.insert(p, v);
            }
        }
    }
    // A duty or curve point that failed on an output whose mode was handed
    // back is moot: firmware drives the fan now, and writing it later would
    // either be refused or take the fan over again.
    let handed_back: Vec<PathBuf> = out.failed.keys().filter(|p| !is_mode(p) && out.restored.contains(&output_pair(p).1)).cloned().collect();
    for p in handed_back {
        out.failed.remove(&p);
    }
    out
}

/// The writes that put a client's fan settings back: curve points, then
/// duties, then the modes that hand control back to firmware.
pub fn restore_order(sysfs: &BTreeMap<PathBuf, String>) -> Vec<(PathBuf, String)> {
    let rank = |p: &Path| if is_mode(p) { 2 } else if is_curve_point(p) { 0 } else { 1 };
    let mut all: Vec<(PathBuf, String)> = sysfs.iter().map(|(p, v)| (p.clone(), v.clone())).collect();
    all.sort_by_key(|(p, _)| rank(p));
    all
}

/// `pwmN_enable`: the attribute that hands an output back.
pub fn is_mode(path: &Path) -> bool {
    path.to_string_lossy().ends_with("_enable")
}

/// `pwmN_auto_pointK_pwm` / `_temp`: a point of the board's own curve.
pub fn is_curve_point(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains("_auto_point"))
}

/// The `pwmN` an attribute belongs to: itself, or the duty attribute of a
/// `pwmN_enable` or `pwmN_auto_pointK_*`.
pub fn output_base(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(b) = s.strip_suffix("_enable") {
        return PathBuf::from(b);
    }
    if let Some(i) = s.find("_auto_point") {
        return PathBuf::from(&s[..i]);
    }
    path.to_path_buf()
}

/// Whether writing `value` to a `pwmN_enable` hands the output back: anything
/// but manual (hwmon ABI: 1 manual, 0 full speed, 2 and up automatic), or the
/// mode the client found it in. Not the recorded original alone: a helper
/// started after another died records the manual state it found. Compared as
/// numbers, the way the kernel reads them ("+1" and "01" are 1).
/// asus_custom_fan_curve differs: its 1 is the custom curve, which this
/// treats as a claim.
pub fn hands_back(value: &str, original: Option<&str>) -> bool {
    let n = |s: &str| s.trim().parse::<u64>().ok();
    n(value) != Some(1) || original.and_then(n) == n(value)
}

/// The duty and mode attributes of a fan output, from any of its attributes.
pub fn output_pair(path: &Path) -> (PathBuf, PathBuf) {
    let base = output_base(path);
    let enable = PathBuf::from(format!("{}_enable", base.to_string_lossy()));
    (base, enable)
}

/// The state an output held *before* a write, read outside any lock so a
/// device that blocks on the read cannot freeze the helper: the attribute
/// being written (the duty, or a curve point) and the mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Originals {
    pub entries: Vec<(PathBuf, Option<String>)>,
}

pub fn read_originals(w: &dyn Writer, path: &Path) -> Originals {
    let (duty, enable) = output_pair(path);
    let first = if is_curve_point(path) { path.to_path_buf() } else { duty };
    Originals { entries: vec![(first.clone(), w.read(&first)), (enable.clone(), w.read(&enable))] }
}

/// Every client's claims plus restores waiting for another attempt.
#[derive(Debug, Default)]
pub struct Ledger {
    pub by_client: HashMap<String, Claims>,
    pub pending: Vec<Pending>,
    /// Restores that ran out of attempts, kept for diagnostics.
    pub given_up: Vec<Pending>,
}

impl Ledger {
    /// Record a fan-output write that *succeeded*: `path` is the duty or mode
    /// attribute, `value` what was written, `originals` what the attributes
    /// held before it. A write that failed changes nothing here: a failed
    /// hand-back keeps its claim, a failed take-over records none.
    pub fn record_write(&mut self, client: &str, path: &Path, value: &str, originals: &Originals) {
        let (base, enable) = output_pair(path);
        // Handing the output back leaves nothing to restore: the client has put
        // the duty and any curve points back itself before giving up the mode.
        let original_mode = self.by_client.get(client).and_then(|c| c.sysfs.get(&enable)).cloned();
        if path == enable && hands_back(value.trim(), original_mode.as_deref()) {
            if let Some(c) = self.by_client.get_mut(client) {
                c.sysfs.retain(|k, _| output_base(k) != base);
            }
            return;
        }
        for (p, read) in &originals.entries {
            // Another client's claim on this output moves over, original intact:
            // what the new client found is the old client's manual state.
            let inherited = self.take_from_others(client, p);
            let c = self.by_client.entry(client.to_string()).or_default();
            if c.sysfs.contains_key(p) {
                continue;
            }
            if let Some(v) = inherited.or_else(|| read.clone()) {
                c.sysfs.insert(p.clone(), v);
            }
        }
    }

    /// Read the originals and record, for tests that model a successful write.
    #[cfg(test)]
    pub fn claim_before_write(&mut self, w: &dyn Writer, client: &str, path: &Path, value: &str) {
        let originals = read_originals(w, path);
        self.record_write(client, path, value, &originals);
    }

    /// Remove `path` from every client but `client`, returning the original it held.
    fn take_from_others(&mut self, client: &str, path: &Path) -> Option<String> {
        let mut found = None;
        for (owner, c) in self.by_client.iter_mut() {
            if owner != client
                && let Some(v) = c.sysfs.remove(path)
            {
                found = Some(v);
            }
        }
        // A restore waiting for another client also loses the output: the new
        // owner's claim now carries the original. So does one given up on.
        for p in self.pending.iter_mut().chain(self.given_up.iter_mut()).filter(|p| p.client != client) {
            if let Some(v) = p.claims.sysfs.remove(path) {
                found.get_or_insert(v);
            }
        }
        self.pending.retain(|p| !p.claims.is_empty());
        self.given_up.retain(|p| !p.claims.is_empty());
        found
    }

    /// A client put a GPU's fans under manual control (or handed them back).
    /// Like a sysfs output, a GPU has one owner: taking it moves it away from
    /// any other client, so a dead client's restore cannot undo a live one's.
    pub fn note_nvidia(&mut self, client: &str, index: u32, manual: bool) {
        if manual {
            for (owner, c) in self.by_client.iter_mut() {
                if owner != client {
                    c.nvidia_fans.remove(&index);
                }
            }
            for p in self.pending.iter_mut().chain(self.given_up.iter_mut()).filter(|p| p.client != client) {
                p.claims.nvidia_fans.remove(&index);
            }
            self.pending.retain(|p| !p.claims.is_empty());
            self.given_up.retain(|p| !p.claims.is_empty());
        }
        let c = self.by_client.entry(client.to_string()).or_default();
        if manual {
            c.nvidia_fans.insert(index);
        } else {
            c.nvidia_fans.remove(&index);
        }
    }

    /// Take a client's claims out for a restore attempt.
    pub fn take(&mut self, client: &str) -> Option<Claims> {
        self.by_client.remove(client).filter(|c| !c.is_empty())
    }

    /// Clients with something to restore.
    pub fn guarding(&self) -> Vec<String> {
        self.by_client.iter().filter(|(_, c)| !c.is_empty()).map(|(k, _)| k.clone()).collect()
    }

    /// Whether the helper still has fans to put back: live claims or retries.
    pub fn has_work(&self) -> bool {
        !self.guarding().is_empty() || !self.pending.is_empty()
    }

    /// Keep what a restore attempt could not put back, scheduling the next try.
    /// Returns `true` when the retries are spent and the entry was parked as
    /// given up: the caller must say so loudly, since nothing drives that fan.
    pub fn retain_failed(&mut self, client: &str, failed: Claims, attempts: u32, error: String, now: Instant) -> bool {
        if failed.is_empty() {
            return false;
        }
        let entry = Pending { client: client.to_string(), claims: failed, attempts, due: now, last_error: error };
        match retry_delay(attempts) {
            Some(d) => {
                self.pending.push(Pending { due: now + d, ..entry });
                false
            }
            None => {
                self.given_up.push(entry);
                true
            }
        }
    }

    /// Keep a restore that could not run *yet* (the GPU is asleep and NVML
    /// would wake it): tried again later without spending an attempt, since
    /// nothing failed.
    pub fn defer(&mut self, client: &str, claims: Claims, attempts: u32, why: String, now: Instant) {
        if claims.is_empty() {
            return;
        }
        self.pending.push(Pending { client: client.to_string(), claims, attempts: attempts.saturating_sub(1), due: now + Duration::from_secs(2), last_error: why });
    }

    /// Restores whose next attempt is due.
    pub fn due(&mut self, now: Instant) -> Vec<Pending> {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending).into_iter().partition(|p| p.due <= now);
        self.pending = later;
        due
    }

    /// Everything waiting, due or not (for a stop that tries once more).
    pub fn drain_pending(&mut self) -> Vec<Pending> {
        std::mem::take(&mut self.pending)
    }

    pub fn report(&self) -> Report {
        Report {
            guarding: self.guarding().len(),
            pending: self
                .pending
                .iter()
                .map(|p| PendingReport {
                    client: p.client.clone(),
                    outputs: p.claims.sysfs.keys().map(|k| k.to_string_lossy().into_owned()).collect(),
                    nvidia_gpus: p.claims.nvidia_fans.iter().copied().collect(),
                    attempts: p.attempts,
                    last_error: p.last_error.clone(),
                })
                .collect(),
            given_up: self
                .given_up
                .iter()
                .map(|p| PendingReport {
                    client: p.client.clone(),
                    outputs: p.claims.sysfs.keys().map(|k| k.to_string_lossy().into_owned()).collect(),
                    nvidia_gpus: p.claims.nvidia_fans.iter().copied().collect(),
                    attempts: p.attempts,
                    last_error: p.last_error.clone(),
                })
                .collect(),
            busy_devices: Vec::new(),
        }
    }
}

/// What a client can ask the helper about its recovery state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// Clients whose fans the helper is guarding.
    pub guarding: usize,
    pub pending: Vec<PendingReport>,
    pub given_up: Vec<PendingReport>,
    /// Devices with an operation that has not returned.
    #[serde(default)]
    pub busy_devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingReport {
    pub client: String,
    pub outputs: Vec<String>,
    pub nvidia_gpus: Vec<u32>,
    pub attempts: u32,
    pub last_error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A sysfs that remembers writes and can be told to refuse some paths.
    #[derive(Default)]
    struct Fake {
        values: RefCell<BTreeMap<PathBuf, String>>,
        refuse: RefCell<BTreeSet<PathBuf>>,
        writes: RefCell<Vec<(PathBuf, String)>>,
    }

    impl Fake {
        fn with(values: &[(&str, &str)]) -> Self {
            let f = Self::default();
            for (p, v) in values {
                f.values.borrow_mut().insert(PathBuf::from(p), v.to_string());
            }
            f
        }
        fn refuse(&self, p: &str) {
            self.refuse.borrow_mut().insert(PathBuf::from(p));
        }
        fn allow(&self, p: &str) {
            self.refuse.borrow_mut().remove(Path::new(p));
        }
        fn value(&self, p: &str) -> Option<String> {
            self.values.borrow().get(Path::new(p)).cloned()
        }
    }

    impl Writer for Fake {
        fn write(&self, path: &Path, value: &str) -> Result<(), String> {
            self.writes.borrow_mut().push((path.to_path_buf(), value.to_string()));
            if self.refuse.borrow().contains(path) {
                return Err("Device or resource busy".into());
            }
            self.values.borrow_mut().insert(path.to_path_buf(), value.to_string());
            Ok(())
        }
        fn read(&self, path: &Path) -> Option<String> {
            self.values.borrow().get(path).cloned()
        }
    }

    const PWM: &str = "/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2";
    const EN: &str = "/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_enable";

    fn claims(pairs: &[(&str, &str)]) -> Claims {
        Claims {
            sysfs: pairs.iter().map(|(p, v)| (PathBuf::from(p), v.to_string())).collect(),
            nvidia_fans: BTreeSet::new(),
        }
    }

    #[test]
    fn a_failed_restore_keeps_its_entries_and_is_retried() {
        let fs = Fake::with(&[(PWM, "255"), (EN, "1")]);
        fs.refuse(EN);
        let mut l = Ledger::default();
        l.by_client.insert("a".into(), claims(&[(PWM, "128"), (EN, "5")]));
        let t0 = Instant::now();
        let c = l.take("a").unwrap();
        let out = restore_sysfs(&fs, &c.sysfs);
        assert_eq!(out.restored, vec![PathBuf::from(PWM)], "the duty went in");
        assert_eq!(out.failed, claims(&[(EN, "5")]).sysfs, "the mode is kept with its original value");
        l.retain_failed("a", Claims { sysfs: out.failed, ..Default::default() }, 1, out.errors.join("; "), t0);
        assert!(l.has_work(), "still something to put back");
        assert!(l.due(t0).is_empty(), "not before the backoff");
        let due = l.due(t0 + Duration::from_secs(3));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        // The device answers now.
        fs.allow(EN);
        let again = restore_sysfs(&fs, &due[0].claims.sysfs);
        assert!(again.failed.is_empty());
        assert_eq!(fs.value(EN).as_deref(), Some("5"), "handed back to its original mode");
        assert!(!l.has_work());
    }

    #[test]
    fn retries_are_bounded_and_the_leftovers_stay_reported() {
        let mut l = Ledger::default();
        let now = Instant::now();
        let mut attempts = 1;
        loop {
            let gave_up = l.retain_failed("a", claims(&[(EN, "5")]), attempts, "busy".into(), now);
            match l.pending.pop() {
                Some(p) => {
                    assert!(!gave_up);
                    attempts = p.attempts + 1;
                }
                None => {
                    assert!(gave_up, "the last attempt says so");
                    break;
                }
            }
            assert!(attempts < 20, "must give up eventually");
        }
        assert_eq!(l.given_up.len(), 1);
        assert_eq!(l.report().given_up[0].outputs, vec![EN.to_string()]);
        assert!(!l.has_work(), "given-up work doesn't keep the helper alive");
    }

    #[test]
    fn a_second_client_inherits_the_original_not_the_manual_state_it_found() {
        let fs = Fake::with(&[(PWM, "200"), (EN, "5")]);
        let mut l = Ledger::default();
        // Client a takes the fan over: original mode 5 (automatic).
        l.claim_before_write(&fs, "a", Path::new(EN), "1");
        fs.write(Path::new(EN), "1").unwrap();
        l.claim_before_write(&fs, "a", Path::new(PWM), "128");
        fs.write(Path::new(PWM), "128").unwrap();
        assert_eq!(l.by_client["a"].sysfs.get(Path::new(EN)).map(String::as_str), Some("5"));
        // Client b writes the same output while a still holds it.
        l.claim_before_write(&fs, "b", Path::new(PWM), "90");
        assert!(l.by_client["a"].sysfs.is_empty(), "the output moved to b");
        assert_eq!(l.by_client["b"].sysfs.get(Path::new(EN)).map(String::as_str), Some("5"), "b's claim carries the true original, not the manual 1 it found");
        assert_eq!(l.by_client["b"].sysfs.get(Path::new(PWM)).map(String::as_str), Some("200"));
        // a dying now restores nothing; b dying restores mode 5.
        assert!(l.take("a").is_none());
        let out = restore_sysfs(&fs, &l.take("b").unwrap().sysfs);
        assert!(out.failed.is_empty());
        assert_eq!(fs.value(EN).as_deref(), Some("5"));
    }

    #[test]
    fn a_pending_restore_loses_an_output_a_new_client_takes() {
        let fs = Fake::with(&[(PWM, "255"), (EN, "1")]);
        let mut l = Ledger::default();
        l.retain_failed("a", claims(&[(EN, "5"), (PWM, "128")]), 1, "busy".into(), Instant::now());
        l.claim_before_write(&fs, "b", Path::new(PWM), "90");
        assert!(l.pending.is_empty(), "nothing left for a's retry");
        assert_eq!(l.by_client["b"].sysfs.get(Path::new(EN)).map(String::as_str), Some("5"));
    }

    #[test]
    fn a_failed_write_changes_no_claim() {
        let fs = Fake::with(&[(PWM, "200"), (EN, "1")]);
        let mut l = Ledger::default();
        // a holds the output (original mode 5) and tries to hand it back, but the write fails.
        l.by_client.insert("a".into(), claims(&[(EN, "5"), (PWM, "200")]));
        fs.refuse(EN);
        let originals = read_originals(&fs, Path::new(EN));
        assert!(fs.write(Path::new(EN), "5").is_err());
        // The caller only records successful writes: the claim stays.
        assert_eq!(l.by_client["a"].sysfs.len(), 2);
        // And a take-over whose write failed records nothing.
        assert!(fs.write(Path::new(PWM), "90").is_ok());
        l.record_write("b", Path::new(PWM), "90", &originals);
        assert_eq!(l.by_client["b"].sysfs.get(Path::new(EN)).map(String::as_str), Some("5"), "b now owns it with the true original");
        assert!(l.by_client["a"].sysfs.is_empty());
    }

    #[test]
    fn curve_points_are_claimed_restored_first_and_dropped_on_hand_back() {
        let p1t = "/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_auto_point1_temp";
        let p1p = "/sys/devices/platform/nct6775.656/hwmon/hwmon3/pwm2_auto_point1_pwm";
        let fs = Fake::with(&[(PWM, "200"), (EN, "5"), (p1t, "20000"), (p1p, "153")]);
        let mut l = Ledger::default();
        // OmaAsus programs a Smart Fan IV curve: each point is claimed with the board's value.
        for (p, v) in [(p1t, "30000"), (p1p, "60")] {
            let o = read_originals(&fs, Path::new(p));
            fs.write(Path::new(p), v).unwrap();
            l.record_write("a", Path::new(p), v, &o);
        }
        let c = &l.by_client["a"];
        assert_eq!(c.sysfs.get(Path::new(p1t)).map(String::as_str), Some("20000"));
        assert_eq!(c.sysfs.get(Path::new(p1p)).map(String::as_str), Some("153"));
        assert_eq!(c.sysfs.get(Path::new(EN)).map(String::as_str), Some("5"), "the mode too");
        // A crash restore writes the points before the mode.
        let order: Vec<String> = restore_order(&c.sysfs).into_iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(order.last().map(String::as_str), Some("pwm2_enable"));
        assert!(order[0].contains("auto_point") && order[1].contains("auto_point"));
        // The client hands the output back: nothing of it stays claimed.
        let o = read_originals(&fs, Path::new(EN));
        fs.write(Path::new(EN), "5").unwrap();
        l.record_write("a", Path::new(EN), "5", &o);
        assert!(l.by_client["a"].is_empty());
        assert_eq!(output_base(Path::new(p1p)), PathBuf::from(PWM));
        assert_eq!(output_base(Path::new(EN)), PathBuf::from(PWM));
        assert_eq!(output_base(Path::new(PWM)), PathBuf::from(PWM));
    }

    #[test]
    fn a_duty_on_a_handed_back_output_is_not_retried() {
        let fs = Fake::with(&[(PWM, "255"), (EN, "1")]);
        fs.refuse(PWM);
        let out = restore_sysfs(&fs, &claims(&[(PWM, "128"), (EN, "5")]).sysfs);
        assert_eq!(out.restored, vec![PathBuf::from(EN)]);
        assert!(out.failed.is_empty(), "firmware has the fan back; its old duty is moot");
        assert_eq!(fs.value(EN).as_deref(), Some("5"));
    }

    #[test]
    fn a_gpu_has_one_owner_too() {
        let mut l = Ledger::default();
        l.note_nvidia("a", 0, true);
        l.note_nvidia("b", 0, true);
        assert!(l.by_client["a"].nvidia_fans.is_empty(), "moved to b");
        assert!(l.by_client["b"].nvidia_fans.contains(&0));
        l.retain_failed("b", Claims { nvidia_fans: BTreeSet::from([0]), ..Default::default() }, 1, "busy".into(), Instant::now());
        l.note_nvidia("c", 0, true);
        assert!(l.pending.is_empty(), "b's retry no longer owns the GPU");
    }

    #[test]
    fn handing_back_drops_the_claim() {
        let fs = Fake::with(&[(PWM, "200"), (EN, "5")]);
        let mut l = Ledger::default();
        l.claim_before_write(&fs, "a", Path::new(EN), "1");
        l.claim_before_write(&fs, "a", Path::new(EN), "5");
        assert!(l.by_client["a"].is_empty());
        assert!(!l.has_work());
    }

    #[test]
    fn duties_before_modes() {
        let sysfs = claims(&[(EN, "2"), (PWM, "128")]).sysfs;
        let order: Vec<String> = restore_order(&sysfs).into_iter().map(|(p, v)| format!("{}={v}", p.file_name().unwrap().to_string_lossy())).collect();
        assert_eq!(order, ["pwm2=128", "pwm2_enable=2"]);
    }

    #[test]
    fn anything_but_manual_hands_a_fan_back() {
        assert!(hands_back("2", Some("2")), "its original mode");
        assert!(hands_back("5", Some("1")), "automatic, though this helper found it manual");
        assert!(hands_back("0", None), "full speed belongs to the driver");
        assert!(hands_back("1", Some("1")), "the manual mode it was found in");
        assert!(!hands_back("1", Some("2")), "manual is a claim");
        assert!(!hands_back("+1", Some("2")), "read as the kernel reads it");
        assert!(!hands_back("01", None), "read as the kernel reads it");
    }

    #[test]
    fn backoff_is_short_first_then_spaced_then_over() {
        assert_eq!(retry_delay(1), Some(Duration::from_secs(2)));
        assert!(retry_delay(4).unwrap() > retry_delay(2).unwrap());
        assert_eq!(retry_delay(6), None);
    }
}

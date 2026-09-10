//! Locate the helper binary, its data files and the install script, both for
//! a source checkout (`target/…`) and for a packaged install (`/usr/…`).

use std::path::PathBuf;

pub fn locate() -> anyhow::Result<(PathBuf, PathBuf, PathBuf)> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    // Packaged layout.
    let packaged = (PathBuf::from("/usr/bin/oma-helper"), PathBuf::from("/usr/share/omaasus/helper"), PathBuf::from("/usr/share/omaasus/install-helper.sh"));
    if packaged.0.exists() && packaged.1.exists() && packaged.2.exists() {
        return Ok(packaged);
    }
    // Source checkout: target/<profile>/omaasus → workspace root.
    let bin = dir.join("oma-helper");
    let root = dir.parent().and_then(|p| p.parent()).map(PathBuf::from).unwrap_or_default();
    let data = root.join("crates/oma-helper/data");
    let script = root.join("scripts/install-helper.sh");
    if bin.exists() && data.exists() && script.exists() {
        return Ok((bin, data, script));
    }
    anyhow::bail!("helper files not found next to {} (build with `cargo build --release` first)", exe.display())
}

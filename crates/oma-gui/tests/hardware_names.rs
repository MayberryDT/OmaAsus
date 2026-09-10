//! The GUI shows what the hardware model found. Names of particular hardware
//! belong in oma-hw's knowledge base, never in the GUI's code.

use std::path::Path;

/// Names that would mean the GUI assumes a particular machine or part. (Not
/// "Crosshair" alone: that is also iced's cursor; the board carries "X670".)
const TOKENS: &[&str] = &["Ryujin", "ryujin", "Lian Li", "nct67", "asusec", "X670", "RTX 4090", "GA403", "Zephyrus", "450.0", "2400.0"];

fn scan(dir: &Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("source dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, found);
            continue;
        }
        if path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source file");
        // Test data may name real hardware.
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        for (n, line) in code.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for t in TOKENS.iter().filter(|t| line.contains(*t)) {
                found.push(format!("{}:{}: {t}", path.display(), n + 1));
            }
        }
    }
}

#[test]
fn the_gui_names_no_particular_hardware() {
    let mut found = Vec::new();
    scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
    assert!(found.is_empty(), "hardware names in the GUI (they belong in oma-hw's knowledge):\n{}", found.join("\n"));
}

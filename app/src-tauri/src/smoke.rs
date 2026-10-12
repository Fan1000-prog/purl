//! `purl --smoke`: a self-check that CI runs against the installed release
//! build. It proves the binary starts (on Windows, that every DLL it links,
//! `onnxruntime.dll` included, resolves) and that the core loads without
//! models, then exits. It opens no window and touches no user data.
//!
//! Release builds have no console on Windows, so the result goes to
//! `<temp>/purl-smoke.log`: `SMOKE OK <version> ...` or `SMOKE FAIL <reason>`.

use crate::dictionary::Dictionary;
use crate::platform;
use crate::settings::Settings;
use std::path::{Path, PathBuf};

pub const LOG_NAME: &str = "purl-smoke.log";

/// Run the checks, write the log in the temp dir, return the exit code.
pub fn run() -> i32 {
    let tmp = std::env::temp_dir();
    run_to(&tmp.join(LOG_NAME), &tmp.join("purl-smoke"))
}

fn run_to(log: &Path, scratch: &Path) -> i32 {
    let (line, code) = match check(scratch) {
        Ok(report) => (report, 0),
        Err(reason) => (format!("SMOKE FAIL {reason}"), 1),
    };
    if let Err(e) = std::fs::write(log, format!("{line}\n")) {
        eprintln!("couldn't write {}: {e}", log.display());
        return 1;
    }
    println!("{line}");
    code
}

/// Everything here runs without models, network or a GUI. `scratch` stands in
/// for the app-data dir and is emptied first.
fn check(scratch: &Path) -> Result<String, String> {
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).map_err(|e| format!("scratch dir: {e}"))?;

    let settings_path: PathBuf = scratch.join("settings.json");
    let settings = Settings::load(&settings_path);
    settings
        .save(&settings_path)
        .map_err(|e| format!("settings save: {e}"))?;
    if Settings::load(&settings_path).hotkey != settings.hotkey {
        return Err("settings didn't round-trip".into());
    }

    let dictionary = Dictionary::load(&scratch.join("dictionary.txt"));

    let paste = platform::injector().can_inject();
    let mic = platform::mic_status();
    Ok(format!(
        "SMOKE OK {} paste={} mic={} dictionary={}",
        env!("CARGO_PKG_VERSION"),
        trust_name(paste),
        trust_name(mic),
        dictionary.entries.len(),
    ))
}

fn trust_name(t: platform::Trust) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_ok_line_with_version() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(LOG_NAME);
        assert_eq!(run_to(&log, &dir.path().join("scratch")), 0);
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(
            text.starts_with(&format!("SMOKE OK {} ", env!("CARGO_PKG_VERSION"))),
            "{text}"
        );
        assert!(text.contains(" paste="), "{text}");
    }

    #[test]
    fn unwritable_scratch_reports_fail() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the scratch dir should go can't be made into a dir.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "").unwrap();
        let log = dir.path().join(LOG_NAME);
        assert_eq!(run_to(&log, &blocker.join("scratch")), 1);
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("SMOKE FAIL "), "{text}");
    }
}

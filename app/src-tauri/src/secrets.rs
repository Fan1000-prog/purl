//! macOS Keychain storage for cloud LLM API keys.
//!
//! Keys never touch `settings.json` or logs — they're the one piece of
//! sensitive state this app holds, since everything else is designed to
//! never leave the machine. Shells out to `/usr/bin/security`; args are
//! passed as an array (no shell interpolation of key material), matching
//! the subprocess pattern used for whisper-server/whisper-cli.

use std::process::Command;

const SERVICE: &str = "app.purl.desktop";
const SECURITY_BIN: &str = "/usr/bin/security";

/// Store (or overwrite) `secret` under `account` in the login keychain.
pub fn set(account: &str, secret: &str) -> Result<(), String> {
    let _ = delete(account); // clears any stale entry; a missing one is fine
    let status = Command::new(SECURITY_BIN)
        .args(["add-generic-password", "-a", account, "-s", SERVICE, "-w", secret])
        .status()
        .map_err(|e| format!("keychain write failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("keychain write failed".into())
    }
}

/// Fetch the secret for `account`, or `None` if unset or the keychain is
/// unreachable (e.g. sandboxed CI).
pub fn get(account: &str) -> Option<String> {
    let out = Command::new(SECURITY_BIN)
        .args(["find-generic-password", "-a", account, "-s", SERVICE, "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Remove the stored secret for `account`. Not-found is not an error.
pub fn delete(account: &str) -> Result<(), String> {
    let out = Command::new(SECURITY_BIN)
        .args(["delete-generic-password", "-a", account, "-s", SERVICE])
        .output()
        .map_err(|e| format!("keychain delete failed: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    // Not-found is not an error: `set` calls delete first to clear any stale
    // entry, and a fresh account has nothing to remove. Anything else is a
    // real failure and must not be reported as a successful removal.
    // `delete-generic-password` takes no `-w`, so no secret is in its argv and
    // its stderr cannot echo one — safe to surface.
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("could not be found") {
        Ok(())
    } else {
        Err(format!("keychain delete failed: {}", stderr.trim()))
    }
}

pub fn has(account: &str) -> bool {
    get(account).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real macOS keychain — safe to run (uses a throwaway
    /// account) but `#[ignore]`d by default so `cargo test` never pops a
    /// keychain-access prompt in CI or on a dev machine mid-flow. Run with
    /// `cargo test -- --ignored` to verify manually.
    #[test]
    #[ignore]
    fn round_trips_through_keychain() {
        let account = "purl-test-secret";
        let _ = delete(account);
        assert!(!has(account));

        set(account, "sk-test-123").unwrap();
        assert_eq!(get(account).as_deref(), Some("sk-test-123"));

        set(account, "sk-test-456").unwrap();
        assert_eq!(get(account).as_deref(), Some("sk-test-456"));

        delete(account).unwrap();
        assert!(!has(account));
    }

    #[test]
    fn missing_account_returns_none() {
        assert!(get("purl-definitely-not-set").is_none());
    }

    #[test]
    fn deleting_a_missing_account_is_not_an_error() {
        // `set` relies on this: it clears any stale entry first, and a fresh
        // account has nothing to remove.
        assert!(delete("purl-definitely-not-set").is_ok());
    }
}

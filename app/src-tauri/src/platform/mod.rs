//! OS seams. Everything that differs between macOS and Windows sits behind
//! these traits, so the pipeline above them is platform-blind.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

use crate::corrections::Snapshot;
use crate::inject::InjectError;
use crate::voice_commands::Key;
use std::path::{Path, PathBuf};

/// Whether this process may synthesize keystrokes into other apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// The permission is granted.
    Granted,
    /// The permission is refused (for paste, output falls back to the
    /// clipboard).
    Denied,
    /// Nothing to grant: the OS has no such permission (paste on Windows), or
    /// it hasn't asked yet (macOS mic before the first dictation).
    NotRequired,
}

impl Trust {
    /// True when a synthesized paste can reach the target app.
    pub fn can_paste(self) -> bool {
        self != Trust::Denied
    }
}

pub trait TextInjector: Send + Sync {
    /// Cmd-V / Ctrl-V.
    fn paste(&self) -> Result<(), InjectError>;
    /// Cmd-C / Ctrl-C.
    fn copy(&self) -> Result<(), InjectError>;
    fn send_key(&self, key: Key) -> Result<(), InjectError>;
    fn can_inject(&self) -> Trust;
    /// Like `can_inject`, but asks the OS to show its grant dialog when the
    /// permission is missing. The default suits OSes without one.
    fn request_trust(&self) -> Trust {
        self.can_inject()
    }
}

/// Reads the focused text field of the frontmost app.
///
/// `read()` returning None is ordinary, not exceptional: some target apps
/// expose nothing, and on Windows v1 nothing is read at all.
pub trait FocusReader: Send {
    fn read(&self) -> Option<Snapshot>;

    /// What the platform reports about the focused element when `read` returns
    /// None. Empty when there is nothing platform-specific to say.
    fn diagnose(&self) -> String {
        String::new()
    }
}

/// Microphone permission, for onboarding and the Home status line.
#[cfg(target_os = "macos")]
pub fn mic_status() -> Trust {
    mac_mic_trust(mac::mic_authorization_status())
}

#[cfg(windows)]
pub fn mic_status() -> Trust {
    windows_mic_trust(windows::mic_consent_value().as_deref())
}

/// `AVAuthorizationStatus` for audio → Trust. Not-yet-asked is
/// `NotRequired`: macOS asks the first time the user dictates.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_mic_trust(status: isize) -> Trust {
    match status {
        3 => Trust::Granted,     // authorized
        1 | 2 => Trust::Denied,  // restricted, denied
        _ => Trust::NotRequired, // notDetermined (0), or unknown
    }
}

/// The `Value` of Windows' microphone consent store → Trust. Only an
/// explicit "Deny" blocks the mic; a missing key means nobody turned it off.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_mic_trust(consent: Option<&str>) -> Trust {
    match consent {
        Some(v) if v.eq_ignore_ascii_case("deny") => Trust::Denied,
        _ => Trust::Granted,
    }
}

/// The paste shortcut, as the user would type it.
pub const PASTE_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘V"
} else {
    "Ctrl+V"
};

/// Correction learning needs a FocusReader that can actually read. Windows
/// (UI Automation) is not built in v1.
pub const LEARNING_SUPPORTED: bool = cfg!(target_os = "macos");

#[cfg(target_os = "macos")]
pub fn injector() -> &'static dyn TextInjector {
    &mac::MacInjector
}

#[cfg(windows)]
pub fn injector() -> &'static dyn TextInjector {
    &windows::WindowsInjector
}

#[cfg(target_os = "macos")]
pub fn focus_reader() -> Box<dyn FocusReader> {
    Box::new(crate::ax_read::SystemReader)
}

#[cfg(not(target_os = "macos"))]
pub fn focus_reader() -> Box<dyn FocusReader> {
    struct NoReader;
    impl FocusReader for NoReader {
        fn read(&self) -> Option<Snapshot> {
            None
        }
    }
    Box::new(NoReader)
}

/// Where a bundled sidecar binary lives: `<resources>/llama/<name>`, with
/// `.exe` on Windows.
pub fn sidecar_path(resource_dir: &Path, name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    resource_dir.join("llama").join(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_adds_exe_only_on_windows() {
        let p = sidecar_path(Path::new("/r"), "llama-server");
        if cfg!(windows) {
            assert!(p.ends_with("llama/llama-server.exe"));
        } else {
            assert!(p.ends_with("llama/llama-server"));
        }
    }

    #[test]
    fn trust_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&Trust::NotRequired).unwrap(),
            "\"not_required\""
        );
    }

    #[test]
    fn only_denied_blocks_paste() {
        assert!(Trust::Granted.can_paste());
        assert!(Trust::NotRequired.can_paste());
        assert!(!Trust::Denied.can_paste());
    }

    #[test]
    fn mac_mic_status_maps_every_authorization_state() {
        assert_eq!(mac_mic_trust(0), Trust::NotRequired);
        assert_eq!(mac_mic_trust(1), Trust::Denied);
        assert_eq!(mac_mic_trust(2), Trust::Denied);
        assert_eq!(mac_mic_trust(3), Trust::Granted);
        assert_eq!(mac_mic_trust(42), Trust::NotRequired);
    }

    /// Exercises the real AVFoundation call: it must answer a valid status
    /// (reading never prompts, so this is safe on CI).
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_mic_status_reads_a_real_status() {
        assert!((0..=3).contains(&mac::mic_authorization_status()));
    }

    /// Exercises the real registry read: present or not, it must not fail.
    #[cfg(windows)]
    #[test]
    fn windows_mic_status_reads_the_registry() {
        let _ = super::windows::mic_consent_value();
        assert_ne!(mic_status(), Trust::NotRequired);
    }

    #[test]
    fn windows_mic_status_is_denied_only_by_an_explicit_deny() {
        assert_eq!(windows_mic_trust(Some("Deny")), Trust::Denied);
        assert_eq!(windows_mic_trust(Some("deny")), Trust::Denied);
        assert_eq!(windows_mic_trust(Some("Allow")), Trust::Granted);
        assert_eq!(windows_mic_trust(None), Trust::Granted);
    }

    #[test]
    fn learning_is_mac_only_in_v1() {
        assert_eq!(LEARNING_SUPPORTED, cfg!(target_os = "macos"));
    }
}

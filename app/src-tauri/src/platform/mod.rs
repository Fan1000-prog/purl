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
    /// macOS Accessibility is granted.
    #[cfg_attr(windows, allow(dead_code))]
    Granted,
    /// macOS Accessibility is not granted; paste falls back to the clipboard.
    #[cfg_attr(windows, allow(dead_code))]
    Denied,
    /// The OS has no such permission (Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
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
#[allow(dead_code)] // llama-server sidecar (plan Task 6) is the caller.
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
    fn learning_is_mac_only_in_v1() {
        assert_eq!(LEARNING_SUPPORTED, cfg!(target_os = "macos"));
    }
}

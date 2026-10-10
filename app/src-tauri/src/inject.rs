//! Text injection at the cursor: set the clipboard, synthesize the OS paste
//! chord, then restore the previous clipboard. The keystrokes themselves are
//! `platform::TextInjector`'s job; this module owns the clipboard choreography.

use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("failed to synthesize keystroke — is Accessibility permission granted?")]
    #[cfg_attr(windows, allow(dead_code))]
    KeyEvent,
    /// The OS refused the synthesized input (e.g. Windows UIPI blocks input
    /// into an elevated window).
    #[error("{0}")]
    #[cfg_attr(not(windows), allow(dead_code))]
    Synthesis(String),
}

/// Put `text` on the clipboard only (fallback mode — no permissions needed).
pub fn copy_to_clipboard(text: &str) -> Result<(), InjectError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    cb.set_text(text.to_string())
        .map_err(|e| InjectError::Clipboard(e.to_string()))
}

/// Inject `text` at the cursor of the frontmost app via clipboard + Cmd-V,
/// restoring whatever text was on the clipboard before.
pub fn inject(text: &str) -> Result<(), InjectError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    let previous = cb.get_text().ok();

    cb.set_text(text.to_string())
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;
    // Give the pasteboard a beat to settle before the paste keystroke.
    sleep(Duration::from_millis(80));

    let result = crate::platform::injector().paste();

    // Let the target app read the clipboard before restoring it.
    sleep(Duration::from_millis(250));
    if let Some(prev) = previous {
        let _ = cb.set_text(prev);
    }
    result
}

/// Sentinel written to the clipboard before the synthesized Cmd-C; if it is
/// still there afterwards, nothing was selected in the frontmost app.
const CAPTURE_SENTINEL: &str = "\u{2063}purl-capture\u{2063}";

/// Read the frontmost app's current selection by synthesizing Cmd-C.
/// Returns `(selection, previous_clipboard)`; `selection` is `None` when
/// nothing was copied. On the `None` and error paths the user's original
/// clipboard is already restored; when a selection IS returned the clipboard
/// holds that selection and the caller must restore `previous_clipboard`
/// after pasting the transformed result.
pub fn capture_selection() -> Result<(Option<String>, Option<String>), InjectError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    let previous = cb.get_text().ok();

    cb.set_text(CAPTURE_SENTINEL.to_string())
        .map_err(|e| InjectError::Clipboard(e.to_string()))?;
    sleep(Duration::from_millis(80));

    let restore = |cb: &mut arboard::Clipboard, prev: &Option<String>| match prev {
        Some(p) => {
            let _ = cb.set_text(p.clone());
        }
        None => {
            let _ = cb.clear();
        }
    };

    if let Err(e) = crate::platform::injector().copy() {
        restore(&mut cb, &previous);
        return Err(e);
    }
    // Give the frontmost app a beat to service the copy event.
    sleep(Duration::from_millis(180));

    match cb.get_text() {
        Ok(text) if text != CAPTURE_SENTINEL && !text.trim().is_empty() => {
            Ok((Some(text), previous))
        }
        _ => {
            restore(&mut cb, &previous);
            Ok((None, previous))
        }
    }
}

/// Put `text` back on the clipboard (used to restore what the user had
/// before a transform).
pub fn restore_clipboard(previous: &Option<String>) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        match previous {
            Some(p) => {
                let _ = cb.set_text(p.clone());
            }
            None => {
                let _ = cb.clear();
            }
        }
    }
}

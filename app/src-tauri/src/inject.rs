//! Text injection at the cursor: set the clipboard, synthesize the OS paste
//! chord, then restore the previous clipboard. The keystrokes themselves are
//! `platform::TextInjector`'s job; this module owns the clipboard choreography.

use crate::platform::TextInjector;
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

/// The clipboard calls `inject` makes; a seam so tests needn't touch the
/// real clipboard.
trait Clipboard {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str) -> Result<(), InjectError>;
}

impl Clipboard for arboard::Clipboard {
    fn get_text(&mut self) -> Option<String> {
        arboard::Clipboard::get_text(self).ok()
    }
    fn set_text(&mut self, text: &str) -> Result<(), InjectError> {
        arboard::Clipboard::set_text(self, text.to_string())
            .map_err(|e| InjectError::Clipboard(e.to_string()))
    }
}

/// Inject `text` at the cursor of the frontmost app via clipboard + the OS
/// paste chord, restoring whatever text was on the clipboard before. If the
/// paste keystroke fails, `text` is left on the clipboard so the user can
/// paste it by hand. `InjectError::Clipboard` means `text` never reached the
/// clipboard.
pub fn inject(text: &str) -> Result<(), InjectError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?;
    inject_with(&mut cb, crate::platform::injector(), text)
}

fn inject_with(
    cb: &mut dyn Clipboard,
    injector: &dyn TextInjector,
    text: &str,
) -> Result<(), InjectError> {
    let previous = cb.get_text();
    cb.set_text(text)?;
    // Give the pasteboard a beat to settle before the paste keystroke.
    sleep(Duration::from_millis(80));

    injector.paste()?;

    // Let the target app read the clipboard before restoring it.
    sleep(Duration::from_millis(250));
    if let Some(prev) = previous {
        let _ = cb.set_text(&prev);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Trust;
    use crate::voice_commands::Key;

    struct FakeClipboard(Option<String>);

    impl Clipboard for FakeClipboard {
        fn get_text(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn set_text(&mut self, text: &str) -> Result<(), InjectError> {
            self.0 = Some(text.to_string());
            Ok(())
        }
    }

    struct FakeInjector {
        paste_ok: bool,
    }

    impl TextInjector for FakeInjector {
        fn paste(&self) -> Result<(), InjectError> {
            if self.paste_ok {
                Ok(())
            } else {
                Err(InjectError::Synthesis("blocked".into()))
            }
        }
        fn copy(&self) -> Result<(), InjectError> {
            Ok(())
        }
        fn send_key(&self, _key: Key) -> Result<(), InjectError> {
            Ok(())
        }
        fn can_inject(&self) -> Trust {
            Trust::NotRequired
        }
    }

    #[test]
    fn successful_paste_restores_the_old_clipboard() {
        let mut cb = FakeClipboard(Some("old".into()));
        inject_with(&mut cb, &FakeInjector { paste_ok: true }, "new").unwrap();
        assert_eq!(cb.0.as_deref(), Some("old"));
    }

    #[test]
    fn failed_paste_leaves_the_text_on_the_clipboard() {
        let mut cb = FakeClipboard(Some("old".into()));
        let r = inject_with(&mut cb, &FakeInjector { paste_ok: false }, "new");
        assert!(matches!(r, Err(InjectError::Synthesis(_))));
        assert_eq!(cb.0.as_deref(), Some("new"));
    }

    /// A clipboard that can be read but refuses every write.
    struct ReadOnlyClipboard(Option<String>);

    impl Clipboard for ReadOnlyClipboard {
        fn get_text(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn set_text(&mut self, _: &str) -> Result<(), InjectError> {
            Err(InjectError::Clipboard("busy".into()))
        }
    }

    #[test]
    fn a_clipboard_error_means_the_text_never_got_there() {
        let mut cb = ReadOnlyClipboard(Some("old".into()));
        let r = inject_with(&mut cb, &FakeInjector { paste_ok: true }, "new");
        assert!(matches!(r, Err(InjectError::Clipboard(_))));
        assert_eq!(cb.0.as_deref(), Some("old"));
    }
}

//! Text injection at the cursor: set the clipboard, synthesize Cmd-V, then
//! restore the previous clipboard. Requires the Accessibility permission.

use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("failed to synthesize keystroke — is Accessibility permission granted?")]
    KeyEvent,
}

/// kVK_ANSI_V
const KEYCODE_V: u16 = 9;
/// kVK_ANSI_C
const KEYCODE_C: u16 = 8;

/// True when this process is trusted for Accessibility (needed for Cmd-V).
pub fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Like `accessibility_trusted`, but shows the macOS grant dialog when not
/// trusted. The grant is tied to the exact binary, so every dev rebuild
/// silently revokes it — prompting at startup makes that visible.
pub fn prompt_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::c_void;
        use std::ptr;

        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
            static kAXTrustedCheckOptionPrompt: *const c_void;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            fn CFDictionaryCreate(
                allocator: *const c_void,
                keys: *const *const c_void,
                values: *const *const c_void,
                num_values: isize,
                key_callbacks: *const c_void,
                value_callbacks: *const c_void,
            ) -> *const c_void;
            fn CFRelease(cf: *const c_void);
            static kCFTypeDictionaryKeyCallBacks: c_void;
            static kCFTypeDictionaryValueCallBacks: c_void;
            static kCFBooleanTrue: *const c_void;
        }

        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            let trusted = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            trusted
        }
    }
    #[cfg(not(target_os = "macos"))]
    false
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

    let result = send_cmd_v();

    // Let the target app read the clipboard before restoring it.
    sleep(Duration::from_millis(250));
    if let Some(prev) = previous {
        let _ = cb.set_text(prev);
    }
    result
}

#[cfg(target_os = "macos")]
fn send_cmd_v() -> Result<(), InjectError> {
    send_keystroke(KEYCODE_V, true)
}

#[cfg(target_os = "macos")]
fn send_cmd_c() -> Result<(), InjectError> {
    send_keystroke(KEYCODE_C, true)
}

#[cfg(not(target_os = "macos"))]
fn send_cmd_c() -> Result<(), InjectError> {
    Err(InjectError::KeyEvent)
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

    let restore = |cb: &mut arboard::Clipboard, prev: &Option<String>| {
        match prev {
            Some(p) => { let _ = cb.set_text(p.clone()); }
            None => { let _ = cb.clear(); }
        }
    };

    if let Err(e) = send_cmd_c() {
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
            Some(p) => { let _ = cb.set_text(p.clone()); }
            None => { let _ = cb.clear(); }
        }
    }
}

/// Synthesize a single unmodified keystroke (arrow keys, Return, Tab…) —
/// used by voice commands like "go down". Same permission requirement as
/// pasting: the Accessibility grant.
pub fn send_key(keycode: u16) -> Result<(), InjectError> {
    send_keystroke(keycode, false)
}

#[cfg(target_os = "macos")]
fn send_keystroke(keycode: u16, with_command: bool) -> Result<(), InjectError> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| InjectError::KeyEvent)?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| InjectError::KeyEvent)?;
    let up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| InjectError::KeyEvent)?;
    if with_command {
        down.set_flags(CGEventFlags::CGEventFlagCommand);
        up.set_flags(CGEventFlags::CGEventFlagCommand);
    }

    down.post(CGEventTapLocation::HID);
    sleep(Duration::from_millis(30));
    up.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn send_keystroke(_keycode: u16, _with_command: bool) -> Result<(), InjectError> {
    Err(InjectError::KeyEvent)
}

#[cfg(not(target_os = "macos"))]
fn send_cmd_v() -> Result<(), InjectError> {
    Err(InjectError::KeyEvent)
}

//! macOS: CGEvent keystrokes, gated by the Accessibility permission.

use super::{TextInjector, Trust};
use crate::inject::InjectError;
use crate::voice_commands::Key;
use std::thread::sleep;
use std::time::Duration;

/// kVK_ANSI_V
const KEYCODE_V: u16 = 9;
/// kVK_ANSI_C
const KEYCODE_C: u16 = 8;

/// macOS virtual keycodes for the keys voice commands can press.
fn mac_keycode(k: Key) -> u16 {
    match k {
        Key::Left => 123,
        Key::Right => 124,
        Key::Down => 125,
        Key::Up => 126,
        Key::Return => 36,
        Key::Tab => 48,
    }
}

pub struct MacInjector;

impl TextInjector for MacInjector {
    fn paste(&self) -> Result<(), InjectError> {
        send_keystroke(KEYCODE_V, true)
    }
    fn copy(&self) -> Result<(), InjectError> {
        send_keystroke(KEYCODE_C, true)
    }
    fn send_key(&self, key: Key) -> Result<(), InjectError> {
        send_keystroke(mac_keycode(key), false)
    }
    fn can_inject(&self) -> Trust {
        trust(accessibility_trusted())
    }
    fn request_trust(&self) -> Trust {
        trust(prompt_accessibility())
    }
}

fn trust(granted: bool) -> Trust {
    if granted {
        Trust::Granted
    } else {
        Trust::Denied
    }
}

/// True when this process is trusted for Accessibility (needed for Cmd-V).
fn accessibility_trusted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

/// Like `accessibility_trusted`, but shows the macOS grant dialog when not
/// trusted. The grant is tied to the exact binary, so every dev rebuild
/// silently revokes it — prompting at startup makes that visible.
fn prompt_accessibility() -> bool {
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

fn send_keystroke(keycode: u16, with_command: bool) -> Result<(), InjectError> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| InjectError::KeyEvent)?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| InjectError::KeyEvent)?;
    let up =
        CGEvent::new_keyboard_event(source, keycode, false).map_err(|_| InjectError::KeyEvent)?;
    if with_command {
        down.set_flags(CGEventFlags::CGEventFlagCommand);
        up.set_flags(CGEventFlags::CGEventFlagCommand);
    }

    down.post(CGEventTapLocation::HID);
    sleep(Duration::from_millis(30));
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// `[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]` as
/// its raw `AVAuthorizationStatus` (0 notDetermined, 1 restricted, 2 denied,
/// 3 authorized). Reading it never prompts.
pub fn mic_authorization_status() -> isize {
    use std::ffi::{c_char, c_void};

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *const c_void;
    }
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *const c_void;
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
    }

    unsafe {
        let class = objc_getClass(c"AVCaptureDevice".as_ptr());
        if class.is_null() {
            return 0;
        }
        let sel = sel_registerName(c"authorizationStatusForMediaType:".as_ptr());
        // objc_msgSend must be called through the exact prototype of the
        // method it dispatches to.
        let send: unsafe extern "C" fn(*const c_void, *const c_void, *const c_void) -> isize =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        send(class, sel, AVMediaTypeAudio)
    }
}

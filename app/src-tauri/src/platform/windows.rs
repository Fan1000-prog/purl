//! Windows: SendInput keystrokes. There is no permission to grant, but
//! UIPI silently drops input aimed at an elevated window, which SendInput
//! reports as fewer events inserted than sent.

use super::{TextInjector, Trust};
use crate::inject::InjectError;
use crate::voice_commands::Key;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_DOWN, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

fn vk(k: Key) -> VIRTUAL_KEY {
    match k {
        Key::Return => VK_RETURN,
        Key::Tab => VK_TAB,
        Key::Up => VK_UP,
        Key::Down => VK_DOWN,
        Key::Left => VK_LEFT,
        Key::Right => VK_RIGHT,
    }
}

fn key_event(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Key-up events for `keys`, in order.
fn release_sequence(keys: &[VIRTUAL_KEY]) -> Vec<INPUT> {
    keys.iter().map(|&k| key_event(k, true)).collect()
}

/// Send `seq`, whose keys `held` must not be left down. A short count means
/// some events were inserted and some weren't, so a key-down may have gone
/// through without its key-up; a stuck Ctrl would turn the user's next
/// keystrokes into shortcuts. Release `held` best-effort and report failure.
///
/// A full count doesn't prove delivery: UIPI can drop input aimed at an
/// elevated window while SendInput still reports every event inserted.
fn send(seq: &[INPUT], held: &[VIRTUAL_KEY]) -> Result<(), InjectError> {
    let size = std::mem::size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(seq, size) };
    if sent as usize == seq.len() {
        return Ok(());
    }
    let _ = unsafe { SendInput(&release_sequence(held), size) };
    Err(InjectError::Synthesis(
        "SendInput was blocked (the target may be running as administrator)".into(),
    ))
}

fn chord(modifier: VIRTUAL_KEY, key: VIRTUAL_KEY) -> Result<(), InjectError> {
    send(
        &[
            key_event(modifier, false),
            key_event(key, false),
            key_event(key, true),
            key_event(modifier, true),
        ],
        &[key, modifier],
    )
}

pub struct WindowsInjector;

impl TextInjector for WindowsInjector {
    fn paste(&self) -> Result<(), InjectError> {
        chord(VK_CONTROL, VIRTUAL_KEY(b'V' as u16))
    }
    fn copy(&self) -> Result<(), InjectError> {
        chord(VK_CONTROL, VIRTUAL_KEY(b'C' as u16))
    }
    fn send_key(&self, key: Key) -> Result<(), InjectError> {
        let k = vk(key);
        send(&[key_event(k, false), key_event(k, true)], &[k])
    }
    fn can_inject(&self) -> Trust {
        Trust::NotRequired
    }
}

/// The `Value` of the user's microphone consent store ("Allow" / "Deny"),
/// or None when the key is missing or unreadable.
pub fn mic_consent_value() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let mut buf = [0u16; 64];
    let mut len = std::mem::size_of_val(&buf) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone"),
            w!("Value"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    rc.ok().ok()?;
    // `len` is in bytes and includes the terminating NUL.
    let chars = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..chars]))
}

/// Turn a window into a tool window: no taskbar button and, unlike
/// `skip_taskbar` alone, no Alt-Tab entry. For the overlay pill.
pub fn make_tool_window(hwnd: HWND) {
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let ex = (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};

    #[test]
    fn tool_window_style_sticks() {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_APPWINDOW,
                w!("STATIC"),
                w!("purl-test"),
                WS_POPUP,
                0,
                0,
                10,
                10,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create a test window");
        make_tool_window(hwnd);
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        unsafe { DestroyWindow(hwnd) }.unwrap();
        assert_ne!(ex & WS_EX_TOOLWINDOW.0 as isize, 0);
        assert_eq!(ex & WS_EX_APPWINDOW.0 as isize, 0);
    }

    #[test]
    fn release_sequence_lifts_the_key_then_the_modifier() {
        let v = VIRTUAL_KEY(b'V' as u16);
        let seq = release_sequence(&[v, VK_CONTROL]);
        let events: Vec<_> = seq
            .iter()
            .map(|i| {
                (i.r#type, unsafe { i.Anonymous.ki.wVk }, unsafe {
                    i.Anonymous.ki.dwFlags
                })
            })
            .collect();
        assert_eq!(
            events,
            vec![
                (INPUT_KEYBOARD, v, KEYEVENTF_KEYUP),
                (INPUT_KEYBOARD, VK_CONTROL, KEYEVENTF_KEYUP),
            ]
        );
    }
}

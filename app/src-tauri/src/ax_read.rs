//! macOS `FocusReader`: the only Accessibility read surface in the app.
//! Everything above it works against `platform::FocusReader`, so no other
//! module needs a Mac to be tested.
//!
//! `read()` returning None is ordinary, not exceptional: some target apps
//! genuinely expose nothing (e.g. a canvas-only view with no focused
//! element at all), and the feature no-ops there.
//!
//! Chromium-based apps — Chrome (Gmail, Google Chat) and Electron apps like
//! VS Code — are *not* such a case, though they looked like one for two
//! releases. **The route to the focused element was the bug, not the element.**
//! `AXUIElementCreateSystemWide()` + `AXFocusedUIElement` is unreliable: it
//! returns -25204 (cannotComplete, i.e. the target didn't answer in time) often
//! enough that the feature never captured anything, and `AXFocusedApplication`
//! on the system-wide element fails the same way. Going through the frontmost
//! *application's* element answers immediately with a readable `AXTextArea`.
//! `focused_element` therefore prefers the application route and keeps
//! system-wide only as a fallback.
//!
//! Two earlier diagnoses were wrong and are recorded here so they aren't
//! re-attempted: that Chromium withholds `AXValue` from non-AT callers (it does
//! gate its tree, but that was not what broke this — VS Code hands over
//! `AXValue` with no coaxing), and that the retry had to re-query because the
//! handle went stale (it doesn't). `AXManualAccessibility` is kept only as a
//! cheap nudge for Electron apps that genuinely withhold the tree; Chrome
//! rejects it outright with -25205, so nothing may depend on it.
//!
//! `diagnose()` and the ignored `ax_live_focus_report` test exist because this
//! surface cannot be unit-tested: `FocusReader` is mocked at exactly the
//! boundary where these bugs live, so the suite passed green through both bad
//! diagnoses. Measure here before believing anything about this file.

use crate::corrections::Snapshot;
use crate::platform::FocusReader;

pub struct SystemReader;

mod mac {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::os::raw::c_char;
    use std::sync::Mutex;

    pub type CFTypeRef = *const c_void;
    pub type AXError = i32;
    const AX_SUCCESS: AXError = 0;
    /// Not an OS code — ours, for "we failed before the call was made".
    const AX_ERR_UNKNOWN: AXError = 1;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> CFTypeRef;
        fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
        fn AXUIElementCopyAttributeValue(
            element: CFTypeRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementSetAttributeValue(
            element: CFTypeRef,
            attribute: CFTypeRef,
            value: CFTypeRef,
        ) -> AXError;
        fn AXUIElementGetPid(element: CFTypeRef, pid: *mut i32) -> AXError;
        fn AXUIElementCopyAttributeNames(element: CFTypeRef, names: *mut CFTypeRef) -> AXError;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFStringCreateWithCString(
            alloc: CFTypeRef,
            cstr: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFStringGetCString(
            s: CFTypeRef,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> bool;
        fn CFStringGetLength(s: CFTypeRef) -> isize;
        fn CFArrayGetCount(a: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(a: CFTypeRef, idx: isize) -> CFTypeRef;
        fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFNumberGetValue(n: CFTypeRef, the_type: isize, out: *mut c_void) -> bool;
        static kCFBooleanTrue: CFTypeRef;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFTypeRef;
    }

    const K_CG_WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
    const K_CG_NULL_WINDOW_ID: u32 = 0;
    const K_CF_NUMBER_SINT32_TYPE: isize = 3;

    /// The pid owning the frontmost on-screen window.
    ///
    /// Needed because the system-wide AX element is not a usable route to the
    /// focused element: `AXUIElementCreateSystemWide()` + `AXFocusedUIElement`
    /// returns -25204 (cannotComplete) for Electron targets, reproducibly and
    /// no matter how long you wait, and so does `AXFocusedApplication`. Asking
    /// the *application's* element instead answers immediately. So we need the
    /// app, and the only way to it is outside AX.
    ///
    /// CoreGraphics lists on-screen windows front-to-back; the first at layer 0
    /// is the frontmost real window (higher layers are menus, docks, overlays).
    fn frontmost_pid() -> Option<i32> {
        let info = Owned(unsafe {
            CGWindowListCopyWindowInfo(K_CG_WINDOW_LIST_ON_SCREEN_ONLY, K_CG_NULL_WINDOW_ID)
        });
        if info.0.is_null() {
            return None;
        }
        let layer_key = cfstr("kCGWindowLayer")?;
        let pid_key = cfstr("kCGWindowOwnerPID")?;

        let count = unsafe { CFArrayGetCount(info.0) };
        for i in 0..count {
            let win = unsafe { CFArrayGetValueAtIndex(info.0, i) };
            if win.is_null() {
                continue;
            }
            if cf_number_i32(unsafe { CFDictionaryGetValue(win, layer_key.0) }) != Some(0) {
                continue;
            }
            if let Some(pid) = cf_number_i32(unsafe { CFDictionaryGetValue(win, pid_key.0) }) {
                return Some(pid);
            }
        }
        None
    }

    fn cf_number_i32(n: CFTypeRef) -> Option<i32> {
        if n.is_null() {
            return None;
        }
        let mut out: i32 = 0;
        let ok = unsafe {
            CFNumberGetValue(
                n,
                K_CF_NUMBER_SINT32_TYPE,
                &mut out as *mut i32 as *mut c_void,
            )
        };
        ok.then_some(out)
    }

    /// An owned CF reference, released on drop. Every `Copy`/`Create` call
    /// hands back +1; forgetting one leaks for the life of the process, and
    /// this watcher runs on every dictation.
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) }
            }
        }
    }

    fn cfstr(s: &str) -> Option<Owned> {
        let c = std::ffi::CString::new(s).ok()?;
        let r = unsafe {
            CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        (!r.is_null()).then_some(Owned(r))
    }

    fn to_string(cf: CFTypeRef) -> Option<String> {
        if cf.is_null() || unsafe { CFGetTypeID(cf) } != unsafe { CFStringGetTypeID() } {
            return None;
        }
        // UTF-8 can need 4 bytes per UTF-16 unit, plus the terminator.
        let cap = unsafe { CFStringGetLength(cf) } * 4 + 1;
        let mut buf = vec![0i8; cap as usize];
        let ok =
            unsafe { CFStringGetCString(cf, buf.as_mut_ptr(), cap, K_CF_STRING_ENCODING_UTF8) };
        if !ok {
            return None;
        }
        let bytes: Vec<u8> = buf
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect();
        String::from_utf8(bytes).ok()
    }

    fn attr(element: CFTypeRef, name: &str) -> Option<Owned> {
        attr_checked(element, name).1
    }

    /// Same as `attr`, but also hands back the raw `AXError`. Diagnostics need
    /// to tell "the attribute is genuinely absent" (-25205) apart from "the app
    /// isn't answering" (-25212, -25204), which a bare Option flattens away.
    fn attr_checked(element: CFTypeRef, name: &str) -> (AXError, Option<Owned>) {
        let Some(key) = cfstr(name) else {
            return (AX_ERR_UNKNOWN, None);
        };
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(element, key.0, &mut out) };
        let owned = (err == AX_SUCCESS && !out.is_null()).then_some(Owned(out));
        (err, owned)
    }

    /// The attribute names an element actually exposes. Names only — never
    /// values, so this cannot leak field contents into the log.
    fn attribute_names(element: CFTypeRef) -> Vec<String> {
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeNames(element, &mut out) };
        if err != AX_SUCCESS || out.is_null() {
            return Vec::new();
        }
        let names = Owned(out);
        let count = unsafe { CFArrayGetCount(names.0) };
        (0..count)
            .filter_map(|i| to_string(unsafe { CFArrayGetValueAtIndex(names.0, i) }))
            .collect()
    }

    /// How many times we'll ask one pid for the full tree before accepting
    /// that it is never going to produce one. Spans dictations, not polls, so
    /// it needs headroom: a single watch (up to 90s) can ask several times.
    const MAX_ENHANCE_ATTEMPTS: u8 = 6;

    /// Attempts spent per pid. Not a plain "already done" set: the request can
    /// legitimately fail to take effect — dictating into Chrome while it is
    /// still starting up, for instance — and a set would cache that first
    /// miss for the rest of the app's life, silently breaking learning in that
    /// app. A budget lets later watches retry, while still bounding the write
    /// so a watch polling every 600ms for up to 90s doesn't reissue it ~150 times.
    /// The same budget covers pid reuse after a process exits.
    static ENHANCE_ATTEMPTS: Mutex<Option<HashMap<i32, u8>>> = Mutex::new(None);

    /// Chromium (Chrome, and any Chromium-based app) only builds its full
    /// accessibility tree — including `AXValue` for contenteditable regions
    /// like Gmail's compose box and Google Chat's message box — for clients
    /// it recognizes as an assistive technology (VoiceOver, Switch Control).
    /// A plain `AXUIElementCopyAttributeValue` caller is not on that list, so
    /// Chrome serves a reduced tree where the focused element's `AXValue` is
    /// absent even though the field is focused and editable. Setting
    /// `AXManualAccessibility` true on the browser's application-level
    /// element is Chromium's documented escape hatch: it forces full tree
    /// construction for any caller, not just recognized AT clients. This is
    /// idempotent and cheap.
    ///
    /// Returns whether the request was actually issued this call. Callers use
    /// that to decide whether waiting for the tree to rebuild is worth it: if
    /// we didn't just ask, there is nothing new to wait for.
    pub(super) fn ensure_enhanced_accessibility(pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        {
            // A poisoned lock must not take the watch thread down with it —
            // failing to enhance is a degraded read, not a crash.
            let Ok(mut guard) = ENHANCE_ATTEMPTS.lock() else {
                return false;
            };
            let spent = guard
                .get_or_insert_with(HashMap::new)
                .entry(pid)
                .or_insert(0);
            if *spent >= MAX_ENHANCE_ATTEMPTS {
                return false;
            }
            *spent += 1;
        }
        set_manual_accessibility(pid) == AX_SUCCESS
    }

    /// Attempts already spent on a pid. Test-only: the budget is the one piece
    /// of this module's behavior that is real logic rather than FFI, so it is
    /// worth asserting directly instead of inferring it from a return value
    /// that a bogus pid makes indistinguishable from failure.
    #[cfg(test)]
    pub(super) fn attempts_spent(pid: i32) -> u8 {
        ENHANCE_ATTEMPTS
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|m| m.get(&pid).copied()))
            .unwrap_or(0)
    }

    /// Issues the request unconditionally and returns the raw `AXError`, so
    /// diagnostics can report whether the app accepted it, rejected it
    /// (-25205 = no such attribute, i.e. not a Chromium app or too old), or
    /// never answered (-25212 = invalid element, -25204 = cannot complete).
    fn set_manual_accessibility(pid: i32) -> AXError {
        let Some(key) = cfstr("AXManualAccessibility") else {
            return AX_ERR_UNKNOWN;
        };
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        if app.0.is_null() {
            return AX_ERR_UNKNOWN;
        }
        unsafe { AXUIElementSetAttributeValue(app.0, key.0, kCFBooleanTrue) }
    }

    /// A snapshot of one element, or None if it exposes no readable text.
    /// A password field short-circuits to an empty secure snapshot — never read,
    /// not even to be discarded.
    fn snapshot_of(el: CFTypeRef) -> Option<Snapshot> {
        let subrole = attr(el, "AXSubrole").and_then(|o| to_string(o.0));
        if subrole.as_deref() == Some("AXSecureTextField") {
            return Some(Snapshot {
                id: String::new(),
                value: String::new(),
                secure: true,
            });
        }
        // AXSelectedText as a fallback: some Chromium roles populate it even
        // when AXValue itself stays empty.
        let value = attr(el, "AXValue")
            .and_then(|o| to_string(o.0))
            .or_else(|| attr(el, "AXSelectedText").and_then(|o| to_string(o.0)))?;

        let mut pid: i32 = 0;
        unsafe { AXUIElementGetPid(el, &mut pid) };
        let role = attr(el, "AXRole")
            .and_then(|o| to_string(o.0))
            .unwrap_or_default();
        let title = attr(el, "AXTitle")
            .and_then(|o| to_string(o.0))
            .unwrap_or_default();
        Some(Snapshot {
            id: format!("{pid}:{role}:{title}"),
            value,
            secure: false,
        })
    }

    /// The focused element, preferring the frontmost application's own element
    /// over the system-wide one. Order matters and is the whole fix: measured
    /// against VS Code, the system-wide route returns -25204 on every attempt
    /// while the application route returns a readable `AXTextArea` immediately.
    /// System-wide stays as a fallback for apps where the window list gives us
    /// no usable pid.
    fn focused_element() -> Option<(Owned, i32)> {
        if let Some(pid) = frontmost_pid() {
            let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
            if !app.0.is_null() {
                if let Some(focused) = attr(app.0, "AXFocusedUIElement") {
                    return Some((focused, pid));
                }
                // Electron can withhold the tree until asked. Cheap, and only
                // this app's pid is ever touched.
                if ensure_enhanced_accessibility(pid) {
                    for wait_ms in [50, 150, 300] {
                        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                        if let Some(focused) = attr(app.0, "AXFocusedUIElement") {
                            return Some((focused, pid));
                        }
                    }
                }
            }
        }
        let system = Owned(unsafe { AXUIElementCreateSystemWide() });
        if system.0.is_null() {
            return None;
        }
        let focused = attr(system.0, "AXFocusedUIElement")?;
        let mut pid: i32 = 0;
        unsafe { AXUIElementGetPid(focused.0, &mut pid) };
        Some((focused, pid))
    }

    pub fn read_focus() -> Option<Snapshot> {
        let (focused, _pid) = focused_element()?;
        snapshot_of(focused.0)
    }

    /// What the Accessibility API actually reports about the focused element,
    /// for the log. Attribute *names*, roles and error codes only — never a
    /// field's contents, so this cannot leak what the user typed.
    pub(super) fn diagnose() -> String {
        let front = frontmost_pid();

        // Both routes, reported separately: the whole v0.7.1 misdiagnosis came
        // from not knowing which of the two was failing.
        let system = Owned(unsafe { AXUIElementCreateSystemWide() });
        let sys_err = if system.0.is_null() {
            AX_ERR_UNKNOWN
        } else {
            attr_checked(system.0, "AXFocusedUIElement").0
        };

        let Some(pid) = front else {
            return format!("no frontmost window pid; system_wide_focus_err={sys_err}");
        };
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        let app_name = attr(app.0, "AXTitle")
            .and_then(|o| to_string(o.0))
            .unwrap_or_default();
        let (app_focus_err, focused) = attr_checked(app.0, "AXFocusedUIElement");

        let Some(focused) = focused else {
            return format!(
                "pid={pid} app={app_name:?} app_focus_err={app_focus_err} \
                 system_wide_focus_err={sys_err} manual_set_err={}",
                set_manual_accessibility(pid)
            );
        };

        let role = attr(focused.0, "AXRole")
            .and_then(|o| to_string(o.0))
            .unwrap_or_default();
        let subrole = attr(focused.0, "AXSubrole")
            .and_then(|o| to_string(o.0))
            .unwrap_or_default();
        let (value_err, value) = attr_checked(focused.0, "AXValue");
        let (sel_err, _) = attr_checked(focused.0, "AXSelectedText");
        // Length only, never the text itself.
        let value_len = value.and_then(|o| to_string(o.0)).map(|s| s.len());

        format!(
            "pid={pid} app={app_name:?} role={role:?} subrole={subrole:?} \
             AXValue_err={value_err} AXValue_len={value_len:?} AXSelectedText_err={sel_err} \
             app_focus_err={app_focus_err} system_wide_focus_err={sys_err} attrs={:?}",
            attribute_names(focused.0)
        )
    }
}

impl FocusReader for SystemReader {
    fn read(&self) -> Option<Snapshot> {
        mac::read_focus()
    }

    fn diagnose(&self) -> String {
        mac::diagnose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Option<Snapshot>);
    impl FocusReader for Fake {
        fn read(&self) -> Option<Snapshot> {
            self.0.clone()
        }
    }

    #[test]
    fn the_trait_is_object_safe_and_returns_snapshots() {
        let r: Box<dyn FocusReader> = Box::new(Fake(Some(Snapshot {
            id: "123:AXTextArea:Compose".into(),
            value: "hello".into(),
            secure: false,
        })));
        assert_eq!(r.read().unwrap().value, "hello");
    }

    #[test]
    fn the_real_reader_never_panics_when_nothing_is_focused() {
        // Under `cargo test` no app is frontmost; the contract is None, not a
        // crash. This is the only test that touches the real FFI.
        let _ = SystemReader.read();
    }

    // The Chrome full-accessibility-tree fallback (`ensure_enhanced_accessibility`,
    // `read_value`) has no seam a fake `FocusReader` can exercise: the bug it
    // fixes is Chrome's real AXUIElement tree omitting `AXValue` for
    // contenteditable regions (Gmail compose, Google Chat), which only a live
    // Chrome process reproduces. `FocusReader` is mocked at exactly the
    // boundary that hides this — a fake can only assert the trait contract,
    // not the FFI behavior underneath it. What's covered here instead: the
    // helper never panics on pids that can't correspond to a real AX
    // application (0, and one vanishingly unlikely to be alive), so a bad
    // read doesn't crash the watch thread.
    //
    // NOT yet confirmed against a live Chrome/VS Code correction cycle — that
    // needs a machine with Accessibility trust granted to the app. Until
    // someone does that run, treat this fix as diagnosed-and-built, not proven.
    #[test]
    fn enhanced_accessibility_request_never_panics_on_bogus_pids() {
        assert!(
            !mac::ensure_enhanced_accessibility(0),
            "pid 0 is not a process"
        );
        assert!(
            !mac::ensure_enhanced_accessibility(-1),
            "negative pids are not processes"
        );
    }

    // diagnose() runs against the live system-wide AX element and is called on
    // the failure path, where a panic would take the watch thread down. It must
    // survive whatever this machine's focus happens to be, including no
    // Accessibility trust at all, and must always say something.
    #[test]
    fn diagnose_always_returns_something_and_never_panics() {
        let d = SystemReader.diagnose();
        assert!(!d.is_empty(), "the failure path needs a reason to log");
    }

    /// Prints what this machine's Accessibility API actually reports, right now,
    /// for whatever is focused. Ignored by default because it depends on live
    /// focus and on the running process holding Accessibility trust.
    ///
    /// This is the only way to check this module for real. Every automated test
    /// here stops at `FocusReader`, which is mocked precisely where the FFI bugs
    /// live — two releases shipped green suites with the feature dead. Run it
    /// with a text field focused whenever this file changes:
    ///
    ///   cargo test -- --ignored --nocapture ax_live_focus_report
    #[test]
    #[ignore]
    fn ax_live_focus_report() {
        eprintln!("diagnose(): {}", SystemReader.diagnose());
        match SystemReader.read() {
            // Length, not contents — this prints to a terminal.
            Some(s) => eprintln!(
                "read(): OK id={:?} len={} secure={}",
                s.id,
                s.value.len(),
                s.secure
            ),
            None => eprintln!("read(): None — the feature would no-op here"),
        }
    }

    // The attempt budget keeps a watch (up to 90s) from reissuing the AX write on every
    // 600ms poll, while still letting a later dictation retry an app that
    // wasn't ready the first time (a cold Chrome). Asserted on the counter
    // rather than the return value: `ensure_enhanced_accessibility` reports
    // whether the *app accepted* the request, which a bogus pid never does, so
    // the return value can't distinguish "budget left" from "budget spent".
    #[test]
    fn enhance_attempts_stop_at_the_budget() {
        // A pid vanishingly unlikely to be a live AX application, so this
        // exercises the bookkeeping and never a real app's tree.
        let pid = i32::MAX - 1;
        assert_eq!(mac::attempts_spent(pid), 0, "untouched pid starts at zero");

        for _ in 0..20 {
            // Always false here: no real app to accept the attribute.
            assert!(!mac::ensure_enhanced_accessibility(pid));
        }
        let spent = mac::attempts_spent(pid);
        assert!(spent > 0, "attempts must actually be recorded");
        assert!(spent <= 6, "the budget must cap the writes, got {spent}");
    }

    // A pid that cannot name a process must not be charged against any budget
    // or reach the FFI at all.
    #[test]
    fn impossible_pids_are_rejected_before_the_ffi() {
        for pid in [0, -1, i32::MIN] {
            assert!(!mac::ensure_enhanced_accessibility(pid));
            assert_eq!(mac::attempts_spent(pid), 0, "pid {pid} must not be charged");
        }
    }
}

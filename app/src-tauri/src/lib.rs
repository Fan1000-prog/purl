mod audio;
#[cfg(target_os = "macos")]
mod ax_read;
mod cleanup;
mod corrections;
mod dictionary;
pub mod engine;
mod fastclean;
mod history;
mod inject;
mod learned;
mod platform;
mod secure_random;
mod settings;
mod snippets;
mod stats;
mod structure;
mod transforms;
mod untrusted;
mod voice_commands;

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use audio::Recorder;
use engine::llm::{LlamaServerEngine, LlmEngine};
use engine::stt::{ParakeetEngine, SttEngine};
use settings::{CleanupMode, InjectMode, Settings};

/// Dictation is push-to-talk: hold the hotkey to record, release to
/// transcribe. Releases faster than this are accidental taps — the recording
/// is discarded instead of running the pipeline on a fraction of a second
/// of audio.
const MIN_HOLD: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Default)]
struct AppState {
    recorder: Mutex<Option<Recorder>>,
    settings: Mutex<Settings>,
    /// Cached copy of transforms.json; hotkey routing reads it on every press.
    transforms: Mutex<Vec<transforms::Transform>>,
    /// Guards against overlapping pipeline runs; see `try_begin_session`.
    processing: Arc<AtomicBool>,
    last_result: Mutex<Option<TranscriptPayload>>,
    hotkey_press: Mutex<HotkeyPress>,
    /// Set when recording starts; drained by stop to compute record_ms.
    recording_started: Mutex<Option<std::time::Instant>>,
    /// The pill's clickable region — [x, y, width, height] in logical points
    /// relative to the overlay window's top-left. Reported by the overlay
    /// itself so its CSS stays the single source of truth. None while nothing
    /// it shows is interactive (e.g. the processing spinner).
    overlay_hit: Mutex<Option<[f64; 4]>>,
    /// Bumped by every `spawn_correction_watch`. A watcher only drives the
    /// `watching-edits` indicator while its generation is still the newest —
    /// otherwise an older watcher that outlives a newer one could flip the
    /// indicator back and forth behind the newer one's back.
    correction_watch_gen: std::sync::atomic::AtomicU64,
    /// Model downloads in flight, with their cancel flags and last errors.
    downloads: engine::Downloads,
    /// In-process speech engine. Set once in `setup`, where the app-data
    /// dir that holds its model is first known.
    stt: OnceLock<Arc<ParakeetEngine>>,
    /// Smart cleanup and transforms (llama-server sidecar). Set once in
    /// `setup`, like `stt`.
    llm: OnceLock<Arc<LlamaServerEngine>>,
}

#[derive(Default)]
struct HotkeyPress {
    /// True between Pressed and Released — macOS key-repeat fires extra
    /// Pressed events while the hotkey is held; those must be ignored.
    is_down: bool,
    pressed_at: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Serialize)]
struct TranscriptPayload {
    raw: String,
    cleaned: String,
    used_llm: bool,
    note: Option<String>,
    transcribe_ms: u128,
    cleanup_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
struct StatusPayload {
    stt: engine::EngineState,
    /// Whether a synthesized paste can reach other apps. `denied` means
    /// macOS Accessibility is missing and output goes to the clipboard.
    paste_trust: platform::Trust,
    /// False where correction learning isn't built (Windows in v1); the UI
    /// hides its toggle.
    learning_supported: bool,
    hotkey: String,
    state: String,
}

/// `StatusPayload` as a pure function of its inputs, so the platform fields
/// can be tested without an `AppHandle`.
fn status_payload_for(
    stt: engine::EngineState,
    trust: platform::Trust,
    hotkey: &str,
    state: &str,
) -> StatusPayload {
    StatusPayload {
        stt,
        paste_trust: trust,
        learning_supported: platform::LEARNING_SUPPORTED,
        hotkey: hotkey.into(),
        state: state.into(),
    }
}

/// Holds the one-session-at-a-time flag; dropping it frees the flag, so a
/// panicking pipeline thread can't leave Purl stuck "processing".
struct SessionGuard(Arc<AtomicBool>);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Claim the one-session-at-a-time guard. None when a dictation or
/// transform is already processing; the caller must then do nothing.
fn try_begin_session(busy: &Arc<AtomicBool>) -> Option<SessionGuard> {
    busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| SessionGuard(busy.clone()))
}

fn app_support_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn dictionary_path(app: &AppHandle) -> PathBuf {
    app_support_dir(app).join("dictionary.txt")
}

fn load_dictionary(app: &AppHandle) -> dictionary::Dictionary {
    dictionary::Dictionary::load(&dictionary_path(app))
}

fn learned_path(app: &AppHandle) -> std::path::PathBuf {
    app_support_dir(app).join("learned.jsonl")
}

/// Append a timestamped line to the diagnostic log in the app data dir.
/// Dictation failures are otherwise invisible (notifications may be off).
fn log_line(app: &AppHandle, msg: &str) {
    use std::io::Write;
    let dir = app_support_dir(app);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("purl.log"))
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

fn notify(app: &AppHandle, body: &str) {
    let _ = app.notification().builder().title("Purl").body(body).show();
}

fn set_state(app: &AppHandle, state: &str) {
    let _ = app.emit("state", state);
    if let Some(tray) = app.tray_by_id("main") {
        let title = match state {
            "recording" => Some("● REC"),
            "processing" => Some("…"),
            _ => None,
        };
        let _ = tray.set_title(title);
    }
    // The pill is a persistent fixture — always visible, never hidden —
    // and just switches which face it shows: idle mic, recording waveform,
    // or the processing spinner. The one exception is the brief hide at the
    // recording→processing hand-off: if the user clicked the pill's ✓, our
    // app took focus, and hiding returns it to the target app before the
    // paste keystroke. The spinner then re-shows without taking focus (the
    // window is non-focusable).
    if let Some(overlay) = app.get_webview_window("overlay") {
        if state == "processing" {
            let _ = overlay.hide();
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if let Some(overlay) = app.get_webview_window("overlay") {
                    position_overlay(&overlay);
                    let _ = overlay.show();
                }
            });
        } else {
            position_overlay(&overlay);
            let _ = overlay.show();
        }
    }
}

/// The cursor in logical (CG point) coordinates — the only coordinate space
/// that is global across displays on macOS. tao's cursor_position() takes the
/// logical NSEvent.mouseLocation and multiplies it by the *primary* display's
/// scale factor, while every per-monitor rect is scaled by that monitor's own
/// factor, so "physical" values from the two are not comparable on a
/// mixed-DPI setup (Retina laptop + 1x external). Divide the primary scale
/// back out and stay logical.
fn cursor_logical(overlay: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let pos = overlay.cursor_position().ok()?;
    let scale = overlay.primary_monitor().ok().flatten()?.scale_factor();
    Some((pos.x / scale, pos.y / scale))
}

/// The monitor currently under the cursor — the pill lives wherever the
/// user is actually looking, not always the primary display.
fn monitor_at_cursor(overlay: &tauri::WebviewWindow) -> Option<tauri::Monitor> {
    cursor_logical(overlay)
        .and_then(|(x, y)| overlay.monitor_from_point(x, y).ok().flatten())
        .or_else(|| overlay.primary_monitor().ok().flatten())
}

/// Park the pill bottom-center of whichever monitor has the cursor, just
/// above the Dock. Uses the monitor's work_area (its usable desktop space,
/// with the Dock and menu bar already excluded by the OS) rather than its
/// raw size — a raw-size calculation with a small margin can land the pill
/// behind the Dock's window layer, where it's rendered but invisible.
fn position_overlay(overlay: &tauri::WebviewWindow) {
    if let Some(monitor) = monitor_at_cursor(overlay) {
        // Work entirely in logical points. The monitor's work_area is
        // physical in *its* scale, but set_position with a PhysicalPosition
        // divides by the *window's current* scale — which is still the old
        // monitor's factor mid-move, so a physical hand-off lands the pill in
        // the wrong place whenever the two displays have different DPIs.
        let mscale = monitor.scale_factor();
        let work = monitor.work_area();
        let (wx, wy) = (
            work.position.x as f64 / mscale,
            work.position.y as f64 / mscale,
        );
        let (ww, wh) = (
            work.size.width as f64 / mscale,
            work.size.height as f64 / mscale,
        );
        let wscale = overlay.scale_factor().unwrap_or(1.0);
        let (ow, oh) = overlay
            .outer_size()
            .map(|s| (s.width as f64 / wscale, s.height as f64 / wscale))
            .unwrap_or((200.0, 82.0));
        let margin = 8.0;
        let x = wx + ((ww - ow).max(0.0) / 2.0);
        let y = wy + (wh - oh - margin).max(0.0);
        let _ = overlay.set_position(tauri::LogicalPosition { x, y });
    }
}

/// Re-park the pill whenever the cursor moves to a different display.
/// Polls rather than hooking a global mouse event — simplest cross-platform
/// option and cheap enough at this interval (a monitor identity comparison,
/// not a redraw) to leave running for the app's whole lifetime.
fn spawn_overlay_monitor_follow(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_monitor: Option<String> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let Some(overlay) = app.get_webview_window("overlay") else {
                continue;
            };
            let Some(monitor) = monitor_at_cursor(&overlay) else {
                continue;
            };
            // Identity by name, not origin — mirrored or stacked displays can
            // share an origin, which would suppress the re-park.
            let id = monitor
                .name()
                .cloned()
                .unwrap_or_else(|| format!("{},{}", monitor.position().x, monitor.position().y));
            if last_monitor.as_deref() != Some(id.as_str()) {
                last_monitor = Some(id);
                position_overlay(&overlay);
            }
        }
    });
}

/// Extra logical points around the pill that still count as "over it", so a
/// click landing between two hover polls isn't lost.
const OVERLAY_HIT_MARGIN: f64 = 6.0;

/// Keep the pill click-through except while the cursor is actually over it.
///
/// The overlay window (200x82 pt) is much bigger than the control it draws —
/// a 36 pt mic when idle — because it also has room for the recording pill and
/// the celebration toast. A normal window swallows mouse events across its
/// whole rect, not just where pixels are drawn, so the empty space around the
/// mic became an invisible dead zone: the pill is always-on-top and parked
/// bottom-center, exactly where apps like Google Meet put their control bar,
/// and clicks aimed at those controls died silently.
///
/// macOS delivers no mouse events at all to a click-through window, so hover
/// can't be detected in the webview. Instead the global cursor position is
/// polled against the bounds the overlay reports for itself, and the window
/// flips interactive only inside them.
fn spawn_overlay_click_through_watch(app: AppHandle) {
    std::thread::spawn(move || {
        // None until the first decision, so the first state is always applied.
        let mut ignoring: Option<bool> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(30));
            let Some(overlay) = app.get_webview_window("overlay") else {
                continue;
            };
            let hit = *app.state::<AppState>().overlay_hit.lock().unwrap();
            let over = overlay.is_visible().unwrap_or(false)
                && hit.is_some_and(|rect| cursor_over_overlay(&overlay, rect));
            if ignoring != Some(!over) {
                let _ = overlay.set_ignore_cursor_events(!over);
                ignoring = Some(!over);
            }
        }
    });
}

/// `rect` is [x, y, width, height] in logical points relative to the overlay
/// window. The whole test runs in logical points: the cursor's "physical"
/// value carries the primary display's scale while the window origin carries
/// its own monitor's, so comparing them raw breaks on mixed-DPI setups.
fn cursor_over_overlay(overlay: &tauri::WebviewWindow, rect: [f64; 4]) -> bool {
    let (Some((cx, cy)), Ok(origin), Ok(scale)) = (
        cursor_logical(overlay),
        overlay.outer_position(),
        overlay.scale_factor(),
    ) else {
        return false;
    };
    let (ox, oy) = (origin.x as f64 / scale, origin.y as f64 / scale);
    let [x, y, w, h] = rect;
    let m = OVERLAY_HIT_MARGIN;
    cx >= ox + x - m && cx <= ox + x + w + m && cy >= oy + y - m && cy <= oy + y + h + m
}

/// Stream the live mic level to the UI while recording.
fn spawn_level_emitter(app: AppHandle, level: std::sync::Arc<std::sync::atomic::AtomicU32>) {
    std::thread::spawn(move || {
        loop {
            if app.state::<AppState>().recorder.lock().unwrap().is_none() {
                break;
            }
            let rms = f32::from_bits(level.load(std::sync::atomic::Ordering::Relaxed));
            let _ = app.emit("audio-level", rms);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = app.emit("audio-level", 0.0f32);
    });
}

const WATCH_POLL: std::time::Duration = std::time::Duration::from_millis(600);
const WATCH_MAX_POLLS: usize = 150; // 90 seconds
/// After a commit, how many quiet polls before the watch declares the edit
/// settled and stops early.
const WATCH_SETTLED_POLLS: usize = 20; // 12 seconds

/// Everything the watch loop needs from the running app: the clock, the
/// indicator, the store, the log, and whether this watcher is still the
/// current one. Split out so `run_correction_watch` — where the exit paths
/// and the supersession rule live — can be tested without an `AppHandle`
/// and without waiting 90 real seconds.
trait WatchHost {
    fn wait(&self, d: std::time::Duration);
    /// Light or clear the disclosure dot.
    fn indicator(&self, on: bool);
    /// The watch's stable correction set changed: retract what the previous
    /// commit stored, persist the new set, and tell the UI about it. Either
    /// slice may be empty (first commit / full revert).
    fn relearn(&self, retract: &[corrections::Correction], learn: &[corrections::Correction]);
    /// Diagnostic line. Never carries text read out of another application.
    fn log(&self, line: &str);
    /// False once a newer dictation has superseded this watcher.
    fn is_current(&self) -> bool;
}

struct AppWatchHost {
    app: AppHandle,
    gen: u64,
}

impl WatchHost for AppWatchHost {
    fn wait(&self, d: std::time::Duration) {
        std::thread::sleep(d);
    }
    fn indicator(&self, on: bool) {
        let _ = self.app.emit("watching-edits", on);
    }
    fn relearn(&self, retract: &[corrections::Correction], learn: &[corrections::Correction]) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = learned_path(&self.app);
        for c in retract {
            if let Err(e) = learned::retract(&path, c) {
                self.log(&format!("learning: retract failed: {e}"));
            }
        }
        let mut stored = false;
        for c in learn {
            match learned::add(&path, c, ts) {
                Ok(()) => stored = true,
                Err(e) => self.log(&format!("learning: store failed: {e}")),
            }
        }
        // One toast per watch: the first commit. Later refinements of the
        // same edit replace the stored rules silently.
        if stored && retract.is_empty() {
            let _ = self.app.emit("correction-learned", &learn[0].to);
        }
    }
    fn log(&self, line: &str) {
        log_line(&self.app, line);
    }
    fn is_current(&self) -> bool {
        self.app
            .state::<AppState>()
            .correction_watch_gen
            .load(std::sync::atomic::Ordering::SeqCst)
            == self.gen
    }
}

/// The watch itself: poll the focused field until the user corrects a word,
/// something makes the span unfollowable, or the window runs out. Returns
/// the outcome for the log.
///
/// Owns the indicator for its whole life. It is lit before the *first* read,
/// not after `Watch::start` succeeds, or that first read (and both of the
/// early-return cases below) would happen with the indicator dark. Chromium
/// targets (Chrome, VS Code) used to hit "field not readable" on essentially
/// every dictation, because `ax_read` reached for the focused element through
/// the system-wide AX element, which frequently just doesn't answer; it now
/// goes via the frontmost application instead. So "field not readable" is no
/// longer an expected steady-state outcome — it means the field really is
/// unreadable, and the accompanying diagnostic log line says why. It is
/// cleared on the way out unless a newer watcher has taken over, in which case
/// that watcher owns the dot now.
fn run_correction_watch(
    reader: &dyn platform::FocusReader,
    pasted: &str,
    host: &dyn WatchHost,
) -> &'static str {
    // Let the target app finish applying the paste before reading it.
    host.wait(WATCH_POLL);
    if !host.is_current() {
        return "superseded";
    }
    host.indicator(true);

    let outcome = watch_span(reader, pasted, host);
    if host.is_current() {
        host.indicator(false);
    }
    outcome
}

fn watch_span(
    reader: &dyn platform::FocusReader,
    pasted: &str,
    host: &dyn WatchHost,
) -> &'static str {
    // Every exit reason is a fixed string; the single "watch ended" line the
    // caller writes carries it, so nothing here needs its own log call.
    let Some(first) = reader.read() else {
        // The one place worth a log line of its own: this is where the feature
        // has silently died for every Chromium target across two releases, and
        // "field not readable" alone never said why. Names and error codes
        // only — no field contents.
        let why = reader.diagnose();
        if !why.is_empty() {
            host.log(&format!("learning: focus unreadable — {why}"));
        }
        return "field not readable";
    };
    let mut watch = match corrections::Watch::start(&first, pasted) {
        Ok(w) => w,
        Err(why) => return why,
    };

    // The watch streams: a stable correction set is committed as soon as it
    // holds for two polls, and if the user keeps refining the same edit (or
    // reverts it), the next stable set retracts the previous commit and
    // replaces it. So an early commit of a half-typed word heals itself, and
    // the watch keeps following until the field goes quiet for the window.
    let mut committed: Vec<corrections::Correction> = Vec::new();
    let mut since_commit = 0usize;
    for _ in 0..WATCH_MAX_POLLS {
        // Once a correction is committed and the field has gone quiet, stop
        // early: the edit settled, and reading another application's field
        // any longer than needed is exposure without purpose.
        if !committed.is_empty() {
            since_commit += 1;
            if since_commit > WATCH_SETTLED_POLLS {
                return "learned";
            }
        }
        host.wait(WATCH_POLL);
        // Checked before the read, not just before an emission: a superseded
        // watcher must stop reading other applications' fields, not merely
        // stop talking about it. This is the design's documented "next
        // dictation starts" early stop.
        if !host.is_current() {
            return "superseded";
        }
        let Some(now) = reader.read() else {
            return "field became unreadable";
        };
        match watch.step(&now) {
            corrections::Step::Continue => continue,
            corrections::Step::Abandon(why) => return why,
            corrections::Step::Learned(cs) => {
                host.relearn(&committed, &cs);
                committed = cs;
                since_commit = 0;
            }
        }
    }
    if committed.is_empty() {
        "timed out"
    } else {
        "learned"
    }
}

/// Watch the field we just pasted into for a hand-made word correction.
///
/// Bookkeeping, like history and stats: it starts after the paste keystroke
/// and can find nothing, fail, or time out without affecting dictation.
/// No log line here may carry text read out of another application.
fn spawn_correction_watch(app: AppHandle, pasted: String) {
    // Supersede any watcher already in flight. Only the watcher holding the
    // newest generation keeps reading and keeps the right to touch
    // `watching-edits`: otherwise an older watcher would go on reading
    // whatever now has focus, and could flip the indicator to false while
    // the newer watcher is still live.
    let gen = app
        .state::<AppState>()
        .correction_watch_gen
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;

    std::thread::spawn(move || {
        let host = AppWatchHost { app, gen };
        let reader = platform::focus_reader();
        let outcome = run_correction_watch(reader.as_ref(), &pasted, &host);
        host.log(&format!("learning: watch ended ({outcome})"));
    });
}

/// Route a global shortcut to its owner: the dictation hotkey keeps its
/// push-to-talk semantics; a transform hotkey fires its transform on press.
fn route_hotkey(app: &AppHandle, pressed: &Shortcut, shortcut_state: ShortcutState) {
    let state = app.state::<AppState>();
    let dictation = state.settings.lock().unwrap().hotkey.clone();
    if dictation.parse::<Shortcut>().is_ok_and(|s| s == *pressed) {
        on_hotkey(app, shortcut_state);
        return;
    }
    if shortcut_state != ShortcutState::Pressed {
        return; // transforms trigger on press; releases are noise
    }
    let transform = {
        let list = state.transforms.lock().unwrap();
        list.iter()
            .find(|t| {
                t.hotkey
                    .as_deref()
                    .is_some_and(|h| h.parse::<Shortcut>().is_ok_and(|s| s == *pressed))
            })
            .cloned()
    };
    if let Some(t) = transform {
        run_transform(app, t);
    }
}

/// Hotkey semantics: pure push-to-talk. Press starts recording, release
/// stops and transcribes. Releases under MIN_HOLD are discarded as
/// accidental taps. Key-repeat Pressed events while held are ignored.
fn on_hotkey(app: &AppHandle, shortcut_state: ShortcutState) {
    let state = app.state::<AppState>();
    match shortcut_state {
        ShortcutState::Pressed => {
            {
                let mut press = state.hotkey_press.lock().unwrap();
                if press.is_down {
                    return; // key repeat
                }
                press.is_down = true;
                press.pressed_at = Some(std::time::Instant::now());
            }
            start_dictation(app);
        }
        ShortcutState::Released => {
            let held = {
                let mut press = state.hotkey_press.lock().unwrap();
                press.is_down = false;
                press.pressed_at.take().map(|t| t.elapsed())
            };
            if state.recorder.lock().unwrap().is_none() {
                return; // nothing recording (e.g. start failed)
            }
            if held.is_some_and(|d| d < MIN_HOLD) {
                log_line(
                    app,
                    "hotkey: tap shorter than MIN_HOLD — recording discarded",
                );
                cancel_dictation_impl(app);
            } else {
                stop_and_transcribe(app);
            }
        }
    }
}

/// Begin recording (hotkey press, tray, or in-app button).
fn start_dictation(app: &AppHandle) {
    let state = app.state::<AppState>();

    // Ignore starts while a previous run is still transcribing.
    if state.processing.load(Ordering::SeqCst) {
        notify(app, "Still processing the previous dictation — one moment.");
        return;
    }

    let mut recorder_slot = state.recorder.lock().unwrap();
    if recorder_slot.is_some() {
        return;
    }
    match Recorder::start() {
        Ok(rec) => {
            let level = rec.level_handle();
            *recorder_slot = Some(rec);
            drop(recorder_slot);
            *state.recording_started.lock().unwrap() = Some(std::time::Instant::now());
            set_state(app, "recording");
            spawn_level_emitter(app.clone(), level);
            // Reloads the speech model if it was dropped for idleness, and
            // starts the Smart server if it will be needed; both load while
            // they talk.
            let warm_app = app.clone();
            let smart = state.settings.lock().unwrap().cleanup_mode == CleanupMode::Smart;
            std::thread::spawn(move || {
                warm_speech(&warm_app.state::<AppState>());
                if smart {
                    llm(&warm_app).warm();
                }
            });
        }
        Err(e) => {
            set_state(app, "idle");
            notify(app, &format!("Couldn't start recording: {e}"));
        }
    }
}

/// Stop recording and run the transcribe→clean→paste pipeline.
fn stop_and_transcribe(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recorder = match state.recorder.lock().unwrap().take() {
        Some(r) => r,
        None => return,
    };
    let Some(session) = try_begin_session(&state.processing) else {
        // A transform got in first. Starting a second pipeline would race
        // it for the clipboard; drop this recording instead.
        recorder.cancel();
        log_line(app, "dictation discarded — another session is processing");
        set_state(app, "processing");
        notify(
            app,
            "Purl was busy with a transform, so that dictation was discarded. Try again in a moment.",
        );
        return;
    };
    // Hides the overlay too — if the user clicked the pill's ✓, focus must
    // return to their target app before the paste keystroke fires.
    set_state(app, "processing");

    let record_ms = state
        .recording_started
        .lock()
        .unwrap()
        .take()
        .map(|t| t.elapsed().as_millis() as u64)
        .unwrap_or(0);

    let app = app.clone();
    std::thread::spawn(move || {
        let result = run_pipeline(&app, recorder, record_ms);
        drop(session);
        set_state(&app, pipeline_end_state(&result));
        if let Err(e) = result {
            log_line(&app, &format!("pipeline ERROR: {e}"));
            let _ = app.emit("pipeline-error", e.clone());
            notify(&app, &e);
        }
    });
}

/// The `state` event that closes a dictation. The overlay flashes "Sent" or
/// "Didn't catch that" for these and then settles back to idle by itself.
fn pipeline_end_state(result: &Result<(), String>) -> &'static str {
    match result {
        Ok(()) => "done",
        Err(_) => "error",
    }
}

/// Discard the in-flight recording without transcribing.
fn cancel_dictation_impl(app: &AppHandle) {
    app.state::<AppState>()
        .recording_started
        .lock()
        .unwrap()
        .take();
    let state = app.state::<AppState>();
    if let Some(rec) = state.recorder.lock().unwrap().take() {
        rec.cancel();
        log_line(app, "dictation cancelled — audio discarded");
    }
    set_state(app, "idle");
}

/// Tray / in-app toggle: start when idle, stop + transcribe when recording.
fn toggle_dictation(app: &AppHandle) {
    let recording = app.state::<AppState>().recorder.lock().unwrap().is_some();
    if recording {
        stop_and_transcribe(app);
    } else {
        start_dictation(app);
    }
}

/// Kick off a transform on the frontmost app's selection. Shares the
/// `processing` guard with dictation so the two can never overlap.
fn run_transform(app: &AppHandle, t: transforms::Transform) {
    let state = app.state::<AppState>();
    let session = if state.recorder.lock().unwrap().is_some() {
        None
    } else {
        try_begin_session(&state.processing)
    };
    let Some(session) = session else {
        notify(
            app,
            "Busy with another dictation or transform — one moment.",
        );
        return;
    };
    if !platform::injector().can_inject().can_paste() {
        drop(session);
        notify(
            app,
            "Transforms need the Accessibility permission to read your selection (see the app window).",
        );
        platform::injector().request_trust();
        return;
    }
    set_state(app, "processing");
    let app = app.clone();
    std::thread::spawn(move || {
        let result = run_transform_pipeline(&app, &t);
        drop(session);
        set_state(&app, "idle");
        match result {
            Ok(()) => {
                let _ = app.emit("transform-result", &t.name);
            }
            Err(e) => {
                log_line(&app, &format!("transform '{}' ERROR: {e}", t.name));
                notify(&app, &e);
            }
        }
    });
}

/// Copy selection → LLM rewrite → paste back, restoring the user's original
/// clipboard on every path. Runs off the main thread.
fn run_transform_pipeline(app: &AppHandle, t: &transforms::Transform) -> Result<(), String> {
    let t0 = std::time::Instant::now();

    let (selection, previous) =
        inject::capture_selection().map_err(|e| format!("Couldn't read the selection: {e}"))?;
    let Some(text) = selection else {
        return Err("Select some text first, then press the transform hotkey.".into());
    };

    let attempt = transforms::run(&*llm(app), &text, &t.prompt);
    let out = match attempt {
        Ok(o) => o,
        Err(e) => {
            inject::restore_clipboard(&previous);
            return Err(format!("{} failed: {e}. Your text is unchanged.", t.name));
        }
    };

    // The selection is still highlighted in the target app, so the paste
    // replaces it in place. If the paste fails, `inject` leaves the result on
    // the clipboard; keep it there rather than restoring the old clipboard.
    if let Err(e) = inject::inject(&out) {
        log_line(
            app,
            &format!("transform paste FAILED: {e} -> clipboard fallback"),
        );
        return Err(format!(
            "Couldn't paste the {} result, so it's on your clipboard — paste with {}.",
            t.name,
            platform::PASTE_SHORTCUT
        ));
    }
    inject::restore_clipboard(&previous);

    log_line(
        app,
        &format!(
            "transform '{}' done in {}ms ({} -> {} chars)",
            t.name,
            t0.elapsed().as_millis(),
            text.len(),
            out.len()
        ),
    );
    Ok(())
}

/// How a dictation reached the user in paste mode.
#[derive(Debug, PartialEq)]
enum Delivery {
    /// Every text segment pasted and every command key sent.
    Sent,
    /// A paste or key failed (typically Windows UIPI: the target runs as
    /// administrator), so the whole dictation went to the clipboard instead.
    /// Holds the failure, for the log.
    Copied(String),
}

/// Paste text segments and send command keys ("go down" → ↓) in order, with
/// a short pause between them so the target app keeps up. On the first
/// failure, stop and put the whole dictation (`full`) on the clipboard: the
/// user gets all of it to paste by hand rather than half of it.
fn deliver_segments(
    segments: &[voice_commands::Segment],
    full: &str,
    paste: &mut dyn FnMut(&str) -> Result<(), inject::InjectError>,
    key: &mut dyn FnMut(voice_commands::Key) -> Result<(), inject::InjectError>,
    copy: &mut dyn FnMut(&str) -> Result<(), inject::InjectError>,
) -> Result<Delivery, String> {
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let sent = match seg {
            voice_commands::Segment::Text(t) => paste(t),
            voice_commands::Segment::Key(k) => key(*k),
        };
        if let Err(e) = sent {
            copy(full).map_err(|e| format!("Clipboard failed: {e}"))?;
            return Ok(Delivery::Copied(e.to_string()));
        }
    }
    Ok(Delivery::Sent)
}

/// stop recording → transcribe → clean → inject. Runs off the main thread.
fn run_pipeline(app: &AppHandle, recorder: Recorder, record_ms: u64) -> Result<(), String> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let dictionary = load_dictionary(app);
    let support = app_support_dir(app);

    let rules = if settings.learn_corrections {
        learned::load(&learned_path(app))
    } else {
        Vec::new()
    };

    let pcm = recorder
        .stop_samples()
        .map_err(|e| format!("Recording failed: {e}"))?;
    if pcm.len() < 16000 * 3 / 10 {
        return Err("Didn't catch that — hold the key a little longer.".into());
    }

    let t0 = std::time::Instant::now();
    let stt = app
        .state::<AppState>()
        .stt
        .get()
        .cloned()
        .ok_or("The speech engine is still starting. Try again in a moment.")?;
    let raw = stt.transcribe(&pcm).map_err(|e| e.to_string())?;
    let transcribe_ms = t0.elapsed().as_millis();

    if raw.trim().is_empty() {
        return Err("Heard nothing — try again closer to the mic.".into());
    }

    let t1 = std::time::Instant::now();
    let cleaned = match settings.cleanup_mode {
        CleanupMode::Off => cleanup::CleanResult {
            text: raw.clone(),
            used_llm: false,
            note: None,
        },
        CleanupMode::Fast | CleanupMode::LegacyAi => cleanup::CleanResult {
            text: fastclean::clean(&raw, &dictionary),
            used_llm: false,
            note: None,
        },
        CleanupMode::Smart => {
            // Learned corrections bias the model too; the prompt treats
            // them as data (they were read out of other apps' fields).
            let terms = dictionary
                .with_extra_terms(&learned::glossary_terms(&rules))
                .llm_terms();
            cleanup::smart_clean(
                &*llm(app),
                &raw,
                &terms,
                settings.structure_mode,
                smart_budget(&settings.smart_model),
                || fastclean::clean(&raw, &dictionary),
            )
        }
    };
    let cleanup_ms = t1.elapsed().as_millis();

    // Learned corrections land before snippets so a rule can never rewrite
    // the inside of an expansion (an email address, a URL), and after
    // cleanup so the model cannot undo a fix the user made by hand.
    let (corrected, fired_rules) = learned::apply(&cleaned.text, &rules);
    let expanded = snippets::expand(&corrected, &snippets::load(&support.join("snippets.json")));

    // Writing mode reshapes layout (e.g. email paragraphs); command parsing
    // then splits spoken commands ("go down") out of the text.
    let structured = structure::apply(&expanded, settings.structure_mode);
    let segments = if settings.command_detection {
        voice_commands::parse(&structured)
    } else {
        vec![voice_commands::Segment::Text(structured.clone())]
    };
    // What the user sees in history/Home: the text minus command phrases.
    let expanded = voice_commands::text_only(&segments);

    let payload = TranscriptPayload {
        raw: raw.clone(),
        cleaned: expanded.clone(),
        used_llm: cleaned.used_llm,
        note: cleaned.note.clone(),
        transcribe_ms,
        cleanup_ms,
    };
    *app.state::<AppState>().last_result.lock().unwrap() = Some(payload.clone());
    let _ = app.emit("result", &payload);

    let paste_trust = platform::injector().can_inject();

    // Pasting while our own window is frontmost would dump the text into
    // Purl itself (e.g. dictation stopped via the in-app button).
    let own_window_focused = app
        .get_webview_window("main")
        .map(|w| w.is_focused().unwrap_or(false))
        .unwrap_or(false);

    log_line(
        app,
        &format!(
            "pipeline done: transcribe={transcribe_ms}ms cleanup={cleanup_ms}ms used_llm={} \
             inject_mode={:?} own_window_focused={own_window_focused} paste_trust={:?}",
            cleaned.used_llm, settings.inject_mode, paste_trust
        ),
    );

    let has_key_commands = voice_commands::has_keys(&segments);
    // Set when a synthesized paste or key fails mid-way and the text went to
    // the clipboard instead.
    let mut paste_failed = false;
    match settings.inject_mode {
        InjectMode::Paste => {
            if own_window_focused {
                log_line(app, "inject: own window focused -> clipboard only");
                inject::copy_to_clipboard(&expanded)
                    .map_err(|e| format!("Clipboard failed: {e}"))?;
                notify(
                    app,
                    "Copied to clipboard (Purl was focused). Use the hotkey from your target app to paste automatically.",
                );
            } else if paste_trust.can_paste() {
                let mut paste = |t: &str| {
                    log_line(app, "inject: pasting via Cmd-V");
                    inject::inject(t)
                };
                let mut key = |k: voice_commands::Key| {
                    log_line(app, &format!("inject: voice command key {k:?}"));
                    platform::injector().send_key(k)
                };
                match deliver_segments(
                    &segments,
                    &expanded,
                    &mut paste,
                    &mut key,
                    &mut inject::copy_to_clipboard,
                )? {
                    Delivery::Sent => log_line(app, "inject: all segments sent OK"),
                    Delivery::Copied(e) => {
                        log_line(app, &format!("inject: FAILED: {e} -> clipboard fallback"));
                        notify(
                            app,
                            &format!(
                                "Copied to clipboard — paste with {}.",
                                platform::PASTE_SHORTCUT
                            ),
                        );
                        paste_failed = true;
                    }
                }
            } else {
                log_line(
                    app,
                    "inject: NOT accessibility trusted -> clipboard fallback",
                );
                inject::copy_to_clipboard(&expanded)
                    .map_err(|e| format!("Clipboard failed: {e}"))?;
                notify(
                    app,
                    "Copied to clipboard. Grant Accessibility permission to auto-paste (see the app window).",
                );
            }
        }
        InjectMode::ClipboardOnly => {
            log_line(app, "inject: clipboard-only mode");
            inject::copy_to_clipboard(&expanded).map_err(|e| format!("Clipboard failed: {e}"))?;
            if has_key_commands {
                notify(
                    app,
                    "Transcript copied. Cursor commands were skipped — they need \"Paste at cursor\" output mode.",
                );
            } else {
                notify(app, "Transcript copied to clipboard.");
            }
        }
    }

    // History + stats bookkeeping must never block dictation — it happens
    // after injection so the paste keystroke isn't delayed behind disk I/O.
    let words = expanded.split_whitespace().count() as u64;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = history::HistoryEntry {
        ts,
        raw: raw.clone(),
        cleaned: expanded.clone(),
        used_llm: cleaned.used_llm,
        record_ms,
        transcribe_ms: transcribe_ms as u64,
        cleanup_ms: cleanup_ms as u64,
        words,
    };
    if let Err(e) = history::append(&support.join("history.jsonl"), &entry) {
        log_line(app, &format!("history append failed: {e}"));
    }
    if let Err(e) = learned::record_hits(&learned_path(app), &fired_rules, ts) {
        log_line(app, &format!("learned hits update failed: {e}"));
    }
    // `paste_trust` is part of the gate, not an implementation
    // detail: without it the paste fell back to the clipboard, so there is
    // nothing in any field to watch. Spawning anyway would light the
    // disclosure dot, read nothing, and clear it — training the user to
    // ignore the one live signal this feature gives them.
    //
    // `expanded` is `text_only(segments)` — every text segment joined —
    // while injection pastes each segment separately with keystrokes in
    // between. `Watch::start` locates the span with whitespace-normalized
    // matching, so a dictation whose command keys only produced line breaks
    // ("new line") still watches; commands that moved the caret elsewhere
    // leave the segments in different places, the joined string can't be
    // found, and the watch is refused — the safe outcome.
    if settings.learn_corrections
        && matches!(settings.inject_mode, InjectMode::Paste)
        && !own_window_focused
        && paste_trust.can_paste()
        && !paste_failed
    {
        spawn_correction_watch(app.clone(), expanded.clone());
    }
    let stats_path = support.join("stats.json");
    let mut stats = stats::Stats::load(&stats_path);
    stats.record(words, record_ms, cleaned.used_llm, &stats::today());
    if let Err(e) = stats.save(&stats_path) {
        log_line(app, &format!("stats save failed: {e}"));
    }

    Ok(())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, new_settings: Settings) -> Result<(), String> {
    // Same coercion as `load`, so a stale UI can't persist a value this
    // version no longer runs.
    let new_settings = new_settings.sanitized();
    let path = app_support_dir(&app).join("settings.json");
    new_settings.save(&path).map_err(|e| e.to_string())?;

    let state = app.state::<AppState>();
    let old_hotkey = state.settings.lock().unwrap().hotkey.clone();
    let new_hotkey = new_settings.hotkey.clone();
    let smart_model = new_settings.smart_model.clone();
    let smart_on = new_settings.cleanup_mode == CleanupMode::Smart;
    *state.settings.lock().unwrap() = new_settings;
    // A different Smart model replaces the running server.
    if let Some(path) = engine::llm::model_path(&app_support_dir(&app), &smart_model) {
        llm(&app).set_model(path);
    }
    if smart_on {
        llm(&app).warm();
    }

    if old_hotkey != new_hotkey {
        let gs = app.global_shortcut();
        let _ = gs.unregister(old_hotkey.as_str());
        gs.register(new_hotkey.as_str())
            .map_err(|e| format!("Couldn't register hotkey {new_hotkey}: {e}"))?;
    }
    Ok(())
}

/// Outcome of a failed shortcut swap: the new combo was rejected, and the
/// old combo either survived (rolled back) or could not be re-registered.
enum SwapError {
    NewRejected(String),
    /// Neither combo is registered — the caller must tell the user.
    NoneActive {
        new_err: String,
        old_err: String,
    },
}

/// Swap the registered global shortcut from `old` to `new`. If `new` can't
/// be registered (owned by another app / reserved by macOS), re-register
/// `old` so dictation never ends up with no working hotkey.
fn swap_shortcut(
    register: &mut dyn FnMut(&str) -> Result<(), String>,
    unregister: &mut dyn FnMut(&str),
    old: &str,
    new: &str,
) -> Result<(), SwapError> {
    if old == new {
        // The combo may currently be suspended (recorder capture) — make
        // "registered" the postcondition rather than assuming it still is.
        unregister(old);
        return register(new).map_err(|e| SwapError::NoneActive {
            new_err: e.clone(),
            old_err: e,
        });
    }
    unregister(old);
    if let Err(new_err) = register(new) {
        if let Err(old_err) = register(old) {
            return Err(SwapError::NoneActive { new_err, old_err });
        }
        return Err(SwapError::NewRejected(new_err));
    }
    Ok(())
}

#[tauri::command]
fn set_hotkey(app: AppHandle, new_hotkey: String) -> Result<(), String> {
    let new_hotkey = new_hotkey.trim().to_string();
    if new_hotkey.is_empty() {
        return Err("Empty hotkey".into());
    }
    let state = app.state::<AppState>();
    let old = state.settings.lock().unwrap().hotkey.clone();

    let taken_by_transform = state
        .transforms
        .lock()
        .unwrap()
        .iter()
        .any(|t| t.hotkey.as_deref() == Some(new_hotkey.as_str()));
    if taken_by_transform {
        return Err("That combo is used by one of your Transforms — pick a different one.".into());
    }

    let gs = app.global_shortcut();
    let mut register = |hk: &str| gs.register(hk).map_err(|e| e.to_string());
    let mut unregister = |hk: &str| {
        let _ = gs.unregister(hk);
    };
    swap_shortcut(&mut register, &mut unregister, &old, &new_hotkey).map_err(|e| match e {
        SwapError::NewRejected(err) => {
            log_line(&app, &format!("set_hotkey: {new_hotkey} rejected: {err}"));
            "That combo couldn't be registered — it may be taken by another app or reserved by macOS.".to_string()
        }
        SwapError::NoneActive { new_err, old_err } => {
            log_line(&app, &format!(
                "set_hotkey: {new_hotkey} rejected ({new_err}) AND old {old} failed to restore ({old_err})"
            ));
            "That combo couldn't be registered, and restoring the old one failed — please set a hotkey now.".to_string()
        }
    })?;

    let mut settings = state.settings.lock().unwrap().clone();
    settings.hotkey = new_hotkey.clone();
    let path = app_support_dir(&app).join("settings.json");
    if let Err(e) = settings.save(&path) {
        // Keep OS registration consistent with the settings we're keeping.
        match swap_shortcut(&mut register, &mut unregister, &new_hotkey, &old) {
            Ok(()) => log_line(
                &app,
                &format!("set_hotkey: save failed, reverted to {old}: {e}"),
            ),
            Err(_) => log_line(
                &app,
                &format!("set_hotkey: save failed AND revert to {old} failed: {e}"),
            ),
        }
        return Err(format!("Couldn't save the new hotkey: {e}"));
    }
    *state.settings.lock().unwrap() = settings;
    log_line(&app, &format!("set_hotkey: now {new_hotkey}"));
    Ok(())
}

/// All accelerators the app owns: the dictation hotkey plus every assigned
/// transform hotkey.
fn all_hotkeys(app: &AppHandle) -> Vec<String> {
    let state = app.state::<AppState>();
    let mut keys = vec![state.settings.lock().unwrap().hotkey.clone()];
    keys.extend(
        state
            .transforms
            .lock()
            .unwrap()
            .iter()
            .filter_map(|t| t.hotkey.clone()),
    );
    keys
}

/// Capture mode: pressing the current dictation hotkey — or any transform
/// hotkey — while the recorder is armed must not fire its action.
#[tauri::command]
fn suspend_hotkey(app: AppHandle) {
    let gs = app.global_shortcut();
    for hk in all_hotkeys(&app) {
        let _ = gs.unregister(hk.as_str());
    }
    log_line(&app, "hotkeys suspended for capture");
}

/// Idempotent: unregister-then-register so calling it when already active
/// is safe.
#[tauri::command]
fn resume_hotkey(app: AppHandle) {
    let gs = app.global_shortcut();
    for hk in all_hotkeys(&app) {
        let _ = gs.unregister(hk.as_str());
        if let Err(e) = gs.register(hk.as_str()) {
            log_line(&app, &format!("resume_hotkey: {hk} failed: {e}"));
        }
    }
}

fn transforms_path(app: &AppHandle) -> PathBuf {
    app_support_dir(app).join("transforms.json")
}

#[tauri::command]
fn get_transforms(state: tauri::State<AppState>) -> Vec<transforms::Transform> {
    state.transforms.lock().unwrap().clone()
}

#[tauri::command]
fn save_transforms(app: AppHandle, list: Vec<transforms::Transform>) -> Result<(), String> {
    apply_transforms(&app, list)
}

#[tauri::command]
fn reset_transforms(app: AppHandle) -> Result<Vec<transforms::Transform>, String> {
    let current = app.state::<AppState>().transforms.lock().unwrap().clone();
    let reset = transforms::reset_builtins(current);
    apply_transforms(&app, reset.clone())?;
    Ok(reset)
}

/// Validate, persist, and swap hotkey registrations for a new transform
/// list. Registration failures don't roll back the save — the transform
/// still works from a future hotkey; the error tells the user which combos
/// to rebind.
fn apply_transforms(app: &AppHandle, list: Vec<transforms::Transform>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let dictation = state.settings.lock().unwrap().hotkey.clone();
    transforms::validate(&list, &dictation)?;
    transforms::save(&transforms_path(app), &list)
        .map_err(|e| format!("Couldn't save transforms: {e}"))?;

    let gs = app.global_shortcut();
    let old = state.transforms.lock().unwrap().clone();
    for hk in old.iter().filter_map(|t| t.hotkey.as_deref()) {
        let _ = gs.unregister(hk);
    }
    let mut failures = Vec::new();
    for t in &list {
        if let Some(hk) = t.hotkey.as_deref() {
            if let Err(e) = gs.register(hk) {
                log_line(
                    app,
                    &format!("transform hotkey {hk} failed to register: {e}"),
                );
                failures.push(format!("{} ({hk})", t.name));
            }
        }
    }
    *state.transforms.lock().unwrap() = list;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Saved, but these hotkeys couldn't be registered: {}. The combo may be taken by another app — pick a different one.",
            failures.join(", ")
        ))
    }
}

#[tauri::command]
fn get_status(app: AppHandle, state: tauri::State<AppState>) -> StatusPayload {
    let settings = state.settings.lock().unwrap().clone();
    let app_state = if state.recorder.lock().unwrap().is_some() {
        "recording"
    } else if state.processing.load(Ordering::SeqCst) {
        "processing"
    } else {
        "idle"
    };
    status_payload_for(
        stt_status(&app),
        platform::injector().can_inject(),
        &settings.hotkey,
        app_state,
    )
}

/// Microphone permission: `granted`, `denied`, or `not_required` (macOS
/// hasn't asked yet; it will on the first dictation).
#[tauri::command]
fn mic_status() -> platform::Trust {
    platform::mic_status()
}

#[tauri::command]
fn get_last_result(state: tauri::State<AppState>) -> Option<TranscriptPayload> {
    state.last_result.lock().unwrap().clone()
}

#[tauri::command]
fn toggle_recording(app: AppHandle) {
    toggle_dictation(&app);
}

#[tauri::command]
fn cancel_dictation(app: AppHandle) {
    cancel_dictation_impl(&app);
}

/// The overlay reports the bounds of whatever it currently draws that should
/// accept clicks — [x, y, width, height] in logical points relative to its own
/// window — or null when nothing in it is interactive. Everything outside that
/// box stays click-through. See `spawn_overlay_click_through_watch`.
#[tauri::command]
fn set_overlay_hit_rect(app: AppHandle, rect: Option<[f64; 4]>) {
    *app.state::<AppState>().overlay_hit.lock().unwrap() = rect;
}

#[tauri::command]
fn get_glossary(app: AppHandle) -> String {
    dictionary::read_text(&dictionary_path(&app))
}

#[tauri::command]
fn save_glossary(app: AppHandle, content: String) -> Result<(), String> {
    let dir = app_support_dir(&app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dictionary_path(&app), content).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_history(app: AppHandle) -> Vec<history::HistoryEntry> {
    history::load(&app_support_dir(&app).join("history.jsonl"))
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<(), String> {
    history::clear(&app_support_dir(&app).join("history.jsonl")).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_stats(app: AppHandle) -> stats::Stats {
    stats::Stats::load(&app_support_dir(&app).join("stats.json"))
}

#[tauri::command]
fn get_snippets(app: AppHandle) -> Vec<snippets::Snippet> {
    snippets::load(&app_support_dir(&app).join("snippets.json"))
}

#[tauri::command]
fn save_snippets(app: AppHandle, list: Vec<snippets::Snippet>) -> Result<(), String> {
    snippets::save(&app_support_dir(&app).join("snippets.json"), &list).map_err(|e| e.to_string())
}

/// A rule plus what the store alone can't tell the UI: whether it is live
/// yet, or still waiting for a second sighting.
#[derive(serde::Serialize)]
struct LearnedRuleView {
    #[serde(flatten)]
    rule: learned::Rule,
    active: bool,
}

#[tauri::command]
fn get_learned_rules(app: AppHandle) -> Vec<LearnedRuleView> {
    learned::load(&learned_path(&app))
        .into_iter()
        .map(|r| LearnedRuleView {
            active: learned::eligible(&r),
            rule: r,
        })
        .collect()
}

#[tauri::command]
fn delete_learned_rule(
    app: AppHandle,
    from: String,
    to: String,
    before: String,
) -> Result<(), String> {
    learned::delete(&learned_path(&app), &from, &to, &before).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_learned_rules(app: AppHandle) -> Result<(), String> {
    learned::clear(&learned_path(&app)).map_err(|e| e.to_string())
}

// ---- Models: registry status and downloads ----

#[derive(Clone, Serialize)]
struct ModelProgressEvent {
    id: &'static str,
    downloaded_bytes: u64,
    size_bytes: u64,
}

#[derive(Clone, Serialize)]
struct ModelDoneEvent {
    id: &'static str,
    ok: bool,
    error: Option<String>,
    /// `DownloadError::kind`; None when ok.
    kind: Option<&'static str>,
}

fn model_done(
    id: &'static str,
    result: &Result<(), engine::download::DownloadError>,
) -> ModelDoneEvent {
    let failure = result.as_ref().err();
    ModelDoneEvent {
        id,
        ok: failure.is_none(),
        error: failure.map(|e| e.to_string()),
        kind: failure.map(|e| e.kind()),
    }
}

#[tauri::command]
fn models_status(app: AppHandle, state: tauri::State<AppState>) -> Vec<engine::ModelView> {
    state.downloads.views(&app_support_dir(&app))
}

/// Starts the download on its own thread and returns at once; progress and
/// the outcome arrive as `model-progress` / `model-done` events. A second
/// call while that model is already downloading is a no-op.
#[tauri::command]
fn download_model(app: AppHandle, state: tauri::State<AppState>, id: String) -> Result<(), String> {
    let spec = engine::models::find(&id).ok_or_else(|| format!("Unknown model {id}"))?;
    let Some(cancel) = state.downloads.try_start(spec.id) else {
        return Ok(());
    };
    let app_data = app_support_dir(&app);
    std::thread::spawn(move || {
        let result = engine::download::download_model(spec, &app_data, &cancel, &mut |p| {
            let _ = app.emit(
                "model-progress",
                ModelProgressEvent {
                    id: spec.id,
                    downloaded_bytes: p.downloaded,
                    size_bytes: p.total,
                },
            );
        });
        let done = model_done(spec.id, &result);
        log_line(
            &app,
            &format!(
                "model {}: download {}",
                spec.id,
                done.error.as_deref().unwrap_or("ok")
            ),
        );
        app.state::<AppState>().downloads.finish(spec.id, &result);
        if result.is_ok() && spec.id == engine::stt::MODEL_ID {
            warm_speech(&app.state::<AppState>());
        }
        if spec.kind == engine::models::Kind::Llm {
            let _ = app.emit("engine-status", engine_status_view(&app));
            let smart_on = app
                .state::<AppState>()
                .settings
                .lock()
                .unwrap()
                .cleanup_mode
                == CleanupMode::Smart;
            if result.is_ok() && smart_on {
                llm(&app).warm();
            }
        }
        let _ = app.emit("model-done", done);
    });
    Ok(())
}

#[tauri::command]
fn cancel_download(state: tauri::State<AppState>, id: String) {
    state.downloads.cancel(&id);
}

/// Removes a model's folder, `.part` files included. Refused while the model
/// is downloading, or while its engine is loading or using it.
#[tauri::command]
fn delete_model(app: AppHandle, state: tauri::State<AppState>, id: String) -> Result<(), String> {
    let spec = engine::models::find(&id).ok_or_else(|| format!("Unknown model {id}"))?;
    if state.downloads.is_running(spec.id) {
        return Err("Cancel the download first".into());
    }
    if spec.id == engine::stt::MODEL_ID {
        release_speech_model(&state)?;
    }
    let smart_model = state.settings.lock().unwrap().smart_model.clone();
    if spec.id == smart_model && !llm(&app).release() {
        return Err("Stop using this model first".into());
    }
    let dir = engine::models::model_dir(&app_support_dir(&app), spec.id);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => {}
    }
    state.downloads.clear_error(spec.id);
    if spec.kind == engine::models::Kind::Llm {
        let _ = app.emit("engine-status", engine_status_view(&app));
    }
    Ok(())
}

#[tauri::command]
fn engine_status(app: AppHandle) -> engine::EngineStatusView {
    engine_status_view(&app)
}

fn engine_status_view(app: &AppHandle) -> engine::EngineStatusView {
    engine::EngineStatusView {
        stt: stt_status(app),
        llm: llm_status(app),
    }
}

/// Safe before `setup` has built the engine; reports Missing until then.
fn llm_status(app: &AppHandle) -> engine::EngineState {
    app.state::<AppState>()
        .llm
        .get()
        .map_or(engine::EngineState::Missing, |e| e.status())
}

fn llm(app: &AppHandle) -> Arc<LlamaServerEngine> {
    Arc::clone(
        app.state::<AppState>()
            .llm
            .get()
            .expect("smart engine is set in setup"),
    )
}

/// The hard deadline for Smart cleanup: the larger model gets longer. On
/// timeout the Fast result is pasted.
fn smart_budget(smart_model: &str) -> std::time::Duration {
    if smart_model == "qwen3.5-4b" {
        std::time::Duration::from_secs(10)
    } else {
        std::time::Duration::from_secs(6)
    }
}

/// Build the Smart engine on the bundled llama-server, reap a server a
/// crashed Purl left behind, broadcast status changes, and stop the server
/// after 5 idle minutes. With Smart on, the server starts now: the first
/// start after an install or update takes 15-20 s on macOS (first-run
/// checks of the new binary), longer than one dictation's budget.
fn start_llm(app: &AppHandle) {
    let app_data = app_support_dir(app);
    let smart_model = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .smart_model
        .clone();
    let model = engine::llm::model_path(&app_data, &smart_model)
        .or_else(|| engine::llm::model_path(&app_data, settings::DEFAULT_SMART_MODEL))
        .expect("the default Smart model is in the registry");
    let bin = engine::sidecar::llama_server_path(&app.path().resource_dir().unwrap_or_default());
    // Windows ties the server to a Job Object instead of a PID file.
    let pid_file = cfg!(unix).then(|| app_data.join("llama.pid"));
    if let Some(pid_file) = &pid_file {
        engine::sidecar::reap_stale(pid_file);
    }
    let engine = LlamaServerEngine::new(bin, model, pid_file);
    let emit_app = app.clone();
    engine.set_listener(Box::new(move || {
        let _ = emit_app.emit("engine-status", engine_status_view(&emit_app));
    }));
    let _ = app.state::<AppState>().llm.set(Arc::clone(&engine));
    let smart_on = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .cleanup_mode
        == CleanupMode::Smart;
    if smart_on {
        engine.warm();
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        engine.stop_if_idle(std::time::Duration::from_secs(5 * 60));
    });
}

/// Safe to call before `setup` has built the engine (a webview can ask
/// that early); reports Loading until then.
fn stt_status(app: &AppHandle) -> engine::EngineState {
    app.state::<AppState>()
        .stt
        .get()
        .map_or(engine::EngineState::Loading, |e| e.status())
}

/// Load the speech model in the background. A no-op before `setup` has
/// built the engine.
fn warm_speech(state: &AppState) {
    if let Some(engine) = state.stt.get() {
        engine.warm();
    }
}

/// Let go of the speech model so its files can be deleted. Before `setup`
/// has built the engine nothing is loaded, so there is nothing to release.
fn release_speech_model(state: &AppState) -> Result<(), String> {
    match state.stt.get() {
        Some(engine) if !engine.release() => Err("Stop using this model first".into()),
        _ => Ok(()),
    }
}

/// Build the speech engine, broadcast its status changes, start loading
/// the model, and drop it after 15 idle minutes.
fn start_stt(app: &AppHandle) {
    let dir = engine::models::model_dir(&app_support_dir(app), engine::stt::MODEL_ID);
    let engine = ParakeetEngine::new(dir);
    let emit_app = app.clone();
    engine.set_listener(Box::new(move || {
        let _ = emit_app.emit("engine-status", engine_status_view(&emit_app));
    }));
    let _ = app.state::<AppState>().stt.set(Arc::clone(&engine));
    engine.warm();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        engine.unload_if_idle(std::time::Duration::from_secs(15 * 60));
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| route_hotkey(app, shortcut, event.state()))
                .build(),
        )
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_hotkey,
            suspend_hotkey,
            resume_hotkey,
            get_status,
            mic_status,
            get_last_result,
            toggle_recording,
            cancel_dictation,
            set_overlay_hit_rect,
            get_glossary,
            save_glossary,
            get_history,
            clear_history,
            get_stats,
            get_snippets,
            save_snippets,
            get_learned_rules,
            delete_learned_rule,
            clear_learned_rules,
            get_transforms,
            save_transforms,
            reset_transforms,
            models_status,
            download_model,
            cancel_download,
            delete_model,
            engine_status
        ])
        .setup(|app| {
            // Load persisted settings before anything uses them.
            let settings_path = app_support_dir(app.handle()).join("settings.json");
            let loaded = Settings::load(&settings_path);
            // Persist the migrated result once so the one-time model migration
            // (and any hard-reset) doesn't re-run on every launch. Best-effort:
            // the runtime always uses the in-memory value regardless.
            let _ = loaded.save(&settings_path);
            let hotkey = loaded.hotkey.clone();
            let wants_paste = loaded.inject_mode == InjectMode::Paste;
            *app.state::<AppState>().settings.lock().unwrap() = loaded;

            // Paste mode needs Accessibility, and macOS ties the grant to the
            // exact binary — a rebuild silently revokes it. Prompt up front so
            // dictation doesn't quietly degrade to clipboard-only.
            if wants_paste && !platform::injector().can_inject().can_paste() {
                log_line(
                    app.handle(),
                    "startup: paste mode but NOT accessibility trusted -> showing grant prompt",
                );
                platform::injector().request_trust();
            }

            // Tray with quick actions; the window is a settings/status panel.
            let toggle_item =
                MenuItem::with_id(app, "toggle", "Start / Stop Dictation", true, None::<&str>)?;
            let show_item =
                MenuItem::with_id(app, "show", "Open Purl", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle_item, &show_item, &quit_item])?;
            TrayIconBuilder::with_id("main")
                // Bead-only mark: the full icon's sound arc blurs at menu-bar size.
                .icon(tauri::include_image!("icons/tray.png"))
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => toggle_dictation(app),
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if let Err(e) = app.global_shortcut().register(hotkey.as_str()) {
                notify(
                    app.handle(),
                    &format!("Couldn't register the dictation hotkey ({e}). Set a different one in Settings."),
                );
            }

            // Transforms: load (seeding built-ins on first run) and register
            // their hotkeys. A failed registration is logged, not fatal — the
            // user can rebind from the Transforms view.
            let tpath = transforms_path(app.handle());
            let tlist = transforms::load(&tpath);
            if !tpath.exists() {
                if let Err(e) = transforms::save(&tpath, &tlist) {
                    log_line(app.handle(), &format!("transforms seed save failed: {e}"));
                }
            }
            for t in &tlist {
                if let Some(hk) = t.hotkey.as_deref() {
                    if let Err(e) = app.global_shortcut().register(hk) {
                        log_line(
                            app.handle(),
                            &format!("transform hotkey {hk} failed to register: {e}"),
                        );
                    }
                }
            }
            *app.state::<AppState>().transforms.lock().unwrap() = tlist;

            // Load the speech model now so the first dictation doesn't pay
            // the model-load cost.
            start_stt(app.handle());
            start_llm(app.handle());

            // The voice pill is a persistent fixture from launch, parked on
            // whichever display currently has the cursor, and follows it
            // across monitors from here on.
            // It starts click-through so it can never steal a click from the
            // app underneath before the hover watcher's first poll.
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_ignore_cursor_events(true);
                // With "Displays have separate Spaces" (the macOS default),
                // a window belongs to one display's Space; joining all Spaces
                // lets the pill render on whichever display it is parked on.
                let _ = overlay.set_visible_on_all_workspaces(true);
                position_overlay(&overlay);
                let _ = overlay.show();
            }
            spawn_overlay_monitor_follow(app.handle().clone());
            spawn_overlay_click_through_watch(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the app (and hotkey) alive in the tray.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Quit stops the Smart server so it can't outlive the app.
            if let tauri::RunEvent::Exit = event {
                if let Some(llm) = app.state::<AppState>().llm.get() {
                    llm.shutdown();
                }
            }
        });
}

#[cfg(test)]
mod speech_engine_access_tests {
    use super::{release_speech_model, warm_speech, AppState};

    /// A webview can call download/delete before `setup` builds the engine.
    #[test]
    fn commands_before_setup_do_not_panic() {
        let state = AppState::default();
        warm_speech(&state);
        assert_eq!(release_speech_model(&state), Ok(()), "nothing loaded yet");
    }
}

#[cfg(test)]
mod pipeline_end_state_tests {
    use super::pipeline_end_state;

    #[test]
    fn success_ends_done_and_any_failure_ends_error() {
        assert_eq!(pipeline_end_state(&Ok(())), "done");
        for e in [
            "Didn't catch that — hold the key a little longer.",
            "The speech model isn't downloaded yet.",
            "Heard nothing — try again closer to the mic.",
        ] {
            assert_eq!(pipeline_end_state(&Err(e.into())), "error");
        }
    }
}

#[cfg(test)]
mod model_done_tests {
    use super::model_done;
    use crate::engine::download::DownloadError;

    #[test]
    fn success_has_no_error_or_kind() {
        let json = serde_json::to_value(model_done("parakeet-v3", &Ok(()))).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"id": "parakeet-v3", "ok": true, "error": null, "kind": null})
        );
    }

    #[test]
    fn failure_carries_message_and_kind() {
        let json =
            serde_json::to_value(model_done("qwen3.5-2b", &Err(DownloadError::Offline))).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["kind"], "offline");
        assert_eq!(
            json["error"],
            "You're offline. Connect to the internet and press Retry."
        );
    }
}

#[cfg(test)]
mod hotkey_swap_tests {
    use super::{swap_shortcut, SwapError};

    /// Test harness: `fails` is the set of combos whose registration errors.
    fn run(fails: &[&str], old: &str, new: &str) -> (Result<(), SwapError>, Vec<String>) {
        let fails: Vec<String> = fails.iter().map(|s| s.to_string()).collect();
        let mut log: Vec<String> = Vec::new();
        let result = {
            let log = std::cell::RefCell::new(&mut log);
            let mut register = |hk: &str| {
                log.borrow_mut().push(format!("reg:{hk}"));
                if fails.contains(&hk.to_string()) {
                    Err(format!("{hk} taken"))
                } else {
                    Ok(())
                }
            };
            let mut unregister = |hk: &str| {
                log.borrow_mut().push(format!("unreg:{hk}"));
            };
            swap_shortcut(&mut register, &mut unregister, old, new)
        };
        (result, log)
    }

    #[test]
    fn swaps_old_for_new_on_success() {
        let (result, log) = run(&[], "Cmd+A", "Cmd+B");
        assert!(result.is_ok());
        assert_eq!(log, vec!["unreg:Cmd+A", "reg:Cmd+B"]);
    }

    #[test]
    fn rolls_back_to_old_when_new_fails() {
        let (result, log) = run(&["Cmd+B"], "Cmd+A", "Cmd+B");
        assert!(matches!(result, Err(SwapError::NewRejected(_))));
        assert_eq!(log, vec!["unreg:Cmd+A", "reg:Cmd+B", "reg:Cmd+A"]);
    }

    #[test]
    fn reports_dead_state_when_rollback_also_fails() {
        let (result, log) = run(&["Cmd+A", "Cmd+B"], "Cmd+A", "Cmd+B");
        assert!(matches!(result, Err(SwapError::NoneActive { .. })));
        assert_eq!(log, vec!["unreg:Cmd+A", "reg:Cmd+B", "reg:Cmd+A"]);
    }

    #[test]
    fn same_combo_reregisters() {
        let (result, log) = run(&[], "Cmd+A", "Cmd+A");
        assert!(result.is_ok());
        assert_eq!(log, vec!["unreg:Cmd+A", "reg:Cmd+A"]);
    }

    #[test]
    fn same_combo_registration_failure_is_dead_state() {
        let (result, log) = run(&["Cmd+A"], "Cmd+A", "Cmd+A");
        assert!(matches!(result, Err(SwapError::NoneActive { .. })));
        assert_eq!(log, vec!["unreg:Cmd+A", "reg:Cmd+A"]);
    }
}

#[cfg(test)]
mod learning_wiring_tests {
    use crate::dictionary::Dictionary;
    use crate::learned::{self, Rule};

    fn rule(to: &str) -> Rule {
        Rule {
            from: "fun".into(),
            to: to.into(),
            before: "regards".into(),
            after: String::new(),
            created: 0,
            hits: 0,
            last_hit: 0,
            seen: 1,
        }
    }

    /// A learned word is present in the augmented dictionary's LLM-terms
    /// output, and absent from the base dictionary's.
    #[test]
    fn learned_terms_appear_in_the_augmented_dictionary_only() {
        let base = Dictionary::parse("SKU = stock keeping unit");
        let rules = vec![rule("Fan")];
        let local = base.with_extra_terms(&learned::glossary_terms(&rules));

        assert!(local.llm_terms().contains("Fan"));
        assert!(!base.llm_terms().contains("Fan"));
    }

    #[test]
    fn disabling_the_setting_yields_no_rules_to_apply() {
        let settings = crate::settings::Settings::default();
        assert_eq!(
            settings.learn_corrections,
            crate::platform::LEARNING_SUPPORTED,
            "learning is on by default wherever it is supported"
        );
    }

    /// Pins the stage order in `run_pipeline`: learned rules run *before*
    /// snippet expansion, so a rule can never rewrite a word inside an
    /// expansion the user never spoke — an address, a URL, a sign-off. The
    /// second half shows the order is load-bearing rather than incidental:
    /// reversed, the same rule eats a word out of the expanded email.
    #[test]
    fn a_rule_never_rewrites_inside_an_expanded_snippet() {
        let rules = vec![Rule {
            from: "fun".into(),
            to: "Fan".into(),
            before: "be".into(),
            after: String::new(),
            created: 0,
            hits: 0,
            last_hit: 0,
            seen: 1,
        }];
        let snippets = vec![crate::snippets::Snippet {
            trigger: "our tagline".into(),
            expansion: "work should be fun".into(),
        }];
        let spoken = "put our tagline at the end";

        // Shipping order: apply, then expand.
        let (corrected, fired) = learned::apply(spoken, &rules);
        let expanded = crate::snippets::expand(&corrected, &snippets);
        assert_eq!(expanded, "put work should be fun at the end");
        assert!(fired.is_empty());

        // Reversed, to prove the ordering is what protects the expansion.
        let expanded_first = crate::snippets::expand(spoken, &snippets);
        let (then_corrected, _) = learned::apply(&expanded_first, &rules);
        assert_eq!(then_corrected, "put work should be Fan at the end");
    }
}

/// The watch loop's exit paths and its supersession rule, against a fake
/// reader and a fake host — the reason `FocusReader` and `WatchHost` exist.
///
/// What this does not cover: `AppWatchHost` itself (one `app.emit` per
/// method and a `learned::add`, all of which need a running Tauri app), and
/// `spawn_correction_watch`'s thread spawn. What they *decide* — when the
/// dot lights, when it must not, which outcome is logged, and when reading
/// stops — is what is tested here.
#[cfg(test)]
mod correction_watch_tests {
    use crate::corrections::{Correction, Snapshot};
    use std::cell::RefCell;
    use std::time::Duration;

    fn snap(value: &str) -> Snapshot {
        Snapshot {
            id: "1:AXTextArea:Compose".into(),
            value: value.into(),
            secure: false,
        }
    }

    /// Returns each queued snapshot in turn, then `None` forever. Counts
    /// reads, so a test can assert that a superseded watcher stopped reading
    /// other applications' fields — not merely stopped emitting.
    struct FakeReader {
        queue: RefCell<std::collections::VecDeque<Option<Snapshot>>>,
        reads: std::cell::Cell<usize>,
    }

    impl FakeReader {
        fn new(items: Vec<Option<Snapshot>>) -> FakeReader {
            FakeReader {
                queue: RefCell::new(items.into_iter().collect()),
                reads: std::cell::Cell::new(0),
            }
        }
    }

    impl crate::platform::FocusReader for FakeReader {
        fn read(&self) -> Option<Snapshot> {
            self.reads.set(self.reads.get() + 1);
            self.queue.borrow_mut().pop_front().flatten()
        }
    }

    #[derive(Debug, PartialEq)]
    enum Event {
        Indicator(bool),
        /// (retracted `to`s, learned `to`s)
        Relearned(Vec<String>, Vec<String>),
    }

    /// Records what the watch did, and goes stale after `current_for` waits
    /// so supersession can be triggered at a chosen point in the loop.
    struct FakeHost {
        events: RefCell<Vec<Event>>,
        waits: std::cell::Cell<usize>,
        current_for: usize,
    }

    impl FakeHost {
        fn new(current_for: usize) -> FakeHost {
            FakeHost {
                events: RefCell::new(Vec::new()),
                waits: std::cell::Cell::new(0),
                current_for,
            }
        }
        fn events(&self) -> std::cell::Ref<'_, Vec<Event>> {
            self.events.borrow()
        }
    }

    impl crate::WatchHost for FakeHost {
        fn wait(&self, _d: Duration) {
            self.waits.set(self.waits.get() + 1);
        }
        fn indicator(&self, on: bool) {
            self.events.borrow_mut().push(Event::Indicator(on));
        }
        fn relearn(&self, retract: &[Correction], learn: &[Correction]) {
            self.events.borrow_mut().push(Event::Relearned(
                retract.iter().map(|c| c.to.clone()).collect(),
                learn.iter().map(|c| c.to.clone()).collect(),
            ));
        }
        fn log(&self, _line: &str) {}
        fn is_current(&self) -> bool {
            self.waits.get() <= self.current_for
        }
    }

    const NEVER_STALE: usize = usize::MAX;

    #[test]
    fn an_unreadable_field_lights_the_dot_and_clears_it() {
        // The common Electron outcome. The dot must still be honest: lit
        // before the read that found nothing, cleared right after.
        let reader = FakeReader::new(vec![None]);
        let host = FakeHost::new(NEVER_STALE);
        let outcome = crate::run_correction_watch(&reader, "hello there", &host);
        assert_eq!(outcome, "field not readable");
        assert_eq!(
            *host.events(),
            vec![Event::Indicator(true), Event::Indicator(false)]
        );
    }

    #[test]
    fn a_field_that_does_not_contain_the_paste_refuses_the_watch() {
        let reader = FakeReader::new(vec![Some(snap("something else entirely"))]);
        let host = FakeHost::new(NEVER_STALE);
        let outcome = crate::run_correction_watch(&reader, "hello there", &host);
        assert_eq!(outcome, "pasted text not found");
        assert_eq!(
            *host.events(),
            vec![Event::Indicator(true), Event::Indicator(false)]
        );
    }

    #[test]
    fn a_secure_field_is_never_watched() {
        let reader = FakeReader::new(vec![Some(Snapshot {
            id: "1:AXTextField:Password".into(),
            value: String::new(),
            secure: true,
        })]);
        let host = FakeHost::new(NEVER_STALE);
        assert_eq!(
            crate::run_correction_watch(&reader, "hello there", &host),
            "secure field"
        );
    }

    #[test]
    fn a_correction_seen_on_two_consecutive_polls_is_learned() {
        let mut reads = vec![Some(snap("Regards, fun."))];
        reads.extend(vec![
            Some(snap("Regards, Fan."));
            crate::WATCH_SETTLED_POLLS + 5
        ]);
        let reader = FakeReader::new(reads);
        let host = FakeHost::new(NEVER_STALE);
        let outcome = crate::run_correction_watch(&reader, "Regards, fun.", &host);
        assert_eq!(outcome, "learned");
        assert_eq!(
            *host.events(),
            vec![
                Event::Indicator(true),
                Event::Relearned(vec![], vec!["Fan".into()]),
                Event::Indicator(false),
            ]
        );
    }

    #[test]
    fn a_refined_edit_retracts_the_earlier_commit_and_stores_the_later_one() {
        // The user pauses long enough mid-edit for "Fa" to commit, then
        // finishes the word. The second stable state must replace the first,
        // not pile on top of it.
        let mut reads = vec![
            Some(snap("Regards, fun.")),
            Some(snap("Regards, Fa.")),
            Some(snap("Regards, Fa.")),
            Some(snap("Regards, Fan.")),
            Some(snap("Regards, Fan.")),
        ];
        reads.extend(vec![
            Some(snap("Regards, Fan."));
            crate::WATCH_SETTLED_POLLS + 5
        ]);
        let reader = FakeReader::new(reads);
        let host = FakeHost::new(NEVER_STALE);
        let outcome = crate::run_correction_watch(&reader, "Regards, fun.", &host);
        assert_eq!(outcome, "learned");
        assert_eq!(
            *host.events(),
            vec![
                Event::Indicator(true),
                Event::Relearned(vec![], vec!["Fa".into()]),
                Event::Relearned(vec!["Fa".into()], vec!["Fan".into()]),
                Event::Indicator(false),
            ]
        );
    }

    #[test]
    fn moving_focus_abandons_the_watch() {
        let mut elsewhere = snap("Regards, fun.");
        elsewhere.id = "9:AXTextArea:Other".into();
        let reader = FakeReader::new(vec![Some(snap("Regards, fun.")), Some(elsewhere)]);
        let host = FakeHost::new(NEVER_STALE);
        assert_eq!(
            crate::run_correction_watch(&reader, "Regards, fun.", &host),
            "focus changed"
        );
    }

    #[test]
    fn an_untouched_field_times_out_after_the_full_window() {
        let reader = FakeReader::new(vec![
            Some(snap("Regards, fun."));
            crate::WATCH_MAX_POLLS + 1
        ]);
        let host = FakeHost::new(NEVER_STALE);
        assert_eq!(
            crate::run_correction_watch(&reader, "Regards, fun.", &host),
            "timed out"
        );
        // One settling wait plus one per poll, and no read beyond them.
        assert_eq!(host.waits.get(), crate::WATCH_MAX_POLLS + 1);
        assert_eq!(reader.reads.get(), crate::WATCH_MAX_POLLS + 1);
    }

    #[test]
    fn a_watcher_superseded_before_its_first_read_never_reads_or_lights_the_dot() {
        let reader = FakeReader::new(vec![Some(snap("Regards, fun."))]);
        let host = FakeHost::new(0); // stale as soon as the settling wait ends
        assert_eq!(
            crate::run_correction_watch(&reader, "Regards, fun.", &host),
            "superseded"
        );
        assert_eq!(reader.reads.get(), 0);
        assert!(host.events().is_empty());
    }

    /// The bug this trait exists to pin. A superseded watcher must stop
    /// polling — it was reading whatever now has focus, for up to 90s after
    /// the dot went dark — and must not clear an indicator the newer watcher
    /// now owns.
    #[test]
    fn a_superseded_watcher_stops_reading_and_leaves_the_dot_alone() {
        let reader = FakeReader::new(vec![Some(snap("Regards, fun.")); 10]);
        let host = FakeHost::new(3); // current through the settling wait + 2 polls
        assert_eq!(
            crate::run_correction_watch(&reader, "Regards, fun.", &host),
            "superseded"
        );
        assert_eq!(reader.reads.get(), 3, "one first read, then two polls");
        assert_eq!(
            *host.events(),
            vec![Event::Indicator(true)],
            "the newer watcher owns the dot now; this one must not clear it"
        );
    }
}

#[cfg(test)]
mod platform_wiring_tests {
    use super::*;
    use crate::platform::Trust;

    /// Review focus #5: a second hotkey press while a session is processing
    /// is refused, and the guard frees up again once the session ends.
    #[test]
    fn second_session_rejected_while_processing() {
        let busy = Arc::new(AtomicBool::new(false));
        let first = try_begin_session(&busy);
        assert!(first.is_some());
        assert!(try_begin_session(&busy).is_none());
        drop(first);
        assert!(try_begin_session(&busy).is_some());
    }

    use crate::inject::InjectError;
    use crate::voice_commands::{Key, Segment};

    /// Runs `deliver_segments` with fakes; `fail_at` is the call number
    /// (paste or key, counting from 0) that fails. Returns the result and a
    /// log of every call made.
    fn deliver(fail_at: Option<usize>) -> (Result<Delivery, String>, Vec<String>) {
        let segments = vec![
            Segment::Text("hello".into()),
            Segment::Key(Key::Return),
            Segment::Text("world".into()),
        ];
        let log = std::cell::RefCell::new(Vec::new());
        let step = |what: String| {
            let n = log.borrow().len();
            log.borrow_mut().push(what);
            if Some(n) == fail_at {
                Err(InjectError::Synthesis("blocked".into()))
            } else {
                Ok(())
            }
        };
        let r = deliver_segments(
            &segments,
            "hello world",
            &mut |t| step(format!("paste:{t}")),
            &mut |k| step(format!("key:{k:?}")),
            &mut |t| {
                log.borrow_mut().push(format!("copy:{t}"));
                Ok(())
            },
        );
        (r, log.into_inner())
    }

    #[test]
    fn segments_are_sent_in_order() {
        let (r, log) = deliver(None);
        assert_eq!(r, Ok(Delivery::Sent));
        assert_eq!(log, ["paste:hello", "key:Return", "paste:world"]);
    }

    #[test]
    fn failed_key_puts_the_whole_dictation_on_the_clipboard() {
        let (r, log) = deliver(Some(1));
        assert!(matches!(r, Ok(Delivery::Copied(_))));
        // Stops at the failure: "world" is not pasted on its own.
        assert_eq!(log, ["paste:hello", "key:Return", "copy:hello world"]);
    }

    #[test]
    fn failed_first_paste_falls_back_too() {
        let (r, log) = deliver(Some(0));
        assert!(matches!(r, Ok(Delivery::Copied(_))));
        assert_eq!(log, ["paste:hello", "copy:hello world"]);
    }

    /// A pipeline thread that panics mid-session must not leave Purl
    /// refusing every later dictation.
    #[test]
    fn panicking_session_releases_the_guard() {
        let busy = Arc::new(AtomicBool::new(false));
        let held = try_begin_session(&busy).unwrap();
        let joined = std::thread::spawn(move || {
            let _held = held;
            panic!("pipeline blew up");
        })
        .join();
        assert!(joined.is_err());
        assert!(try_begin_session(&busy).is_some());
    }

    /// Review focus #1: a lost Accessibility grant reaches the UI as Denied,
    /// so Home can say "Paste needs Accessibility".
    #[test]
    fn trust_denied_surfaces_in_status() {
        let s = status_payload_for(
            crate::engine::EngineState::Loading,
            Trust::Denied,
            "CmdOrCtrl+Shift+Space",
            "idle",
        );
        assert_eq!(s.paste_trust, Trust::Denied);
        assert_eq!(s.learning_supported, platform::LEARNING_SUPPORTED);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["paste_trust"], "denied");
    }
}

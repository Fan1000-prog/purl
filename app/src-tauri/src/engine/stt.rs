//! Speech-to-text with Parakeet v3, in-process through ONNX Runtime.
//!
//! The model loads on a background thread, stays resident between
//! dictations and is dropped after a stretch of idleness. A dictation that
//! arrives mid-load waits for it rather than failing.

use super::models::{find, is_installed_in};
use super::EngineState;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

pub const MODEL_ID: &str = "parakeet-v3";
const SAMPLE_RATE: usize = 16_000;
/// Parakeet's attention cost grows with input length; 30 s keeps memory flat.
const MAX_SEGMENT_SECS: f32 = 30.0;
/// How long a dictation waits for a model that is still loading.
const LOAD_WAIT: Duration = Duration::from_secs(20);

pub trait SttEngine: Send + Sync {
    fn transcribe(&self, pcm16k_mono: &[f32]) -> Result<String, SttError>;
    fn status(&self) -> EngineState;
}

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("The speech model isn't downloaded yet.")]
    Missing,
    #[error("The speech model is still loading. Try again in a few seconds.")]
    StillLoading,
    #[error("Speech engine error: {0}")]
    Engine(String),
}

/// A loaded model. The seam lets the load/wait logic run against a stub.
trait Decoder: Send {
    fn decode(&mut self, pcm: &[f32]) -> Result<String, String>;
}

struct Parakeet(ParakeetModel);

impl Decoder for Parakeet {
    fn decode(&mut self, pcm: &[f32]) -> Result<String, String> {
        self.0
            .transcribe_with(pcm, &ParakeetParams::default())
            .map(|r| r.text)
            .map_err(|e| e.to_string())
    }
}

type Installed = Box<dyn Fn() -> bool + Send + Sync>;
type Loader = Box<dyn Fn() -> Result<Box<dyn Decoder>, String> + Send + Sync>;
type Listener = Box<dyn Fn() + Send + Sync>;
/// Shared so a decode can run without holding the state lock, and so an
/// unload can tell a decode is in flight (strong count > 1).
type SharedDecoder = Arc<Mutex<Box<dyn Decoder>>>;

enum Phase {
    Unloaded,
    Loading,
    Loaded(SharedDecoder),
    Failed(String),
}

struct Slot {
    phase: Phase,
    last_used: Instant,
}

pub struct ParakeetEngine {
    installed: Installed,
    loader: Loader,
    slot: Mutex<Slot>,
    /// Signalled when a load finishes, either way.
    load_done: Condvar,
    listener: Mutex<Option<Listener>>,
}

impl ParakeetEngine {
    pub fn new(model_dir: PathBuf) -> Arc<Self> {
        let load_dir = model_dir.clone();
        Self::with_backend(
            Box::new(move || find(MODEL_ID).is_some_and(|s| is_installed_in(&model_dir, s))),
            Box::new(move || {
                ParakeetModel::load(&load_dir, &Quantization::Int8)
                    .map(|m| Box::new(Parakeet(m)) as Box<dyn Decoder>)
                    .map_err(|e| e.to_string())
            }),
        )
    }

    fn with_backend(installed: Installed, loader: Loader) -> Arc<Self> {
        Arc::new(ParakeetEngine {
            installed,
            loader,
            slot: Mutex::new(Slot {
                phase: Phase::Unloaded,
                last_used: Instant::now(),
            }),
            load_done: Condvar::new(),
            listener: Mutex::new(None),
        })
    }

    /// Called after every status change, outside the engine's locks.
    pub fn set_listener(&self, listener: Listener) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    /// Load on a background thread if the model is installed and not
    /// already loaded or loading.
    pub fn warm(self: &Arc<Self>) {
        if self.begin_load() {
            let me = Arc::clone(self);
            std::thread::spawn(move || me.finish_load());
        }
    }

    /// Drop the model once it has gone unused for `idle`. Never mid-decode.
    pub fn unload_if_idle(&self, idle: Duration) {
        let unloaded = {
            let mut slot = self.slot.lock().unwrap();
            let idle_now = matches!(&slot.phase, Phase::Loaded(d) if Arc::strong_count(d) == 1)
                && slot.last_used.elapsed() >= idle;
            if idle_now {
                slot.phase = Phase::Unloaded;
            }
            idle_now
        };
        if unloaded {
            self.notify();
        }
    }

    /// Let go of the model files so they can be deleted. False while the
    /// model is loading or decoding.
    pub fn release(&self) -> bool {
        {
            let mut slot = self.slot.lock().unwrap();
            match &slot.phase {
                Phase::Loading => return false,
                Phase::Loaded(d) if Arc::strong_count(d) > 1 => return false,
                _ => slot.phase = Phase::Unloaded,
            }
        }
        self.notify();
        true
    }

    /// Unloaded/Failed + installed → Loading. True when this call claimed it.
    fn begin_load(&self) -> bool {
        let claimed = {
            let mut slot = self.slot.lock().unwrap();
            let can =
                matches!(slot.phase, Phase::Unloaded | Phase::Failed(_)) && (self.installed)();
            if can {
                slot.phase = Phase::Loading;
            }
            can
        };
        if claimed {
            self.notify();
        }
        claimed
    }

    /// Run the loader and record the outcome. A panic inside it (ONNX
    /// Runtime, a corrupt file) is caught and recorded as Failed, so the
    /// engine never sits in Loading with waiters blocked on it.
    fn finish_load(&self) {
        let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.loader)()));
        let phase = match loaded {
            Ok(Ok(d)) => Phase::Loaded(Arc::new(Mutex::new(d))),
            Ok(Err(e)) => Phase::Failed(e),
            Err(panic) => Phase::Failed(format!(
                "the speech model crashed while loading: {}",
                panic_message(&panic)
            )),
        };
        {
            let mut slot = self.slot.lock().unwrap();
            slot.phase = phase;
            slot.last_used = Instant::now();
        }
        self.load_done.notify_all();
        self.notify();
    }

    /// The loaded model, loading it on this thread or waiting for a load
    /// already in progress.
    fn acquire(&self) -> Result<SharedDecoder, SttError> {
        let mut tried = false;
        loop {
            let slot = self.slot.lock().unwrap();
            match &slot.phase {
                Phase::Loaded(d) => return Ok(Arc::clone(d)),
                Phase::Loading => {
                    let (slot, wait) = self
                        .load_done
                        .wait_timeout_while(slot, LOAD_WAIT, |s| matches!(s.phase, Phase::Loading))
                        .unwrap();
                    if wait.timed_out() {
                        return Err(SttError::StillLoading);
                    }
                    drop(slot);
                }
                Phase::Failed(reason) if tried => return Err(SttError::Engine(reason.clone())),
                Phase::Unloaded | Phase::Failed(_) => {
                    drop(slot);
                    if !self.begin_load() {
                        // Not installed, or another thread just claimed the load.
                        if !(self.installed)() {
                            return Err(SttError::Missing);
                        }
                        continue;
                    }
                    self.finish_load();
                    tried = true;
                }
            }
        }
    }

    fn notify(&self) {
        if let Some(listener) = self.listener.lock().unwrap().as_ref() {
            listener();
        }
    }
}

impl ParakeetEngine {
    /// Drop `decoder` if it is still the loaded model, so the next use loads
    /// a fresh one. For a decoder that panicked (and poisoned its mutex).
    fn discard(&self, decoder: &SharedDecoder) {
        let dropped = {
            let mut slot = self.slot.lock().unwrap();
            let current = matches!(&slot.phase, Phase::Loaded(d) if Arc::ptr_eq(d, decoder));
            if current {
                slot.phase = Phase::Unloaded;
            }
            current
        };
        if dropped {
            self.notify();
        }
    }

    /// Let go of a decoder that crashed, and say why in the error.
    fn crashed(&self, decoder: &SharedDecoder, why: &str) -> SttError {
        self.discard(decoder);
        SttError::Engine(format!("the speech engine crashed ({why}); try again"))
    }
}

/// The text of a caught panic, when it has one.
fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

impl SttEngine for ParakeetEngine {
    fn transcribe(&self, pcm16k_mono: &[f32]) -> Result<String, SttError> {
        let decoder = self.acquire()?;
        // A decoder that panicked mid-decode is unusable: its mutex is
        // poisoned and its state unknown. Let it go and report an error;
        // the next dictation loads a fresh model.
        // `None`: the mutex was poisoned by an earlier panic.
        let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut d = decoder.lock().ok()?;
            Some(
                segment(pcm16k_mono, MAX_SEGMENT_SECS)
                    .into_iter()
                    .map(|seg| d.decode(seg))
                    .collect::<Result<Vec<_>, _>>(),
            )
        }));
        let parts = match decoded {
            Ok(Some(Ok(parts))) => parts,
            Ok(Some(Err(e))) => return Err(SttError::Engine(e)),
            Ok(None) => return Err(self.crashed(&decoder, "an earlier decode crashed")),
            Err(panic) => return Err(self.crashed(&decoder, &panic_message(&panic))),
        };
        self.slot.lock().unwrap().last_used = Instant::now();
        let parts: Vec<&str> = parts
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        Ok(parts.join(" "))
    }

    fn status(&self) -> EngineState {
        match &self.slot.lock().unwrap().phase {
            Phase::Loaded(_) => EngineState::Ready,
            Phase::Loading => EngineState::Loading,
            Phase::Failed(reason) => EngineState::Failed {
                reason: reason.clone(),
            },
            Phase::Unloaded if (self.installed)() => EngineState::Idle,
            Phase::Unloaded => EngineState::Missing,
        }
    }
}

/// Split audio into pieces of at most `max_secs`, cutting each at the
/// middle of the quietest 200 ms window in its last third so words aren't
/// sliced in half.
pub fn segment(pcm: &[f32], max_secs: f32) -> Vec<&[f32]> {
    let max_len = ((max_secs * SAMPLE_RATE as f32) as usize).max(1);
    let mut out = Vec::new();
    let mut rest = pcm;
    while rest.len() > max_len {
        let (head, tail) = rest.split_at(quietest_cut(&rest[..max_len]));
        out.push(head);
        rest = tail;
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

fn quietest_cut(chunk: &[f32]) -> usize {
    const WINDOW: usize = SAMPLE_RATE / 5;
    const STEP: usize = SAMPLE_RATE / 100;
    let energy = |start: usize| {
        chunk[start..start + WINDOW]
            .iter()
            .map(|s| s * s)
            .sum::<f32>()
    };
    let mut best: Option<(usize, f32)> = None;
    let mut start = chunk.len() * 2 / 3;
    while start + WINDOW <= chunk.len() {
        let e = energy(start);
        if best.is_none_or(|(_, b)| e < b) {
            best = Some((start, e));
        }
        start += STEP;
    }
    best.map_or(chunk.len(), |(start, _)| start + WINDOW / 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    const SR: usize = 16_000;

    /// Guards the Windows build, which links Microsoft's onnxruntime.dll
    /// dynamically: a stale copy found first (System32 ships an older one)
    /// would show up here as a different version.
    #[test]
    fn links_the_pinned_onnx_runtime() {
        // The version string, not `ort::info()`: Microsoft's release DLL
        // reports git-branch=HEAD in its build info, so only the API's own
        // version names the release on both platforms.
        // SAFETY: OrtGetApiBase returns a static, non-null table, and
        // GetVersionString returns a static, NUL-terminated string.
        let version = unsafe {
            let base = ort::sys::OrtGetApiBase();
            std::ffi::CStr::from_ptr(((*base).GetVersionString)())
        }
        .to_string_lossy();
        assert!(version.starts_with("1.28."), "{version}");
    }

    #[test]
    fn short_audio_is_one_segment() {
        let a = vec![0.0; SR * 5];
        assert_eq!(segment(&a, 30.0).len(), 1);
    }

    #[test]
    fn long_audio_splits_under_limit() {
        let mut a = vec![0.3f32; SR * 70];
        for s in &mut a[SR * 28..SR * 28 + 3200] {
            *s = 0.0;
        } // a pause at 28 s
        let segs = segment(&a, 30.0);
        assert!(segs.iter().all(|s| s.len() <= SR * 30));
        assert_eq!(segs.iter().map(|s| s.len()).sum::<usize>(), a.len());
        assert_eq!(segs[0].len(), SR * 28 + 1600); // cut in the middle of the pause
    }

    #[test]
    fn empty_audio_is_no_segments() {
        assert!(segment(&[], 30.0).is_empty());
    }

    #[test]
    fn missing_model_reports_missing() {
        let e = ParakeetEngine::new("/nonexistent".into());
        assert_eq!(e.status(), EngineState::Missing);
        assert!(matches!(
            e.transcribe(&[0.0; 16000]),
            Err(SttError::Missing)
        ));
    }

    /// Answers "ok" for every segment.
    struct Stub;
    impl Decoder for Stub {
        fn decode(&mut self, _pcm: &[f32]) -> Result<String, String> {
            Ok("ok".into())
        }
    }

    fn slow_stub_engine(load_ms: u64, loads: Arc<AtomicUsize>) -> Arc<ParakeetEngine> {
        ParakeetEngine::with_backend(
            Box::new(|| true),
            Box::new(move || {
                loads.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(load_ms));
                Ok(Box::new(Stub) as Box<dyn Decoder>)
            }),
        )
    }

    #[test]
    fn transcribe_waits_for_loading_model() {
        let loads = Arc::new(AtomicUsize::new(0));
        let e = slow_stub_engine(300, loads.clone());
        assert_eq!(e.status(), EngineState::Idle);

        e.warm();
        assert_eq!(e.status(), EngineState::Loading);
        let t0 = Instant::now();
        assert_eq!(e.transcribe(&[0.0; 16000]).unwrap(), "ok");
        assert!(t0.elapsed() >= Duration::from_millis(200), "it waited");
        assert_eq!(e.status(), EngineState::Ready);
        assert_eq!(loads.load(Ordering::SeqCst), 1, "no second load");
    }

    #[test]
    fn transcribe_loads_an_unloaded_model_itself() {
        let loads = Arc::new(AtomicUsize::new(0));
        let e = slow_stub_engine(0, loads.clone());
        assert_eq!(e.transcribe(&[0.0; 16000]).unwrap(), "ok");
        assert_eq!(e.status(), EngineState::Ready);
    }

    #[test]
    fn a_panicking_loader_ends_failed_not_stuck_loading() {
        // Background load (warm) and inline load (transcribe) alike.
        let e = ParakeetEngine::with_backend(Box::new(|| true), Box::new(|| panic!("ort blew up")));
        e.warm();
        let t0 = std::time::Instant::now();
        while e.status() == EngineState::Loading {
            assert!(t0.elapsed() < Duration::from_secs(5), "stuck in Loading");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            matches!(e.status(), EngineState::Failed { reason } if reason.contains("ort blew up"))
        );
        // Inline: the retry panics too, and transcribe returns an error.
        assert!(matches!(
            e.transcribe(&[0.0; 16000]),
            Err(SttError::Engine(_))
        ));
    }

    /// Decodes fine on every load but the first, where it panics mid-decode
    /// (poisoning the decoder's mutex on the way out).
    struct PanicsOnce(bool);
    impl Decoder for PanicsOnce {
        fn decode(&mut self, _pcm: &[f32]) -> Result<String, String> {
            if self.0 {
                panic!("decoder blew up");
            }
            Ok("ok".into())
        }
    }

    #[test]
    fn a_panicking_decoder_is_dropped_and_reloaded() {
        let loads = Arc::new(AtomicUsize::new(0));
        let l = loads.clone();
        let e = ParakeetEngine::with_backend(
            Box::new(|| true),
            Box::new(move || {
                let first = l.fetch_add(1, Ordering::SeqCst) == 0;
                Ok(Box::new(PanicsOnce(first)) as Box<dyn Decoder>)
            }),
        );
        assert!(matches!(
            e.transcribe(&[0.0; 16000]),
            Err(SttError::Engine(r)) if r.contains("decoder blew up")
        ));
        assert_eq!(e.status(), EngineState::Idle, "the broken model was let go");
        assert_eq!(e.transcribe(&[0.0; 16000]).unwrap(), "ok");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failed_load_reports_reason() {
        let e =
            ParakeetEngine::with_backend(Box::new(|| true), Box::new(|| Err("bad onnx".into())));
        assert!(matches!(e.transcribe(&[0.0; 16000]), Err(SttError::Engine(r)) if r == "bad onnx"));
        assert_eq!(
            e.status(),
            EngineState::Failed {
                reason: "bad onnx".into()
            }
        );
    }

    #[test]
    fn idle_model_unloads_and_reports_idle() {
        let e = slow_stub_engine(0, Arc::new(AtomicUsize::new(0)));
        e.transcribe(&[0.0; 16000]).unwrap();
        e.unload_if_idle(Duration::from_secs(60));
        assert_eq!(e.status(), EngineState::Ready, "recently used stays loaded");
        e.unload_if_idle(Duration::ZERO);
        assert_eq!(e.status(), EngineState::Idle);
    }

    #[test]
    fn listener_hears_every_change() {
        let e = slow_stub_engine(0, Arc::new(AtomicUsize::new(0)));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let probe = e.clone();
        e.set_listener(Box::new(move || s.lock().unwrap().push(probe.status())));
        e.transcribe(&[0.0; 16000]).unwrap();
        e.unload_if_idle(Duration::ZERO);
        assert_eq!(
            *seen.lock().unwrap(),
            [EngineState::Loading, EngineState::Ready, EngineState::Idle]
        );
    }

    #[test]
    fn release_refuses_while_loading() {
        let e = slow_stub_engine(300, Arc::new(AtomicUsize::new(0)));
        e.warm();
        assert!(!e.release());
        e.transcribe(&[0.0; 16000]).unwrap();
        assert!(e.release());
        assert_eq!(e.status(), EngineState::Idle);
    }
}

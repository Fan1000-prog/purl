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

    fn finish_load(&self) {
        let phase = match (self.loader)() {
            Ok(d) => Phase::Loaded(Arc::new(Mutex::new(d))),
            Err(e) => Phase::Failed(e),
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

impl SttEngine for ParakeetEngine {
    fn transcribe(&self, pcm16k_mono: &[f32]) -> Result<String, SttError> {
        let decoder = self.acquire()?;
        let parts = {
            let mut d = decoder.lock().unwrap();
            segment(pcm16k_mono, MAX_SEGMENT_SECS)
                .into_iter()
                .map(|seg| d.decode(seg))
                .collect::<Result<Vec<_>, _>>()
                .map_err(SttError::Engine)?
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

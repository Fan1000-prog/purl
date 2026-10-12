//! Smart cleanup and transforms on a local LLM: Qwen3.5 GGUF served by the
//! bundled llama.cpp `llama-server`, spoken to over 127.0.0.1 only.
//!
//! The server starts on first use (or when a dictation begins), stays up
//! between dictations and is stopped after a stretch of idleness. A call
//! never waits past its budget: if the server is still starting, the call
//! times out and the start carries on for the next one.

use super::models::{find, is_file_verified, model_dir, registry, Kind};
use super::sidecar::{api_key, loopback_client, Sidecar};
use super::EngineState;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

pub trait LlmEngine: Send + Sync {
    /// One chat completion. `budget` is a hard deadline for the whole call,
    /// server start-up included.
    fn complete(&self, system: &str, user: &str, budget: Duration) -> Result<String, LlmError>;
    fn status(&self) -> EngineState;
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Smart model not downloaded")]
    Missing,
    #[error("Smart cleanup timed out")]
    Timeout,
    #[error("Smart engine error: {0}")]
    Engine(String),
}

/// The GGUF file for a Smart model id, or `None` for an unknown or
/// non-LLM id.
pub fn model_path(app_data: &Path, id: &str) -> Option<PathBuf> {
    let spec = find(id).filter(|s| s.kind == Kind::Llm)?;
    Some(model_dir(app_data, id).join(spec.files.first()?.name))
}

/// True when `path` is a registry model file that finished downloading and
/// passed its checksum.
fn is_verified_model(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    registry()
        .iter()
        .flat_map(|s| s.files)
        .find(|f| std::ffi::OsStr::new(f.name) == name)
        .is_some_and(|f| is_file_verified(dir, f))
}

/// The chat request. Thinking is switched off in the template as well as on
/// the server, and the reply is capped near the input's size: cleanup output
/// is about as long as its input, and the guard rejects anything far longer.
pub(crate) fn request_body(system: &str, user: &str) -> serde_json::Value {
    // ~3 characters per token is a safe over-estimate for English.
    let input_tokens = user.chars().count() / 3;
    let max_tokens = (input_tokens * 2 + 64).clamp(256, 1024);
    serde_json::json!({
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "temperature": 0.1,
        "max_tokens": max_tokens,
        "chat_template_kwargs": { "enable_thinking": false },
    })
}

/// The assistant message from a chat response. A reply cut off at
/// `max_tokens` is an error: half a sentence must never be pasted.
pub(crate) fn parse_reply(response: &serde_json::Value) -> Result<String, LlmError> {
    let choice = &response["choices"][0];
    if choice["finish_reason"] == "length" {
        return Err(LlmError::Engine("the reply was cut off".into()));
    }
    choice["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| LlmError::Engine("the reply had no text".into()))
}

/// Why one chat request failed.
pub(crate) enum ChatError {
    Timeout,
    /// The server process is gone; the next call starts a new one.
    Gone(String),
    Other(String),
}

/// A running server, as the engine sees it.
pub(crate) trait Server: Send + Sync {
    fn chat(
        &self,
        body: &serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, ChatError>;
}

/// The real thing: a `llama-server` sidecar and an HTTP client for it.
struct LlamaServer {
    sidecar: Mutex<Sidecar>,
    client: reqwest::blocking::Client,
    url: String,
    key: String,
}

impl Server for LlamaServer {
    fn chat(
        &self,
        body: &serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, ChatError> {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(&self.key)
            .json(body)
            .timeout(timeout)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json());
        response.map_err(|e| {
            if e.is_timeout() {
                ChatError::Timeout
            } else if !self.sidecar.lock().unwrap().is_alive() {
                ChatError::Gone(format!("llama-server stopped: {e}"))
            } else {
                ChatError::Other(e.to_string())
            }
        })
    }
}

/// Start `llama-server` for `model`, trying a second free port if the
/// first attempt fails (port taken, crash on start).
fn launch(bin: &Path, model: &Path, pid_file: Option<&Path>) -> Result<Arc<dyn Server>, String> {
    let mut last = String::new();
    for _ in 0..2 {
        let key = api_key().map_err(|e| format!("Smart engine couldn't start: {e}"))?;
        match Sidecar::spawn_llama(bin, model, &key, pid_file) {
            Ok(sidecar) => {
                let client = loopback_client(Duration::from_secs(60)).map_err(|e| e.to_string())?;
                let url = format!("http://127.0.0.1:{}/v1/chat/completions", sidecar.port());
                return Ok(Arc::new(LlamaServer {
                    sidecar: Mutex::new(sidecar),
                    client,
                    url,
                    key,
                }));
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("Smart engine couldn't start: {last}"))
}

type Installed = Box<dyn Fn(&Path) -> bool + Send + Sync>;
type Launcher = Box<dyn Fn(&Path) -> Result<Arc<dyn Server>, String> + Send + Sync>;
type Listener = Box<dyn Fn() + Send + Sync>;

enum Phase {
    Stopped,
    Starting,
    Running(Arc<dyn Server>),
    Failed(String),
}

struct Slot {
    phase: Phase,
    model: PathBuf,
    /// Bumped whenever the model changes or the server is stopped, so a
    /// start that finishes afterwards knows it is stale.
    generation: u64,
    last_used: Instant,
}

pub struct LlamaServerEngine {
    me: Weak<LlamaServerEngine>,
    installed: Installed,
    launcher: Launcher,
    slot: Mutex<Slot>,
    /// Signalled when a start finishes, either way.
    started: Condvar,
    listener: Mutex<Option<Listener>>,
}

impl LlamaServerEngine {
    /// `bin` is the bundled server, `model` the GGUF to serve. `pid_file`
    /// (macOS) records the running server so a crashed Purl's leftover can
    /// be reaped on the next launch.
    pub fn new(bin: PathBuf, model: PathBuf, pid_file: Option<PathBuf>) -> Arc<Self> {
        Self::with_backend(
            model,
            Box::new(is_verified_model),
            Box::new(move |model: &Path| launch(&bin, model, pid_file.as_deref())),
        )
    }

    fn with_backend(model: PathBuf, installed: Installed, launcher: Launcher) -> Arc<Self> {
        Arc::new_cyclic(|me| LlamaServerEngine {
            me: me.clone(),
            installed,
            launcher,
            slot: Mutex::new(Slot {
                phase: Phase::Stopped,
                model,
                generation: 0,
                last_used: Instant::now(),
            }),
            started: Condvar::new(),
            listener: Mutex::new(None),
        })
    }

    /// Called after every status change, outside the engine's locks.
    pub fn set_listener(&self, listener: Listener) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    /// Start the server on a background thread if the model is installed
    /// and it isn't already up or starting.
    pub fn warm(&self) {
        let Some((generation, model)) = self.begin_start() else {
            return;
        };
        let me = self.me.clone();
        std::thread::spawn(move || {
            if let Some(me) = me.upgrade() {
                me.finish_start(generation, &model);
            }
        });
    }

    /// Stop the server once it has gone unused for `idle`. Never mid-request.
    pub fn stop_if_idle(&self, idle: Duration) {
        self.stop_when(|slot| match &slot.phase {
            Phase::Running(s) => Arc::strong_count(s) == 1 && slot.last_used.elapsed() >= idle,
            _ => false,
        });
    }

    /// Serve `model` from now on. A running server is stopped; the next use
    /// starts one on the new model.
    pub fn set_model(&self, model: PathBuf) {
        let old = {
            let mut slot = self.slot.lock().unwrap();
            if slot.model == model {
                return;
            }
            slot.model = model;
            slot.generation += 1;
            std::mem::replace(&mut slot.phase, Phase::Stopped)
        };
        drop(old);
        self.started.notify_all();
        self.notify();
    }

    /// Stop the server so its model file can be deleted. False while it is
    /// starting or answering a request. The check and the stop share one
    /// lock, so a dictation can't take the server in between.
    pub fn release(&self) -> bool {
        self.stop_when(|slot| match &slot.phase {
            Phase::Starting => false,
            Phase::Running(s) => Arc::strong_count(s) == 1,
            _ => true,
        })
    }

    /// Stop the server now, whatever it is doing. For app exit.
    pub fn shutdown(&self) {
        self.stop_when(|_| true);
    }

    /// Stop (and forget any failure) when `should` says so. True when the
    /// server is stopped afterwards, including when it already was.
    fn stop_when(&self, should: impl FnOnce(&Slot) -> bool) -> bool {
        let old = {
            let mut slot = self.slot.lock().unwrap();
            if matches!(slot.phase, Phase::Stopped) {
                return true;
            }
            if !should(&slot) {
                return false;
            }
            slot.generation += 1;
            std::mem::replace(&mut slot.phase, Phase::Stopped)
        };
        // Dropping the last handle kills the process; do it unlocked.
        drop(old);
        self.started.notify_all();
        self.notify();
        true
    }

    /// Stopped/Failed + installed → Starting. Returns what to start when
    /// this call claimed it.
    fn begin_start(&self) -> Option<(u64, PathBuf)> {
        let claimed = {
            let mut slot = self.slot.lock().unwrap();
            let can = matches!(slot.phase, Phase::Stopped | Phase::Failed(_))
                && (self.installed)(&slot.model);
            if can {
                slot.phase = Phase::Starting;
            }
            can.then(|| (slot.generation, slot.model.clone()))
        };
        if claimed.is_some() {
            self.notify();
        }
        claimed
    }

    fn finish_start(&self, generation: u64, model: &Path) {
        let result = (self.launcher)(model);
        let stale = {
            let mut slot = self.slot.lock().unwrap();
            if slot.generation != generation {
                // The model changed or the engine was stopped meanwhile.
                Some(result)
            } else {
                slot.phase = match result {
                    Ok(server) => Phase::Running(server),
                    Err(reason) => Phase::Failed(reason),
                };
                slot.last_used = Instant::now();
                None
            }
        };
        drop(stale);
        self.started.notify_all();
        self.notify();
    }

    /// The running server, starting it if needed and waiting no later than
    /// `deadline`. A failed start is retried once per call.
    fn acquire(&self, deadline: Instant) -> Result<Arc<dyn Server>, LlmError> {
        let mut retried = false;
        loop {
            let slot = self.slot.lock().unwrap();
            match &slot.phase {
                Phase::Running(s) => return Ok(Arc::clone(s)),
                Phase::Failed(reason) if retried => return Err(LlmError::Engine(reason.clone())),
                // Nothing could be started: the model went away, or the
                // engine was stopped while this call waited.
                Phase::Stopped if retried => {
                    return Err(if (self.installed)(&slot.model) {
                        LlmError::Engine("Smart engine was stopped".into())
                    } else {
                        LlmError::Missing
                    })
                }
                Phase::Stopped | Phase::Failed(_) => {
                    drop(slot);
                    retried = true;
                    self.warm();
                }
                Phase::Starting => {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    let (_slot, result) = self
                        .started
                        .wait_timeout_while(slot, wait, |s| matches!(s.phase, Phase::Starting))
                        .unwrap();
                    if result.timed_out() {
                        return Err(LlmError::Timeout);
                    }
                }
            }
        }
    }

    /// Forget `server` after it died, so the next call starts a fresh one.
    fn forget(&self, server: &Arc<dyn Server>) {
        self.stop_when(|slot| matches!(&slot.phase, Phase::Running(s) if Arc::ptr_eq(s, server)));
    }

    fn notify(&self) {
        if let Some(listener) = self.listener.lock().unwrap().as_ref() {
            listener();
        }
    }
}

impl LlmEngine for LlamaServerEngine {
    fn complete(&self, system: &str, user: &str, budget: Duration) -> Result<String, LlmError> {
        let deadline = Instant::now() + budget;
        let model = self.slot.lock().unwrap().model.clone();
        if !(self.installed)(&model) {
            return Err(LlmError::Missing);
        }
        let server = self.acquire(deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LlmError::Timeout);
        }
        let result = server.chat(&request_body(system, user), remaining);
        self.slot.lock().unwrap().last_used = Instant::now();
        match result {
            Ok(response) => parse_reply(&response),
            Err(ChatError::Timeout) => Err(LlmError::Timeout),
            Err(ChatError::Gone(e)) => {
                self.forget(&server);
                Err(LlmError::Engine(e))
            }
            Err(ChatError::Other(e)) => Err(LlmError::Engine(e)),
        }
    }

    fn status(&self) -> EngineState {
        let slot = self.slot.lock().unwrap();
        match &slot.phase {
            Phase::Running(_) => EngineState::Ready,
            Phase::Starting => EngineState::Loading,
            Phase::Failed(reason) => EngineState::Failed {
                reason: reason.clone(),
            },
            Phase::Stopped if (self.installed)(&slot.model) => EngineState::Idle,
            Phase::Stopped => EngineState::Missing,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Stands in for a running llama-server: answers with `reply`, or as a
    /// crashed server when `gone` is set. Flags `dropped` when stopped.
    struct FakeServer {
        reply: &'static str,
        gone: bool,
        dropped: Arc<AtomicBool>,
    }

    impl Server for FakeServer {
        fn chat(
            &self,
            body: &serde_json::Value,
            _: Duration,
        ) -> Result<serde_json::Value, ChatError> {
            assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
            if self.gone {
                return Err(ChatError::Gone("connection refused".into()));
            }
            Ok(serde_json::json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": self.reply } }]
            }))
        }
    }

    impl Drop for FakeServer {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    struct Harness {
        engine: Arc<LlamaServerEngine>,
        launches: Arc<Mutex<Vec<PathBuf>>>,
        dropped: Arc<AtomicBool>,
    }

    /// An engine whose launcher takes `start_ms` and then yields a server
    /// built by `server`; `fail` makes every launch fail instead.
    fn harness(start_ms: u64, fail: bool, reply: &'static str, gone: bool) -> Harness {
        let launches = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let (l, d) = (launches.clone(), dropped.clone());
        let engine = LlamaServerEngine::with_backend(
            PathBuf::from("/m/qwen.gguf"),
            Box::new(|p: &Path| !p.starts_with("/missing")),
            Box::new(move |p: &Path| {
                std::thread::sleep(Duration::from_millis(start_ms));
                l.lock().unwrap().push(p.to_path_buf());
                if fail {
                    return Err("llama-server exited during start-up".into());
                }
                Ok(Arc::new(FakeServer {
                    reply,
                    gone,
                    dropped: d.clone(),
                }) as Arc<dyn Server>)
            }),
        );
        Harness {
            engine,
            launches,
            dropped,
        }
    }

    const BUDGET: Duration = Duration::from_secs(5);

    #[test]
    fn missing_model_reports_missing() {
        let h = harness(0, false, "ok", false);
        h.engine.set_model(PathBuf::from("/missing/qwen.gguf"));
        assert_eq!(h.engine.status(), EngineState::Missing);
        assert!(matches!(
            h.engine.complete("s", "u", BUDGET),
            Err(LlmError::Missing)
        ));
        assert!(h.launches.lock().unwrap().is_empty());
    }

    #[test]
    fn model_deleted_mid_call_is_missing_not_a_hang() {
        let installed = Arc::new(AtomicUsize::new(0));
        let i = installed.clone();
        // Installed for the up-front check only, gone by the time of the start.
        let engine = LlamaServerEngine::with_backend(
            PathBuf::from("/m/qwen.gguf"),
            Box::new(move |_: &Path| i.fetch_add(1, Ordering::SeqCst) == 0),
            Box::new(|_: &Path| unreachable!("nothing to start")),
        );
        let t0 = Instant::now();
        assert!(matches!(
            engine.complete("s", "u", BUDGET),
            Err(LlmError::Missing)
        ));
        assert!(t0.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn first_call_starts_the_server_then_reuses_it() {
        let h = harness(0, false, "Hello.", false);
        assert_eq!(h.engine.status(), EngineState::Idle);
        assert_eq!(h.engine.complete("s", "u", BUDGET).unwrap(), "Hello.");
        assert_eq!(h.engine.status(), EngineState::Ready);
        assert_eq!(h.engine.complete("s", "u", BUDGET).unwrap(), "Hello.");
        assert_eq!(h.launches.lock().unwrap().len(), 1);
    }

    #[test]
    fn budget_covers_start_up_and_the_start_carries_on() {
        let h = harness(400, false, "Hello.", false);
        let t0 = Instant::now();
        assert!(matches!(
            h.engine.complete("s", "u", Duration::from_millis(50)),
            Err(LlmError::Timeout)
        ));
        assert!(
            t0.elapsed() < Duration::from_millis(300),
            "blocked past its budget"
        );
        assert_eq!(h.engine.status(), EngineState::Loading);
        // The next dictation finds the server the first one started.
        assert_eq!(h.engine.complete("s", "u", BUDGET).unwrap(), "Hello.");
        assert_eq!(h.launches.lock().unwrap().len(), 1);
    }

    #[test]
    fn failed_start_reports_the_reason() {
        let h = harness(0, true, "", false);
        assert!(matches!(
            h.engine.complete("s", "u", BUDGET),
            Err(LlmError::Engine(_))
        ));
        assert_eq!(
            h.engine.status(),
            EngineState::Failed {
                reason: "llama-server exited during start-up".into()
            }
        );
    }

    #[test]
    fn crashed_server_is_restarted_on_next_use() {
        let h = harness(0, false, "", true);
        assert!(matches!(
            h.engine.complete("s", "u", BUDGET),
            Err(LlmError::Engine(_))
        ));
        assert_eq!(h.engine.status(), EngineState::Idle);
        let _ = h.engine.complete("s", "u", BUDGET);
        assert_eq!(h.launches.lock().unwrap().len(), 2);
    }

    #[test]
    fn idle_server_stops() {
        let h = harness(0, false, "Hello.", false);
        h.engine.complete("s", "u", BUDGET).unwrap();
        h.engine.stop_if_idle(Duration::from_secs(300));
        assert_eq!(
            h.engine.status(),
            EngineState::Ready,
            "not idle long enough"
        );
        h.engine.stop_if_idle(Duration::ZERO);
        assert_eq!(h.engine.status(), EngineState::Idle);
        assert!(
            h.dropped.load(Ordering::SeqCst),
            "server process not stopped"
        );
    }

    #[test]
    fn new_model_restarts_lazily() {
        let h = harness(0, false, "Hello.", false);
        h.engine.complete("s", "u", BUDGET).unwrap();
        h.engine.set_model(PathBuf::from("/m/qwen-4b.gguf"));
        assert!(h.dropped.load(Ordering::SeqCst));
        assert_eq!(h.engine.status(), EngineState::Idle);
        h.engine.complete("s", "u", BUDGET).unwrap();
        assert_eq!(
            *h.launches.lock().unwrap(),
            [
                PathBuf::from("/m/qwen.gguf"),
                PathBuf::from("/m/qwen-4b.gguf")
            ]
        );
    }

    #[test]
    fn a_start_for_an_old_model_is_discarded() {
        let h = harness(300, false, "Hello.", false);
        h.engine.warm();
        h.engine.set_model(PathBuf::from("/m/qwen-4b.gguf"));
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            h.dropped.load(Ordering::SeqCst),
            "stale server kept running"
        );
        assert_eq!(h.engine.status(), EngineState::Idle);
    }

    #[test]
    fn release_and_shutdown_stop_the_server() {
        let h = harness(0, false, "Hello.", false);
        h.engine.complete("s", "u", BUDGET).unwrap();
        assert!(h.engine.release());
        assert!(h.dropped.load(Ordering::SeqCst));
        h.engine.complete("s", "u", BUDGET).unwrap();
        h.engine.shutdown();
        assert_eq!(h.engine.status(), EngineState::Idle);
    }

    #[test]
    fn release_waits_for_a_request_in_flight() {
        let h = harness(0, false, "Hello.", false);
        h.engine.complete("s", "u", BUDGET).unwrap();
        // A dictation holds the server, as `complete` does mid-request.
        let in_flight = h.engine.acquire(Instant::now() + BUDGET).unwrap();
        assert!(!h.engine.release(), "released mid-request");
        assert!(!h.dropped.load(Ordering::SeqCst));
        assert_eq!(h.engine.status(), EngineState::Ready);
        drop(in_flight);
        assert!(h.engine.release());
        assert!(h.dropped.load(Ordering::SeqCst));
        assert!(h.engine.release(), "already stopped counts as released");
    }

    #[test]
    fn release_refuses_while_starting() {
        let h = harness(300, false, "Hello.", false);
        h.engine.warm();
        assert_eq!(h.engine.status(), EngineState::Loading);
        assert!(!h.engine.release());
        assert_eq!(h.engine.status(), EngineState::Loading);
    }

    #[test]
    fn release_clears_a_failed_start() {
        let h = harness(0, true, "", false);
        let _ = h.engine.complete("s", "u", BUDGET);
        assert!(matches!(h.engine.status(), EngineState::Failed { .. }));
        assert!(h.engine.release());
        assert_eq!(h.engine.status(), EngineState::Idle);
    }

    #[test]
    fn listener_hears_start_and_stop() {
        let h = harness(0, false, "Hello.", false);
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        h.engine.set_listener(Box::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        h.engine.complete("s", "u", BUDGET).unwrap();
        h.engine.stop_if_idle(Duration::ZERO);
        assert!(calls.load(Ordering::SeqCst) >= 3, "loading, ready, idle");
    }

    /// Something else grabbed the port and says yes to everything: the
    /// start is refused, and the dictation is cleaned with Fast instead.
    #[cfg(unix)]
    #[test]
    fn a_squatter_on_the_port_falls_back_to_fast() {
        use crate::engine::sidecar::Sidecar;
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = stream.read(&mut [0u8; 4096]);
                let body =
                    r#"{"choices":[{"finish_reason":"stop","message":{"content":"Pwned."}}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let engine = LlamaServerEngine::with_backend(
            PathBuf::from("/m/qwen.gguf"),
            Box::new(|_: &Path| true),
            Box::new(move |_: &Path| {
                let mut cmd = std::process::Command::new("sleep");
                cmd.arg("30");
                match Sidecar::start(cmd, port, Duration::from_secs(5), None) {
                    Ok(_) => Err("the squatter was accepted".into()),
                    Err(e) => Err(e.to_string()),
                }
            }),
        );
        let r = crate::cleanup::smart_clean(
            &*engine,
            "um hello there",
            "",
            crate::settings::StructureMode::None,
            Duration::from_secs(10),
            || "Hello there.".into(),
        );
        assert_eq!(r.text, "Hello there.");
        assert!(!r.used_llm);
        assert_eq!(r.note.as_deref(), Some("Smart cleanup failed, used Fast"));
        assert!(
            matches!(engine.status(), EngineState::Failed { reason } if reason.contains("another program")),
            "{:?}",
            engine.status()
        );
    }

    #[test]
    fn request_turns_thinking_off_and_bounds_output() {
        let body = request_body("sys", "hello");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "sys");
        assert_eq!(body["messages"][1]["content"], "hello");
        assert_eq!(body["temperature"], 0.1);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["max_tokens"], 256);
        let long = request_body("sys", &"word ".repeat(2000));
        assert_eq!(long["max_tokens"], 1024);
    }

    #[test]
    fn reply_is_the_message_content_unless_cut_off() {
        let reply = |finish: &str| serde_json::json!({ "choices": [{ "finish_reason": finish, "message": { "content": "Hi." } }] });
        assert_eq!(parse_reply(&reply("stop")).unwrap(), "Hi.");
        assert!(parse_reply(&reply("length")).is_err());
        assert!(parse_reply(&serde_json::json!({})).is_err());
    }

    /// The real server from `vendor/llama/` (run `npm run fetch-llama`) on
    /// the downloaded 2B model (`cargo run --example fetch_models qwen3.5-2b`).
    pub(crate) fn real_engine() -> Arc<LlamaServerEngine> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let home =
            PathBuf::from(std::env::var(if cfg!(windows) { "APPDATA" } else { "HOME" }).unwrap());
        let app_data = if cfg!(windows) {
            home.join("app.purl.desktop")
        } else {
            home.join("Library/Application Support/app.purl.desktop")
        };
        let model = model_path(&app_data, "qwen3.5-2b").unwrap();
        assert!(
            is_verified_model(&model),
            "download the model first: {}",
            model.display()
        );
        let bin = crate::engine::sidecar::llama_server_path(&manifest.join("vendor"));
        LlamaServerEngine::new(bin, model, None)
    }

    #[test]
    #[ignore = "needs llama-server and the 1.4 GB Qwen3.5-2B model; run: cargo test -- --ignored sidecar_dies_with_app"]
    fn sidecar_dies_with_app() {
        let engine = real_engine();
        let t0 = Instant::now();
        engine.warm();
        while engine.status() == EngineState::Loading {
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "server never came up"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(engine.status(), EngineState::Ready);
        eprintln!("llama-server ready in {:?}", t0.elapsed());
        let pid = match &engine.slot.lock().unwrap().phase {
            Phase::Running(_) => server_pid(),
            _ => unreachable!(),
        };
        engine.shutdown();
        let t1 = Instant::now();
        while process_alive(pid) {
            assert!(
                t1.elapsed() < Duration::from_secs(2),
                "llama-server {pid} outlived the engine"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The one llama-server this test process started.
    fn server_pid() -> u32 {
        let out = std::process::Command::new("pgrep")
            .args(["-P", &std::process::id().to_string(), "llama-server"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn process_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

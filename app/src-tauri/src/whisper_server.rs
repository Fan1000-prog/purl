//! Resident `whisper-server` (whisper.cpp) so the ~466 MB model is loaded
//! once at startup instead of on every dictation — measured ~2s saved per
//! run. The server binds to 127.0.0.1 on a random free port; audio never
//! leaves the machine. If the server is missing, still loading, or has
//! crashed, callers fall back to the `whisper-cli` subprocess path.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Per-request ceiling: transcription of a long dictation on a slow CPU.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub struct WhisperServer {
    child: Child,
    port: u16,
    /// Model the server was started with — a settings change to a different
    /// model requires a restart to pick up.
    pub model: PathBuf,
}

/// Locate whisper-server with the same precedence as whisper-cli: bundled
/// sidecar, Homebrew locations, then PATH.
pub fn find_bin(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = resource_dir {
        candidates.push(dir.join("bin/whisper-server"));
    }
    candidates.push(PathBuf::from("/usr/local/bin/whisper-server"));
    candidates.push(PathBuf::from("/opt/homebrew/bin/whisper-server"));
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    let out = Command::new("which").arg("whisper-server").output().ok()?;
    if out.status.success() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

/// Ask the OS for a free localhost port. Small race window between drop and
/// the server binding it — acceptable, a failed spawn just means CLI fallback.
fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    Some(listener.local_addr().ok()?.port())
}

impl WhisperServer {
    /// Start the server. Returns immediately; the model loads in the
    /// background (~2s) and requests before readiness simply fail (callers
    /// fall back to CLI for that one dictation).
    pub fn spawn(bin: &Path, model: &Path, threads: usize) -> Result<Self, String> {
        if !model.is_file() {
            return Err(format!("model not found at {}", model.display()));
        }
        // A force-quit or dev hot-restart skips our exit hook and orphans the
        // previous server (~500 MB each). Any server on a model in our
        // directory is ours and stale by definition — the caller holds the
        // state lock with an empty slot. Match the directory, not the exact
        // file, so a model switch also reaps the old model's server.
        let sweep_dir = model.parent().unwrap_or(model);
        let _ = Command::new("pkill")
            .arg("-f")
            .arg(format!("whisper-server -m {}", sweep_dir.display()))
            .status();
        let port = free_port().ok_or("no free port")?;
        let child = Command::new(bin)
            .arg("-m")
            .arg(model)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("-t")
            .arg(threads.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn failed: {e}"))?;
        Ok(Self {
            child,
            port,
            model: model.to_path_buf(),
        })
    }

    /// True while the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Transcribe a 16 kHz mono WAV via the resident server. Greedy decoding
    /// (beam 1) — measurably faster than whisper-cli's default beam 5 with
    /// no observed quality loss on dictation-length audio.
    pub fn transcribe(&self, wav: &Path, prompt: &str) -> Result<String, String> {
        let form = reqwest::blocking::multipart::Form::new()
            .file("file", wav)
            .map_err(|e| format!("read wav: {e}"))?
            .text("response_format", "text")
            .text("prompt", prompt.to_string())
            .text("beam_size", "1")
            .text("best_of", "1")
            .text("temperature", "0");

        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .post(format!("http://127.0.0.1:{}/inference", self.port))
            .multipart(form)
            .send()
            .map_err(|e| format!("request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("server returned {}", resp.status()));
        }
        let text = resp.text().map_err(|e| e.to_string())?;
        Ok(crate::transcribe::normalize(&text))
    }
}

impl Drop for WhisperServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

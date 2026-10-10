//! Transcription via a `whisper-cli` subprocess (whisper.cpp).
//! Args are always passed as an array — no shell interpolation anywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("whisper-cli not found — install with `brew install whisper-cpp`")]
    BinaryNotFound,
    #[error("whisper model not found at {0} — see README for the download step")]
    ModelNotFound(PathBuf),
    #[error("whisper-cli failed: {0}")]
    Failed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Locate whisper-cli: bundled sidecar first (inside the .app), then the
/// usual Homebrew locations, then PATH.
pub fn find_whisper_bin(resource_dir: Option<&Path>) -> Result<PathBuf, TranscribeError> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = resource_dir {
        candidates.push(dir.join("bin/whisper-cli"));
    }
    candidates.push(PathBuf::from("/usr/local/bin/whisper-cli"));
    candidates.push(PathBuf::from("/opt/homebrew/bin/whisper-cli"));
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    // PATH fallback.
    if let Ok(out) = Command::new("which").arg("whisper-cli").output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Ok(PathBuf::from(p));
            }
        }
    }
    Err(TranscribeError::BinaryNotFound)
}

/// Run whisper-cli on a 16 kHz mono WAV and return the transcript text.
pub fn transcribe(
    wav: &Path,
    model: &Path,
    whisper_bin: &Path,
    prompt: &str,
) -> Result<String, TranscribeError> {
    if !model.is_file() {
        return Err(TranscribeError::ModelNotFound(model.to_path_buf()));
    }

    // -otxt writes `<prefix>.txt`; keep the prefix next to the wav.
    let out_prefix = wav.with_extension("out");
    let threads = thread_count();

    let output = Command::new(whisper_bin)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(wav)
        .arg("--prompt")
        .arg(prompt)
        .arg("-t")
        .arg(threads.to_string())
        // Greedy decoding — ~25% faster than the default beam 5 with no
        // observed quality loss on dictation-length audio.
        .arg("-bs")
        .arg("1")
        .arg("-bo")
        .arg("1")
        .arg("-np")
        .arg("-otxt")
        .arg("-of")
        .arg(&out_prefix)
        .output()?;

    if !output.status.success() {
        return Err(TranscribeError::Failed(
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(500)
                .collect(),
        ));
    }

    let txt_path = out_prefix.with_extension("out.txt");
    let text = std::fs::read_to_string(&txt_path)?;
    let _ = std::fs::remove_file(&txt_path);

    Ok(normalize(&text))
}

/// Whisper worker threads: leave two cores for the UI and audio capture.
pub fn thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(2).max(2))
        .unwrap_or(4)
}

/// Collapse whisper's line breaks/indent into a single clean paragraph.
pub fn normalize(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_joins_lines_and_trims() {
        let raw = " Hey team, the QBR deck\n is in the shared drive.\n\n";
        assert_eq!(normalize(raw), "Hey team, the QBR deck is in the shared drive.");
    }

    #[test]
    fn missing_model_is_reported() {
        let err = transcribe(
            Path::new("/tmp/x.wav"),
            Path::new("/nonexistent/model.bin"),
            Path::new("/usr/local/bin/whisper-cli"),
            "",
        )
        .unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }

    /// End-to-end against the real binary + model when both are present
    /// (dev machines); skipped otherwise so CI stays green.
    #[test]
    fn transcribes_synthesized_speech_when_toolchain_present() {
        let model = dirs_model();
        let bin = match find_whisper_bin(None) {
            Ok(b) => b,
            Err(_) => return, // no whisper on this machine — skip
        };
        if !model.is_file() {
            return; // no model — skip
        }
        let wav = std::env::temp_dir().join("purl-test-say.wav");
        let aiff = std::env::temp_dir().join("purl-test-say.aiff");
        let ok = Command::new("say")
            .arg("-o")
            .arg(&aiff)
            .arg("The QBR deck is in the shared drive")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // no `say` — skip
        }
        Command::new("afconvert")
            .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
            .arg(&aiff)
            .arg(&wav)
            .status()
            .unwrap();
        let text = transcribe(&wav, &model, &bin, "Vocabulary: QBR.").unwrap();
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_file(&aiff);
        assert!(!text.is_empty());
        assert!(text.to_uppercase().contains("QBR"), "got: {text}");
    }

    fn dirs_model() -> PathBuf {
        let models = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Library/Application Support/app.purl.desktop/models");
        let base = models.join("ggml-base.en.bin");
        if base.is_file() {
            base
        } else {
            models.join("ggml-small.en.bin")
        }
    }
}

//! Pull an Ollama model with streamed progress. The HTTP streaming lives here;
//! the Tauri command in lib.rs adapts progress into `ollama-pull` events.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::time::Duration;

use crate::cleanup::OLLAMA_URL;

/// Progress emitted to the frontend for one pull.
#[derive(Debug, Clone, Serialize)]
pub struct PullEvent {
    pub model: String,
    pub percent: u8,
    pub status: String,
    pub done: bool,
    pub error: Option<String>,
}

/// One line of Ollama's /api/pull stream.
#[derive(Deserialize)]
struct PullLine {
    #[serde(default)]
    status: String,
    completed: Option<u64>,
    total: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

/// completed/total → percent, clamped to 100; carries `last` when the line has
/// no byte counts (status-only lines like "verifying sha256").
pub fn pull_percent(completed: Option<u64>, total: Option<u64>, last: u8) -> u8 {
    match (completed, total) {
        (Some(c), Some(t)) if t > 0 => {
            let pct = ((c as f64 / t as f64) * 100.0).round() as u64;
            pct.min(100) as u8
        }
        _ => last,
    }
}

/// Stream a model pull, invoking `on_event` for each progress update and once
/// more with `done: true` (and `error` set on failure). Never panics.
pub fn stream_pull(model: &str, mut on_event: impl FnMut(PullEvent)) {
    let emit = |percent: u8, status: String, done: bool, error: Option<String>| PullEvent {
        model: model.to_string(),
        percent,
        status,
        done,
        error,
    };

    let client = match reqwest::blocking::Client::builder()
        // A large pull can run for minutes; no overall timeout, only connect.
        .connect_timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            on_event(emit(0, String::new(), true, Some(e.to_string())));
            return;
        }
    };

    let resp = client
        .post(format!("{OLLAMA_URL}/api/pull"))
        .json(&serde_json::json!({ "name": model, "stream": true }))
        .send();

    let resp = match resp {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            on_event(emit(0, String::new(), true, Some(format!("Ollama returned {}", r.status()))));
            return;
        }
        Err(e) => {
            on_event(emit(0, String::new(), true, Some(format!("Ollama unreachable: {e}"))));
            return;
        }
    };

    let mut last = 0u8;
    let reader = BufReader::new(resp);
    for line in reader.lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(e) => {
                on_event(emit(last, String::new(), true, Some(e.to_string())));
                return;
            }
        };
        let parsed: PullLine = match serde_json::from_str(&line) {
            Ok(p) => p,
            Err(_) => continue, // skip a malformed line rather than abort
        };
        if let Some(err) = parsed.error {
            on_event(emit(last, String::new(), true, Some(err)));
            return;
        }
        last = pull_percent(parsed.completed, parsed.total, last);
        on_event(emit(last, parsed.status, false, None));
    }
    on_event(emit(100, "success".into(), true, None));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_from_counts() {
        assert_eq!(pull_percent(Some(50), Some(100), 0), 50);
        assert_eq!(pull_percent(Some(100), Some(100), 0), 100);
    }

    #[test]
    fn percent_clamps_and_carries_last() {
        assert_eq!(pull_percent(Some(200), Some(100), 0), 100); // clamp
        assert_eq!(pull_percent(None, Some(100), 42), 42); // status-only line
        assert_eq!(pull_percent(Some(0), Some(0), 7), 7); // zero total
    }
}

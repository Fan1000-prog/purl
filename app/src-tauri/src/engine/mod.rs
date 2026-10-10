//! On-device engines and the models they run on.

pub mod download;
pub mod models;
pub mod stt;

use models::{is_installed, model_dir, registry, Kind, ModelSpec};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Where an engine is in its lifecycle (`EngineState` in `src/types.ts`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineState {
    /// Its model isn't downloaded.
    Missing,
    Loading,
    Ready,
    /// Downloaded but not in memory; the next use loads it.
    Idle,
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EngineStatusView {
    pub stt: EngineState,
    pub llm: EngineState,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    Missing,
    Downloading,
    Ready,
    Failed,
}

/// One registry model as the UI sees it (`ModelView` in `src/types.ts`).
#[derive(Clone, Debug, Serialize)]
pub struct ModelView {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub size_bytes: u64,
    pub state: ModelState,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
}

/// Which models are downloading right now, and why the last attempt failed.
/// One download per model id at a time.
#[derive(Default)]
pub struct Downloads {
    /// Cancel flag per running download.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Last failure per model id, cleared when a new attempt starts.
    errors: Mutex<HashMap<String, String>>,
}

impl Downloads {
    /// Claim `id` for a new download. `None` when one is already running.
    pub fn try_start(&self, id: &str) -> Option<Arc<AtomicBool>> {
        let mut running = self.running.lock().unwrap();
        if running.contains_key(id) {
            return None;
        }
        self.errors.lock().unwrap().remove(id);
        let cancel = Arc::new(AtomicBool::new(false));
        running.insert(id.to_string(), cancel.clone());
        Some(cancel)
    }

    /// Release `id`, remembering `error` (if any) for `models_status`.
    pub fn finish(&self, id: &str, error: Option<String>) {
        if let Some(e) = error {
            self.errors.lock().unwrap().insert(id.to_string(), e);
        }
        self.running.lock().unwrap().remove(id);
    }

    /// Ask a running download to stop. No-op when none is running.
    pub fn cancel(&self, id: &str) {
        if let Some(flag) = self.running.lock().unwrap().get(id) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.lock().unwrap().contains_key(id)
    }

    /// Forget a remembered failure, e.g. after the model is deleted.
    pub fn clear_error(&self, id: &str) {
        self.errors.lock().unwrap().remove(id);
    }

    pub fn views(&self, app_data: &Path) -> Vec<ModelView> {
        registry()
            .iter()
            .map(|spec| self.view(app_data, spec))
            .collect()
    }

    fn view(&self, app_data: &Path, spec: &ModelSpec) -> ModelView {
        let error = self.errors.lock().unwrap().get(spec.id).cloned();
        let state = if self.is_running(spec.id) {
            ModelState::Downloading
        } else if is_installed(app_data, spec) {
            ModelState::Ready
        } else if error.is_some() {
            ModelState::Failed
        } else {
            ModelState::Missing
        };
        ModelView {
            id: spec.id,
            label: spec.label,
            kind: spec.kind,
            size_bytes: spec.size(),
            state,
            downloaded_bytes: bytes_on_disk(app_data, spec),
            error: if state == ModelState::Failed {
                error
            } else {
                None
            },
        }
    }
}

/// Bytes already fetched: finished files plus any `.part` in progress.
fn bytes_on_disk(app_data: &Path, spec: &ModelSpec) -> u64 {
    let dir = model_dir(app_data, spec.id);
    let len = |name: String| std::fs::metadata(dir.join(name)).map(|m| m.len()).ok();
    spec.files
        .iter()
        .map(|f| {
            len(f.name.to_string())
                .or_else(|| len(format!("{}.part", f.name)))
                .unwrap_or(0)
                .min(f.size)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_app_data(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("purl-engine-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn engine_state_serializes_as_tagged_object() {
        let json = serde_json::to_string(&EngineStatusView {
            stt: EngineState::Failed { reason: "x".into() },
            llm: EngineState::Missing,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"stt":{"state":"failed","reason":"x"},"llm":{"state":"missing"}}"#
        );
    }

    #[test]
    fn second_start_for_same_id_is_refused() {
        let d = Downloads::default();
        assert!(d.try_start("parakeet-v3").is_some());
        assert!(d.try_start("parakeet-v3").is_none());
        assert!(
            d.try_start("qwen3.5-2b").is_some(),
            "other ids are independent"
        );
        d.finish("parakeet-v3", None);
        assert!(d.try_start("parakeet-v3").is_some());
    }

    #[test]
    fn cancel_flips_the_running_flag() {
        let d = Downloads::default();
        let flag = d.try_start("parakeet-v3").unwrap();
        d.cancel("parakeet-v3");
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn views_cover_registry_with_states() {
        let app_data = empty_app_data("views");
        let d = Downloads::default();
        d.try_start("parakeet-v3");
        d.try_start("qwen3.5-2b");
        d.finish("qwen3.5-2b", Some("You're offline.".into()));

        let views = d.views(&app_data);
        let ids: Vec<_> = views.iter().map(|v| v.id).collect();
        assert_eq!(ids, ["parakeet-v3", "qwen3.5-2b", "qwen3.5-4b"]);
        assert_eq!(views[0].state, ModelState::Downloading);
        assert_eq!(views[1].state, ModelState::Failed);
        assert_eq!(views[1].error.as_deref(), Some("You're offline."));
        assert_eq!(views[2].state, ModelState::Missing);
        assert_eq!(views[2].error, None);
        assert_eq!(views[0].size_bytes, 670_619_803);
    }

    #[test]
    fn new_attempt_clears_old_error() {
        let app_data = empty_app_data("retry");
        let d = Downloads::default();
        d.try_start("qwen3.5-2b");
        d.finish("qwen3.5-2b", Some("boom".into()));
        d.try_start("qwen3.5-2b");
        d.finish("qwen3.5-2b", None);
        assert_eq!(d.views(&app_data)[1].state, ModelState::Missing);
    }

    #[test]
    fn downloaded_bytes_counts_part_files() {
        let app_data = empty_app_data("bytes");
        let dir = model_dir(&app_data, "parakeet-v3");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vocab.txt.part"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.join("config.json"), vec![0u8; 97]).unwrap();

        let view = Downloads::default().views(&app_data).remove(0);
        assert_eq!(view.downloaded_bytes, 1097);
        let _ = std::fs::remove_dir_all(&app_data);
    }
}

export interface HistoryEntry {
  ts: number;
  raw: string;
  cleaned: string;
  used_llm: boolean;
  record_ms: number;
  transcribe_ms: number;
  cleanup_ms: number;
  words: number;
}

export interface Stats {
  total_words: number;
  total_record_ms: number;
  total_sessions: number;
  llm_cleanups: number;
  per_day: Record<string, number>;
}

export interface Snippet {
  trigger: string;
  expansion: string;
}

export interface LearnedRule {
  from: string;
  to: string;
  before: string;
  after: string;
  created: number;
  hits: number;
  last_hit: number;
  /** Times this exact correction has been observed. */
  seen: number;
  /** False while a risky rule waits for its second sighting. */
  active: boolean;
}

// ---- Engine + model contract (Rust: src-tauri/src/engine) ----
// Commands: models_status(): ModelView[], download_model(id), cancel_download(id),
// delete_model(id), engine_status(): EngineStatusView.
// Events: "model-progress" ModelProgress, "model-done" ModelDone,
// "engine-status" EngineStatusView.

export type ModelKind = "stt" | "llm";
export type ModelState = "missing" | "downloading" | "ready" | "failed";
export interface ModelView {
  id: string;            // "parakeet-v3" | "qwen3.5-2b" | "qwen3.5-4b"
  label: string;         // "Parakeet v3 (speech)"
  kind: ModelKind;
  size_bytes: number;
  state: ModelState;
  downloaded_bytes: number;
  error: string | null;  // user-facing sentence when state === "failed"
}
export interface ModelProgress {
  id: string;
  downloaded_bytes: number;
  size_bytes: number;
}
export type DownloadFailure =
  | "disk_full" | "offline" | "http" | "checksum" | "cancelled" | "io" | "stopped";
export interface ModelDone {
  id: string;
  ok: boolean;
  error: string | null;
  kind: DownloadFailure | null; // null when ok
}
export type EngineState =
  | { state: "missing" } | { state: "loading" } | { state: "ready" }
  | { state: "idle" }    | { state: "failed"; reason: string };
export interface EngineStatusView { stt: EngineState; llm: EngineState; }
export type CleanupMode = "off" | "fast" | "smart";
export type Trust = "granted" | "denied" | "not_required";

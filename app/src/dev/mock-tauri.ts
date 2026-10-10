// Dev-only stand-in for the Rust side, so the UI runs in a plain browser:
// `npm run dev`, then open http://localhost:1420/?mock=1. It implements the
// shared command/event contract (models, downloads, engine status) plus
// enough of the older commands for every view to load.
//
// Never in a release: the install call below sits behind
// `import.meta.env.DEV`, which Vite replaces with `false` in `vite build`, so
// the whole mock is dropped as dead code.
//
// URL switches (all optional):
//   platform=windows        render as Windows (no Accessibility row)
//   mic=denied|granted      microphone permission (default granted)
//   ax=denied|granted       Accessibility permission on macOS (default granted)
//   fail=offline|diskfull|http   first download attempt fails this way
//   onboarded=1             skip onboarding (onboarding_complete = true)
//   models=ready            every model already downloaded
//   step_ms=<n>             ms between progress events (default 120)
// Tests also drive it through `window.__purlMock` (see MockControls).

import type { EngineStatusView, ModelView, Trust } from "../types";

type Handler = (event: { event: string; id: number; payload: unknown }) => void;

export interface MockControls {
  emit(event: string, payload: unknown): void;
  setTrust(next: { mic?: Trust; ax?: Trust }): void;
  /** Simulates one dictation: recording → processing → result → idle. */
  dictate(text: string): void;
  /** Commands invoked so far, in order. */
  calls: { cmd: string; args: unknown }[];
}

declare global {
  interface Window {
    __purlMock?: MockControls;
    __PURL_PLATFORM__?: "macos" | "windows";
  }
}

const ERRORS = {
  offline: "You're offline. Connect to the internet and press Retry.",
  diskfull: "Not enough free space: need 720 MB, have 100 MB.",
  http: "The download server said 503. Press Retry in a minute.",
  cancelled: "Download cancelled.",
} as const;

function install() {
  const q = new URLSearchParams(location.search);
  const stepMs = Number(q.get("step_ms") ?? 120);
  const allReady = q.get("models") === "ready";
  let pendingFailure = q.get("fail") as keyof typeof ERRORS | null;
  window.__PURL_PLATFORM__ = q.get("platform") === "windows" ? "windows" : "macos";

  const trust = {
    mic: (q.get("mic") as Trust | null) ?? "granted",
    ax: window.__PURL_PLATFORM__ === "windows" ? "not_required" : ((q.get("ax") as Trust | null) ?? "granted"),
  };

  const model = (id: string, label: string, size: number, kind: "stt" | "llm"): ModelView => ({
    id,
    label,
    kind,
    size_bytes: size,
    state: allReady ? "ready" : "missing",
    downloaded_bytes: allReady ? size : 0,
    error: null,
  });
  const models: ModelView[] = [
    model("parakeet-v3", "Parakeet v3 (speech)", 670_590_000, "stt"),
    model("qwen3.5-2b", "Qwen3.5 2B (smart cleanup)", 1_396_198_496, "llm"),
    model("qwen3.5-4b", "Qwen3.5 4B (smart cleanup, larger)", 3_013_027_808, "llm"),
  ];
  const timers = new Map<string, ReturnType<typeof setInterval>>();
  const engine: EngineStatusView = {
    stt: allReady ? { state: "ready" } : { state: "missing" },
    llm: { state: "idle" },
  };

  const settings: Record<string, unknown> = {
    hotkey: "CmdOrCtrl+Shift+Space",
    inject_mode: "paste",
    cleanup_mode: "fast",
    structure_mode: "none",
    command_detection: true,
    learn_corrections: false,
    onboarding_complete: q.get("onboarded") === "1",
    field_reading_disclosed: false,
    last_seen_version: "0.9.0",
  };

  const callbacks = new Map<number, Handler>();
  const listeners = new Map<string, number[]>();
  const calls: MockControls["calls"] = [];
  let nextId = 1;

  const emit = (event: string, payload: unknown) => {
    for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
  };

  const setEngine = (stt: EngineStatusView["stt"]) => {
    engine.stt = stt;
    emit("engine-status", { ...engine });
  };

  const finish = (m: ModelView, error: string | null) => {
    clearInterval(timers.get(m.id));
    timers.delete(m.id);
    m.state = error === null ? "ready" : error === ERRORS.cancelled ? "missing" : "failed";
    m.error = error === ERRORS.cancelled ? null : error;
    emit("model-done", { id: m.id, ok: error === null, error });
    if (error === null && m.kind === "stt") {
      setEngine({ state: "loading" });
      setTimeout(() => setEngine({ state: "ready" }), 600);
    }
  };

  const startDownload = (id: string) => {
    const m = models.find((x) => x.id === id);
    if (!m) return Promise.reject(`Unknown model ${id}`);
    if (timers.has(id)) return Promise.resolve(null);
    m.state = "downloading";
    m.error = null;
    const failure = pendingFailure;
    pendingFailure = null;
    if (failure === "diskfull") {
      setTimeout(() => finish(m, ERRORS.diskfull), 50);
      return Promise.resolve(null);
    }
    const chunk = Math.ceil(m.size_bytes / 25);
    timers.set(
      id,
      setInterval(() => {
        m.downloaded_bytes = Math.min(m.size_bytes, m.downloaded_bytes + chunk);
        emit("model-progress", { id, downloaded_bytes: m.downloaded_bytes, size_bytes: m.size_bytes });
        if (failure && m.downloaded_bytes > m.size_bytes * 0.3) finish(m, ERRORS[failure]);
        else if (m.downloaded_bytes >= m.size_bytes) finish(m, null);
      }, stepMs),
    );
    return Promise.resolve(null);
  };

  const status = () => ({
    whisper_bin_found: true,
    whisper_model_found: engine.stt.state === "ready",
    whisper_model_path: "",
    paste_trust: trust.ax,
    learning_supported: window.__PURL_PLATFORM__ !== "windows",
    hotkey: settings.hotkey,
    state: "idle",
  });

  const commands: Record<string, (args: any) => unknown> = {
    models_status: () => models.map((m) => ({ ...m })),
    download_model: ({ id }) => startDownload(id),
    cancel_download: ({ id }) => {
      const m = models.find((x) => x.id === id);
      if (m && timers.has(id)) finish(m, ERRORS.cancelled);
      return null;
    },
    delete_model: ({ id }) => {
      const m = models.find((x) => x.id === id);
      if (m) Object.assign(m, { state: "missing", downloaded_bytes: 0, error: null });
      return null;
    },
    engine_status: () => ({ ...engine }),
    mic_status: () => trust.mic,
    get_status: status,
    get_settings: () => ({ ...settings }),
    save_settings: ({ newSettings }) => Object.assign(settings, newSettings) && null,
    get_history: () => [],
    get_stats: () => ({ total_words: 0, total_record_ms: 0, total_sessions: 0, llm_cleanups: 0, per_day: {} }),
    get_snippets: () => [],
    get_transforms: () => [],
    get_learned_rules: () => [],
    get_glossary: () => "",
    get_last_result: () => null,
    toggle_recording: () => null,
    "plugin:app|version": () => "0.9.0",
    "plugin:updater|check": () => null,
  };

  (window as any).__TAURI_INTERNALS__ = {
    transformCallback(cb: Handler) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    invoke(cmd: string, args: any) {
      if (cmd === "plugin:event|listen") {
        listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
        return Promise.resolve(nextId++);
      }
      if (cmd === "plugin:event|unlisten") return Promise.resolve(null);
      calls.push({ cmd, args });
      const handler = commands[cmd];
      return handler ? Promise.resolve(handler(args ?? {})) : Promise.resolve(null);
    },
  };

  window.__purlMock = {
    emit,
    calls,
    setTrust(next) {
      Object.assign(trust, next);
    },
    dictate(text) {
      emit("state", "recording");
      setTimeout(() => emit("state", "processing"), 150);
      setTimeout(() => {
        emit("result", { raw: text, cleaned: text, used_llm: false, note: null, transcribe_ms: 200, cleanup_ms: 1 });
        emit("state", "idle");
      }, 300);
    },
  };
}

if (import.meta.env.DEV && new URLSearchParams(location.search).get("mock") === "1") install();

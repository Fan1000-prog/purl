// Speech-engine state for the UI. Prefers the engine's own report
// (`engine_status` / `engine-status`); builds without it fall back to whether
// the speech model is on disk, so Home can still say "download the model".

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { EngineState, EngineStatusView, ModelDone, ModelView } from "./types";

export const SPEECH_MODEL = "parakeet-v3";

async function fromModels(): Promise<EngineState> {
  const models = await invoke<ModelView[]>("models_status");
  return models.find((m) => m.id === SPEECH_MODEL)?.state === "ready" ? { state: "ready" } : { state: "missing" };
}

/** Calls `onChange` with the speech engine's state now and on every change. */
export function watchSpeechEngine(onChange: (stt: EngineState) => void): void {
  let reported = false;
  void listen<EngineStatusView>("engine-status", (e) => {
    reported = true;
    onChange(e.payload.stt);
  });
  // Without engine events, a finished speech download is the only change.
  void listen<ModelDone>("model-done", (e) => {
    if (!reported && e.payload.id === SPEECH_MODEL) void fromModels().then(onChange, () => {});
  });
  void invoke<EngineStatusView>("engine_status").then(
    (v) => {
      reported = true;
      onChange(v.stt);
    },
    () => void fromModels().then(onChange, (err) => onChange({ state: "failed", reason: String(err) })),
  );
}

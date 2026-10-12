// App status from `get_status`, normalized across Rust versions: the platform
// layer reports `paste_trust` and `learning_supported`; older builds only had
// `accessibility_trusted`. Engine and model state come from engine.ts.

import { invoke } from "@tauri-apps/api/core";
import { platform } from "./platform";
import type { Trust } from "./types";

interface RawStatus {
  paste_trust?: Trust;
  learning_supported?: boolean;
  accessibility_trusted?: boolean;
  hotkey: string;
  state: string;
}

export interface AppStatus {
  pasteTrust: Trust;
  learningSupported: boolean;
  hotkey: string;
  state: string;
}

export function normalizeStatus(raw: RawStatus): AppStatus {
  const legacyTrust: Trust =
    platform() === "windows" ? "not_required" : raw.accessibility_trusted === false ? "denied" : "granted";
  return {
    pasteTrust: raw.paste_trust ?? legacyTrust,
    learningSupported: raw.learning_supported ?? platform() === "macos",
    hotkey: raw.hotkey,
    state: raw.state,
  };
}

export async function getStatus(): Promise<AppStatus> {
  return normalizeStatus(await invoke<RawStatus>("get_status"));
}

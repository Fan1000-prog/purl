// Must stay first: installs the dev-only Tauri mock before anything calls it.
import "./dev/mock-tauri";
import { displayHotkey as prettyHotkey } from "./platform";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { initHome } from "./views/home";
import { initDictionary } from "./views/dictionary";
import { initSnippets } from "./views/snippets";
import { initTransforms } from "./views/transforms";
import { initSettings, showSettingsUnavailable } from "./views/settings";
import { initOnboarding } from "./views/onboarding";
import { initFieldNotice } from "./views/field-notice";
import { initUpdates } from "./views/updates";
import { HOTKEY_RECORDER_CANCEL } from "./views/hotkey-recorder";
import { celebrate } from "./celebrate";
import type { CleanupMode } from "./types";

export interface Settings {
  hotkey: string;
  inject_mode: "paste" | "clipboard_only";
  cleanup_mode: CleanupMode;
  smart_model: string;
  structure_mode: "none" | "email" | "coding";
  command_detection: boolean;
  learn_corrections: boolean;
  onboarding_complete: boolean;
  field_reading_disclosed: boolean;
  last_seen_version: string;
}

let currentSettings: Settings | null = null;

/** Last-loaded settings; views read hotkey/cleanup_mode from here. */
export function getSettings(): Settings | null {
  return currentSettings;
}

/** The recorder owns hotkey persistence; it syncs the cached copy here. */
export function setCurrentHotkey(hk: string): void {
  if (currentSettings) currentSettings.hotkey = hk;
}

/** Change some settings. Read-modify-write against the backend, so a stale
 *  cached copy can never clobber a setting changed elsewhere. */
export async function saveSettings(patch: Partial<Settings>): Promise<Settings> {
  const s = { ...(await invoke<Settings>("get_settings")), ...patch };
  await invoke("save_settings", { newSettings: s });
  currentSettings = s;
  return s;
}

/** Persist onboarding_complete=true, and stamp the version: a "what's new"
 *  card for the only version they have ever run is noise. `disclosed` is
 *  true only when setup showed the field-reading disclosure (macOS); then the
 *  one-time notice would repeat it, so it is marked seen. Otherwise the
 *  notice still owes it. */
export async function saveOnboardingComplete(disclosed: boolean): Promise<void> {
  await saveSettings({
    onboarding_complete: true,
    ...(disclosed ? { field_reading_disclosed: true } : {}),
    last_seen_version: await getVersion().catch(() => ""),
  });
}

/** Mark this version's "what's new" card as seen, and the field-reading
 *  disclosure with it: the card states it. */
export async function saveVersionSeen(version: string): Promise<void> {
  await saveSettings({ last_seen_version: version, field_reading_disclosed: true });
}

export interface TranscriptPayload {
  raw: string;
  cleaned: string;
  used_llm: boolean;
  note: string | null;
  transcribe_ms: number;
  cleanup_ms: number;
}

const refreshers: Record<string, (() => void) | undefined> = {};

/** Views register a refresh function; called on nav click and on new results. */
export function onViewShow(view: string, fn: () => void) {
  refreshers[view] = fn;
}

export function switchView(view: string) {
  // Leaving a view must not strand an armed hotkey recorder.
  document.dispatchEvent(new Event(HOTKEY_RECORDER_CANCEL));
  document.getElementById("content")!.dataset.view = view;
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    const active = b.dataset.view === view;
    b.classList.toggle("active", active);
    if (active) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  });
  refreshers[view]?.();
}

/// "CmdOrCtrl+Shift+Space" → "⌘⇧Space" (or "Ctrl+Shift+Space" on Windows).
export { prettyHotkey };

window.addEventListener("DOMContentLoaded", () => {
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.view ?? "home"));
  });

  initHome();
  initDictionary();
  initSnippets();
  initTransforms();
  initUpdates();

  void invoke<Settings>("get_settings")
    .then((s) => {
      currentSettings = s;
      initSettings(s);
      initOnboarding();
      initFieldNotice();
    })
    .catch(showSettingsUnavailable);

  void listen<TranscriptPayload>("result", () => {
    refreshers["home"]?.();
    // `result` only fires on a successful run; failures arrive as
    // `pipeline-error`. Not conditioned on `used_llm`: falling back to Fast
    // cleanup still delivered the user's words.
    celebrate("Your words are ready.");
  });
  void listen<string>("transform-result", (e) => {
    celebrate(`"${e.payload}" applied. Your text is ready.`);
  });
});

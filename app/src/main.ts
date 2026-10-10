// Must stay first: installs the dev-only Tauri mock before anything calls it.
import "./dev/mock-tauri";
import { displayHotkey as prettyHotkey } from "./platform";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { initHome } from "./views/home";
import { initInsights } from "./views/insights";
import { initSnippets } from "./views/snippets";
import { initLearning } from "./views/learning";
import { initTransforms } from "./views/transforms";
import { initOnboarding } from "./views/onboarding";
import { initFieldNotice } from "./views/field-notice";
import { initUpdates } from "./views/updates";
import { createHotkeyRecorder, HOTKEY_RECORDER_CANCEL } from "./views/hotkey-recorder";
import { initModelPicker } from "./views/model-picker";
import { celebrate } from "./celebrate";
import { makeToken, providerLabel, splitToken } from "./engine-token";

export interface Settings {
  hotkey: string;
  ollama_model: string;
  ai_engine: "local" | "claude" | "gemini";
  claude_model: string;
  gemini_model: string;
  whisper_model_path: string;
  inject_mode: "paste" | "clipboard_only";
  cleanup_mode: "fast" | "ai" | "off";
  structure_mode: "none" | "email" | "coding";
  command_detection: boolean;
  learn_corrections: boolean;
  onboarding_complete: boolean;
  field_reading_disclosed: boolean;
  last_seen_version: string;
  model_migrated: boolean;
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

/** Persist onboarding_complete=true without disturbing other settings.
 *  `disclosed` is true only when setup showed the field-reading disclosure
 *  (macOS); then the one-time notice would repeat it, so it is marked seen.
 *  Otherwise the notice still owes it. */
export async function saveOnboardingComplete(disclosed: boolean): Promise<void> {
  const s = await invoke<Settings>("get_settings");
  s.onboarding_complete = true;
  if (disclosed) s.field_reading_disclosed = true;
  // Stamp the current version too: someone who just finished setup has
  // read everything the "what's new" card would tell them, and a changelog
  // for the only version they have ever run is noise.
  s.last_seen_version = await getVersion().catch(() => "");
  await invoke("save_settings", { newSettings: s });
  if (currentSettings) {
    currentSettings.onboarding_complete = true;
    currentSettings.field_reading_disclosed = s.field_reading_disclosed;
    currentSettings.last_seen_version = s.last_seen_version;
  }
}

/** Persist field_reading_disclosed=true without disturbing other settings. */
export async function saveFieldReadingDisclosed(): Promise<void> {
  const s = await invoke<Settings>("get_settings");
  s.field_reading_disclosed = true;
  await invoke("save_settings", { newSettings: s });
  if (currentSettings) currentSettings.field_reading_disclosed = true;
}

/** Mark this version's "what's new" card as seen, and the field-reading
 *  disclosure with it — the card states it, so the separate notice would be
 *  telling the user the same thing twice. Read-modify-write so a stale cached
 *  copy cannot clobber a setting changed elsewhere. */
export async function saveVersionSeen(version: string): Promise<void> {
  const s = await invoke<Settings>("get_settings");
  s.last_seen_version = version;
  s.field_reading_disclosed = true;
  await invoke("save_settings", { newSettings: s });
  if (currentSettings) {
    currentSettings.last_seen_version = version;
    currentSettings.field_reading_disclosed = true;
  }
}

export interface TranscriptPayload {
  raw: string;
  cleaned: string;
  used_llm: boolean;
  note: string | null;
  transcribe_ms: number;
  cleanup_ms: number;
}

interface StatusPayload {
  whisper_bin_found: boolean;
  whisper_model_found: boolean;
  whisper_model_path: string;
  ollama_up: boolean;
  ollama_model_found: boolean;
  ollama_model: string;
  ai_engine: "local" | "claude" | "gemini";
  accessibility_trusted: boolean;
  hotkey: string;
  state: string;
}

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const refreshers: Record<string, (() => void) | undefined> = {};

/** Views register a refresh function; called on nav click and on new results. */
export function onViewShow(view: string, fn: () => void) {
  refreshers[view] = fn;
}

export function switchView(view: string) {
  // Leaving a view must not strand an armed hotkey recorder.
  document.dispatchEvent(new Event(HOTKEY_RECORDER_CANCEL));
  $<HTMLElement>("content").dataset.view = view;
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === view);
  });
  refreshers[view]?.();
}

/** Settings is one view with four sub-sections routed by the gear flyout. */
export function switchSettingsSection(section: string) {
  $<HTMLElement>("view-settings").dataset.section = section;
  document.querySelectorAll<HTMLButtonElement>(".flyout-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.settingsSection === section);
  });
}

const toggleBtn = $<HTMLButtonElement>("toggle-btn");
const note = $<HTMLParagraphElement>("note");

function setState(state: string) {
  $<HTMLSpanElement>("toggle-label").textContent =
    state === "recording"
      ? "Stop & transcribe"
      : state === "processing"
        ? "Transcribing…"
        : "Start dictation";
  toggleBtn.disabled = state === "processing";
  toggleBtn.classList.toggle("is-recording", state === "recording");
  toggleBtn.classList.toggle("is-processing", state === "processing");
}

function checkItem(ok: boolean, label: string, fix?: string): HTMLLIElement {
  const li = document.createElement("li");
  li.className = ok ? "ok" : "bad";
  const status = document.createElement("span");
  status.textContent = "●";
  status.className = "mark";
  li.appendChild(status);
  li.appendChild(document.createTextNode(` ${label}`));
  if (!ok && fix) {
    const hint = document.createElement("div");
    hint.className = "fix";
    hint.textContent = fix;
    li.appendChild(hint);
  }
  return li;
}

/// "CmdOrCtrl+Shift+Space" → "⌘⇧Space" for the kbd chip.
export { prettyHotkey };

async function refreshStatus() {
  const s = await invoke<StatusPayload>("get_status");
  setState(s.state);
  $<HTMLElement>("hotkey-label").textContent = prettyHotkey(s.hotkey);

  const list = $<HTMLUListElement>("check-items");
  list.replaceChildren(
    checkItem(
      s.whisper_bin_found,
      "Speech engine (whisper-cli)",
      "Not found. In Terminal, run: brew install whisper-cpp",
    ),
    checkItem(
      s.whisper_model_found,
      "Speech model downloaded",
      `Missing — expected at ${s.whisper_model_path}. The install script downloads it, or see the README.`,
    ),
    // Green for cloud engines so a working setup shows no false alarm — but the
    // label must still be true. Ollama is the fallback rung on every cloud
    // failure, so if it is down we say so rather than claiming it is running.
    checkItem(
      s.ollama_up || s.ai_engine !== "local",
      s.ai_engine === "local"
        ? "Ollama running (only needed for AI cleanup)"
        : s.ollama_up
          ? `Ollama running (fallback for ${providerLabel(s.ai_engine)} cleanup)`
          : `Ollama not running — no local fallback if ${providerLabel(s.ai_engine)} fails`,
      "Open the Ollama app, or run: ollama serve",
    ),
    checkItem(
      s.ollama_model_found || s.ai_engine !== "local",
      s.ai_engine === "local"
        ? `AI cleanup model (${s.ollama_model})`
        : s.ollama_model_found
          ? `Local fallback model (${s.ollama_model})`
          : `Local fallback model missing (${s.ollama_model})`,
      `One-time download: ollama pull ${s.ollama_model}`,
    ),
    checkItem(
      s.accessibility_trusted,
      "Accessibility permission — lets it type for you, and read the field it typed into",
      "System Settings → Privacy & Security → Accessibility → turn on Purl. Until then, transcripts go to your clipboard — paste with ⌘V. The same grant lets it read that field back to catch words you fix (Settings → Learn corrections).",
    ),
  );
}

async function loadSettings(): Promise<Settings> {
  const s = await invoke<Settings>("get_settings");
  currentSettings = s;
  const modelSelect = $<HTMLSelectElement>("cleanup-model");
  const token = makeToken(
    s.ai_engine,
    s.ai_engine === "local"
      ? s.ollama_model
      : s.ai_engine === "claude"
        ? s.claude_model
        : s.gemini_model,
  );
  // An id not in the picker — a hand-edited settings.json, or a model this
  // release dropped — becomes a real option rather than being snapped to a
  // default. Cloud ids are validated permissively precisely so a user can
  // point at a newer id than this build ships; silently selecting something
  // else would mean that merely opening Settings and pressing Save discards
  // their engine and their local model choice with no warning. Surfacing it
  // keeps the save path a faithful round-trip.
  const known = [...modelSelect.options].some((o) => o.value === token);
  if (!known) {
    const opt = document.createElement("option");
    opt.value = token;
    const shown = splitToken(token).model;
    opt.textContent = `${shown} (${s.ai_engine === "local" ? "local" : "cloud"}, from settings.json)`;
    modelSelect.appendChild(opt);
  }
  modelSelect.value = token;
  $<HTMLSelectElement>("cleanup-mode").value = s.cleanup_mode;
  $<HTMLSelectElement>("inject-mode").value = s.inject_mode;
  $<HTMLSelectElement>("structure-mode").value = s.structure_mode;
  $<HTMLInputElement>("command-detection").checked = s.command_detection;
  $<HTMLInputElement>("learn-corrections").checked = s.learn_corrections;
  return s;
}

/** Settings failed to load. The controls still hold their HTML defaults,
 *  and for "Learn corrections" that default reads as OFF while the backend
 *  may well have it ON — the app would be showing "not watching" while it
 *  watches. Say so, leave the privacy toggle visibly indeterminate rather
 *  than falsely unchecked, and block Save so a stale form cannot be written
 *  back over the real settings. */
function showSettingsUnavailable(err: unknown) {
  $<HTMLSpanElement>("settings-msg").textContent =
    `Couldn't load your settings (${String(err)}). What's shown below may not match what's saved — reopen the app before changing anything.`;
  const learn = $<HTMLInputElement>("learn-corrections");
  learn.indeterminate = true;
  learn.disabled = true;
  $<HTMLInputElement>("command-detection").disabled = true;
  document
    .querySelectorAll<HTMLButtonElement>("#settings-form button[type=submit]")
    .forEach((b) => (b.disabled = true));
}

async function loadGlossary() {
  $<HTMLTextAreaElement>("glossary-text").value =
    await invoke<string>("get_glossary");
}

window.addEventListener("DOMContentLoaded", () => {
  void refreshStatus();
  void loadSettings()
    .then(() => {
      // Recorder reads the current hotkey from settings — mount after load.
      $<HTMLElement>("hotkey-recorder").appendChild(
        createHotkeyRecorder({ onSaved: () => void refreshStatus() }),
      );
      initOnboarding();
      initFieldNotice();
      initModelPicker(
        $<HTMLSelectElement>("cleanup-model"),
        $<HTMLElement>("cleanup-model-status"),
      );
    })
    .catch(showSettingsUnavailable);
  void loadGlossary();

  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.view ?? "home"));
  });

  switchSettingsSection("setup");
  document.querySelectorAll<HTMLButtonElement>(".nav-settings, .flyout-item").forEach((b) => {
    b.addEventListener("click", () => {
      switchSettingsSection(b.dataset.settingsSection ?? "setup");
      switchView("settings");
    });
  });

  initHome();
  initInsights();
  initSnippets();
  initTransforms();
  initUpdates();
  initLearning();

  // Permissions/Ollama can change outside the app; poll cheaply.
  setInterval(() => void refreshStatus(), 5000);

  void listen<string>("state", (e) => setState(e.payload));
  void listen<TranscriptPayload>("result", (e) => {
    $<HTMLParagraphElement>("note").textContent = e.payload.note ?? "";
    refreshers["home"]?.();
    refreshers["insights"]?.();
    // Same celebration snippets and transforms get. `result` only fires on a
    // successful run — an empty transcript or a pipeline failure goes out as
    // `pipeline-error` instead — so this cannot fire on a failed dictation.
    // Deliberately not conditioned on `used_llm`: falling back to the local
    // model, or to the raw transcript, still delivered the user's words.
    celebrate("Your words are ready.");
  });
  void listen<string>("pipeline-error", (e) => {
    note.textContent = e.payload;
  });
  void listen<string>("transform-result", (e) => {
    celebrate(`"${e.payload}" applied — your text is ready.`);
  });

  toggleBtn.addEventListener("click", () => void invoke("toggle_recording"));

  $<HTMLFormElement>("settings-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const msg = $<HTMLSpanElement>("settings-msg");
    // Per-engine fields: writing only the selected engine's model preserves
    // the others, so switching back restores the previous choice.
    const selected = splitToken($<HTMLSelectElement>("cleanup-model").value);
    const newSettings: Settings = {
      // The recorder owns the hotkey via set_hotkey; pass through unchanged.
      hotkey: currentSettings?.hotkey ?? "CmdOrCtrl+Shift+Space",
      ollama_model:
        selected.engine === "local"
          ? selected.model
          : (currentSettings?.ollama_model ?? "qwen2.5:3b"),
      ai_engine: selected.engine,
      claude_model:
        selected.engine === "claude"
          ? selected.model
          : (currentSettings?.claude_model ?? "claude-haiku-4-5-20251001"),
      gemini_model:
        selected.engine === "gemini"
          ? selected.model
          : (currentSettings?.gemini_model ?? "gemini-3.5-flash-lite"),
      whisper_model_path: "",
      inject_mode: $<HTMLSelectElement>("inject-mode").value as
        | "paste"
        | "clipboard_only",
      cleanup_mode: $<HTMLSelectElement>("cleanup-mode").value as
        | "fast"
        | "ai"
        | "off",
      structure_mode: $<HTMLSelectElement>("structure-mode").value as
        | "none"
        | "email"
        | "coding",
      command_detection: $<HTMLInputElement>("command-detection").checked,
      learn_corrections: $<HTMLInputElement>("learn-corrections").checked,
      onboarding_complete: currentSettings?.onboarding_complete ?? false,
      field_reading_disclosed: currentSettings?.field_reading_disclosed ?? false,
      last_seen_version: currentSettings?.last_seen_version ?? "",
      model_migrated: currentSettings?.model_migrated ?? true,
    };
    invoke("save_settings", { newSettings })
      .then(() => {
        currentSettings = newSettings;
        msg.textContent = "Saved ✓";
        void refreshStatus();
      })
      .catch((err) => {
        msg.textContent = String(err);
      });
  });

  $<HTMLButtonElement>("glossary-save").addEventListener("click", () => {
    const msg = $<HTMLSpanElement>("glossary-msg");
    invoke("save_glossary", {
      content: $<HTMLTextAreaElement>("glossary-text").value,
    })
      .then(() => (msg.textContent = "Saved ✓"))
      .catch((err) => (msg.textContent = String(err)));
  });
});

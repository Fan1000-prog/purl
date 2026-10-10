// Settings: every control saves on change (read-modify-write through
// saveSettings). Smart cleanup only switches on once its model is on disk;
// until then the choice shows the same download card as onboarding.

import { invoke } from "@tauri-apps/api/core";
import { getSettings, saveSettings, onViewShow, type Settings } from "../main";
import { createHotkeyRecorder } from "./hotkey-recorder";
import { mountModelDownload, formatBytes, type ModelDownload } from "../components/model-download";
import { confirmButton } from "../components/confirm-button";
import { getStatus } from "../status";
import type { CleanupMode, ModelView } from "../types";

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

const CLEANUP_HELP: Record<CleanupMode, string> = {
  off: "Your words exactly as heard.",
  fast: "Fixes punctuation, capitals and filler words. Instant.",
  smart: "A small language model on this computer also fixes grammar and phrasing. Adds a second or two.",
};
const SMART_PENDING = "Smart turns on when the download finishes. Until then, Fast cleanup is used.";

let smartDownload: ModelDownload | null = null;
let shownMode: CleanupMode = "fast";

function showError(text: string) {
  const el = $("settings-msg");
  el.textContent = text;
  el.classList.toggle("hidden", text === "");
}

async function save(patch: Partial<Settings>): Promise<boolean> {
  try {
    await saveSettings(patch);
    showError("");
    return true;
  } catch (err) {
    showError(`Couldn't save that change (${String(err)}).`);
    return false;
  }
}

function setSegmented(group: HTMLElement, value: string) {
  group.querySelectorAll<HTMLButtonElement>("button").forEach((b) => {
    b.setAttribute("aria-checked", String(b.dataset.value === value));
  });
}

function onSegmented(group: HTMLElement, handler: (value: string) => void) {
  group.querySelectorAll<HTMLButtonElement>("button").forEach((b) => {
    b.addEventListener("click", () => handler(b.dataset.value!));
  });
}

// ---------- cleanup ----------

async function smartModelReady(id: string): Promise<boolean> {
  const models = await invoke<ModelView[]>("models_status").catch(() => []);
  return models.find((m) => m.id === id)?.state === "ready";
}

function renderCleanup(mode: CleanupMode, pending = false) {
  shownMode = mode;
  setSegmented($("cleanup-mode"), mode);
  $("cleanup-help").textContent = pending ? SMART_PENDING : CLEANUP_HELP[mode];
  $("smart-setup").classList.toggle("hidden", mode !== "smart");
}

function mountSmartDownload(id: string) {
  smartDownload?.destroy();
  smartDownload = mountModelDownload($("smart-download"), {
    id,
    title: id === "qwen3.5-4b" ? "Smart cleanup, larger model" : "Smart cleanup",
    detail: id === "qwen3.5-4b" ? "better on long dictation, slower" : "recommended",
    onReady: () => {
      // Picking Smart before the model existed parked the choice; apply it now.
      if (shownMode === "smart" && getSettings()?.cleanup_mode !== "smart") {
        void save({ cleanup_mode: "smart" }).then(() => renderCleanup("smart"));
      }
    },
  });
}

async function chooseCleanup(mode: CleanupMode) {
  const s = getSettings();
  if (!s) return;
  if (mode === "smart") {
    mountSmartDownload(s.smart_model);
    if (!(await smartModelReady(s.smart_model))) {
      renderCleanup("smart", true);
      return;
    }
  }
  if (await save({ cleanup_mode: mode })) renderCleanup(mode);
  else renderCleanup(s.cleanup_mode);
}

async function chooseSmartModel(id: string) {
  if (!(await save({ smart_model: id }))) return;
  setSegmented($("smart-model"), id);
  mountSmartDownload(id);
  // A switch to a model that isn't downloaded falls back like a first pick.
  if (getSettings()?.cleanup_mode === "smart" && !(await smartModelReady(id))) {
    if (await save({ cleanup_mode: "fast" })) renderCleanup("smart", true);
  }
}

// ---------- models ----------

function modelRow(m: ModelView): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "list-item list-row";
  const name = document.createElement("span");
  name.className = "item-main";
  name.textContent = m.label;
  const meta = document.createElement("span");
  meta.className = "muted";
  const state = { ready: "downloaded", missing: "not downloaded", downloading: "downloading", failed: "failed" }[m.state];
  meta.textContent = `${formatBytes(m.size_bytes)} · ${state}`;
  li.append(name, meta);
  if (m.state === "ready" || (m.state === "failed" && m.downloaded_bytes > 0)) {
    const del = document.createElement("button");
    del.type = "button";
    del.className = "btn btn-quiet";
    del.textContent = "Delete";
    del.setAttribute("aria-label", `Delete ${m.label}`);
    confirmButton(del, "Delete? Re-download any time.", () => {
      invoke("delete_model", { id: m.id })
        .then(refreshModels)
        .catch((err) => showModelsError(`Couldn't delete ${m.label} (${String(err)}).`));
    });
    li.append(del);
  }
  return li;
}

function showModelsError(text: string) {
  const el = $("models-error");
  el.textContent = text;
  el.classList.toggle("hidden", text === "");
}

async function refreshModels() {
  try {
    const models = await invoke<ModelView[]>("models_status");
    $("models-list").replaceChildren(...models.map(modelRow));
    showModelsError("");
    // Deleting the Smart model while Smart is on would leave nothing to run.
    const s = getSettings();
    if (s?.cleanup_mode === "smart" && models.find((m) => m.id === s.smart_model)?.state !== "ready") {
      if (await save({ cleanup_mode: "fast" })) {
        renderCleanup("smart", true);
        mountSmartDownload(s.smart_model);
      }
    }
  } catch (err) {
    $("models-list").replaceChildren();
    showModelsError(`Couldn't list models (${String(err)}).`);
  }
}

// ---------- init ----------

function bindSelect(id: string, key: "inject_mode" | "structure_mode") {
  const el = $<HTMLSelectElement>(id);
  el.addEventListener("change", () => void save({ [key]: el.value } as Partial<Settings>));
}

function bindToggle(id: string, key: "command_detection" | "learn_corrections") {
  const el = $<HTMLInputElement>(id);
  el.addEventListener("change", () => {
    void save({ [key]: el.checked } as Partial<Settings>).then((ok) => {
      if (!ok) el.checked = !el.checked;
    });
  });
}

export function initSettings(s: Settings) {
  $("hotkey-recorder").appendChild(createHotkeyRecorder());
  $<HTMLSelectElement>("inject-mode").value = s.inject_mode;
  $<HTMLSelectElement>("structure-mode").value = s.structure_mode;
  $<HTMLInputElement>("command-detection").checked = s.command_detection;
  $<HTMLInputElement>("learn-corrections").checked = s.learn_corrections;
  bindSelect("inject-mode", "inject_mode");
  bindSelect("structure-mode", "structure_mode");
  bindToggle("command-detection", "command_detection");
  bindToggle("learn-corrections", "learn_corrections");

  setSegmented($("smart-model"), s.smart_model);
  renderCleanup(s.cleanup_mode);
  if (s.cleanup_mode === "smart") mountSmartDownload(s.smart_model);
  onSegmented($("cleanup-mode"), (v) => void chooseCleanup(v as CleanupMode));
  onSegmented($("smart-model"), (v) => void chooseSmartModel(v));

  void getStatus().then(
    (st) => {
      if (st.learningSupported) return;
      const learn = $<HTMLInputElement>("learn-corrections");
      learn.disabled = true;
      learn.checked = false;
      $("learn-help").textContent = "Coming to Windows soon.";
    },
    () => {},
  );

  onViewShow("settings", () => void refreshModels());
  void refreshModels();
}

/** Settings failed to load. The controls hold their HTML defaults, and for
 *  "Learn corrections" that default reads as OFF while the backend may have
 *  it ON. Say so, leave that toggle visibly indeterminate, and disable every
 *  control so a stale form can't be written over the real settings. */
export function showSettingsUnavailable(err: unknown) {
  showError(`Couldn't load your settings (${String(err)}). What's shown may not match what's saved. Reopen Purl before changing anything.`);
  $<HTMLInputElement>("learn-corrections").indeterminate = true;
  $("view-settings")
    .querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLButtonElement>("input, select, .segmented button")
    .forEach((el) => (el.disabled = true));
}

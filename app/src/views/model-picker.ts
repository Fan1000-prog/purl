import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { splitToken } from "../engine-token";
import { ensureKey, renderCloudRow } from "./cloud-key";

interface InstalledModels {
  up: boolean;
  installed: string[];
}

interface PullEvent {
  model: string;
  percent: number;
  status: string;
  done: boolean;
  error: string | null;
}

/** Same match rule as the Rust side. */
function modelInstalled(installed: string[], model: string): boolean {
  return installed.some((m) => m === model || m.startsWith(model + ":"));
}

/**
 * Drives the cleanup-model install status next to the model <select>: shows a
 * checkmark when installed, a Download button + live progress bar when not.
 */
export function initModelPicker(
  select: HTMLSelectElement,
  statusEl: HTMLElement,
): void {
  let latest: InstalledModels = { up: false, installed: [] };
  let pullingModel: string | null = null;
  // Cancelling the key modal must put the select back, or a cloud engine
  // sits selected with no key while every dictation quietly falls back to
  // local — the UI would claim cloud and deliver local.
  let lastGoodToken = select.value;

  /** The Ollama tag for the current selection, or null when a cloud model
   *  is selected and this renderer should stand down. */
  const selectedLocal = (): string | null => {
    const { engine, model } = splitToken(select.value);
    return engine === "local" ? model : null;
  };

  function renderInstalled() {
    statusEl.replaceChildren();
    const span = document.createElement("span");
    span.className = "ok";
    span.textContent = "Installed ✓";
    statusEl.appendChild(span);
  }

  function renderOllamaDown() {
    statusEl.replaceChildren();
    const span = document.createElement("span");
    span.textContent = "Start Ollama to download or check models.";
    statusEl.appendChild(span);
  }

  function renderDownload() {
    statusEl.replaceChildren();
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "ghost";
    btn.textContent = "Download";
    btn.addEventListener("click", () => void startPull());
    statusEl.appendChild(btn);
  }

  function renderProgress(percent: number, label: string) {
    statusEl.replaceChildren();
    const bar = document.createElement("div");
    bar.className = "bar";
    const fill = document.createElement("span");
    fill.style.width = `${percent}%`;
    bar.appendChild(fill);
    const text = document.createElement("span");
    text.textContent = label ? `${label} ${percent}%` : `Downloading… ${percent}%`;
    statusEl.appendChild(bar);
    statusEl.appendChild(text);
  }

  function renderError(msg: string) {
    statusEl.replaceChildren();
    const err = document.createElement("span");
    err.className = "err";
    err.textContent = msg;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "ghost";
    btn.textContent = "Retry";
    btn.addEventListener("click", () => void startPull());
    statusEl.appendChild(err);
    statusEl.appendChild(btn);
  }

  function render() {
    const local = selectedLocal();
    if (local === null) {
      // Clear before handing the row to cloud-key.ts (Task 12). Switching to
      // a cloud model mid-download otherwise leaves a frozen progress bar:
      // this returns early and the `ollama-pull` listener stops matching, so
      // nothing would ever repaint it. Clearing here removes the dependency
      // on which listener happens to run first.
      statusEl.replaceChildren();
      return;
    }
    if (pullingModel !== null && pullingModel === local) return; // progress renders itself via events
    if (!latest.up) {
      renderOllamaDown();
    } else if (modelInstalled(latest.installed, local)) {
      renderInstalled();
    } else {
      renderDownload();
    }
  }

  async function refresh() {
    latest = await invoke<InstalledModels>("list_installed_models");
    if (selectedLocal() === null) {
      renderCloudRow(statusEl, select.value);
    } else {
      render();
    }
  }

  async function startPull() {
    const model = selectedLocal();
    if (model === null) return;
    pullingModel = model;
    renderProgress(0, "Downloading…");
    try {
      await invoke("pull_ollama_model", { model });
    } catch (err) {
      if (pullingModel === model) pullingModel = null;
      renderError(String(err));
    }
  }

  void listen<PullEvent>("ollama-pull", (e) => {
    const ev = e.payload;
    const terminal = ev.done || !!ev.error;
    if (terminal && ev.model === pullingModel) {
      pullingModel = null;
    }
    if (ev.done) {
      // Keep the installed cache fresh regardless of what's selected, so
      // switching back to a finished model later reflects reality instead
      // of a stale "not installed" snapshot.
      void refresh();
    }
    if (ev.model !== selectedLocal()) return; // don't paint for a non-selected model
    if (ev.error) {
      renderError(ev.error);
      return;
    }
    if (ev.done) return; // refresh() above already repainted the selected model
    renderProgress(ev.percent, ev.status);
  });

  select.addEventListener("change", () => {
    const token = select.value;
    if (selectedLocal() !== null) {
      lastGoodToken = token;
      render();
      return;
    }
    void ensureKey(token).then((ok) => {
      if (!ok) {
        select.value = lastGoodToken;
        // render() clears and returns for a cloud token, which would wipe the
        // persistent disclosure. Repaint it instead.
        if (splitToken(lastGoodToken).engine === "local") {
          render();
        } else {
          renderCloudRow(statusEl, lastGoodToken);
        }
        return;
      }
      lastGoodToken = token;
      renderCloudRow(statusEl, token);
    });
  });

  void refresh();
}

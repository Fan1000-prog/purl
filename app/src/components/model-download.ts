// One model's download card: Download → progress with Cancel → Ready, or the
// failure sentence from Rust with Retry. Shared by onboarding (speech model),
// the Home Smart-cleanup offer and Settings. Talks only to the shared
// contract: models_status / download_model / cancel_download and the
// model-progress / model-done events.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ModelDone, ModelProgress, ModelView } from "../types";

export interface ModelDownloadOptions {
  id: string;
  /** Card heading; defaults to the model's label. */
  title?: string;
  /** Shown after the size, e.g. "runs on your computer". */
  detail?: string;
  /** Called once the model is on disk and verified (also if it already was). */
  onReady?: () => void;
}

export interface ModelDownload {
  destroy(): void;
}

// download.rs words a disk-full refusal this way. Retrying can't help until
// space is freed, so Retry waits for the window to regain focus.
const DISK_FULL = /^Not enough free space/;

export function formatBytes(bytes: number): string {
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
}

export function mountModelDownload(el: HTMLElement, opts: ModelDownloadOptions): ModelDownload {
  const card = document.createElement("div");
  card.className = "model-card";
  card.innerHTML = `
    <div class="model-head">
      <div>
        <div class="model-title"></div>
        <div class="model-meta muted"></div>
      </div>
      <span class="model-ready" hidden>Ready</span>
    </div>
    <div class="model-progress" hidden>
      <div class="model-bar" role="progressbar" aria-valuemin="0" aria-valuemax="100"><span></span></div>
      <div class="model-count muted"></div>
    </div>
    <p class="model-error warn-text" hidden></p>
    <p class="model-live sr-only" aria-live="polite"></p>
    <div class="model-actions">
      <button type="button" class="btn btn-primary model-go">Download</button>
      <button type="button" class="btn model-cancel" hidden>Cancel</button>
    </div>`;
  el.appendChild(card);

  const q = <T extends HTMLElement>(sel: string) => card.querySelector(sel) as T;
  const go = q<HTMLButtonElement>(".model-go");
  const cancel = q<HTMLButtonElement>(".model-cancel");
  const bar = q<HTMLElement>(".model-bar");
  const errorEl = q<HTMLElement>(".model-error");

  let model: ModelView | null = null;
  let announced: ModelView["state"] | null = null;
  let cancelling = false;
  let readyFired = false;
  const unlisteners: Promise<UnlistenFn>[] = [];

  // One polite announcement per state change, never per progress tick.
  function announce(m: ModelView) {
    if (announced === null || announced === m.state) {
      announced = m.state;
      return;
    }
    const name = opts.title ?? m.label;
    q(".model-live").textContent =
      m.state === "downloading" ? `Downloading ${name}.`
      : m.state === "ready" ? `${name} is ready.`
      : m.state === "failed" ? `Download failed. ${m.error ?? ""}`
      : "Download cancelled. You can resume it.";
    announced = m.state;
  }

  function render() {
    if (!model) return;
    const m = model;
    // Hiding the focused button would drop focus to <body>; hand it over.
    const focused = document.activeElement;
    card.dataset.state = m.state;
    q(".model-title").textContent = opts.title ?? m.label;
    q(".model-meta").textContent = [formatBytes(m.size_bytes), opts.detail].filter(Boolean).join(" · ");

    const downloading = m.state === "downloading";
    q(".model-progress").hidden = !downloading;
    q(".model-ready").hidden = m.state !== "ready";
    q(".model-actions").hidden = m.state === "ready";
    go.hidden = downloading;
    cancel.hidden = !downloading;
    errorEl.hidden = m.state !== "failed";
    errorEl.textContent = m.error ?? "";

    const pct = m.size_bytes ? Math.floor((m.downloaded_bytes / m.size_bytes) * 100) : 0;
    bar.setAttribute("aria-valuenow", String(pct));
    bar.setAttribute("aria-label", `Downloading ${opts.title ?? m.label}`);
    (bar.firstElementChild as HTMLElement).style.width = `${pct}%`;
    q(".model-count").textContent = `${formatBytes(m.downloaded_bytes)} of ${formatBytes(m.size_bytes)}`;

    go.textContent =
      m.state === "failed" ? "Retry"
      : m.downloaded_bytes > 0 ? `Resume (${formatBytes(m.downloaded_bytes)} of ${formatBytes(m.size_bytes)})`
      : "Download";
    go.disabled = m.state === "failed" && DISK_FULL.test(m.error ?? "");

    if (focused === go && go.hidden && !cancel.hidden) cancel.focus();
    if (focused === cancel && cancel.hidden && !go.hidden) go.focus();
    announce(m);

    if (m.state === "ready" && !readyFired) {
      readyFired = true;
      opts.onReady?.();
    }
  }

  async function refresh() {
    const all = await invoke<ModelView[]>("models_status");
    model = all.find((m) => m.id === opts.id) ?? null;
    if (!model) {
      card.dataset.state = "failed";
      errorEl.hidden = false;
      errorEl.textContent = `Purl doesn't know a model called "${opts.id}".`;
      errorEl.setAttribute("role", "alert");
      go.hidden = true;
      return;
    }
    render();
  }

  go.addEventListener("click", () => {
    if (!model) return;
    model = { ...model, state: "downloading", error: null };
    render();
    invoke("download_model", { id: opts.id }).catch((err) => {
      if (!model) return;
      model = { ...model, state: "failed", error: String(err) };
      render();
    });
  });

  cancel.addEventListener("click", () => {
    cancelling = true;
    void invoke("cancel_download", { id: opts.id });
  });

  unlisteners.push(
    listen<ModelProgress>("model-progress", (e) => {
      if (!model || e.payload.id !== opts.id) return;
      model = { ...model, state: "downloading", downloaded_bytes: e.payload.downloaded_bytes, size_bytes: e.payload.size_bytes };
      render();
    }),
    listen<ModelDone>("model-done", (e) => {
      if (!model || e.payload.id !== opts.id) return;
      if (e.payload.ok) model = { ...model, state: "ready", downloaded_bytes: model.size_bytes, error: null };
      else if (cancelling) model = { ...model, state: "missing", error: null };
      else model = { ...model, state: "failed", error: e.payload.error };
      cancelling = false;
      render();
    }),
  );

  // Space may have been freed in Finder / Explorer meanwhile.
  const onFocus = () => {
    if (model?.state === "failed" && DISK_FULL.test(model.error ?? "")) go.disabled = false;
  };
  window.addEventListener("focus", onFocus);

  void refresh().catch((err) => {
    card.dataset.state = "failed";
    errorEl.hidden = false;
    errorEl.textContent = `Couldn't check the model (${String(err)}).`;
    errorEl.setAttribute("role", "alert");
  });

  return {
    destroy() {
      window.removeEventListener("focus", onFocus);
      for (const u of unlisteners) void u.then((fn) => fn());
      card.remove();
    },
  };
}

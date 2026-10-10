// Home: a title and state dot showing what's happening, the hotkey, the last
// dictation (cleaned or raw), today's stats and recent history. A banner
// takes the top when something blocks dictation or paste.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { EngineState, HistoryEntry } from "../types";
import { onViewShow, prettyHotkey, switchView, type TranscriptPayload } from "../main";
import { mountModelDownload, type ModelDownload } from "../components/model-download";
import { confirmButton } from "../components/confirm-button";
import { onRadio, setRadio } from "../components/radio-group";
import { SPEECH_MODEL, watchSpeechEngine } from "../engine";
import { getStatus } from "../status";
import { privacySettingsUrl } from "../platform";
import { renderStats } from "./insights";
import type { Trust } from "../types";

const STATUS_POLL_MS = 5000;
const DONE_MS = 2400;
const HISTORY_SHOWN = 20;

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

let dictation: "idle" | "recording" | "processing" | "done" = "idle";
let stt: EngineState = { state: "ready" };
let pasteTrust: Trust = "granted";
let doneTimer: ReturnType<typeof setTimeout> | undefined;
let last: TranscriptPayload | null = null;
let showRaw = false;
let bannerDownload: ModelDownload | null = null;
let bannerKey = "";

// ---------- hero: state, title, button ----------

type HeroState = "idle" | "listening" | "thinking" | "done" | "blocked";

function renderHero() {
  const ready = stt.state === "ready" || stt.state === "idle";
  const [title, state]: [string, HeroState] =
    dictation === "recording" ? ["Listening…", "listening"]
    : dictation === "processing" ? ["Tidying up…", "thinking"]
    : dictation === "done" ? ["Sent.", "done"]
    : stt.state === "loading" ? ["Warming up…", "thinking"]
    : stt.state === "missing" ? ["One more step.", "blocked"]
    : stt.state === "failed" ? ["Speech isn't working.", "blocked"]
    : ["Ready when you are.", "idle"];
  $("home-title").textContent = title;
  $("home-state").dataset.state = state;

  const btn = $<HTMLButtonElement>("toggle-btn");
  btn.textContent = dictation === "recording" ? "Stop and send" : "Start dictation";
  btn.disabled = dictation === "processing" || !(ready || stt.state === "loading");
  btn.classList.toggle("is-recording", dictation === "recording");
}

// ---------- banner: what blocks dictation or paste ----------

function renderBanner() {
  const banner = $("home-banner");
  const key = stt.state === "missing" ? "missing" : stt.state === "failed" ? "failed" : pasteTrust === "denied" ? "trust" : "";
  if (key === bannerKey) return;
  bannerKey = key;
  bannerDownload?.destroy();
  bannerDownload = null;
  banner.classList.toggle("hidden", key === "");
  banner.dataset.kind = key;
  banner.replaceChildren();
  if (!key) return;

  const text = document.createElement("p");
  banner.append(text);
  if (key === "missing") {
    text.textContent = "Download the speech model to start dictating. It runs on this computer, so it's needed once.";
    const slot = document.createElement("div");
    banner.append(slot);
    bannerDownload = mountModelDownload(slot, { id: SPEECH_MODEL, title: "Parakeet v3", detail: "runs on your computer" });
  } else if (key === "failed") {
    text.textContent = stt.state === "failed" ? stt.reason : "";
    banner.append(actionButton("Open Settings", () => switchView("settings")));
  } else {
    text.textContent = "Paste needs Accessibility. Until you allow it, your words go to the clipboard and you paste them yourself.";
    const url = privacySettingsUrl("accessibility");
    if (url) banner.append(actionButton("Open Accessibility settings", () => void openUrl(url)));
  }
}

function actionButton(label: string, onClick: () => void): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "btn";
  b.textContent = label;
  b.addEventListener("click", onClick);
  return b;
}

async function refreshStatus() {
  try {
    const s = await getStatus();
    pasteTrust = s.pasteTrust;
    $("hotkey-label").textContent = prettyHotkey(s.hotkey);
    if (dictation !== "done") dictation = s.state === "recording" || s.state === "processing" ? s.state : "idle";
    renderHero();
    renderBanner();
  } catch {
    // Status is advisory here; the next poll tries again.
  }
}

// ---------- last dictation ----------

function renderLast() {
  const has = last !== null;
  $("last-text").hidden = !has;
  $("last-meta").hidden = !has;
  $("last-empty").hidden = has;
  const toggle = $("view-home").querySelector<HTMLElement>(".segmented")!;
  setRadio(toggle, (b) => (b.dataset.show === "raw") === showRaw);
  toggle.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = !has));
  if (!last) return;
  $("last-text").textContent = (showRaw ? last.raw : last.cleaned) || "—";
  const meta = $("last-meta");
  meta.replaceChildren();
  const how = document.createElement("span");
  how.textContent = last.used_llm ? "Smart cleanup" : "Fast cleanup";
  meta.append(how, copyButton(() => (showRaw ? last!.raw : last!.cleaned)));
  if (last.note) {
    const note = document.createElement("span");
    note.className = "muted";
    note.textContent = last.note;
    meta.append(note);
  }
}

function copyButton(text: () => string): HTMLButtonElement {
  const copy = actionButton("Copy", () => {
    void navigator.clipboard.writeText(text()).then(() => {
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1200);
    });
  });
  copy.className = "btn btn-quiet";
  return copy;
}

// ---------- recent history ----------

function relativeTime(unixSecs: number): string {
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} h ago`;
  return `${Math.floor(secs / 86400)} d ago`;
}

// All content set via textContent, never innerHTML.
function historyItem(e: HistoryEntry): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "list-item history-item";
  const text = document.createElement("p");
  text.className = "history-text";
  text.textContent = e.cleaned || "—";
  const meta = document.createElement("div");
  meta.className = "row-meta";
  const when = document.createElement("span");
  when.textContent = `${relativeTime(e.ts)} · ${e.words} words`;
  meta.append(when, copyButton(() => e.cleaned));
  li.append(text, meta);
  return li;
}

async function refreshHistory() {
  const list = $("history-list");
  const empty = $("history-empty");
  const error = $("history-error");
  try {
    const entries = await invoke<HistoryEntry[]>("get_history");
    list.replaceChildren(...entries.slice(-HISTORY_SHOWN).reverse().map(historyItem));
    empty.classList.toggle("hidden", entries.length > 0);
    error.classList.add("hidden");
    $("clear-history-btn").hidden = entries.length === 0;
  } catch (err) {
    list.replaceChildren();
    empty.classList.add("hidden");
    error.textContent = `Couldn't load your history (${String(err)}). It's still on disk; try reopening Purl.`;
    error.classList.remove("hidden");
  }
}

async function refreshLast() {
  try {
    last = await invoke<TranscriptPayload | null>("get_last_result");
  } catch {
    last = null;
  }
  renderLast();
}

function refreshAll() {
  void refreshLast();
  void refreshHistory();
  void renderStats($("home-stats"));
}

export function initHome() {
  $("toggle-btn").addEventListener("click", () => void invoke("toggle_recording"));

  onRadio($("view-home").querySelector<HTMLElement>(".segmented")!, (b) => {
    showRaw = b.dataset.show === "raw";
    renderLast();
  });

  confirmButton($("clear-history-btn"), "Delete all history? It only lives on this computer.", () => {
    invoke("clear_history")
      .then(refreshAll)
      .catch((err) => ($("note").textContent = String(err)));
  });

  void listen<string>("state", (e) => {
    if (e.payload !== "recording" && e.payload !== "processing" && e.payload !== "idle") return;
    // A finished dictation shows "Sent." for a moment before idle wins.
    if (e.payload === "idle" && dictation === "done") return;
    clearTimeout(doneTimer);
    dictation = e.payload;
    renderHero();
  });
  void listen<TranscriptPayload>("result", (e) => {
    last = e.payload;
    $("note").textContent = "";
    dictation = "done";
    renderHero();
    renderLast();
    clearTimeout(doneTimer);
    doneTimer = setTimeout(() => {
      dictation = "idle";
      renderHero();
    }, DONE_MS);
  });
  void listen<string>("pipeline-error", (e) => {
    $("note").textContent = e.payload;
  });

  watchSpeechEngine((next) => {
    stt = next;
    renderHero();
    renderBanner();
  });

  onViewShow("home", refreshAll);
  refreshAll();
  void refreshStatus();
  setInterval(() => void refreshStatus(), STATUS_POLL_MS);
}

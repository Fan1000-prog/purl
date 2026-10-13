// Persistent voice pill: the brand dictation pill (src/brand/components),
// driven by the app's events. Idle shows "Hold <hotkey>" (click to start
// dictating); recording shows the live waveform and timer from "audio-level"
// events, with ✕ cancel and ✓ finish-and-transcribe beside it; processing is
// "Polishing" ("Warming up…" while the speech model loads); a finished
// dictation flashes "Sent clean"; a failure flashes "Didn't catch that".
// Transforms and learned corrections reuse the pill for a caption with a 5 s
// countdown.

// Must stay first: installs the dev-only Tauri mock (?mock=1) for previews.
import "./dev/mock-tauri";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { mountPill, type PillFlowState } from "./brand/components";
import { watchSpeechEngine } from "./engine";
import { micLevel } from "./level";
import { getStatus } from "./status";
import { pillHotkey } from "./platform";

type AppState = "idle" | "recording" | "processing";
type Flash = "done" | "error" | "celebrate";
type View = AppState | Flash;

const DONE_MS = 1600;
const ERROR_MS = 2400;
const CELEBRATE_MS = 5000;
const DEFAULT_HOTKEY = "CmdOrCtrl+Shift+Space";
const ERROR_TEXT = "Didn't catch that";
const WARMING_TEXT = "Warming up…";

const PILL_STATE: Record<Exclude<View, "error">, PillFlowState> = {
  idle: "idle",
  recording: "listening",
  processing: "polishing",
  done: "sent",
  celebrate: "sent",
};

const control = document.getElementById("pill")!;
const fillEl = document.getElementById("countdown-fill")!;
const pill = mountPill(document.getElementById("capsule")!, {
  hotkey: pillHotkey(DEFAULT_HOTKEY),
  idleLabel: "Hold",
});

// What Rust last said, and a short-lived flash (done / error / caption) that
// shows while Rust is idle. Recording and processing always win over a flash.
let appState: AppState = "idle";
let flash: Flash | null = null;
let caption = "";
let flashTimer: ReturnType<typeof setTimeout> | undefined;
// A first dictation can wait on the speech model loading; say so, so the
// pill doesn't look stuck.
let warming = false;

function currentView(): View {
  return appState === "idle" && flash ? flash : appState;
}

// The view and label the pill last showed. Setting the pill rewrites its
// live region, which makes a screen reader say it again, so a render that
// changes neither (a repeat state event, an engine-status update) skips it.
let shown = "";

function render() {
  const view = currentView();
  document.body.dataset.view = view;
  document.body.dataset.app = appState;
  const label =
    view === "error" ? ERROR_TEXT
    : view === "celebrate" ? caption
    : view === "processing" && warming ? WARMING_TEXT
    : undefined;
  const next = `${view}\n${label ?? ""}`;
  if (next !== shown) {
    shown = next;
    if (view === "error") pill.error(ERROR_TEXT);
    else pill.set(PILL_STATE[view], label);
  }
  requestAnimationFrame(reportHitRect);
}

function showFlash(kind: Flash, ms: number) {
  clearTimeout(flashTimer);
  flash = kind;
  render();
  flashTimer = setTimeout(() => {
    flash = null;
    render();
  }, ms);
}

// This window is far bigger than the pill and sits bottom-centre, on top of
// other apps' controls. Rust keeps it click-through and only lets clicks land
// inside the bounds reported here, so the empty space around the pill never
// swallows a click meant for the app underneath. While processing the pill
// isn't interactive, so it reports nothing. Only a changed rect is sent;
// one Rust didn't take is sent again on the next report.
let lastHitRect = "";
function reportHitRect() {
  const r = appState === "processing" ? null : control.getBoundingClientRect();
  const rect = r ? [r.left, r.top, r.width, r.height] : null;
  const key = JSON.stringify(rect);
  if (key === lastHitRect) return;
  lastHitRect = key;
  invoke("set_overlay_hit_rect", { rect }).catch(() => {
    if (lastHitRect === key) lastHitRect = "";
  });
}
window.addEventListener("resize", reportHitRect);
// The pill also changes size on its own: the capsule morphs between states,
// and the UI font finishing loading changes label widths after the first
// report. Re-report on every change.
new ResizeObserver(() => reportHitRect()).observe(control);

// ---------- mic level and hotkey ----------

void listen<number>("audio-level", (e) => pill.level(micLevel(e.payload)));

function setHotkey(hk: string) {
  pill.setHotkey(pillHotkey(hk || DEFAULT_HOTKEY));
}
void getStatus().then((s) => setHotkey(s.hotkey), () => {});
// The main window broadcasts a newly recorded hotkey (main.ts).
void listen<string>("hotkey-changed", (e) => setHotkey(e.payload));

// ---------- app state ----------

void listen<string>("state", (e) => {
  switch (e.payload) {
    case "recording":
    case "processing":
    case "idle":
      if (e.payload === "recording") {
        clearTimeout(flashTimer);
        flash = null;
      }
      appState = e.payload;
      render();
      break;
    case "done":
      appState = "idle";
      showFlash("done", DONE_MS);
      break;
    case "error":
      appState = "idle";
      showFlash("error", ERROR_MS);
      break;
  }
});

// Every successful dictation gets "Sent clean".
void listen("result", () => showFlash("done", DONE_MS));

control.addEventListener("click", (e) => {
  if (appState !== "idle" || (e.target as HTMLElement).closest("button")) return;
  void invoke("toggle_recording");
});
document.getElementById("cancel")!.addEventListener("click", () => {
  void invoke("cancel_dictation");
});
document.getElementById("confirm")!.addEventListener("click", () => {
  void invoke("toggle_recording");
});

// ---------- transform / correction caption with a 5 s countdown ----------

function celebrate(text: string) {
  caption = text;
  showFlash("celebrate", CELEBRATE_MS);

  // Restart the countdown bar: drop the transition, snap full, force a
  // reflow, then animate 100% → 0% even if a previous caption is winding down.
  fillEl.style.transition = "none";
  fillEl.style.transform = "scaleX(1)";
  void fillEl.offsetWidth;
  fillEl.style.transition = `transform ${CELEBRATE_MS}ms linear`;
  fillEl.style.transform = "scaleX(0)";
}

void listen<string>("transform-result", (e) => celebrate(`"${e.payload}" applied`));
void listen<string>("correction-learned", (e) => celebrate(`Got it, "${e.payload}" logged`));

// The only live signal that the app is reading the field it pasted into.
// macOS will not show one: the Accessibility grant was already given for
// Cmd-V, so nothing at the OS level marks this as new.
void listen<boolean>("watching-edits", (e) => {
  document.body.classList.toggle("watching", e.payload);
});

watchSpeechEngine((stt) => {
  warming = stt.state === "loading";
  render();
});

render();

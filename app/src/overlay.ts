// Persistent voice pill: Purl the mascot plus a state label on a Paper pill.
// Idle shows the mascot alone (click to start dictating); recording adds
// ✕ cancel and ✓ finish-and-transcribe around "Listening", with a Sprout ring
// that swells with the mic level ("audio-level" events from the recorder
// thread); processing is "Tidying up"; a finished dictation flashes "Sent".
// Transforms and learned corrections reuse the pill for a caption with a 5 s
// countdown, plus a confetti burst.

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { burstConfetti } from "./celebrate";
import { mountMascot, type MascotState } from "./components/mascot";
import { watchSpeechEngine } from "./engine";

type AppState = "idle" | "recording" | "processing";
type Flash = "done" | "error" | "celebrate";
type View = AppState | Flash;

const DONE_MS = 1600;
const ERROR_MS = 2400;
const CELEBRATE_MS = 5000;
const CONFETTI_COLORS = ["#C0683F", "#F3D36B", "#3F6B4B", "#B7CDA3"];

const VIEWS: Record<View, { mascot: MascotState; label: string }> = {
  idle: { mascot: "idle", label: "" },
  recording: { mascot: "listening", label: "Listening" },
  processing: { mascot: "thinking", label: "Tidying up" },
  done: { mascot: "done", label: "Sent" },
  error: { mascot: "idle", label: "Didn't catch that" },
  celebrate: { mascot: "done", label: "" },
};

const pill = document.getElementById("pill")!;
const labelEl = document.getElementById("label")!;
const fillEl = document.getElementById("countdown-fill")!;
const slot = document.getElementById("mascot-slot")!;
const confettiLayer = document.getElementById("confetti-layer")!;
const mascot = mountMascot(slot, { size: 44, surface: "cream" });

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

function render() {
  const view = currentView();
  document.body.dataset.view = view;
  document.body.dataset.app = appState;
  mascot.set(VIEWS[view].mascot);
  labelEl.textContent =
    view === "celebrate" ? caption : view === "processing" && warming ? "Warming up" : VIEWS[view].label;
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
// isn't interactive, so it reports nothing.
function reportHitRect() {
  const r = appState === "processing" ? null : pill.getBoundingClientRect();
  void invoke("set_overlay_hit_rect", {
    rect: r ? [r.left, r.top, r.width, r.height] : null,
  });
}
window.addEventListener("resize", reportHitRect);

// ---------- mic level ring ----------

// Raw RMS is ~0.005 (whisper-quiet) to ~0.3 (loud). Normalize, then smooth
// with fast attack / slow decay so speech feels snappy. The loop only runs
// while recording.
let target = 0;
let level = 0;
let levelFrame = 0;

void listen<number>("audio-level", (e) => {
  target = Math.min(1, e.payload * 6);
});

function levelTick() {
  level += (target - level) * (target > level ? 0.5 : 0.12);
  slot.style.setProperty("--level", level.toFixed(3));
  levelFrame = requestAnimationFrame(levelTick);
}

function setRecording(on: boolean) {
  cancelAnimationFrame(levelFrame);
  target = 0;
  level = 0;
  slot.style.setProperty("--level", "0");
  if (on) levelFrame = requestAnimationFrame(levelTick);
}

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
      setRecording(e.payload === "recording");
      appState = e.payload;
      render();
      break;
    case "done":
      appState = "idle";
      setRecording(false);
      showFlash("done", DONE_MS);
      break;
    case "error":
      appState = "idle";
      setRecording(false);
      showFlash("error", ERROR_MS);
      break;
  }
});

// Every successful dictation gets "Sent" and a small confetti burst.
void listen("result", () => {
  showFlash("done", DONE_MS);
  burstConfetti(confettiLayer, {
    colors: CONFETTI_COLORS,
    pieceCount: 14,
    spread: 100,
    fallDistance: 70,
  });
});

pill.addEventListener("click", (e) => {
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
  burstConfetti(confettiLayer, {
    colors: CONFETTI_COLORS,
    pieceCount: 18,
    spread: 120,
    fallDistance: 70,
  });

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

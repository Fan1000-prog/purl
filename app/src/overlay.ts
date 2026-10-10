// Persistent voice pill: idle mic (click to start dictating), recording
// pill (✕ cancel, live waveform, ✓ finish-and-transcribe), or the
// transcribing spinner. Bar heights track the speaker's mic level
// ("audio-level" events from the recorder thread) and scroll left like a
// voice-memo waveform. Also surfaces a confetti + countdown toast whenever
// a transform finishes running on the user's selected text.

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { burstConfetti } from "./celebrate";

const canvas = document.getElementById("wave") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;
const dpr = window.devicePixelRatio || 1;
const W = 70;
const H = 26;
canvas.width = W * dpr;
canvas.height = H * dpr;
canvas.style.width = `${W}px`;
canvas.style.height = `${H}px`;
ctx.scale(dpr, dpr);

const BAR_WIDTH = 2.2;
const BAR_GAP = 1.5;
const N_BARS = Math.floor(W / (BAR_WIDTH + BAR_GAP));
const MIN_BAR = 2;
const MAX_BAR = H - 6;

// Mic level: raw RMS is ~0.005 (whisper-quiet) to ~0.3 (loud). Normalize,
// then smooth with fast attack / slow decay so speech feels snappy.
let target = 0;
let level = 0;
const history: number[] = new Array(N_BARS).fill(0);

void listen<number>("audio-level", (e) => {
  target = Math.min(1, e.payload * 6);
});

// Build the 12-spoke spinner shown while transcribing (staggered fade,
// like the native macOS activity indicator).
const spinner = document.getElementById("spinner")!;
const SPOKES = 12;
for (let i = 0; i < SPOKES; i++) {
  const spoke = document.createElement("span");
  spoke.style.transform = `rotate(${(360 / SPOKES) * i}deg) translateY(-5px)`;
  spoke.style.animationDelay = `${-0.8 + (0.8 / SPOKES) * i}s`;
  spinner.appendChild(spoke);
}

// This window is far bigger than the control it draws — it also has room for
// the recording pill and the celebration toast — and sits bottom-center, on
// top of other apps' controls. Rust keeps it click-through and only lets
// clicks land inside the bounds reported here, so the empty space around the
// mic never swallows a click meant for the app underneath. The spinner and the
// celebration toast aren't interactive, so those states report nothing.
function reportHitRect() {
  const shown = document.querySelector<HTMLElement>(
    "body.recording #pill, body:not(.recording):not(.processing) #idle",
  );
  const r = shown?.getBoundingClientRect();
  void invoke("set_overlay_hit_rect", {
    rect: r ? [r.left, r.top, r.width, r.height] : null,
  });
}
requestAnimationFrame(reportHitRect);
window.addEventListener("resize", reportHitRect);

// Swap idle ↔ pill ↔ spinner with the app state; reset the waveform trace
// whenever a new recording starts.
void listen<string>("state", (e) => {
  document.body.classList.toggle("recording", e.payload === "recording");
  document.body.classList.toggle("processing", e.payload === "processing");
  requestAnimationFrame(reportHitRect);
  if (e.payload === "recording") {
    history.fill(0);
    target = 0;
    level = 0;
  }
});

// Push a sample every other frame (~30/s) so the scroll speed reads
// naturally against the 20 samples/s the recorder emits.
let tick = 0;
function frame() {
  level += (target - level) * (target > level ? 0.5 : 0.12);
  tick += 1;
  if (tick % 2 === 0) {
    history.push(level);
    history.shift();
  }

  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = "#f5f3ef";
  for (let i = 0; i < N_BARS; i++) {
    const h = MIN_BAR + history[i] * (MAX_BAR - MIN_BAR);
    const x = i * (BAR_WIDTH + BAR_GAP);
    const y = (H - h) / 2;
    ctx.beginPath();
    ctx.roundRect(x, y, BAR_WIDTH, h, BAR_WIDTH / 2);
    ctx.fill();
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

document.getElementById("idle")!.addEventListener("click", () => {
  void invoke("toggle_recording");
});
document.getElementById("cancel")!.addEventListener("click", () => {
  void invoke("cancel_dictation");
});
document.getElementById("confirm")!.addEventListener("click", () => {
  void invoke("toggle_recording");
});

// ---------- transform celebration: confetti + 5s countdown caption ----------

const celebrateEl = document.getElementById("celebrate")!;
const captionEl = document.getElementById("celebrate-caption")!;
const fillEl = document.getElementById("celebrate-fill") as HTMLElement;
const confettiLayer = document.getElementById("confetti-layer")!;
let celebrateTimer: ReturnType<typeof setTimeout> | undefined;

// Every successful dictation gets confetti, but NOT the caption/countdown the
// transform celebration shows: dictation happens constantly, and a 5s banner on
// each one would be noise. `#confetti-layer` is a sibling of `#celebrate` and is
// not gated by `.show`, so the burst stands alone. `reportHitRect` keys off the
// pill/idle elements, so this cannot affect what the pill can click.
void listen("result", () => {
  burstConfetti(confettiLayer, {
    colors: ["#f5f3ef", "#b08968", "#8a6c4f", "#2c2c31"],
    pieceCount: 14,
    spread: 100,
    fallDistance: 80,
  });
});

void listen<string>("transform-result", (e) => {
  clearTimeout(celebrateTimer);
  captionEl.textContent = `"${e.payload}" applied ✓`;
  burstConfetti(confettiLayer, {
    colors: ["#f5f3ef", "#b08968", "#8a6c4f", "#2c2c31"],
    pieceCount: 18,
    spread: 120,
    fallDistance: 90,
  });

  // Restart the countdown bar: drop the transition, snap full, force a
  // reflow, then re-enable the transition and set the animated target so
  // it runs 100% → 0% over 5s even if a previous celebration is still
  // winding down.
  celebrateEl.classList.add("show");
  fillEl.style.transition = "none";
  fillEl.style.transform = "scaleX(1)";
  void fillEl.offsetWidth; // force reflow
  fillEl.style.transition = "transform 5000ms linear";
  fillEl.style.transform = "scaleX(0)";

  celebrateTimer = setTimeout(() => {
    celebrateEl.classList.remove("show");
  }, 5000);
});

// A correction was learned. Reuses the transform celebration rather than
// adding a second toast: same caption, same 5s countdown.
void listen<string>("correction-learned", (e) => {
  clearTimeout(celebrateTimer);
  captionEl.textContent = `Got it — "${e.payload}" now logged`;
  burstConfetti(confettiLayer, {
    colors: ["#f5f3ef", "#b08968", "#8a6c4f", "#2c2c31"],
    pieceCount: 18,
    spread: 120,
    fallDistance: 90,
  });
  celebrateEl.classList.add("show");
  fillEl.style.transition = "none";
  fillEl.style.transform = "scaleX(1)";
  void fillEl.offsetWidth; // force reflow
  fillEl.style.transition = "transform 5000ms linear";
  fillEl.style.transform = "scaleX(0)";
  celebrateTimer = setTimeout(() => celebrateEl.classList.remove("show"), 5000);
});

// The only live signal that the app is reading the field it pasted into.
// macOS will not show one: the Accessibility grant was already given for
// Cmd-V, so nothing at the OS level marks this as new.
void listen<boolean>("watching-edits", (e) => {
  document.body.classList.toggle("watching", e.payload);
});

// Dictation pill: live, state-driven version of the motion pack's
// 02-dictation-pill. Import pill.css alongside.
//
//   const pill = mountPill(el, { hotkey: "Ctrl+Space" });
//   pill.set("listening"); pill.level(0.6); pill.set("polishing");
//   pill.set("sent"); pill.error("Microphone not found"); pill.warn("Mic is very quiet");

import { alertSvg, checkSvg, tickSvg } from "./glyphs";
import { clamp01, frameLoop, valueNoise, watchReducedMotion } from "./motion";

export type PillFlowState = "idle" | "listening" | "polishing" | "sent";
export type PillState = PillFlowState | "error" | "warning";

export interface PillOptions {
  /** The user's real hotkey, e.g. "fn", "Ctrl+Space", "⌥ Space". Parts split on "+". */
  hotkey: string;
  /** Word before the keycaps in the idle state. Default "Hold". */
  idleLabel?: string;
}

export interface PillHandle {
  /**
   * Change state. `label` replaces the text of "polishing" ("Warming up…")
   * or "sent" (a caption); omit it to get the default back. Set as text, never HTML.
   */
  set(state: PillFlowState, label?: string): void;
  /** Mic level 0..1. Drives the bars and the bead while listening. */
  level(n: number): void;
  /** One short line, e.g. "Microphone not found". */
  error(message: string): void;
  /** Softer one-liner, e.g. "Mic is very quiet". */
  warn(message: string): void;
  setHotkey(hotkey: string): void;
  readonly state: PillState;
  destroy(): void;
}

const BAR_COUNT = 32;
const BAR_MIN = 4;
const BAR_RANGE = 38;

/** States whose label the caller may replace. */
const LABELLED = new Set<PillFlowState>(["polishing", "sent"]);

const SPOKEN: Record<PillFlowState, string> = {
  idle: "Ready",
  listening: "Listening",
  polishing: "Polishing",
  sent: "Sent clean",
};

export function formatElapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

function hotkeyParts(hotkey: string): string[] {
  const parts = hotkey.split("+").map((p) => p.trim()).filter(Boolean);
  if (!parts.length) throw new TypeError("mountPill: `hotkey` must be a non-empty string such as \"Ctrl+Space\"");
  return parts;
}

export function mountPill(host: HTMLElement, options: PillOptions): PillHandle {
  const keys = hotkeyParts(options.hotkey);

  host.classList.add("purl-pill", "theme-dark");
  host.setAttribute("role", "status");
  host.innerHTML = `
    <div class="purl-pill__shell" aria-hidden="true">
      <span class="purl-pill__spin"></span>
      <div class="purl-pill__body">
        <span class="purl-pill__tick">${tickSvg(22)}</span>
        <span class="purl-pill__layer" data-layer="idle"><span class="purl-pill__hold"></span><span class="purl-pill__keys"></span></span>
        <span class="purl-pill__layer" data-layer="listening"><span class="purl-pill__bars">${"<i></i>".repeat(BAR_COUNT)}</span><span class="purl-pill__timer">00:00</span></span>
        <span class="purl-pill__layer" data-layer="polishing"><span class="purl-pill__shimmer purl-pill__label">Polishing</span></span>
        <span class="purl-pill__layer" data-layer="sent">${checkSvg(18, "purl-pill__check")}<span class="purl-pill__label">Sent clean</span></span>
        <span class="purl-pill__layer" data-layer="error">${alertSvg(18, "purl-pill__icon")}<span class="purl-pill__msg"></span></span>
        <span class="purl-pill__layer" data-layer="warning">${alertSvg(18, "purl-pill__icon")}<span class="purl-pill__msg"></span></span>
      </div>
    </div>
    <span class="purl-sr-only"></span>`;

  const q = <T extends Element>(sel: string) => host.querySelector<T>(sel)!;
  const shell = q<HTMLElement>(".purl-pill__shell");
  const spoken = q<HTMLElement>(".purl-sr-only");
  const timer = q<HTMLElement>(".purl-pill__timer");
  const bars = [...host.querySelectorAll<HTMLElement>(".purl-pill__bars i")];
  const layer = (name: PillState) => q<HTMLElement>(`[data-layer="${name}"]`);

  let state: PillState = "idle";
  let target = 0;
  let smooth = 0;
  let listenStart = 0;
  let timerId = 0;
  let reduced = false;

  const renderKeys = (parts: string[]) => {
    q<HTMLElement>(".purl-pill__hold").textContent = options.idleLabel ?? "Hold";
    const holder = q<HTMLElement>(".purl-pill__keys");
    holder.replaceChildren(
      ...parts.map((part) => {
        const kbd = document.createElement("kbd");
        kbd.className = "purl-kbd";
        kbd.textContent = part;
        return kbd;
      }),
    );
  };

  // The capsule hugs the visible layer; the host keeps its reserved box.
  const fitWidth = () => {
    const styles = getComputedStyle(host);
    const pad = parseFloat(styles.getPropertyValue("--pill-pad-l")) + parseFloat(styles.getPropertyValue("--pill-pad-r"));
    shell.style.setProperty("--pill-w", `${Math.ceil(layer(state).offsetWidth + pad + 3)}px`);
  };

  const drawBars = (t: number) => {
    smooth += (target - smooth) * 0.35;
    host.style.setProperty("--lvl", smooth.toFixed(3));
    const mid = (BAR_COUNT - 1) / 2;
    bars.forEach((bar, i) => {
      const c = (i - mid) / (BAR_COUNT / 2);
      const envelope = smooth * (1 - c * c * 0.5);
      const grain = reduced ? 0.7 : 0.35 + 0.65 * valueNoise(i * 1.7 + t * 9, 3);
      bar.style.setProperty("--h", `${(BAR_MIN + envelope * grain * BAR_RANGE).toFixed(1)}px`);
      bar.style.setProperty("--e", Math.min(1, envelope * 1.2).toFixed(2));
    });
  };
  const loop = frameLoop(drawBars);

  const tickTimer = () => {
    timer.textContent = formatElapsed((performance.now() - listenStart) / 1000);
  };

  const syncLoops = () => {
    const listening = state === "listening";
    if (listening && !reduced) loop.start();
    else loop.stop();
    if (listening && reduced) drawBars(0);
    if (!listening) {
      target = smooth = 0;
      host.style.setProperty("--lvl", "0");
    }
    if (listening && !timerId) timerId = window.setInterval(tickTimer, 250);
    if (!listening && timerId) {
      clearInterval(timerId);
      timerId = 0;
    }
  };

  const enter = (next: PillState, announce: string) => {
    if (next === "listening" && state !== "listening") {
      listenStart = performance.now();
      tickTimer();
      bars.forEach((bar) => bar.style.setProperty("--h", `${BAR_MIN}px`));
    }
    state = next;
    host.dataset.state = next;
    spoken.textContent = announce;
    fitWidth();
    syncLoops();
  };

  const showMessage = (kind: "error" | "warning", message: string) => {
    const text = message.trim() || (kind === "error" ? "Something went wrong" : "Check your microphone");
    const msg = layer(kind).querySelector<HTMLElement>(".purl-pill__msg")!;
    msg.textContent = text;
    host.title = text;
    enter(kind, text);
  };

  const unwatch = watchReducedMotion((r) => {
    reduced = r;
    syncLoops();
  });

  renderKeys(keys);
  host.dataset.state = state;
  spoken.textContent = SPOKEN.idle;
  // Fonts change label widths; refit once they land.
  document.fonts?.ready.then(fitWidth);
  fitWidth();

  return {
    set(next, label) {
      if (!(next in SPOKEN)) throw new TypeError(`pill.set: unknown state "${String(next)}"`);
      if (label !== undefined && !LABELLED.has(next)) throw new TypeError(`pill.set: "${next}" takes no label`);
      const text = label?.trim() || SPOKEN[next];
      if (LABELLED.has(next)) layer(next).querySelector<HTMLElement>(".purl-pill__label")!.textContent = text;
      if (text === SPOKEN[next]) host.removeAttribute("title");
      else host.title = text;
      enter(next, text);
    },
    level(n) {
      target = Math.sqrt(clamp01(n)); // perceptual: quiet speech still shows
      if (reduced && state === "listening") {
        smooth = target; // no easing without motion
        drawBars(0);
      }
    },
    error: (message) => showMessage("error", message),
    warn: (message) => showMessage("warning", message),
    setHotkey(hotkey) {
      renderKeys(hotkeyParts(hotkey));
      fitWidth();
    },
    get state() {
      return state;
    },
    destroy() {
      loop.stop();
      clearInterval(timerId);
      unwatch();
      host.replaceChildren();
      host.classList.remove("purl-pill", "theme-dark");
      host.removeAttribute("data-state");
      host.removeAttribute("role");
      host.removeAttribute("title");
    },
  };
}

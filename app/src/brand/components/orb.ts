// Voice orb. Import orb.css alongside.
//
//   const orb = mountOrb(el, { size: 240 });
//   orb.set("listening"); orb.level(0.7); orb.set("polishing"); orb.set("done");

import { tickSvg } from "./glyphs";
import { clamp01, frameLoop, watchReducedMotion } from "./motion";

export type OrbState = "idle" | "listening" | "polishing" | "done";

export interface OrbOptions {
  /** Outer box in px (the glow can bleed past it). Default 240. */
  size?: number;
}

export interface OrbHandle {
  set(state: OrbState): void;
  /** Mic level 0..1. Swells the orb and the inner waveform. */
  level(n: number): void;
  readonly state: OrbState;
  destroy(): void;
}

const STATES: readonly OrbState[] = ["idle", "listening", "polishing", "done"];
const SPOKEN: Record<OrbState, string> = { idle: "Ready", listening: "Listening", polishing: "Polishing", done: "Sent clean" };

const WAVE_POINTS = 48;
const WAVE_STROKES = [
  { colour: "var(--cream)", width: 3, alpha: 0.95 },
  { colour: "var(--sprout)", width: 2, alpha: 0.6 },
  { colour: "var(--butter)", width: 1.5, alpha: 0.45 },
];

/** Waveform path in a 100x100 box. */
function wavePath(level: number, t: number, layer: number): string {
  const amp = 42 * (level * 1.1 + 0.07) * (1 - layer * 0.28);
  let d = "";
  for (let i = 0; i <= WAVE_POINTS; i++) {
    const u = i / WAVE_POINTS;
    const env = 1 - (2 * u - 1) ** 2;
    const y = 50 + Math.sin(i * 0.75 + t * (8 + layer * 2) + layer) * env * amp;
    d += `${i ? "L" : "M"}${(u * 100).toFixed(1)} ${y.toFixed(1)}`;
  }
  return d;
}

export function mountOrb(host: HTMLElement, options: OrbOptions = {}): OrbHandle {
  host.classList.add("purl-orb");
  host.setAttribute("role", "status");
  host.style.setProperty("--orb-size", `${options.size ?? 240}px`);
  host.innerHTML = `
    <span class="purl-orb__glow" aria-hidden="true"></span>
    <span class="purl-orb__ring" aria-hidden="true"></span>
    <span class="purl-orb__ring purl-orb__ring--b" aria-hidden="true"></span>
    <span class="purl-orb__spinner" aria-hidden="true"></span>
    <span class="purl-orb__ripple" aria-hidden="true"></span>
    <div class="purl-orb__core" aria-hidden="true">
      <svg class="purl-orb__wave" viewBox="0 0 100 100" preserveAspectRatio="none">${WAVE_STROKES.map(
        (s) => `<path style="stroke: ${s.colour}; opacity: ${s.alpha}" stroke-width="${s.width}" vector-effect="non-scaling-stroke"/>`,
      ).join("")}</svg>
      <div class="purl-orb__lines"><i></i><i></i><i></i></div>
      <div class="purl-orb__tick">${tickSvg(64)}</div>
    </div>
    <span class="purl-sr-only"></span>`;

  const paths = [...host.querySelectorAll<SVGPathElement>(".purl-orb__wave path")];
  const spoken = host.querySelector<HTMLElement>(".purl-sr-only")!;

  let state: OrbState = "idle";
  let target = 0;
  let smooth = 0;
  let reduced = false;

  const draw = (t: number) => {
    smooth += (target - smooth) * 0.3;
    host.style.setProperty("--lvl", smooth.toFixed(3));
    paths.forEach((p, layer) => p.setAttribute("d", wavePath(smooth, t, layer)));
  };
  const loop = frameLoop(draw);

  const sync = () => {
    const listening = state === "listening";
    if (listening && !reduced) loop.start();
    else loop.stop();
    if (listening && reduced) {
      smooth = target;
      draw(0);
    }
    if (!listening) {
      target = smooth = 0;
      host.style.setProperty("--lvl", "0");
    }
  };

  const unwatch = watchReducedMotion((r) => {
    reduced = r;
    sync();
  });

  const enter = (next: OrbState) => {
    state = next;
    host.dataset.state = next;
    spoken.textContent = SPOKEN[next];
    sync();
  };
  enter("idle");
  draw(0);

  return {
    set(next) {
      if (!STATES.includes(next)) throw new TypeError(`orb.set: unknown state "${String(next)}"`);
      enter(next);
    },
    level(n) {
      target = Math.sqrt(clamp01(n)); // perceptual: quiet speech still shows
      if (reduced && state === "listening") sync();
    },
    get state() {
      return state;
    },
    destroy() {
      loop.stop();
      unwatch();
      host.replaceChildren();
      host.classList.remove("purl-orb");
      host.removeAttribute("data-state");
      host.removeAttribute("role");
      host.style.removeProperty("--orb-size");
      host.style.removeProperty("--lvl");
    },
  };
}

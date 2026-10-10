// Shared motion helpers for the live components.

const QUERY = "(prefers-reduced-motion: reduce)";

/** Calls `onChange` now and whenever the reduced-motion preference flips. Returns an unsubscribe. */
export function watchReducedMotion(onChange: (reduced: boolean) => void): () => void {
  const media = matchMedia(QUERY);
  const handler = () => onChange(media.matches);
  handler();
  media.addEventListener("change", handler);
  return () => media.removeEventListener("change", handler);
}

export const clamp01 = (x: number) => Math.min(1, Math.max(0, Number.isFinite(x) ? x : 0));

/** Smooth 1-D value noise in [0, 1]; same shape as the motion pack's vnoise. */
export function valueNoise(x: number, seed = 0): number {
  const hash = (n: number) => {
    const v = Math.sin((n + seed * 131.7) * 127.1) * 43758.5453;
    return v - Math.floor(v);
  };
  const i = Math.floor(x);
  const f = x - i;
  const u = f * f * (3 - 2 * f);
  return hash(i) + (hash(i + 1) - hash(i)) * u;
}

/** Runs `frame(timeSeconds)` on every animation frame until stopped. */
export function frameLoop(frame: (t: number) => void): { start(): void; stop(): void } {
  let id = 0;
  const tick = (now: number) => {
    frame(now / 1000);
    id = requestAnimationFrame(tick);
  };
  return {
    start() {
      if (!id) id = requestAnimationFrame(tick);
    },
    stop() {
      cancelAnimationFrame(id);
      id = 0;
    },
  };
}

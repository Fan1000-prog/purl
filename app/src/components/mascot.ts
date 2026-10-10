// Purl, the little p, as a state indicator. Each state is a self-contained
// animated SVG (CSS keyframes inside the file), shown through an <img> so its
// styles never leak into the page. Swapping state swaps `src` inside a
// fixed-size box, so the surrounding layout never moves.
//
// Static fallbacks, per the character README:
// - below 40 px the loops' waves and sparkles turn to noise, so it is the
//   static idle pose whatever the state;
// - under prefers-reduced-motion it is the static pose for the current state,
//   and it follows the OS setting live.

export type MascotState = "idle" | "listening" | "thinking" | "done";

export interface MascotOptions {
  size: number;
  /** The field the mascot sits on. The loops are transparent, so this only
   *  tags the box (`data-surface`) for styling; the body stays Fern. */
  surface: "cream" | "deep";
}

export interface Mascot {
  set(state: MascotState): void;
  destroy(): void;
}

const SMALL_SIZE = 40;
const BASE = `${import.meta.env.BASE_URL}mascot/`;
const STATES: MascotState[] = ["idle", "listening", "thinking", "done"];

export function mascotSrc(state: MascotState, size: number, reducedMotion: boolean): string {
  if (size < SMALL_SIZE) return `${BASE}purl-pose-idle.png`;
  if (reducedMotion) return `${BASE}purl-pose-${state}.png`;
  return `${BASE}purl-${state}.svg`;
}

export function mountMascot(el: HTMLElement, opts: MascotOptions): Mascot {
  const box = document.createElement("span");
  box.className = "mascot";
  box.dataset.surface = opts.surface;
  box.style.cssText = `display:inline-block;flex:none;width:${opts.size}px;height:${opts.size}px;`;

  const img = document.createElement("img");
  img.alt = "";
  img.width = opts.size;
  img.height = opts.size;
  img.draggable = false;
  img.style.cssText = "display:block;width:100%;height:100%;";
  box.appendChild(img);
  el.appendChild(box);

  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let state: MascotState = "idle";

  const render = () => {
    box.dataset.state = state;
    img.src = mascotSrc(state, opts.size, motion.matches);
  };
  motion.addEventListener("change", render);

  // Warm the cache so the first swap to each state doesn't blank for a frame.
  if (opts.size >= SMALL_SIZE && !motion.matches) {
    for (const s of STATES) new Image().src = mascotSrc(s, opts.size, false);
  }

  render();

  return {
    set(next) {
      if (next === state) return;
      state = next;
      render();
    },
    destroy() {
      motion.removeEventListener("change", render);
      box.remove();
    },
  };
}

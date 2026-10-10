/** Confetti burst + toast for "you just did a meaningful thing" moments —
 *  saved a snippet, created a transform, set a hotkey, ran a transform.
 *  Vanilla DOM/CSS, no deps; respects prefers-reduced-motion. */

const COLORS = ["var(--green)", "var(--rust)", "var(--mustard)", "var(--ink)"];
const TOAST_MS = 2600;

let root: HTMLDivElement | null = null;

function getRoot(): HTMLDivElement {
  if (root) return root;
  root = document.createElement("div");
  root.id = "celebrate-root";
  document.body.appendChild(root);
  return root;
}

export interface ConfettiOptions {
  colors?: string[];
  pieceCount?: number;
  /** Horizontal spread in px either side of center. */
  spread?: number;
  /** How far pieces fall in px. */
  fallDistance?: number;
}

/** A burst of falling confetti pieces appended to `container`, self-removing
 *  once the animation finishes. Exported so the overlay's small pill window
 *  (its own document, its own palette) can reuse the same mechanic. */
export function burstConfetti(container: HTMLElement, opts: ConfettiOptions = {}) {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const colors = opts.colors ?? COLORS;
  const pieceCount = opts.pieceCount ?? 26;
  const spread = opts.spread ?? 220;
  const fallDistance = opts.fallDistance ?? 160;

  const burst = document.createElement("div");
  burst.className = "confetti-burst";
  for (let i = 0; i < pieceCount; i++) {
    const piece = document.createElement("span");
    piece.className = "confetti-piece";
    const x = (Math.random() - 0.5) * spread;
    const rot = Math.random() * 360;
    const delay = Math.random() * 0.12;
    const duration = 0.9 + Math.random() * 0.5;
    piece.style.setProperty("--x", `${x}px`);
    piece.style.setProperty("--rot", `${rot}deg`);
    piece.style.setProperty("--delay", `${delay}s`);
    piece.style.setProperty("--duration", `${duration}s`);
    piece.style.setProperty("--fall", `${fallDistance}px`);
    piece.style.background = colors[i % colors.length];
    burst.appendChild(piece);
  }
  container.appendChild(burst);
  setTimeout(() => burst.remove(), 1600);
}

/** Fire a confetti burst + a short-lived toast with the given message. */
export function celebrate(message: string) {
  const container = getRoot();
  burstConfetti(container);
  celebrateToast(container, message);
}

function celebrateToast(container: HTMLElement, message: string) {
  const toast = document.createElement("div");
  toast.className = "celebrate-toast";
  toast.textContent = message;
  container.appendChild(toast);
  requestAnimationFrame(() => toast.classList.add("in"));
  setTimeout(() => {
    toast.classList.remove("in");
    setTimeout(() => toast.remove(), 200);
  }, TOAST_MS);
}

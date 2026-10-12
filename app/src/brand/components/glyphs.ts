// SVG glyphs shared by the pill and the orb. Geometry is the locked Purl
// mark (512 box): bead at (264, 218) r28, sound arc r54.

/** Breath tick: bead + sound arc, cropped to its own box. */
export function tickSvg(size: number): string {
  return `<svg width="${size}" height="${size}" viewBox="230 160 110 116" aria-hidden="true" focusable="false">
  <circle cx="264" cy="218" r="28" style="fill: var(--butter)"/>
  <path d="M306.6 184.8 A54 54 0 0 1 306.6 251.2" fill="none" style="stroke: var(--sprout)" stroke-width="26" stroke-linecap="round"/>
</svg>`;
}

export function checkSvg(size: number, className: string): string {
  return `<svg class="${className}" width="${size}" height="${size}" viewBox="0 0 20 20" aria-hidden="true" focusable="false">
  <circle cx="10" cy="10" r="10"/><path d="M5.5 10.4l3 3 6-6.4"/>
</svg>`;
}

/** Round alert mark in currentColor. */
export function alertSvg(size: number, className: string): string {
  return `<svg class="${className}" width="${size}" height="${size}" viewBox="0 0 20 20" aria-hidden="true" focusable="false">
  <circle cx="10" cy="10" r="9" fill="none" stroke="currentColor" stroke-width="2"/>
  <path d="M10 5.5v5.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
  <circle cx="10" cy="14.2" r="1.2" fill="currentColor"/>
</svg>`;
}

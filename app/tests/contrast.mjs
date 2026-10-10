// WCAG AA check for every text/background pair the UI actually uses, in both
// themes. Values come straight from src/styles/tokens.css, and every
// `color: var(--x)` in the stylesheets must name a foreground listed here,
// so a new text colour can't ship unchecked. Run: node tests/contrast.mjs

import { readFileSync } from "node:fs";

const AA_TEXT = 4.5;
const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

// [text token, background token, where it's used]
const PAIRS = [
  ["text", "bg", "body copy, list rows inside panels"],
  ["text", "surface", "panels, cards, inputs, kbd, toasts"],
  ["muted", "bg", "nav, ledes, empty states, segmented options"],
  ["muted", "surface", "help text, meta rows, stat labels, quiet buttons"],
  ["accent", "bg", "links, focus rings"],
  ["accent", "surface", "Ready / Allowed status"],
  ["accent-ink", "accent", "primary buttons, selected segment"],
  ["warn-text", "bg", "pipeline errors on Home"],
  ["warn-text", "surface", "error states, failed downloads, armed buttons"],
  ["bg", "text", "Start dictation while recording (inverted)"],
];

// The overlay pins the light palette (.theme-light): checked in light only.
const OVERLAY_PAIRS = [
  ["ink", "paper", "pill label"],
  ["warn-text", "paper", "pill error label"],
  ["ink", "cream", "cancel button"],
  ["paper", "fern", "confirm button"],
];

const STYLESHEETS = ["src/styles/tokens.css", "src/styles/base.css", "src/styles/onboarding.css", "src/styles.css", "overlay.html"];

const css = read("src/styles/tokens.css");

function declarations(block) {
  const vars = {};
  for (const [, name, value] of block.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) vars[name] = value.trim();
  return vars;
}

/** The declarations of the first rule whose selector list starts with `selector`. */
function block(selector) {
  const at = css.search(new RegExp(`^\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`, "m"));
  if (at < 0) throw new Error(`tokens.css: no rule starting "${selector}"`);
  const open = css.indexOf("{", at);
  return css.slice(open + 1, css.indexOf("}", open));
}

const light = declarations(block(":root,"));
const dark = { ...light, ...declarations(block(":root:not(.theme-light)")) };

function resolve(vars, name, seen = new Set()) {
  if (seen.has(name)) throw new Error(`cycle at --${name}`);
  seen.add(name);
  const value = vars[name];
  if (value === undefined) throw new Error(`--${name} is not defined`);
  const ref = value.match(/^var\(--([\w-]+)\)$/);
  return ref ? resolve(vars, ref[1], seen) : value;
}

function luminance(hex) {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) throw new Error(`expected #rrggbb, got ${hex}`);
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(m[1].slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

let failures = 0;
function check(theme, vars, pairs) {
  for (const [fg, bg, use] of pairs) {
    const ratio = contrast(resolve(vars, fg), resolve(vars, bg));
    const ok = ratio >= AA_TEXT;
    if (!ok) failures++;
    console.log(`${ok ? "pass" : "FAIL"}  ${theme.padEnd(5)}  --${fg} on --${bg}  ${ratio.toFixed(2)}:1  (${use})`);
  }
}
check("light", light, PAIRS);
check("dark", dark, PAIRS);
check("light", light, OVERLAY_PAIRS);

// Every foreground colour in the stylesheets must be one checked above.
const checked = new Set([...PAIRS, ...OVERLAY_PAIRS].map(([fg]) => fg));
for (const file of STYLESHEETS) {
  for (const [, name] of read(file).matchAll(/(?:^|[^-\w])color:\s*var\(--([\w-]+)\)/g)) {
    if (!checked.has(name)) {
      failures++;
      console.log(`FAIL  ${file}: color: var(--${name}) is not in PAIRS`);
    }
  }
}

if (failures) {
  console.error(`\n${failures} problem(s): pairs below WCAG AA ${AA_TEXT}:1 or unchecked text colours`);
  process.exit(1);
}
console.log(`\nAll ${PAIRS.length * 2 + OVERLAY_PAIRS.length} pairs meet WCAG AA, and every text colour is covered.`);

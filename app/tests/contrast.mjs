// WCAG AA check for every text/background pair the UI actually uses, in both
// themes. Values come straight from the vendored brand tokens
// (src/brand/tokens.css) plus the app layer (src/styles/tokens.css), and
// every `color: var(--x)` in the stylesheets must name a foreground listed
// here, so a new text colour can't ship unchecked. Run: node tests/contrast.mjs

import { readFileSync } from "node:fs";

const AA_TEXT = 4.5;
const AA_NON_TEXT = 3;
const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

// [text token, background token, where it's used]
const PAIRS = [
  ["text", "bg", "body copy, list rows inside panels"],
  ["text", "surface", "panels, cards, inputs, kbd, toasts"],
  ["muted", "bg", "nav, ledes, empty states, segmented options"],
  ["muted", "surface", "help text, meta rows, stat labels, eyebrows, quiet buttons"],
  ["accent", "bg", "links"],
  ["accent", "surface", "Ready / Allowed status, transform plus"],
  ["accent-ink", "accent", "primary buttons, selected segment, switch knob"],
  ["warn-text", "bg", "pipeline errors on Home"],
  ["warn-text", "surface", "error states, failed downloads, armed buttons"],
  ["bg", "text", "Start dictation while recording (inverted)"],
];

// Non-text graphics that carry meaning: 3:1.
const GRAPHICS = [
  ["focus", "bg", "focus ring on the page"],
  ["focus", "surface", "focus ring on panels"],
  ["warn", "surface", "banner and error-state borders"],
  ["muted", "bg", "switch knob when off"],
  ["control-line", "bg", "switch edge"],
  ["control-line", "surface", "input and select edges"],
];

// The Home hero lays translucent field tints over --surface; text there is
// checked against the strongest tint, composited.
const HERO_TEXT = ["text", "muted", "accent"];
const HERO_FIELDS = ["field-a", "field-b"];

// The overlay is always dark (.theme-dark): checked in dark only. The pill
// itself is the brand component, checked by the brand's own contrast test.
const OVERLAY_PAIRS = [
  ["text", "surface", "cancel button"],
  ["night", "signal", "confirm button"],
];

const STYLESHEETS = ["src/styles/tokens.css", "src/styles/base.css", "src/styles/onboarding.css", "src/styles.css", "overlay.html"];

const brand = read("src/brand/tokens.css");
const app = read("src/styles/tokens.css");

function declarations(css, selector) {
  const at = css.search(new RegExp(`^\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`, "m"));
  if (at < 0) throw new Error(`no rule starting "${selector}"`);
  const open = css.indexOf("{", at);
  const body = css.slice(open + 1, css.indexOf("}", open));
  const vars = {};
  for (const [, name, value] of body.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) vars[name] = value.trim();
  return vars;
}

let failures = 0;
let checks = 0;

const LIGHT = '.theme-light,\n[data-theme="light"]';
const raw = declarations(brand, ":root {");
const themes = {
  dark: { ...raw, ...declarations(brand, ":root,\n.theme-dark"), ...declarations(app, ":root,\n.theme-dark") },
  light: { ...raw, ...declarations(brand, LIGHT), ...declarations(app, LIGHT) },
};

// data-theme="auto" (the main window) repeats the light values under a
// prefers-color-scheme media query, in both the brand file and the app
// layer. A drift there would ship a light theme nobody checked.
for (const [file, css] of [["src/brand/tokens.css", brand], ["src/styles/tokens.css", app]]) {
  const lightBlock = declarations(css, LIGHT);
  const autoBlock = declarations(css, '[data-theme="auto"] {');
  for (const name of new Set([...Object.keys(lightBlock), ...Object.keys(autoBlock)])) {
    if (lightBlock[name] !== autoBlock[name]) {
      failures++;
      console.log(`FAIL  ${file}: --${name} is "${lightBlock[name]}" in light but "${autoBlock[name]}" in auto`);
    }
  }
}

function resolve(vars, name, seen = new Set()) {
  if (seen.has(name)) throw new Error(`cycle at --${name}`);
  seen.add(name);
  const value = vars[name];
  if (value === undefined) throw new Error(`--${name} is not defined`);
  const ref = value.match(/^var\(--([\w-]+)\)$/);
  return ref ? resolve(vars, ref[1], seen) : value;
}

/** [r, g, b, a] from #rrggbb or rgb(r g b / a). */
function rgba(color) {
  const hex = color.match(/^#([0-9a-f]{6})$/i);
  if (hex) return [0, 2, 4].map((i) => parseInt(hex[1].slice(i, i + 2), 16)).concat(1);
  const fn = color.match(/^rgb\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)\s*(?:\/\s*([\d.]+))?\s*\)$/);
  if (fn) return [Number(fn[1]), Number(fn[2]), Number(fn[3]), fn[4] === undefined ? 1 : Number(fn[4])];
  throw new Error(`expected #rrggbb or rgb(), got ${color}`);
}

function over(top, under) {
  const [r, g, b, a] = rgba(top);
  const [r2, g2, b2] = rgba(under);
  const mix = (x, y) => Math.round(x * a + y * (1 - a));
  return [mix(r, r2), mix(g, g2), mix(b, b2), 1];
}

function luminance([r, g, b]) {
  const lin = (v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

function report(ok, theme, label, ratio, use) {
  checks++;
  if (!ok) failures++;
  console.log(`${ok ? "pass" : "FAIL"}  ${theme.padEnd(5)}  ${label}  ${ratio.toFixed(2)}:1  (${use})`);
}

function check(theme, pairs, min) {
  const vars = themes[theme];
  for (const [fg, bg, use] of pairs) {
    const ratio = contrast(rgba(resolve(vars, fg)), rgba(resolve(vars, bg)));
    report(ratio >= min, theme, `--${fg} on --${bg}`, ratio, use);
  }
}

for (const theme of ["dark", "light"]) {
  check(theme, PAIRS, AA_TEXT);
  check(theme, GRAPHICS, AA_NON_TEXT);
  const vars = themes[theme];
  for (const field of HERO_FIELDS) {
    const bg = over(resolve(vars, field), resolve(vars, "surface"));
    for (const fg of HERO_TEXT) {
      const ratio = contrast(rgba(resolve(vars, fg)), bg);
      report(ratio >= AA_TEXT, theme, `--${fg} on --${field} over --surface`, ratio, "Home hero");
    }
  }
}
check("dark", OVERLAY_PAIRS, AA_TEXT);

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
  console.error(`\n${failures} problem(s): pairs below WCAG AA or unchecked text colours`);
  process.exit(1);
}
console.log(`\nAll ${checks} pairs meet WCAG AA (text ${AA_TEXT}:1, graphics ${AA_NON_TEXT}:1), and every text colour is covered.`);

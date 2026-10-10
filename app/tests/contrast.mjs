// WCAG AA check for every text/background pair the design tokens promise,
// in both themes. Reads the values straight from src/styles/tokens.css so the
// test can't drift from the stylesheet. Run: node tests/contrast.mjs

import { readFileSync } from "node:fs";

const AA_TEXT = 4.5;

// [text token, background token]
const PAIRS = [
  ["text", "bg"],
  ["text", "surface"],
  ["muted", "bg"],
  ["muted", "surface"],
  ["accent", "bg"],
  ["accent", "surface"],
  ["accent-ink", "accent"],
  ["warn-text", "bg"],
  ["warn-text", "surface"],
];

const css = readFileSync(new URL("../src/styles/tokens.css", import.meta.url), "utf8");

function declarations(block) {
  const vars = {};
  for (const [, name, value] of block.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) vars[name] = value.trim();
  return vars;
}

function blockAfter(marker) {
  const at = css.indexOf(marker);
  if (at < 0) throw new Error(`tokens.css: "${marker}" not found`);
  const open = css.indexOf("{", at + marker.length);
  const close = css.indexOf("}", open);
  return css.slice(open + 1, close);
}

const light = declarations(blockAfter(".theme-light"));
const dark = { ...light, ...declarations(blockAfter(":root:not(.theme-light)")) };

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
for (const [theme, vars] of [["light", light], ["dark", dark]]) {
  for (const [fg, bg] of PAIRS) {
    const ratio = contrast(resolve(vars, fg), resolve(vars, bg));
    const ok = ratio >= AA_TEXT;
    if (!ok) failures++;
    console.log(`${ok ? "pass" : "FAIL"}  ${theme.padEnd(5)}  --${fg} on --${bg}  ${ratio.toFixed(2)}:1`);
  }
}

if (failures) {
  console.error(`\n${failures} pair(s) below WCAG AA ${AA_TEXT}:1`);
  process.exit(1);
}
console.log(`\nAll ${PAIRS.length * 2} pairs meet WCAG AA.`);

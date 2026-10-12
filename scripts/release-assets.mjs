#!/usr/bin/env node
// Release asset names, and the step that copies tauri's bundle output to them.
//
// Names are version-free so the landing page and README can link to
// /releases/latest/download/<name> without knowing the version.
//
//   node scripts/release-assets.mjs collect --target <triple> --out <dir> [--no-updater]
//   node scripts/release-assets.mjs check-tag <tag>
import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_DIR = join(ROOT, "app", "src-tauri");

// Per build target: the updater platform key, the installer, and the file the
// updater downloads (signed by tauri into "<file>.sig"). Paths under `from`
// are relative to target/<triple>/release/bundle; {version} is the app version.
export const TARGETS = {
  "aarch64-apple-darwin": {
    platform: "darwin-aarch64",
    installer: { from: "dmg/Purl_{version}_aarch64.dmg", to: "Purl_aarch64.dmg" },
    updater: { from: "macos/Purl.app.tar.gz", to: "Purl_aarch64.app.tar.gz" },
  },
  "x86_64-pc-windows-msvc": {
    platform: "windows-x86_64",
    installer: { from: "nsis/Purl_{version}_x64-setup.exe", to: "Purl_x64-setup.exe" },
    // Tauri v2 updates Windows by running the NSIS installer itself.
    updater: { from: "nsis/Purl_{version}_x64-setup.exe", to: "Purl_x64-setup.exe" },
  },
};

/** The version in tauri.conf.json: what the built app reports. */
export function appVersion() {
  return JSON.parse(readFileSync(join(TAURI_DIR, "tauri.conf.json"), "utf8")).version;
}

/** True when `tag` is v<version>, optionally with a pre-release suffix (v0.9.0-rc.1). */
export function tagMatchesVersion(tag, version) {
  const m = /^v(\d+\.\d+\.\d+)(-[0-9A-Za-z.-]+)?$/.exec(tag);
  return m !== null && m[1] === version;
}

/** [from, to] pairs to copy for one target. Throws on an unknown target. */
export function assetPlan(target, version, { updater = true } = {}) {
  const t = TARGETS[target];
  if (!t) throw new Error(`unknown target ${target}; expected one of ${Object.keys(TARGETS).join(", ")}`);
  const fill = (p) => p.replaceAll("{version}", version);
  const pairs = [[fill(t.installer.from), t.installer.to]];
  if (updater) {
    if (t.updater.to !== t.installer.to) pairs.push([fill(t.updater.from), t.updater.to]);
    pairs.push([`${fill(t.updater.from)}.sig`, `${t.updater.to}.sig`]);
  }
  return pairs;
}

function collect(target, out, updater) {
  const bundle = join(TAURI_DIR, "target", target, "release", "bundle");
  const pairs = assetPlan(target, appVersion(), { updater });
  const missing = pairs.map(([from]) => join(bundle, from)).filter((p) => !existsSync(p));
  if (missing.length) throw new Error(`tauri build output is missing:\n  ${missing.join("\n  ")}`);
  mkdirSync(out, { recursive: true });
  for (const [from, to] of pairs) {
    copyFileSync(join(bundle, from), join(out, to));
    console.log(`${from} -> ${to}`);
  }
}

function flag(args, name) {
  const i = args.indexOf(name);
  if (i === -1 || i + 1 >= args.length) throw new Error(`missing ${name}`);
  return args[i + 1];
}

function main(argv) {
  const [cmd, ...args] = argv;
  if (cmd === "collect") {
    collect(flag(args, "--target"), flag(args, "--out"), !args.includes("--no-updater"));
  } else if (cmd === "check-tag") {
    const version = appVersion();
    if (!tagMatchesVersion(args[0] ?? "", version)) {
      throw new Error(`tag ${args[0]} doesn't match app version ${version} in tauri.conf.json`);
    }
    console.log(`tag ${args[0]} matches app version ${version}`);
  } else {
    throw new Error("usage: release-assets.mjs collect --target <triple> --out <dir> [--no-updater] | check-tag <tag>");
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`release-assets: ${e.message}`);
    process.exit(1);
  }
}

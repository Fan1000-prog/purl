#!/usr/bin/env node
// Write the updater manifest (latest.json) for a release.
//
//   node scripts/make-latest-json.mjs --tag <tag> --repo <owner/name> --dir <assets> --out <file> [--notes <text>]
//
// <assets> holds the collected release assets (see release-assets.mjs); each
// platform's updater file must have its "<file>.sig" beside it.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { TARGETS, appVersion } from "./release-assets.mjs";

/**
 * The manifest tauri-plugin-updater reads. `signatures` maps an updater
 * platform key to the contents of its .sig file; every target must be there.
 */
export function buildManifest({ version, notes, pubDate, baseUrl, signatures }) {
  const platforms = {};
  for (const { platform, updater } of Object.values(TARGETS)) {
    const signature = signatures[platform]?.trim();
    if (!signature) throw new Error(`no updater signature for ${platform}`);
    platforms[platform] = { signature, url: `${baseUrl}/${updater.to}` };
  }
  return { version, notes, pub_date: pubDate, platforms };
}

/** Read every target's .sig from `dir`. */
export function readSignatures(dir) {
  const signatures = {};
  for (const { platform, updater } of Object.values(TARGETS)) {
    const sig = join(dir, `${updater.to}.sig`);
    if (!existsSync(sig)) throw new Error(`missing ${sig}`);
    signatures[platform] = readFileSync(sig, "utf8");
  }
  return signatures;
}

function flag(args, name, fallback) {
  const i = args.indexOf(name);
  if (i !== -1 && i + 1 < args.length) return args[i + 1];
  if (fallback !== undefined) return fallback;
  throw new Error(`missing ${name}`);
}

function main(args) {
  const tag = flag(args, "--tag");
  const version = appVersion();
  const manifest = buildManifest({
    version,
    notes: flag(args, "--notes", `Purl ${version}`),
    pubDate: new Date().toISOString(),
    baseUrl: `https://github.com/${flag(args, "--repo")}/releases/download/${tag}`,
    signatures: readSignatures(flag(args, "--dir")),
  });
  writeFileSync(flag(args, "--out"), `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(JSON.stringify(manifest, null, 2));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`make-latest-json: ${e.message}`);
    process.exit(1);
  }
}

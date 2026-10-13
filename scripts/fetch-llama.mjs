#!/usr/bin/env node
// Fetch the pinned llama.cpp release and stage llama-server plus the
// libraries it loads into app/src-tauri/vendor/llama/, which the app bundles
// as `llama/`. Run from anywhere: `node scripts/fetch-llama.mjs`.
//
// Pinned: llama.cpp b11429 (the build behind release v0.6.0, 2026-10-05).
// sha256 of each archive, checked against GitHub's published asset digests:
//   llama-b11429-bin-macos-arm64.tar.gz   740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459
//   llama-b11429-bin-win-cpu-x64.zip      1283323272b04cd07905816a597a0da810918102de958f4ff6f7bbaa70ed2efe
//
// PURL_TARGET (aarch64-apple-darwin | x86_64-pc-windows-msvc) picks the
// archive; it defaults to the host. Skips the download when vendor/llama/
// already holds this tag for this target.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TAG = "b11429";
const ARCHIVES = {
  "aarch64-apple-darwin": {
    name: `llama-${TAG}-bin-macos-arm64.tar.gz`,
    sha256: "740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459",
  },
  "x86_64-pc-windows-msvc": {
    name: `llama-${TAG}-bin-win-cpu-x64.zip`,
    sha256: "1283323272b04cd07905816a597a0da810918102de958f4ff6f7bbaa70ed2efe",
  },
};
// The archive is a few tens of MB; anything far bigger is not what we pinned.
const MAX_BYTES = 200 * 1024 * 1024;
const MAX_REDIRECTS = 5;

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VENDOR = path.join(ROOT, "app", "src-tauri", "vendor", "llama");
const STAMP = path.join(VENDOR, ".fetched");

function hostTarget() {
  if (process.platform === "darwin" && process.arch === "arm64") return "aarch64-apple-darwin";
  if (process.platform === "win32" && process.arch === "x64") return "x86_64-pc-windows-msvc";
  return null;
}

/** Download `url` (following redirects) to `dest`; resolves to its sha256. */
function download(url, dest, redirects = 0) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "purl-fetch-llama" } }, (res) => {
        if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location) {
          res.resume();
          if (redirects >= MAX_REDIRECTS) return reject(new Error(`too many redirects for ${url}`));
          const next = new URL(res.headers.location, url).toString();
          return resolve(download(next, dest, redirects + 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`GET ${url} -> HTTP ${res.statusCode}`));
        }
        const hash = createHash("sha256");
        const out = fs.createWriteStream(dest);
        let bytes = 0;
        res.on("data", (chunk) => {
          bytes += chunk.length;
          if (bytes > MAX_BYTES) {
            res.destroy(new Error(`${url} is larger than ${MAX_BYTES} bytes`));
            return;
          }
          hash.update(chunk);
        });
        res.on("error", reject);
        out.on("error", reject);
        out.on("finish", () => resolve(hash.digest("hex")));
        res.pipe(out);
      })
      .on("error", reject);
  });
}

/** The files llama-server needs at run time, from an extracted archive dir. */
function wanted(name, target) {
  if (target === "x86_64-pc-windows-msvc") {
    if (name === "llama-server.exe" || name.startsWith("LICENSE")) return true;
    // Every runtime DLL (ggml CPU variants are picked at run time), but not
    // the other tools' implementation libraries.
    return name.endsWith(".dll") && (!name.endsWith("-impl.dll") || name === "llama-server-impl.dll");
  }
  // macOS: the binary, its server implementation and the `lib*.0.dylib`
  // names it links against (`@rpath`, resolved next to the binary).
  return (
    name === "llama-server" ||
    name === "LICENSE" ||
    name === "libllama-server-impl.dylib" ||
    /^lib[\w-]+\.0\.dylib$/.test(name)
  );
}

/** bsdtar, which reads both .tar.gz and .zip and ships with macOS and
 *  Windows 10+. On Windows it is named by full path: under Git Bash a plain
 *  "tar" is GNU tar, which reads `C:\...` as host:path and can't open a zip. */
export function tarCommand(platform = process.platform, env = process.env) {
  if (platform !== "win32") return "tar";
  return path.win32.join(env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
}

async function main() {
  const target = process.env.PURL_TARGET || hostTarget();
  const archive = ARCHIVES[target];
  if (!archive) {
    console.error(
      `fetch-llama: no llama.cpp build for ${target ?? `${process.platform}-${process.arch}`}. ` +
        `Purl ships for ${Object.keys(ARCHIVES).join(" and ")}; set PURL_TARGET to one of them.`,
    );
    process.exit(1);
  }
  const stamp = `${TAG} ${target}`;
  if (fs.existsSync(STAMP) && fs.readFileSync(STAMP, "utf8").trim() === stamp) {
    console.log(`fetch-llama: ${stamp} already in place`);
    return;
  }

  const work = fs.mkdtempSync(path.join(os.tmpdir(), "purl-llama-"));
  try {
    const file = path.join(work, archive.name);
    const url = `https://github.com/ggml-org/llama.cpp/releases/download/${TAG}/${archive.name}`;
    console.log(`fetch-llama: downloading ${url}`);
    const sha = await download(url, file);
    if (sha !== archive.sha256) {
      throw new Error(`${archive.name}: sha256 ${sha}, expected ${archive.sha256}`);
    }

    const extracted = path.join(work, "x");
    fs.mkdirSync(extracted);
    const tar = spawnSync(tarCommand(), ["-xf", file, "-C", extracted], { stdio: "inherit" });
    if (tar.status !== 0) throw new Error(`tar failed to extract ${archive.name}`);

    const top = fs.readdirSync(extracted);
    const dir = top.length === 1 && fs.statSync(path.join(extracted, top[0])).isDirectory()
      ? path.join(extracted, top[0])
      : extracted;

    for (const old of fs.readdirSync(VENDOR)) {
      if (old !== ".gitkeep") fs.rmSync(path.join(VENDOR, old), { recursive: true, force: true });
    }
    let copied = 0;
    for (const name of fs.readdirSync(dir)) {
      if (!wanted(name, target)) continue;
      // copyFileSync follows symlinks, so `lib*.0.dylib` lands as a real file.
      fs.copyFileSync(path.join(dir, name), path.join(VENDOR, name));
      copied += 1;
    }
    const server = path.join(VENDOR, target.includes("windows") ? "llama-server.exe" : "llama-server");
    if (!fs.existsSync(server)) throw new Error(`${archive.name} has no llama-server`);
    fs.chmodSync(server, 0o755);
    fs.writeFileSync(STAMP, `${stamp}\n`);
    console.log(`fetch-llama: staged ${copied} files for ${stamp} in ${path.relative(ROOT, VENDOR)}`);
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((e) => {
    console.error(`fetch-llama: ${e.message}`);
    process.exit(1);
  });
}

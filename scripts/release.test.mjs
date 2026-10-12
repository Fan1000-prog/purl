// node --test scripts/*.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TARGETS, assetPlan, tagMatchesVersion } from "./release-assets.mjs";
import { buildManifest, readSignatures } from "./make-latest-json.mjs";

test("release ships exactly macOS Apple Silicon and Windows x64", () => {
  assert.deepEqual(
    Object.values(TARGETS).map((t) => t.platform).sort(),
    ["darwin-aarch64", "windows-x86_64"],
  );
});

test("installer names are version-free, for /releases/latest/download links", () => {
  assert.equal(TARGETS["aarch64-apple-darwin"].installer.to, "Purl_aarch64.dmg");
  assert.equal(TARGETS["x86_64-pc-windows-msvc"].installer.to, "Purl_x64-setup.exe");
  for (const t of Object.values(TARGETS)) {
    assert.doesNotMatch(t.installer.to + t.updater.to, /\d+\.\d+/);
  }
});

test("mac plan copies dmg, updater archive and its signature", () => {
  assert.deepEqual(assetPlan("aarch64-apple-darwin", "1.2.3"), [
    ["dmg/Purl_1.2.3_aarch64.dmg", "Purl_aarch64.dmg"],
    ["macos/Purl.app.tar.gz", "Purl_aarch64.app.tar.gz"],
    ["macos/Purl.app.tar.gz.sig", "Purl_aarch64.app.tar.gz.sig"],
  ]);
});

test("windows plan copies the installer once, plus its signature", () => {
  assert.deepEqual(assetPlan("x86_64-pc-windows-msvc", "1.2.3"), [
    ["nsis/Purl_1.2.3_x64-setup.exe", "Purl_x64-setup.exe"],
    ["nsis/Purl_1.2.3_x64-setup.exe.sig", "Purl_x64-setup.exe.sig"],
  ]);
});

test("unsigned builds copy only the installer", () => {
  assert.deepEqual(assetPlan("aarch64-apple-darwin", "1.2.3", { updater: false }), [
    ["dmg/Purl_1.2.3_aarch64.dmg", "Purl_aarch64.dmg"],
  ]);
});

test("unknown target is refused", () => {
  assert.throws(() => assetPlan("x86_64-apple-darwin", "1.2.3"), /unknown target/);
});

test("tag must match the app version, pre-release suffix allowed", () => {
  assert.ok(tagMatchesVersion("v0.9.0", "0.9.0"));
  assert.ok(tagMatchesVersion("v0.9.0-rc.1", "0.9.0"));
  assert.ok(!tagMatchesVersion("v0.9.1", "0.9.0"));
  assert.ok(!tagMatchesVersion("0.9.0", "0.9.0"));
  assert.ok(!tagMatchesVersion("v0.9.0.1", "0.9.0"));
});

test("manifest has both platforms with tag URLs and trimmed signatures", () => {
  const m = buildManifest({
    version: "0.9.0",
    notes: "n",
    pubDate: "2026-10-10T00:00:00.000Z",
    baseUrl: "https://github.com/o/r/releases/download/v0.9.0",
    signatures: { "darwin-aarch64": "MAC\n", "windows-x86_64": "WIN" },
  });
  assert.deepEqual(m, {
    version: "0.9.0",
    notes: "n",
    pub_date: "2026-10-10T00:00:00.000Z",
    platforms: {
      "darwin-aarch64": {
        signature: "MAC",
        url: "https://github.com/o/r/releases/download/v0.9.0/Purl_aarch64.app.tar.gz",
      },
      "windows-x86_64": {
        signature: "WIN",
        url: "https://github.com/o/r/releases/download/v0.9.0/Purl_x64-setup.exe",
      },
    },
  });
});

test("manifest refuses a missing or empty signature", () => {
  const base = { version: "1", notes: "", pubDate: "", baseUrl: "u" };
  assert.throws(
    () => buildManifest({ ...base, signatures: { "darwin-aarch64": "x" } }),
    /windows-x86_64/,
  );
  assert.throws(
    () => buildManifest({ ...base, signatures: { "darwin-aarch64": " ", "windows-x86_64": "x" } }),
    /darwin-aarch64/,
  );
});

test("signatures are read from <updater file>.sig", () => {
  const dir = mkdtempSync(join(tmpdir(), "purl-sigs-"));
  writeFileSync(join(dir, "Purl_aarch64.app.tar.gz.sig"), "MAC");
  assert.throws(() => readSignatures(dir), /Purl_x64-setup\.exe\.sig/);
  writeFileSync(join(dir, "Purl_x64-setup.exe.sig"), "WIN");
  assert.deepEqual(readSignatures(dir), { "darwin-aarch64": "MAC", "windows-x86_64": "WIN" });
});

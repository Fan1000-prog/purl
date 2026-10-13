// node --test scripts/*.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./is-main.mjs";

const here = (name) => fileURLToPath(new URL(name, import.meta.url));

test("main only for this module's own path; never without a script path", () => {
  assert.equal(isMain(import.meta.url, null), false);
  assert.equal(isMain(import.meta.url, ""), false);
  assert.equal(isMain(import.meta.url, join(tmpdir(), "no-such-script.mjs")), false);
  assert.equal(isMain(import.meta.url, here("./is-main.mjs")), false);
  assert.equal(isMain(import.meta.url, here("./is-main.test.mjs")), true);
});

// Each script, run through a symlink with arguments that fail fast and
// offline, must run its main and fail, not exit 0 having done nothing.
const SCRIPTS = [
  { file: "release-assets.mjs", args: [], says: /usage: release-assets/ },
  { file: "make-latest-json.mjs", args: [], says: /make-latest-json: / },
  { file: "fetch-llama.mjs", args: [], env: { PURL_TARGET: "none" }, says: /no llama\.cpp build for none/ },
];

for (const s of SCRIPTS) {
  test(`${s.file} runs its main through a symlink`, () => {
    const dir = mkdtempSync(join(tmpdir(), "purl-is-main-"));
    try {
      const link = join(dir, s.file);
      symlinkSync(here(`./${s.file}`), link);
      const r = spawnSync(process.execPath, [link, ...s.args], {
        encoding: "utf8",
        env: { ...process.env, ...s.env },
      });
      assert.equal(r.status, 1, r.stdout + r.stderr);
      assert.match(r.stderr, s.says);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
}

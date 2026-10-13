// node --test scripts/*.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { tarCommand } from "./fetch-llama.mjs";

test("Windows extracts with the system bsdtar, by full path", () => {
  assert.equal(tarCommand("win32", { SystemRoot: "D:\\Win" }), "D:\\Win\\System32\\tar.exe");
  assert.equal(tarCommand("win32", {}), "C:\\Windows\\System32\\tar.exe");
});

test("macOS uses tar from PATH (bsdtar)", () => {
  assert.equal(tarCommand("darwin", {}), "tar");
});

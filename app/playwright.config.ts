import { defineConfig, devices } from "@playwright/test";

// UI smoke tests run against this checkout's own Vite dev server (no Tauri
// runtime); pages that call Tauri get the dev mock or tests/fixtures/
// tauri-stub.js. The server gets its own port and is never reused: another
// worktree's server on the usual 1420 would otherwise be tested silently.
// --strictPort makes a busy port fail the run loudly. Override with
// PURL_TEST_PORT.
const PORT = Number(process.env.PURL_TEST_PORT ?? 1430);

export default defineConfig({
  testDir: "tests",
  testMatch: "**/*.spec.ts",
  fullyParallel: true,
  reporter: "list",
  use: {
    baseURL: `http://localhost:${PORT}`,
    ...devices["Desktop Chrome"],
  },
  webServer: {
    command: `npx vite --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: false,
  },
});

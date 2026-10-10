import { defineConfig, devices } from "@playwright/test";

// UI smoke tests run against the Vite dev server (no Tauri runtime); pages
// that call Tauri get tests/fixtures/tauri-stub.js injected instead.
export default defineConfig({
  testDir: "tests",
  testMatch: "**/*.spec.ts",
  fullyParallel: true,
  reporter: "list",
  use: {
    baseURL: "http://localhost:1420",
    ...devices["Desktop Chrome"],
  },
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
  },
});

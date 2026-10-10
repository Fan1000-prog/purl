import { test, expect, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

// The overlay window is 200x82 (tauri.conf.json); the pill must fit inside it.
const WINDOW = { width: 200, height: 82 };
const STUB = fileURLToPath(new URL("./fixtures/tauri-stub.js", import.meta.url));
const CAPTURES = process.env.PURL_CAPTURES;

async function open(page: Page) {
  await page.setViewportSize(WINDOW);
  await page.addInitScript({ path: STUB });
  await page.goto("/overlay.html");
  await expect(page.locator("#mascot-slot img")).toBeVisible();
}

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([e, p]) => (window as any).__emit(e, p), [event, payload] as const);

async function expectInsideWindow(page: Page) {
  const r = (await page.locator("#pill").boundingBox())!;
  expect(r.x).toBeGreaterThanOrEqual(0);
  expect(r.y).toBeGreaterThanOrEqual(0);
  expect(r.x + r.width).toBeLessThanOrEqual(WINDOW.width);
  expect(r.y + r.height).toBeLessThanOrEqual(WINDOW.height);
}

async function capture(page: Page, name: string) {
  if (CAPTURES) await page.screenshot({ path: join(CAPTURES, `overlay-${name}.png`), omitBackground: true });
}

const STATES = [
  { name: "idle", drive: async (_: Page) => {}, mascot: "idle", label: "" },
  { name: "listening", drive: (p: Page) => emit(p, "state", "recording"), mascot: "listening", label: "Listening" },
  { name: "thinking", drive: (p: Page) => emit(p, "state", "processing"), mascot: "thinking", label: "Tidying up" },
  { name: "done", drive: (p: Page) => emit(p, "result", { cleaned: "hi" }), mascot: "done", label: "Sent" },
  { name: "error", drive: (p: Page) => emit(p, "state", "error"), mascot: "idle", label: "Didn't catch that" },
];

for (const s of STATES) {
  test(`${s.name}: mascot, label, fits the window`, async ({ page }) => {
    await open(page);
    await s.drive(page);
    await expect(page.locator("#mascot-slot img")).toHaveAttribute("src", new RegExp(`purl-${s.mascot}\\.svg$`));
    await expect(page.locator("#label")).toHaveText(s.label);
    await expectInsideWindow(page);
    await capture(page, s.name);
  });
}

test("error label uses the AA warn text colour", async ({ page }) => {
  await open(page);
  await emit(page, "state", "error");
  await expect(page.locator("#label")).toHaveCSS("color", "rgb(164, 83, 47)");
});

test("recording shows cancel + confirm and wires them to Rust", async ({ page }) => {
  await open(page);
  await emit(page, "state", "recording");
  await page.locator("#cancel").click();
  await page.locator("#confirm").click();
  const cmds = await page.evaluate(() => (window as any).__invoked.map((c: any) => c.cmd));
  expect(cmds).toContain("cancel_dictation");
  expect(cmds).toContain("toggle_recording");
});

test("clicking the idle pill starts dictation; processing reports no hit rect", async ({ page }) => {
  await open(page);
  await page.locator("#pill").click();
  await emit(page, "state", "processing");
  await page.waitForTimeout(50);
  const calls = await page.evaluate(() => (window as any).__invoked);
  expect(calls.some((c: any) => c.cmd === "toggle_recording")).toBe(true);
  const lastRect = calls.filter((c: any) => c.cmd === "set_overlay_hit_rect").at(-1);
  expect(lastRect.args.rect).toBeNull();
});

test("'Sent' clears back to idle, and recording overrides it at once", async ({ page }) => {
  await open(page);
  await emit(page, "result", {});
  await expect(page.locator("#label")).toHaveText("Sent");
  await emit(page, "state", "recording");
  await expect(page.locator("#label")).toHaveText("Listening");
  await emit(page, "state", "idle");
  await expect(page.locator("body")).toHaveAttribute("data-view", "idle");
});

test("transform caption with countdown fits the window", async ({ page }) => {
  await open(page);
  await emit(page, "transform-result", "Polish email");
  await expect(page.locator("#label")).toHaveText('"Polish email" applied');
  await expect(page.locator("#countdown")).toBeVisible();
  await expectInsideWindow(page);
  await capture(page, "caption");
});

test("reduced motion: static pose in the overlay", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  await emit(page, "state", "recording");
  await expect(page.locator("#mascot-slot img")).toHaveAttribute("src", /purl-pose-listening\.png$/);
});

test("the hit rect Rust gets is exactly the pill, idle and recording", async ({ page }) => {
  await open(page);
  for (const state of ["idle", "recording"]) {
    await emit(page, "state", state);
    await page.waitForTimeout(50);
    const box = (await page.locator("#pill").boundingBox())!;
    const rect = await page.evaluate(
      () => (window as any).__invoked.filter((c: any) => c.cmd === "set_overlay_hit_rect").at(-1).args.rect,
    );
    expect(rect, state).toEqual([box.x, box.y, box.width, box.height]);
  }
});

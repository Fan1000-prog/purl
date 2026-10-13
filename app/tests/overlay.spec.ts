import { test, expect, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

// The overlay window is 420x100 (tauri.conf.json); the pill must fit inside it.
const WINDOW = { width: 420, height: 100 };
const STUB = fileURLToPath(new URL("./fixtures/tauri-stub.js", import.meta.url));
const CAPTURES = process.env.PURL_CAPTURES;

// Playwright's "Desktop Chrome" reports a Windows user agent; pin macOS
// unless a test asks for Windows.
async function open(page: Page, os: "macos" | "windows" = "macos") {
  await page.setViewportSize(WINDOW);
  await page.addInitScript({ path: STUB });
  await page.addInitScript((p) => ((window as any).__PURL_PLATFORM__ = p), os);
  await page.goto("/overlay.html");
  await expect(page.locator("#capsule.purl-pill")).toHaveAttribute("data-state", "idle");
}

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([e, p]) => (window as any).__emit(e, p), [event, payload] as const);

/** The text of the pill's visible layer, one space between pieces
 *  (the label and each keycap). */
const label = (page: Page) =>
  page.locator("#capsule").evaluate((el) => {
    const layer = el.querySelector<HTMLElement>(`[data-layer="${el.getAttribute("data-state")}"]`)!;
    const walk = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT);
    const parts: string[] = [];
    while (walk.nextNode()) parts.push(walk.currentNode.textContent!.trim());
    return parts.filter(Boolean).join(" ");
  });

const lastHitRect = (page: Page) =>
  page.evaluate(() => (window as any).__invoked.filter((c: any) => c.cmd === "set_overlay_hit_rect").at(-1).args.rect);

async function expectInsideWindow(page: Page) {
  const r = (await page.locator("#pill").boundingBox())!;
  expect(r.x).toBeGreaterThanOrEqual(0);
  expect(r.y).toBeGreaterThanOrEqual(0);
  expect(r.x + r.width).toBeLessThanOrEqual(WINDOW.width);
  expect(r.y + r.height).toBeLessThanOrEqual(WINDOW.height);
}

/** Lets the capsule's width morph (--dur-morph, 800 ms) settle. */
const settle = (page: Page) => page.waitForTimeout(900);

async function capture(page: Page, name: string) {
  if (CAPTURES) await page.screenshot({ path: join(CAPTURES, `overlay-${name}.png`), omitBackground: true });
}

const STATES = [
  { name: "idle", drive: async (_: Page) => {}, pill: "idle", label: "Hold ⌘ ⇧ Space" },
  { name: "listening", drive: (p: Page) => emit(p, "state", "recording"), pill: "listening", label: "00:00" },
  { name: "polishing", drive: (p: Page) => emit(p, "state", "processing"), pill: "polishing", label: "Polishing" },
  { name: "sent", drive: (p: Page) => emit(p, "result", { cleaned: "hi" }), pill: "sent", label: "Sent clean" },
  { name: "error", drive: (p: Page) => emit(p, "state", "error"), pill: "error", label: "Didn't catch that" },
];

for (const s of STATES) {
  test(`${s.name}: pill state, label, fits the window`, async ({ page }) => {
    await open(page);
    await s.drive(page);
    await expect(page.locator("#capsule")).toHaveAttribute("data-state", s.pill);
    await expect.poll(() => label(page)).toBe(s.label);
    await settle(page);
    await expectInsideWindow(page);
    await capture(page, s.name);
  });
}

test("idle shows the real hotkey from settings, and follows a change", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__results = { get_status: { hotkey: "Alt+Space", state: "idle" } };
  });
  await open(page);
  await expect.poll(() => label(page)).toBe("Hold ⌥ Space");
  await emit(page, "hotkey-changed", "CmdOrCtrl+Shift+D");
  await expect.poll(() => label(page)).toBe("Hold ⌘ ⇧ D");
});

test("Windows spells the hotkey out", async ({ page }) => {
  await open(page, "windows");
  await expect.poll(() => label(page)).toBe("Hold Ctrl Shift Space");
});

test("mic level drives the waveform while recording", async ({ page }) => {
  await open(page);
  await emit(page, "state", "recording");
  const tallest = () =>
    page.locator("#capsule .purl-pill__bars i").evaluateAll((bars) => Math.max(...bars.map((b) => b.getBoundingClientRect().height)));
  const quiet = await tallest();
  for (let i = 0; i < 10; i++) {
    await emit(page, "audio-level", 0.2);
    await page.waitForTimeout(30);
  }
  expect(await tallest()).toBeGreaterThan(quiet + 10);
});

test("'Warming up…' while the speech model loads", async ({ page }) => {
  await open(page);
  await emit(page, "engine-status", { stt: { state: "loading" }, llm: { state: "idle" } });
  await emit(page, "state", "processing");
  await expect(page.locator("#capsule")).toHaveAttribute("data-state", "polishing");
  await expect.poll(() => label(page)).toBe("Warming up…");
  await emit(page, "engine-status", { stt: { state: "ready" }, llm: { state: "idle" } });
  await expect.poll(() => label(page)).toBe("Polishing");
});

test("error label uses the AA warn text colour", async ({ page }) => {
  await open(page);
  await emit(page, "state", "error");
  // Ember (--warn-text, dark theme) on the alert icon; the message is --text.
  await expect(page.locator('#capsule [data-layer="error"]')).toHaveCSS("color", "rgb(236, 138, 90)");
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
  await page.locator("#capsule").click();
  await emit(page, "state", "processing");
  await page.waitForTimeout(50);
  const calls = await page.evaluate(() => (window as any).__invoked);
  expect(calls.some((c: any) => c.cmd === "toggle_recording")).toBe(true);
  expect(await lastHitRect(page)).toBeNull();
});

test("'Sent clean' clears back to idle, and recording overrides it at once", async ({ page }) => {
  await open(page);
  await emit(page, "result", {});
  await expect(page.locator("#capsule")).toHaveAttribute("data-state", "sent");
  await emit(page, "state", "recording");
  await expect(page.locator("#capsule")).toHaveAttribute("data-state", "listening");
  await emit(page, "state", "idle");
  await expect(page.locator("body")).toHaveAttribute("data-view", "idle");
});

test("transform caption with countdown fits the window", async ({ page }) => {
  await open(page);
  await emit(page, "transform-result", "Polish email");
  await expect.poll(() => label(page)).toBe('"Polish email" applied');
  await expect(page.locator("#countdown")).toBeVisible();
  await settle(page);
  await expectInsideWindow(page);
  await capture(page, "caption");
});

test("a long caption is cut short, not pushed out of the window", async ({ page }) => {
  await open(page);
  await emit(page, "correction-learned", "Supercalifragilisticexpialidocious and then some more words");
  await settle(page);
  await expectInsideWindow(page);
});

test("reduced motion: the pill is still", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  await emit(page, "state", "processing");
  await expect(page.locator("#capsule")).toHaveAttribute("data-state", "polishing");
  const running = await page
    .locator("#capsule")
    .evaluate((el) => [el, ...el.querySelectorAll("*")].filter((n) => getComputedStyle(n).animationName !== "none").length);
  expect(running).toBe(0);
});

test("the hit rect Rust gets is exactly the control, idle and recording", async ({ page }) => {
  await open(page);
  for (const state of ["idle", "recording"]) {
    await emit(page, "state", state);
    await settle(page);
    const box = (await page.locator("#pill").boundingBox())!;
    expect(await lastHitRect(page), state).toEqual([box.x, box.y, box.width, box.height]);
  }
});

test("the hit rect is re-sent when the pill changes size on its own", async ({ page }) => {
  await open(page);
  await settle(page);
  const before = (await lastHitRect(page)) as number[];
  // A size change with no state event, like a font landing late.
  await page.locator("#capsule").evaluate((el) => {
    const shell = el.querySelector<HTMLElement>(".purl-pill__shell")!;
    shell.style.setProperty("--pill-w", `${shell.offsetWidth + 60}px`);
  });
  await settle(page);
  const after = (await lastHitRect(page)) as number[];
  const box = (await page.locator("#pill").boundingBox())!;
  expect(after[2]).toBeGreaterThan(before[2]);
  expect(after).toEqual([box.x, box.y, box.width, box.height]);
});

test("a render that changes nothing doesn't re-announce the pill", async ({ page }) => {
  await open(page);
  await settle(page);
  // Count every write to the pill's screen-reader live region from here on.
  await page.locator("#capsule .purl-sr-only").evaluate((el) => {
    (window as any).__announced = 0;
    new MutationObserver((m) => ((window as any).__announced += m.length)).observe(el, {
      childList: true,
      characterData: true,
      subtree: true,
    });
  });
  const announced = () => page.evaluate(() => (window as any).__announced as number);
  // Engine updates (speech model ready, then unloaded) and a repeat idle.
  await emit(page, "engine-status", { stt: { state: "ready" }, llm: { state: "idle" } });
  await emit(page, "engine-status", { stt: { state: "idle" }, llm: { state: "idle" } });
  await emit(page, "state", "idle");
  await page.waitForTimeout(50);
  expect(await announced()).toBe(0);
  await emit(page, "state", "recording");
  await expect.poll(announced).toBeGreaterThan(0);
});

test("the hit rect is sent only when it changes", async ({ page }) => {
  await open(page);
  await settle(page);
  const sent = () =>
    page.evaluate(() => (window as any).__invoked.filter((c: any) => c.cmd === "set_overlay_hit_rect").length);
  const before = await sent();
  await emit(page, "engine-status", { stt: { state: "ready" }, llm: { state: "idle" } });
  await emit(page, "state", "idle");
  await page.waitForTimeout(100);
  expect(await sent()).toBe(before);
  await emit(page, "state", "processing");
  await expect.poll(() => lastHitRect(page)).toBeNull();
  expect(await sent()).toBe(before + 1);
});

test("a hit rect Rust didn't take is sent again", async ({ page }) => {
  await page.addInitScript(() => ((window as any).__rejects = { set_overlay_hit_rect: true }));
  await open(page);
  await settle(page);
  const sent = () =>
    page.evaluate(() => (window as any).__invoked.filter((c: any) => c.cmd === "set_overlay_hit_rect").length);
  const before = await sent();
  await page.evaluate(() => ((window as any).__rejects = {}));
  await emit(page, "state", "idle");
  await expect.poll(sent).toBe(before + 1);
});

test("the pill's tooltip says hold to talk", async ({ page }) => {
  await open(page);
  await expect(page.locator("#pill")).toHaveAttribute("title", "Hold your dictation hotkey and talk");
});

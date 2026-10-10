import { test, expect, type Page } from "@playwright/test";

// Drives the onboarding against the dev-only Tauri mock (src/dev/mock-tauri.ts).
const open = (page: Page, params = "") => page.goto(`/?mock=1&step_ms=40${params}`);
const panel = (page: Page) => page.locator("#onboarding");
const primary = (page: Page) => panel(page).locator(".onb-actions .btn-primary");

async function toModelStep(page: Page) {
  await primary(page).click(); // Set up Purl
  await expect(panel(page)).toHaveAttribute("data-step", "permissions");
  await primary(page).click(); // Continue
  await expect(panel(page)).toHaveAttribute("data-step", "model");
}

test("happy path: welcome → permissions → download → try it → finish", async ({ page }) => {
  await open(page);
  await expect(panel(page).locator(".onb-title")).toHaveText("Speak softly. Send it clean.");
  await expect(panel(page).locator(".mascot img")).toHaveAttribute("src", /purl-idle\.svg$/);
  await expect(primary(page)).toHaveText("Set up Purl");
  await primary(page).click();

  const rows = panel(page).locator(".perm-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toHaveAttribute("data-trust", "granted");
  await expect(rows.nth(1)).toContainText("Accessibility");
  // Finishing marks field reading as disclosed, so setup must say it plainly.
  await expect(rows.nth(1)).toContainText("read that text field for 30 seconds");
  await primary(page).click();

  const card = panel(page).locator(".model-card");
  await expect(card).toContainText("Parakeet v3");
  await expect(card).toContainText("671 MB · runs on your computer");
  await expect(primary(page)).toBeDisabled();
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card.getByRole("progressbar")).toBeVisible();
  await expect(card.locator(".model-count")).toContainText("of 671 MB");
  await expect(card).toHaveAttribute("data-state", "ready");
  await expect(primary(page)).toBeEnabled();
  await primary(page).click();

  await expect(panel(page)).toHaveAttribute("data-step", "try");
  await expect(panel(page)).toContainText("Press ⌘⇧Space and say something");
  await page.evaluate(() => window.__purlMock!.dictate("Testing Purl, one two three."));
  await expect(panel(page).locator(".onb-try")).toHaveValue("Testing Purl, one two three.");
  await primary(page).click(); // Finish

  await expect(panel(page)).toBeHidden();
  const saved = await page.evaluate(() =>
    window.__purlMock!.calls.filter((c) => c.cmd === "save_settings").map((c: any) => c.args.newSettings.onboarding_complete),
  );
  expect(saved).toContain(true);
  await expect(page.locator("#smart-offer")).toContainText("1.4 GB · optional");
});

test("download failure shows the reason, then Retry succeeds", async ({ page }) => {
  await open(page, "&fail=offline");
  await toModelStep(page);
  const card = panel(page).locator(".model-card");
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card.getByRole("alert")).toHaveText("You're offline. Connect to the internet and press Retry.");
  await card.getByRole("button", { name: "Retry" }).click();
  await expect(card).toHaveAttribute("data-state", "ready");
  await expect(card.getByRole("alert")).toBeHidden();
  await expect(primary(page)).toBeEnabled();
});

test("disk full disables Retry until the window regains focus", async ({ page }) => {
  await open(page, "&fail=diskfull");
  await toModelStep(page);
  const card = panel(page).locator(".model-card");
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card.getByRole("alert")).toContainText("Not enough free space");
  const retry = card.getByRole("button", { name: "Retry" });
  await expect(retry).toBeDisabled();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(retry).toBeEnabled();
});

test("cancel mid-download returns to Resume, not an error", async ({ page }) => {
  await open(page, "&step_ms=200");
  await toModelStep(page);
  const card = panel(page).locator(".model-card");
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card.locator(".model-count")).not.toHaveText(/^0 MB/);
  await card.getByRole("button", { name: "Cancel" }).click();
  await expect(card).toHaveAttribute("data-state", "missing");
  await expect(card.getByRole("button", { name: "Resume" })).toBeVisible();
  await expect(card.getByRole("alert")).toBeHidden();
  await expect(primary(page)).toBeDisabled();
});

test("denied microphone says so and links to Settings", async ({ page }) => {
  await open(page, "&mic=denied&ax=denied");
  await primary(page).click();
  const rows = panel(page).locator(".perm-row");
  await expect(rows.nth(0)).toHaveAttribute("data-trust", "denied");
  await expect(rows.nth(0).getByRole("button", { name: "Open Settings" })).toBeVisible();
  await expect(panel(page).locator(".onb-note")).toContainText("can't hear you");
  await page.evaluate(() => window.__purlMock!.setTrust({ mic: "granted" }));
  await expect(rows.nth(0)).toHaveAttribute("data-trust", "granted");
});

test("Windows: no Accessibility row, Ctrl hotkey", async ({ page }) => {
  await open(page, "&platform=windows&models=ready");
  await primary(page).click();
  await expect(panel(page).locator(".perm-row")).toHaveCount(1);
  await primary(page).click();
  await expect(panel(page).locator(".model-card")).toHaveAttribute("data-state", "ready");
  await primary(page).click();
  await expect(panel(page)).toContainText("Press Ctrl+Shift+Space and say something");
});

test("reduced motion: welcome mascot is the static pose", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  await expect(panel(page).locator(".mascot img")).toHaveAttribute("src", /purl-pose-idle\.png$/);
});

test("360×640: no horizontal scroll on any step", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 640 });
  await open(page, "&mic=denied&ax=denied");
  const noOverflow = () =>
    page.evaluate(() => {
      const el = document.getElementById("onboarding")!;
      return el.scrollWidth <= el.clientWidth && document.documentElement.scrollWidth <= 360;
    });
  for (const step of ["welcome", "permissions", "model", "try"]) {
    await expect(panel(page)).toHaveAttribute("data-step", step);
    expect(await noOverflow(), step).toBe(true);
    if (step === "model") {
      await panel(page).getByRole("button", { name: "Download" }).click();
      await expect(panel(page).locator(".model-card")).toHaveAttribute("data-state", "ready");
    }
    if (step !== "try") await primary(page).click();
  }
});

test("Set up later closes setup and offers Smart cleanup; dismiss sticks", async ({ page }) => {
  await open(page);
  await primary(page).click();
  await panel(page).getByRole("button", { name: "Set up later" }).click();
  await expect(panel(page)).toBeHidden();
  const offer = page.locator("#smart-offer");
  await expect(offer).toBeVisible();
  await offer.getByRole("button", { name: "Dismiss" }).click();
  await expect(offer).toHaveCount(0);
  // The mock forgets settings on reload; localStorage keeps the dismissal.
  await open(page, "&onboarded=1");
  await expect(panel(page)).toBeHidden();
  await expect(page.locator("#history-empty")).toBeVisible();
  await expect(page.locator("#smart-offer")).toHaveCount(0);
});

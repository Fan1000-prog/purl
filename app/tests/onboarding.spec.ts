import { test, expect, type Page } from "@playwright/test";

// Drives the onboarding against the dev-only Tauri mock (src/dev/mock-tauri.ts).
const open = (page: Page, params = "") => page.goto(`/?mock=1&step_ms=40${params}`);
const panel = (page: Page) => page.locator("#onboarding");
const primary = (page: Page) => panel(page).locator(".onb-actions .btn-primary");

const lastSaved = (page: Page) =>
  page.evaluate(() => {
    const saves = window.__purlMock!.calls.filter((c) => c.cmd === "save_settings");
    return (saves.at(-1)?.args as any)?.newSettings;
  });

async function toModelStep(page: Page) {
  await primary(page).click(); // Set up Purl
  await expect(panel(page)).toHaveAttribute("data-step", "permissions");
  await primary(page).click(); // Continue
  await expect(panel(page)).toHaveAttribute("data-step", "model");
}

test("happy path: welcome → permissions → download → try it → finish", async ({ page }) => {
  await open(page);
  await expect(panel(page).locator(".onb-title")).toHaveText("Speak softly. Send it clean.");
  await expect(panel(page).locator(".onb-art .purl-orb")).toHaveAttribute("data-state", "idle");
  await expect(primary(page)).toHaveText("Set up Purl");
  await primary(page).click();

  const rows = panel(page).locator(".perm-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toHaveAttribute("data-trust", "granted");
  await expect(rows.nth(1)).toContainText("Accessibility");
  // Finishing marks field reading as disclosed, so setup must say it plainly,
  // with the real window (lib.rs WATCH_MAX_POLLS × WATCH_POLL).
  await expect(rows.nth(1)).toContainText("read that text field for up to 90 seconds after pasting");
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
  await expect(panel(page)).toContainText("Hold ⌘⇧Space and say something. Let go to paste.");
  await page.evaluate(() => window.__purlMock!.dictate("Testing Purl, one two three."));
  await expect(panel(page).locator(".onb-try")).toHaveValue("Testing Purl, one two three.");
  await primary(page).click(); // Finish

  await expect(panel(page)).toBeHidden();
  const saved = await lastSaved(page);
  expect(saved).toMatchObject({ onboarding_complete: true, field_reading_disclosed: true });
  await expect(page.locator("#smart-offer")).toContainText("1.4 GB · optional");
});

test("download failure shows the reason, then Retry succeeds", async ({ page }) => {
  await open(page, "&fail=offline");
  await toModelStep(page);
  const card = panel(page).locator(".model-card");
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card.locator(".model-error")).toHaveText("You're offline. Connect to the internet and press Retry.");
  await card.getByRole("button", { name: "Retry" }).click();
  await expect(card).toHaveAttribute("data-state", "ready");
  await expect(card.locator(".model-error")).toBeHidden();
  await expect(primary(page)).toBeEnabled();
});

test("disk full disables Retry until the window regains focus", async ({ page }) => {
  await open(page, "&fail=diskfull");
  await toModelStep(page);
  const card = panel(page).locator(".model-card");
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card.locator(".model-error")).toContainText("Not enough free space");
  const retry = card.getByRole("button", { name: "Retry" });
  await expect(retry).toBeDisabled();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(retry).toBeEnabled();
});

test("cancel mid-download returns to Resume with sizes, and stays Resume after Back", async ({ page }) => {
  await open(page, "&step_ms=200");
  await toModelStep(page);
  const card = () => panel(page).locator(".model-card");
  await card().getByRole("button", { name: "Download" }).click();
  await expect(card().locator(".model-count")).not.toHaveText(/^0 MB/);
  await card().getByRole("button", { name: "Cancel" }).click();
  await expect(card()).toHaveAttribute("data-state", "missing");
  await expect(card().getByRole("button", { name: /^Resume \(\d+ MB of 671 MB\)$/ })).toBeVisible();
  await expect(card().locator(".model-error")).toBeHidden();
  await expect(card().locator(".model-live")).toHaveText("Download cancelled. You can resume it.");
  await expect(primary(page)).toBeDisabled();

  await panel(page).getByRole("button", { name: "Back" }).click();
  await primary(page).click();
  await expect(card().getByRole("button", { name: /^Resume \(\d+ MB of 671 MB\)$/ })).toBeVisible();
});

test("keyboard: focus follows Download → Cancel → Resume, and Ready is announced", async ({ page }) => {
  await open(page, "&step_ms=200");
  await toModelStep(page);
  const card = panel(page).locator(".model-card");
  await card.getByRole("button", { name: "Download" }).focus();
  await page.keyboard.press("Enter");
  await expect(card.getByRole("button", { name: "Cancel" })).toBeFocused();
  await expect(card.locator(".model-live")).toHaveText("Downloading Parakeet v3.");
  await expect(card.locator(".model-count")).not.toHaveText(/^0 MB/);
  await page.keyboard.press("Enter");
  await expect(card.getByRole("button", { name: /^Resume/ })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(card.getByRole("button", { name: "Cancel" })).toBeFocused();
  await expect(card).toHaveAttribute("data-state", "ready", { timeout: 10_000 });
  await expect(card.locator(".model-live")).toHaveText("Parakeet v3 is ready.");
  await expect(primary(page)).toBeFocused();
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

test("Windows: no Accessibility row or field-reading text, Ctrl hotkey, nothing marked disclosed", async ({ page }) => {
  await open(page, "&platform=windows&models=ready");
  await primary(page).click();
  await expect(panel(page).locator(".perm-row")).toHaveCount(1);
  await expect(panel(page)).not.toContainText("90 seconds");
  await primary(page).click();
  await expect(panel(page).locator(".model-card")).toHaveAttribute("data-state", "ready");
  await primary(page).click();
  await expect(panel(page)).toContainText("Hold Ctrl+Shift+Space and say something. Let go to paste.");
  await primary(page).click(); // Finish
  await expect(panel(page)).toBeHidden();
  expect(await lastSaved(page)).toMatchObject({ onboarding_complete: true, field_reading_disclosed: false });
});

test("reduced motion: the welcome orb is still", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  const orb = panel(page).locator(".onb-art .purl-orb");
  await expect(orb).toHaveAttribute("data-state", "idle");
  const running = await orb.evaluate((el) =>
    [el, ...el.querySelectorAll("*")].filter((n) => getComputedStyle(n).animationName !== "none").length,
  );
  expect(running).toBe(0);
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

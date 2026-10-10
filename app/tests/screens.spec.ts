import { test, expect, type Page } from "@playwright/test";

// The redesigned screens against the dev-only Tauri mock (src/dev/mock-tauri.ts).
const SCREENS = [
  { view: "home", title: "Ready when you are." },
  { view: "dictionary", title: "Dictionary" },
  { view: "snippets", title: "Snippets" },
  { view: "transforms", title: "Transforms" },
  { view: "settings", title: "Settings" },
] as const;

async function open(page: Page, params = "") {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`/?mock=1&onboarded=1&step_ms=30${params}`);
  await expect(page.locator("#view-home")).toBeVisible();
  return errors;
}

async function go(page: Page, view: string) {
  await page.locator(`.nav-btn[data-view="${view}"]`).click();
  await expect(page.locator(`#view-${view}`)).toBeVisible();
}

const calls = (page: Page, cmd: string) =>
  page.evaluate((c) => window.__purlMock!.calls.filter((x) => x.cmd === c).map((x) => x.args as any), cmd);

for (const scheme of ["light", "dark"] as const) {
  test(`every screen renders in ${scheme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    const errors = await open(page, "&data=demo&models=ready");
    for (const s of SCREENS) {
      await go(page, s.view);
      await expect(page.locator(`#view-${s.view} .page-title`)).toHaveText(s.title);
      await expect(page.locator(`.nav-btn[data-view="${s.view}"]`)).toHaveAttribute("aria-current", "page");
    }
    const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(bg).toBe(scheme === "light" ? "rgb(247, 241, 232)" : "rgb(42, 35, 30)");
    expect(errors).toEqual([]);
  });
}

test("empty states on every list", async ({ page }) => {
  await open(page, "&models=ready");
  await expect(page.locator("#last-empty")).toBeVisible();
  await expect(page.locator("#history-empty")).toBeVisible();
  await expect(page.locator("#clear-history-btn")).toBeHidden();
  await go(page, "dictionary");
  await expect(page.locator("#dict-empty")).toBeVisible();
  await expect(page.locator("#learned-empty")).toBeVisible();
  await go(page, "snippets");
  await expect(page.locator("#snippet-empty")).toBeVisible();
  await go(page, "transforms");
  await expect(page.locator("#transforms-empty")).toBeVisible();
});

test("error states on every list, and no form that could overwrite the file", async ({ page }) => {
  await open(page, "&models=ready&fail_cmd=get_history,get_stats,get_glossary,get_learned_rules,get_snippets,get_transforms");
  await expect(page.locator("#history-error")).toContainText("Couldn't load your history");
  await expect(page.locator("#home-stats")).toContainText("Couldn't load your stats");
  await go(page, "dictionary");
  await expect(page.locator("#dict-error")).toContainText("Couldn't load your dictionary");
  await expect(page.locator("#dict-form")).toBeHidden();
  await expect(page.locator("#learned-msg")).toContainText("Couldn't load learned corrections");
  await go(page, "snippets");
  await expect(page.locator("#snippet-error")).toContainText("Couldn't load your snippets");
  await expect(page.locator("#snippet-form")).toBeHidden();
  await go(page, "transforms");
  await expect(page.locator("#transform-list .error-state")).toContainText("Couldn't load your transforms");
  await expect(page.locator("#transforms-reset")).toBeHidden();
});

test("Home: last dictation toggles cleaned and raw, and updates on a new result", async ({ page }) => {
  await open(page, "&data=demo&models=ready");
  await expect(page.locator("#last-text")).toHaveText("Hey Sam, thanks for the notes. I'll send the deck by Friday.");
  await page.getByRole("radio", { name: "Raw" }).click();
  await expect(page.locator("#last-text")).toHaveText("hey sam thanks for the notes ill send the deck by friday");
  await page.getByRole("radio", { name: "Cleaned" }).click();
  await page.evaluate(() => window.__purlMock!.dictate("Testing Purl, one two three."));
  await expect(page.locator("#home-title")).toHaveText("Sent.");
  await expect(page.locator("#home-state")).toHaveAttribute("data-state", "done");
  await expect(page.locator("#last-text")).toHaveText("Testing Purl, one two three.");
});

test("Home banners: Accessibility denied, speech model missing, speech failed", async ({ page }) => {
  await open(page, "&models=ready&ax=denied");
  await expect(page.locator("#home-banner")).toContainText("Paste needs Accessibility");
  await expect(page.locator("#home-banner").getByRole("button", { name: "Open Accessibility settings" })).toBeVisible();

  await open(page, "");
  await expect(page.locator("#home-banner")).toContainText("Download the speech model");
  await expect(page.locator("#home-title")).toHaveText("One more step.");
  await expect(page.locator("#home-state")).toHaveAttribute("data-state", "blocked");
  await expect(page.locator("#toggle-btn")).toBeDisabled();
  await page.locator("#home-banner").getByRole("button", { name: "Download" }).click();
  await expect(page.locator("#home-banner")).toBeHidden();
  await expect(page.locator("#toggle-btn")).toBeEnabled();

  await open(page, "&models=ready&stt=failed");
  await expect(page.locator("#home-banner")).toContainText("couldn't load");
  await expect(page.locator("#home-title")).toHaveText("Speech isn't working.");
});

test("Dictionary: add and remove keep the rest of the file, comments included", async ({ page }) => {
  await open(page, "&data=demo&models=ready");
  await go(page, "dictionary");
  await expect(page.locator("#dict-list li")).toHaveCount(3);
  await page.locator("#dict-term").fill("Fani");
  await page.locator("#dict-form button[type=submit]").click();
  await expect(page.locator("#dict-list li")).toHaveCount(4);
  await page.locator("#dict-term").fill("yesware");
  await page.locator("#dict-form button[type=submit]").click();
  await expect(page.locator("#dict-msg")).toHaveText('"yesware" is already in your dictionary.');
  await page.getByRole("button", { name: "Remove Yesware" }).click();
  await expect(page.locator("#dict-list li")).toHaveCount(3);
  const saved = await calls(page, "save_glossary");
  expect(saved.at(-1).content).toBe("# My words\npr = PR\nGoogle Chat\nFani\n");
});

test("Settings: choosing Smart without a model downloads it, then turns Smart on", async ({ page }) => {
  await open(page, "&data=demo");
  await go(page, "settings");
  await page.locator('#cleanup-mode button[data-value="smart"]').click();
  await expect(page.locator("#cleanup-help")).toContainText("Smart turns on when the download finishes");
  const card = page.locator("#smart-download .model-card");
  await expect(card).toContainText("Smart cleanup");
  await expect(card).toContainText("1.4 GB");
  expect((await calls(page, "save_settings")).some((a) => a.newSettings.cleanup_mode === "smart")).toBe(false);

  await page.locator('#smart-model button[data-value="qwen3.5-4b"]').click();
  await expect(card).toContainText("3.0 GB");
  await card.getByRole("button", { name: "Download" }).click();
  await expect(card).toHaveAttribute("data-state", "ready");
  await expect(page.locator("#cleanup-help")).toContainText("language model on this computer");
  const last = (await calls(page, "save_settings")).at(-1).newSettings;
  expect(last).toMatchObject({ cleanup_mode: "smart", smart_model: "qwen3.5-4b" });
});

test("Settings: toggles save on change; models can be deleted after a confirm", async ({ page }) => {
  await open(page, "&models=ready");
  await go(page, "settings");
  await page.locator("#command-detection").click();
  await expect.poll(async () => (await calls(page, "save_settings")).at(-1)?.newSettings.command_detection).toBe(false);
  const del = page.getByRole("button", { name: "Delete Parakeet v3 (speech)" });
  await del.click();
  await expect(del).toHaveText("Delete? Re-download any time.");
  await del.click();
  await expect(page.locator("#models-list")).toContainText("Parakeet v3 (speech)671 MB · not downloaded");
});

test("Windows mode: no learning, Ctrl hotkeys", async ({ page }) => {
  await open(page, "&platform=windows&models=ready&data=demo");
  await expect(page.locator("#hotkey-label")).toHaveText("Ctrl+Shift+Space");
  await go(page, "settings");
  await expect(page.locator("#learn-corrections")).toBeDisabled();
  await expect(page.locator("#learn-help")).toHaveText("Coming to Windows soon.");
  await go(page, "dictionary");
  await expect(page.locator("#learned-section")).toBeHidden();
  await go(page, "transforms");
  await expect(page.locator(".transform-chip").first()).toHaveText("Alt+1");
});

test("Transforms say they need Smart cleanup's model when none is downloaded", async ({ page }) => {
  await open(page, "&data=demo");
  await go(page, "transforms");
  await expect(page.locator("#transforms-needs-smart")).toContainText("Transforms use Smart cleanup's model");
});

test("settings that fail to load lock every control", async ({ page }) => {
  await open(page, "&models=ready&fail_cmd=get_settings");
  await go(page, "settings");
  await expect(page.locator("#settings-msg")).toContainText("Couldn't load your settings");
  await expect(page.locator("#learn-corrections")).toBeDisabled();
  await expect(page.locator('#cleanup-mode button[data-value="smart"]')).toBeDisabled();
});

test("360 px wide: no horizontal scroll on any screen", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 700 });
  await open(page, "&data=demo&models=ready");
  for (const s of SCREENS) {
    await go(page, s.view);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, s.view).toBeLessThanOrEqual(0);
  }
});

test("the what's-new card owes the 90-second disclosure on macOS only", async ({ page }) => {
  await open(page, "&models=ready&disclosed=0");
  const card = page.locator("#field-notice");
  await expect(card).toBeVisible();
  await expect(card).toContainText("up to 90 seconds after pasting");
  await card.getByRole("button", { name: "Got it" }).click();
  await expect(card).toBeHidden();
  expect((await calls(page, "save_settings")).at(-1).newSettings.field_reading_disclosed).toBe(true);

  await open(page, "&models=ready&disclosed=0&platform=windows");
  await expect(page.locator("#field-notice")).toBeHidden();
  // A new version on Windows: still no field-reading paragraph, so no card.
  await open(page, "&models=ready&platform=windows&version=1.0.0");
  await expect(page.locator("#field-notice")).toBeHidden();
});

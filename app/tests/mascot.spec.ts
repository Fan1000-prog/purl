import { test, expect, type Page } from "@playwright/test";

const bigImg = (page: Page) => page.locator("#big img");

test("renders the animated idle SVG", async ({ page }) => {
  await page.goto("/tests/fixtures/mascot.html");
  await expect(bigImg(page)).toHaveAttribute("src", /\/mascot\/purl-idle\.svg$/);
  await expect.poll(() => bigImg(page).evaluate((i: HTMLImageElement) => i.naturalWidth)).toBeGreaterThan(0);
});

test("set() swaps src without moving the box", async ({ page }) => {
  await page.goto("/tests/fixtures/mascot.html");
  const box = page.locator("#big .mascot");
  const before = await box.boundingBox();
  await page.evaluate(() => (window as any).big.set("listening"));
  await expect(bigImg(page)).toHaveAttribute("src", /\/mascot\/purl-listening\.svg$/);
  expect(await box.boundingBox()).toEqual(before);
});

test("reduced motion shows the static pose for the current state", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/tests/fixtures/mascot.html");
  await expect(bigImg(page)).toHaveAttribute("src", /purl-pose-idle\.png$/);
  await page.evaluate(() => (window as any).big.set("thinking"));
  await expect(bigImg(page)).toHaveAttribute("src", /purl-pose-thinking\.png$/);
});

test("follows a live change to reduced motion", async ({ page }) => {
  await page.goto("/tests/fixtures/mascot.html");
  await expect(bigImg(page)).toHaveAttribute("src", /purl-idle\.svg$/);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(bigImg(page)).toHaveAttribute("src", /purl-pose-idle\.png$/);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(bigImg(page)).toHaveAttribute("src", /purl-idle\.svg$/);
});

test("below 40 px it is always the static idle pose", async ({ page }) => {
  await page.goto("/tests/fixtures/mascot.html");
  const img = page.locator("#small img");
  await expect(img).toHaveAttribute("src", /purl-pose-idle\.png$/);
  await page.evaluate(() => (window as any).small.set("done"));
  await expect(img).toHaveAttribute("src", /purl-pose-idle\.png$/);
  expect(await page.locator("#small .mascot").boundingBox()).toMatchObject({ width: 32, height: 32 });
});

test("destroy() removes the mascot", async ({ page }) => {
  await page.goto("/tests/fixtures/mascot.html");
  await page.evaluate(() => (window as any).big.destroy());
  await expect(page.locator("#big .mascot")).toHaveCount(0);
});

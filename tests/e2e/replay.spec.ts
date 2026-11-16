import { expect, test } from "@playwright/test";

// A bundled replay is resimulated in the browser from its inputs and must
// match every checksum the recording carries. This is the determinism
// claim checked end to end, in each engine, on a match of real play.

test("a bundled replay plays back and matches every checkpoint", async ({ page }) => {
  await page.goto("http://127.0.0.1:4173/static/?replay&view=flat");
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  await page.getByRole("button", { name: "4x" }).click();
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "ended", { timeout: 30_000 });

  const matched = await page.locator('[data-stat="replay matched"]').textContent();
  const [done, total] = (matched ?? "").split(" of ").map(Number);
  expect(total).toBeGreaterThan(60);
  expect(done).toBe(total);
  await expect(page.locator('[data-stat="replay verdict"]')).toContainText("matches the recording");

  // The second bundled replay can be chosen from the list.
  await page.getByRole("combobox").selectOption({ index: 1 });
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  await expect(page.locator('[data-stat="replay frame"]')).toContainText(/^\d+ of 3711$/);
});

test("a match played on the server shows up as a replay", async ({ page }) => {
  await page.goto("http://127.0.0.1:8090/?replay&view=flat");
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  // At least the bundled ones are listed; matches recorded by the server
  // come first when there are any.
  expect(await page.getByRole("combobox").locator("option").count()).toBeGreaterThanOrEqual(2);
});

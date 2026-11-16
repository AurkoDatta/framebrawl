import { expect, test, type Page } from "@playwright/test";

// The Netcode Lab: server and client in one tab over a simulated link.
// It needs no server of its own; this one only hands out the page.

const lab = "http://127.0.0.1:8090/?lab&view=flat";

const stat = (page: Page, name: string) => page.locator(`[data-stat="${name}"]`);
const number = async (page: Page, name: string) => parseFloat((await stat(page, name).textContent()) ?? "");

test("a bad link deepens rollbacks and a corrupted client is caught", async ({ page }) => {
  await page.goto(lab);
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  await expect(stat(page, "transport")).toHaveText("in this tab");

  await page.getByRole("button", { name: "Perfect" }).click();
  await page.keyboard.down("KeyD");
  await expect.poll(() => number(page, "confirmed")).toBeGreaterThan(120);
  expect(await number(page, "lab rtt")).toBeLessThan(40);

  // 150 ms round trip. The measurement includes up to a tick of waiting
  // at each end, so it reads a little higher than the slider.
  await page.getByRole("button", { name: "Across an ocean" }).click();
  await expect.poll(() => number(page, "lab rtt"), { timeout: 15_000 }).toBeGreaterThan(140);
  await expect.poll(() => number(page, "deepest rollback"), { timeout: 15_000 }).toBeGreaterThan(6);
  expect(await number(page, "desyncs")).toBe(0);

  await page.getByRole("button", { name: "Corrupt my game state" }).click();
  await expect(stat(page, "lab report")).toContainText("player.falls[0], client", {
    timeout: 15_000,
  });
  await expect(stat(page, "desyncs")).toHaveText("1");

  // The client was resynced: it keeps confirming frames and matching
  // checkpoints, and there is no second desync.
  const before = await number(page, "checkpoints");
  await expect.poll(() => number(page, "checkpoints"), { timeout: 15_000 }).toBeGreaterThan(before + 3);
  await expect(stat(page, "desyncs")).toHaveText("1");
  await page.keyboard.up("KeyD");
});

test("changing the input delay restarts the match with that delay", async ({ page }) => {
  await page.goto(lab);
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  await page.getByRole("group", { name: /Input delay/ }).getByRole("button", { name: "2" }).click();
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  await expect.poll(() => number(page, "confirmed")).toBeGreaterThan(60);
  await expect(stat(page, "lab delay")).toHaveText("2");
});

test("the static build plays against bots with no server at all", async ({ page }) => {
  // Served from a sub-path by a plain file server, like a project page.
  const requests: string[] = [];
  page.on("request", (request) => requests.push(new URL(request.url()).pathname));
  await page.goto("http://127.0.0.1:4173/static/?view=flat");
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
  await expect(stat(page, "transport")).toHaveText("in this tab");
  await page.keyboard.down("KeyD");
  await expect.poll(() => number(page, "confirmed")).toBeGreaterThan(120);
  expect(await number(page, "desyncs")).toBe(0);
  expect(requests.filter((path) => !path.startsWith("/static/"))).toEqual([]);

  await page.getByRole("link", { name: "Netcode Lab" }).click();
  await expect(page.getByRole("heading", { name: "Netcode Lab" })).toBeVisible();
  await expect(page.locator(".hud")).toHaveAttribute("data-state", "playing");
});

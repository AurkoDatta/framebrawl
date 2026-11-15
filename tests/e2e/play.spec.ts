import { expect, test, type Page } from "@playwright/test";

// The real server and the built page: a match against bots over each
// transport, and two browsers sharing a room.

const server = "http://127.0.0.1:8090";

const stat = (page: Page, name: string) => page.locator(`[data-stat="${name}"]`);

async function number(page: Page, name: string): Promise<number> {
  return Number(await stat(page, name).textContent());
}

// Holds a key and waits for the server to confirm `frames` more frames.
// A page may have joined a match that is already minutes old, so the
// count starts from wherever it came in.
async function playFor(page: Page, frames: number) {
  await expect(page.locator("#status")).toHaveAttribute("data-state", "playing");
  await expect(stat(page, "confirmed")).toHaveText(/\d/);
  const target = (await number(page, "confirmed")) + frames;
  await page.keyboard.down("KeyD");
  await expect.poll(() => number(page, "confirmed"), { timeout: 30_000 }).toBeGreaterThan(target);
  await page.keyboard.up("KeyD");
}

async function expectHealthy(page: Page) {
  expect(await number(page, "desyncs")).toBe(0);
  expect(await number(page, "checkpoints")).toBeGreaterThan(2);
  // On loopback the client should sit a frame or two ahead of the server.
  // Both numbers come from one read so the page cannot advance between.
  const ahead = await page.evaluate(() => {
    const read = (name: string) =>
      Number(document.querySelector(`[data-stat="${name}"]`)?.textContent);
    return read("frame") - read("confirmed");
  });
  expect(ahead).toBeGreaterThanOrEqual(0);
  expect(ahead).toBeLessThan(15);
}

test("plays against bots over the default transport", async ({ page, browserName }) => {
  await page.goto(server);
  await playFor(page, 300);
  await expectHealthy(page);
  // Firefox will not pair with a loopback candidate, so it lands on the
  // WebSocket unless a LAN address is advertised.
  const expected = browserName === "firefox" ? /webrtc|websocket/ : /webrtc/;
  await expect(stat(page, "transport")).toHaveText(expected);
});

test("plays against bots over the WebSocket", async ({ page }) => {
  await page.goto(`${server}/?transport=websocket`);
  await playFor(page, 300);
  await expectHealthy(page);
  await expect(stat(page, "transport")).toHaveText("websocket");
});

test("two players share a room", async ({ page, context, browserName }) => {
  const room = `e2e-${browserName}`;
  await page.goto(`${server}/?room=${room}`);
  await playFor(page, 120);

  const second = await context.newPage();
  await second.goto(`${server}/?room=${room}`);
  await playFor(second, 120);
  expect(await number(page, "frame")).toBeGreaterThan(240);

  await expectHealthy(page);
  await expectHealthy(second);
  const health = await (await page.request.get(`${server}/api/health`)).json();
  expect(health.rooms).toBeGreaterThanOrEqual(1);
});

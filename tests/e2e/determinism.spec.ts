import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { firstDifference } from "./soak.mjs";

interface Harness {
  straight: string;
  rewound: string;
  rollback8Ms: number;
  saveMs: number;
  stepMs: number;
}

const native = readFileSync(new URL("./pkg/native-checksums.txt", import.meta.url), "utf8");

test("wasm in the browser matches native, with and without rewinds", async ({ page }, testInfo) => {
  await page.goto("/");
  await page.waitForFunction(() => "harness" in window);
  const harness = await page.evaluate(() => (window as unknown as { harness: Harness }).harness);

  expect(firstDifference(harness.straight, native), "first differing frame").toBe(-1);
  expect(firstDifference(harness.rewound, native), "first differing frame after rewinds").toBe(-1);

  // Budget from the design: resimulating 8 frames must fit in 4 ms. CI
  // machines are slower and noisier than a desk, so only fail on a miss
  // large enough to mean a real regression.
  const timings = `rollback of 8 frames ${harness.rollback8Ms.toFixed(3)} ms, save ${harness.saveMs.toFixed(4)} ms, step ${harness.stepMs.toFixed(4)} ms`;
  console.log(`${testInfo.project.name}: ${timings}`);
  testInfo.annotations.push({ type: "timings", description: timings });
  expect(harness.rollback8Ms).toBeLessThan(12);
});

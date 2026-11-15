import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { initSync, soak_checksums, soak_checksums_with_rewinds } from "./pkg/fb_wasm.js";
import { FRAMES, PLAYERS, REWIND_DEPTH, SEED, firstDifference, toHexLines } from "./soak.mjs";

initSync({ module: readFileSync(new URL("./pkg/fb_wasm_bg.wasm", import.meta.url)) });
const native = readFileSync(new URL("./pkg/native-checksums.txt", import.meta.url), "utf8");

test("wasm matches the native checksum stream", () => {
  const wasm = toHexLines(soak_checksums(SEED, PLAYERS, FRAMES));
  assert.equal(firstDifference(wasm, native), -1, "first differing frame");
});

test("wasm rewinds reproduce the straight run", () => {
  const rewound = toHexLines(soak_checksums_with_rewinds(SEED, PLAYERS, FRAMES, REWIND_DEPTH));
  assert.equal(firstDifference(rewound, native), -1, "first differing frame");
});

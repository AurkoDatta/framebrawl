import type { View } from "../view";

/** Something that can draw the match. */
export interface Stage {
  /** What ended up drawing: "webgpu", "webgl" or "flat". */
  readonly kind: string;
  /** A new simulation state arrived. */
  push(view: View, corrected: boolean, nowMs: number): void;
  /** Draw one display frame. */
  frame(nowMs: number): void;
  resize(): void;
}

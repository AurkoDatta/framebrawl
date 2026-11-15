import type { View } from "../view";
import type { Cue } from "./motion";

/** Something that can draw the match. */
export interface Stage {
  /** What ended up drawing: "webgpu", "webgl" or "flat". */
  readonly kind: string;
  /** A new simulation state arrived. */
  push(view: View, corrected: boolean, nowMs: number): void;
  /** Draw one display frame. Returns what happened since the last one. */
  frame(nowMs: number): Cue[];
  resize(): void;
}

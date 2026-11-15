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
  /**
   * Lab only: where the server has each player, as
   * `[frame, (x, y, z, facing, in play) per player]`, or null to hide.
   */
  showServer(truth: Float32Array | null): void;
}

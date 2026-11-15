// Messages between the page and the simulation worker.

export type ToWorker =
  | { type: "packet"; bytes: Uint8Array }
  | { type: "input"; buttons: number; aim: number };

export type FromWorker =
  | { type: "ready"; layout: ViewLayout }
  | { type: "send"; reliable: boolean; bytes: Uint8Array }
  // `corrected` is set when a rollback changed the past since the last view,
  // so the renderer knows a jump is a correction and not real motion.
  | { type: "view"; view: Float32Array; corrected: boolean }
  | { type: "event"; event: GameEvent }
  | { type: "error"; message: string };

/** Float counts for each section of a view, as the wasm side reports them. */
export interface ViewLayout {
  header: number;
  perPlayer: number;
  perTile: number;
  perProp: number;
}

export type GameEvent =
  | { kind: "welcome"; player: number; players: number; grid: number }
  | { kind: "synced"; frame: number }
  | { kind: "rollback"; from: number; depth: number }
  | { kind: "desync"; frame: number }
  | { kind: "desync-located"; frame: number }
  | { kind: "match-over"; scores: number[] };

export const Buttons = { move: 1, jump: 2, shove: 4, dash: 8 } as const;

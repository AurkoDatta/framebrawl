// Messages between the page and the simulation worker.

export interface Network {
  /** Round trip in milliseconds; half is applied in each direction. */
  roundTripMs: number;
  jitterMs: number;
  lossPercent: number;
  duplicatePercent: number;
}

export type ToWorker =
  // The first message decides what the worker runs: a client for a real
  // server, or the Lab, where server and client share the worker.
  | { type: "start"; mode: "online" }
  | { type: "start"; mode: "lab"; seed: number; inputDelay: number; network: Network }
  | { type: "start"; mode: "replay"; bytes: Uint8Array }
  // Replay only: frames to play per tick; 0 pauses.
  | { type: "speed"; speed: number }
  | { type: "packet"; bytes: Uint8Array }
  | { type: "input"; buttons: number; aim: number }
  | { type: "network"; network: Network }
  | { type: "corrupt" };

export type FromWorker =
  | { type: "ready"; layout: ViewLayout }
  | { type: "send"; reliable: boolean; bytes: Uint8Array }
  // `corrected` is set when a rollback changed the past since the last view,
  // so the renderer knows a jump is a correction and not real motion.
  | { type: "view"; view: Float32Array; corrected: boolean }
  // Lab only: where the server has each player, the numbers for the
  // timeline, and the explanation of a desync.
  | { type: "lab"; truth: Float32Array; sample: Float64Array }
  | { type: "report"; text: string }
  | { type: "replay"; progress: ReplayProgress }
  | { type: "event"; event: GameEvent }
  | { type: "error"; message: string };

export interface ReplayProgress {
  frame: number;
  frames: number;
  checkpoints: number;
  matched: number;
  /** First checkpoint frame that did not match, or -1. */
  divergedAt: number;
}

/** Float counts for each section of a view, as the wasm side reports them. */
export interface ViewLayout {
  header: number;
  perPlayer: number;
  perTile: number;
  perProp: number;
}

export type GameEvent =
  | {
      kind: "welcome";
      player: number;
      players: number;
      grid: number;
      durationFrames: number;
      scoreLimit: number;
    }
  | { kind: "synced"; frame: number }
  | { kind: "rollback"; from: number; depth: number }
  | { kind: "desync"; frame: number }
  | { kind: "desync-located"; frame: number }
  | { kind: "match-over"; scores: number[] }
  | { kind: "replay-ended" };

export const Buttons = { move: 1, jump: 2, shove: 4, dash: 8 } as const;

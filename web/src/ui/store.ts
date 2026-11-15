// What the interface shows, kept outside React so the game loop can
// update it without caring who is listening.

import type { TransportKind } from "../transport";

export type Phase =
  | { name: "connecting" }
  | { name: "playing" }
  | { name: "between"; scores: number[] }
  | { name: "failed"; reason: string };

export interface SeatState {
  score: number;
  strain: number;
  inPlay: boolean;
}

export interface Netcode {
  view: string;
  transport: TransportKind | "";
  frame: number;
  confirmed: number;
  rttMs: number;
  inputDelay: number;
  rollbacks: number;
  rollbacksPerSecond: number;
  deepestRollback: number;
  stalls: number;
  lateInputs: number;
  checkpoints: number;
  desyncs: number;
}

export interface HudState {
  phase: Phase;
  me: number;
  seats: SeatState[];
  secondsLeft: number | null;
  scoreLimit: number;
  net: Netcode;
  /** Depth of each recent rollback, newest last. */
  recentRollbacks: number[];
  muted: boolean;
}

const initial: HudState = {
  phase: { name: "connecting" },
  me: 0,
  seats: [],
  secondsLeft: null,
  scoreLimit: 0,
  net: {
    view: "",
    transport: "",
    frame: 0,
    confirmed: 0,
    rttMs: 0,
    inputDelay: 0,
    rollbacks: 0,
    rollbacksPerSecond: 0,
    deepestRollback: 0,
    stalls: 0,
    lateInputs: 0,
    checkpoints: 0,
    desyncs: 0,
  },
  recentRollbacks: [],
  muted: false,
};

let state = initial;
const listeners = new Set<() => void>();

export const hud = {
  get: () => state,
  subscribe(listener: () => void) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  set(patch: Partial<HudState>) {
    state = { ...state, ...patch };
    listeners.forEach((listener) => listener());
  },
  setNet(patch: Partial<Netcode>) {
    hud.set({ net: { ...state.net, ...patch } });
  },
};

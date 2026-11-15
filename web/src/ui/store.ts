// What the interface shows, kept outside React so the game loop can
// update it without caring who is listening.

import type { Network } from "../protocol";
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
  transport: TransportKind | "in this tab" | "";
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

/**
 * "online" plays on a server. "offline" and "lab" run the server in this
 * tab; the Lab adds the controls for the simulated connection.
 */
export type Mode = "online" | "offline" | "lab";

export interface LabState {
  network: Network;
  /** Frames of input delay, or -1 to let the client choose. */
  inputDelay: number;
  showServer: boolean;
  /** The explanation of the most recent desync, if there has been one. */
  report: string;
  /** Per-second figures the timeline cannot show as bars. */
  kilobitsUp: number;
  kilobitsDown: number;
  lossPercent: number;
  serverFrame: number;
}

export interface HudState {
  mode: Mode;
  lab: LabState;
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

export const cleanNetwork: Network = {
  roundTripMs: 0,
  jitterMs: 0,
  lossPercent: 0,
  duplicatePercent: 0,
};

const initial: HudState = {
  mode: "online",
  lab: {
    network: { roundTripMs: 120, jitterMs: 15, lossPercent: 2, duplicatePercent: 0 },
    inputDelay: -1,
    showServer: true,
    report: "",
    kilobitsUp: 0,
    kilobitsDown: 0,
    lossPercent: 0,
    serverFrame: 0,
  },
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
  setLab(patch: Partial<LabState>) {
    hud.set({ lab: { ...state.lab, ...patch } });
  },
};

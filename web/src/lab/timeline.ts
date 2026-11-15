// The last few seconds of netcode, one entry per tick, kept outside React
// because it changes 60 times a second and only a canvas reads it.

export interface Tick {
  /** Frames the client is simulating ahead of the last confirmed one. */
  ahead: number;
  /** Frames resimulated this tick; 0 when the prediction held. */
  rollback: number;
  /** The client ran out of confirmed frames and had to wait. */
  stalled: boolean;
  /** Packets the link dropped this tick, both directions. */
  lost: number;
  inputDelay: number;
}

export const TIMELINE_TICKS = 480;

export interface Totals {
  clientFrame: number;
  serverFrame: number;
  roundTripMs: number;
  stalls: number;
  lateInputs: number;
  sent: number;
  lost: number;
  bytesUp: number;
  bytesDown: number;
  desyncs: number;
}

const ticks: Tick[] = [];
let totals: Totals | null = null;

/** Takes one row of `Lab.sample()`. */
export function record(sample: Float64Array) {
  const at = (index: number) => sample[index] ?? 0;
  const next: Totals = {
    clientFrame: at(0),
    serverFrame: at(2),
    roundTripMs: at(5),
    stalls: at(6),
    lateInputs: at(7),
    sent: at(8),
    lost: at(9),
    bytesUp: at(10),
    bytesDown: at(11),
    desyncs: at(12),
  };
  ticks.push({
    ahead: Math.max(0, at(0) - at(1)),
    rollback: at(3),
    stalled: totals !== null && next.stalls > totals.stalls,
    lost: totals === null ? 0 : next.lost - totals.lost,
    inputDelay: at(4),
  });
  if (ticks.length > TIMELINE_TICKS) ticks.shift();
  totals = next;
}

export function reset() {
  ticks.length = 0;
  totals = null;
}

export const recent = (): readonly Tick[] => ticks;
export const latestTotals = (): Totals | null => totals;

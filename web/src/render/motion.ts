// Turns the 60 Hz stream of simulation states into something smooth to
// draw at any refresh rate.
//
// Two things happen here. Between ticks, positions are interpolated. And
// when a rollback rewrites the recent past, the body does not teleport to
// its corrected place: the difference is kept as an offset that fades over
// a few frames, so a correction reads as a nudge.

import type { Body, Player, Prop, Tile, View } from "../view";

const TICK_MS = 1000 / 60;
/// Share of a correction offset that survives each tick.
const OFFSET_KEPT = 0.82;
/// Corrections larger than this are shown at once: it is a respawn or a
/// resync, and sliding across the arena would look worse than a cut.
const SNAP_DISTANCE = 3;

export type Cue =
  | { kind: "shove"; seat: number; x: number; y: number; z: number; turn: number }
  | { kind: "dash"; seat: number; x: number; y: number; z: number }
  | { kind: "hit"; seat: number; x: number; y: number; z: number }
  | { kind: "ring-out"; seat: number; x: number; y: number; z: number }
  | { kind: "tile-fell"; x: number; z: number };

export interface Posed<T> {
  now: T;
  x: number;
  y: number;
  z: number;
}

export interface Sample {
  view: View;
  players: (Posed<Player> & { turn: number })[];
  tiles: (Posed<Tile> & Body)[];
  props: (Posed<Prop> & Body)[];
}

type Offset = [number, number, number];

interface Keyframe {
  view: View;
  playerOffsets: Offset[];
  propOffsets: Offset[];
}

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

function turnOf(facing: number): number {
  return (facing / 256) * 2 * Math.PI;
}

function lerpTurn(a: number, b: number, t: number): number {
  let delta = (b - a) % (2 * Math.PI);
  if (delta > Math.PI) delta -= 2 * Math.PI;
  if (delta < -Math.PI) delta += 2 * Math.PI;
  return a + delta * t;
}

// Normalised lerp: cheap, and indistinguishable from slerp across one tick.
function blendRotation(a: Body, b: Body, t: number) {
  const sign = a.qx * b.qx + a.qy * b.qy + a.qz * b.qz + a.qw * b.qw < 0 ? -1 : 1;
  const qx = lerp(a.qx, b.qx * sign, t);
  const qy = lerp(a.qy, b.qy * sign, t);
  const qz = lerp(a.qz, b.qz * sign, t);
  const qw = lerp(a.qw, b.qw * sign, t);
  const length = Math.hypot(qx, qy, qz, qw) || 1;
  return { qx: qx / length, qy: qy / length, qz: qz / length, qw: qw / length };
}

function decayed(offset: Offset | undefined): Offset {
  if (!offset) return [0, 0, 0];
  return [offset[0] * OFFSET_KEPT, offset[1] * OFFSET_KEPT, offset[2] * OFFSET_KEPT];
}

export class Motion {
  private previous: Keyframe | null = null;
  private current: Keyframe | null = null;
  private arrivedAt = 0;
  private cues: Cue[] = [];

  push(view: View, corrected: boolean, nowMs: number) {
    const last = this.current;
    const next: Keyframe = {
      view,
      playerOffsets: view.players.map((_, i) => decayed(last?.playerOffsets[i])),
      propOffsets: view.props.map((_, i) => decayed(last?.propOffsets[i])),
    };

    const continuous = last !== null && view.frame > last.view.frame && view.frame - last.view.frame < 30;
    if (last && continuous) {
      this.collectCues(last.view, view);
      if (corrected) {
        const ticks = view.frame - last.view.frame;
        view.players.forEach((player, i) => {
          const before = last.view.players[i];
          if (!before || !before.inPlay || !player.inPlay) return;
          // Where the player would be now had nothing been rewritten.
          const expected: Offset = [
            before.x + (before.vx * ticks) / 60,
            before.y + (before.vy * ticks) / 60,
            before.z + (before.vz * ticks) / 60,
          ];
          const error: Offset = [expected[0] - player.x, expected[1] - player.y, expected[2] - player.z];
          const size = Math.hypot(...error);
          if (size > 0.03 && size < SNAP_DISTANCE) {
            const offset = next.playerOffsets[i]!;
            offset[0] += error[0];
            offset[1] += error[1];
            offset[2] += error[2];
          }
        });
      }
    }

    // A prop that fell out of the world shifts the rest down the list, so
    // offsets and interpolation by index would pair up different bodies.
    const sameProps = last !== null && last.view.props.length === view.props.length;
    if (!continuous || !sameProps) {
      next.propOffsets = view.props.map(() => [0, 0, 0]);
    }

    this.previous = continuous ? last : null;
    this.current = next;
    this.arrivedAt = nowMs;
  }

  takeCues(): Cue[] {
    const cues = this.cues;
    this.cues = [];
    return cues;
  }

  sample(nowMs: number): Sample | null {
    const to = this.current;
    if (!to) return null;
    const from = this.previous ?? to;
    const t = Math.min(1, Math.max(0, (nowMs - this.arrivedAt) / TICK_MS));
    const pairProps = from.view.props.length === to.view.props.length;

    return {
      view: to.view,
      players: to.view.players.map((player, i) => {
        const before = from.view.players[i];
        const a = before && before.inPlay === player.inPlay ? before : player;
        const offA = (a === before ? from.playerOffsets[i] : to.playerOffsets[i]) ?? [0, 0, 0];
        const offB = to.playerOffsets[i] ?? [0, 0, 0];
        return {
          now: player,
          x: lerp(a.x + offA[0], player.x + offB[0], t),
          y: lerp(a.y + offA[1], player.y + offB[1], t),
          z: lerp(a.z + offA[2], player.z + offB[2], t),
          turn: lerpTurn(turnOf(a.facing), turnOf(player.facing), t),
        };
      }),
      tiles: to.view.tiles.map((tile, i) => {
        const a = from.view.tiles[i] ?? tile;
        return {
          now: tile,
          x: lerp(a.x, tile.x, t),
          y: lerp(a.y, tile.y, t),
          z: lerp(a.z, tile.z, t),
          ...blendRotation(a, tile, t),
        };
      }),
      props: to.view.props.map((prop, i) => {
        const a = (pairProps && from.view.props[i]) || prop;
        return {
          now: prop,
          x: lerp(a.x, prop.x, t),
          y: lerp(a.y, prop.y, t),
          z: lerp(a.z, prop.z, t),
          ...blendRotation(a, prop, t),
        };
      }),
    };
  }

  // Things worth a flash or a sound are read off the difference between
  // two states. A rollback can occasionally replay one; that is accepted.
  private collectCues(before: View, after: View) {
    after.players.forEach((player, seat) => {
      const was = before.players[seat];
      if (!was) return;
      const at = { seat, x: player.x, y: player.y, z: player.z };
      if (player.shoveCooldown > was.shoveCooldown) {
        this.cues.push({ kind: "shove", ...at, turn: turnOf(player.facing) });
      }
      if (player.dashCooldown > was.dashCooldown) {
        this.cues.push({ kind: "dash", ...at });
      }
      if (player.strain > was.strain) {
        this.cues.push({ kind: "hit", ...at });
      }
      if (was.inPlay && !player.inPlay) {
        this.cues.push({ kind: "ring-out", seat, x: was.x, y: was.y, z: was.z });
      }
    });
    after.tiles.forEach((tile, i) => {
      const was = before.tiles[i];
      if (was && was.state < 2 && tile.state >= 2) {
        this.cues.push({ kind: "tile-fell", x: was.x, z: was.z });
      }
    });
  }
}

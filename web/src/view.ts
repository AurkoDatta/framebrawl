// Reads the flat float list the worker posts after each tick.

import type { ViewLayout } from "./protocol";

export interface Player {
  x: number;
  y: number;
  z: number;
  facing: number;
  inPlay: boolean;
  grounded: boolean;
  shielded: boolean;
  stunned: boolean;
  strain: number;
  score: number;
}

export interface Body {
  x: number;
  y: number;
  z: number;
}

export const TileState = { solid: 0, cracking: 1, falling: 2, gone: 3 } as const;

export interface Tile extends Body {
  state: number;
}

export interface View {
  frame: number;
  confirmedFrame: number;
  me: number;
  rttMs: number;
  inputDelay: number;
  rollbacks: number;
  resimulatedFrames: number;
  deepestRollback: number;
  stalls: number;
  substitutedInputs: number;
  checkpointsVerified: number;
  finished: boolean;
  players: Player[];
  tiles: Tile[];
  props: Body[];
}

export function decodeView(raw: Float32Array, layout: ViewLayout): View {
  const at = (index: number) => raw[index] ?? 0;
  const playerCount = at(3);
  const tileCount = at(4);
  const propCount = at(5);

  let offset = layout.header;
  const players: Player[] = [];
  for (let i = 0; i < playerCount; i++, offset += layout.perPlayer) {
    const flags = at(offset + 7);
    players.push({
      x: at(offset),
      y: at(offset + 1),
      z: at(offset + 2),
      facing: at(offset + 6),
      inPlay: (flags & 1) !== 0,
      grounded: (flags & 2) !== 0,
      shielded: (flags & 4) !== 0,
      stunned: (flags & 8) !== 0,
      strain: at(offset + 8),
      score: at(offset + 9),
    });
  }
  const tiles: Tile[] = [];
  for (let i = 0; i < tileCount; i++, offset += layout.perTile) {
    tiles.push({ state: at(offset), x: at(offset + 1), y: at(offset + 2), z: at(offset + 3) });
  }
  const props: Body[] = [];
  for (let i = 0; i < propCount; i++, offset += layout.perProp) {
    props.push({ x: at(offset), y: at(offset + 1), z: at(offset + 2) });
  }

  return {
    frame: at(0),
    confirmedFrame: at(1),
    me: at(2),
    rttMs: at(6),
    inputDelay: at(7),
    rollbacks: at(8),
    resimulatedFrames: at(9),
    deepestRollback: at(10),
    stalls: at(11),
    substitutedInputs: at(13),
    checkpointsVerified: at(14),
    finished: at(15) !== 0,
    players,
    tiles,
    props,
  };
}

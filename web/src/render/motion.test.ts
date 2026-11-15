import assert from "node:assert/strict";
import { test } from "node:test";

import type { Player, View } from "../view";
import { Motion } from "./motion.ts";

const TICK = 1000 / 60;

function player(patch: Partial<Player>): Player {
  return {
    x: 0,
    y: 1,
    z: 0,
    vx: 0,
    vy: 0,
    vz: 0,
    facing: 0,
    inPlay: true,
    grounded: true,
    shielded: false,
    stunned: false,
    strain: 0,
    score: 0,
    shoveCooldown: 0,
    dashCooldown: 0,
    ...patch,
  };
}

function view(frame: number, players: Player[]): View {
  return {
    frame,
    confirmedFrame: frame,
    me: 0,
    rttMs: 0,
    inputDelay: 0,
    rollbacks: 0,
    resimulatedFrames: 0,
    deepestRollback: 0,
    stalls: 0,
    substitutedInputs: 0,
    checkpointsVerified: 0,
    finished: false,
    players,
    tiles: [],
    props: [],
  };
}

test("positions are interpolated between ticks", () => {
  const motion = new Motion();
  motion.push(view(1, [player({ x: 0 })]), false, 0);
  motion.push(view(2, [player({ x: 1 })]), false, TICK);
  assert.equal(motion.sample(TICK)!.players[0]!.x, 0);
  assert.ok(Math.abs(motion.sample(TICK * 1.5)!.players[0]!.x - 0.5) < 1e-9);
  assert.equal(motion.sample(TICK * 3)!.players[0]!.x, 1);
});

test("a correction is faded in rather than shown at once", () => {
  const motion = new Motion();
  // Moving at 6 units a second along x, then a rollback puts the player
  // half a unit to the side of where it was heading.
  motion.push(view(1, [player({ x: 0, vx: 6 })]), false, 0);
  motion.push(view(2, [player({ x: 0.1, vx: 6 })]), false, TICK);
  motion.push(view(3, [player({ x: 0.2, z: 0.5, vx: 6 })]), true, TICK * 2);

  // At the moment the corrected state is fully shown, the player is drawn
  // where it would have been without the correction.
  const shown = motion.sample(TICK * 3)!.players[0]!;
  assert.ok(Math.abs(shown.z) < 1e-6, `z jumped to ${shown.z}`);

  // With no further corrections the drawn position closes on the truth.
  let frame = 3;
  let error = Infinity;
  for (let i = 0; i < 30; i++) {
    frame += 1;
    motion.push(view(frame, [player({ x: 0.2, z: 0.5 })]), false, TICK * (frame - 1));
    const next = Math.abs(motion.sample(TICK * frame)!.players[0]!.z - 0.5);
    assert.ok(next < error, "the offset shrinks every tick");
    error = next;
  }
  assert.ok(error < 0.01);
});

test("a correction across the arena is shown at once", () => {
  const motion = new Motion();
  motion.push(view(1, [player({ x: 0 })]), false, 0);
  motion.push(view(2, [player({ x: 8 })]), true, TICK);
  assert.equal(motion.sample(TICK * 2)!.players[0]!.x, 8);
});

test("uncorrected motion gets no offset", () => {
  const motion = new Motion();
  motion.push(view(1, [player({ x: 0, vx: 6 })]), false, 0);
  // A shove changes velocity sharply; that is real motion, not an error.
  motion.push(view(2, [player({ x: 0.1, z: 0.4 })]), false, TICK);
  assert.equal(motion.sample(TICK * 2)!.players[0]!.z, 0.4);
});

test("a respawn does not slide in from the place of the fall", () => {
  const motion = new Motion();
  motion.push(view(1, [player({ x: 9, y: -12, inPlay: false })]), false, 0);
  motion.push(view(2, [player({ x: 0, y: 3, inPlay: true })]), false, TICK);
  const shown = motion.sample(TICK * 1.5)!.players[0]!;
  assert.equal(shown.x, 0);
  assert.equal(shown.y, 3);
});

test("cues are read off the difference between states", () => {
  const motion = new Motion();
  motion.push(view(1, [player({}), player({ x: 2 })]), false, 0);
  motion.push(
    view(2, [player({ shoveCooldown: 24 }), player({ x: 2, strain: 12 })]),
    false,
    TICK,
  );
  motion.push(view(3, [player({ shoveCooldown: 23 }), player({ x: 2, inPlay: false })]), false, TICK * 2);
  assert.deepEqual(
    motion.takeCues().map((cue) => cue.kind),
    ["shove", "hit", "ring-out"],
  );
  assert.deepEqual(motion.takeCues(), []);
});

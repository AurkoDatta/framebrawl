// Keyboard state to the two bytes the simulation takes: a button bitfield
// and an aim in 1/256 of a turn. Players run in the direction they aim.

import { Buttons } from "./protocol";

const held = new Set<string>();
let aim = 0;

const directions: Record<string, [number, number]> = {
  KeyW: [0, -1],
  ArrowUp: [0, -1],
  KeyS: [0, 1],
  ArrowDown: [0, 1],
  KeyA: [-1, 0],
  ArrowLeft: [-1, 0],
  KeyD: [1, 0],
  ArrowRight: [1, 0],
};

const actions: Record<string, number> = {
  Space: Buttons.jump,
  KeyJ: Buttons.shove,
  KeyK: Buttons.dash,
};

export function listen(target: Window, changed: () => void) {
  const handle = (down: boolean) => (event: KeyboardEvent) => {
    if (!(event.code in directions) && !(event.code in actions)) {
      return;
    }
    event.preventDefault();
    if (down) {
      held.add(event.code);
    } else {
      held.delete(event.code);
    }
    changed();
  };
  target.addEventListener("keydown", handle(true));
  target.addEventListener("keyup", handle(false));
  target.addEventListener("blur", () => {
    held.clear();
    changed();
  });
}

export function current(): { buttons: number; aim: number } {
  let dx = 0;
  let dz = 0;
  let buttons = 0;
  for (const code of held) {
    const direction = directions[code];
    if (direction) {
      dx += direction[0];
      dz += direction[1];
    }
    buttons |= actions[code] ?? 0;
  }
  if (dx !== 0 || dz !== 0) {
    // Aim 0 points along +x and 64 along +z.
    aim = Math.round((Math.atan2(dz, dx) / (2 * Math.PI)) * 256) & 255;
    buttons |= Buttons.move;
  }
  return { buttons, aim };
}

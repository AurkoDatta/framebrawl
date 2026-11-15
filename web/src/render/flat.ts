// A plain top-down drawing of the match, used when the browser cannot
// give us a 3D context at all.

import { TileState, type View } from "../view";
import { seatColours } from "../palette";
import type { Stage } from "./stage";

const TILE = 2;

function draw(context: CanvasRenderingContext2D, view: View) {
  const { width, height } = context.canvas;
  const grid = Math.round(Math.sqrt(view.tiles.length));
  const scale = width / ((grid + 2) * TILE);
  const toX = (x: number) => width / 2 + x * scale;
  const toY = (z: number) => height / 2 + z * scale;
  // Things shrink as they fall so a ring-out reads without a third axis.
  const depth = (y: number) => Math.max(0.2, Math.min(1.3, 1 + y / 14));

  context.clearRect(0, 0, width, height);

  for (const tile of view.tiles) {
    if (tile.state === TileState.gone) {
      continue;
    }
    const size = TILE * scale * (tile.state === TileState.falling ? depth(tile.y) : 1) - 2;
    context.fillStyle =
      tile.state === TileState.solid
        ? "#2a2e36"
        : tile.state === TileState.cracking
          ? "#5a3b33"
          : "#1d2026";
    context.fillRect(toX(tile.x) - size / 2, toY(tile.z) - size / 2, size, size);
  }

  context.fillStyle = "#6b7280";
  for (const prop of view.props) {
    const size = 0.8 * scale * depth(prop.y);
    context.fillRect(toX(prop.x) - size / 2, toY(prop.z) - size / 2, size, size);
  }

  view.players.forEach((player, seat) => {
    if (!player.inPlay) {
      return;
    }
    const x = toX(player.x);
    const y = toY(player.z);
    const radius = 0.45 * scale * depth(player.y);
    const colour = seatColours[seat] ?? "#ffffff";
    const turn = (player.facing / 256) * 2 * Math.PI;

    context.globalAlpha = player.shielded ? 0.5 : 1;
    context.fillStyle = colour;
    context.beginPath();
    context.arc(x, y, radius, 0, 2 * Math.PI);
    context.fill();
    context.globalAlpha = 1;

    context.strokeStyle = player.stunned ? "#ffffff" : colour;
    context.lineWidth = 3;
    context.beginPath();
    context.moveTo(x, y);
    context.lineTo(x + Math.cos(turn) * radius * 1.8, y + Math.sin(turn) * radius * 1.8);
    context.stroke();

    if (seat === view.me) {
      context.strokeStyle = "#e6e3dc";
      context.lineWidth = 2;
      context.beginPath();
      context.arc(x, y, radius + 5, 0, 2 * Math.PI);
      context.stroke();
    }
  });
}

export function createFlatStage(canvas: HTMLCanvasElement): Stage {
  const context = canvas.getContext("2d")!;
  let latest: View | null = null;
  const resize = () => {
    const side = Math.min(canvas.clientWidth, canvas.clientHeight) * devicePixelRatio;
    canvas.width = side;
    canvas.height = side;
  };
  resize();
  return {
    kind: "flat",
    push(view) {
      latest = view;
    },
    frame() {
      if (latest) draw(context, latest);
      return [];
    },
    resize,
    showServer() {},
  };
}

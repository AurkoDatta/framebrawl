// The simulation runs here, off the main thread, so that rendering and
// rollback never compete for the same frame budget and the match keeps
// ticking when the tab is in the background.

import init, { NetClient } from "./wasm/fb_wasm.js";
import type { FromWorker, GameEvent, ToWorker } from "./protocol";

const TICK_MS = 1000 / 60;
// Never try to make up more than this many ticks after a long pause; the
// client's own pacing closes the remaining gap.
const MAX_BACKLOG = 4;

const post = (message: FromWorker, transfer: Transferable[] = []) =>
  (self as unknown as Worker).postMessage(message, transfer);

function decodeEvent(raw: Float64Array): GameEvent | null {
  const [kind = 0, a = 0, b = 0, c = 0, d = 0, e = 0] = raw;
  switch (kind) {
    case 1:
      return {
        kind: "welcome",
        player: a,
        players: b,
        grid: c,
        durationFrames: d,
        scoreLimit: e,
      };
    case 2:
      return { kind: "synced", frame: a };
    case 3:
      return { kind: "rollback", from: a, depth: b };
    case 4:
      return { kind: "desync", frame: a };
    case 5:
      return { kind: "desync-located", frame: a };
    case 6:
      return { kind: "match-over", scores: Array.from(raw.subarray(1)) };
    default:
      return null;
  }
}

async function main() {
  await init();
  const client = new NetClient();
  let buttons = 0;
  let aim = 0;
  let corrected = false;

  const drainEvents = () => {
    for (;;) {
      const event = decodeEvent(client.next_event());
      if (!event) {
        return;
      }
      corrected ||= event.kind === "rollback" || event.kind === "synced";
      post({ type: "event", event });
    }
  };

  self.onmessage = (message: MessageEvent<ToWorker>) => {
    const data = message.data;
    if (data.type === "input") {
      buttons = data.buttons;
      aim = data.aim;
      return;
    }
    try {
      client.receive(data.bytes, performance.now() >>> 0);
    } catch (error) {
      post({ type: "error", message: String(error) });
    }
    drainEvents();
  };

  const tick = () => {
    const count = client.tick(buttons, aim, performance.now() >>> 0);
    for (let i = 0; i < count; i++) {
      const bytes = client.outgoing_bytes(i);
      post({ type: "send", reliable: client.outgoing_reliable(i), bytes }, [bytes.buffer]);
    }
    drainEvents();
  };

  let next = performance.now();
  setInterval(() => {
    const now = performance.now();
    if (now - next > MAX_BACKLOG * TICK_MS) {
      next = now - TICK_MS;
    }
    let ticked = false;
    while (next <= now) {
      next += TICK_MS;
      tick();
      ticked = true;
    }
    if (ticked) {
      const view = client.view();
      if (view.length > 0) {
        post({ type: "view", view, corrected }, [view.buffer]);
        corrected = false;
      }
    }
  }, 2);

  const [header = 0, perPlayer = 0, perTile = 0, perProp = 0] = NetClient.view_layout();
  post({ type: "ready", layout: { header, perPlayer, perTile, perProp } });
}

main().catch((error) => post({ type: "error", message: String(error) }));

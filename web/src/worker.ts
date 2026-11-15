// The simulation runs here, off the main thread, so that rendering and
// rollback never compete for the same frame budget and the match keeps
// ticking when the tab is in the background.

import init, { Lab, NetClient } from "./wasm/fb_wasm.js";
import type { FromWorker, GameEvent, Network, ToWorker } from "./protocol";

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

// What the worker steps 60 times a second. Online it is a client whose
// packets travel through the page; in the Lab it is a client and a server
// together, and nothing leaves the worker.
interface Runner {
  receive(bytes: Uint8Array): void;
  tick(buttons: number, aim: number): void;
  nextEvent(): Float64Array;
  view(): Float32Array;
  afterTick(): void;
  network(network: Network): void;
  corrupt(): void;
}

function onlineRunner(): Runner {
  const client = new NetClient();
  return {
    receive: (bytes) => client.receive(bytes, performance.now() >>> 0),
    tick(buttons, aim) {
      const count = client.tick(buttons, aim, performance.now() >>> 0);
      for (let i = 0; i < count; i++) {
        const bytes = client.outgoing_bytes(i);
        post({ type: "send", reliable: client.outgoing_reliable(i), bytes }, [bytes.buffer]);
      }
    },
    nextEvent: () => client.next_event(),
    view: () => client.view(),
    afterTick() {},
    network() {},
    corrupt() {},
  };
}

function labRunner(seed: number, inputDelay: number, network: Network): Runner {
  const lab = new Lab(BigInt(seed), inputDelay);
  const setNetwork = (n: Network) =>
    lab.set_network(
      Math.round(n.roundTripMs / 2),
      Math.round(n.jitterMs),
      Math.round(n.lossPercent * 10),
      Math.round(n.duplicatePercent * 10),
    );
  setNetwork(network);
  return {
    receive() {},
    tick: (buttons, aim) => lab.tick(buttons, aim),
    nextEvent: () => lab.next_event(),
    view: () => lab.view(),
    afterTick() {
      const truth = lab.truth();
      const sample = lab.sample();
      post({ type: "lab", truth, sample }, [truth.buffer, sample.buffer]);
      const text = lab.take_report();
      if (text) post({ type: "report", text });
    },
    network: setNetwork,
    corrupt: () => lab.corrupt(),
  };
}

async function main() {
  await init();
  let runner: Runner | null = null;
  let buttons = 0;
  let aim = 0;
  let corrected = false;

  const drainEvents = () => {
    for (;;) {
      const event = runner && decodeEvent(runner.nextEvent());
      if (!event) {
        return;
      }
      corrected ||= event.kind === "rollback" || event.kind === "synced";
      post({ type: "event", event });
    }
  };

  self.onmessage = (message: MessageEvent<ToWorker>) => {
    const data = message.data;
    switch (data.type) {
      case "start":
        runner =
          data.mode === "lab"
            ? labRunner(data.seed, data.inputDelay, data.network)
            : onlineRunner();
        break;
      case "input":
        buttons = data.buttons;
        aim = data.aim;
        break;
      case "network":
        runner?.network(data.network);
        break;
      case "corrupt":
        runner?.corrupt();
        break;
      case "packet":
        try {
          runner?.receive(data.bytes);
        } catch (error) {
          post({ type: "error", message: String(error) });
        }
        drainEvents();
        break;
    }
  };

  let next = performance.now();
  setInterval(() => {
    if (!runner) return;
    const now = performance.now();
    if (now - next > MAX_BACKLOG * TICK_MS) {
      next = now - TICK_MS;
    }
    let ticked = false;
    while (next <= now) {
      next += TICK_MS;
      runner.tick(buttons, aim);
      runner.afterTick();
      drainEvents();
      ticked = true;
    }
    if (ticked) {
      const view = runner.view();
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

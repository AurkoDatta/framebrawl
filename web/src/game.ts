// Wires the pieces together: transport on this thread, simulation in a
// worker, a stage to draw on, and the interface store.

import * as input from "./input";
import type { FromWorker, Network, ToWorker, ViewLayout } from "./protocol";
import { createFlatStage } from "./render/flat";
import type { Stage } from "./render/stage";
import * as sound from "./sound";
import { connect, type Transport } from "./transport";
import * as timeline from "./lab/timeline";
import { cleanNetwork, hud, type Mode } from "./ui/store";
import { decodeView, type View } from "./view";

type ViewChoice = "webgpu" | "webgl" | "flat";

const ROLLBACK_HISTORY = 90;

// A canvas that has handed out one kind of context will not hand out
// another, so every attempt gets a fresh element.
function freshCanvas(): HTMLCanvasElement {
  const old = document.querySelector<HTMLCanvasElement>("#arena")!;
  const canvas = old.cloneNode() as HTMLCanvasElement;
  old.replaceWith(canvas);
  return canvas;
}

// The best view the browser can manage: WebGPU, then WebGL2, then a flat
// drawing. `onLost` is called if the GPU device disappears mid-match.
async function createStage(choice: ViewChoice, onLost: () => void): Promise<Stage> {
  const steps: ViewChoice[] = ["webgpu", "webgl", "flat"];
  for (const step of steps.slice(steps.indexOf(choice))) {
    if (step === "flat") break;
    try {
      // three.js is most of the download; load it apart from the interface.
      const { createScene } = await import("./render/scene");
      return await createScene(freshCanvas(), { forceWebGL: step === "webgl", onLost });
    } catch (error) {
      console.info(`${step} view unavailable:`, error);
    }
  }
  return createFlatStage(freshCanvas());
}

export interface Game {
  /** Lab: change the simulated link while the match runs. */
  setNetwork(network: Network): void;
  /** Lab: break the client's state on purpose. */
  corrupt(): void;
  /** Lab: start over with a different input delay. */
  restart(inputDelay: number): void;
}

export async function startGame(mode: Mode): Promise<Game> {
  const params = new URLSearchParams(location.search);
  hud.set({ mode });
  const requested = params.get("view");
  const first: ViewChoice = requested === "flat" || requested === "webgl" ? requested : "webgpu";

  let stage: Stage | null = null;
  const useStage = async (choice: ViewChoice) => {
    stage = null;
    // A lost WebGPU device is retried once on WebGL2, and after that the
    // flat view, which needs no GPU.
    stage = await createStage(choice, () => void useStage(choice === "webgpu" ? "webgl" : "flat"));
    hud.setNet({ view: stage.kind });
  };
  await useStage(first);
  window.addEventListener("resize", () => stage?.resize());

  let worker: Worker;
  const toWorker = (message: ToWorker, transfer: Transferable[] = []) =>
    worker.postMessage(message, transfer);

  let layout: ViewLayout | null = null;
  let latest: View | null = null;
  let transport: Transport | null = null;
  let durationFrames = 0;
  let desyncs = 0;
  let recentRollbacks: number[] = [];

  const onMessage = async (message: MessageEvent<FromWorker>) => {
    const data = message.data;
    switch (data.type) {
      case "ready":
        layout = data.layout;
        if (mode !== "online") {
          if (mode === "offline") hud.setLab({ network: cleanNetwork, showServer: false });
          const lab = hud.get().lab;
          toWorker({
            type: "start",
            mode: "lab",
            seed: Math.floor(Math.random() * 2 ** 31),
            inputDelay: lab.inputDelay,
            network: lab.network,
          });
          hud.setNet({ transport: "in this tab" });
          break;
        }
        toWorker({ type: "start", mode: "online" });
        try {
          transport = await connect({
            room: params.get("room"),
            websocketOnly: params.get("transport") === "websocket",
            webrtcTimeoutMs: 1500,
          });
        } catch {
          hud.set({ phase: { name: "failed", reason: "No server answered at this address." } });
          return;
        }
        hud.setNet({ transport: transport.kind });
        transport.onmessage = (bytes) => toWorker({ type: "packet", bytes }, [bytes.buffer]);
        transport.onclose = () =>
          hud.set({ phase: { name: "failed", reason: "The connection to the server was lost." } });
        break;
      case "send":
        transport?.send(data.bytes, data.reliable);
        break;
      case "view":
        if (layout) {
          latest = decodeView(data.view, layout);
          stage?.push(latest, data.corrected, performance.now());
        }
        break;
      case "event": {
        const event = data.event;
        if (event.kind === "welcome") {
          durationFrames = event.durationFrames;
          hud.set({ scoreLimit: event.scoreLimit });
        } else if (event.kind === "synced") {
          hud.set({ phase: { name: "playing" } });
        } else if (event.kind === "rollback") {
          recentRollbacks = [...recentRollbacks.slice(1 - ROLLBACK_HISTORY), event.depth];
        } else if (event.kind === "desync") {
          desyncs += 1;
        } else if (event.kind === "match-over") {
          hud.set({ phase: { name: "between", scores: event.scores } });
        }
        break;
      }
      case "lab":
        timeline.record(data.sample);
        stage?.showServer(hud.get().lab.showServer ? data.truth : null);
        break;
      case "report":
        hud.setLab({ report: data.text });
        break;
      case "error":
        hud.set({ phase: { name: "failed", reason: data.message } });
        break;
    }
  };

  const launch = () => {
    worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = onMessage;
  };
  launch();

  input.listen(window, () => {
    sound.unlock();
    toWorker({ type: "input", ...input.current() });
  });

  // The picture is redrawn every display frame; the numbers around it
  // change far less often than that and are pushed ten times a second.
  let lastHudAt = 0;
  let lastRateAt = 0;
  let lastRollbacks = 0;
  let rollbacksPerSecond = 0;

  const frame = (now: number) => {
    const cues = stage?.frame(now) ?? [];
    if (latest) {
      for (const cue of cues) sound.play(cue, latest.me);
    }
    if (latest && now - lastHudAt >= 100) {
      lastHudAt = now;
      if (now - lastRateAt >= 1000) {
        rollbacksPerSecond = Math.round(((latest.rollbacks - lastRollbacks) * 1000) / (now - lastRateAt));
        lastRollbacks = latest.rollbacks;
        lastRateAt = now;
      }
      hud.set({
        me: latest.me,
        seats: latest.players.map((p) => ({ score: p.score, strain: p.strain, inPlay: p.inPlay })),
        secondsLeft:
          durationFrames > 0 ? Math.max(0, Math.ceil((durationFrames - latest.frame) / 60)) : null,
        recentRollbacks,
        net: {
          ...hud.get().net,
          frame: latest.frame,
          confirmed: latest.confirmedFrame,
          rttMs: latest.rttMs,
          inputDelay: latest.inputDelay,
          rollbacks: latest.rollbacks,
          rollbacksPerSecond,
          deepestRollback: latest.deepestRollback,
          stalls: latest.stalls,
          lateInputs: latest.substitutedInputs,
          checkpoints: latest.checkpointsVerified,
          desyncs,
        },
      });
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  // Link figures that only make sense per second.
  let lastTotals = timeline.latestTotals();
  setInterval(() => {
    const totals = timeline.latestTotals();
    if (totals && lastTotals && totals.sent >= lastTotals.sent) {
      const sent = totals.sent - lastTotals.sent;
      hud.setLab({
        kilobitsUp: ((totals.bytesUp - lastTotals.bytesUp) * 8) / 1000,
        kilobitsDown: ((totals.bytesDown - lastTotals.bytesDown) * 8) / 1000,
        lossPercent: sent > 0 ? ((totals.lost - lastTotals.lost) / sent) * 100 : 0,
        serverFrame: totals.serverFrame,
      });
    }
    lastTotals = totals;
  }, 1000);

  return {
    setNetwork(network) {
      hud.setLab({ network });
      toWorker({ type: "network", network });
    },
    corrupt() {
      toWorker({ type: "corrupt" });
    },
    restart(inputDelay) {
      worker.terminate();
      timeline.reset();
      latest = null;
      desyncs = 0;
      recentRollbacks = [];
      lastRollbacks = 0;
      hud.set({ phase: { name: "connecting" } });
      hud.setLab({ inputDelay, report: "" });
      launch();
    },
  };
}

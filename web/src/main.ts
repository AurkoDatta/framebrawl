import * as input from "./input";
import { seatColours } from "./palette";
import { createFlatStage } from "./render/flat";
import { createScene } from "./render/scene";
import type { Stage } from "./render/stage";
import type { FromWorker, ToWorker, ViewLayout } from "./protocol";
import { connect, type Transport } from "./transport";
import { decodeView, type View } from "./view";

const params = new URLSearchParams(location.search);
const status = document.querySelector<HTMLElement>("#status")!;
const stats = document.querySelector<HTMLElement>("#stats")!;
const scores = document.querySelector<HTMLElement>("#scores")!;

function setStatus(state: "connecting" | "playing" | "failed", text: string) {
  status.dataset.state = state;
  status.textContent = text;
}

// Each stat is a <dd data-stat="name"> so the numbers can be read back by
// name, by a person in the inspector or by the end-to-end tests.
const cells = new Map<string, HTMLElement>();
function setStat(name: string, value: string | number) {
  let cell = cells.get(name);
  if (!cell) {
    const label = document.createElement("dt");
    label.textContent = name;
    cell = document.createElement("dd");
    cell.dataset.stat = name;
    stats.append(label, cell);
    cells.set(name, cell);
  }
  const text = String(value);
  if (cell.textContent !== text) {
    cell.textContent = text;
  }
}

function showScores(view: View) {
  while (scores.children.length < view.players.length) {
    const row = document.createElement("li");
    row.style.setProperty("--seat", seatColours[scores.children.length] ?? "#fff");
    row.append(document.createElement("span"), document.createElement("span"));
    scores.append(row);
  }
  view.players.forEach((player, seat) => {
    const [name, score] = scores.children[seat]!.children;
    name!.textContent = seat === view.me ? `seat ${seat + 1} (you)` : `seat ${seat + 1}`;
    score!.textContent = `${player.score}  ${player.strain}%`;
  });
}

type ViewChoice = "webgpu" | "webgl" | "flat";

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
      return await createScene(freshCanvas(), { forceWebGL: step === "webgl", onLost });
    } catch (error) {
      console.info(`${step} view unavailable:`, error);
    }
  }
  return createFlatStage(freshCanvas());
}

async function start() {
  const requested = params.get("view");
  const first: ViewChoice = requested === "flat" || requested === "webgl" ? requested : "webgpu";
  let stage: Stage | null = null;
  const useStage = async (choice: ViewChoice) => {
    stage = null;
    // A lost WebGPU device is retried once on WebGL2, and after that the
    // flat view, which needs no GPU.
    stage = await createStage(choice, () => void useStage(choice === "webgpu" ? "webgl" : "flat"));
    setStat("view", stage.kind);
  };
  await useStage(first);
  window.addEventListener("resize", () => stage?.resize());

  const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
  const toWorker = (message: ToWorker, transfer: Transferable[] = []) =>
    worker.postMessage(message, transfer);

  let layout: ViewLayout | null = null;
  let latest: View | null = null;
  let transport: Transport | null = null;
  let desyncs = 0;

  worker.onmessage = async (message: MessageEvent<FromWorker>) => {
    const data = message.data;
    switch (data.type) {
      case "ready":
        layout = data.layout;
        try {
          transport = await connect({
            room: params.get("room"),
            websocketOnly: params.get("transport") === "websocket",
            webrtcTimeoutMs: 1500,
          });
        } catch (error) {
          setStatus("failed", `Could not reach the server: ${String(error)}`);
          return;
        }
        setStat("transport", transport.kind);
        transport.onmessage = (bytes) => toWorker({ type: "packet", bytes }, [bytes.buffer]);
        transport.onclose = () => setStatus("failed", "Disconnected");
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
      case "event":
        if (data.event.kind === "synced") {
          setStatus("playing", "Playing");
        } else if (data.event.kind === "desync") {
          desyncs += 1;
        } else if (data.event.kind === "match-over") {
          setStatus("connecting", "Match over, next one starting");
        }
        break;
      case "error":
        setStatus("failed", data.message);
        break;
    }
  };

  input.listen(window, () => toWorker({ type: "input", ...input.current() }));

  const frame = (now: number) => {
    stage?.frame(now);
    if (latest) {
      showScores(latest);
      setStat("frame", latest.frame);
      setStat("confirmed", latest.confirmedFrame);
      setStat("rtt ms", latest.rttMs);
      setStat("input delay", latest.inputDelay);
      setStat("rollbacks", latest.rollbacks);
      setStat("deepest rollback", latest.deepestRollback);
      setStat("stalls", latest.stalls);
      setStat("late inputs", latest.substitutedInputs);
      setStat("checkpoints", latest.checkpointsVerified);
      setStat("desyncs", desyncs);
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

start();

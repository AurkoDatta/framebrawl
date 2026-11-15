import { draw, seatColours } from "./draw";
import * as input from "./input";
import type { FromWorker, ToWorker, ViewLayout } from "./protocol";
import { connect, type Transport } from "./transport";
import { decodeView, type View } from "./view";

const params = new URLSearchParams(location.search);
const canvas = document.querySelector<HTMLCanvasElement>("#arena")!;
const context = canvas.getContext("2d")!;
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

async function start() {
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

  const frame = () => {
    if (latest) {
      draw(context, latest);
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

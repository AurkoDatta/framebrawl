import { useEffect, useRef, useState, useSyncExternalStore } from "react";

import type { Game } from "../game";
import { LabPanel } from "../lab/LabPanel";
import { seatColours } from "../palette";
import * as sound from "../sound";
import { hud, type HudState, type Netcode } from "./store";

function useHud(): HudState {
  return useSyncExternalStore(hud.subscribe, hud.get);
}

function clock(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

// Strain is how far the next shove will send you. The bar turns to the
// warning colour as it fills.
function Seat({ seat, state }: { seat: number; state: HudState }) {
  const { score, strain, inPlay } = state.seats[seat]!;
  const share = Math.min(1, strain / 200);
  return (
    <li
      className="seat"
      data-out={!inPlay}
      style={{ "--seat": seatColours[seat] } as React.CSSProperties}
    >
      <span className="seat-score">{score}</span>
      <span className="seat-name">{seat === state.me ? "You" : `Bot ${seat + 1}`}</span>
      <span className="seat-strain" aria-label={`strain ${strain}`}>
        <span style={{ width: `${share * 100}%` }} data-high={share > 0.6} />
      </span>
    </li>
  );
}

// One bar per recent rollback, taller for deeper ones. Flat when the
// network is behaving; a comb when it is not.
function RollbackTrace({ depths }: { depths: number[] }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const context = canvas.current?.getContext("2d");
    if (!context) return;
    const { width, height } = context.canvas;
    context.clearRect(0, 0, width, height);
    depths.forEach((depth, i) => {
      const bar = Math.min(height, (depth / 16) * height);
      context.fillStyle = depth > 8 ? "#ff9d2e" : "#8fb9c9";
      context.fillRect(width - (depths.length - i) * 2, height - bar, 1, bar);
    });
  }, [depths]);
  return <canvas ref={canvas} width={180} height={28} className="trace" aria-hidden />;
}

const netRows: [string, keyof Netcode, string][] = [
  ["view", "view", "How the picture is drawn"],
  ["transport", "transport", "How this browser reaches the server"],
  ["frame", "frame", "The frame being predicted now"],
  ["confirmed", "confirmed", "The newest frame the server has settled"],
  ["rtt ms", "rttMs", "Round trip to the server and back"],
  ["input delay", "inputDelay", "Frames your input waits before it applies"],
  ["rollbacks", "rollbacks", "Times the past was corrected"],
  ["deepest rollback", "deepestRollback", "Most frames resimulated at once"],
  ["stalls", "stalls", "Ticks spent waiting for the server"],
  ["late inputs", "lateInputs", "Your inputs the server had to replace"],
  ["checkpoints", "checkpoints", "State checksums that matched the server"],
  ["desyncs", "desyncs", "State checksums that did not"],
];

const transportNames = {
  webrtc: "WebRTC",
  websocket: "WebSocket",
  "in this tab": "In this tab",
  "": "…",
};

function NetcodePanel({ state }: { state: HudState }) {
  const [open, setOpen] = useState(false);
  const net = state.net;
  useEffect(() => {
    const toggle = (event: KeyboardEvent) => {
      if (event.code === "KeyN") setOpen((value) => !value);
    };
    window.addEventListener("keydown", toggle);
    return () => window.removeEventListener("keydown", toggle);
  }, []);

  return (
    <section className="net">
      <button
        type="button"
        className="net-summary"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        title="Show or hide netcode details (N)"
      >
        <span>{transportNames[net.transport]}</span>
        <span>{net.rttMs} ms</span>
        <span>{net.rollbacksPerSecond} rollbacks/s</span>
        <RollbackTrace depths={state.recentRollbacks} />
      </button>
      {/* Always in the document, so the numbers can be read by name
          whether or not the panel is open. */}
      <dl hidden={!open}>
        {netRows.map(([label, key, hint]) => (
          <div key={key} title={hint}>
            <dt>{label}</dt>
            <dd data-stat={label}>{net[key]}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function Notice({ state }: { state: HudState }) {
  const phase = state.phase;
  if (phase.name === "playing") return null;
  if (phase.name === "connecting") {
    return <p className="notice">{state.mode === "lab" ? "Starting a match" : "Joining a match"}</p>;
  }
  if (phase.name === "failed") {
    return (
      <div className="notice" role="alert">
        <p>{phase.reason}</p>
        <button type="button" onClick={() => location.reload()}>
          Reconnect
        </button>
      </div>
    );
  }
  const best = Math.max(...phase.scores);
  const winners = phase.scores.flatMap((score, seat) => (score === best ? [seat] : []));
  const names = winners.map((seat) => (seat === state.me ? "You" : `Bot ${seat + 1}`));
  const verb = winners.length === 1 && names[0] !== "You" ? "wins" : "win";
  return (
    <div className="notice">
      <p className="notice-title" style={{ color: seatColours[winners[0] ?? 0] }}>
        {winners.length > 1 ? `${names.join(" and ")} tie` : `${names[0]} ${verb}`}
      </p>
      <p>Next match starts in a few seconds</p>
    </div>
  );
}

export function Hud({ game }: { game: Game | null }) {
  const state = useHud();
  const lab = state.mode === "lab";
  useEffect(() => {
    const mute = (event: KeyboardEvent) => {
      if (event.code === "KeyM") {
        const muted = !hud.get().muted;
        sound.setMuted(muted);
        hud.set({ muted });
      }
    };
    window.addEventListener("keydown", mute);
    return () => window.removeEventListener("keydown", mute);
  }, []);

  return (
    <div className="hud" data-state={state.phase.name} data-mode={state.mode}>
      <header>
        <ol className="seats">
          {state.seats.map((_, seat) => (
            <Seat key={seat} seat={seat} state={state} />
          ))}
        </ol>
        {state.secondsLeft !== null && state.seats.length > 0 && (
          <p className="clock" title={`First to ${state.scoreLimit} wins`}>
            {clock(state.secondsLeft)}
          </p>
        )}
      </header>
      <Notice state={state} />
      {lab && game && <LabPanel game={game} state={state} />}
      <footer>
        <NetcodePanel state={state} />
        <a className="mode" href={lab ? "./" : "./?lab"}>
          {lab ? "Play online" : "Open the Netcode Lab"}
        </a>
        <p className="keys">
          <kbd>WASD</kbd> run <kbd>Space</kbd> jump <kbd>J</kbd> shove <kbd>K</kbd> dash{" "}
          <kbd>M</kbd> {state.muted ? "unmute" : "mute"}
        </p>
      </footer>
    </div>
  );
}

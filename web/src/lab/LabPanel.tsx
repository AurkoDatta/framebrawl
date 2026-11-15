import { useEffect, useRef } from "react";

import type { Game } from "../game";
import type { Network } from "../protocol";
import { cleanNetwork, hud, type HudState } from "../ui/store";
import { recent, TIMELINE_TICKS } from "./timeline";

const presets: [string, Network][] = [
  ["Perfect", cleanNetwork],
  ["Same city", { roundTripMs: 30, jitterMs: 4, lossPercent: 0.5, duplicatePercent: 0 }],
  ["Across an ocean", { roundTripMs: 150, jitterMs: 15, lossPercent: 2, duplicatePercent: 0 }],
  ["Train Wi-Fi", { roundTripMs: 260, jitterMs: 70, lossPercent: 12, duplicatePercent: 3 }],
];

const delays: [string, number][] = [
  ["Auto", -1],
  ["0", 0],
  ["1", 1],
  ["2", 2],
  ["3", 3],
];

function Slider(props: {
  label: string;
  unit: string;
  max: number;
  step: number;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="slider">
      <span>{props.label}</span>
      <output>
        {props.value}
        {props.unit}
      </output>
      <input
        type="range"
        min={0}
        max={props.max}
        step={props.step}
        value={props.value}
        onChange={(event) => props.onChange(Number(event.target.value))}
      />
    </label>
  );
}

const COLUMN = 2;
const SCALE = 32;

// One column per tick, newest on the right. The pale bar is how many
// frames the client is running on guesses; the amber bar on top of it is
// how many of them it had to redo that tick.
function Timeline() {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const context = canvas.current?.getContext("2d");
    if (!context) return;
    let handle = 0;
    const draw = () => {
      const { width, height } = context.canvas;
      const floor = height - 6;
      const unit = floor / SCALE;
      context.clearRect(0, 0, width, height);

      context.fillStyle = "rgb(243 239 228 / 0.12)";
      for (const frames of [10, 20, 30]) {
        context.fillRect(0, Math.round(floor - frames * unit), width, 1);
      }

      const ticks = recent();
      ticks.forEach((tick, i) => {
        const x = width - (ticks.length - i) * COLUMN;
        const ahead = Math.min(SCALE, tick.ahead) * unit;
        context.fillStyle = "#6d93a3";
        context.fillRect(x, floor - ahead, COLUMN, ahead);
        if (tick.rollback > 0) {
          const redo = Math.min(SCALE, tick.rollback) * unit;
          context.fillStyle = "#ff9d2e";
          context.fillRect(x, floor - redo, COLUMN, redo);
        }
        if (tick.stalled) {
          context.fillStyle = "#ea5f4b";
          context.fillRect(x, 0, COLUMN, floor);
        }
        if (tick.lost > 0) {
          context.fillStyle = "#f3efe4";
          context.fillRect(x, height - 4, COLUMN, 4);
        }
      });
      handle = requestAnimationFrame(draw);
    };
    handle = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(handle);
  }, []);

  return (
    <figure className="timeline">
      <canvas ref={canvas} width={TIMELINE_TICKS * COLUMN} height={110} />
      <figcaption>
        <span className="key key-ahead">frames predicted</span>
        <span className="key key-rollback">frames redone</span>
        <span className="key key-stall">waiting for server</span>
        <span className="key key-lost">packet lost</span>
        <span className="timeline-span">last {TIMELINE_TICKS / 60} seconds</span>
      </figcaption>
    </figure>
  );
}

export function LabPanel({ game, state }: { game: Game; state: HudState }) {
  const { lab, net } = state;
  const set = (patch: Partial<Network>) => game.setNetwork({ ...lab.network, ...patch });

  return (
    <>
      <aside className="lab">
        <h2>Netcode Lab</h2>
        <p>
          The server and your client both run in this tab, with a simulated connection between
          them. Make it worse and keep playing.
        </p>

        <div className="presets">
          {presets.map(([name, network]) => (
            <button
              type="button"
              key={name}
              aria-pressed={JSON.stringify(network) === JSON.stringify(lab.network)}
              onClick={() => game.setNetwork(network)}
            >
              {name}
            </button>
          ))}
        </div>

        <Slider
          label="Round trip"
          unit=" ms"
          max={400}
          step={10}
          value={lab.network.roundTripMs}
          onChange={(roundTripMs) => set({ roundTripMs })}
        />
        <Slider
          label="Jitter"
          unit=" ms"
          max={100}
          step={5}
          value={lab.network.jitterMs}
          onChange={(jitterMs) => set({ jitterMs })}
        />
        <Slider
          label="Packet loss"
          unit="%"
          max={30}
          step={1}
          value={lab.network.lossPercent}
          onChange={(lossPercent) => set({ lossPercent })}
        />
        <Slider
          label="Duplicates"
          unit="%"
          max={20}
          step={1}
          value={lab.network.duplicatePercent}
          onChange={(duplicatePercent) => set({ duplicatePercent })}
        />

        <fieldset>
          <legend>Input delay, frames (restarts the match)</legend>
          {delays.map(([name, frames]) => (
            <button
              type="button"
              key={name}
              aria-pressed={lab.inputDelay === frames}
              onClick={() => game.restart(frames)}
            >
              {name}
            </button>
          ))}
        </fieldset>

        <label className="check">
          <input
            type="checkbox"
            checked={lab.showServer}
            onChange={(event) => hud.setLab({ showServer: event.target.checked })}
          />
          Outline where the server has each player
        </label>

        <dl className="lab-numbers">
          <div>
            <dt>Measured round trip</dt>
            <dd data-stat="lab rtt">{net.rttMs} ms</dd>
          </div>
          <div>
            <dt>Input delay in use</dt>
            <dd data-stat="lab delay">{net.inputDelay}</dd>
          </div>
          <div>
            <dt>Rollbacks</dt>
            <dd>{net.rollbacksPerSecond}/s</dd>
          </div>
          <div>
            <dt>Deepest rollback</dt>
            <dd>{net.deepestRollback} frames</dd>
          </div>
          <div>
            <dt>Inputs that arrived late</dt>
            <dd>{net.lateInputs}</dd>
          </div>
          <div>
            <dt>Packets lost</dt>
            <dd>{lab.lossPercent.toFixed(1)}%</dd>
          </div>
          <div>
            <dt>Traffic up / down</dt>
            <dd>
              {lab.kilobitsUp.toFixed(1)} / {lab.kilobitsDown.toFixed(1)} kbit/s
            </dd>
          </div>
          <div>
            <dt>Checkpoints matched</dt>
            <dd>{net.checkpoints}</dd>
          </div>
        </dl>

        <button type="button" className="corrupt" onClick={() => game.corrupt()}>
          Corrupt my game state
        </button>
        <p className="report" data-stat="lab report" role="status">
          {lab.report
            ? `${lab.report} The server sent the match again and the client rebuilt its state.`
            : "Changes one number in the client's copy of the world. The next checkpoint should catch it."}
        </p>
      </aside>
      <Timeline />
    </>
  );
}

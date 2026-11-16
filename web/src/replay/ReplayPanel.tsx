import type { Game } from "../game";
import type { HudState } from "../ui/store";

const speeds: [string, number][] = [
  ["Pause", 0],
  ["1x", 1],
  ["2x", 2],
  ["4x", 4],
];

export function ReplayPanel({ game, state }: { game: Game; state: HudState }) {
  const { choices, url, bytes, speed, progress } = state.replay;
  const done = progress ? progress.frame / Math.max(1, progress.frames) : 0;

  return (
    <aside className="lab replay">
      <h2>Replay</h2>
      <p>
        A replay is the match settings and every input, nothing else. This browser is running the
        simulation again from those inputs and comparing its state with the checksums the server
        recorded.
      </p>

      <label className="pick">
        <span>Match</span>
        <select value={url} onChange={(event) => game.openReplay(event.target.value)}>
          {choices.map((choice) => (
            <option key={choice.url} value={choice.url}>
              {choice.name}
            </option>
          ))}
        </select>
      </label>

      <div className="presets">
        {speeds.map(([name, value]) => (
          <button
            type="button"
            key={name}
            aria-pressed={speed === value}
            onClick={() => game.setSpeed(value)}
          >
            {name}
          </button>
        ))}
        <button type="button" onClick={() => game.openReplay(url)}>
          Start again
        </button>
      </div>

      <progress max={1} value={done} aria-label="Position in the replay" />

      <dl className="lab-numbers">
        <div>
          <dt>Frame</dt>
          <dd data-stat="replay frame">
            {progress?.frame ?? 0} of {progress?.frames ?? 0}
          </dd>
        </div>
        <div>
          <dt>File size</dt>
          <dd>{bytes.toLocaleString()} bytes</dd>
        </div>
        <div>
          <dt>Checkpoints matched</dt>
          <dd data-stat="replay matched">
            {progress?.matched ?? 0} of {progress?.checkpoints ?? 0}
          </dd>
        </div>
      </dl>
      <p className="report" data-stat="replay verdict" role="status">
        {!progress || progress.divergedAt < 0
          ? "Every checkpoint so far matches the recording."
          : `This browser's state differed from the recording at frame ${progress.divergedAt}. The simulation is not deterministic here.`}
      </p>
    </aside>
  );
}

// Sound effects made on the spot with a few oscillators, so there are no
// audio files to load or license.

import type { Cue } from "./render/motion";

let context: AudioContext | null = null;
let muted = false;

export function setMuted(value: boolean) {
  muted = value;
}

// Browsers only allow audio after the person has interacted with the page.
export function unlock() {
  context ??= new AudioContext();
  if (context.state === "suspended") void context.resume();
}

function tone(from: number, to: number, seconds: number, volume: number, type: OscillatorType) {
  if (!context || muted || context.state !== "running") return;
  const now = context.currentTime;
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  oscillator.type = type;
  oscillator.frequency.setValueAtTime(from, now);
  oscillator.frequency.exponentialRampToValueAtTime(to, now + seconds);
  gain.gain.setValueAtTime(volume, now);
  gain.gain.exponentialRampToValueAtTime(0.0001, now + seconds);
  oscillator.connect(gain).connect(context.destination);
  oscillator.start(now);
  oscillator.stop(now + seconds);
}

export function play(cue: Cue, me: number) {
  const mine = "seat" in cue && cue.seat === me;
  const near = mine ? 1 : 0.5;
  switch (cue.kind) {
    case "shove":
      tone(220, 90, 0.09, 0.12 * near, "square");
      break;
    case "dash":
      tone(500, 900, 0.12, 0.07 * near, "sawtooth");
      break;
    case "hit":
      tone(160, 50, 0.16, 0.22 * near, "triangle");
      break;
    case "ring-out":
      tone(700, 60, 0.6, 0.18, "sine");
      break;
    case "tile-fell":
      tone(90, 40, 0.3, 0.1, "triangle");
      break;
  }
}

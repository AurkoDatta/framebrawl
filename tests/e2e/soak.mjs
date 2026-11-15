// Parameters of the reference match. They must match the defaults of
// crates/fb-sim/examples/soak.rs, which produces the native stream.
export const SEED = 0x5eedn;
export const PLAYERS = 4;
export const FRAMES = 10_000;
export const REWIND_DEPTH = 8;

export function toHexLines(checksums) {
  return Array.from(checksums, (c) => c.toString(16).padStart(16, "0") + "\n").join("");
}

/** Index of the first line that differs, or -1. */
export function firstDifference(a, b) {
  const [la, lb] = [a.split("\n"), b.split("\n")];
  for (let i = 0; i < Math.max(la.length, lb.length); i++) {
    if (la[i] !== lb[i]) return i;
  }
  return -1;
}

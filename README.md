# framebrawl

A 3D physics brawler built to show its netcode: a deterministic Rust
simulation that runs natively on the server and as WebAssembly in the
browser, with server-authoritative rollback on top. Two to four players
shove each other off a floating arena whose tiles crack and fall away.

Work in progress. What exists today:

- `crates/fb-sim`: the simulation core. Fixed 60 Hz tick, rapier3d physics
  built for cross-platform determinism, two-byte inputs, a seeded PCG32, and
  a canonical state checksum.

## Determinism

Rollback and replays need every machine to compute the same state from the
same inputs, to the bit. The simulation avoids everything that breaks that
(wall clock, OS randomness, unordered iteration, platform `sin`/`cos`,
SIMD, threads), and the tests do not take it on trust: seeded matches are
replayed with random rewinds and compared checksum by checksum.

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The toolchain is pinned in `rust-toolchain.toml`.

## Licence

MIT

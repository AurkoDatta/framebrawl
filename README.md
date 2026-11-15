# framebrawl

A 3D physics brawler built to show its netcode: a deterministic Rust
simulation that runs natively on the server and as WebAssembly in the
browser, with server-authoritative rollback on top. Two to four players
shove each other off a floating arena whose tiles crack and fall away.

## Run it

With Docker installed and nothing else:

```
docker compose up --build
```

Then open http://localhost:8080. You are dropped into a match with three
bots. WASD or the arrow keys run, Space jumps, J shoves, K dashes. Each
hit you take raises your strain, the bar under your score, and the next
shove sends you further. N opens the netcode numbers, M mutes.

To play against yourself or a friend on the same machine, open the same
room in two windows: http://localhost:8080/?room=anything. A player who
joins takes over a bot's seat mid-match; when they leave, the bot comes
back.

The first build compiles the server and the WebAssembly module and takes a
few minutes. Finished matches are saved as replays in a Docker volume and
listed at http://localhost:8080/api/replays.

Chromium and Safari connect over WebRTC. Firefox refuses WebRTC to
127.0.0.1 and falls back to a WebSocket after 1.5 seconds; to give it
WebRTC too, or to play from another device on your network, tell the
server which addresses to offer:

```
FB_PUBLIC_IP=127.0.0.1,192.168.1.20 docker compose up --build
```

Adding `?transport=websocket` to the URL forces the fallback in any
browser. The picture is drawn with WebGPU where the browser has it and
WebGL2 where it does not; `?view=webgl` and `?view=flat` force the simpler
ones.

## Status

Work in progress. The netcode, the server and the 3D view are in place
and tested. The Netcode Lab, where you degrade the network and watch
rollback cope, is next.

- `crates/fb-sim`: the simulation. Fixed 60 Hz tick, rapier3d physics
  built for cross-platform determinism, two-byte inputs, a seeded PCG32, and
  a canonical state checksum.
- `crates/fb-rollback`: the netcode core, with no I/O and no clocks. A
  server-side authority that owns the input stream, and a client session
  that predicts, rolls back and resimulates, with frame-based clock sync,
  adaptive input delay and desync detection with bisect. See
  [docs/netcode.md](docs/netcode.md).
- `crates/fb-net`: the bit-packed wire format and a simulated bad link.
- `crates/fb-host` and `crates/fb-client`: the two ends of the protocol
  as plain state machines: a room that runs matches, and the client that
  joins one.
- `crates/fb-bot`: the bots. They produce inputs like any other player.
- `crates/fb-replay`: replays as seed, config and inputs, verified by
  running them.
- `crates/fb-server`: the binary. HTTP, WebRTC on one UDP port, WebSocket
  fallback, rooms, replay storage.
- `crates/fb-wasm`, `web/`: the browser side. The simulation runs in a
  worker; the page draws it with three.js, interpolating between ticks
  and fading rollback corrections in over a few frames.
- `tests/e2e`: the cross-target determinism harness and browser tests
  against the real server.

## Determinism

Rollback and replays need every machine to compute the same state from the
same inputs, to the bit. The simulation avoids everything that breaks that:

- no wall clock or OS randomness; time is the frame number and randomness
  is a PCG32 whose state is part of the world
- rapier3d with `enhanced-determinism`, without SIMD or threads
- no `sin`/`cos` from the platform; the 256 aim headings come from a
  polynomial that uses only multiplication and addition
- inputs are integers, and the state checksum is FNV-1a 64 over a fixed
  field order, not `std::hash`

None of that is taken on trust. A seeded four-player match with scripted
inputs runs for 10,000 frames and its per-frame checksums are compared
across:

| Target | Where it runs |
|---|---|
| native x86_64 Linux | CI |
| native arm64 Linux | CI |
| native arm64 macOS | development machine |
| wasm32 in Node | CI, both architectures |
| wasm32 in Chromium, Firefox, WebKit | CI, both architectures |

The same match is also replayed with a rewind of up to 8 frames roughly
every fourth frame, restoring a snapshot and resimulating, and must still
produce the identical stream. That is the property rollback depends on:
a restored world has to step exactly like the original, which only holds
because the snapshot includes rapier's contact caches, broad phase and
sleep state along with the bodies.

## Netcode

Clients send two-byte inputs; the server decides which frame each lands on
and broadcasts the result. Clients simulate ahead on predictions and
resimulate when the server's stream disagrees. Every packet carries all
unconfirmed history, so loss never needs a retransmit.

Four players on the real simulation for five minutes over a simulated
link with a 150 ms round trip and 5% loss:

| | per client |
|---|---|
| desyncs | 0 |
| checkpoints verified | 599 of 599 |
| stalls | 0 |
| rollbacks | about 4,650 (15 per second) |
| rollback depth | mean 8.7 frames, max 16 |
| inputs substituted by the server | 0.4% |
| input delay chosen | 3 frames |

The same sessions are fuzzed over 300 random link configurations with up
to 45% loss, heavy jitter and duplication, and must never desync.

On the wire, a client's per-tick message is 16 bytes and the server's is
22 with twelve frames of history in flight, about 1 KB/s up and 1.3 KB/s
down before transport overhead. Physics bodies cost nothing: only inputs
travel. A complete match through the real server from a browser (2,144
frames, WebRTC on loopback) ended with 71 of 71 checkpoints verified and
a replay file of 3,311 bytes.

## Rollback cost

One rollback is a restore followed by 8 x (snapshot + step). Measured on
an Apple M4 Pro, default arena (81 tiles, 4 players, 10 props), 400 frames
into a match:

| | rollback of 8 frames | snapshot | step |
|---|---|---|---|
| native arm64 | 0.18 ms | 0.013 ms | 0.006 ms |
| wasm, Chromium 159 | 0.22 ms | 0.013 ms | 0.010 ms |
| wasm, Firefox 159 | 0.25 ms | 0.014 ms | 0.012 ms |
| wasm, WebKit 27.2 | 0.22 ms | 0.012 ms | 0.010 ms |

The budget is 4 ms, a quarter of a frame. An earlier stress scene with 60
loose bodies and 144 tiles cost about 1 ms in the browser.

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run --release -p fb-sim --example rollback_cost
```

To work on the page with hot reload, run the server and Vite side by
side. This needs Node and `wasm-bindgen-cli` 0.2.130:

```
cargo run --release -p fb-server
cd web && npm ci && npm run wasm && npm run dev
```

The determinism harness and the browser tests:

```
cd tests/e2e
npm ci && npx playwright install
npm run build    # wasm, native checksum stream, server, page
npm test         # Node, then Chromium, Firefox and WebKit
```

Or run everything in a container, with nothing but Docker installed:

```
docker compose --profile test run --rm --build test
```

The toolchain is pinned in `rust-toolchain.toml`.

## Licence

MIT

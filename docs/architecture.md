# Architecture

One simulation, written once in Rust, runs in three places: the server,
the browser, and the tests. Everything else is arranged so that the same
code is exercised in all three.

## Crates

```
fb-sim        the game: world, rules, physics, checksum      (no I/O, no clock)
  ^
fb-rollback   authority (server side) and session (client side) over any
  ^           simulation; pacing, input delay, bisect         (no I/O, no clock)
  |
fb-net        bit-packed messages; a simulated bad link
fb-bot        bots: world in, input out
fb-replay     config + inputs + checksums; replay by resimulation
  ^
fb-host       a room: authority + bots + recorder; join, leave, match after match
fb-client     the client: bytes in, bytes out, predicted world
  ^
fb-server     tokio, axum, str0m: sockets, HTTP, files
fb-wasm       wasm-bindgen: the client, the Lab, the replay player
```

Everything above the last two lines is a library of plain state machines.
Nothing in them opens a socket, reads a clock or spawns a thread; callers
pass bytes and ticks in and take bytes out. `fb-server` and `fb-wasm` are
thin shells that supply the I/O.

That split is what lets the same `Room` and `Client` run:

- on the server, driven by tokio tasks and real sockets;
- in the test suite, joined by a seeded simulated link, where five minutes
  of four-player play at 150 ms and 5% loss runs in a few seconds;
- in the browser's Netcode Lab, joined by that same simulated link with
  sliders on it;
- in the static build, where the "server" is just a `Room` in the tab.

## Online play

```
 browser                                            server
 -------                                            ------
 page (main thread)                                 HTTP task (axum)
   keyboard --> input                                 /api/rtc  SDP offer -> answer
   three.js scene  <-- view (Float32Array)            /api/ws   WebSocket fallback
   React interface <-- numbers, 10 Hz                 /api/replays
   transport: WebRTC or WebSocket <================>  static files
        |  ^                                        UDP thread (str0m)
  bytes |  | bytes                                    every WebRTC session, one socket
        v  |                                               |
 worker                                             room task, one per room, 60 Hz
   fb-wasm NetClient                                  fb-host Room
     fb-client -> fb-rollback Session -> fb-sim         Authority -> fb-sim, bots, recorder
```

The connection lives on the page's main thread because browsers do not
offer WebRTC in workers. The simulation lives in a worker so that
rollback never competes with rendering, and keeps ticking when the tab is
in the background.

Each room is one task that owns its state outright; other tasks reach it
only by sending commands down a channel. There are no locks around game
state.

## What crosses each boundary

| Boundary | What crosses | Form |
|---|---|---|
| client to server, every tick | unconfirmed inputs | 16 bytes, unreliable channel |
| server to client, every tick | confirmed inputs, a checksum now and then | 22 bytes, unreliable channel |
| server to client, on join or resync | the match so far, as inputs | a few KB, reliable channel |
| worker to page, every tick | the predicted world | one transferred `Float32Array` |
| disk | a finished match | config + inputs + checksums, about 6 KB a minute |

World state never crosses a boundary in serialized form. A world is only
ever obtained by running inputs through the simulation.

## Where to look

| To understand | Read |
|---|---|
| the rules of the game | `crates/fb-sim/src/world.rs` |
| what keeps it deterministic | `crates/fb-sim/src/heading.rs`, `canon.rs`, `tests/determinism.rs` |
| prediction and rollback | `crates/fb-rollback/src/session.rs` |
| the server's final say on inputs | `crates/fb-rollback/src/authority.rs` |
| staying the right distance ahead | `crates/fb-rollback/src/pacing.rs` |
| the wire format | `crates/fb-net/src/wire.rs` |
| the Lab | `crates/fb-wasm/src/lab.rs`, `web/src/lab/` |
| smoothing rollback corrections on screen | `web/src/render/motion.ts` |

The netcode itself is described in [netcode.md](netcode.md).

# Netcode design

framebrawl uses rollback with an authoritative server. This note covers
what that means here, what each side does every tick, and the trade-offs.
The code is in `crates/fb-rollback`.

## The model in one paragraph

The simulation is deterministic, so a match is fully described by its
config and the list of inputs for every frame. The server owns that list.
Clients send their inputs, the server decides which frame each one lands
on, simulates, and broadcasts the inputs it used. A client does not wait:
it simulates ahead using its own inputs and a guess for everyone else, and
when the server's list disagrees with what it guessed, it restores an
earlier state and simulates forward again with the right inputs.

Only inputs cross the network: two bytes per player per frame. Physics
objects cost nothing on the wire.

## Frames, not milliseconds

Nothing in `fb-rollback` reads a clock. Time is the frame number, and
packets are plain structs. The same code runs on the server, in the
browser, and in tests against a simulated network, and a test run is
reproducible from its seed.

Frame `F`'s inputs take the state at `F` to the state at `F + 1`.

## Server: `Authority`

Each tick, for frame `F`:

1. For every player, take the input they sent for `F`. If none has
   arrived, repeat their previous input and count a substitution. This is
   final; a late packet for `F` is ignored.
2. Save the state, step the simulation, append the inputs to history.
3. Every `K` frames, record a checksum of the state (a checkpoint).

Each packet to a client carries every input row the client has not
acknowledged (up to 32 frames), the latest checkpoint, and the client's
`lead`: how many frames early its newest input was at this tick.

## Client: `Session`

`advance(local_input)` simulates one predicted frame:

- The local input is scheduled `delay` frames ahead and kept until the
  server confirms that frame.
- Inputs for the frame being simulated are the best knowledge available:
  the server's row if it has arrived, otherwise the local scheduled input
  plus each remote player's last confirmed input, repeated.
- The state before the step is saved into a ring indexed by frame.

`receive(packet)` applies server rows:

1. Append new rows (a packet that would leave a gap is ignored; the
   server resends from the acknowledgement).
2. Find the first simulated frame whose inputs differ from the best
   knowledge now. That includes frames that are still predictions: if a
   remote player's last confirmed input changed, every later guess based
   on the old one is stale.
3. If there is one, restore the ring's state for that frame and simulate
   forward to the present again. That is a rollback, and its depth is the
   number of frames simulated again.
4. If the server is ahead of the client (after a stall), simulate the
   confirmed frames to catch up.
5. Compare the state at the server's checkpoint frame with its checksum.

Every packet in both directions is redundant: clients resend all
unconfirmed inputs, the server resends all unacknowledged rows. Loss costs
latency of confirmation, never correctness, and nothing is retransmitted
by a transport layer.

## Staying the right distance ahead

A client's input for frame `F` has to reach the server before the server
simulates `F`, so the client must run ahead of the server by its one-way
latency plus a margin. Too little margin and inputs get substituted; too
much and every rollback is longer than it needs to be.

The server's `lead` report closes that loop. The client keeps the last 40
reports and steers the worst of them towards one frame by nudging its tick
rate by up to 12%. If it is more than 10 frames out (after a stall or a
resync) it runs the missing frames at once instead.

## Input delay

With zero delay, a client predicts about one round trip ahead. Delaying
local input by `d` frames lets it run `d` frames less far ahead, so
rollbacks are `d` frames shorter, at the cost of `d` frames of latency on
its own controls. The delay is chosen from measured prediction depth:
none up to 5 frames, then one step per 3 frames of depth, to a maximum of
3. It moves one step at a time, and only after the measurement has held
for 90 frames, so a spike does not change how the controls feel.

Inputs already sent are never rewritten when the delay changes: growing
the delay repeats the current sample into the gap, shrinking it drops one
sample.

## Desync detection and bisect

Determinism is tested hard, but a desync must still be caught if one
happens. Clients compare checksums at every checkpoint. On a mismatch:

1. Both sides still hold the states since the previous checkpoint. They
   exchange per-frame checksums, and the first frame that differs is the
   frame where the divergence began.
2. Both dump that frame's canonical field list (the same traversal that
   produces the checksum). The first differing entry names the field, for
   example `player.strain[2]`.
3. The server sends a full snapshot and the client resumes from it.

## Limits

- **Prediction limit.** A client stops simulating when it is 30 frames
  past the last confirmed frame, and waits. It does not guess further.
- **History.** The server keeps 256 frames of inputs. A client that falls
  further behind gets a snapshot instead.
- **Snapshots are large.** A full physics snapshot is around 100 to 250 KB
  depending on the scene, which is fine for the client's ring but means
  resync and late join need a reliable, compressed path.

## What the tests show

Clients and the authority are run over simulated links with delay,
jitter, loss and duplication, all seeded:

- 300 random configurations (up to 45% loss, 12 frames of delay, 10 of
  jitter, 20% duplication, 2 to 4 players): no desync, and every client's
  confirmed states equal the server's after the link clears.
- The real simulation, four players, five minutes at 150 ms round trip
  and 5% loss: no desyncs, no stalls, 599 of 599 checkpoints verified per
  client, about 4,650 rollbacks per client with a mean depth of 8.7 frames
  and a maximum of 16, input delay settled at 3, and 0.4% of inputs
  substituted by the server.
- A link blackout longer than the server's input history: the client
  stalls, receives a snapshot, and catches up.
- A deliberately corrupted client: caught at the next checkpoint, bisected
  to the exact frame and field, resynced.

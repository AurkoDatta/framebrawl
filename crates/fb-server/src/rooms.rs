//! Room registry and the task that runs each room in real time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fb_host::{Outgoing, Room, RoomOptions};
use fb_net::ClientMessage;
use tokio::sync::{mpsc, oneshot};
use tokio::time::MissedTickBehavior;

use crate::replays::ReplayStore;

/// Messages queued for one peer before the transport is considered stuck.
const PEER_QUEUE: usize = 512;
const TICK: Duration = Duration::from_nanos(1_000_000_000 / fb_sim::TICK_RATE as u64);

/// Bytes for a peer, and whether they need the reliable channel.
pub type PeerMessage = (bool, Vec<u8>);

pub enum RoomCommand {
    Join {
        peer: mpsc::Sender<PeerMessage>,
        reply: oneshot::Sender<Option<usize>>,
    },
    Message {
        player: usize,
        bytes: Vec<u8>,
    },
    Leave {
        player: usize,
    },
}

#[derive(Clone)]
pub struct RoomHandle {
    pub commands: mpsc::UnboundedSender<RoomCommand>,
    humans: Arc<AtomicUsize>,
    players: usize,
}

/// A seat in a room, as seen by a transport.
pub struct Seat {
    pub player: usize,
    pub commands: mpsc::UnboundedSender<RoomCommand>,
    pub incoming: mpsc::Receiver<PeerMessage>,
}

#[derive(Clone)]
pub struct Rooms {
    inner: Arc<Inner>,
}

struct Inner {
    rooms: Mutex<HashMap<String, RoomHandle>>,
    options: RoomOptions,
    replays: ReplayStore,
    runtime: tokio::runtime::Handle,
    created: AtomicU64,
}

impl Rooms {
    pub fn new(options: RoomOptions, replays: ReplayStore) -> Self {
        Rooms {
            inner: Arc::new(Inner {
                rooms: Mutex::new(HashMap::new()),
                options,
                replays,
                runtime: tokio::runtime::Handle::current(),
                created: AtomicU64::new(0),
            }),
        }
    }

    pub fn count(&self) -> usize {
        self.inner.rooms.lock().expect("rooms lock").len()
    }

    /// Finds the room for `code`, or any room with a free seat when no
    /// code is given, creating one if needed. Safe to call from any thread.
    pub fn find_or_create(&self, code: Option<&str>) -> RoomHandle {
        let mut rooms = self.inner.rooms.lock().expect("rooms lock");
        let existing = match code {
            Some(code) => rooms.get(code),
            None => rooms
                .values()
                .find(|room| room.humans.load(Ordering::Relaxed) < room.players),
        };
        if let Some(room) = existing {
            return room.clone();
        }

        let serial = self.inner.created.fetch_add(1, Ordering::Relaxed);
        let code = code.map_or_else(|| format!("room-{serial}"), str::to_owned);
        let (commands, receiver) = mpsc::unbounded_channel();
        let handle = RoomHandle {
            commands,
            humans: Arc::new(AtomicUsize::new(0)),
            players: self.inner.options.config.players as usize,
        };
        rooms.insert(code.clone(), handle.clone());
        // The clock only seeds the match; nothing in the simulation reads it.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let seed = nanos ^ serial.rotate_left(32);
        self.inner.runtime.spawn(run_room(
            self.clone(),
            code,
            seed,
            receiver,
            handle.humans.clone(),
        ));
        handle
    }

    /// Takes a seat. `None` if the room is full or has shut down.
    pub async fn join(&self, code: Option<&str>) -> Option<Seat> {
        let pending = self.request_seat(code);
        pending.resolve().await
    }

    /// The non-blocking half of [`join`](Self::join), for callers that
    /// cannot await.
    pub fn request_seat(&self, code: Option<&str>) -> PendingSeat {
        let room = self.find_or_create(code);
        let (peer, incoming) = mpsc::channel(PEER_QUEUE);
        let (reply, answer) = oneshot::channel();
        let _ = room.commands.send(RoomCommand::Join { peer, reply });
        PendingSeat {
            answer,
            commands: room.commands,
            incoming: Some(incoming),
        }
    }
}

pub struct PendingSeat {
    answer: oneshot::Receiver<Option<usize>>,
    commands: mpsc::UnboundedSender<RoomCommand>,
    incoming: Option<mpsc::Receiver<PeerMessage>>,
}

pub enum SeatPoll {
    Waiting,
    Seated(Seat),
    Refused,
}

impl PendingSeat {
    fn seat(&mut self, player: usize) -> Seat {
        Seat {
            player,
            commands: self.commands.clone(),
            incoming: self.incoming.take().expect("resolved once"),
        }
    }

    pub async fn resolve(mut self) -> Option<Seat> {
        let player = (&mut self.answer).await.ok()??;
        Some(self.seat(player))
    }

    pub fn poll(&mut self) -> SeatPoll {
        match self.answer.try_recv() {
            Ok(Some(player)) => SeatPoll::Seated(self.seat(player)),
            Ok(None) | Err(oneshot::error::TryRecvError::Closed) => SeatPoll::Refused,
            Err(oneshot::error::TryRecvError::Empty) => SeatPoll::Waiting,
        }
    }
}

async fn run_room(
    rooms: Rooms,
    code: String,
    seed: u64,
    mut commands: mpsc::UnboundedReceiver<RoomCommand>,
    humans: Arc<AtomicUsize>,
) {
    let mut room = Room::new(seed, rooms.inner.options);
    let mut peers: Vec<Option<mpsc::Sender<PeerMessage>>> = vec![None; room.players()];
    let mut out: Vec<Outgoing> = Vec::new();
    let mut ticker = tokio::time::interval(TICK);
    // After a stall, run the missed ticks so the room keeps real time.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Burst);
    let mut seated_once = false;
    tracing::info!(room = %code, "room opened");

    loop {
        tokio::select! {
            biased;
            command = commands.recv() => match command {
                Some(RoomCommand::Join { peer, reply }) => {
                    let player = room.join(&mut out);
                    if let Some(player) = player {
                        peers[player] = Some(peer);
                        seated_once = true;
                        humans.store(room.humans(), Ordering::Relaxed);
                        tracing::info!(room = %code, player, "player joined");
                    }
                    let _ = reply.send(player);
                }
                Some(RoomCommand::Message { player, bytes }) => {
                    if peers[player].is_none() {
                        continue;
                    }
                    match ClientMessage::decode(&bytes) {
                        Ok(message) => room.receive(player, message, &mut out),
                        Err(error) => {
                            tracing::warn!(room = %code, player, %error, "dropping peer");
                            unseat(&mut room, &mut peers, player, &humans);
                        }
                    }
                }
                Some(RoomCommand::Leave { player }) => {
                    if peers[player].is_some() {
                        unseat(&mut room, &mut peers, player, &humans);
                        tracing::info!(room = %code, player, "player left");
                    }
                }
                None => break,
            },
            _ = ticker.tick() => {
                if let Some(finished) = room.tick(&mut out) {
                    tracing::info!(room = %code, scores = ?finished.scores, "match over");
                    rooms.inner.replays.save(finished);
                }
            }
        }

        let players = room.players();
        for message in out.drain(..) {
            let Some(peer) = &peers[message.player] else {
                continue;
            };
            let bytes = message.message.encode(players);
            match peer.try_send((message.reliable, bytes)) {
                Ok(()) => {}
                // Per-tick data is replaced by the next tick's anyway.
                Err(mpsc::error::TrySendError::Full(_)) if !message.reliable => {}
                // A reliable message that cannot be queued means the
                // transport is gone or hopelessly behind.
                Err(_) => {
                    tracing::warn!(room = %code, player = message.player, "peer queue stuck");
                    unseat(&mut room, &mut peers, message.player, &humans);
                }
            }
        }

        if seated_once && room.humans() == 0 {
            // Remove the room under the lock, then make sure nobody was
            // handed it in the meantime.
            let mut registry = rooms.inner.rooms.lock().expect("rooms lock");
            if commands.is_empty() {
                registry.remove(&code);
                break;
            }
        }
    }
    tracing::info!(room = %code, "room closed");
}

fn unseat(
    room: &mut Room,
    peers: &mut [Option<mpsc::Sender<PeerMessage>>],
    player: usize,
    humans: &AtomicUsize,
) {
    peers[player] = None;
    room.leave(player);
    humans.store(room.humans(), Ordering::Relaxed);
}

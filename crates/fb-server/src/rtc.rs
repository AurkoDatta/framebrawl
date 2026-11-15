//! WebRTC data channels, all sessions on one UDP socket.
//!
//! str0m does no I/O of its own, so one thread owns the socket and drives
//! every session: it feeds each datagram to the session that claims it and
//! sends whatever the sessions produce. A browser opens two channels, an
//! unordered one with no retransmits for per-tick traffic and an ordered
//! reliable one for the rest.

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer};
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig, RtcError};

use crate::rooms::{PendingSeat, RoomCommand, Rooms, Seat, SeatPoll};

pub const FRAMES_LABEL: &str = "frames";
pub const CONTROL_LABEL: &str = "control";

/// Longest the loop sleeps in the socket, which bounds how long an
/// outgoing message waits to be picked up.
const MAX_WAIT: Duration = Duration::from_millis(2);

struct NewSession {
    rtc: Rtc,
    room: Option<String>,
}

#[derive(Clone)]
pub struct RtcEndpoint {
    candidates: Vec<SocketAddr>,
    sessions: Sender<NewSession>,
}

impl RtcEndpoint {
    /// Binds the UDP socket and starts the thread that serves it.
    /// `candidates` are the addresses browsers will be told to send to.
    pub fn start(
        bind: SocketAddr,
        candidates: Vec<SocketAddr>,
        rooms: Rooms,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        let (sessions, receiver) = channel();
        let advertised = candidates.clone();
        std::thread::Builder::new()
            .name("rtc".into())
            .spawn(move || serve(socket, receiver, advertised, rooms))?;
        Ok(RtcEndpoint {
            candidates,
            sessions,
        })
    }

    /// Answers a browser's offer and hands the session to the UDP thread.
    pub fn accept(&self, offer: SdpOffer, room: Option<String>) -> Result<SdpAnswer, RtcError> {
        // ICE lite: the server has a known address and only answers checks.
        let mut rtc = RtcConfig::new().set_ice_lite(true).build(Instant::now());
        for &address in &self.candidates {
            if let Ok(candidate) = Candidate::host(address, "udp") {
                rtc.add_local_candidate(candidate);
            }
        }
        let answer = rtc.sdp_api().accept_offer(offer)?;
        // If the thread has died there is nobody to serve the session; the
        // browser's connection attempt will time out and fall back.
        let _ = self.sessions.send(NewSession { rtc, room });
        Ok(answer)
    }
}

enum Stage {
    /// Waiting for the browser to open both channels.
    Connecting,
    Joining(PendingSeat),
    Seated(Seat),
}

struct Session {
    rtc: Rtc,
    room: Option<String>,
    frames: Option<ChannelId>,
    control: Option<ChannelId>,
    stage: Stage,
    /// Reliable messages the channel had no room for yet.
    backlog: Vec<Vec<u8>>,
}

impl Session {
    fn leave(&mut self) {
        if let Stage::Seated(seat) = &self.stage {
            let _ = seat.commands.send(RoomCommand::Leave {
                player: seat.player,
            });
        }
        self.stage = Stage::Connecting;
        self.rtc.disconnect();
    }

    fn handle_event(&mut self, event: Event, rooms: &Rooms) {
        match event {
            Event::ChannelOpen(id, label) => {
                match label.as_str() {
                    FRAMES_LABEL => self.frames = Some(id),
                    CONTROL_LABEL => self.control = Some(id),
                    _ => {}
                }
                if self.frames.is_some()
                    && self.control.is_some()
                    && matches!(self.stage, Stage::Connecting)
                {
                    self.stage = Stage::Joining(rooms.request_seat(self.room.as_deref()));
                }
            }
            Event::ChannelData(data) => {
                if let Stage::Seated(seat) = &self.stage {
                    let _ = seat.commands.send(RoomCommand::Message {
                        player: seat.player,
                        bytes: data.data,
                    });
                }
            }
            Event::ChannelClose(_)
            | Event::IceConnectionStateChange(IceConnectionState::Disconnected) => self.leave(),
            _ => {}
        }
    }

    /// Moves messages from the room onto the data channels.
    fn pump_outgoing(&mut self) {
        if let Stage::Joining(pending) = &mut self.stage {
            match pending.poll() {
                SeatPoll::Waiting => {}
                SeatPoll::Seated(seat) => self.stage = Stage::Seated(seat),
                SeatPoll::Refused => {
                    self.rtc.disconnect();
                    return;
                }
            }
        }
        let (Stage::Seated(seat), Some(frames), Some(control)) =
            (&mut self.stage, self.frames, self.control)
        else {
            return;
        };

        // Reliable messages go out strictly in order: stop at the first
        // one the channel has no room for.
        let mut failed = false;
        let mut flushed = 0;
        for bytes in &self.backlog {
            match write(&mut self.rtc, control, bytes) {
                Some(true) => flushed += 1,
                Some(false) => break,
                None => {
                    failed = true;
                    break;
                }
            }
        }
        self.backlog.drain(..flushed);
        while !failed {
            match seat.incoming.try_recv() {
                Ok((true, bytes)) => {
                    if !self.backlog.is_empty() {
                        self.backlog.push(bytes);
                    } else {
                        match write(&mut self.rtc, control, &bytes) {
                            Some(true) => {}
                            Some(false) => self.backlog.push(bytes),
                            None => failed = true,
                        }
                    }
                }
                // No room in the buffer: this tick's data is simply lost,
                // which the protocol is built for.
                Ok((false, bytes)) => failed = write(&mut self.rtc, frames, &bytes).is_none(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                // The room dropped us (or closed).
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => failed = true,
            }
        }
        if failed {
            self.leave();
        }
    }
}

/// `Some(accepted)`, or `None` if the channel is gone.
fn write(rtc: &mut Rtc, channel: ChannelId, bytes: &[u8]) -> Option<bool> {
    rtc.channel(channel)?.write(true, bytes).ok()
}

fn serve(
    socket: UdpSocket,
    new_sessions: Receiver<NewSession>,
    candidates: Vec<SocketAddr>,
    rooms: Rooms,
) {
    let mut sessions: Vec<Session> = Vec::new();
    let mut buffer = vec![0u8; 2000];

    loop {
        sessions.extend(new_sessions.try_iter().map(|new| Session {
            rtc: new.rtc,
            room: new.room,
            frames: None,
            control: None,
            stage: Stage::Connecting,
            backlog: Vec::new(),
        }));

        let mut deadline = Instant::now() + MAX_WAIT;
        for session in &mut sessions {
            session.pump_outgoing();
            loop {
                match session.rtc.poll_output() {
                    Ok(Output::Timeout(at)) => {
                        deadline = deadline.min(at);
                        break;
                    }
                    Ok(Output::Transmit(packet)) => {
                        let _ = socket.send_to(&packet.contents, packet.destination);
                    }
                    Ok(Output::Event(event)) => session.handle_event(event, &rooms),
                    Err(error) => {
                        tracing::debug!(%error, "rtc session failed");
                        session.leave();
                        break;
                    }
                }
            }
        }
        sessions.retain_mut(|session| {
            let alive = session.rtc.is_alive();
            if !alive {
                session.leave();
            }
            alive
        });

        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_micros(100));
        if socket.set_read_timeout(Some(wait)).is_err() {
            return;
        }
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => route(&mut sessions, &candidates, source, &buffer[..length]),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => {
                tracing::error!(%error, "udp socket failed; WebRTC is unavailable");
                return;
            }
        }

        let now = Instant::now();
        for session in &mut sessions {
            if session.rtc.handle_input(Input::Timeout(now)).is_err() {
                session.leave();
            }
        }
    }
}

/// Gives a datagram to the session that claims it.
///
/// The socket is bound to a wildcard address, usually behind published
/// container ports, so the address the browser aimed at is not the
/// socket's own. It is one of the advertised candidates; try each.
fn route(sessions: &mut [Session], candidates: &[SocketAddr], source: SocketAddr, data: &[u8]) {
    for &destination in candidates {
        let Ok(contents) = data.try_into() else {
            return;
        };
        let input = Input::Receive(
            Instant::now(),
            Receive {
                proto: Protocol::Udp,
                source,
                destination,
                contents,
            },
        );
        if let Some(session) = sessions.iter_mut().find(|s| s.rtc.accepts(&input)) {
            if session.rtc.handle_input(input).is_err() {
                session.leave();
            }
            return;
        }
    }
}

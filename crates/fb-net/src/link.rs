//! An in-memory link that misbehaves on purpose.
//!
//! Tests and the Netcode Lab put this between a client and the server so
//! the real protocol code runs against latency, jitter, loss, duplication
//! and reordering, all reproducible from a seed.

use fb_sim::Pcg32;

/// How a link misbehaves. Times are in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Impairment {
    /// One-way delay added to every packet.
    pub latency_ms: u32,
    /// Extra delay, uniform in `0..=jitter_ms`, drawn per packet. Packets
    /// drawn shorter than their predecessors overtake them.
    pub jitter_ms: u32,
    /// Chance in thousandths that a packet is dropped.
    pub loss_permille: u32,
    /// Chance in thousandths that a packet is delivered twice.
    pub duplicate_permille: u32,
}

struct InFlight<T> {
    due_us: u64,
    /// Send order, to break ties so equal delays keep their order.
    sequence: u64,
    packet: T,
}

/// One direction of a link. The caller owns the clock: it stamps sends
/// with the current time and asks for whatever is due.
pub struct ImpairedLink<T> {
    impairment: Impairment,
    rng: Pcg32,
    in_flight: Vec<InFlight<T>>,
    sequence: u64,
    sent: u64,
    dropped: u64,
}

impl<T: Clone> ImpairedLink<T> {
    pub fn new(impairment: Impairment, seed: u64) -> Self {
        ImpairedLink {
            impairment,
            rng: Pcg32::new(seed),
            in_flight: Vec::new(),
            sequence: 0,
            sent: 0,
            dropped: 0,
        }
    }

    /// Changes take effect for packets sent from now on.
    pub fn set_impairment(&mut self, impairment: Impairment) {
        self.impairment = impairment;
    }

    pub fn impairment(&self) -> Impairment {
        self.impairment
    }

    pub fn send(&mut self, now_us: u64, packet: T) {
        self.sent += 1;
        if self.rng.below(1000) < self.impairment.loss_permille {
            self.dropped += 1;
            return;
        }
        let copies = 1 + (self.rng.below(1000) < self.impairment.duplicate_permille) as usize;
        for _ in 0..copies {
            let jitter_us = match self.impairment.jitter_ms {
                0 => 0,
                jitter => self.rng.below(jitter * 1000 + 1) as u64,
            };
            self.in_flight.push(InFlight {
                due_us: now_us + self.impairment.latency_ms as u64 * 1000 + jitter_us,
                sequence: self.sequence,
                packet: packet.clone(),
            });
            self.sequence += 1;
        }
    }

    /// Removes and returns every packet due by `now_us`, in arrival order.
    pub fn receive(&mut self, now_us: u64) -> Vec<T> {
        let mut due: Vec<InFlight<T>> = Vec::new();
        let mut i = 0;
        while i < self.in_flight.len() {
            if self.in_flight[i].due_us <= now_us {
                due.push(self.in_flight.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by_key(|p| (p.due_us, p.sequence));
        due.into_iter().map(|p| p.packet).collect()
    }

    /// Packets offered and packets dropped since creation.
    pub fn counts(&self) -> (u64, u64) {
        (self.sent, self.dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1000;

    fn drain(link: &mut ImpairedLink<u32>, until_ms: u64) -> Vec<(u64, u32)> {
        let mut out = Vec::new();
        for t in 0..=until_ms {
            out.extend(link.receive(t * MS).into_iter().map(|p| (t, p)));
        }
        out
    }

    #[test]
    fn a_perfect_link_delivers_everything_once_in_order_at_once() {
        let mut link = ImpairedLink::new(Impairment::default(), 1);
        for i in 0..50 {
            link.send(i as u64 * MS, i);
        }
        let got = drain(&mut link, 60);
        assert_eq!(got.len(), 50);
        assert!(got.iter().all(|&(t, p)| t == p as u64));
    }

    #[test]
    fn latency_and_jitter_bound_every_arrival_time() {
        let impairment = Impairment {
            latency_ms: 40,
            jitter_ms: 25,
            ..Impairment::default()
        };
        let mut link = ImpairedLink::new(impairment, 2);
        for i in 0..500u32 {
            link.send(i as u64 * 16 * MS, i);
        }
        let got = drain(&mut link, 500 * 16 + 100);
        assert_eq!(got.len(), 500);
        let mut reordered = 0;
        for (index, &(arrived, packet)) in got.iter().enumerate() {
            let flight = arrived - packet as u64 * 16;
            assert!(
                (40..=66).contains(&flight),
                "packet {packet} took {flight} ms"
            );
            if index > 0 && packet < got[index - 1].1 {
                reordered += 1;
            }
        }
        assert!(
            reordered > 20,
            "jitter wider than the send interval reorders: {reordered}"
        );
    }

    #[test]
    fn loss_and_duplication_rates_are_close_to_requested() {
        let impairment = Impairment {
            loss_permille: 200,
            duplicate_permille: 100,
            ..Impairment::default()
        };
        let mut link = ImpairedLink::new(impairment, 3);
        for i in 0..20_000u32 {
            link.send(0, i);
        }
        let got = link.receive(0);
        let (sent, dropped) = link.counts();
        assert_eq!(sent, 20_000);
        assert!((3700..=4300).contains(&dropped), "dropped {dropped}");
        let duplicates = got.len() as u64 - (sent - dropped);
        assert!(
            (1400..=1800).contains(&duplicates),
            "duplicated {duplicates}"
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_behaviour() {
        let impairment = Impairment {
            latency_ms: 30,
            jitter_ms: 30,
            loss_permille: 100,
            duplicate_permille: 50,
        };
        let run = |seed| {
            let mut link = ImpairedLink::new(impairment, seed);
            for i in 0..300u32 {
                link.send(i as u64 * 16 * MS, i);
            }
            drain(&mut link, 6000)
        };
        assert_eq!(run(4), run(4));
        assert_ne!(run(4), run(5));
    }

    #[test]
    fn total_loss_delivers_nothing_and_recovers_when_lifted() {
        let mut link = ImpairedLink::new(
            Impairment {
                loss_permille: 1000,
                ..Impairment::default()
            },
            6,
        );
        link.send(0, 1);
        assert!(link.receive(10 * MS).is_empty());
        link.set_impairment(Impairment::default());
        link.send(10 * MS, 2);
        assert_eq!(link.receive(10 * MS), [2]);
    }
}

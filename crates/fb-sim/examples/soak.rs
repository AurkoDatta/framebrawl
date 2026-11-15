//! Prints the per-frame checksums of a seeded random-input match, one hex
//! value per line. The cross-target harness diffs this against wasm.
//!
//! usage: soak [frames] [seed] [players]

use fb_sim::MatchConfig;
use fb_sim::soak::{checksums, checksums_with_rewinds};
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut next = |default: u64| args.next().and_then(|a| a.parse().ok()).unwrap_or(default);
    let frames = next(10_000) as u32;
    let config = MatchConfig {
        seed: next(0x5eed),
        players: next(4) as u8,
        ..MatchConfig::default()
    };

    let straight = checksums(config, frames);
    let rewound = checksums_with_rewinds(config, frames, 8);
    if let Some(frame) = straight.iter().zip(&rewound).position(|(a, b)| a != b) {
        eprintln!("rewound run diverged from the straight run at frame {frame}");
        std::process::exit(1);
    }

    let mut out = std::io::stdout().lock();
    for checksum in straight {
        writeln!(out, "{checksum:016x}").expect("write to stdout");
    }
}

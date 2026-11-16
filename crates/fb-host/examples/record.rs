//! Plays one match between bots and writes its replay to a file.
//!
//!     cargo run --release -p fb-host --example record -- 7 web/public/replays/bots.fbr
//!
//! The page bundles a replay made this way. It has to be made again after
//! any change to the simulation or the bots, because a replay only plays
//! back on the simulation that recorded it.

use fb_host::{Room, RoomOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(seed), Some(path)) = (args.next().and_then(|s| s.parse().ok()), args.next()) else {
        eprintln!("usage: record <seed> <output file>");
        std::process::exit(2);
    };

    let mut room = Room::new(seed, RoomOptions::default());
    let mut out = Vec::new();
    let finished = loop {
        out.clear();
        if let Some(finished) = room.tick(&mut out) {
            break finished;
        }
    };
    let bytes = finished.replay.encode();
    std::fs::write(&path, &bytes).expect("writes the replay");
    println!(
        "{path}: {} frames, scores {:?}, {} bytes",
        finished.replay.frames(),
        finished.scores,
        bytes.len()
    );
}

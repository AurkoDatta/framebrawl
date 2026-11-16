//! The replays bundled with the page must play back on the current
//! simulation. If this fails after a change to the simulation or the bots,
//! record them again (see `crates/fb-host/examples/record.rs`).

use std::path::Path;

use fb_wasm::ReplayPlayer;

#[test]
fn bundled_replays_play_back_and_match_every_checkpoint() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/public/replays");
    let mut played = 0;
    for entry in std::fs::read_dir(&folder).expect("the replay folder") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|ext| ext != "fbr") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("the replay file");
        let Ok(mut player) = ReplayPlayer::new(&bytes) else {
            panic!("{} does not decode", path.display());
        };
        while player.step() {}
        assert_eq!(player.frame(), player.frames());
        assert_eq!(player.diverged_at(), -1, "{}", path.display());
        assert!(player.checkpoints() > 60);
        assert!(!player.view().is_empty());
        played += 1;
    }
    assert!(
        played >= 2,
        "found {played} replays in {}",
        folder.display()
    );
}

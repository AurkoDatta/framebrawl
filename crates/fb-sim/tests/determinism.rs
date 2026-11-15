use fb_sim::soak::{InputScript, checksums, checksums_with_rewinds, digest};
use fb_sim::{MatchConfig, World};

fn config(seed: u64) -> MatchConfig {
    MatchConfig {
        seed,
        ..MatchConfig::default()
    }
}

#[test]
fn same_seed_same_stream_and_different_seed_differs() {
    let a = checksums(config(11), 600);
    assert_eq!(a, checksums(config(11), 600));
    assert_ne!(digest(&a), digest(&checksums(config(12), 600)));
}

#[test]
fn rewinding_and_resimulating_reproduces_the_straight_run() {
    for seed in [1, 2, 3] {
        let straight = checksums(config(seed), 3000);
        let rewound = checksums_with_rewinds(config(seed), 3000, 8);
        let first_diff = straight.iter().zip(&rewound).position(|(a, b)| a != b);
        assert_eq!(first_diff, None, "seed {seed} diverged");
    }
}

#[test]
fn the_soak_run_exercises_the_whole_rule_set() {
    // If random inputs stop producing ring-outs or falling tiles, the
    // determinism harness quietly stops covering those code paths.
    let cfg = MatchConfig {
        duration_frames: u32::MAX,
        score_limit: u16::MAX,
        ..config(5)
    };
    let mut world = World::new(cfg);
    let mut script = InputScript::new(cfg.seed, 4);
    for _ in 0..6000 {
        let inputs = script.next_frame(&world);
        world.step(&inputs);
    }
    let players: Vec<_> = (0..4).map(|i| world.player(i)).collect();
    assert!(
        players.iter().map(|p| p.falls).sum::<u16>() >= 4,
        "{players:?}"
    );
    assert!(
        players.iter().map(|p| p.score).sum::<u16>() >= 1,
        "{players:?}"
    );
    let gone = (0..world.tile_count())
        .filter(|&i| world.tile(i).1.is_none())
        .count();
    assert!(gone >= 20, "{gone} tiles gone");
}

#[test]
fn a_changed_field_is_named_by_the_canonical_view() {
    let mut a = World::new(config(1));
    let mut b = a.clone();
    let mut script = InputScript::new(1, 4);
    let mut inputs = script.next_frame(&a);
    a.step(&inputs);
    inputs[2].aim = inputs[2].aim.wrapping_add(64);
    inputs[2].buttons |= fb_sim::Input::MOVE;
    b.step(&inputs);

    assert_ne!(a.checksum(), b.checksum());
    let (fa, fb) = (a.fields(), b.fields());
    let diff = fa
        .iter()
        .zip(&fb)
        .find(|(x, y)| x != y)
        .expect("a difference");
    assert_eq!((diff.0.name, diff.0.index), ("player.facing", 2));
}

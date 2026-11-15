mod support;

use fb_rollback::bisect::{first_divergent_field, first_divergent_frame};
use fb_rollback::{InputDelay, SessionConfig, Simulation};
use fb_sim::{MatchConfig, Pcg32, World};
use support::{LinkQuality, Net, ToySim};

fn fixed_delay(delay: u8) -> SessionConfig {
    SessionConfig {
        input_delay: InputDelay::Fixed(delay),
        ..SessionConfig::default()
    }
}

/// Every client's confirmed states must equal the server's on every frame
/// both still hold.
fn assert_converged<S: Simulation>(net: &Net<S>, context: &str) {
    let server = net.authority.recent_checksums();
    for (player, client) in net.clients.iter().enumerate() {
        assert_eq!(client.session.desync(), None, "{context}: player {player}");
        let ours = client.session.recent_checksums();
        let shared = ours
            .iter()
            .filter(|(f, _)| server.iter().any(|(g, _)| g == f))
            .count();
        assert!(
            shared > 0,
            "{context}: player {player} shares no frames with the server"
        );
        assert_eq!(
            first_divergent_frame(&ours, &server),
            None,
            "{context}: player {player}"
        );
    }
}

#[test]
fn a_clean_link_never_stalls_or_substitutes_once_in_sync() {
    let mut net = Net::new(ToySim::new(3), 3, fixed_delay(0), LinkQuality::PERFECT, 1);
    net.run(120);
    let warmup: Vec<u32> = (0..3).map(|p| net.authority.substitutions(p)).collect();
    net.run(1800);
    for (player, client) in net.clients.iter().enumerate() {
        assert_eq!(
            net.authority.substitutions(player),
            warmup[player],
            "player {player}"
        );
        let stats = client.session.stats();
        assert_eq!(stats.stalls, 0);
        assert!(stats.checkpoints_verified >= 55, "{stats:?}");
        assert!(
            client.deepest_prediction <= 5,
            "predicted {} frames",
            client.deepest_prediction
        );
        assert!(
            stats.rollbacks > 50,
            "remote inputs change, so predictions miss: {stats:?}"
        );
    }
    assert_converged(&net, "clean link");
}

#[test]
fn design_target_link_stays_in_sync_for_five_minutes() {
    let mut net = Net::new(
        ToySim::new(4),
        4,
        SessionConfig::default(),
        LinkQuality::TARGET,
        2,
    );
    net.run(18_000);
    for (player, client) in net.clients.iter().enumerate() {
        let stats = client.session.stats();
        assert_eq!(client.session.desync(), None);
        assert_eq!(stats.stalls, 0, "player {player}: {stats:?}");
        assert!(
            stats.checkpoints_verified >= 590,
            "player {player}: {stats:?}"
        );
        assert!(
            net.authority.substitutions(player) < 180,
            "player {player}: {} of 18000 inputs arrived late",
            net.authority.substitutions(player)
        );
        assert!(
            client.deepest_prediction <= 20,
            "player {player} predicted {} frames ahead",
            client.deepest_prediction
        );
    }
    net.settle();
    assert_converged(&net, "target link");
}

#[test]
fn adaptive_delay_rises_on_a_slow_link_and_shortens_rollbacks() {
    let slow = LinkQuality {
        delay: 6,
        jitter: 0,
        loss_percent: 0,
        duplicate_percent: 0,
    };
    let mean_depth = |config: SessionConfig| {
        let mut net = Net::new(ToySim::new(2), 2, config, slow, 3);
        net.run(1200);
        net.clients[0].rollbacks.clear();
        net.run(1200);
        let rollbacks = &net.clients[0].rollbacks;
        let total: u32 = rollbacks.iter().map(|r| r.depth).sum();
        (
            total as f64 / rollbacks.len() as f64,
            net.clients[0].session.input_delay(),
        )
    };
    let (without, _) = mean_depth(fixed_delay(0));
    let (with, delay) = mean_depth(SessionConfig::default());
    assert_eq!(delay, 3, "a 200 ms round trip wants the full delay");
    // Three frames of delay buy a little under three frames of depth: the
    // pacer tolerates sitting one frame early rather than hunting.
    assert!(
        with <= without - 1.5,
        "mean rollback {with:.1} frames with delay, {without:.1} without"
    );
}

#[test]
fn random_bad_networks_never_desync() {
    for seed in 0..300u64 {
        let mut rng = Pcg32::new(seed ^ 0xfeed);
        let players = 2 + rng.below(3) as usize;
        let quality = LinkQuality {
            delay: rng.below(12),
            jitter: rng.below(10),
            loss_percent: rng.below(45),
            duplicate_percent: rng.below(20),
        };
        let config = SessionConfig {
            max_prediction: 12 + rng.below(30),
            input_delay: if rng.below(2) == 0 {
                InputDelay::Fixed(rng.below(4) as u8)
            } else {
                InputDelay::Adaptive { max: 3 }
            },
            checkpoint_interval: 1 + rng.below(40),
        };
        let mut net = Net::new(ToySim::new(players), players, config, quality, seed);
        net.run(1500);
        let context = format!("seed {seed} {quality:?} {config:?}");
        for client in &net.clients {
            assert_eq!(client.session.desync(), None, "{context}");
        }
        net.settle();
        assert_converged(&net, &context);
        for client in &net.clients {
            assert!(client.session.stats().checkpoints_verified > 0, "{context}");
        }
    }
}

#[test]
fn a_blackout_longer_than_the_input_history_recovers_by_snapshot() {
    let mut net = Net::new(
        ToySim::new(2),
        2,
        SessionConfig::default(),
        LinkQuality::PERFECT,
        5,
    );
    net.run(300);
    net.quality.loss_percent = 100;
    net.run(400);
    assert!(
        net.clients[0].session.stats().stalls > 300,
        "client waits rather than guessing"
    );
    net.settle();
    assert_converged(&net, "after blackout");
    let frame = net.clients[0].session.frame();
    assert!(
        frame + 10 > net.authority.frame(),
        "client caught up to frame {frame}"
    );
}

#[test]
fn a_corrupted_client_is_caught_and_the_bisect_names_frame_and_field() {
    let mut net = Net::new(ToySim::new(2), 2, fixed_delay(0), LinkQuality::TARGET, 6);
    net.run(200);

    // Corrupt player 1's simulation only; the server and player 0 are fine.
    let (frame, state, last) = (
        net.authority.frame(),
        net.authority.state().clone(),
        net.authority.last_inputs().to_vec(),
    );
    let mut bad = state;
    bad.glitch_at = Some(frame + 47);
    net.clients[1].session.resync(frame, bad, &last);
    net.authority.snapshot_sent(1);

    let mut caught = None;
    for _ in 0..200 {
        net.step();
        if let Some(desync) = net.clients[1].session.desync() {
            caught = Some(desync);
            break;
        }
    }
    let desync = caught.expect("checkpoint comparison notices");
    assert!(
        desync.frame > frame + 47 && desync.frame <= frame + 47 + 30,
        "{desync:?}"
    );
    assert_eq!(net.clients[0].session.desync(), None);

    // Both sides still hold the frames since the previous checkpoint.
    let ours = net.clients[1].session.recent_checksums();
    let theirs = net.authority.recent_checksums();
    let first_bad = first_divergent_frame(&ours, &theirs).expect("overlapping history");
    assert_eq!(first_bad, frame + 48, "the state after the corrupted step");
    let diff = first_divergent_field(
        &net.clients[1]
            .session
            .dump(first_bad)
            .expect("client state"),
        &net.authority.dump(first_bad).expect("server state"),
    )
    .expect("a differing field");
    assert_eq!((diff.name, diff.index), ("cell", 0));
    assert_eq!(diff.ours ^ diff.theirs, 1);

    // The server answers a desync report with a snapshot.
    net.clients[1].session.resync(
        net.authority.frame(),
        net.authority.state().clone(),
        net.authority.last_inputs(),
    );
    net.authority.snapshot_sent(1);
    net.run(600);
    net.settle();
    assert_converged(&net, "after resync");
}

#[test]
fn the_real_simulation_survives_the_design_target_link() {
    let config = MatchConfig {
        seed: 77,
        duration_frames: u32::MAX,
        score_limit: u16::MAX,
        ..MatchConfig::default()
    };
    // Five minutes in release builds; debug builds would take too long.
    let ticks = if cfg!(debug_assertions) { 900 } else { 18_000 };
    let mut net = Net::new(
        World::new(config),
        4,
        SessionConfig::default(),
        LinkQuality::TARGET,
        7,
    );
    net.run(ticks);
    let rollbacks: u32 = net
        .clients
        .iter()
        .map(|c| c.session.stats().rollbacks)
        .sum();
    assert!(rollbacks > ticks / 10, "{rollbacks} rollbacks");
    net.settle();
    assert_converged(&net, "real simulation");
    for client in &net.clients {
        assert!(client.session.stats().checkpoints_verified as u64 >= ticks as u64 / 40);
    }
}

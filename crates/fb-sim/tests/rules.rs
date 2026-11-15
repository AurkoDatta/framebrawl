use fb_sim::{Input, MatchConfig, TileState, World};

fn duel(seed: u64) -> World {
    World::new(MatchConfig {
        seed,
        players: 2,
        props: 0,
        ..MatchConfig::default()
    })
}

fn idle(world: &mut World, frames: u32) {
    let inputs = vec![Input::default(); world.player_count()];
    for _ in 0..frames {
        world.step(&inputs);
    }
}

/// Aim byte pointing from `from` towards `to` on the ground plane.
fn aim_at(from: [f32; 3], to: [f32; 3]) -> u8 {
    let turns = (to[2] - from[2]).atan2(to[0] - from[0]) / std::f32::consts::TAU;
    (turns.rem_euclid(1.0) * 256.0).round() as u32 as u8
}

#[test]
fn players_settle_on_the_arena() {
    let mut world = duel(1);
    idle(&mut world, 120);
    for i in 0..2 {
        let p = world.player(i);
        assert!(p.grounded, "player {i} is standing");
        assert!(
            (p.position[1] - 0.75).abs() < 0.05,
            "resting height {}",
            p.position[1]
        );
        assert!(p.velocity.iter().all(|v| v.abs() < 0.05));
    }
}

#[test]
fn running_reaches_top_speed_and_stops_when_released() {
    let mut world = duel(1);
    idle(&mut world, 60);
    let start = world.player(0).position;
    for _ in 0..30 {
        world.step(&[Input::new(Input::MOVE, 0), Input::default()]);
    }
    let p = world.player(0);
    assert!(p.position[0] > start[0] + 2.0);
    assert!((p.velocity[0] - 7.0).abs() < 0.2, "speed {}", p.velocity[0]);
    assert!((p.position[2] - start[2]).abs() < 0.01, "no sideways drift");
    idle(&mut world, 20);
    assert!(world.player(0).velocity[0].abs() < 0.05);
}

#[test]
fn jump_needs_ground_and_a_fresh_press() {
    let mut world = duel(1);
    idle(&mut world, 60);
    let jump = [Input::new(Input::JUMP, 0), Input::default()];
    world.step(&jump);
    assert!(world.player(0).velocity[1] > 5.0);

    // Still holding the button in the air and on landing: no second jump.
    let mut peak: f32 = 0.0;
    for _ in 0..120 {
        world.step(&jump);
        peak = peak.max(world.player(0).position[1]);
    }
    assert!(peak > 1.8, "jump clears a crate, peaked at {peak}");
    let p = world.player(0);
    assert!(p.grounded && p.velocity[1].abs() < 0.05);
}

#[test]
fn walking_off_the_edge_costs_a_fall_and_respawns_shielded() {
    let mut world = duel(1);
    idle(&mut world, 60);
    // Player 0 starts near the -x edge.
    let run_off = [Input::new(Input::MOVE, 128), Input::default()];
    let mut fell_at = None;
    for frame in 0..400 {
        world.step(&run_off);
        if !world.player(0).in_play {
            fell_at = Some(frame);
            break;
        }
    }
    assert!(fell_at.is_some(), "player fell off");
    let p = world.player(0);
    assert_eq!((p.falls, p.score), (1, 0));
    assert_eq!(world.player(1).score, 0, "nobody shoved, nobody scores");

    idle(&mut world, 121);
    let p = world.player(0);
    assert!(p.in_play && p.shielded);
    idle(&mut world, 90);
    let p = world.player(0);
    assert!(
        p.grounded && !p.shielded,
        "landed on a tile at {:?}",
        p.position
    );
}

#[test]
fn repeated_shoves_ring_out_the_victim_and_score() {
    let mut world = duel(3);
    idle(&mut world, 95);

    let mut shove_held = false;
    for _ in 0..6000 {
        let (me, them) = (world.player(0), world.player(1));
        let aim = aim_at(me.position, them.position);
        let gap = (me.position[0] - them.position[0]).hypot(me.position[2] - them.position[2]);
        let mut buttons = Input::MOVE;
        // Tap rather than hold: a shove fires on the press.
        shove_held = !shove_held && gap < 1.6 && them.in_play;
        if shove_held {
            buttons |= Input::SHOVE;
        }
        world.step(&[Input::new(buttons, aim), Input::default()]);
        if world.player(0).score > 0 {
            break;
        }
    }
    assert_eq!(world.player(0).score, 1, "attacker credited");
    assert_eq!(world.player(1).falls, 1);
    assert_eq!(world.player(1).strain, 0, "strain resets on ring-out");
}

#[test]
fn a_missed_shove_still_costs_its_cooldown() {
    let mut world = duel(3);
    idle(&mut world, 60);
    world.step(&[Input::new(Input::SHOVE, 32), Input::default()]);
    assert_eq!(world.player(1).strain, 0, "victim is across the arena");
    assert!(world.player(0).shove_cooldown > 0);
}

#[test]
fn tiles_crack_then_fall_then_disappear() {
    let mut world = World::new(MatchConfig {
        seed: 9,
        players: 2,
        ..MatchConfig::default()
    });
    let count = |world: &World, want: fn(&TileState) -> bool| {
        (0..world.tile_count())
            .filter(|&i| want(&world.tile(i).0))
            .count()
    };
    idle(&mut world, 599);
    assert_eq!(
        count(&world, |s| *s == TileState::Solid),
        81,
        "grace period"
    );
    idle(&mut world, 2);
    assert_eq!(
        count(&world, |s| matches!(s, TileState::Cracking { .. })),
        1
    );
    idle(&mut world, 90);
    assert_eq!(count(&world, |s| *s == TileState::Falling), 1);
    idle(&mut world, 3000);
    let gone = count(&world, |s| *s == TileState::Gone);
    assert!(gone > 10, "{gone} tiles gone after a minute");
    let gone_have_no_body = (0..world.tile_count())
        .all(|i| (world.tile(i).0 == TileState::Gone) == world.tile(i).1.is_none());
    assert!(gone_have_no_body);
}

#[test]
fn the_arena_never_shrinks_below_one_tile_per_player() {
    let mut world = World::new(MatchConfig {
        seed: 4,
        players: 2,
        grid: 4,
        props: 0,
        duration_frames: u32::MAX,
        ..MatchConfig::default()
    });
    idle(&mut world, 4000);
    let solid = (0..world.tile_count())
        .filter(|&i| world.tile(i).0 == TileState::Solid)
        .count();
    assert_eq!(solid, 2);
}

#[test]
fn match_ends_on_time() {
    let mut world = World::new(MatchConfig {
        players: 2,
        duration_frames: 10,
        ..MatchConfig::default()
    });
    idle(&mut world, 9);
    assert!(!world.finished());
    idle(&mut world, 1);
    assert!(world.finished());
}

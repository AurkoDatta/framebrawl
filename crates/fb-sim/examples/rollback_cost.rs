//! Times the pieces of a rollback natively: step, snapshot, and a full
//! restore-and-resimulate of 8 frames. The browser numbers come from the
//! e2e harness.

use fb_sim::soak::InputScript;
use fb_sim::{MatchConfig, World};
use std::hint::black_box;
use std::time::Instant;

fn per_call_ms(iterations: u32, mut run: impl FnMut()) -> f64 {
    for _ in 0..iterations / 8 {
        run();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        run();
    }
    start.elapsed().as_secs_f64() * 1e3 / iterations as f64
}

fn main() {
    let config = MatchConfig {
        seed: 0x5eed,
        ..MatchConfig::default()
    };
    let mut world = World::new(config);
    let mut script = InputScript::new(config.seed, config.players as usize);
    for _ in 0..400 {
        let inputs = script.next_frame(&world);
        world.step(&inputs);
    }
    let snapshot = world.clone();

    let rollback = per_call_ms(2000, || {
        world.clone_from(&snapshot);
        for _ in 0..8 {
            black_box(world.clone());
            let inputs = script.next_frame(&world);
            world.step(&inputs);
        }
    });
    let save = per_call_ms(8000, || {
        black_box(world.clone());
    });
    world.clone_from(&snapshot);
    let step = per_call_ms(4000, || {
        let inputs = script.next_frame(&world);
        world.step(&inputs);
    });

    println!("rollback of 8 frames  {rollback:.3} ms");
    println!("snapshot              {save:.4} ms");
    println!("step                  {step:.4} ms");
}

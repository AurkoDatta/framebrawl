//! The Lab object driven natively, the way the page's worker drives it.

use fb_wasm::Lab;

const CLIENT_FRAME: usize = 0;
const CONFIRMED: usize = 1;
const SERVER_FRAME: usize = 2;
const STALLS: usize = 6;
const DESYNCS: usize = 12;

fn run(lab: &mut Lab, ticks: u32) -> u32 {
    let mut deepest = 0;
    for tick in 0..ticks {
        // Run in a slow circle and shove now and then.
        let buttons = 1 | if tick % 50 == 0 { 4 } else { 0 };
        lab.tick(buttons, (tick / 2) as u8);
        deepest = deepest.max(lab.sample()[3] as u32);
    }
    deepest
}

#[test]
fn a_clean_link_keeps_the_client_a_step_ahead_and_never_desyncs() {
    let mut lab = Lab::new(7, -1);
    let deepest = run(&mut lab, 600);
    let sample = lab.sample();
    assert!(sample[CONFIRMED] > 550.0, "{sample:?}");
    assert!(sample[CLIENT_FRAME] >= sample[SERVER_FRAME], "{sample:?}");
    assert!(sample[CLIENT_FRAME] - sample[CONFIRMED] < 6.0, "{sample:?}");
    assert_eq!(sample[DESYNCS], 0.0);
    assert!(deepest <= 4, "deepest rollback {deepest}");
    assert!(!lab.view().is_empty());
    assert_eq!(lab.truth().len(), 1 + 4 * 5);
}

#[test]
fn a_bad_link_means_deeper_rollbacks_but_still_no_desync() {
    let mut lab = Lab::new(7, -1);
    run(&mut lab, 120);
    // 75 ms each way, 20 ms of jitter, 5% loss, 2% duplication.
    lab.set_network(75, 20, 50, 20);
    let deepest = run(&mut lab, 1800);
    let sample = lab.sample();
    assert!(deepest >= 8, "deepest rollback {deepest}");
    assert_eq!(sample[DESYNCS], 0.0);
    // The link got worse in one step, so the client may run out of
    // confirmed frames once or twice while its pacing catches up.
    assert!(sample[STALLS] <= 3.0, "{sample:?}");
    assert!(sample[9] > 0.0, "some packets were lost");
    assert!(sample[5] >= 150.0, "round trip {} ms", sample[5]);
}

#[test]
fn a_corrupted_client_is_caught_explained_and_resynced() {
    let mut lab = Lab::new(7, 2);
    lab.set_network(30, 5, 20, 0);
    run(&mut lab, 300);
    assert_eq!(lab.take_report(), "");

    lab.corrupt();
    run(&mut lab, 120);
    let report = lab.take_report();
    assert!(report.contains("player.strain[0]"), "{report}");
    assert_eq!(lab.sample()[DESYNCS], 1.0);

    // After the resync the client verifies checkpoints again and stays in
    // step: no second desync over the next ten seconds.
    let before = lab.sample()[CONFIRMED];
    run(&mut lab, 600);
    let sample = lab.sample();
    assert!(sample[CONFIRMED] > before + 500.0, "{sample:?}");
    assert_eq!(sample[DESYNCS], 1.0);
}

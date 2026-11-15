//! Keeping a client the right distance ahead of the server.
//!
//! The server reports, with every packet, how many frames early the
//! client's newest input was. Too little and inputs miss their frame and
//! get substituted; too much and the client is predicting further ahead
//! than it needs to, which makes rollbacks longer. The client nudges its
//! tick rate to hold the worst recent lead at a small positive target.

use std::collections::VecDeque;

const WINDOW: usize = 40;
/// Worst lead, over the window, that the client aims for.
const TARGET_LEAD: i32 = 1;
/// Tick rate change per frame of error, in thousandths of a tick.
const GAIN: i32 = 40;
const MAX_NUDGE: i32 = 120;
/// Beyond this many frames of deficit, catch up at once instead of slowly.
const WARP_THRESHOLD: i32 = 10;

#[derive(Clone, Debug, Default)]
pub struct Pacer {
    leads: VecDeque<i8>,
    /// Fractional ticks owed, in thousandths.
    credit: i32,
}

impl Pacer {
    pub fn sample(&mut self, lead: i8) {
        if self.leads.len() == WINDOW {
            self.leads.pop_front();
        }
        self.leads.push_back(lead);
    }

    /// Frames the client should move ahead (positive) or fall back
    /// (negative) relative to the server. Zero until enough samples exist.
    pub fn error(&self) -> i32 {
        if self.leads.len() < WINDOW / 4 {
            return 0;
        }
        let worst = self.leads.iter().copied().min().unwrap_or(0) as i32;
        let error = TARGET_LEAD - worst;
        // Being a frame early is cheap; chasing it would make the rate
        // hunt. Only slow down when clearly too far ahead.
        if error == -1 { 0 } else { error }
    }

    /// How many simulation frames to run on this timer tick: usually one,
    /// occasionally zero or two, and a burst after a long stall.
    pub fn frames_due(&mut self) -> u32 {
        let error = self.error();
        if error >= WARP_THRESHOLD {
            // The samples describe a position we are about to leave.
            self.leads.clear();
            return 1 + error as u32;
        }
        self.credit += 1000 + (error * GAIN).clamp(-MAX_NUDGE, MAX_NUDGE);
        let due = self.credit / 1000;
        self.credit -= due * 1000;
        due as u32
    }

    pub fn reset(&mut self) {
        self.leads.clear();
        self.credit = 0;
    }
}

/// Chooses how many frames to delay local input.
///
/// A client predicts roughly one round trip ahead. Each frame of input
/// delay lets it run one frame less far ahead, trading a little latency on
/// its own controls for shorter rollbacks. Delay only grows once prediction
/// is deep enough to matter, and changes one step at a time after the
/// measurement has held steady.
#[derive(Clone, Debug)]
pub struct DelayPolicy {
    max_delay: u8,
    /// Smoothed prediction depth with the delay added back, in 1/256ths
    /// of a frame.
    depth_x256: u32,
    steady_for: u32,
}

const SETTLE_FRAMES: u32 = 90;

impl DelayPolicy {
    pub fn new(max_delay: u8) -> Self {
        DelayPolicy {
            max_delay,
            depth_x256: 0,
            steady_for: 0,
        }
    }

    /// Feeds one observation of how many frames were predicted beyond the
    /// confirmed frame, and returns the delay to use from now on.
    pub fn update(&mut self, predicted_frames: u32, current_delay: u8) -> u8 {
        let undelayed = (predicted_frames + current_delay as u32) * 256;
        self.depth_x256 = (self.depth_x256 * 15 + undelayed) / 16;
        let depth = (self.depth_x256 + 128) / 256;
        let wanted = match depth {
            0..=5 => 0,
            6..=8 => 1,
            9..=11 => 2,
            _ => 3,
        }
        .min(self.max_delay);

        if wanted == current_delay {
            self.steady_for = 0;
            return current_delay;
        }
        self.steady_for += 1;
        if self.steady_for < SETTLE_FRAMES {
            return current_delay;
        }
        self.steady_for = 0;
        if wanted > current_delay {
            current_delay + 1
        } else {
            current_delay - 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pacer: &mut Pacer, lead: i8, ticks: u32) -> u32 {
        (0..ticks)
            .map(|_| {
                pacer.sample(lead);
                pacer.frames_due()
            })
            .sum()
    }

    #[test]
    fn runs_one_frame_per_tick_when_on_target() {
        let mut pacer = Pacer::default();
        assert_eq!(run(&mut pacer, 1, 600), 600);
        assert_eq!(
            run(&mut pacer, 2, 600),
            600,
            "one frame early is left alone"
        );
    }

    #[test]
    fn speeds_up_when_inputs_arrive_late_and_slows_when_far_ahead() {
        let mut late = Pacer::default();
        let frames = run(&mut late, -2, 600);
        assert!(
            (640..=680).contains(&frames),
            "ran {frames} frames in 600 ticks"
        );

        let mut early = Pacer::default();
        let frames = run(&mut early, 6, 600);
        assert!(
            (520..=560).contains(&frames),
            "ran {frames} frames in 600 ticks"
        );
    }

    #[test]
    fn warps_after_a_long_stall() {
        let mut pacer = Pacer::default();
        for _ in 0..WINDOW {
            pacer.sample(-30);
        }
        assert_eq!(pacer.frames_due(), 32);
        assert_eq!(pacer.frames_due(), 1, "and then starts measuring afresh");
    }

    #[test]
    fn delay_follows_prediction_depth_one_step_at_a_time() {
        let mut policy = DelayPolicy::new(3);
        let mut delay = 0;
        let mut changes = Vec::new();
        for frame in 0..1200 {
            // A 200 ms round trip: about 12 frames predicted with no delay.
            let next = policy.update(12 - delay as u32, delay);
            if next != delay {
                changes.push((frame, next));
                delay = next;
            }
        }
        assert_eq!(changes.iter().map(|c| c.1).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(changes.windows(2).all(|w| w[1].0 - w[0].0 >= SETTLE_FRAMES));

        for _ in 0..600 {
            delay = policy.update(3u32.saturating_sub(delay as u32), delay);
        }
        assert_eq!(delay, 0, "falls back when the link improves");
    }

    #[test]
    fn delay_ignores_a_brief_spike() {
        let mut policy = DelayPolicy::new(3);
        let mut delay = 0;
        for frame in 0..600 {
            let depth = if (300..320).contains(&frame) { 14 } else { 3 };
            delay = policy.update(depth, delay);
            assert_eq!(delay, 0, "frame {frame}");
        }
    }
}

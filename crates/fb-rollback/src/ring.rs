use crate::Frame;

/// Saved states for the most recent frames, addressed by frame number.
///
/// Slots are reused with `clone_from`, so after warm-up saving a state
/// allocates nothing. A slot remembers which frame it holds; asking for a
/// frame that has since been overwritten returns `None` rather than the
/// wrong state.
pub struct StateRing<S> {
    slots: Vec<Option<(Frame, S)>>,
}

impl<S: Clone> StateRing<S> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        StateRing {
            slots: (0..capacity).map(|_| None).collect(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn save(&mut self, frame: Frame, state: &S) {
        let index = frame as usize % self.slots.len();
        match &mut self.slots[index] {
            Some((held, slot)) => {
                *held = frame;
                slot.clone_from(state);
            }
            empty => *empty = Some((frame, state.clone())),
        }
    }

    pub fn get(&self, frame: Frame) -> Option<&S> {
        match &self.slots[frame as usize % self.slots.len()] {
            Some((held, state)) if *held == frame => Some(state),
            _ => None,
        }
    }

    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(|slot| *slot = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_sim::Pcg32;
    use std::collections::BTreeMap;

    #[test]
    fn holds_the_last_capacity_frames_and_forgets_older_ones() {
        let mut ring = StateRing::new(4);
        for frame in 0..10u32 {
            ring.save(frame, &frame.to_string());
        }
        for frame in 0..6 {
            assert_eq!(ring.get(frame), None, "frame {frame} was overwritten");
        }
        for frame in 6..10 {
            assert_eq!(
                ring.get(frame).map(String::as_str),
                Some(&*frame.to_string())
            );
        }
        assert_eq!(ring.get(10), None, "never saved");
    }

    #[test]
    fn agrees_with_a_map_under_random_saves_and_rewinds() {
        // Rollback saves frames out of order: it rewinds and saves the same
        // frames again with different contents.
        for seed in 0..200 {
            let mut rng = Pcg32::new(seed);
            let capacity = 1 + rng.below(12) as usize;
            let mut ring = StateRing::new(capacity);
            let mut model: BTreeMap<Frame, u32> = BTreeMap::new();
            let mut frame: Frame = rng.below(1000);
            for _ in 0..300 {
                if rng.below(4) == 0 {
                    frame = frame.saturating_sub(rng.below(capacity as u32 + 2));
                }
                let value = rng.next_u32();
                ring.save(frame, &value);
                model.retain(|&f, _| f as usize % capacity != frame as usize % capacity);
                model.insert(frame, value);
                frame += 1;

                let probe = frame.saturating_sub(rng.below(2 * capacity as u32 + 2));
                assert_eq!(
                    ring.get(probe),
                    model.get(&probe),
                    "seed {seed} frame {probe}"
                );
            }
        }
    }
}

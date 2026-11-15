use std::collections::VecDeque;

use crate::Frame;

/// A contiguous run of per-frame values that grows at the end and is
/// trimmed from the start.
#[derive(Clone, Debug)]
pub struct FrameLog<T> {
    start: Frame,
    items: VecDeque<T>,
}

impl<T> FrameLog<T> {
    pub fn starting_at(start: Frame) -> Self {
        FrameLog {
            start,
            items: VecDeque::new(),
        }
    }

    pub fn start(&self) -> Frame {
        self.start
    }

    /// One past the last frame held.
    pub fn end(&self) -> Frame {
        self.start + self.items.len() as Frame
    }

    pub fn get(&self, frame: Frame) -> Option<&T> {
        self.items.get(frame.checked_sub(self.start)? as usize)
    }

    pub fn get_mut(&mut self, frame: Frame) -> Option<&mut T> {
        self.items.get_mut(frame.checked_sub(self.start)? as usize)
    }

    /// Appends the value for frame `self.end()`.
    pub fn push(&mut self, item: T) {
        self.items.push_back(item);
    }

    /// Drops everything before `frame`. An empty log jumps forward so the
    /// next push lands on `frame`.
    pub fn trim_before(&mut self, frame: Frame) {
        let drop = frame
            .saturating_sub(self.start)
            .min(self.items.len() as Frame);
        self.items.drain(..drop as usize);
        self.start += drop;
        if self.items.is_empty() && frame > self.start {
            self.start = frame;
        }
    }

    pub fn reset(&mut self, start: Frame) {
        self.items.clear();
        self.start = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_by_frame_and_trims_from_the_front() {
        let mut log = FrameLog::starting_at(10);
        for value in 0..5 {
            log.push(value);
        }
        assert_eq!((log.start(), log.end()), (10, 15));
        assert_eq!(log.get(9), None);
        assert_eq!(log.get(12), Some(&2));
        assert_eq!(log.get(15), None);

        log.trim_before(13);
        assert_eq!((log.start(), log.end()), (13, 15));
        assert_eq!(log.get(12), None);
        assert_eq!(log.get(13), Some(&3));

        log.trim_before(5);
        assert_eq!(log.start(), 13, "trimming backwards is a no-op");
    }

    #[test]
    fn trimming_past_the_end_moves_an_empty_log_forward() {
        let mut log = FrameLog::starting_at(0);
        log.push('a');
        log.trim_before(7);
        assert_eq!((log.start(), log.end()), (7, 7));
        log.push('b');
        assert_eq!(log.get(7), Some(&'b'));
    }
}

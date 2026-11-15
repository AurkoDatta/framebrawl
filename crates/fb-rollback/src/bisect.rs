//! Narrowing a desync down to the first frame and field that differ.

use fb_sim::canon::Field;

use crate::Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDiff {
    pub name: &'static str,
    pub index: u32,
    pub ours: u64,
    pub theirs: u64,
}

/// The earliest frame present in both lists whose checksums differ. Both
/// lists must be sorted by frame.
pub fn first_divergent_frame(ours: &[(Frame, u64)], theirs: &[(Frame, u64)]) -> Option<Frame> {
    let (mut i, mut j) = (0, 0);
    while i < ours.len() && j < theirs.len() {
        match ours[i].0.cmp(&theirs[j].0) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                if ours[i].1 != theirs[j].1 {
                    return Some(ours[i].0);
                }
                i += 1;
                j += 1;
            }
        }
    }
    None
}

/// The first field that differs between two canonical dumps of one frame.
/// A length mismatch is reported against the first missing field.
pub fn first_divergent_field(ours: &[Field], theirs: &[Field]) -> Option<FieldDiff> {
    for (a, b) in ours.iter().zip(theirs) {
        if a != b {
            return Some(FieldDiff {
                name: a.name,
                index: a.index,
                ours: a.bits,
                theirs: b.bits,
            });
        }
    }
    let (longer, ours_longer) = if ours.len() > theirs.len() {
        (ours, true)
    } else {
        (theirs, false)
    };
    longer.get(ours.len().min(theirs.len())).map(|f| FieldDiff {
        name: f.name,
        index: f.index,
        ours: if ours_longer { f.bits } else { 0 },
        theirs: if ours_longer { 0 } else { f.bits },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_first_common_frame_that_differs() {
        let ours = [(10, 1), (11, 2), (12, 3), (13, 4)];
        let theirs = [(11, 2), (12, 9), (13, 9), (14, 9)];
        assert_eq!(first_divergent_frame(&ours, &theirs), Some(12));
        assert_eq!(first_divergent_frame(&ours, &ours), None);
        assert_eq!(first_divergent_frame(&ours, &[(20, 0)]), None, "no overlap");
    }

    #[test]
    fn names_the_field_including_when_one_dump_is_shorter() {
        let f = |name, index, bits| Field { name, index, bits };
        let ours = [f("frame", 0, 5), f("player.strain", 1, 12)];
        let theirs = [f("frame", 0, 5), f("player.strain", 1, 24)];
        assert_eq!(
            first_divergent_field(&ours, &theirs),
            Some(FieldDiff {
                name: "player.strain",
                index: 1,
                ours: 12,
                theirs: 24
            })
        );
        assert_eq!(first_divergent_field(&ours, &ours), None);
        let diff = first_divergent_field(&ours[..1], &theirs).expect("length mismatch");
        assert_eq!(
            (diff.name, diff.ours, diff.theirs),
            ("player.strain", 0, 24)
        );
    }
}

//! Unit vectors for the 256 aim headings.
//!
//! `f32::sin` and `f32::cos` are allowed to differ between platforms, so the
//! table is built from a polynomial that uses only multiplication and
//! addition, which IEEE 754 pins down exactly.

use std::f32::consts::PI;

/// `(x, z)` of the unit vector for `aim`.
pub fn heading(aim: u8) -> (f32, f32) {
    let t = (aim & 63) as f32 * (PI / 128.0);
    let (s, c) = (sin_quarter(t), cos_quarter(t));
    match aim >> 6 {
        0 => (c, s),
        1 => (-s, c),
        2 => (-c, -s),
        _ => (s, -c),
    }
}

/// The aim whose heading points most nearly along `(dx, dz)`.
///
/// A search over the table rather than `atan2`, so bots and tests get the
/// same answer on every target. Returns 0 for a zero vector.
pub fn aim_towards(dx: f32, dz: f32) -> u8 {
    let mut best = (0u8, f32::NEG_INFINITY);
    for aim in 0..=255u8 {
        let (x, z) = heading(aim);
        let dot = x * dx + z * dz;
        if dot > best.1 {
            best = (aim, dot);
        }
    }
    best.0
}

// Taylor series in Horner form, valid on [0, pi/2). Truncation error is
// below 1e-7 there, under f32 resolution near 1.
fn sin_quarter(t: f32) -> f32 {
    let t2 = t * t;
    t * (1.0
        + t2 * (-1.0 / 6.0
            + t2 * (1.0 / 120.0
                + t2 * (-1.0 / 5040.0 + t2 * (1.0 / 362_880.0 + t2 * (-1.0 / 39_916_800.0))))))
}

fn cos_quarter(t: f32) -> f32 {
    let t2 = t * t;
    1.0 + t2
        * (-0.5
            + t2 * (1.0 / 24.0
                + t2 * (-1.0 / 720.0
                    + t2 * (1.0 / 40_320.0
                        + t2 * (-1.0 / 3_628_800.0 + t2 * (1.0 / 479_001_600.0))))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinal_headings_are_exact() {
        assert_eq!(heading(0), (1.0, 0.0));
        assert_eq!(heading(64), (-0.0, 1.0));
        assert_eq!(heading(128), (-1.0, -0.0));
        assert_eq!(heading(192), (0.0, -1.0));
    }

    #[test]
    fn aim_towards_inverts_heading() {
        for aim in 0..=255u8 {
            let (x, z) = heading(aim);
            assert_eq!(aim_towards(x * 3.0, z * 3.0), aim);
        }
        assert_eq!(aim_towards(0.0, 0.0), 0);
        assert_eq!(aim_towards(-5.0, 0.01), 128);
    }

    #[test]
    fn close_to_real_trigonometry_and_unit_length() {
        for aim in 0..=255u8 {
            let (x, z) = heading(aim);
            let angle = aim as f64 * std::f64::consts::TAU / 256.0;
            assert!((x as f64 - angle.cos()).abs() < 1e-6, "cos at {aim}");
            assert!((z as f64 - angle.sin()).abs() < 1e-6, "sin at {aim}");
            assert!((x * x + z * z - 1.0).abs() < 1e-6);
        }
    }
}

//! The canonical view of a world: a fixed-order list of named 64-bit values.
//!
//! The checksum hashes the values; the desync bisect compares two lists and
//! reports the first name that differs. Keeping both on one traversal means
//! they cannot drift apart.

pub trait Sink {
    fn put(&mut self, name: &'static str, index: u32, bits: u64);
}

/// FNV-1a 64 over each value's little-endian bytes.
pub struct Fnv64(pub u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }
}

impl Sink for Fnv64 {
    fn put(&mut self, _name: &'static str, _index: u32, bits: u64) {
        for byte in bits.to_le_bytes() {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub index: u32,
    pub bits: u64,
}

impl Sink for Vec<Field> {
    fn put(&mut self, name: &'static str, index: u32, bits: u64) {
        self.push(Field { name, index, bits });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_published_vectors() {
        // FNV-1a 64 of the empty input and of eight zero bytes.
        assert_eq!(Fnv64::default().0, 0xcbf2_9ce4_8422_2325);
        let mut h = Fnv64::default();
        h.put("", 0, 0);
        let mut expect: u64 = 0xcbf2_9ce4_8422_2325;
        for _ in 0..8 {
            expect = expect.wrapping_mul(0x0000_0100_0000_01b3);
        }
        assert_eq!(h.0, expect);
    }
}

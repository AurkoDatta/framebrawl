//! Bit-level reading and writing, least significant bit first.

#[derive(Debug, PartialEq, Eq)]
pub struct OutOfData;

#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    /// Bits used in the last byte; 0 means the next bit starts a new byte.
    used: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        BitWriter::default()
    }

    /// Writes the low `count` bits of `value`. `count` is at most 64.
    pub fn put(&mut self, value: u64, count: u8) {
        debug_assert!(count <= 64);
        debug_assert!(
            count == 64 || value >> count == 0,
            "value wider than {count} bits"
        );
        let mut value = value;
        let mut left = count;
        while left > 0 {
            if self.used == 0 {
                self.bytes.push(0);
            }
            let take = (8 - self.used).min(left);
            let mask = ((1u16 << take) - 1) as u64;
            *self.bytes.last_mut().expect("pushed above") |= ((value & mask) as u8) << self.used;
            value = if take == 64 { 0 } else { value >> take };
            self.used = (self.used + take) % 8;
            left -= take;
        }
    }

    pub fn put_bool(&mut self, value: bool) {
        self.put(value as u64, 1);
    }

    /// Pads to a byte boundary, then appends `bytes` verbatim.
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        self.used = 0;
        self.bytes.extend_from_slice(bytes);
    }

    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

pub struct BitReader<'a> {
    bytes: &'a [u8],
    /// Position in bits from the start.
    at: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        BitReader { bytes, at: 0 }
    }

    pub fn get(&mut self, count: u8) -> Result<u64, OutOfData> {
        debug_assert!(count <= 64);
        if self.at + count as usize > self.bytes.len() * 8 {
            return Err(OutOfData);
        }
        let mut value = 0u64;
        let mut got = 0u8;
        while got < count {
            let byte = self.bytes[self.at / 8];
            let offset = (self.at % 8) as u8;
            let take = (8 - offset).min(count - got);
            let mask = ((1u16 << take) - 1) as u8;
            value |= (((byte >> offset) & mask) as u64) << got;
            got += take;
            self.at += take as usize;
        }
        Ok(value)
    }

    pub fn get_bool(&mut self) -> Result<bool, OutOfData> {
        Ok(self.get(1)? == 1)
    }

    /// Skips to a byte boundary and returns everything after it.
    pub fn rest(&mut self) -> &'a [u8] {
        let start = self.at.div_ceil(8).min(self.bytes.len());
        self.at = self.bytes.len() * 8;
        &self.bytes[start..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_sim::Pcg32;

    #[test]
    fn round_trips_random_fields_of_every_width() {
        for seed in 0..200 {
            let mut rng = Pcg32::new(seed);
            let fields: Vec<(u64, u8)> = (0..1 + rng.below(40))
                .map(|_| {
                    let width = 1 + rng.below(64) as u8;
                    let raw = (rng.next_u32() as u64) << 32 | rng.next_u32() as u64;
                    let value = if width == 64 {
                        raw
                    } else {
                        raw & ((1 << width) - 1)
                    };
                    (value, width)
                })
                .collect();
            let mut writer = BitWriter::new();
            for &(value, width) in &fields {
                writer.put(value, width);
            }
            let bytes = writer.finish();
            let total: usize = fields.iter().map(|f| f.1 as usize).sum();
            assert_eq!(bytes.len(), total.div_ceil(8));

            let mut reader = BitReader::new(&bytes);
            for &(value, width) in &fields {
                assert_eq!(reader.get(width), Ok(value), "seed {seed}");
            }
        }
    }

    #[test]
    fn reading_past_the_end_is_an_error_not_a_panic() {
        let mut reader = BitReader::new(&[0xff]);
        assert_eq!(reader.get(5), Ok(31));
        assert_eq!(reader.get(4), Err(OutOfData));
        assert_eq!(reader.get(3), Ok(7), "a failed read consumes nothing");
        assert_eq!(BitReader::new(&[]).get(1), Err(OutOfData));
    }

    #[test]
    fn raw_bytes_follow_a_byte_boundary() {
        let mut writer = BitWriter::new();
        writer.put(0b101, 3);
        writer.put_bytes(b"hi");
        let bytes = writer.finish();
        assert_eq!(bytes, [0b101, b'h', b'i']);
        let mut reader = BitReader::new(&bytes);
        assert_eq!(reader.get(3), Ok(0b101));
        assert_eq!(reader.rest(), b"hi");
    }
}

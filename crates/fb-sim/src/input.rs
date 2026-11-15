/// One player's input for one frame: two bytes on the wire.
///
/// `aim` is a heading in 1/256ths of a turn, counter-clockwise from +x when
/// seen from above with +z towards the viewer. Everything a client can
/// express is an integer, so there is nothing to round differently between
/// machines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub buttons: u8,
    pub aim: u8,
}

impl Input {
    pub const MOVE: u8 = 1 << 0;
    pub const JUMP: u8 = 1 << 1;
    pub const SHOVE: u8 = 1 << 2;
    pub const DASH: u8 = 1 << 3;
    const KNOWN: u8 = Self::MOVE | Self::JUMP | Self::SHOVE | Self::DASH;

    pub fn new(buttons: u8, aim: u8) -> Self {
        Input {
            buttons: buttons & Self::KNOWN,
            aim,
        }
    }

    pub fn held(self, button: u8) -> bool {
        self.buttons & button != 0
    }

    pub fn to_bytes(self) -> [u8; 2] {
        [self.buttons, self.aim]
    }

    /// Unknown button bits are dropped so a hostile or newer client cannot
    /// put two machines into different states.
    pub fn from_bytes(bytes: [u8; 2]) -> Self {
        Input::new(bytes[0], bytes[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_valid_input() {
        for buttons in 0..=Input::KNOWN {
            for aim in [0, 1, 127, 128, 255] {
                let input = Input::new(buttons, aim);
                assert_eq!(Input::from_bytes(input.to_bytes()), input);
            }
        }
    }

    #[test]
    fn unknown_bits_are_masked() {
        assert_eq!(Input::from_bytes([0xff, 9]).buttons, Input::KNOWN);
    }
}

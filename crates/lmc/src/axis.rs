//! Extended-axis protocol facts from MeerK40t d511944 balormk/controller.py and
//! galvo_commands.py, and galvoplotter fc35136 galvo/controller.py (MIT).
//! D12 is UNSIGNED and unverified on the JPT board. No homing or stop-axis opcode is used.
use crate::{Single, op};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisMove {
    pub target_pulses: i32,
    pub min_speed: u16,
    pub max_speed: u16,
    pub accel_ms: u16,
}

impl AxisMove {
    pub fn valid(&self) -> bool {
        self.target_pulses != i32::MIN && self.min_speed > 0 && self.max_speed >= self.min_speed && (1..=10000).contains(&self.accel_ms)
    }
}

/// Sign-magnitude position. i32::MIN has no wire representation; callers must refuse it.
/// The pure encoding still preserves its sign and low 31 magnitude bits without overflowing.
pub fn axis_singles(m: &AxisMove) -> [Single; 3] {
    let magnitude = m.target_pulses.unsigned_abs();
    let word = magnitude | if m.target_pulses < 0 { 0x8000_0000 } else { 0 };
    let [a, b, c, d] = word.to_le_bytes();
    [
        Single([op::SET_AXIS_MOTION_PARAM, m.min_speed, m.max_speed, 0, 0, 0]),
        Single([op::SET_AXIS_ORIGIN_PARAM, m.accel_ms, 0, 0, 0, 0]),
        Single([op::MOVE_AXIS_TO, u16::from_le_bytes([a, b]), u16::from_le_bytes([c, d]), 0, 0, 0]),
    ]
}

pub fn axis_position(reply: [u16; 4]) -> i32 {
    let [_, lo, hi, _] = reply;
    let word = u32::from(lo) | (u32::from(hi) << 16);
    let magnitude = i32::try_from(word & 0x7fff_ffff).unwrap_or(i32::MAX);
    if word & 0x8000_0000 != 0 { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn singles_have_exact_little_endian_sign_magnitude_bytes() {
        for (target, bytes) in [(0, [0, 0, 0, 0]), (1, [1, 0, 0, 0]), (-1, [1, 0, 0, 128]), (i32::MAX, [255, 255, 255, 127]), (-i32::MAX, [255, 255, 255, 255])]
        {
            let m = AxisMove { target_pulses: target, min_speed: 0x1234, max_speed: 0x5678, accel_ms: 100 };
            let [a, b, c] = axis_singles(&m);
            assert_eq!(a.bytes(), [0x26, 0, 0x34, 0x12, 0x78, 0x56, 0, 0, 0, 0, 0, 0]);
            assert_eq!(b.bytes(), [0x27, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert_eq!(c.bytes(), [0x29, 0, bytes[0], bytes[1], bytes[2], bytes[3], 0, 0, 0, 0, 0, 0]);
            assert_eq!(axis_position([0, c.0[1], c.0[2], 0]), target);
        }
        assert!(!AxisMove { target_pulses: i32::MIN, min_speed: 1, max_speed: 1, accel_ms: 1 }.valid());
    }
}

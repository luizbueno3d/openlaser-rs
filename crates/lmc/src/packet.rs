//! Single commands, list packets and status words.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::op;

pub const PACKET_LEN: usize = 3072;
pub const SLOTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Single(pub [u16; 6]);

impl Single {
    /// Six little-endian words.
    pub fn bytes(&self) -> [u8; 12] {
        let mut out = [0u8; 12];
        let (chunks, _) = out.as_chunks_mut::<2>();
        for (chunk, word) in chunks.iter_mut().zip(self.0) {
            *chunk = word.to_le_bytes();
        }
        out
    }

    /// Every single command except WriteCorLine has an 8-byte reply.
    pub fn reads_reply(&self) -> bool {
        self.0.first().copied() != Some(op::WRITE_COR_LINE)
    }
}

/// Manual Debug: sha256 prefix + real command count.
#[derive(Clone, PartialEq, Eq)]
pub struct Packet(Box<[u8; PACKET_LEN]>);

impl Packet {
    /// Builds a padded packet. At most 256 words are accepted; unused slots
    /// are `EndOfList`. This is also the primitive used by both typed list
    /// builders, so partial/truncated packets cannot be produced.
    pub fn from_words(words: &[[u16; 6]]) -> Result<Self, &'static str> {
        if words.len() > SLOTS {
            return Err("an LMC packet has at most 256 slots");
        }
        let mut bytes = Box::new([0u8; PACKET_LEN]);
        for (slot, word) in bytes.as_chunks_mut::<12>().0.iter_mut().zip(std::iter::repeat([op::LIST_END_OF_LIST, 0, 0, 0, 0, 0])) {
            for (pair, value) in slot.as_chunks_mut::<2>().0.iter_mut().zip(word) {
                *pair = value.to_le_bytes();
            }
        }
        for (slot, word) in bytes.as_chunks_mut::<12>().0.iter_mut().zip(words.iter().copied()) {
            for (pair, value) in slot.as_chunks_mut::<2>().0.iter_mut().zip(word) {
                *pair = value.to_le_bytes();
            }
        }
        Ok(Packet(bytes))
    }

    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        let array = <[u8; PACKET_LEN]>::try_from(bytes).map_err(|_| "an LMC packet must be exactly 3072 bytes")?;
        Ok(Packet(Box::new(array)))
    }

    pub fn bytes(&self) -> &[u8; PACKET_LEN] {
        &self.0
    }

    pub fn words(&self) -> Vec<[u16; 6]> {
        self.0
            .as_chunks::<12>()
            .0
            .iter()
            .map(|slot| {
                let mut out = [0u16; 6];
                for (value, pair) in out.iter_mut().zip(slot.as_chunks::<2>().0) {
                    *value = u16::from_le_bytes(*pair);
                }
                out
            })
            .collect()
    }

    pub fn real_commands(&self) -> usize {
        self.words().iter().position(|word| word.first().copied() == Some(op::LIST_END_OF_LIST)).unwrap_or(SLOTS)
    }

    /// SHA-256 of the full wire packet, used to identify fixture and simulator packets.
    pub fn sha256_hex(&self) -> String {
        Sha256::digest(self.0.as_slice()).iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digest = self.sha256_hex();
        let prefix = digest.get(..8).unwrap_or("????????");
        f.debug_struct("Packet").field("sha256", &prefix).field("commands", &self.real_commands()).finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusWord(pub u16);

impl StatusWord {
    pub const BUSY: u16 = 0x04;
    pub const READY: u16 = 0x20;
    pub const AXIS: u16 = 0x40;

    pub fn busy(self) -> bool {
        self.0 & Self::BUSY != 0
    }

    pub fn ready(self) -> bool {
        self.0 & Self::READY != 0
    }
}

/// Word 3; unknown bits kept (one board idles at 0x0220).
pub fn status_from_reply(reply: [u16; 4]) -> StatusWord {
    let [_, _, _, status] = reply;
    StatusWord(status)
}

/// R05 §1.4 verbatim: GetSerialNo … EnableZ, then a 50 ms pause (the session sleeps; not in the list).
/// Initialization command order from R05 §1.4; the session supplies the final 50 ms pause.
pub fn init_sequence() -> Vec<Single> {
    vec![
        Single([op::GET_SERIAL_NO, 0, 0, 0, 0, 0]),
        Single([op::GET_VERSION, 0, 0, 0, 0, 0]),
        Single([op::RESET, 0, 0, 0, 0, 0]),
        Single([op::WRITE_COR_TABLE, 0, 0, 0, 0, 0]),
        Single([op::ENABLE_LASER, 0, 0, 0, 0, 0]),
        Single([op::SET_CONTROL_MODE, 0, 0, 0, 0, 0]),
        Single([op::SET_LASER_MODE, 1, 0, 0, 0, 0]),
        Single([op::SET_DELAY_MODE, 1, 0, 0, 0, 0]),
        Single([op::SET_TIMING, 1, 0, 0, 0, 0]),
        Single([op::SET_STANDBY, 2000, 20, 0, 0, 0]),
        Single([op::SET_FIRST_PULSE_KILLER, 200, 0, 0, 0, 0]),
        Single([op::SET_PWM_HALF_PERIOD, 125, 0, 0, 0, 0]),
        Single([op::SET_PWM_PULSE_WIDTH, 125, 0, 0, 0, 0]),
        Single([op::FIBER_SET_MO, 0, 0, 0, 0, 0]),
        Single([op::SET_FPK_PARAM_2, 0x0ffb, 1, 409, 100, 0]),
        Single([op::SET_FLY_RES, 0, 99, 1000, 25, 0]),
        Single([op::ENABLE_Z, 0, 0, 0, 0, 0]),
        Single([op::WRITE_ANALOG_PORT_1, 0x07ff, 0, 0, 0, 0]),
        Single([op::ENABLE_Z, 0, 0, 0, 0, 0]),
    ]
}

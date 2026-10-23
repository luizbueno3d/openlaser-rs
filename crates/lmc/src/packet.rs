//! Single commands, list packets and status words.

use std::fmt;

use sha2::{Digest, Sha256};

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

    /// Every single except WriteCorLine. WP0 stub: true.
    pub fn reads_reply(&self) -> bool {
        true
    }
}

/// Manual Debug: sha256 prefix + real command count.
#[derive(Clone, PartialEq, Eq)]
pub struct Packet(Box<[u8; PACKET_LEN]>);

impl Packet {
    pub fn bytes(&self) -> &[u8; PACKET_LEN] {
        &self.0
    }

    /// WP0 stub: empty.
    pub fn words(&self) -> Vec<[u16; 6]> {
        Vec::new()
    }

    /// WP0 stub: 0.
    pub fn real_commands(&self) -> usize {
        0
    }
}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digest = Sha256::digest(self.0.as_slice());
        let prefix: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
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
/// WP0 stub: empty.
pub fn init_sequence() -> Vec<Single> {
    Vec::new()
}

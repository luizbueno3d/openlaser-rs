//! GP's sequencing (SPEC §5.5): one controller per job, per-job init, parameter cache, elided first jump, per-job
//! abort with the dummy packet, close (USB reset). Must reproduce tests/golden/t1_lmc/v0/*.json exactly.
//! Test-only (feature `py-compat`). Kept deliberately separate from shipping encoding.

use crate::Error;
use crate::op;
use crate::packet::{Packet, Single, init_sequence};

#[derive(Debug, Clone, PartialEq)]
pub enum WireEvent {
    Open,
    Single { hex: String },
    Read { status: u16 },
    List { sha256: String, n_cmds: u16, bytes: Packet },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OmParams {
    pub power: f64,
    pub frequency_khz: f64,
    pub pulse_width_ns: f64,
    pub speed_mm_s: f64,
}

pub fn mark_polylines(polys: &[Vec<[f64; 2]>], p: &OmParams, field_mm: f64) -> Result<Vec<WireEvent>, Error> {
    let _ = (polys, p, field_mm);
    Err(Error::NotImplemented("openlaser_lmc::py_compat::mark_polylines"))
}

pub fn frame_polylines(polys: &[Vec<[f64; 2]>], field_mm: f64) -> Result<Vec<WireEvent>, Error> {
    let _ = (polys, field_mm);
    Err(Error::NotImplemented("openlaser_lmc::py_compat::frame_polylines"))
}

pub fn init_abort() -> Vec<WireEvent> {
    let mut events = vec![WireEvent::Open];
    for cmd in init_sequence() {
        add_single(&mut events, cmd);
    }
    for cmd in [
        Single([op::STOP_EXECUTE, 0, 0, 0, 0, 0]),
        Single([op::FIBER_SET_MO, 0, 0, 0, 0, 0]),
        Single([op::RESET_LIST, 0, 0, 0, 0, 0]),
        Single([op::GET_VERSION, 0, 0, 0, 0, 0]),
    ] {
        add_single(&mut events, cmd);
    }
    if let Ok(packet) = Packet::from_words(&[]) {
        events.push(WireEvent::List { sha256: packet.sha256_hex(), n_cmds: 0, bytes: packet });
    }
    for cmd in [
        Single([op::SET_END_OF_LIST, 0, 0, 0, 0, 0]),
        Single([op::EXECUTE_LIST, 0, 0, 0, 0, 0]),
        Single([op::FIBER_SET_MO, 0, 0, 0, 0, 0]),
        Single([op::WRITE_PORT, 0, 0, 0, 0, 0]),
    ] {
        add_single(&mut events, cmd);
    }
    events.push(WireEvent::Close);
    events
}

fn add_single(events: &mut Vec<WireEvent>, cmd: Single) {
    let hex = cmd.bytes().iter().map(|b| format!("{b:02x}")).collect();
    events.push(WireEvent::Single { hex });
    if cmd.reads_reply() {
        events.push(WireEvent::Read { status: 0x20 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_testkit::t1::{Event, load};

    #[test]
    fn init_abort_matches_v0_fixture() {
        let fixture = load("v0", "init_abort").unwrap_or_else(|e| panic!("{e}"));
        let expected: Vec<_> = fixture
            .events
            .into_iter()
            .map(|e| match e {
                Event::Open => WireEvent::Open,
                Event::Close => WireEvent::Close,
                Event::Single { hex, .. } => WireEvent::Single { hex },
                Event::Read { status } => WireEvent::Read { status },
                Event::List { sha256, n_cmds } => WireEvent::List { sha256, n_cmds, bytes: Packet::from_words(&[]).unwrap_or_else(|e| panic!("{e}")) },
            })
            .collect();
        assert_eq!(init_abort(), expected);
    }
}

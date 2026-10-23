//! GP's sequencing (SPEC §5.5): one controller per job, per-job init, parameter cache, elided first jump, per-job
//! abort with the dummy packet, close (USB reset). Must reproduce tests/golden/t1_lmc/v0/*.json exactly.
//! Test-only (feature `py-compat`). WP0 stubs.

use crate::Error;
use crate::packet::Packet;

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
    Vec::new()
}

//! GRBL G-code (SPEC §3.11; owned by WP2). WP0 stubs.
#![forbid(unsafe_code)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use std::time::Duration;

pub mod om;

pub use openlaser_machine::GantryMachine;
pub use openlaser_safety::{EmitPlan, FramePlan};
pub use openlaser_types::{Fingerprint, MachineId};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GcodeProgram {
    pub text: String,
    pub lines: u32,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub distance_mm: f64,
    pub estimate: Duration,
}

impl GcodeProgram {
    /// blake3 of `text` (S22).
    pub fn bytes_hash(&self) -> Fingerprint {
        Fingerprint(*blake3::hash(self.text.as_bytes()).as_bytes())
    }
}

/// (sealed) A laser-off frame program: built only by `frame_program`, so GRBL's `Backend::Frame` cannot be handed a
/// burning program (the LMC side gets the same guarantee from `LightingList`).
#[derive(Debug, Clone, PartialEq)]
pub struct GcodeFrame {
    program: GcodeProgram,
}

impl GcodeFrame {
    pub fn program(&self) -> &GcodeProgram {
        &self.program
    }
}

/// Tests pass "<MASKED>".
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub machine_name: String,
    pub generated: String,
}

/// Shipping encoder (B11: only the gantry block is read). Overspeed never reaches here: safety refuses it.
pub fn compile(emit: &EmitPlan, m: &GantryMachine, h: &Header) -> Result<GcodeProgram, Error> {
    let _ = (emit, m, h);
    Err(Error::NotImplemented("openlaser_gcode::compile"))
}

/// Laser-off frame around the job (G0 only, no M3/M4/S>0; a test greps the text).
pub fn frame_program(frame: &FramePlan, m: &GantryMachine, h: &Header) -> Result<GcodeFrame, Error> {
    let _ = (frame, m, h);
    Err(Error::NotImplemented("openlaser_gcode::frame_program"))
}

/// OM `_number`: format!("{:.3}") then trim zeros and dot; "-0" → "0". T2 number_format (100,019 rows).
/// WP0 stub: empty.
pub fn number(v: f64) -> String {
    let _ = v;
    String::new()
}

//! Decoding, disassembly and the emission summary from bytes. WP0 stubs.

use openlaser_machine::GalvoMachine;
use openlaser_plan::JobPlan;
use openlaser_types::{EmissionSummary, EmittedSegment, Notice};

use crate::Error;
use crate::encode::{LmcFrameJob, LmcJob};
use crate::packet::Packet;

#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    ReadyMark,
    DelayTime(u16),
    WritePort(u16),
    JumpSpeed(u16),
    MarkSpeed(u16),
    PulseWidth(u16),
    MarkCurrent(u16),
    QSwitchPeriod(u16),
    LaserOnDelay(u16, u16),
    LaserOffDelay(u16, u16),
    PolygonDelay(u16, u16),
    JumpDelay(u16, u16),
    JumpTo { x: u16, y: u16, dist: u16 },
    MarkTo { x: u16, y: u16, dist: u16 },
    EndOfList,
    Other([u16; 6]),
}

/// Stops at the first 0x8002 fill run.
pub fn decode_packet(p: &Packet) -> Vec<Decoded> {
    let _ = p;
    Vec::new()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListingLine {
    pub list: Option<u32>,
    pub packet: Option<u32>,
    pub slot: Option<u16>,
    pub words: [u16; 6],
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Listing {
    pub lines: Vec<ListingLine>,
}

/// e.g. "MarkTo x=34152 y=31384 (5.000, −5.000 mm) dist=2768"
pub fn disassemble(job: &LmcJob, m: &GalvoMachine) -> Listing {
    let _ = (job, m);
    Listing::default()
}

pub fn disassemble_frame(job: &LmcFrameJob, m: &GalvoMachine) -> Listing {
    let _ = (job, m);
    Listing::default()
}

/// Everything numeric comes from decoding `job`'s packets; `plan` supplies only names, notices and repeats.
pub fn emission_summary(job: &LmcJob, m: &GalvoMachine, plan: &JobPlan, notices: &[Notice]) -> Result<EmissionSummary, Error> {
    let _ = (job, m, plan, notices);
    Err(Error::NotImplemented("openlaser_lmc::emission_summary"))
}

pub fn emitted_segments(job: &LmcJob, m: &GalvoMachine) -> Vec<EmittedSegment> {
    let _ = (job, m);
    Vec::new()
}

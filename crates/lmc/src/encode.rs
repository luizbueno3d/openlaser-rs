//! Job and frame encoders (§5.2). WP0 stubs.

use std::time::Duration;

use openlaser_machine::GalvoMachine;
use openlaser_safety::{EmitPlan, FramePlan};
use openlaser_types::{Fingerprint, FrameSource, FrameStyle, LayerKey, MachineId};

use crate::Error;
use crate::packet::{Packet, Single};

/// A burn, ready for the driver. One list per layer-pass (PLAN §3.3), each list packetised into 3072-byte packets
/// padded with 0x8002. Every byte is fixed at encode time: nothing in a packet depends on session state (the first
/// JumpTo's `dist` of every list is computed from (0x8000, 0x8000), §5.2 rule 6), so the bytes the summary was
/// decoded from are the bytes sent, and `bytes_hash` is stable.
#[derive(Debug, Clone, PartialEq)]
pub struct LmcJob {
    /// Sent before EACH list: ResetList, WritePort(0x0001), Fiber_SetMo(1), GetVersion.
    pub prelude: Vec<Single>,
    /// Per layer-pass, in burn order (one repeat).
    pub lists: Vec<LmcList>,
    /// Run by the driver, stop checked between repeats; never expanded.
    pub repeats: u16,
    /// After EACH list, only once !BUSY: Fiber_SetMo(0), WritePort(0), GetMarkTime(3).
    pub postlude: Vec<Single>,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
}

impl LmcJob {
    /// S22: the `EmissionSummary.bytes_hash` definition (blake3 over every single command and packet of one repeat in
    /// send order, then the repeat count as u16 LE). WP0 stub: all zeros (WP4 implements it).
    pub fn bytes_hash(&self) -> Fingerprint {
        Fingerprint([0; 32])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LmcList {
    pub layer: LayerKey,
    pub pass: u16,
    pub packets: Vec<Packet>,
    pub packet_est: Vec<Duration>,
}

/// A red-light frame. `once`: ONE list with the OM Frame Once pass count baked in (≥ 2 s of tracing); `pass`: ONE
/// complete list (ReadyMark … EndOfList) holding exactly one pass, which the driver submits as a fresh list per Loop
/// pass (S20). Neither can contain MarkTo, LaserOnPoint or MO open (type-level). `once_passes` is the baked count
/// the receipt reports.
#[derive(Debug, Clone, PartialEq)]
pub struct LmcFrameJob {
    pub once: Vec<Packet>,
    pub once_passes: u32,
    pub once_est: Duration,
    pub pass: Vec<Packet>,
    pub pass_est: Duration,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub style: FrameStyle,
    pub source: FrameSource,
}

pub fn encode_job(emit: &EmitPlan, m: &GalvoMachine) -> Result<LmcJob, Error> {
    let _ = (emit, m);
    Err(Error::NotImplemented("openlaser_lmc::encode_job"))
}

pub fn encode_frame(frame: &FramePlan, m: &GalvoMachine) -> Result<LmcFrameJob, Error> {
    let _ = (frame, m);
    Err(Error::NotImplemented("openlaser_lmc::encode_frame"))
}

//! The fake LMC board and its emission recorder.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_lmc::{LmcJob, LmcTransport, Packet, Single, XferError};
use openlaser_machine::GalvoMachine;
use openlaser_types::{Clock, EmittedSegment, MonoTime};

#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    /// Packets the board queues before READY drops; swept 1..=8 until H2.
    pub buffer_depth: u8,
    /// 0 (virtual) or 2 ms (real-time T6 variant).
    pub transfer_latency: Duration,
    /// e.g. 0x0200 (an idle 0x0220 board).
    pub status_extra_bits: u16,
    /// 0x9899; 0x9980 = clone board (refused by detect/open).
    pub pid: u16,
    pub faults: Vec<Fault>,
    /// S23 stand-in for tests of the bench cap; None by default.
    pub emission_cap_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    StallWrites {
        after_packets: u32,
    },
    ReadyNeverReturns {
        after_execute: bool,
    },
    BusyStuck,
    DisconnectMidPacket {
        packet: u32,
    },
    /// A MODEL until H23 measures the board.
    PartialPacketAfterCancel,
    ReplyTimeouts {
        every: u32,
    },
}

/// Board model behind `LmcTransport`: READY/BUSY from the galvo time model, MO and port state, list execution,
/// and an emission recorder that decodes the wire independently (it uses only `lmc::op` constants, not
/// `lmc::decode_packet`, so it is a second opinion on the encoder).
pub struct FakeLmc {
    cfg: SimConfig,
    clock: Arc<dyn Clock>,
    recorder: Arc<Recorder>,
    hook: Mutex<Option<WireHook>>,
}

/// See [`FakeLmc::set_hook`].
pub type WireHook = Box<dyn FnMut(&SimWire, MonoTime) + Send>;

impl FakeLmc {
    /// `Arc<dyn Clock>`: `VirtualClock` in T6, `SystemClock` behind `SimProvider` (the GUI's Simulator machine).
    pub fn new(cfg: SimConfig, clock: Arc<dyn Clock>) -> Self {
        FakeLmc { cfg, clock, recorder: Arc::new(Recorder::default()), hook: Mutex::new(None) }
    }

    /// Shared; readable while the driver owns the FakeLmc.
    pub fn recorder(&self) -> Arc<Recorder> {
        Arc::clone(&self.recorder)
    }

    /// Called on the driver thread for every wire event, before the board model reacts. T6 uses it to trip a
    /// `Stopper` at an exact wire event or virtual tick, so "stop at a random tick" is seeded and deterministic
    /// (a stop from another thread would land at a scheduler-dependent virtual time).
    pub fn set_hook(&self, hook: WireHook) {
        *self.hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }
}

/// WP0 stub: every transfer is refused.
impl LmcTransport for FakeLmc {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        let _ = (cmd, &self.cfg, self.clock.now(), &self.hook);
        Err(XferError::NotImplemented("openlaser_lmc_sim::FakeLmc::submit_single"))
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        let _ = slice;
        Err(XferError::NotImplemented("openlaser_lmc_sim::FakeLmc::poll_single"))
    }

    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        let _ = pkt;
        Err(XferError::NotImplemented("openlaser_lmc_sim::FakeLmc::submit_packet"))
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        let _ = slice;
        Err(XferError::NotImplemented("openlaser_lmc_sim::FakeLmc::poll_packet"))
    }

    fn cancel_all(&mut self) {}

    fn drain_replies(&mut self) {}
}

/// One wire event with its virtual timestamp (for T6 ordering asserts).
#[derive(Debug, Clone, PartialEq)]
pub enum SimWire {
    Single { at: MonoTime, words: [u16; 6] },
    Packet { at: MonoTime, sha256: String },
    Read { at: MonoTime, status: u16 },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recording {
    /// single/packet/read with virtual timestamps (for T6 ordering asserts).
    pub wire: Vec<SimWire>,
    /// MarkTo while executing, MO open, port bit 0 set.
    pub burned: Vec<EmittedSegment>,
    /// Red-light traces (port bit 8 set, JumpTo).
    pub lit: Vec<[[f64; 2]; 2]>,
    pub mo_open: Vec<(MonoTime, MonoTime)>,
    /// Must stay 0.
    pub writes_while_not_ready: u32,
    /// Must stay 0 unless stopping.
    pub mo_closed_while_busy: u32,
}

#[derive(Debug, Default)]
pub struct Recorder {
    recording: Mutex<Recording>,
}

impl Recorder {
    pub fn snapshot(&self) -> Recording {
        self.recording.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Offline run of an encoded job (no driver thread): what `job_simulate` and `openlaser-cli simulate` show.
/// WP0 stub: an empty recording.
pub fn simulate_job(job: &LmcJob, m: &GalvoMachine) -> Recording {
    let _ = (job, m);
    Recording::default()
}

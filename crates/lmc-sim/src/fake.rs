//! The fake LMC board and its emission recorder.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_lmc::{LmcJob, LmcTransport, Packet, Single, XferError, op};
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
    single_pending: Option<bool>,
    packet_pending_until: Option<MonoTime>,
    packet_count: u32,
    executing: bool,
    mo: bool,
    port: u16,
    position: (u16, u16),
    ready_lost: bool,
    mark_current: u16,
    field_mm: [f64; 2],
}

/// See [`FakeLmc::set_hook`].
pub type WireHook = Box<dyn FnMut(&SimWire, MonoTime) + Send>;

impl FakeLmc {
    /// `Arc<dyn Clock>`: `VirtualClock` in T6, `SystemClock` behind `SimProvider` (the GUI's Simulator machine).
    pub fn new(cfg: SimConfig, clock: Arc<dyn Clock>) -> Self {
        FakeLmc {
            cfg,
            clock,
            recorder: Arc::new(Recorder::default()),
            hook: Mutex::new(None),
            single_pending: None,
            packet_pending_until: None,
            packet_count: 0,
            executing: false,
            mo: false,
            port: 0,
            position: (0x8000, 0x8000),
            ready_lost: false,
            mark_current: 0,
            field_mm: [236.68, 236.68],
        }
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

    pub(crate) fn set_field_mm(&mut self, field_mm: [f64; 2]) {
        self.field_mm = field_mm;
    }
}

impl LmcTransport for FakeLmc {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        let at = self.clock.now();
        self.notify(SimWire::Single { at, words: cmd.0 });
        if self.packet_pending_until.is_some() {
            self.note_not_ready_write();
        }
        if cmd.0[0] == op::FIBER_SET_MO {
            self.mo = cmd.0[1] != 0;
        }
        if cmd.0[0] == op::WRITE_PORT {
            self.port = cmd.0[1];
        }
        if cmd.0[0] == op::EXECUTE_LIST {
            self.executing = true;
            self.ready_lost = false;
        }
        if cmd.0[0] == op::STOP_EXECUTE {
            self.executing = false;
        }
        if cmd.0[0] == op::RESET_LIST {
            self.executing = false;
        }
        self.single_pending = Some(cmd.reads_reply());
        Ok(())
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        self.clock.sleep(slice);
        let Some(reply) = self.single_pending.take() else { return Ok(None) };
        if !reply {
            return Ok(Some(None));
        }
        let status = self.status();
        self.notify(SimWire::Read { at: self.clock.now(), status });
        Ok(Some(Some([0, 0, 0, status])))
    }

    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        if self.packet_pending_until.is_some() {
            self.note_not_ready_write();
        }
        self.packet_count = self.packet_count.saturating_add(1);
        if self.cfg.faults.iter().any(|f| matches!(f,Fault::StallWrites{after_packets} if self.packet_count>*after_packets)) {
            return Err(XferError::Stall);
        }
        if self.cfg.faults.iter().any(|f| matches!(f,Fault::DisconnectMidPacket{packet} if *packet==self.packet_count)) {
            return Err(XferError::Disconnected);
        }
        let at = self.clock.now();
        let digest = packet_sha256(pkt);
        self.notify(SimWire::Packet { at, sha256: digest });
        self.record_packet(pkt);
        self.packet_pending_until = Some(at.plus(self.cfg.transfer_latency));
        if self.cfg.transfer_latency.is_zero() {
            self.packet_pending_until = None;
        }
        Ok(())
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        self.clock.sleep(slice);
        let Some(until) = self.packet_pending_until else { return Ok(Some(())) };
        if self.clock.now() >= until {
            self.packet_pending_until = None;
            Ok(Some(()))
        } else {
            Ok(None)
        }
    }

    fn cancel_all(&mut self) {
        self.packet_pending_until = None;
        self.single_pending = None;
    }

    fn drain_replies(&mut self) {
        self.single_pending = None;
    }
}

impl FakeLmc {
    fn status(&self) -> u16 {
        if self.cfg.faults.iter().any(|f| matches!(f, Fault::BusyStuck))
            || (self.cfg.faults.iter().any(|f| matches!(f, Fault::ReadyNeverReturns { after_execute: true })) && self.executing)
        {
            return self.cfg.status_extra_bits | openlaser_lmc::StatusWord::BUSY;
        }
        if self.packet_pending_until.is_some() || (self.executing && self.ready_lost) {
            self.cfg.status_extra_bits | openlaser_lmc::StatusWord::BUSY
        } else {
            self.cfg.status_extra_bits | openlaser_lmc::StatusWord::READY
        }
    }
    fn notify(&self, event: SimWire) {
        if let Some(hook) = self.hook.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            hook(&event, self.clock.now());
        }
        self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner()).wire.push(event);
    }
    fn note_not_ready_write(&self) {
        let mut r = self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner());
        r.writes_while_not_ready = r.writes_while_not_ready.saturating_add(1);
    }
    fn record_packet(&mut self, pkt: &Packet) {
        for w in pkt.words() {
            let Some(opcode) = w.first().copied() else { continue };
            if opcode == op::LIST_END_OF_LIST {
                break;
            }
            match opcode {
                op::LIST_WRITE_PORT => self.port = w[1],
                op::LIST_MARK_CURRENT => self.mark_current = w[1],
                op::LIST_FIBER_OPEN_MO => self.mo = w[1] != 0,
                op::LIST_JUMP_TO => {
                    let to = (w[1], w[2]);
                    if self.port & 0x0100 != 0 {
                        self.add_lit(self.position, to);
                    }
                    self.position = to;
                }
                op::LIST_MARK_TO => {
                    let to = (w[1], w[2]);
                    if self.mo && self.port & 1 != 0 {
                        self.add_burn(self.position, to);
                    }
                    self.position = to;
                }
                _ => {}
            }
        }
    }
    fn add_lit(&self, a: (u16, u16), b: (u16, u16)) {
        let cv = |p: (u16, u16)| [(f64::from(p.0) - 32768.0) * self.field_mm[0] / 65535.0, (f64::from(p.1) - 32768.0) * self.field_mm[1] / 65535.0];
        self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner()).lit.push([cv(a), cv(b)]);
    }
    fn add_burn(&self, a: (u16, u16), b: (u16, u16)) {
        let cv = |p: (u16, u16)| [(f64::from(p.0) - 32768.0) * self.field_mm[0] / 65535.0, (f64::from(p.1) - 32768.0) * self.field_mm[1] / 65535.0];
        self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner()).burned.push(EmittedSegment {
            a: cv(a),
            b: cv(b),
            power_pct: f64::from(self.mark_current) * 100.0 / 4095.0,
        });
    }
}

fn packet_sha256(packet: &Packet) -> String {
    packet.sha256_hex()
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
/// Replays the supplied prelude, packets and postlude against an isolated in-memory board.
pub fn simulate_job(job: &LmcJob, m: &GalvoMachine) -> Recording {
    let sim = FakeLmc::new(
        SimConfig { buffer_depth: 1, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid: 0x9899, faults: Vec::new(), emission_cap_pct: None },
        Arc::new(openlaser_types::VirtualClock::new()),
    );
    let mut board = sim;
    board.set_field_mm(m.field_f());
    for cmd in &job.prelude {
        if board.submit_single(*cmd).is_err() {
            return board.recorder().snapshot();
        }
    }
    for list in &job.lists {
        for packet in &list.packets {
            if board.submit_packet(packet).is_err() {
                return board.recorder().snapshot();
            }
        }
    }
    for cmd in &job.postlude {
        if board.submit_single(*cmd).is_err() {
            return board.recorder().snapshot();
        }
    }
    let record = board.recorder().snapshot();
    let _ = m;
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FakeLmc;
    use openlaser_lmc::op;
    use openlaser_types::VirtualClock;

    fn config(latency: Duration) -> SimConfig {
        SimConfig { buffer_depth: 1, transfer_latency: latency, status_extra_bits: 0x0200, pid: 0x9899, faults: Vec::new(), emission_cap_pct: None }
    }

    #[test]
    fn fake_records_mark_box10_emission() {
        let mut board = FakeLmc::new(config(Duration::ZERO), Arc::new(VirtualClock::new()));
        assert!(board.submit_single(Single([op::FIBER_SET_MO, 1, 0, 0, 0, 0])).is_ok());
        assert!(board.submit_single(Single([op::WRITE_PORT, 1, 0, 0, 0, 0])).is_ok());
        let words = [
            [op::LIST_MARK_CURRENT, 2048, 0, 0, 0, 0],
            [op::LIST_MARK_TO, 34152, 31384, 0, 2768, 0],
            [op::LIST_MARK_TO, 34152, 34152, 0, 2768, 0],
            [op::LIST_MARK_TO, 31384, 34152, 0, 2768, 0],
            [op::LIST_MARK_TO, 31384, 31384, 0, 2768, 0],
        ];
        let packet = Packet::from_words(&words).unwrap_or_else(|e| panic!("{e}"));
        assert!(board.submit_packet(&packet).is_ok());
        assert_eq!(board.recorder().snapshot().burned.len(), 4);
    }

    #[test]
    fn fake_frame_records_zero_emission() {
        let mut board = FakeLmc::new(config(Duration::ZERO), Arc::new(VirtualClock::new()));
        assert!(board.submit_single(Single([op::WRITE_PORT, 0x0100, 0, 0, 0, 0])).is_ok());
        let packet = Packet::from_words(&[
            [op::LIST_JUMP_TO, 33000, 33000, 0, 0, 0],
            [op::LIST_WRITE_PORT, 0x0100, 0, 0, 0, 0],
            [op::LIST_JUMP_TO, 34000, 33000, 0, 1000, 0],
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(board.submit_packet(&packet).is_ok());
        let r = board.recorder().snapshot();
        assert!(r.burned.is_empty());
        assert!(!r.lit.is_empty());
    }

    #[test]
    fn fake_counts_writes_while_not_ready() {
        let mut board = FakeLmc::new(config(Duration::from_millis(2)), Arc::new(VirtualClock::new()));
        let packet = Packet::from_words(&[[op::LIST_READY_MARK, 0, 0, 0, 0, 0]]).unwrap_or_else(|e| panic!("{e}"));
        assert!(board.submit_packet(&packet).is_ok());
        assert!(board.submit_packet(&packet).is_ok());
        assert_eq!(board.recorder().snapshot().writes_while_not_ready, 1);
    }
}

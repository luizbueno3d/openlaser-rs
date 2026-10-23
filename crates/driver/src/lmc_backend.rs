//! The LMC backend (SPEC §3.13, §5.3; red zone): `LmcJob`/`LmcFrameJob` mapped onto the session of
//! [`crate::lmc_session`], over the `LmcTransport` the provider opened.

use std::time::Duration;

use openlaser_lmc::{LmcFrameJob, LmcJob, LmcTransport, Packet, Single, XferError, init_sequence};
use openlaser_machine::{DeviceKind, GalvoMachine};
use openlaser_types::{Fingerprint, FrameMode, FrameOrigin, FrameReceipt, MachineId, RefusalCode, SessionId};

use crate::backend::{Backend, Capabilities, DeviceInfo, DriverError, DriverEvent, EncodedJob, FrameOutcome, MachineState, MachineStatus, Outcome};
use crate::lmc_ops;
use crate::lmc_session::{FrameSpec, Halt, JobSpec, Progress, Session, SessionCfg, Wire};
use crate::stop::{Cx, StopReport};

/// `LmcTransport` seen through the session's packet-generic seam.
pub(crate) struct TransportWire<T>(pub(crate) T);

impl<T: LmcTransport> Wire for TransportWire<T> {
    type Pkt = Packet;

    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        self.0.submit_single(cmd)
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        self.0.poll_single(slice)
    }

    fn submit_packet(&mut self, p: &Packet) -> Result<(), XferError> {
        self.0.submit_packet(p)
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        self.0.poll_packet(slice)
    }

    fn cancel_all(&mut self) {
        self.0.cancel_all();
    }

    fn drain_replies(&mut self) {
        self.0.drain_replies();
    }

    fn describe_packet(p: &Packet) -> String {
        format!("{p:?}")
    }
}

/// The abort's dummy end-of-list packet (256 × `0x8002`, sha256 `e479558e…6a18`). `openlaser_lmc::Packet` has no
/// public constructor in the WP0 contract, so the driver cannot build it: until the protocol crate exports one
/// (REQUEST at I1), the emergency stop skips the dummy packet and says so in the `StopReport` notes. Skipping it
/// is the conservative side of §5.3 (it is only ever written when READY, for OM parity until H14).
fn end_of_list_packet() -> Option<Packet> {
    None
}

/// LMC session (red zone): §5.
pub struct LmcBackend<T: LmcTransport> {
    session: Session<TransportWire<T>>,
    machine: GalvoMachine,
    cfg_error: Option<String>,
    provider: &'static str,
    connected: bool,
}

impl<T: LmcTransport> LmcBackend<T> {
    pub fn new(transport: T, machine: GalvoMachine) -> Self {
        let (cfg, cfg_error) = match SessionCfg::from_machine(&machine) {
            Ok(cfg) => (cfg, None),
            Err(e) => (SessionCfg::fallback(), Some(e)),
        };
        LmcBackend { session: Session::new(TransportWire(transport), cfg, end_of_list_packet()), machine, cfg_error, provider: "lmc", connected: false }
    }

    /// The provider name `DeviceInfo` reports ("usb", "sim"); the engine knows it, the transport does not.
    pub fn with_provider_name(mut self, name: &'static str) -> Self {
        self.provider = name;
        self
    }

    fn laser_mask(&self) -> Result<u16, DriverError> {
        1u16.checked_shl(u32::from(self.machine.ports.laser_bit))
            .ok_or_else(|| DriverError::Protocol(format!("machine profile is invalid: laser port bit {} is not 0..=15", self.machine.ports.laser_bit)))
    }

    fn ready_for_work(&self) -> Result<(), DriverError> {
        if let Some(e) = &self.cfg_error {
            return Err(DriverError::refused(RefusalCode::MachineMismatch, format!("the machine profile cannot drive the board: {e}")));
        }
        if !self.connected {
            return Err(DriverError::refused(RefusalCode::NotConnected, "the board is not connected: connect first"));
        }
        Ok(())
    }
}

fn halt_to_result<O>(halt: Halt, stopped: impl FnOnce() -> O) -> Result<O, DriverError> {
    match halt {
        Halt::Stopped => Ok(stopped()),
        Halt::Fault(e) => Err(e),
    }
}

impl<T: LmcTransport + 'static> Backend for LmcBackend<T> {
    type Job = LmcJob;
    type Frame = LmcFrameJob;

    fn capabilities(&self) -> Capabilities {
        Capabilities { kind: DeviceKind::GalvoLmc, frame: true, run: true, pause: false, jog: false, home: false, inputs: false }
    }

    /// The init sequence verbatim (R05 §1.4, incl. Reset and WriteCorTable(0)), then the 50 ms pause. The board
    /// lock was taken by the provider when it opened the transport.
    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError> {
        if let Some(e) = &self.cfg_error {
            return Err(DriverError::refused(RefusalCode::MachineMismatch, format!("the machine profile cannot drive the board: {e}")));
        }
        self.connected = false;
        match self.session.connect(&init_sequence(), cx) {
            Ok(version) => {
                self.connected = true;
                let b = &self.machine.board;
                Ok(DeviceInfo { vid: b.vid, pid: b.pid, serial: b.serial.clone(), version, provider: self.provider.to_owned() })
            }
            Err(Halt::Stopped) => Err(DriverError::Protocol("connect was stopped before the init sequence finished".to_owned())),
            Err(Halt::Fault(e)) => Err(e),
        }
    }

    fn frame(&mut self, f: &LmcFrameJob, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError> {
        self.ready_for_work()?;
        let laser = self.laser_mask()?;
        let (packets, what) = match mode {
            FrameMode::Once => (&f.once, "Frame Once"),
            FrameMode::Loop => (&f.pass, "Frame Loop"),
        };
        if packets.is_empty() || (mode == FrameMode::Once && f.once_passes == 0) {
            return Err(DriverError::refused(RefusalCode::NothingToMark, format!("{what}: the frame job holds no pass to trace")));
        }
        for p in f.once.iter().chain(&f.pass) {
            lmc_ops::check_frame_packet(p.bytes(), laser).map_err(|e| DriverError::refused(RefusalCode::EmissionNotAllowed, e))?;
        }
        let spec = FrameSpec { once: &f.once, once_est: f.once_est, once_passes: f.once_passes, pass: &f.pass, pass_est: f.pass_est };
        // S20: called by the session only after the pass's list executed (!BUSY after it).
        let outcome = self.session.frame(&spec, mode == FrameMode::Once, cx, &mut |cx, passes| {
            let r = FrameReceipt {
                fingerprint: f.fingerprint,
                machine: f.machine.clone(),
                session,
                full_passes: passes,
                at: cx.now(),
                style: f.style,
                source: f.source,
                origin,
            };
            cx.emit(DriverEvent::FrameCompleted(r));
        });
        match outcome {
            Ok(passes) => Ok(FrameOutcome::Completed { passes }),
            Err(halt) => halt_to_result(halt, || FrameOutcome::Stopped(self.session.emergency(cx))),
        }
    }

    fn run(&mut self, job: &LmcJob, cx: &mut Cx) -> Result<Outcome, DriverError> {
        self.ready_for_work()?;
        if job.repeats == 0 || job.lists.is_empty() {
            return Err(DriverError::refused(RefusalCode::NothingToMark, "the job holds no list to mark"));
        }
        let mut packets: u32 = 0;
        let mut one_repeat = Duration::ZERO;
        for (i, l) in job.lists.iter().enumerate() {
            if l.packets.is_empty() || l.packet_est.len() != l.packets.len() {
                return Err(DriverError::Protocol(format!("list {} has {} packets and {} estimates", i + 1, l.packets.len(), l.packet_est.len())));
            }
            packets = packets.saturating_add(u32::try_from(l.packets.len()).unwrap_or(u32::MAX));
            one_repeat = one_repeat.saturating_add(l.packet_est.iter().sum());
        }
        let total = packets.saturating_mul(u32::from(job.repeats));
        let n = job.lists.len();
        let lists = job
            .lists
            .iter()
            .enumerate()
            .map(|(i, l)| (format!("list {}/{n} {} pass {}", i + 1, l.layer, l.pass), l.packets.as_slice(), l.packet_est.as_slice()))
            .collect();
        let spec = JobSpec { prelude: &job.prelude, lists, postlude: &job.postlude, repeats: job.repeats };
        let mut progress = Progress { acked: 0, total, unsent: one_repeat.saturating_mul(u32::from(job.repeats)) };
        cx.emit(DriverEvent::State(MachineState::Running { packets_acked: 0, total }));
        let started = cx.now();
        match self.session.run_job(&spec, cx, &mut progress) {
            Ok(()) => Ok(Outcome::Finished { duration: cx.now().saturating_since(started) }),
            Err(halt) => halt_to_result(halt, || Outcome::Stopped(self.session.emergency(cx))),
        }
    }

    fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport {
        self.session.emergency(cx)
    }

    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError> {
        let s = self.session.read_status(cx).map_err(|h| match h {
            Halt::Fault(e) => e,
            Halt::Stopped => DriverError::Protocol("status read stopped".to_owned()),
        })?;
        let state = if self.connected { MachineState::Idle } else { MachineState::Disconnected };
        Ok(MachineStatus { state, status_bits: Some(s.0), ready: s.ready(), busy: s.busy() })
    }

    /// MO(0) · WritePort(0) on an idle board (errors traced), then the session is closed. The transport itself is
    /// released when the backend is dropped (the driver thread ends with its handle).
    fn disconnect(&mut self, cx: &mut Cx) {
        if self.connected {
            self.session.park(cx);
        }
        self.connected = false;
    }
}

impl EncodedJob for LmcJob {
    fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    fn machine(&self) -> &MachineId {
        &self.machine
    }

    fn emits(&self) -> bool {
        true
    }

    fn bytes_hash(&self) -> Fingerprint {
        LmcJob::bytes_hash(self)
    }
}

impl EncodedJob for LmcFrameJob {
    fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    fn machine(&self) -> &MachineId {
        &self.machine
    }

    fn emits(&self) -> bool {
        false
    }

    /// Frames are never armed, so nothing compares this hash. It is still computed from the frame's own bytes
    /// (blake3 is not a dependency of the driver: FNV-1a 64 over every packet, in send order, `once` then `pass`,
    /// repeated four times with different seeds to fill 32 bytes), so two different frames never share it.
    fn bytes_hash(&self) -> Fingerprint {
        let mut out = [0u8; 32];
        let (lanes, _) = out.as_chunks_mut::<8>();
        for (lane, chunk) in lanes.iter_mut().enumerate() {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ u64::try_from(lane).unwrap_or(0).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            for p in self.once.iter().chain(&self.pass) {
                for b in p.bytes() {
                    h ^= u64::from(*b);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            *chunk = h.to_le_bytes();
        }
        Fingerprint(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openlaser_types::VirtualClock;

    use super::*;
    use crate::stop::StopLatch;

    /// A transport that refuses everything, like the WP0 stubs and a hardware-disabled `UsbLmc`.
    struct Refusing;

    impl LmcTransport for Refusing {
        fn submit_single(&mut self, _cmd: Single) -> Result<(), XferError> {
            Err(XferError::HardwareDisabled)
        }
        fn poll_single(&mut self, _slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
            Err(XferError::HardwareDisabled)
        }
        fn submit_packet(&mut self, _pkt: &Packet) -> Result<(), XferError> {
            Err(XferError::HardwareDisabled)
        }
        fn poll_packet(&mut self, _slice: Duration) -> Result<Option<()>, XferError> {
            Err(XferError::HardwareDisabled)
        }
        fn cancel_all(&mut self) {}
        fn drain_replies(&mut self) {}
    }

    fn galvo() -> GalvoMachine {
        match openlaser_machine::jpt_m7_200_sim().kind {
            openlaser_machine::Kinematics::Galvo(g) => g,
            openlaser_machine::Kinematics::Gantry(_) => panic!("the JPT M7 is a galvo"),
        }
    }

    #[test]
    fn work_is_refused_until_connected() {
        let clock = Arc::new(VirtualClock::new());
        let (wake, _w) = crossbeam_channel::bounded(1);
        let latch = StopLatch::new(wake);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut cx = Cx::new(&latch, &tx, &*clock);
        let mut b = LmcBackend::new(Refusing, galvo());
        let job = LmcJob {
            prelude: Vec::new(),
            lists: Vec::new(),
            repeats: 1,
            postlude: Vec::new(),
            fingerprint: Fingerprint([1; 32]),
            machine: MachineId("m".into()),
        };
        let err = b.run(&job, &mut cx).unwrap_err();
        assert!(matches!(&err, DriverError::Refused(r) if r.code == RefusalCode::NotConnected), "{err:?}");
        // A transport that refuses makes connect fail (and the WP0 init stub is empty): never "connected".
        assert!(b.connect(&mut cx).is_err());
        assert!(b.run(&job, &mut cx).is_err());
        // The emergency stop still runs every step and reports it could not confirm.
        let report = b.emergency_stop(&mut cx);
        assert!(!report.confirmed && !report.mo_closed && !report.ports_zero);
        assert!(report.notes.iter().any(|n| n.contains("Fiber_SetMo")), "{:?}", report.notes);
    }

    #[test]
    fn an_invalid_profile_is_refused() {
        let clock = Arc::new(VirtualClock::new());
        let (wake, _w) = crossbeam_channel::bounded(1);
        let latch = StopLatch::new(wake);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut cx = Cx::new(&latch, &tx, &*clock);
        let mut m = galvo();
        m.transfer_timeout_ms = 0;
        let mut b = LmcBackend::new(Refusing, m);
        let err = b.connect(&mut cx).unwrap_err();
        assert!(err.is_refusal() && err.to_string().contains("transfer_timeout_ms"), "{err}");
    }

    #[test]
    fn frame_bytes_hash_is_stable_and_empty_frames_hash_alike() {
        let f = LmcFrameJob {
            once: Vec::new(),
            once_passes: 1,
            once_est: Duration::ZERO,
            pass: Vec::new(),
            pass_est: Duration::ZERO,
            fingerprint: Fingerprint([0; 32]),
            machine: MachineId("m".into()),
            style: openlaser_types::FrameStyle::Bounds,
            source: openlaser_types::FrameSource::Job,
        };
        assert_eq!(f.bytes_hash(), f.clone().bytes_hash());
        assert!(!f.emits());
    }
}

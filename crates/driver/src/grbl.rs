//! GRBL streaming (SPEC §5.7). Tested on FakeGrbl only in this build. WP0 stub: refuses everything.

use std::time::Duration;

use openlaser_gcode::{GcodeFrame, GcodeProgram};
use openlaser_machine::{DeviceKind, GantryMachine};
use openlaser_types::{Fingerprint, FrameMode, FrameOrigin, MachineId, SessionId};

use crate::backend::{Backend, Capabilities, DeviceInfo, DriverError, EncodedJob, FrameOutcome, MachineStatus, Outcome};
use crate::stop::{Cx, StopReport};

pub trait ByteLink: Send {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError>;
    /// ! ~ ? 0x18 0x85: bypass the queue.
    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError>;
    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError>;
}

impl<L: ByteLink + ?Sized> ByteLink for Box<L> {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        (**self).write_all(bytes, slice)
    }

    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        (**self).write_realtime(byte)
    }

    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        (**self).read_available(buf, slice)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LinkError {
    #[error("timeout")]
    Timeout,
    #[error("closed")]
    Closed,
    #[error("io: {0}")]
    Io(String),
    #[error("hardware access is disabled")]
    HardwareDisabled,
    /// WP0 stub marker (SPEC §3.0); removed at I3.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub struct GrblBackend<L: ByteLink> {
    link: L,
    machine: GantryMachine,
}

impl<L: ByteLink> GrblBackend<L> {
    pub fn new(link: L, machine: GantryMachine) -> Self {
        GrblBackend { link, machine }
    }
}

/// `Frame = GcodeFrame` (sealed, built only by `gcode::frame_program`), so a burning program cannot be framed.
impl<L: ByteLink + 'static> Backend for GrblBackend<L> {
    type Job = GcodeProgram;
    type Frame = GcodeFrame;

    fn capabilities(&self) -> Capabilities {
        Capabilities { kind: DeviceKind::Grbl, frame: false, run: false, pause: false, jog: false, home: false, inputs: false }
    }

    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError> {
        let _ = (cx, &self.machine);
        Err(DriverError::NotImplemented("openlaser_driver::GrblBackend::connect"))
    }

    fn frame(&mut self, f: &GcodeFrame, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError> {
        let _ = (f, mode, origin, session, cx);
        Err(DriverError::NotImplemented("openlaser_driver::GrblBackend::frame"))
    }

    fn run(&mut self, job: &GcodeProgram, cx: &mut Cx) -> Result<Outcome, DriverError> {
        let _ = (job, cx);
        Err(DriverError::NotImplemented("openlaser_driver::GrblBackend::run"))
    }

    fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport {
        let _ = cx;
        // Feed hold then soft reset; errors are reported, never ignored silently (WP5).
        let hold = self.link.write_realtime(b'!');
        let reset = self.link.write_realtime(0x18);
        StopReport {
            confirmed: false,
            elapsed: Duration::ZERO,
            possible_desync: true,
            mo_closed: false,
            ports_zero: false,
            notes: vec![format!("not implemented: openlaser_driver::GrblBackend::emergency_stop (hold: {hold:?}, reset: {reset:?})")],
        }
    }

    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError> {
        let _ = cx;
        Err(DriverError::NotImplemented("openlaser_driver::GrblBackend::status"))
    }

    fn disconnect(&mut self, cx: &mut Cx) {
        let _ = cx;
    }
}

impl EncodedJob for GcodeProgram {
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
        GcodeProgram::bytes_hash(self)
    }
}

impl EncodedJob for GcodeFrame {
    fn fingerprint(&self) -> &Fingerprint {
        &self.program().fingerprint
    }

    fn machine(&self) -> &MachineId {
        &self.program().machine
    }

    fn emits(&self) -> bool {
        false
    }

    fn bytes_hash(&self) -> Fingerprint {
        self.program().bytes_hash()
    }
}

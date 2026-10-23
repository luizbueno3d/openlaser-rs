//! The LMC session (SPEC §5.3; red zone). WP0 stub: refuses everything.

use std::time::Duration;

use openlaser_lmc::{LmcFrameJob, LmcJob, LmcTransport};
use openlaser_machine::{DeviceKind, GalvoMachine};
use openlaser_types::{Fingerprint, FrameMode, FrameOrigin, MachineId, SessionId};

use crate::backend::{Backend, Capabilities, DeviceInfo, DriverError, EncodedJob, FrameOutcome, MachineStatus, Outcome};
use crate::stop::{Cx, StopReport};

pub struct LmcBackend<T: LmcTransport> {
    transport: T,
    machine: GalvoMachine,
}

impl<T: LmcTransport> LmcBackend<T> {
    pub fn new(transport: T, machine: GalvoMachine) -> Self {
        LmcBackend { transport, machine }
    }
}

impl<T: LmcTransport + 'static> Backend for LmcBackend<T> {
    type Job = LmcJob;
    type Frame = LmcFrameJob;

    fn capabilities(&self) -> Capabilities {
        Capabilities { kind: DeviceKind::GalvoLmc, frame: false, run: false, pause: false, jog: false, home: false, inputs: false }
    }

    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError> {
        let _ = (cx, &self.machine);
        Err(DriverError::NotImplemented("openlaser_driver::LmcBackend::connect"))
    }

    fn frame(&mut self, f: &LmcFrameJob, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError> {
        let _ = (f, mode, origin, session, cx);
        Err(DriverError::NotImplemented("openlaser_driver::LmcBackend::frame"))
    }

    fn run(&mut self, job: &LmcJob, cx: &mut Cx) -> Result<Outcome, DriverError> {
        let _ = (job, cx);
        Err(DriverError::NotImplemented("openlaser_driver::LmcBackend::run"))
    }

    fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport {
        let _ = cx;
        self.transport.cancel_all();
        StopReport {
            confirmed: false,
            elapsed: Duration::ZERO,
            possible_desync: true,
            mo_closed: false,
            ports_zero: false,
            notes: vec!["not implemented: openlaser_driver::LmcBackend::emergency_stop".to_owned()],
        }
    }

    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError> {
        let _ = cx;
        Err(DriverError::NotImplemented("openlaser_driver::LmcBackend::status"))
    }

    fn disconnect(&mut self, cx: &mut Cx) {
        let _ = cx;
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

    /// WP0 stub: all zeros (frames are never armed; WP4/WP5 decide whether frames need it).
    fn bytes_hash(&self) -> Fingerprint {
        Fingerprint([0; 32])
    }
}

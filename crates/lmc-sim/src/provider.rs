//! DeviceProvider for the GUI/CLI "Simulator" machines (`GalvoLink::Sim`, `GantryLink::Sim`).

use std::sync::{Arc, Mutex};

use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError};
use openlaser_lmc::LmcTransport;
use openlaser_machine::{GalvoMachine, GantryMachine};
use openlaser_types::{Clock, Refusal, RefusalCode, SystemClock};

use crate::fake::{FakeLmc, Recorder, SimConfig};
use crate::grbl::FakeGrbl;

/// USBLMCV4 (SPEC §5.1).
const LMC_PID: u16 = 0x9899;
/// The clone board: detected and refused, never given firmware.
const CLONE_PID: u16 = 0x9980;

pub struct SimProvider {
    cfg: SimConfig,
    /// SystemClock-paced by default.
    clock: Arc<dyn Clock>,
    last_recorder: Mutex<Option<Arc<Recorder>>>,
}

impl SimProvider {
    pub fn new(cfg: SimConfig) -> Self {
        SimProvider { cfg, clock: Arc::new(SystemClock::new()), last_recorder: Mutex::new(None) }
    }

    pub fn last_recorder(&self) -> Option<Arc<Recorder>> {
        self.last_recorder.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl DeviceProvider for SimProvider {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError> {
        Ok(vec![DetectedDevice {
            vid: 0x9588,
            pid: self.cfg.pid,
            serial: None,
            location: "sim:lmc0".into(),
            supported: self.cfg.pid == LMC_PID,
            note: match self.cfg.pid {
                LMC_PID => "simulated JCZ/BJJCZ LMC".into(),
                CLONE_PID => "simulated clone board (pid 0x9980): refused, firmware is never loaded".into(),
                pid => format!("simulated board with unsupported pid {pid:#06x}"),
            },
        }])
    }

    /// Refuses a clone board (pid 0x9980) as `CloneBoard` and any other unsupported pid as `NoDevice`, as the USB
    /// provider does (SPEC §5.1).
    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        match self.cfg.pid {
            LMC_PID => {}
            CLONE_PID => {
                return Err(DriverError::Refused(Refusal::new(
                    RefusalCode::CloneBoard,
                    "The board is a clone (USB pid 0x9980). OpenLaser does not load firmware into it; use a JCZ/BJJCZ LMC board (pid 0x9899).",
                )));
            }
            pid => {
                return Err(DriverError::Refused(Refusal::new(
                    RefusalCode::NoDevice,
                    format!("The board's USB pid {pid:#06x} is not a supported LMC board; only 0x9899 is."),
                )));
            }
        }
        let mut board = FakeLmc::new(self.cfg.clone(), Arc::clone(&self.clock));
        *self.last_recorder.lock().unwrap_or_else(|e| e.into_inner()) = Some(board.recorder());
        board.set_field_mm(m.field_f());
        Ok(Box::new(board))
    }

    fn open_link(&self, _m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError> {
        Ok(Box::new(FakeGrbl::new(Arc::clone(&self.clock))))
    }

    /// `SimConfig.emission_cap_pct` (None by default; tests set it).
    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError> {
        Ok(self.cfg.emission_cap_pct)
    }

    /// The simulator keeps no bench session: nothing to record.
    fn record_emission(&self) -> Result<(), DriverError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use openlaser_machine::{Kinematics, jpt_m7_200_sim};

    use super::*;

    fn provider(pid: u16) -> SimProvider {
        SimProvider::new(SimConfig { buffer_depth: 3, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid, faults: vec![], emission_cap_pct: None })
    }
    fn refusal(r: Result<Box<dyn LmcTransport>, DriverError>) -> Option<RefusalCode> {
        match r {
            Err(DriverError::Refused(refusal)) => Some(refusal.code),
            _ => None,
        }
    }

    #[test]
    fn clone_boards_are_refused_as_clone_board() {
        let Kinematics::Galvo(m) = jpt_m7_200_sim().kind else { panic!("the sim profile is a galvo") };
        assert_eq!(refusal(provider(CLONE_PID).open_lmc(&m)), Some(RefusalCode::CloneBoard));
        assert_eq!(refusal(provider(0x1234).open_lmc(&m)), Some(RefusalCode::NoDevice));
        let ok = provider(LMC_PID);
        assert!(ok.last_recorder().is_none());
        assert!(ok.open_lmc(&m).is_ok());
        assert!(ok.last_recorder().is_some());
        let detected = provider(CLONE_PID).detect().unwrap();
        assert!(!detected[0].supported);
        assert!(detected[0].note.contains("0x9980"));
        assert!(provider(LMC_PID).detect().unwrap()[0].supported);
    }
}

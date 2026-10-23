//! DeviceProvider for the GUI/CLI "Simulator" machines (`GalvoLink::Sim`, `GantryLink::Sim`).

use std::sync::{Arc, Mutex};

use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError};
use openlaser_lmc::LmcTransport;
use openlaser_machine::{GalvoMachine, GantryMachine};
use openlaser_types::{Clock, SystemClock};

use crate::fake::{FakeLmc, Recorder, SimConfig};
use crate::grbl::FakeGrbl;

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
            supported: self.cfg.pid == 0x9899,
            note: if self.cfg.pid == 0x9899 { "simulated JCZ/BJJCZ LMC".into() } else { "unsupported simulated clone board".into() },
        }])
    }

    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        if self.cfg.pid != 0x9899 {
            return Err(DriverError::NotImplemented("simulated clone board is not supported"));
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

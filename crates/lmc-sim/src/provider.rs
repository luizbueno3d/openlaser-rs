//! DeviceProvider for the GUI/CLI "Simulator" machines (`GalvoLink::Sim`, `GantryLink::Sim`).

use std::sync::Arc;

use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError};
use openlaser_lmc::LmcTransport;
use openlaser_machine::{GalvoMachine, GantryMachine};
use openlaser_types::{Clock, SystemClock};

use crate::fake::{Recorder, SimConfig};

pub struct SimProvider {
    cfg: SimConfig,
    /// SystemClock-paced by default.
    clock: Arc<dyn Clock>,
}

impl SimProvider {
    pub fn new(cfg: SimConfig) -> Self {
        SimProvider { cfg, clock: Arc::new(SystemClock::new()) }
    }

    /// WP0 stub: none.
    pub fn last_recorder(&self) -> Option<Arc<Recorder>> {
        None
    }
}

/// WP0 stub: detects and opens nothing.
impl DeviceProvider for SimProvider {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError> {
        Err(DriverError::NotImplemented("openlaser_lmc_sim::SimProvider::detect"))
    }

    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        let _ = (m, &self.cfg, &self.clock);
        Err(DriverError::NotImplemented("openlaser_lmc_sim::SimProvider::open_lmc"))
    }

    fn open_link(&self, m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError> {
        let _ = m;
        Err(DriverError::NotImplemented("openlaser_lmc_sim::SimProvider::open_link"))
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

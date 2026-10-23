//! Rust port of openlaser-rust-plan/rayforge-test/fake_grbl.py: GRBL 1.1 banner, $$, $I, $G, $#, $H, $X, $J=,
//! 15-block planner, 128-byte RX buffer, realtime ? ! ~ 0x18 0x85, alarms, status reports, motion time scaled.
//! WP0 stub: refuses every transfer.

use std::sync::Arc;
use std::time::Duration;

use openlaser_driver::{ByteLink, LinkError};
use openlaser_types::Clock;

pub struct FakeGrbl {
    clock: Arc<dyn Clock>,
    log: Vec<String>,
}

impl FakeGrbl {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        FakeGrbl { clock, log: Vec::new() }
    }

    pub fn log(&self) -> Vec<String> {
        self.log.clone()
    }
}

impl ByteLink for FakeGrbl {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        let _ = (bytes, slice, self.clock.now());
        Err(LinkError::NotImplemented("openlaser_lmc_sim::grbl::FakeGrbl::write_all"))
    }

    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        let _ = byte;
        Err(LinkError::NotImplemented("openlaser_lmc_sim::grbl::FakeGrbl::write_realtime"))
    }

    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        let _ = (buf, slice);
        Err(LinkError::NotImplemented("openlaser_lmc_sim::grbl::FakeGrbl::read_available"))
    }
}

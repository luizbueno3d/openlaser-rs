//! The bench session (S17, S23). WP0 stubs refuse (fail closed).

use openlaser_types::{Refusal, RefusalCode};

/// `<data_dir>/OpenLaser/bench-session.json` {createdUnix, pid, emissions}; valid ≤ 2 h (wall clock; the file
/// crosses processes). No tty name is stored: reading it needs `ttyname(3)` (unsafe); `IsTerminal` is enough.
pub fn valid() -> Result<(), Refusal> {
    Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_transport::bench::valid"))
}

/// Only openlaser-hw calls it; stdin and stdout must be TTYs.
pub fn start_from_tty() -> Result<(), Refusal> {
    Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_transport::bench::start_from_tty"))
}

/// S23: `Some(10.0)` until one emission has completed in this bench session, then `None`.
pub fn emission_cap_pct() -> Result<Option<f64>, Refusal> {
    Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_transport::bench::emission_cap_pct"))
}

/// After a run finishes (not after a stop or fault).
pub fn record_emission() -> Result<(), Refusal> {
    Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_transport::bench::record_emission"))
}

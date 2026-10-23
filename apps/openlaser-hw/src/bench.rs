//! `openlaser-hw bench start|status|end` (S17, S23; PLAN §5.2): the bench session that every real-USB open needs. It
//! lives in `transport::bench` (the per-user `bench-session.json`, valid ≤ 2 h), which only a `hw` build links. TTY
//! only.

use openlaser_types::RefusalCode;

use crate::args::BenchCmd;
use crate::{HwError, Terminal, require_tty};

pub fn run(cmd: BenchCmd, term: &mut dyn Terminal) -> Result<(), HwError> {
    require_tty(term, "bench")?;
    imp::run(cmd, term)
}

#[cfg(feature = "hw")]
mod imp {
    use openlaser_transport::bench;

    use super::*;

    pub(super) fn run(cmd: BenchCmd, term: &mut dyn Terminal) -> Result<(), HwError> {
        match cmd {
            BenchCmd::Start => {
                bench::start_from_tty()?;
                term.write("Bench session started (valid for 2 h). The first emission is capped at 10 % power.\n");
                Ok(())
            }
            BenchCmd::Status => {
                bench::valid()?;
                match bench::emission_cap_pct()? {
                    Some(cap) => term.write(&format!("Bench session valid. First emission still capped at {cap} %.\n")),
                    None => term.write("Bench session valid. The first emission has completed; no cap.\n"),
                }
                Ok(())
            }
            BenchCmd::End => Err(HwError::refused(
                RefusalCode::NotImplemented,
                "`bench end` needs transport::bench::end (integrator request); the session still expires 2 h after it started",
            )),
        }
    }
}

#[cfg(not(feature = "hw"))]
mod imp {
    use super::*;

    pub(super) fn run(cmd: BenchCmd, _term: &mut dyn Terminal) -> Result<(), HwError> {
        let what = match cmd {
            BenchCmd::Start => "start",
            BenchCmd::Status => "status",
            BenchCmd::End => "end",
        };
        Err(HwError::refused(
            RefusalCode::HardwareDisabled,
            format!("`bench {what}`: bench sessions guard the real laser, and this openlaser-hw was built without the `hw` feature"),
        ))
    }
}

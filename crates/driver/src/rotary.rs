//! Human bench motion seam, deliberately absent from engine/CLI/MCP command dispatch.
//! Uses the same sliced waits and emergency sequence as marking, with MO always closed.
use crate::lmc_backend::TransportWire;
use crate::lmc_session::{Halt, Session, SessionCfg};
use crate::stop::{Cx, StopLatch, StopMailbox, Stopper};
use crate::{DriverError, DriverEvent};
use openlaser_lmc::{AxisMove, GalvoMachine, LmcTransport, abort_packet};
use openlaser_types::Clock;
use std::sync::Arc;

pub struct RotarySession<T: LmcTransport> {
    session: Session<TransportWire<T>>,
    machine: GalvoMachine,
    clock: Arc<dyn Clock>,
    latch: Arc<StopLatch>,
    mailbox: Arc<StopMailbox>,
    events: crossbeam_channel::Sender<DriverEvent>,
}
impl<T: LmcTransport> RotarySession<T> {
    pub fn new(transport: T, machine: GalvoMachine, clock: Arc<dyn Clock>) -> Result<Self, DriverError> {
        let r = machine
            .rotary
            .as_ref()
            .filter(|r| r.enabled)
            .ok_or_else(|| DriverError::Protocol("rotary.enabled is required; enable it in the human-only profile settings".into()))?;
        r.validate(machine.work_area).map_err(DriverError::Protocol)?;
        let cfg = SessionCfg::from_machine(&machine).map_err(DriverError::Protocol)?;
        let (wake, _) = crossbeam_channel::bounded(1);
        let latch = Arc::new(StopLatch::with_clock(wake, clock.clone()));
        let (events, _) = crossbeam_channel::bounded(1);
        Ok(Self {
            session: Session::new(TransportWire(transport), cfg, abort_packet()),
            machine,
            clock,
            latch,
            mailbox: Arc::new(StopMailbox::default()),
            events,
        })
    }
    pub fn stopper(&self) -> Stopper {
        Stopper::with_mailbox(self.latch.clone(), self.mailbox.clone())
    }

    /// Degrees are relative to the current position; targets are sign-magnitude absolute pulses.
    pub fn execute(&mut self, degrees: Option<f64>) -> Result<i32, DriverError> {
        let cx = Cx::new(&self.latch, &self.events, self.clock.as_ref());
        let result = (|| {
            let status = self.session.axis_status(&cx)?;
            if status.busy() || status.0 & openlaser_lmc::StatusWord::AXIS != 0 {
                return Err(Halt::Fault(DriverError::refused(
                    openlaser_types::RefusalCode::MachineBusy,
                    "machine or rotary axis is active; stop it before bench motion",
                )));
            }
            let position = self.session.read_axis(&cx)?;
            let Some(deg) = degrees else {
                return Ok(position);
            };
            let r = self.machine.rotary.as_ref().ok_or_else(|| Halt::Fault(DriverError::Protocol("missing rotary settings".into())))?;
            let delta = (deg / 360.0 * f64::from(r.pulses_per_rev)).round() * if r.reverse { -1.0 } else { 1.0 };
            let target = f64::from(position) + delta;
            if !deg.is_finite() || delta.abs() > r.max_turns_per_job * f64::from(r.pulses_per_rev) || target.abs() > f64::from(i32::MAX) {
                return Err(Halt::Fault(DriverError::refused(
                    openlaser_types::RefusalCode::UnsupportedMode,
                    "rotary motion exceeds configured turns or sign-magnitude range",
                )));
            }
            // The board is idle. Close MO and ports before any extended-axis command; no init or marking list.
            self.session.darken(&cx)?;
            let mv = AxisMove { target_pulses: target as i32, min_speed: r.min_speed_pps, max_speed: r.max_speed_pps, accel_ms: r.accel_ms };
            self.session.move_axis(&mv, &cx)?;
            self.session.read_axis(&cx)
        })();
        match result {
            Ok(p) => Ok(p),
            Err(Halt::Fault(e)) if e.is_refusal() => Err(e),
            Err(h) => {
                let report = self.session.emergency(&cx);
                self.mailbox.post(self.latch.generation(), report.clone());
                let message = match h {
                    Halt::Stopped => "rotary operation stopped".into(),
                    Halt::Fault(e) => e.to_string(),
                };
                Err(DriverError::Protocol(format!(
                    "{message}; stop in {} ms (MO closed: {}, ports zero: {}); {}",
                    report.elapsed.as_millis(),
                    report.mo_closed,
                    report.ports_zero,
                    report.notes.join("; ")
                )))
            }
        }
    }
}

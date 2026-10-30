//! `--trace <f>` on the real USB device: a [`DeviceProvider`] wrapper whose LMC transport writes every wire event
//! (open, each single command, each status reply, each list packet by sha256, close) to the trace file as it
//! happens, in the same T1 line format as the simulator's trace ([`crate::trace`]).
//!
//! Recording never changes what is sent or when: the event is written, then the call is passed through unchanged. A
//! write failure stops the recording (reported once by the rig) and never fails a transfer; a poisoned lock skips the
//! line. Each line is flushed, so a crash or a signal loses nothing already seen.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError, GalvoMachine, GantryMachine, LmcTransport, Packet, Single, XferError};
use openlaser_types::Clock;

use crate::trace::{TraceWriter, WireEvent};

/// The open trace file and its first failure.
pub(crate) struct Sink {
    writer: TraceWriter,
    clock: Arc<dyn Clock>,
    failed: Option<String>,
    reported: bool,
}

pub(crate) type SharedSink = Arc<Mutex<Sink>>;

pub(crate) fn sink(path: &Path, clock: Arc<dyn Clock>) -> Result<SharedSink, String> {
    Ok(Arc::new(Mutex::new(Sink { writer: TraceWriter::create(path)?, clock, failed: None, reported: false })))
}

fn record(sink: &SharedSink, ev: &WireEvent) {
    let Ok(mut s) = sink.lock() else { return };
    if s.failed.is_some() {
        return;
    }
    let at = s.clock.now();
    if let Err(e) = s.writer.write(ev, at) {
        s.failed = Some(e);
    }
}

/// The first write failure, once.
pub(crate) fn take_failure(sink: &SharedSink) -> Option<String> {
    let mut s = sink.lock().ok()?;
    if s.reported {
        return None;
    }
    let f = s.failed.clone()?;
    s.reported = true;
    Some(f)
}

pub(crate) struct TracingProvider {
    inner: Arc<dyn DeviceProvider>,
    sink: SharedSink,
}

impl TracingProvider {
    pub(crate) fn new(inner: Arc<dyn DeviceProvider>, sink: SharedSink) -> Self {
        TracingProvider { inner, sink }
    }
}

impl DeviceProvider for TracingProvider {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError> {
        self.inner.detect()
    }

    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        let inner = self.inner.open_lmc(m)?;
        record(&self.sink, &WireEvent::Open);
        Ok(Box::new(TracingLmc { inner, sink: Arc::clone(&self.sink) }))
    }

    fn open_link(&self, m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError> {
        self.inner.open_link(m)
    }

    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError> {
        self.inner.emission_cap_pct()
    }

    fn record_emission(&self) -> Result<(), DriverError> {
        self.inner.record_emission()
    }
}

struct TracingLmc {
    inner: Box<dyn LmcTransport>,
    sink: SharedSink,
}

impl LmcTransport for TracingLmc {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        record(&self.sink, &WireEvent::Single { words: cmd.0 });
        self.inner.submit_single(cmd)
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        let r = self.inner.poll_single(slice);
        if let Ok(Some(Some(words))) = &r {
            record(&self.sink, &WireEvent::Read { status: words[3] });
        }
        r
    }

    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        record(&self.sink, &WireEvent::List { sha256: pkt.sha256_hex(), n_cmds: u16::try_from(pkt.real_commands()).ok() });
        self.inner.submit_packet(pkt)
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        self.inner.poll_packet(slice)
    }

    fn cancel_all(&mut self) {
        self.inner.cancel_all();
    }

    fn drain_replies(&mut self) {
        self.inner.drain_replies();
    }
}

impl Drop for TracingLmc {
    fn drop(&mut self) {
        record(&self.sink, &WireEvent::Close);
    }
}

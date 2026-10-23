//! GRBL streaming (SPEC §5.7; PLAN §5.3; research 05 §3.3). Tested on fakes only in this build.
//!
//! Connect waits for the banner and reads `$I`, `$$`, `$G`; it never homes or moves the machine. Streaming is
//! character counting against the RX size from `[OPT:]` (default 128). The realtime bytes `!`, `~`, `?`, `0x18`,
//! `0x85` are written only by the driver thread. On `error:` or `ALARM` the job fails at once and the runtime's
//! emergency stop sends `!` then `0x18`; the fault is sticky until the operator connects again. Written from the
//! GRBL 1.1 interface documentation (gnea/grbl `doc/markdown/interface.md`); no sender code was copied.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use openlaser_gcode::{GcodeFrame, GcodeProgram};
use openlaser_machine::{DeviceKind, GantryMachine, GantryTool};
use openlaser_types::{Fingerprint, FrameMode, FrameOrigin, MachineId, MonoTime, RefusalCode, SessionId};

use crate::backend::{Backend, Capabilities, DeviceInfo, DriverError, DriverEvent, EncodedJob, FrameOutcome, MachineState, MachineStatus, Outcome};
use crate::stop::{Cx, StopReport, Stopped};

pub trait ByteLink: Send {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError>;
    /// ! ~ ? 0x18 0x85: bypass the queue.
    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError>;
    /// Waits ≤ `slice` for input and appends what arrived to `buf`; Ok(0) when nothing came.
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

/// GRBL's documented default serial RX buffer.
pub const DEFAULT_RX_SIZE: usize = 128;
/// How long connect waits for the welcome line (opening a port resets Arduino-class boards).
const BANNER_WAIT: Duration = Duration::from_millis(2500);
/// How long a `$` query waits for its `ok`.
const QUERY_WAIT: Duration = Duration::from_secs(2);
/// `?` status polls while streaming and waiting for Idle (research 05 §3.3: 5–10 Hz).
const STATUS_INTERVAL: Duration = Duration::from_millis(200);
/// No `ok`, no state or position change for this long: the controller is stuck → fault and stop.
const STALL: Duration = Duration::from_secs(10);
/// The soft reset is confirmed by the welcome line within this.
const RESET_CONFIRM: Duration = Duration::from_millis(250);
/// One read slice.
const READ_SLICE: Duration = Duration::from_millis(5);
/// A line longer than this without a newline is not GRBL talking.
const MAX_LINE: usize = 4096;

/// What a received line means.
#[derive(Debug, Clone, PartialEq)]
enum Reply {
    Ok,
    Error(String),
    Alarm(String),
    Status { state: String, position: String },
    Banner,
    Other,
}

fn classify(line: &str) -> Reply {
    if line == "ok" {
        Reply::Ok
    } else if let Some(code) = line.strip_prefix("error:") {
        Reply::Error(code.to_owned())
    } else if let Some(code) = line.strip_prefix("ALARM:") {
        Reply::Alarm(code.to_owned())
    } else if let Some(body) = line.strip_prefix('<') {
        let body = body.trim_end_matches('>');
        let mut fields = body.split('|');
        let state = fields.next().unwrap_or_default().to_owned();
        let position = fields.find(|f| f.starts_with("MPos:") || f.starts_with("WPos:")).unwrap_or_default().to_owned();
        Reply::Status { state, position }
    } else if line.starts_with("Grbl ") || line.starts_with("GrblHAL ") {
        Reply::Banner
    } else {
        Reply::Other
    }
}

/// The lines of a program GRBL is sent: trimmed, without blank lines and whole-line comments.
fn program_lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('(')).collect()
}

fn link_fault(what: &str, e: &LinkError) -> Halt {
    Halt::Fault(DriverError::Transport(format!("{what}: {e}")))
}

#[derive(Debug, Clone, PartialEq)]
enum Halt {
    Stopped,
    Fault(DriverError),
}

impl From<Stopped> for Halt {
    fn from(_: Stopped) -> Self {
        Halt::Stopped
    }
}

pub struct GrblBackend<L: ByteLink> {
    link: L,
    machine: GantryMachine,
    provider: &'static str,
    connected: bool,
    rx: Vec<u8>,
    rx_size: usize,
    settings: BTreeMap<u32, f64>,
    version: Option<String>,
    alarm: Option<String>,
    preflight: Result<(), String>,
    /// The last status said the machine was moving: a soft reset now may lose position.
    moving: bool,
}

impl<L: ByteLink> GrblBackend<L> {
    pub fn new(link: L, machine: GantryMachine) -> Self {
        GrblBackend {
            link,
            machine,
            provider: "grbl",
            connected: false,
            rx: Vec::new(),
            rx_size: DEFAULT_RX_SIZE,
            settings: BTreeMap::new(),
            version: None,
            alarm: None,
            preflight: Err("not connected".to_owned()),
            moving: false,
        }
    }

    /// The provider name `DeviceInfo` reports ("usb", "sim").
    pub fn with_provider_name(mut self, name: &'static str) -> Self {
        self.provider = name;
        self
    }

    /// The RX buffer size streaming counts against (from `[OPT:]`).
    pub fn rx_size(&self) -> usize {
        self.rx_size
    }

    /// `$n=v` settings read at connect.
    pub fn settings(&self) -> &BTreeMap<u32, f64> {
        &self.settings
    }

    /// Reads what is there (≤ one slice), returns the complete lines. Ignores the latch.
    fn read_lines(&mut self, slice: Duration) -> Result<Vec<String>, Halt> {
        let mut chunk = Vec::new();
        match self.link.read_available(&mut chunk, slice) {
            Ok(_) | Err(LinkError::Timeout) => {}
            Err(e) => return Err(link_fault("read", &e)),
        }
        self.rx.extend_from_slice(&chunk);
        let mut lines = Vec::new();
        while let Some(pos) = self.rx.iter().position(|b| *b == b'\n') {
            let rest = self.rx.split_off(pos.saturating_add(1));
            let raw = std::mem::replace(&mut self.rx, rest);
            let line = String::from_utf8_lossy(&raw).trim().to_owned();
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if self.rx.len() > MAX_LINE {
            self.rx.clear();
            return Err(Halt::Fault(DriverError::Protocol(format!("more than {MAX_LINE} bytes without a newline: not a GRBL controller"))));
        }
        Ok(lines)
    }

    fn write_line(&mut self, line: &str, cx: &Cx) -> Result<(), Halt> {
        cx.trace(true, line);
        let mut bytes = Vec::with_capacity(line.len().saturating_add(1));
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        self.link.write_all(&bytes, READ_SLICE).map_err(|e| link_fault("write", &e))
    }

    fn realtime(&mut self, byte: u8, cx: &Cx) -> Result<(), LinkError> {
        if byte != b'?' {
            cx.trace(true, format!("realtime 0x{byte:02X}"));
        }
        self.link.write_realtime(byte)
    }

    /// Waits ≤ `wait` for the welcome line. `live`: the latch ends the wait.
    fn wait_banner(&mut self, cx: &Cx, wait: Duration, live: bool) -> Result<bool, Halt> {
        let start = cx.now();
        let mut slices: u32 = 0;
        loop {
            if live {
                cx.check()?;
            }
            for line in self.read_lines(READ_SLICE)? {
                cx.trace(false, line.as_str());
                if classify(&line) == Reply::Banner {
                    return Ok(true);
                }
            }
            slices = slices.saturating_add(1);
            if cx.now().saturating_since(start).max(READ_SLICE.saturating_mul(slices)) >= wait {
                return Ok(false);
            }
        }
    }

    /// Sends a `$` query and collects its lines until `ok` (or `error:`/`ALARM`).
    fn query(&mut self, cmd: &str, cx: &Cx) -> Result<Vec<String>, Halt> {
        self.write_line(cmd, cx)?;
        let start = cx.now();
        let mut slices: u32 = 0;
        let mut lines = Vec::new();
        loop {
            cx.check()?;
            for line in self.read_lines(READ_SLICE)? {
                cx.trace(false, line.as_str());
                match classify(&line) {
                    Reply::Ok => return Ok(lines),
                    Reply::Error(code) => return Err(Halt::Fault(DriverError::Protocol(format!("`{cmd}` answered error:{code}")))),
                    Reply::Alarm(code) => {
                        self.alarm = Some(code);
                        lines.push(line);
                    }
                    _ => lines.push(line),
                }
            }
            slices = slices.saturating_add(1);
            if cx.now().saturating_since(start).max(READ_SLICE.saturating_mul(slices)) >= QUERY_WAIT {
                return Err(Halt::Fault(DriverError::Protocol(format!("`{cmd}`: no `ok` within {} ms", QUERY_WAIT.as_millis()))));
            }
        }
    }

    fn parse_info(&mut self, lines: &[String]) {
        for line in lines {
            if let Some(v) = line.strip_prefix("[VER:") {
                self.version = Some(v.trim_end_matches(']').to_owned());
            }
            if let Some(opt) = line.strip_prefix("[OPT:") {
                // [OPT:flags,planner_blocks,rx_buffer]
                let rx = opt.trim_end_matches(']').split(',').nth(2).and_then(|v| v.trim().parse::<usize>().ok());
                if let Some(rx) = rx.filter(|r| (16..=65_536).contains(r)) {
                    self.rx_size = rx;
                }
            }
        }
    }

    fn parse_settings(&mut self, lines: &[String]) {
        for line in lines {
            let Some(body) = line.strip_prefix('$') else { continue };
            let Some((k, v)) = body.split_once('=') else { continue };
            let v = v.split_whitespace().next().unwrap_or_default();
            if let (Ok(k), Ok(v)) = (k.trim().parse::<u32>(), v.parse::<f64>())
                && v.is_finite()
            {
                self.settings.insert(k, v);
            }
        }
    }

    /// §5.7 preflight: `$32=1` on laser profiles, `$30` equals the profile's `s_max`, `$130`/`$131` cover the bed,
    /// and no alarm. Every problem is listed.
    fn compute_preflight(&self) -> Result<(), String> {
        let m = &self.machine;
        let mut problems = Vec::new();
        let get = |k: u32| self.settings.get(&k).copied();
        if let Some(code) = &self.alarm {
            problems.push(format!("the controller is in ALARM:{code}: unlock ($X) or home it at the machine"));
        }
        if matches!(m.tool, GantryTool::Laser { .. }) {
            match get(32) {
                Some(v) if (v - 1.0).abs() < 1e-9 => {}
                Some(v) => problems.push(format!("laser mode is off ($32={v}): set $32=1 so the laser is off on rapid moves")),
                None => problems.push("the controller did not report $32 (laser mode)".to_owned()),
            }
        }
        let s_max = f64::from(m.s_max);
        match get(30) {
            Some(v) if (v - s_max).abs() < 0.5 => {}
            Some(v) => problems.push(format!("$30={v} but the profile's S max is {}: make them equal", m.s_max)),
            None => problems.push("the controller did not report $30 (max spindle/laser S)".to_owned()),
        }
        for (key, axis, need) in [(130, "X", m.bed_mm[0]), (131, "Y", m.bed_mm[1])] {
            match get(key) {
                Some(v) if v + 1e-6 >= need => {}
                Some(v) => problems.push(format!("${key}={v} mm (max {axis} travel) is smaller than the profile's bed ({need} mm)")),
                None => problems.push(format!("the controller did not report ${key} (max {axis} travel)")),
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
    }

    /// The welcome line (one soft reset if none came: harmless on an idle controller, it never moves), then `$I`,
    /// `$$`, `$G`.
    fn handshake(&mut self, cx: &Cx) -> Result<(), Halt> {
        if !self.wait_banner(cx, BANNER_WAIT, true)? {
            self.realtime(0x18, cx).map_err(|e| link_fault("soft reset", &e))?;
            if !self.wait_banner(cx, BANNER_WAIT, true)? {
                return Err(Halt::Fault(DriverError::Protocol("no GRBL welcome line: is this a GRBL controller at the right baud rate?".to_owned())));
            }
        }
        let info = self.query("$I", cx)?;
        self.parse_info(&info);
        let settings = self.query("$$", cx)?;
        self.parse_settings(&settings);
        self.query("$G", cx)?;
        Ok(())
    }

    /// The `[VER:]` string read at connect.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    fn ready_for_work(&self, text: &str) -> Result<(), DriverError> {
        if !self.connected {
            return Err(DriverError::refused(RefusalCode::NotConnected, "the controller is not connected: connect first"));
        }
        if let Err(p) = &self.preflight {
            return Err(DriverError::refused(RefusalCode::GrblPreflight, format!("GRBL preflight: {p}")));
        }
        if let Some((i, l)) = program_lines(text).iter().enumerate().find(|(_, l)| l.len().saturating_add(1) > self.rx_size) {
            return Err(DriverError::refused(
                RefusalCode::GrblPreflight,
                format!("program line {} is {} bytes; the controller's RX buffer is {} bytes", i + 1, l.len(), self.rx_size),
            ));
        }
        Ok(())
    }

    /// Character-counting stream, then wait until the controller reports Idle.
    fn stream(&mut self, text: &str, cx: &Cx, report_progress: bool) -> Result<(), Halt> {
        let lines = program_lines(text);
        let total = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        let mut inflight: VecDeque<usize> = VecDeque::new();
        let mut used: usize = 0;
        let mut next: usize = 0;
        let mut acked: u32 = 0;
        let mut last_progress = cx.now();
        let mut last_poll: Option<MonoTime> = None;
        let mut last_seen: Option<(String, String)> = None;
        let mut idle_after_last_ok = false;
        loop {
            cx.check()?;
            while let Some(line) = lines.get(next) {
                let len = line.len().saturating_add(1);
                if used.saturating_add(len) > self.rx_size {
                    break;
                }
                self.write_line(line, cx)?;
                inflight.push_back(len);
                used = used.saturating_add(len);
                next += 1;
            }
            let now = cx.now();
            if last_poll.is_none_or(|t| now.saturating_since(t) >= STATUS_INTERVAL) {
                self.realtime(b'?', cx).map_err(|e| link_fault("status poll", &e))?;
                last_poll = Some(now);
            }
            for line in self.read_lines(READ_SLICE)? {
                match classify(&line) {
                    Reply::Ok => {
                        if let Some(len) = inflight.pop_front() {
                            used = used.saturating_sub(len);
                            acked = acked.saturating_add(1);
                            last_progress = cx.now();
                            if report_progress {
                                cx.emit(DriverEvent::Progress { packets_acked: acked, total, eta_ms: 0 });
                            }
                        } else {
                            cx.trace(false, "ok (unexpected)");
                        }
                    }
                    Reply::Error(code) => {
                        let at = usize::try_from(acked).unwrap_or(usize::MAX);
                        let which = lines.get(at).copied().unwrap_or("?");
                        return Err(Halt::Fault(DriverError::Protocol(format!("GRBL error:{code} on line {} `{which}`", at.saturating_add(1)))));
                    }
                    Reply::Alarm(code) => {
                        self.alarm = Some(code.clone());
                        return Err(Halt::Fault(DriverError::Protocol(format!("GRBL ALARM:{code}"))));
                    }
                    Reply::Banner => return Err(Halt::Fault(DriverError::Protocol("the controller reset itself during the job".to_owned()))),
                    Reply::Status { state, position } => {
                        self.moving = !state.starts_with("Idle");
                        let seen = (state.clone(), position);
                        if last_seen.as_ref() != Some(&seen) {
                            last_progress = cx.now();
                            last_seen = Some(seen);
                        }
                        if next >= lines.len() && inflight.is_empty() && state.starts_with("Idle") {
                            idle_after_last_ok = true;
                        }
                    }
                    Reply::Other => cx.trace(false, line.as_str()),
                }
            }
            if idle_after_last_ok {
                return Ok(());
            }
            let stalled = cx.now().saturating_since(last_progress);
            if stalled > STALL {
                let waited_ms = u64::try_from(stalled.as_millis()).unwrap_or(u64::MAX);
                return Err(Halt::Fault(DriverError::Watchdog { waited_ms }));
            }
        }
    }
}

/// `Frame = GcodeFrame` (sealed, built only by `gcode::frame_program`), so a burning program cannot be framed.
impl<L: ByteLink + 'static> Backend for GrblBackend<L> {
    type Job = GcodeProgram;
    type Frame = GcodeFrame;

    fn capabilities(&self) -> Capabilities {
        Capabilities { kind: DeviceKind::Grbl, frame: true, run: true, pause: false, jog: false, home: false, inputs: false }
    }

    /// Banner (a soft reset if none came), `$I`, `$$`, `$G`, preflight. Never homes and never moves the machine,
    /// whatever `home_on_connect` says.
    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError> {
        let cx: &Cx = cx;
        self.connected = false;
        self.rx.clear();
        self.alarm = None;
        self.settings.clear();
        match self.handshake(cx) {
            Ok(()) => {}
            Err(Halt::Stopped) => return Err(DriverError::Protocol("connect was stopped".to_owned())),
            Err(Halt::Fault(e)) => return Err(e),
        }
        if self.machine.home_on_connect {
            cx.trace(false, "home_on_connect is ignored: OpenLaser never moves the machine on connect");
        }
        self.preflight = self.compute_preflight();
        if let Err(p) = &self.preflight {
            cx.trace(false, format!("preflight: {p}"));
        }
        self.connected = true;
        Ok(DeviceInfo { vid: 0, pid: 0, serial: None, version: None, provider: self.provider.to_owned() })
    }

    /// Laser-off framing. GRBL frames issue NO `FrameReceipt` in this build: `GcodeFrame` does not carry the frame
    /// style and source a receipt must record (S21), and a receipt that guessed `source` could let a tool-layer frame
    /// count for the job (S8). So a gantry run is refused `FrameRequired` until `gcode` exports them (REQUEST).
    fn frame(&mut self, f: &GcodeFrame, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError> {
        let _ = (origin, session);
        let cx: &Cx = cx;
        let text = f.program().text.as_str();
        self.ready_for_work(text)?;
        cx.trace(false, "GRBL frames do not count toward ARM in this build (the frame's style and source are unknown)");
        let mut passes: u32 = 0;
        loop {
            match self.stream(text, cx, false) {
                Ok(()) => passes = passes.saturating_add(1),
                Err(Halt::Stopped) => return Ok(FrameOutcome::Stopped(self.emergency(cx))),
                Err(Halt::Fault(e)) => return Err(e),
            }
            if mode == FrameMode::Once {
                return Ok(FrameOutcome::Completed { passes });
            }
        }
    }

    fn run(&mut self, job: &GcodeProgram, cx: &mut Cx) -> Result<Outcome, DriverError> {
        let cx: &Cx = cx;
        self.ready_for_work(&job.text)?;
        let total = u32::try_from(program_lines(&job.text).len()).unwrap_or(u32::MAX);
        cx.emit(DriverEvent::State(MachineState::Running { packets_acked: 0, total }));
        let started = cx.now();
        match self.stream(&job.text, cx, true) {
            Ok(()) => Ok(Outcome::Finished { duration: cx.now().saturating_since(started) }),
            Err(Halt::Stopped) => Ok(Outcome::Stopped(self.emergency(cx))),
            Err(Halt::Fault(e)) => Err(e),
        }
    }

    fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport {
        self.emergency(cx)
    }

    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError> {
        let cx: &Cx = cx;
        self.realtime(b'?', cx).map_err(|e| DriverError::Transport(format!("status poll: {e}")))?;
        let start = cx.now();
        let mut slices: u32 = 0;
        loop {
            let lines = self.read_lines(READ_SLICE).map_err(|h| match h {
                Halt::Fault(e) => e,
                Halt::Stopped => DriverError::Protocol("status read stopped".to_owned()),
            })?;
            for line in lines {
                if let Reply::Status { state, .. } = classify(&line) {
                    let idle = state.starts_with("Idle");
                    let st = if !self.connected {
                        MachineState::Disconnected
                    } else if state.starts_with("Alarm") {
                        MachineState::Fault
                    } else {
                        MachineState::Idle
                    };
                    return Ok(MachineStatus { state: st, status_bits: None, ready: idle, busy: !idle });
                }
            }
            slices = slices.saturating_add(1);
            if cx.now().saturating_since(start).max(READ_SLICE.saturating_mul(slices)) >= RESET_CONFIRM {
                return Err(DriverError::Transport("no status report within 250 ms".to_owned()));
            }
        }
    }

    /// Sends nothing: the machine never moves on connect or disconnect.
    fn disconnect(&mut self, cx: &mut Cx) {
        let _ = cx;
        self.connected = false;
    }
}

impl<L: ByteLink> GrblBackend<L> {
    /// Feed hold, then soft reset (the reset turns the laser off), then wait ≤ 250 ms for the welcome line that
    /// confirms it. Ignores the latch; never returns early.
    fn emergency(&mut self, cx: &Cx) -> StopReport {
        let requested = cx.stop_requested_at();
        let mut notes = Vec::new();
        if let Err(e) = self.realtime(b'!', cx) {
            notes.push(format!("feed hold: {e}"));
        }
        let reset = self.realtime(0x18, cx);
        if let Err(e) = &reset {
            notes.push(format!("soft reset: {e}"));
        }
        let confirmed = reset.is_ok()
            && match self.wait_banner(cx, RESET_CONFIRM, false) {
                Ok(seen) => seen,
                Err(Halt::Fault(e)) => {
                    notes.push(format!("{e}"));
                    false
                }
                Err(Halt::Stopped) => false,
            };
        if !confirmed {
            notes.push("the controller did not confirm the soft reset".to_owned());
        }
        let possible_desync = self.moving;
        if possible_desync {
            notes.push("the machine was moving: its position may be lost; unlock ($X) or home it before the next job".to_owned());
        }
        self.moving = false;
        StopReport { confirmed, elapsed: cx.now().saturating_since(requested), possible_desync, mo_closed: confirmed, ports_zero: confirmed, notes }
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use openlaser_machine::{GantryLink, MmPerS, WorkspaceOrigin};
    use openlaser_types::VirtualClock;

    use super::*;
    use crate::stop::StopLatch;

    /// A scripted GRBL 1.1: welcome line, `$I`/`$$`/`$G`, an RX buffer it checks, one queued line processed per
    /// read, `?` status reports, `!`/`0x18`, scripted errors. Time is virtual.
    #[derive(Default)]
    struct Log {
        written: Vec<u8>,
        realtime: Vec<u8>,
        max_rx_in_use: usize,
        lines: Vec<String>,
    }

    struct FakeGrbl {
        clock: Arc<VirtualClock>,
        log: Arc<Mutex<Log>>,
        out: VecDeque<u8>,
        rx_in_use: usize,
        queue: VecDeque<String>,
        settings: Vec<(u32, f64)>,
        error_on_line: Option<usize>,
        processed: usize,
        moving_for: u32,
        hook: Option<Box<dyn FnMut(usize) + Send>>,
    }

    impl FakeGrbl {
        fn new(clock: Arc<VirtualClock>) -> Self {
            let mut f = FakeGrbl {
                clock,
                log: Arc::default(),
                out: VecDeque::new(),
                rx_in_use: 0,
                queue: VecDeque::new(),
                settings: vec![(30, 1000.0), (32, 1.0), (130, 400.0), (131, 400.0)],
                error_on_line: None,
                processed: 0,
                moving_for: 0,
                hook: None,
            };
            f.say("\r\nGrbl 1.1h ['$' for help]\r\n");
            f
        }

        fn say(&mut self, s: &str) {
            self.out.extend(s.bytes());
        }

        fn process_one(&mut self) {
            let Some(line) = self.queue.pop_front() else { return };
            self.rx_in_use = self.rx_in_use.saturating_sub(line.len() + 1);
            match line.as_str() {
                "$I" => self.say("[VER:1.1h.20190825:fake]\r\n[OPT:VML,15,128]\r\nok\r\n"),
                "$$" => {
                    let s: String = self.settings.iter().map(|(k, v)| format!("${k}={v}\r\n")).collect();
                    self.say(&s);
                    self.say("ok\r\n");
                }
                "$G" => self.say("[GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0]\r\nok\r\n"),
                _ => {
                    self.processed += 1;
                    if Some(self.processed) == self.error_on_line {
                        self.say("error:20\r\n");
                    } else {
                        self.moving_for = 3;
                        self.say("ok\r\n");
                    }
                    if let Some(h) = self.hook.as_mut() {
                        h(self.processed);
                    }
                }
            }
        }
    }

    impl ByteLink for FakeGrbl {
        fn write_all(&mut self, bytes: &[u8], _slice: Duration) -> Result<(), LinkError> {
            let mut log = self.log.lock().unwrap();
            log.written.extend_from_slice(bytes);
            for line in String::from_utf8_lossy(bytes).split_terminator('\n') {
                self.rx_in_use += line.len() + 1;
                log.max_rx_in_use = log.max_rx_in_use.max(self.rx_in_use);
                log.lines.push(line.to_owned());
                self.queue.push_back(line.to_owned());
            }
            Ok(())
        }

        fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
            match byte {
                b'?' => {
                    let state = if self.moving_for > 0 || !self.queue.is_empty() { "Run" } else { "Idle" };
                    self.moving_for = self.moving_for.saturating_sub(1);
                    let s = format!("<{state}|MPos:{}.000,0.000,0.000|FS:0,0>\r\n", self.processed);
                    self.say(&s);
                }
                0x18 => {
                    self.queue.clear();
                    self.rx_in_use = 0;
                    self.say("\r\nGrbl 1.1h ['$' for help]\r\n");
                    self.log.lock().unwrap().realtime.push(byte);
                }
                _ => self.log.lock().unwrap().realtime.push(byte),
            }
            Ok(())
        }

        fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
            self.process_one();
            self.clock.advance(slice);
            let n = self.out.len();
            buf.extend(self.out.drain(..));
            Ok(n)
        }
    }

    fn gantry() -> GantryMachine {
        GantryMachine {
            bed_mm: [400.0, 400.0],
            origin: WorkspaceOrigin::BottomLeft,
            max_speed: MmPerS::new(250.0).unwrap(),
            travel_speed: MmPerS::new(250.0).unwrap(),
            s_max: 1000,
            tool: GantryTool::Laser { dynamic: true },
            air_assist: true,
            frame_before_mark: true,
            home_on_connect: true,
            link: GantryLink::Sim,
        }
    }

    fn program(lines: usize) -> GcodeProgram {
        let mut text = String::from("; OpenLaser G-code\nG21\nG90\nM5\n");
        for i in 0..lines {
            text.push_str(&format!("G1 X{}.5 Y{}.25 F600\n", i % 300, i % 200));
        }
        text.push_str("M5\nG0 X0 Y0\n");
        GcodeProgram {
            text,
            lines: 0,
            fingerprint: Fingerprint([1; 32]),
            machine: MachineId("diode".into()),
            distance_mm: 0.0,
            estimate: Duration::from_secs(1),
        }
    }

    struct Rig {
        clock: Arc<VirtualClock>,
        latch: Arc<StopLatch>,
        tx: crossbeam_channel::Sender<DriverEvent>,
        _rx: crossbeam_channel::Receiver<DriverEvent>,
        _w: crossbeam_channel::Receiver<()>,
    }

    impl Rig {
        fn new() -> Self {
            let clock = Arc::new(VirtualClock::new());
            let (wake, _w) = crossbeam_channel::bounded(1);
            let latch = Arc::new(StopLatch::with_clock(wake, clock.clone()));
            let (tx, _rx) = crossbeam_channel::unbounded();
            Rig { clock, latch, tx, _rx, _w }
        }
    }

    #[test]
    fn grbl_never_moves_on_connect() {
        let rig = Rig::new();
        let fake = FakeGrbl::new(rig.clock.clone());
        let log = Arc::clone(&fake.log);
        let mut b = GrblBackend::new(fake, gantry());
        let mut cx = Cx::new(&rig.latch, &rig.tx, &*rig.clock);
        b.connect(&mut cx).unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log.lines, ["$I", "$$", "$G"], "only queries, even with home_on_connect = true");
        assert!(log.realtime.is_empty());
        assert_eq!(b.rx_size(), 128);
        assert_eq!(b.settings().get(&32), Some(&1.0));
    }

    #[test]
    fn grbl_preflight_refusals() {
        for (settings, expect) in [
            (vec![(30, 1000.0), (32, 0.0), (130, 400.0), (131, 400.0)], "$32"),
            (vec![(30, 255.0), (32, 1.0), (130, 400.0), (131, 400.0)], "$30"),
            (vec![(30, 1000.0), (32, 1.0), (130, 300.0), (131, 400.0)], "$130"),
            (vec![(30, 1000.0), (130, 400.0), (131, 400.0)], "$32"),
        ] {
            let rig = Rig::new();
            let mut fake = FakeGrbl::new(rig.clock.clone());
            fake.settings = settings;
            let log = Arc::clone(&fake.log);
            let mut b = GrblBackend::new(fake, gantry());
            let mut cx = Cx::new(&rig.latch, &rig.tx, &*rig.clock);
            b.connect(&mut cx).unwrap();
            let err = b.run(&program(3), &mut cx).unwrap_err();
            let DriverError::Refused(r) = &err else { panic!("{err:?}") };
            assert_eq!(r.code, RefusalCode::GrblPreflight);
            assert!(r.message.contains(expect), "{}", r.message);
            assert_eq!(log.lock().unwrap().lines.len(), 3, "nothing streamed");
        }
    }

    #[test]
    fn grbl_char_counting_respects_rx() {
        let rig = Rig::new();
        let fake = FakeGrbl::new(rig.clock.clone());
        let log = Arc::clone(&fake.log);
        let mut b = GrblBackend::new(fake, gantry());
        let mut cx = Cx::new(&rig.latch, &rig.tx, &*rig.clock);
        b.connect(&mut cx).unwrap();
        let out = b.run(&program(300), &mut cx).unwrap();
        assert!(matches!(out, Outcome::Finished { .. }));
        let log = log.lock().unwrap();
        assert!(log.max_rx_in_use <= 128, "{}", log.max_rx_in_use);
        assert!(log.max_rx_in_use > 64, "the buffer was actually used");
        assert_eq!(log.lines.iter().filter(|l| l.starts_with("G1")).count(), 300);
        assert!(!log.lines.iter().any(|l| l.starts_with(';')), "comments are not sent");
    }

    #[test]
    fn grbl_error_fails_the_job_and_the_stop_sends_hold_then_reset() {
        let rig = Rig::new();
        let mut fake = FakeGrbl::new(rig.clock.clone());
        fake.error_on_line = Some(5);
        let log = Arc::clone(&fake.log);
        let mut b = GrblBackend::new(fake, gantry());
        let mut cx = Cx::new(&rig.latch, &rig.tx, &*rig.clock);
        b.connect(&mut cx).unwrap();
        let err = b.run(&program(50), &mut cx).unwrap_err();
        assert!(matches!(&err, DriverError::Protocol(m) if m.contains("error:20")), "{err:?}");
        // The runtime's convention: a fault is followed at once by the emergency stop.
        let report = b.emergency_stop(&mut cx);
        assert_eq!(log.lock().unwrap().realtime, [b'!', 0x18]);
        assert!(report.confirmed);
    }

    #[test]
    fn grbl_stop_under_100ms() {
        let rig = Rig::new();
        let mut fake = FakeGrbl::new(rig.clock.clone());
        let latch = Arc::clone(&rig.latch);
        fake.hook = Some(Box::new(move |n| {
            if n == 40 {
                latch.trip();
            }
        }));
        let log = Arc::clone(&fake.log);
        let mut b = GrblBackend::new(fake, gantry());
        let mut cx = Cx::new(&rig.latch, &rig.tx, &*rig.clock);
        b.connect(&mut cx).unwrap();
        let out = b.run(&program(500), &mut cx).unwrap();
        let Outcome::Stopped(report) = out else { panic!("{out:?}") };
        assert!(report.confirmed, "{report:?}");
        assert!(report.elapsed <= Duration::from_millis(100), "{:?}", report.elapsed);
        assert!(report.possible_desync, "it was moving");
        let log = log.lock().unwrap();
        assert_eq!(log.realtime, [b'!', 0x18]);
        assert!(log.lines.len() < 200, "streaming stopped at once");
    }

    #[test]
    fn a_line_longer_than_rx_is_refused_before_streaming() {
        let rig = Rig::new();
        let fake = FakeGrbl::new(rig.clock.clone());
        let mut b = GrblBackend::new(fake, gantry());
        let mut cx = Cx::new(&rig.latch, &rig.tx, &*rig.clock);
        b.connect(&mut cx).unwrap();
        let mut job = program(1);
        job.text.push_str(&"G1 X1 ".repeat(40));
        job.text.push('\n');
        let err = b.run(&job, &mut cx).unwrap_err();
        assert!(matches!(&err, DriverError::Refused(r) if r.code == RefusalCode::GrblPreflight && r.message.contains("RX buffer")), "{err:?}");
    }

    #[test]
    fn replies_are_classified() {
        assert_eq!(classify("ok"), Reply::Ok);
        assert_eq!(classify("error:9"), Reply::Error("9".into()));
        assert_eq!(classify("ALARM:3"), Reply::Alarm("3".into()));
        assert_eq!(classify("<Hold:0|MPos:1.000,2.000,0.000|FS:0,0>"), Reply::Status { state: "Hold:0".into(), position: "MPos:1.000,2.000,0.000".into() });
        assert_eq!(classify("Grbl 1.1h ['$' for help]"), Reply::Banner);
        assert_eq!(classify("[MSG:Reset to continue]"), Reply::Other);
        assert_eq!(program_lines("; c\n\n G1 X1 \n(note)\nM5\n"), ["G1 X1", "M5"]);
    }
}

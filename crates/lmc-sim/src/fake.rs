//! The fake LMC board and its emission recorder.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_lmc::{LmcJob, LmcTransport, Packet, Single, XferError, op};
use openlaser_machine::GalvoMachine;
use openlaser_types::{Clock, EmittedSegment, MonoTime};

#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    /// Packets the board queues before READY drops; swept 1..=8 until H2.
    pub buffer_depth: u8,
    /// 0 (virtual) or 2 ms (real-time T6 variant).
    pub transfer_latency: Duration,
    /// e.g. 0x0200 (an idle 0x0220 board).
    pub status_extra_bits: u16,
    /// 0x9899; 0x9980 = clone board (refused by detect/open).
    pub pid: u16,
    pub faults: Vec<Fault>,
    /// S23 stand-in for tests of the bench cap; None by default.
    pub emission_cap_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    StallWrites {
        after_packets: u32,
    },
    ReadyNeverReturns {
        after_execute: bool,
    },
    BusyStuck,
    AxisNeverStops,
    DisconnectMidPacket {
        packet: u32,
    },
    /// A MODEL until H23 measures the board.
    PartialPacketAfterCancel,
    ReplyTimeouts {
        every: u32,
    },
}

/// Board model behind `LmcTransport`: READY/BUSY from the galvo time model, MO and port state, list execution,
/// and an emission recorder that decodes the wire independently (it uses only `lmc::op` constants, not
/// `lmc::decode_packet`, so it is a second opinion on the encoder).
pub struct FakeLmc {
    cfg: SimConfig,
    clock: Arc<dyn Clock>,
    recorder: Arc<Recorder>,
    hook: Mutex<Option<WireHook>>,
    single_pending: Option<(bool, MonoTime, bool)>,
    packet_pending: Option<(Packet, MonoTime, bool)>,
    packet_count: u32,
    reply_count: u32,
    disconnected: bool,
    queue: std::collections::VecDeque<std::collections::VecDeque<[u16; 6]>>,
    active: Option<Active>,
    cursor: MonoTime,
    executing: bool,
    stopping: bool,
    ever_executed: bool,
    mo: bool,
    mo_since: Option<MonoTime>,
    port: u16,
    position: [f64; 2],
    mark_current: u16,
    mark_speed: u16,
    jump_speed: u16,
    jump_delay: u16,
    laser_on: u16,
    laser_off: u16,
    polygon: u16,
    field_mm: [f64; 2],
    axis_position: i32,
    axis_start: i32,
    axis_started: MonoTime,
    axis_target: i32,
    axis_until: Option<MonoTime>,
    axis_max_speed: u16,
    single_opcode: u16,
}

struct Active {
    words: [u16; 6],
    start: MonoTime,
    end: MonoTime,
    travel: Duration,
    from: [f64; 2],
    fraction: f64,
    burn: Option<usize>,
    lit: Option<usize>,
}

pub type WireHook = Box<dyn FnMut(&SimWire, MonoTime) + Send>;

impl FakeLmc {
    pub fn new(cfg: SimConfig, clock: Arc<dyn Clock>) -> Self {
        let cursor = clock.now();
        Self {
            cfg,
            clock,
            recorder: Arc::new(Recorder::default()),
            hook: Mutex::new(None),
            single_pending: None,
            packet_pending: None,
            packet_count: 0,
            reply_count: 0,
            disconnected: false,
            queue: std::collections::VecDeque::new(),
            active: None,
            cursor,
            executing: false,
            stopping: false,
            ever_executed: false,
            mo: false,
            mo_since: None,
            port: 0,
            position: [32768.0; 2],
            mark_current: 0,
            mark_speed: 1,
            jump_speed: 830,
            jump_delay: 0,
            laser_on: 0,
            laser_off: 0,
            polygon: 0,
            field_mm: [236.68; 2],
            axis_position: 0,
            axis_start: 0,
            axis_started: cursor,
            axis_target: 0,
            axis_until: None,
            axis_max_speed: 1,
            single_opcode: 0,
        }
    }
    pub fn recorder(&self) -> Arc<Recorder> {
        Arc::clone(&self.recorder)
    }
    /// Invoked before each transfer's board effects, on the caller's thread.
    pub fn set_hook(&self, hook: WireHook) {
        *self.hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }
    /// The effective field F per axis (machine `field_f()`), used to report emission in mm. Default 236.68.
    pub fn set_field_mm(&mut self, field_mm: [f64; 2]) {
        self.field_mm = field_mm;
    }
    fn check(&self) -> Result<(), XferError> {
        if self.disconnected {
            Err(XferError::Disconnected)
        } else if self.cfg.pid != 0x9899 {
            Err(XferError::Io("unsupported simulated PID".into()))
        } else if !(1..=8).contains(&self.cfg.buffer_depth) {
            Err(XferError::Io("buffer_depth must be 1..=8".into()))
        } else {
            Ok(())
        }
    }
    fn ready(&self) -> bool {
        !self.cfg.faults.iter().any(|f| matches!(f,Fault::ReadyNeverReturns{after_execute} if !after_execute || self.ever_executed))
            && self.queue.len() < usize::from(self.cfg.buffer_depth)
            && self.packet_pending.is_none()
    }
    /// `status_extra_bits` adds unknown bits (an idle 0x0220 board); it can never claim READY or BUSY, which come only
    /// from the board model.
    fn status(&self) -> u16 {
        self.cfg.status_extra_bits & !(openlaser_lmc::StatusWord::READY | openlaser_lmc::StatusWord::BUSY | openlaser_lmc::StatusWord::AXIS)
            | if self.ready() { openlaser_lmc::StatusWord::READY } else { 0 }
            | if self.executing { openlaser_lmc::StatusWord::BUSY } else { 0 }
            | if self.axis_moving() { openlaser_lmc::StatusWord::AXIS } else { 0 }
    }
    fn axis_moving(&self) -> bool {
        self.axis_until.is_some_and(|until| self.clock.now() < until || self.cfg.faults.contains(&Fault::AxisNeverStops))
    }
    fn notify(&self, event: SimWire) {
        if let Some(h) = self.hook.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            h(&event, self.clock.now());
        }
        self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner()).wire.push(event);
    }
    fn note_not_ready_write(&self) {
        let mut r = self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner());
        r.writes_while_not_ready = r.writes_while_not_ready.saturating_add(1);
    }
    fn set_mo(&mut self, open: bool, at: MonoTime) {
        let mut r = self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner());
        if open && !self.mo {
            self.mo_since = Some(at);
        }
        if !open && self.mo {
            if let Some(start) = self.mo_since.take() {
                r.mo_open.push((start, at));
            }
            if self.executing && !self.stopping {
                r.mo_closed_while_busy = r.mo_closed_while_busy.saturating_add(1);
            }
        }
        self.mo = open;
        r.mo = open;
        r.mo_open_since = self.mo_since;
    }
    fn set_port(&mut self, port: u16) {
        self.port = port;
        self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner()).port = port;
    }
    // Decode the raw wire independently: no lmc decoder, disassembler or plan estimator.
    fn accept(&mut self, packet: &Packet, slots: usize) {
        let mut words = std::collections::VecDeque::new();
        for slot in packet.bytes().as_chunks::<12>().0.iter().take(slots) {
            let mut w = [0u16; 6];
            for (word, pair) in w.iter_mut().zip(slot.as_chunks::<2>().0) {
                *word = u16::from_le_bytes(*pair);
            }
            words.push_back(w);
            if w[0] == op::LIST_END_OF_LIST {
                break;
            }
        }
        if self.queue.is_empty() && self.active.is_none() {
            self.cursor = self.clock.now();
        }
        self.queue.push_back(words);
    }
    fn duration(&self, w: [u16; 6]) -> Result<Duration, XferError> {
        let seconds = match w[0] {
            op::LIST_DELAY_TIME | op::LIST_LASER_ON_POINT => f64::from(w[1]) * 0.00001,
            op::LIST_JUMP_TO | op::LIST_MARK_TO => {
                let dx = f64::from(w[1]) - self.position[0];
                let dy = f64::from(w[2]) - self.position[1];
                let mark = w[0] == op::LIST_MARK_TO;
                let speed = if mark { self.mark_speed } else { self.jump_speed };
                if speed == 0 {
                    return Err(XferError::Io("motion with zero speed".into()));
                }
                dx.hypot(dy) / f64::from(speed) / 1000.0
                    + if mark {
                        (f64::from(self.laser_on) + f64::from(self.laser_off) + f64::from(self.polygon)) * 0.000001
                    } else {
                        f64::from(self.jump_delay) * 0.000001
                    }
            }
            _ => 0.0,
        };
        Duration::try_from_secs_f64(seconds).map_err(|_| XferError::Io("invalid simulated command duration".into()))
    }
    fn pump(&mut self) -> Result<(), XferError> {
        if let Some(until) = self.axis_until {
            let total = until.saturating_since(self.axis_started).as_secs_f64();
            let fraction = if total == 0.0 { 1.0 } else { (self.clock.now().saturating_since(self.axis_started).as_secs_f64() / total).min(1.0) };
            self.axis_position = (f64::from(self.axis_start) + (f64::from(self.axis_target) - f64::from(self.axis_start)) * fraction).round() as i32;
        }
        if self.axis_until.is_some() && !self.axis_moving() {
            self.axis_position = self.axis_target;
            self.axis_until = None;
        }
        {
            let mut r = self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner());
            r.axis_position = self.axis_position;
            r.axis_target = self.axis_target;
            r.axis_moving = self.axis_moving();
        }

        if !self.executing {
            return Ok(());
        }
        let now = self.clock.now();
        loop {
            if let Some(mut active) = self.active.take() {
                self.record_progress(&mut active, now);
                if active.end > now {
                    self.active = Some(active);
                    break;
                }
                self.cursor = active.end;
                self.apply(active.words, active.end);
            }
            if !self.executing {
                break;
            }
            while self.queue.front().is_some_and(|p| p.is_empty()) {
                self.queue.pop_front();
            }
            let Some(w) = self.queue.front_mut().and_then(|p| p.pop_front()) else {
                break;
            };
            if w[0] == op::LIST_END_OF_LIST {
                self.queue.pop_front();
                if !self.cfg.faults.contains(&Fault::BusyStuck) {
                    self.executing = false;
                }
                break;
            }
            let end = self.cursor.plus(self.duration(w)?);
            let motion = matches!(w[0], op::LIST_MARK_TO | op::LIST_JUMP_TO);
            let speed = if w[0] == op::LIST_MARK_TO { self.mark_speed } else { self.jump_speed };
            let travel = if motion {
                let distance = (f64::from(w[1]) - self.position[0]).hypot(f64::from(w[2]) - self.position[1]);
                Duration::try_from_secs_f64(distance / f64::from(speed) / 1000.0).map_err(|_| XferError::Io("invalid simulated motion".into()))?
            } else {
                Duration::ZERO
            };
            self.active = Some(Active { words: w, start: self.cursor, end, travel, from: self.position, fraction: 0.0, burn: None, lit: None });
        }
        Ok(())
    }
    /// Record the travelled part of a command, including motion interrupted by a stop or MO close.
    /// Successive polls extend one segment; changing the emission gate starts a separate segment.
    fn record_progress(&mut self, a: &mut Active, now: MonoTime) {
        let w = a.words;
        if w[0] == op::LIST_LASER_ON_POINT {
            // The laser fires in place for the command's duration: a zero-length burn where the galvo is.
            if a.burn.is_none() && self.mo && self.port & 1 != 0 {
                let here = [(self.position[0] - 32768.0) * self.field_mm[0] / 65535.0, (self.position[1] - 32768.0) * self.field_mm[1] / 65535.0];
                let mut r = self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner());
                a.burn = Some(r.burned.len());
                r.burned.push(EmittedSegment { a: here, b: here, power_pct: f64::from(self.mark_current) * 100.0 / 4095.0 });
            }
            return;
        }
        if !matches!(w[0], op::LIST_MARK_TO | op::LIST_JUMP_TO) {
            return;
        }
        let fraction = if a.travel.is_zero() { 1.0 } else { (now.saturating_since(a.start).as_secs_f64() / a.travel.as_secs_f64()).min(1.0) };
        if fraction <= a.fraction {
            return;
        }
        let to = [f64::from(w[1]), f64::from(w[2])];
        let at = |f: f64| [a.from[0] + (to[0] - a.from[0]) * f, a.from[1] + (to[1] - a.from[1]) * f];
        let cv = |p: [f64; 2]| [(p[0] - 32768.0) * self.field_mm[0] / 65535.0, (p[1] - 32768.0) * self.field_mm[1] / 65535.0];
        let begin = cv(at(a.fraction));
        let end = cv(at(fraction));
        let mut r = self.recorder.recording.lock().unwrap_or_else(|e| e.into_inner());
        if w[0] == op::LIST_MARK_TO && self.mo && self.port & 1 != 0 {
            if let Some(segment) = a.burn.and_then(|i| r.burned.get_mut(i)) {
                segment.b = end;
            } else {
                a.burn = Some(r.burned.len());
                r.burned.push(EmittedSegment { a: begin, b: end, power_pct: f64::from(self.mark_current) * 100.0 / 4095.0 });
            }
        } else {
            a.burn = None;
        }
        if w[0] == op::LIST_JUMP_TO && self.port & 256 != 0 {
            if let Some(segment) = a.lit.and_then(|i| r.lit.get_mut(i)) {
                segment[1] = end;
            } else {
                a.lit = Some(r.lit.len());
                r.lit.push([begin, end]);
            }
        } else {
            a.lit = None;
        }
        self.position = at(fraction);
        a.fraction = fraction;
    }
    fn apply(&mut self, w: [u16; 6], at: MonoTime) {
        match w[0] {
            op::LIST_WRITE_PORT => self.set_port(w[1]),
            op::LIST_MARK_CURRENT => self.mark_current = w[1],
            op::LIST_MARK_SPEED => self.mark_speed = w[1],
            op::LIST_JUMP_SPEED => self.jump_speed = w[1],
            op::LIST_JUMP_DELAY => self.jump_delay = w[1],
            op::LIST_LASER_ON_DELAY => self.laser_on = w[1],
            op::LIST_LASER_OFF_DELAY => self.laser_off = w[1],
            op::LIST_POLYGON_DELAY => self.polygon = w[1],
            op::LIST_FIBER_OPEN_MO => self.set_mo(w[1] != 0, at),
            op::LIST_JUMP_TO | op::LIST_MARK_TO => self.position = [f64::from(w[1]), f64::from(w[2])],
            _ => {}
        }
    }
}

impl LmcTransport for FakeLmc {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        self.check()?;
        self.pump()?;
        if self.single_pending.is_some() || self.packet_pending.is_some() {
            return Err(XferError::Io("transfer already in flight".into()));
        }
        let at = self.clock.now();
        self.notify(SimWire::Single { at, words: cmd.0 });
        self.single_opcode = cmd.0[0];
        match cmd.0[0] {
            op::SET_AXIS_MOTION_PARAM => {
                if cmd.0[1] == 0 || cmd.0[2] < cmd.0[1] {
                    return Err(XferError::Io("invalid rotary speed parameters".into()));
                }
                self.axis_max_speed = cmd.0[2];
            }
            op::SET_AXIS_ORIGIN_PARAM if !(1..=10000).contains(&cmd.0[1]) => return Err(XferError::Io("invalid rotary acceleration parameter".into())),
            op::MOVE_AXIS_TO => {
                if self.executing || self.mo || self.port != 0 || self.axis_moving() {
                    return Err(XferError::Io("unsafe rotary move while list, MO, ports or axis active".into()));
                }
                self.axis_start = self.axis_position;
                self.axis_started = at;
                self.axis_target = openlaser_lmc::axis_position([0, cmd.0[1], cmd.0[2], 0]);
                let delta = (i64::from(self.axis_target) - i64::from(self.axis_position)).unsigned_abs();
                let duration = Duration::try_from_secs_f64(delta as f64 / f64::from(self.axis_max_speed)).map_err(|e| XferError::Io(e.to_string()))?;
                self.axis_until = Some(at.plus(duration));
            }
            op::FIBER_SET_MO => {
                if cmd.0[1] != 0 && self.axis_moving() {
                    return Err(XferError::Io("MO opened during rotary motion".into()));
                }
                self.set_mo(cmd.0[1] != 0, at);
            }
            op::WRITE_PORT => self.set_port(cmd.0[1]),
            op::EXECUTE_LIST => {
                self.executing = true;
                self.stopping = false;
                self.ever_executed = true;
                self.cursor = at;
            }
            op::STOP_EXECUTE | op::RESET_LIST | op::RESET => {
                self.executing = false;
                self.stopping = true;
                self.active = None;
                self.queue.clear();
                if cmd.0[0] == op::RESET {
                    self.set_mo(false, at);
                    self.set_port(0);
                }
            }
            _ => {}
        }
        let reply = cmd.reads_reply();
        if reply {
            self.reply_count = self.reply_count.saturating_add(1);
        }
        let timeout = reply && self.cfg.faults.iter().any(|f| matches!(f,Fault::ReplyTimeouts{every} if *every>0 && self.reply_count.is_multiple_of(*every)));
        self.single_pending = Some((reply, at.plus(self.cfg.transfer_latency), timeout));
        Ok(())
    }
    /// Like a USB `wait_next_complete(slice)`: returns as soon as the reply has arrived (`transfer_latency` after
    /// the submit), and waits the whole slice only while it has not (no transfer pending, or a reply that never
    /// comes).
    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        self.check()?;
        let wait = match self.single_pending {
            Some((_, until, false)) => until.saturating_since(self.clock.now()).min(slice),
            _ => slice,
        };
        self.clock.sleep(wait);
        self.pump()?;
        let Some((reply, until, timeout)) = self.single_pending else { return Ok(None) };
        if timeout || self.clock.now() < until {
            return Ok(None);
        }
        self.single_pending = None;
        if !reply {
            return Ok(Some(None));
        }
        let status = self.status();
        self.notify(SimWire::Read { at: self.clock.now(), status });
        let reply = if self.single_opcode == op::GET_AXIS_POS {
            let word = self.axis_position.unsigned_abs() | if self.axis_position < 0 { 0x8000_0000 } else { 0 };
            let [a, b, c, d] = word.to_le_bytes();
            [0, u16::from_le_bytes([a, b]), u16::from_le_bytes([c, d]), status]
        } else {
            [0, 0, 0, status]
        };
        Ok(Some(Some(reply)))
    }
    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        self.check()?;
        self.pump()?;
        if self.single_pending.is_some() {
            return Err(XferError::Io("single transfer in flight".into()));
        }
        // The OUT transfer goes on the wire whatever the board's state, so it is recorded first: a write into a board
        // that is not READY shows in `wire` as well as in `writes_while_not_ready` (it is then refused).
        let at = self.clock.now();
        self.notify(SimWire::Packet { at, sha256: pkt.sha256_hex() });
        if !self.ready() {
            self.note_not_ready_write();
            return Err(XferError::Io("packet submitted while not READY".into()));
        }
        self.packet_count = self.packet_count.saturating_add(1);
        if self.cfg.faults.iter().any(|f| matches!(f,Fault::StallWrites{after_packets} if self.packet_count>*after_packets)) {
            return Err(XferError::Stall);
        }
        let disconnect = self.cfg.faults.iter().any(|f| matches!(f,Fault::DisconnectMidPacket{packet} if self.packet_count==*packet));
        self.packet_pending = Some((pkt.clone(), at.plus(self.cfg.transfer_latency), disconnect));
        Ok(())
    }
    /// Returns as soon as the packet transfer has completed (`transfer_latency` after the submit), never later than
    /// the slice; with nothing in flight it returns at once.
    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        self.check()?;
        let wait = self.packet_pending.as_ref().map_or(Duration::ZERO, |(_, until, _)| until.saturating_since(self.clock.now()).min(slice));
        self.clock.sleep(wait);
        self.pump()?;
        let Some((_, until, _)) = &self.packet_pending else { return Ok(Some(())) };
        if self.clock.now() < *until {
            return Ok(None);
        }
        if let Some((pkt, _, disconnect)) = self.packet_pending.take() {
            if disconnect {
                self.disconnected = true;
                return Err(XferError::Disconnected);
            }
            self.accept(&pkt, 256);
            self.pump()?;
        }
        Ok(Some(()))
    }
    fn cancel_all(&mut self) {
        if let Some((pkt, _, _)) = self.packet_pending.take()
            && self.cfg.faults.contains(&Fault::PartialPacketAfterCancel)
        {
            self.accept(&pkt, 128);
        }
        self.single_pending = None;
    }
    fn drain_replies(&mut self) {
        self.single_pending = None;
    }
}

/// One wire event with its virtual timestamp (for T6 ordering asserts).
#[derive(Debug, Clone, PartialEq)]
pub enum SimWire {
    Single { at: MonoTime, words: [u16; 6] },
    Packet { at: MonoTime, sha256: String },
    Read { at: MonoTime, status: u16 },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recording {
    pub axis_position: i32,
    pub axis_target: i32,
    pub axis_moving: bool,
    /// Offline simulation refuses malformed jobs explicitly; a partial trace is never presented as success.
    pub error: Option<XferError>,
    /// single/packet/read with virtual timestamps (for T6 ordering asserts).
    pub wire: Vec<SimWire>,
    /// MarkTo while executing, MO open, port bit 0 set (and LaserOnPoint, as a zero-length segment).
    pub burned: Vec<EmittedSegment>,
    /// Red-light traces (port bit 8 set, JumpTo).
    pub lit: Vec<[[f64; 2]; 2]>,
    /// Closed MO-open intervals.
    pub mo_open: Vec<(MonoTime, MonoTime)>,
    /// The start of an MO-open interval that has not closed yet (`None` while MO is closed).
    pub mo_open_since: Option<MonoTime>,
    /// The board's current (at the end: final) MO state.
    pub mo: bool,
    /// The board's current (at the end: final) output port word.
    pub port: u16,
    /// Must stay 0.
    pub writes_while_not_ready: u32,
    /// Must stay 0 unless stopping.
    pub mo_closed_while_busy: u32,
}

#[derive(Debug, Default)]
pub struct Recorder {
    recording: Mutex<Recording>,
}

impl Recorder {
    pub fn snapshot(&self) -> Recording {
        self.recording.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Offline run of an encoded job (no driver thread): what `job_simulate` and `openlaser-cli simulate` show.
fn replay_axis(board: &mut FakeLmc, mv: &openlaser_lmc::AxisMove, clock: &Arc<openlaser_types::VirtualClock>) -> Result<(), XferError> {
    if !mv.valid() {
        return Err(XferError::Io("invalid simulation axis move".into()));
    }
    for cmd in openlaser_lmc::axis_singles(mv) {
        board.submit_single(cmd)?;
        board.poll_single(Duration::ZERO)?;
    }
    loop {
        board.submit_single(Single([op::GET_VERSION, 0, 0, 0, 0, 0]))?;
        board.poll_single(Duration::ZERO)?;
        if !board.axis_moving() {
            break;
        }
        clock.advance(Duration::from_millis(10));
        board.pump()?;
    }
    Ok(())
}

/// Replays the supplied prelude, packets and postlude against an isolated in-memory board.
pub fn simulate_job(job: &LmcJob, m: &GalvoMachine) -> Recording {
    let clock = Arc::new(openlaser_types::VirtualClock::new());
    let mut board = FakeLmc::new(
        SimConfig { buffer_depth: 3, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid: 0x9899, faults: vec![], emission_cap_pct: None },
        clock.clone(),
    );
    board.set_field_mm(m.field_f());
    let run = |board: &mut FakeLmc| -> Result<(), XferError> {
        if job.repeats == 0 || job.lists.is_empty() {
            return Err(XferError::Io("empty simulation job".into()));
        }
        for list in &job.lists {
            if list.packets.last().is_none_or(|p| p.real_commands() == 256) {
                return Err(XferError::Io("simulation list has no end-of-list command".into()));
            }
            if list.packets.iter().take(list.packets.len().saturating_sub(1)).any(|p| p.real_commands() != 256) {
                return Err(XferError::Io("simulation list ends before its last packet".into()));
            }
        }
        for _ in 0..job.repeats {
            for list in &job.lists {
                if let Some(mv) = list.axis_move {
                    replay_axis(board, &mv, &clock)?;
                }
                for cmd in &job.prelude {
                    board.submit_single(*cmd)?;
                    board.poll_single(Duration::ZERO)?;
                }
                for (i, packet) in list.packets.iter().enumerate() {
                    while !board.ready() {
                        let wait = board.active.as_ref().map_or(Duration::from_millis(1), |a| a.end.saturating_since(clock.now()));
                        clock.advance(wait);
                        board.pump()?;
                    }
                    board.submit_packet(packet)?;
                    board.poll_packet(Duration::ZERO)?;
                    board.submit_single(Single([op::SET_END_OF_LIST, 0, 0, 0, 0, 0]))?;
                    board.poll_single(Duration::ZERO)?;
                    if i == 0 {
                        board.submit_single(Single([op::EXECUTE_LIST, 0, 0, 0, 0, 0]))?;
                        board.poll_single(Duration::ZERO)?;
                    }
                }
                while board.executing {
                    let wait = board.active.as_ref().map_or(Duration::from_millis(1), |a| a.end.saturating_since(clock.now()));
                    clock.advance(wait);
                    board.pump()?;
                }
                for cmd in &job.postlude {
                    board.submit_single(*cmd)?;
                    board.poll_single(Duration::ZERO)?;
                }
            }
        }
        if let Some(mv) = job.return_axis {
            replay_axis(board, &mv, &clock)?;
        }
        Ok(())
    };
    if let Err(error) = run(&mut board) {
        board.executing = false;
        board.set_mo(false, clock.now());
        board.set_port(0);
        board.recorder.recording.lock().unwrap_or_else(|e| e.into_inner()).error = Some(error);
    }
    board.recorder().snapshot()
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;

//! Decoding, disassembly and summaries derived from the exact bytes sent.
//!
//! Opcode layouts follow galvoplotter 0.2.0 `galvo/controller.py` (MIT).

//! Protocol source: meerk40t/galvoplotter@fc35136 galvo/controller.py (0.2.0), MIT.

/*
MIT License

Copyright (c) 2023 MeerK40t

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

use std::time::Duration;

use openlaser_machine::GalvoMachine;
use openlaser_plan::{CutQ, JobPlan};
use openlaser_types::galvo::{CENTER, distance_word, units_to_speed};
use openlaser_types::{Bounds, DeviceKind, EmissionSummary, EmittedSegment, GalvoWords, Notice, PassRow};

use crate::Error;
use crate::encode::{LmcFrameJob, LmcJob, MARK_CURRENT_MAX, job_postlude, job_prelude};
use crate::op;
use crate::packet::{Packet, Single};

/// An unused slot: `listEndOfList` with zero parameters (also the explicit EndOfList the encoder writes).
const PAD: [u16; 6] = [op::LIST_END_OF_LIST, 0, 0, 0, 0, 0];

#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    ReadyMark,
    DelayTime(u16),
    WritePort(u16),
    JumpSpeed(u16),
    MarkSpeed(u16),
    PulseWidth(u16),
    MarkCurrent(u16),
    QSwitchPeriod(u16),
    LaserOnDelay(u16, u16),
    LaserOffDelay(u16, u16),
    PolygonDelay(u16, u16),
    JumpDelay(u16, u16),
    JumpTo { x: u16, y: u16, dist: u16 },
    MarkTo { x: u16, y: u16, dist: u16 },
    EndOfList,
    Other([u16; 6]),
}

fn decode_words(w: [u16; 6]) -> Decoded {
    let [opcode, a, b, _, dist, _] = w;
    match opcode {
        op::LIST_READY_MARK => Decoded::ReadyMark,
        op::LIST_DELAY_TIME => Decoded::DelayTime(a),
        op::LIST_WRITE_PORT => Decoded::WritePort(a),
        op::LIST_JUMP_SPEED => Decoded::JumpSpeed(a),
        op::LIST_MARK_SPEED => Decoded::MarkSpeed(a),
        op::LIST_FIBER_YLPM_PULSE_WIDTH => Decoded::PulseWidth(a),
        op::LIST_MARK_CURRENT => Decoded::MarkCurrent(a),
        op::LIST_Q_SWITCH_PERIOD => Decoded::QSwitchPeriod(a),
        op::LIST_LASER_ON_DELAY => Decoded::LaserOnDelay(a, b),
        op::LIST_LASER_OFF_DELAY => Decoded::LaserOffDelay(a, b),
        op::LIST_POLYGON_DELAY => Decoded::PolygonDelay(a, b),
        op::LIST_JUMP_DELAY => Decoded::JumpDelay(a, b),
        op::LIST_JUMP_TO => Decoded::JumpTo { x: a, y: b, dist },
        op::LIST_MARK_TO => Decoded::MarkTo { x: a, y: b, dist },
        op::LIST_END_OF_LIST => Decoded::EndOfList,
        _ => Decoded::Other(w),
    }
}

/// Stops at the first end-of-list or padding command.
pub fn decode_packet(p: &Packet) -> Vec<Decoded> {
    let mut out = Vec::new();
    for w in p.words() {
        if w[0] == op::LIST_END_OF_LIST {
            break;
        }
        out.push(decode_words(w));
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListingLine {
    pub list: Option<u32>,
    /// `None` for a single command (a list's prelude or postlude).
    pub packet: Option<u32>,
    pub slot: Option<u16>,
    pub words: [u16; 6],
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Listing {
    pub lines: Vec<ListingLine>,
}

fn point_mm(w: u16, field: f64) -> f64 {
    (f64::from(w) - 32768.0) * field / 65535.0
}

fn point(p: (u16, u16), field: [f64; 2]) -> [f64; 2] {
    [point_mm(p.0, field[0]), point_mm(p.1, field[1])]
}

fn decoded_text(d: &Decoded, m: &GalvoMachine) -> String {
    let [fx, fy] = m.field_f();
    match *d {
        Decoded::ReadyMark => "ReadyMark".into(),
        Decoded::DelayTime(v) => format!("DelayTime({v})"),
        Decoded::WritePort(v) => format!("WritePort({v})"),
        Decoded::JumpSpeed(v) => format!("JumpSpeed({v})"),
        Decoded::MarkSpeed(v) => format!("MarkSpeed({v})"),
        Decoded::PulseWidth(v) => format!("PulseWidth({v})"),
        Decoded::MarkCurrent(v) => format!("MarkCurrent({v})"),
        Decoded::QSwitchPeriod(v) => format!("QSwitchPeriod({v})"),
        Decoded::LaserOnDelay(a, b) => format!("LaserOnDelay({a},{b})"),
        Decoded::LaserOffDelay(a, b) => format!("LaserOffDelay({a},{b})"),
        Decoded::PolygonDelay(a, b) => format!("PolygonDelay({a},{b})"),
        Decoded::JumpDelay(a, b) => format!("JumpDelay({a},{b})"),
        Decoded::JumpTo { x, y, dist } => format!("JumpTo x={x} y={y} ({:.3}, {:.3} mm) dist={dist}", point_mm(x, fx), point_mm(y, fy)),
        Decoded::MarkTo { x, y, dist } => format!("MarkTo x={x} y={y} ({:.3}, {:.3} mm) dist={dist}", point_mm(x, fx), point_mm(y, fy)),
        Decoded::EndOfList => "EndOfList".into(),
        Decoded::Other([opcode, params @ ..]) => format!("{}({params:?})", op::list_name(opcode)),
    }
}

/// Single commands that take no parameter (every other one shows at least its first, even when it is 0).
fn parameterless(opcode: u16) -> bool {
    matches!(
        opcode,
        op::RESET_LIST
            | op::GET_VERSION
            | op::GET_SERIAL_NO
            | op::EXECUTE_LIST
            | op::SET_END_OF_LIST
            | op::STOP_EXECUTE
            | op::STOP_LIST
            | op::RESTART_LIST
            | op::RESET
            | op::ENABLE_LASER
            | op::DISABLE_LASER
            | op::ENABLE_Z
            | op::DISABLE_Z
            | op::GET_LIST_STATUS
            | op::GET_POSITION_XY
            | op::READ_PORT
    )
}

/// e.g. "Fiber_SetMo(0)", "WritePort(1)", "GetMarkTime(3)", "ResetList": the name, then the parameters up to the last
/// non-zero one.
fn single_text(words: [u16; 6]) -> String {
    let [opcode, params @ ..] = words;
    let floor = usize::from(!parameterless(opcode));
    let used = params.iter().rposition(|p| *p != 0).map_or(0, |i| i + 1).max(floor);
    let name = op::single_name(opcode);
    match params.get(..used) {
        Some(shown) if !shown.is_empty() => format!("{name}({})", shown.iter().map(u16::to_string).collect::<Vec<_>>().join(",")),
        _ => name.to_owned(),
    }
}

fn add_singles(out: &mut Listing, singles: &[Single], list_index: Option<u32>) {
    for s in singles {
        out.lines.push(ListingLine { list: list_index, packet: None, slot: None, words: s.0, text: single_text(s.0) });
    }
}

/// Every slot up to and including the first EndOfList, and any slot after it that is not plain padding (so a stray
/// command hidden behind an EndOfList is shown, not skipped). A full packet lists all 256 slots.
fn add_packets(out: &mut Listing, packets: &[Packet], list_index: Option<u32>, m: &GalvoMachine) {
    for (pi, packet) in packets.iter().enumerate() {
        let words = packet.words();
        let eol = words.iter().position(|w| w[0] == op::LIST_END_OF_LIST);
        let last_real = words.iter().rposition(|w| *w != PAD);
        let end = match (eol, last_real) {
            (Some(e), Some(l)) => e.max(l) + 1,
            (Some(e), None) => e + 1,
            (None, _) => words.len(),
        };
        for (si, words) in words.into_iter().enumerate().take(end) {
            out.lines.push(ListingLine {
                list: list_index,
                packet: u32::try_from(pi).ok(),
                slot: u16::try_from(si).ok(),
                words,
                text: decoded_text(&decode_words(words), m),
            });
        }
    }
}

/// Per list, in send order: its prelude singles, every packet, its postlude singles.
pub fn disassemble(job: &LmcJob, m: &GalvoMachine) -> Listing {
    let mut out = Listing::default();
    for (i, list) in job.lists.iter().enumerate() {
        let index = u32::try_from(i).ok();
        add_singles(&mut out, &job.prelude, index);
        add_packets(&mut out, &list.packets, index, m);
        add_singles(&mut out, &job.postlude, index);
    }
    out
}
pub fn disassemble_frame(job: &LmcFrameJob, m: &GalvoMachine) -> Listing {
    let mut out = Listing::default();
    add_packets(&mut out, &job.once, Some(0), m);
    add_packets(&mut out, &job.pass, Some(1), m);
    out
}

/// The nominal LMC time model (the one FakeLmc runs), carried across the packets of one list starting at the centre:
/// DelayTime and LaserOnPoint n × 10 µs; JumpTo distance / JumpSpeed (galvo units per ms) + JumpDelay µs; MarkTo
/// distance / MarkSpeed + laser-on + laser-off + polygon delays µs. Delay magnitudes count whatever their sign word,
/// and a motion before its speed is set (or at 0 units) is timed at 1 unit/ms, the slowest a speed word can say: an
/// upper bound wherever the words leave the board's state open. Real galvos also accelerate (H1, H25 measure it).
pub(crate) fn packet_times(packets: &[Packet]) -> Vec<Duration> {
    let mut model = TimeModel::default();
    packets.iter().map(|p| model.packet(p)).collect()
}

struct TimeModel {
    pos: [f64; 2],
    jump_speed: u16,
    mark_speed: u16,
    jump_delay: u16,
    laser_on: u16,
    laser_off: u16,
    polygon: u16,
}

impl Default for TimeModel {
    fn default() -> Self {
        let c = f64::from(CENTER);
        TimeModel { pos: [c, c], jump_speed: 0, mark_speed: 0, jump_delay: 0, laser_on: 0, laser_off: 0, polygon: 0 }
    }
}

impl TimeModel {
    fn packet(&mut self, p: &Packet) -> Duration {
        let secs: f64 = p.words().into_iter().map(|w| self.command(w)).sum();
        Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX)
    }

    fn command(&mut self, w: [u16; 6]) -> f64 {
        const TEN_US: f64 = 1e-5;
        const US: f64 = 1e-6;
        let [opcode, a, b, ..] = w;
        match opcode {
            op::LIST_DELAY_TIME | op::LIST_LASER_ON_POINT => f64::from(a) * TEN_US,
            op::LIST_JUMP_SPEED => {
                self.jump_speed = a;
                0.0
            }
            op::LIST_MARK_SPEED => {
                self.mark_speed = a;
                0.0
            }
            op::LIST_JUMP_DELAY => {
                self.jump_delay = a;
                0.0
            }
            op::LIST_LASER_ON_DELAY => {
                self.laser_on = a;
                0.0
            }
            op::LIST_LASER_OFF_DELAY => {
                self.laser_off = a;
                0.0
            }
            op::LIST_POLYGON_DELAY => {
                self.polygon = a;
                0.0
            }
            op::LIST_JUMP_TO | op::LIST_MARK_TO => {
                let to = [f64::from(a), f64::from(b)];
                let [px, py] = self.pos;
                let dist = (to[0] - px).hypot(to[1] - py);
                self.pos = to;
                let (speed, delays) = if opcode == op::LIST_MARK_TO {
                    (self.mark_speed, f64::from(self.laser_on) + f64::from(self.laser_off) + f64::from(self.polygon))
                } else {
                    (self.jump_speed, f64::from(self.jump_delay))
                };
                dist / f64::from(speed.max(1)) / 1000.0 + delays * US
            }
            _ => 0.0,
        }
    }
}

/// The real commands of one list, after checking the framing the board relies on: every packet but the last holds
/// 256 real commands (no EndOfList anywhere), and the last holds an explicit EndOfList followed only by padding.
fn framed_commands(packets: &[Packet]) -> Result<Vec<[u16; 6]>, Error> {
    let Some((last, full)) = packets.split_last() else { return Err(Error::SummaryMismatch) };
    let mut out = Vec::new();
    for p in full {
        let words = p.words();
        if words.iter().any(|w| w[0] == op::LIST_END_OF_LIST) {
            return Err(Error::SummaryMismatch);
        }
        out.extend(words);
    }
    let words = last.words();
    let eol = words.iter().position(|w| w[0] == op::LIST_END_OF_LIST).ok_or(Error::SummaryMismatch)?;
    let (real, tail) = words.split_at_checked(eol).ok_or(Error::SummaryMismatch)?;
    if tail.iter().any(|w| *w != PAD) {
        return Err(Error::SummaryMismatch);
    }
    out.extend_from_slice(real);
    Ok(out)
}

/// The parameter block of a marking list (SPEC §5.2 rule 5): each word at most once.
#[derive(Default)]
struct Block {
    jump_speed: Option<u16>,
    pulse: Option<u16>,
    mark_current: Option<u16>,
    q_period: Option<u16>,
    mark_speed: Option<u16>,
    laser_on: Option<(u16, u16)>,
    laser_off: Option<(u16, u16)>,
    polygon: Option<(u16, u16)>,
}

impl Block {
    /// `Some` only once the whole block (the five galvo words and the three delays) has been read.
    fn words(&self) -> Option<GalvoWords> {
        self.laser_on.and(self.laser_off).and(self.polygon)?;
        Some(GalvoWords {
            mark_current: self.mark_current?,
            q_period: self.q_period?,
            pulse_ns: self.pulse?,
            speed_units: self.mark_speed?,
            jump_speed_units: self.jump_speed?,
        })
    }
}

/// A second value for a parameter fails closed: one row must describe every mark of its list.
fn once<T>(slot: &mut Option<T>, v: T) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::SummaryMismatch);
    }
    *slot = Some(v);
    Ok(())
}

/// The parameter words a command does not use must be 0 (what the board would do with them is unknown).
fn unused_zero(w: &[u16; 6], first_unused: usize) -> Result<(), Error> {
    if w.iter().skip(first_unused).all(|v| *v == 0) { Ok(()) } else { Err(Error::SummaryMismatch) }
}

/// A decoded galvo position lies in the rated work area, within one galvo unit of quantisation.
fn in_work_area(to: (u16, u16), m: &GalvoMachine) -> bool {
    let [fx, fy] = m.field_f();
    let inside = |w: u16, f: f64, lo: f64, hi: f64| {
        let v = point_mm(w, f);
        let unit = f / 65535.0;
        v >= lo - unit && v <= hi + unit
    };
    let [x_min, y_min] = m.work_area.min;
    let [x_max, y_max] = m.work_area.max;
    inside(to.0, fx, x_min, x_max) && inside(to.1, fy, y_min, y_max)
}

struct MarkingDecode {
    words: GalvoWords,
    paths: u32,
    mark_mm: f64,
    bounds: Bounds,
}

/// Strict decode of one marking list: exactly the grammar `encode_job` writes. ReadyMark, DelayTime, WritePort(laser
/// bit) and JumpDelay anywhere; the parameter block (JumpSpeed, PulseWidth, MarkCurrent, QSwitchPeriod, MarkSpeed,
/// LaserOn/Off/PolygonDelay) once each and complete before the first motion; the first motion a JumpTo; every `dist`
/// word GP's; every point inside the work area. Anything else (another opcode, a second value, a MarkTo before the
/// block, stray words after the EndOfList) is `SummaryMismatch`: the summary would not describe what the board does.
fn decode_marking_list(packets: &[Packet], m: &GalvoMachine) -> Result<MarkingDecode, Error> {
    let commands = framed_commands(packets)?;
    let laser = 1u16.checked_shl(u32::from(m.ports.laser_bit)).ok_or(Error::InvalidPort(m.ports.laser_bit))?;
    let field = m.field_f();
    let mut block = Block::default();
    let mut words: Option<GalvoWords> = None;
    let mut prev = (CENTER, CENTER);
    let mut jumped = false;
    let mut marking = false;
    let mut paths = 0u32;
    let mut mark_mm = 0.0;
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for w in &commands {
        let [opcode, a, b, ..] = *w;
        match opcode {
            op::LIST_READY_MARK => unused_zero(w, 1)?,
            op::LIST_DELAY_TIME => unused_zero(w, 2)?,
            op::LIST_WRITE_PORT if a == laser => unused_zero(w, 2)?,
            op::LIST_JUMP_DELAY => unused_zero(w, 3)?,
            op::LIST_JUMP_SPEED => unused_zero(w, 2).and_then(|()| once(&mut block.jump_speed, a))?,
            op::LIST_FIBER_YLPM_PULSE_WIDTH => unused_zero(w, 2).and_then(|()| once(&mut block.pulse, a))?,
            op::LIST_MARK_CURRENT => unused_zero(w, 2).and_then(|()| once(&mut block.mark_current, a))?,
            op::LIST_Q_SWITCH_PERIOD => unused_zero(w, 2).and_then(|()| once(&mut block.q_period, a))?,
            op::LIST_MARK_SPEED => unused_zero(w, 2).and_then(|()| once(&mut block.mark_speed, a))?,
            op::LIST_LASER_ON_DELAY => unused_zero(w, 3).and_then(|()| once(&mut block.laser_on, (a, b)))?,
            op::LIST_LASER_OFF_DELAY => unused_zero(w, 3).and_then(|()| once(&mut block.laser_off, (a, b)))?,
            op::LIST_POLYGON_DELAY => unused_zero(w, 3).and_then(|()| once(&mut block.polygon, (a, b)))?,
            op::LIST_JUMP_TO | op::LIST_MARK_TO => {
                if words.is_none() {
                    words = Some(block.words().ok_or(Error::SummaryMismatch)?);
                }
                let [_, x, y, z, dist, rest] = *w;
                let to = (x, y);
                if z != 0 || rest != 0 || dist != distance_word(prev, to) || !in_work_area(to, m) {
                    return Err(Error::SummaryMismatch);
                }
                if opcode == op::LIST_JUMP_TO {
                    jumped = true;
                    marking = false;
                } else {
                    if !jumped {
                        return Err(Error::SummaryMismatch);
                    }
                    if !marking {
                        paths = paths.checked_add(1).ok_or(Error::TooManyCommands)?;
                        marking = true;
                    }
                    let pa = point(prev, field);
                    let pb = point(to, field);
                    mark_mm += (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
                    for p in [pa, pb] {
                        for ((lo, hi), v) in min.iter_mut().zip(max.iter_mut()).zip(p) {
                            *lo = lo.min(v);
                            *hi = hi.max(v);
                        }
                    }
                }
                prev = to;
            }
            _ => return Err(Error::SummaryMismatch),
        }
    }
    let words = words.ok_or(Error::SummaryMismatch)?;
    if paths == 0 || words.q_period == 0 || words.speed_units == 0 || words.jump_speed_units == 0 || words.mark_current > MARK_CURRENT_MAX {
        return Err(Error::SummaryMismatch);
    }
    Ok(MarkingDecode { words, paths, mark_mm, bounds: Bounds { min, max } })
}

/// The largest sum of `window` consecutive estimates (the whole list when it is shorter).
fn max_window_sum(est: &[Duration], window: usize) -> Duration {
    let n = window.clamp(1, est.len().max(1));
    est.windows(n).map(|w| w.iter().fold(Duration::ZERO, |a, b| a.saturating_add(*b))).max().unwrap_or(Duration::ZERO)
}

/// Every segment a MarkTo (or LaserOnPoint, as a zero-length segment) would burn, in machine mm, at the MarkCurrent
/// last set in its list. Deliberately lenient and conservative, for the preview thumbnail: it walks every slot that
/// is not padding (also behind a stray EndOfList), and a mark before any MarkCurrent shows the worst case (word
/// 65535). `emission_summary` is the strict reading.
pub fn emitted_segments(job: &LmcJob, m: &GalvoMachine) -> Vec<EmittedSegment> {
    let field = m.field_f();
    let worst = f64::from(u16::MAX) * 100.0 / 4095.0;
    let mut out = Vec::new();
    let mut pos = (CENTER, CENTER);
    for list in &job.lists {
        let mut power: Option<f64> = None;
        for w in list.packets.iter().flat_map(Packet::words) {
            let [opcode, a, b, ..] = w;
            match opcode {
                op::LIST_MARK_CURRENT => power = Some(f64::from(a) * 100.0 / 4095.0),
                op::LIST_JUMP_TO => pos = (a, b),
                op::LIST_MARK_TO => {
                    out.push(EmittedSegment { a: point(pos, field), b: point((a, b), field), power_pct: power.unwrap_or(worst) });
                    pos = (a, b);
                }
                op::LIST_LASER_ON_POINT => {
                    let here = point(pos, field);
                    out.push(EmittedSegment { a: here, b: here, power_pct: power.unwrap_or(worst) });
                }
                _ => {}
            }
        }
    }
    out
}

/// The ARM confirmation (S22). Every number is decoded from `job`'s bytes with the strict list grammar; `plan`
/// supplies only names and notices, and must be the plan this job was encoded from: same fingerprint, machine,
/// device, repeats, one list per section with the section's layer and pass, and the section's quantised words equal
/// to the decoded ones. The prelude and postlude must be the standard ones. Anything else is `SummaryMismatch`.
///
/// `crash_bound` (PLAN §3.5): the emission the board can still run after a host crash is what the run-ahead window
/// lets it hold: at most `run_ahead.max_packets` (never < 1) consecutive packets of one list. Each packet counts at
/// the larger of its `packet_est` and its nominal time from its own bytes; the bound is the largest such window over
/// every list. Lists never overlap (each waits for !BUSY), so a window never spans two lists.
pub fn emission_summary(job: &LmcJob, m: &GalvoMachine, plan: &JobPlan, notices: &[Notice]) -> Result<EmissionSummary, Error> {
    if job.fingerprint != plan.fingerprint
        || job.machine != plan.machine
        || plan.device != DeviceKind::GalvoLmc
        || job.repeats != plan.repeats
        || job.repeats == 0
        || job.lists.is_empty()
        || job.lists.len() != plan.sections.len()
    {
        return Err(Error::SummaryMismatch);
    }
    let laser = 1u16.checked_shl(u32::from(m.ports.laser_bit)).ok_or(Error::InvalidPort(m.ports.laser_bit))?;
    if job.prelude != job_prelude(laser) || job.postlude != job_postlude() {
        return Err(Error::SummaryMismatch);
    }
    let window = usize::from(m.run_ahead.max_packets.max(1));
    let mut rows = Vec::with_capacity(job.lists.len());
    let mut crash_bound = Duration::ZERO;
    let mut one_repeat = Duration::ZERO;
    for (list, section) in job.lists.iter().zip(&plan.sections) {
        if list.layer != section.layer || list.pass != section.pass || list.packet_est.len() != list.packets.len() {
            return Err(Error::SummaryMismatch);
        }
        let CutQ::Galvo(q) = &section.cut else { return Err(Error::NotGalvo) };
        let d = decode_marking_list(&list.packets, m)?;
        let planned = GalvoWords {
            mark_current: q.mark_current,
            q_period: q.q_period,
            pulse_ns: q.pulse.word().ok_or(Error::SummaryMismatch)?,
            speed_units: q.speed_units,
            jump_speed_units: q.timings.jump_speed_units,
        };
        if d.words != planned {
            return Err(Error::SummaryMismatch);
        }
        let est: Vec<Duration> = packet_times(&list.packets).into_iter().zip(&list.packet_est).map(|(t, e)| t.max(*e)).collect();
        let estimate = est.iter().fold(Duration::ZERO, |a, b| a.saturating_add(*b));
        crash_bound = crash_bound.max(max_window_sum(&est, window));
        one_repeat = one_repeat.saturating_add(estimate);
        let w = d.words;
        rows.push(PassRow {
            layer: section.layer,
            layer_name: section.layer_name.clone(),
            sub: section.sub,
            pass: section.pass,
            of: section.of,
            mode: section.mode,
            power_pct: f64::from(w.mark_current) * 100.0 / 4095.0,
            frequency_khz: Some(20000.0 / f64::from(w.q_period)),
            pulse_ns: Some(w.pulse_ns),
            speed_mm_s: units_to_speed(w.speed_units, m.field_f()[0]),
            galvo: Some(w),
            paths: d.paths,
            mark_mm: d.mark_mm,
            bounds: d.bounds,
            estimate,
        });
    }
    if crash_bound.is_zero() {
        return Err(Error::SummaryMismatch);
    }
    let packets = job.lists.iter().map(|l| l.packets.len()).try_fold(0usize, usize::checked_add).ok_or(Error::TooManyCommands)?;
    Ok(EmissionSummary {
        fingerprint: job.fingerprint,
        bytes_hash: job.bytes_hash(),
        machine: job.machine.clone(),
        rows,
        notices: notices.to_vec(),
        repeats: job.repeats,
        estimate: plan.estimate.max(one_repeat.saturating_mul(u32::from(job.repeats))),
        crash_bound,
        packets: u32::try_from(packets).map_err(|_| Error::TooManyCommands)?,
        work_area: Bounds { min: m.work_area.min, max: m.work_area.max },
        negate: m.cal.negate,
        swap_xy: m.cal.swap_xy,
    })
}

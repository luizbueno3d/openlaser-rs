//! Pure, session-independent LMC list encoding (SPEC §5.2).
//!
//! Command ordering and packet filling are ports of galvoplotter 0.2.0
//! `galvo/controller.py` (MeerK40t contributors, MIT). OpenLaser's explicit
//! first jump, settle, leading lighting speed and session lifetime are the
//! reviewed divergences in SPEC §5.6.

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

use blake3::Hasher;
use openlaser_machine::GalvoMachine;
use openlaser_plan::{CutQ, Section};
use openlaser_safety::{EmitPlan, FramePlan};
use openlaser_types::galvo::{CENTER, EncodeError, delay_word, distance_word, mm_to_galvo, speed_to_units};
use openlaser_types::{DeviceKind, Fingerprint, FrameSource, FrameStyle, LayerKey, MachineId};

use crate::Error;
use crate::decode::packet_times;
use crate::op;
use crate::packet::{Packet, SLOTS, Single};

type Word = [u16; 6];

/// The MarkCurrent word for 100 % (`power_to_mark_current(100.0)`). A larger word asks for more than full power.
pub(crate) const MARK_CURRENT_MAX: u16 = 4095;

/// Built only from `EmitPlan` sections (SPEC §5.2 rule 9).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkingList {
    words: Vec<Word>,
}

impl MarkingList {
    fn push(&mut self, op: u16, p: [u16; 5]) {
        self.words.push([op, p[0], p[1], p[2], p[3], p[4]]);
    }
    fn finish(mut self) -> Result<Vec<Packet>, Error> {
        self.push(op::LIST_END_OF_LIST, [0; 5]);
        packetize(&self.words)
    }
}

/// A list builder whose only input is [`LightCommand`] (SPEC §5.2 rule 9): it cannot express `MarkTo`,
/// `LaserOnPoint`, MO open or any parameter of the laser source, and its `WritePort` accepts only 0 or the red-light
/// bit (never the laser bit; `lighting_packets` refuses a profile whose red bit is the laser bit).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LightingList {
    words: Vec<Word>,
    red: u16,
}

/// Everything a lighting list can say. There is no variant for an emitting opcode.
#[derive(Debug, Clone, Copy)]
enum LightCommand {
    Ready,
    Port(u16),
    Speed(u16),
    JumpDelay([u16; 5]),
    Jump([u16; 5]),
    Delay(u16),
    End,
}

impl LightingList {
    fn new(red: u16) -> Result<Self, Error> {
        // The red bit must be exactly one bit (`bit()` builds it); 0 would make every port write a no-op claim.
        if red == 0 || !red.is_power_of_two() {
            return Err(Error::UnsafeLighting);
        }
        Ok(LightingList { words: Vec::new(), red })
    }
    fn push(&mut self, command: LightCommand) -> Result<(), Error> {
        let (opcode, p) = match command {
            LightCommand::Ready => (op::LIST_READY_MARK, [0; 5]),
            LightCommand::Port(v) if v == 0 || v == self.red => (op::LIST_WRITE_PORT, [v, 0, 0, 0, 0]),
            LightCommand::Port(_) => return Err(Error::UnsafeLighting),
            LightCommand::Speed(v) => (op::LIST_JUMP_SPEED, [nonzero_speed(v)?, 0, 0, 0, 0]),
            LightCommand::JumpDelay(p) => (op::LIST_JUMP_DELAY, p),
            LightCommand::Jump(p) => (op::LIST_JUMP_TO, p),
            LightCommand::Delay(v) => (op::LIST_DELAY_TIME, [v, 0, 0, 0, 0]),
            LightCommand::End => (op::LIST_END_OF_LIST, [0; 5]),
        };
        self.words.push([opcode, p[0], p[1], p[2], p[3], p[4]]);
        Ok(())
    }
    fn finish(mut self) -> Result<Vec<Packet>, Error> {
        self.push(LightCommand::End)?;
        packetize(&self.words)
    }
}

/// 256 commands per packet; the last packet carries the explicit EndOfList and is padded with `0x8002`.
fn packetize(words: &[Word]) -> Result<Vec<Packet>, Error> {
    words.chunks(SLOTS).map(|chunk| Packet::from_words(chunk).map_err(Error::Packet)).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct LmcJob {
    /// After the final postlude of the job, with MO closed.
    pub return_axis: Option<crate::AxisMove>,
    /// Sent before EACH list: ResetList, WritePort(laser bit), Fiber_SetMo(1), GetVersion.
    pub prelude: Vec<Single>,
    /// Per layer-pass, in burn order (one repeat).
    pub lists: Vec<LmcList>,
    /// Run by the driver, stop checked between repeats; never expanded.
    pub repeats: u16,
    /// After EACH list, only once !BUSY: Fiber_SetMo(0), WritePort(0), GetMarkTime(3).
    pub postlude: Vec<Single>,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
}

impl LmcJob {
    /// S22: blake3 over, in send order, every single command (12 bytes) and packet (3072 bytes, padding included) of
    /// one repeat (per list: optional axis singles, prelude, packets, postlude), optional final return singles,
    /// then the repeat count as u16 LE. The final return executes once after all repeats.
    ///
    /// Not hashed: the session's own singles (status polls, SetEndOfList, ExecuteList, whose position depends on the
    /// board), `packet_est`, each list's `layer`/`pass` labels, `fingerprint` and `machine`. Those are checked
    /// separately (`emission_summary` against the plan, the driver against the ARM token).
    pub fn bytes_hash(&self) -> Fingerprint {
        let mut hash = Hasher::new();
        for list in &self.lists {
            if let Some(m) = list.axis_move {
                for cmd in crate::axis_singles(&m) {
                    hash.update(&cmd.bytes());
                }
            }
            for cmd in &self.prelude {
                hash.update(&cmd.bytes());
            }
            for packet in &list.packets {
                hash.update(packet.bytes());
            }
            for cmd in &self.postlude {
                hash.update(&cmd.bytes());
            }
        }
        if let Some(m) = self.return_axis {
            for cmd in crate::axis_singles(&m) {
                hash.update(&cmd.bytes());
            }
        }
        hash.update(&self.repeats.to_le_bytes());
        Fingerprint(*hash.finalize().as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LmcList {
    pub axis_move: Option<crate::AxisMove>,
    pub layer: LayerKey,
    pub pass: u16,
    pub packets: Vec<Packet>,
    /// Per packet: the larger of the packet's share of the section estimate and its nominal LMC time decoded from
    /// its own bytes (`decode::packet_times`), so the run-ahead window never under-counts a packet.
    pub packet_est: Vec<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LmcFrameJob {
    pub once: Vec<Packet>,
    pub once_passes: u32,
    pub once_est: Duration,
    pub pass: Vec<Packet>,
    pub pass_est: Duration,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub style: FrameStyle,
    pub source: FrameSource,
}

/// The prelude sent before each marking list (SPEC §3.12).
pub(crate) fn job_prelude(laser: u16) -> Vec<Single> {
    vec![
        Single([op::RESET_LIST, 0, 0, 0, 0, 0]),
        Single([op::WRITE_PORT, laser, 0, 0, 0, 0]),
        Single([op::FIBER_SET_MO, 1, 0, 0, 0, 0]),
        Single([op::GET_VERSION, 0, 0, 0, 0, 0]),
    ]
}

/// The postlude sent after each marking list, once !BUSY (SPEC §3.12).
pub(crate) fn job_postlude() -> Vec<Single> {
    vec![Single([op::FIBER_SET_MO, 0, 0, 0, 0, 0]), Single([op::WRITE_PORT, 0, 0, 0, 0, 0]), Single([op::GET_MARK_TIME, 3, 0, 0, 0, 0])]
}

pub fn encode_job(emit: &EmitPlan, m: &GalvoMachine) -> Result<LmcJob, Error> {
    if emit.device() != DeviceKind::GalvoLmc {
        return Err(Error::NotGalvo);
    }
    if emit.sections().is_empty() || emit.repeats() == 0 {
        return Err(Error::Empty);
    }
    let laser = bit(m.ports.laser_bit)?;
    let mut lists = Vec::with_capacity(emit.sections().len());
    for section in emit.sections() {
        let packets = encode_section(section, m, laser)?;
        let packet_est = packet_estimates(section.estimate, &packets);
        lists.push(LmcList { axis_move: section_axis(section, m)?, layer: section.layer, pass: section.pass, packets, packet_est });
    }

    Ok(LmcJob {
        return_axis: return_axis(emit.sections(), m)?,
        prelude: job_prelude(laser),
        lists,
        repeats: emit.repeats(),
        postlude: job_postlude(),
        fingerprint: *emit.fingerprint(),
        machine: emit.machine().clone(),
    })
}

/// One layer-pass as its marking list's packets (SPEC §5.2): the parameter block, the explicit first jump with the
/// long delay and the settle, then the section's paths. `emission_summary` re-encodes each section with it and
/// refuses a list whose bytes differ (a list spliced in from another job with the same words is not this plan's).
pub(crate) fn encode_section(section: &Section, m: &GalvoMachine, laser: u16) -> Result<Vec<Packet>, Error> {
    let CutQ::Galvo(q) = &section.cut else { return Err(Error::NotGalvo) };
    // The encoder reads only the planner's words (SPEC §5.2 rule 4) and refuses any it cannot send as-is.
    let pulse = q.pulse.word().ok_or(Error::InvalidPulse)?;
    if !m.source.pulse_table_ns.contains(&pulse) {
        return Err(Error::InvalidPulse);
    }
    if q.mark_current > MARK_CURRENT_MAX {
        return Err(EncodeError::OutOfRange { what: "mark current", value: f64::from(q.mark_current) }.into());
    }
    if q.q_period == 0 {
        return Err(EncodeError::OutOfRange { what: "q-switch period", value: 0.0 }.into());
    }
    let mut list = MarkingList::default();
    list.push(op::LIST_READY_MARK, [0; 5]);
    list.push(op::LIST_DELAY_TIME, [q.timings.mo_settle_10us, 0, 0, 0, 0]);
    list.push(op::LIST_WRITE_PORT, [laser, 0, 0, 0, 0]);
    list.push(op::LIST_JUMP_SPEED, [nonzero_speed(q.timings.jump_speed_units)?, 0, 0, 0, 0]);
    list.push(op::LIST_FIBER_YLPM_PULSE_WIDTH, [pulse, 0, 0, 0, 0]);
    list.push(op::LIST_MARK_CURRENT, [q.mark_current, 0, 0, 0, 0]);
    list.push(op::LIST_Q_SWITCH_PERIOD, [q.q_period, 0, 0, 0, 0]);
    list.push(op::LIST_MARK_SPEED, [nonzero_speed(q.speed_units)?, 0, 0, 0, 0]);
    list.push(op::LIST_LASER_ON_DELAY, delay_params(q.timings.laser_on));
    list.push(op::LIST_LASER_OFF_DELAY, delay_params(q.timings.laser_off));
    list.push(op::LIST_POLYGON_DELAY, delay_params(q.timings.polygon));

    let mut last = (CENTER, CENTER);
    let mut wrote_path = false;
    let mut marks = 0usize;
    let mut jump_delay = None;
    for path in &section.paths {
        // A path needs a start and at least one more point; a shorter one is refused, never skipped (N5).
        let [first, rest @ ..] = path.0.as_slice() else { return Err(Error::ShortPath) };
        if rest.is_empty() {
            return Err(Error::ShortPath);
        }
        let start = encode_point(*first, m)?;
        let delay = if wrote_path { q.timings.jump_delay_short } else { q.timings.jump_delay_long };
        if !wrote_path || start != last {
            if jump_delay != Some(delay) {
                list.push(op::LIST_JUMP_DELAY, delay_params(delay));
                jump_delay = Some(delay);
            }
            list.push(op::LIST_JUMP_TO, motion(start, last));
            last = start;
        }
        if !wrote_path {
            list.push(op::LIST_DELAY_TIME, [q.timings.first_jump_settle_10us, 0, 0, 0, 0]);
        }
        for p in rest {
            let to = encode_point(*p, m)?;
            // GP elides a motion to the point the galvo is already at (it would mark nothing).
            if to == last {
                continue;
            }
            list.push(op::LIST_MARK_TO, motion(to, last));
            marks += 1;
            last = to;
        }
        wrote_path = true;
    }
    if !wrote_path {
        return Err(Error::Empty);
    }
    // A section whose every path is below one galvo unit would send MO open with nothing to mark (WP4 N-f).
    if marks == 0 {
        return Err(Error::SubUnitSection { layer: section.layer, pass: section.pass });
    }
    list.finish()
}

pub fn encode_frame(frame: &FramePlan, m: &GalvoMachine) -> Result<LmcFrameJob, Error> {
    if frame.paths().iter().any(|p| p.0.len() < 2) {
        return Err(Error::ShortPath);
    }
    let paths: Vec<Vec<[f64; 2]>> = frame.paths().iter().map(|p| p.0.clone()).collect();
    if paths.is_empty() {
        return Err(Error::Empty);
    }
    // Every point is checked (and a bad one named) before any length or pass count is derived from them.
    for p in paths.iter().flatten() {
        encode_light_point(*p, m)?;
    }
    let points = paths.iter().try_fold(0usize, |n, p| n.checked_add(p.len()).ok_or(Error::TooManyCommands))?;
    // OM Frame Once includes dark travel between paths and the return to the start.
    let mut length = 0.0;
    let mut previous: Option<[f64; 2]> = None;
    for &p in paths.iter().flatten() {
        if let Some(a) = previous {
            length += (p[0] - a[0]).hypot(p[1] - a[1]);
        }
        previous = Some(p);
    }
    if let (Some(a), Some(b)) = (previous, paths.first().and_then(|p| p.first())) {
        length += (b[0] - a[0]).hypot(b[1] - a[1]);
    }
    let pass_est = Duration::try_from_secs_f64(length / m.light_speed.get()).map_err(|_| Error::Packet("invalid frame duration"))?;
    let cap_by_points = u32::try_from(20_000usize / points.max(1)).map_err(|_| Error::TooManyCommands)?.clamp(1, 500);
    let desired = if pass_est.is_zero() { cap_by_points } else { ceil_u32(2.0 / (length / m.light_speed.get()).max(0.001))?.clamp(1, 500) };
    let once_passes = desired.min(cap_by_points).max(1);
    let pass = lighting_packets(&paths, 1, m)?;
    let once = lighting_packets(&paths, once_passes, m)?;
    Ok(LmcFrameJob {
        once,
        once_passes,
        once_est: pass_est.saturating_mul(once_passes),
        pass,
        pass_est,
        fingerprint: *frame.fingerprint(),
        machine: frame.machine().clone(),
        style: frame.style(),
        source: frame.source(),
    })
}

fn lighting_packets(paths: &[Vec<[f64; 2]>], passes: u32, m: &GalvoMachine) -> Result<Vec<Packet>, Error> {
    let fields = m.field_f();
    let light_speed = nonzero_speed(speed_to_units(m.light_speed.get(), fields[0])?)?;
    if m.ports.red_bit == m.ports.laser_bit {
        return Err(Error::UnsafeLighting);
    }
    let red = bit(m.ports.red_bit)?;
    let long = delay_word(m.first_jump.delay_us)?;
    let short = delay_word(m.timings.jump_delay_short_us)?;
    let mut list = LightingList::new(red)?;
    list.push(LightCommand::Ready)?;
    list.push(LightCommand::Port(red))?;
    list.push(LightCommand::Port(0))?;
    list.push(LightCommand::Speed(light_speed))?;
    let start = encode_light_point(paths.first().and_then(|p| p.first()).copied().ok_or(Error::Empty)?, m)?;
    list.push(LightCommand::JumpDelay(delay_params(long)))?;
    list.push(LightCommand::Jump(motion(start, (CENTER, CENTER))))?;
    list.push(LightCommand::Delay(m.first_jump.settle_10us))?;
    let mut last = start;
    let mut port = 0;
    let mut delay = long;
    let mut lit_speed = false;
    // Retain GP's first light-move speed command; D3 adds only the leading dark-jump speed.
    for _ in 0..passes {
        for path in paths {
            for (i, &p) in path.iter().enumerate() {
                let to = encode_light_point(p, m)?;
                if to == last {
                    continue;
                }
                let wanted = if i == 0 { 0 } else { red };
                if port != wanted {
                    list.push(LightCommand::Port(wanted))?;
                    port = wanted;
                }
                if wanted == red && !lit_speed {
                    list.push(LightCommand::Speed(light_speed))?;
                    lit_speed = true;
                }
                if delay != short {
                    list.push(LightCommand::JumpDelay(delay_params(short)))?;
                    delay = short;
                }
                list.push(LightCommand::Jump(motion(to, last)))?;
                last = to;
            }
        }
    }
    list.finish()
}

/// SPEC §5.2 rule 4 / B7: a motion speed of 0 units is refused (the board would never finish the move).
fn nonzero_speed(v: u16) -> Result<u16, Error> {
    if v == 0 { Err(Error::ZeroSpeed) } else { Ok(v) }
}

/// SPEC §5.2 rules 2 and 3 (B10): a point is sent only if it lies inside the machine's rated work area (closed, ±100 mm
/// for Luiz's lens) AND strictly inside the encodable square |v| < F/2. Refused, never clamped. F is used only for the
/// unit conversion.
fn encode_point(p: [f64; 2], m: &GalvoMachine) -> Result<(u16, u16), Error> {
    let [x, y] = p;
    let [fx, fy] = m.field_f();
    let [x_min, y_min] = m.work_area.min;
    let [x_max, y_max] = m.work_area.max;
    let refused = || Error::NotEncodable { x, y };
    let gx = encode_axis(x, x_min, x_max, fx).ok_or_else(refused)?;
    let gy = encode_axis(y, y_min, y_max, fy).ok_or_else(refused)?;
    Ok((gx, gy))
}

/// Written so that a NaN in the value, the bounds or F refuses (every comparison with NaN is false).
fn encode_axis(v: f64, min: f64, max: f64, field: f64) -> Option<u16> {
    let in_work_area = v >= min && v <= max;
    let in_field = v.abs() < field / 2.0;
    if in_work_area && in_field { mm_to_galvo(v, field).ok() } else { None }
}

/// A red-light point: the frame point itself must lie in the work area, and so must the point the galvo is sent to
/// after the red-dot correction (identity until H26).
fn encode_light_point(p: [f64; 2], m: &GalvoMachine) -> Result<(u16, u16), Error> {
    encode_point(p, m)?;
    encode_point([p[0] * m.red_dot.scale[0] + m.red_dot.offset_mm[0], p[1] * m.red_dot.scale[1] + m.red_dot.offset_mm[1]], m)
}

fn motion(to: (u16, u16), last: (u16, u16)) -> [u16; 5] {
    [to.0, to.1, 0, distance_word(last, to), 0]
}
fn delay_params(d: (u16, u16)) -> [u16; 5] {
    [d.0, d.1, 0, 0, 0]
}

fn bit(index: u8) -> Result<u16, Error> {
    1u16.checked_shl(u32::from(index)).ok_or(Error::InvalidPort(index))
}

/// Each packet: max(its share of the section estimate, its nominal time decoded from its bytes).
fn packet_estimates(section: Duration, packets: &[Packet]) -> Vec<Duration> {
    let share = u32::try_from(packets.len()).ok().filter(|n| *n > 0).map_or(Duration::ZERO, |n| section / n);
    packet_times(packets).into_iter().map(|t| t.max(share)).collect()
}

fn ceil_u32(v: f64) -> Result<u32, Error> {
    if !v.is_finite() || v < 0.0 || v > f64::from(u32::MAX) {
        return Err(Error::TooManyCommands);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(v.ceil() as u32)
}

pub(crate) fn section_axis(s: &Section, m: &GalvoMachine) -> Result<Option<crate::AxisMove>, Error> {
    match s.rotary {
        None => Ok(None),
        Some(step) => {
            let r = m.rotary.as_ref().filter(|r| r.enabled).ok_or(Error::SummaryMismatch)?;
            r.validate(m.work_area).map_err(|_| Error::SummaryMismatch)?;
            let mv = crate::AxisMove { target_pulses: step.target_pulses, min_speed: r.min_speed_pps, max_speed: r.max_speed_pps, accel_ms: r.accel_ms };
            if !mv.valid() {
                return Err(Error::SummaryMismatch);
            }
            Ok(Some(mv))
        }
    }
}
pub(crate) fn return_axis(sections: &[Section], m: &GalvoMachine) -> Result<Option<crate::AxisMove>, Error> {
    if m.rotary.as_ref().is_some_and(|r| r.enabled && r.return_to_start)
        && let Some(s) = sections.iter().find(|s| s.rotary.is_some())
    {
        return Ok(section_axis(s, m)?.map(|mv| crate::AxisMove { target_pulses: 0, ..mv }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_list_refuses_any_port_but_zero_and_red() {
        let mut list = LightingList::new(0x0100).unwrap();
        assert!(list.push(LightCommand::Port(0x0100)).is_ok());
        assert!(list.push(LightCommand::Port(0)).is_ok());
        for v in [0x0001, 0x0101, 0x0200, 0xFFFF] {
            assert!(matches!(list.push(LightCommand::Port(v)), Err(Error::UnsafeLighting)), "port {v:#06x}");
        }
        assert!(matches!(list.push(LightCommand::Speed(0)), Err(Error::ZeroSpeed)));
        assert!(matches!(LightingList::new(0), Err(Error::UnsafeLighting)));
        assert!(matches!(LightingList::new(0x0101), Err(Error::UnsafeLighting)));
        let words = list.words.clone();
        assert!(words.iter().all(|w| w[0] == op::LIST_WRITE_PORT && (w[1] == 0 || w[1] == 0x0100)));
    }
}

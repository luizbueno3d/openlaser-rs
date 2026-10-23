//! Pure, session-independent LMC list encoding (SPEC §5.2).
//!
//! Command ordering and packet filling are ports of galvoplotter 0.2.0
//! `galvo/controller.py` (MeerK40t contributors, MIT). OpenLaser's explicit
//! first jump, settle, leading lighting speed and session lifetime are the
//! reviewed divergences in SPEC §5.6.

use std::time::Duration;

use blake3::Hasher;
use openlaser_machine::GalvoMachine;
use openlaser_plan::CutQ;
use openlaser_safety::{EmitPlan, FramePlan};
use openlaser_types::galvo::{CENTER, distance_word, mm_to_galvo, speed_to_units};
use openlaser_types::{Fingerprint, FrameSource, FrameStyle, LayerKey, MachineId};

use crate::Error;
use crate::op;
use crate::packet::{Packet, SLOTS, Single};

type Word = [u16; 6];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkingList {
    words: Vec<Word>,
}

impl MarkingList {
    fn push(&mut self, op: u16, p: [u16; 5]) { self.words.push([op, p[0], p[1], p[2], p[3], p[4]]); }
    fn finish(mut self) -> Result<Vec<Packet>, Error> {
        self.push(op::LIST_END_OF_LIST, [0; 5]);
        packetize(&self.words)
    }
}

/// A list builder whose API contains no emitting command. It cannot express
/// `MarkTo`, `LaserOnPoint`, or MO control.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LightingList {
    words: Vec<Word>,
}

impl LightingList {
    fn push(&mut self, op: u16, p: [u16; 5]) { self.words.push([op, p[0], p[1], p[2], p[3], p[4]]); }
    fn finish(mut self) -> Result<Vec<Packet>, Error> {
        self.push(op::LIST_END_OF_LIST, [0; 5]);
        packetize(&self.words)
    }
}

fn packetize(words: &[Word]) -> Result<Vec<Packet>, Error> {
    words.chunks(SLOTS).map(|chunk| Packet::from_words(chunk).map_err(Error::Packet)).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct LmcJob {
    pub prelude: Vec<Single>,
    pub lists: Vec<LmcList>,
    pub repeats: u16,
    pub postlude: Vec<Single>,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
}

impl LmcJob {
    pub fn bytes_hash(&self) -> Fingerprint {
        let mut hash = Hasher::new();
        for list in &self.lists {
            for cmd in &self.prelude { hash.update(&cmd.bytes()); }
            for packet in &list.packets { hash.update(packet.bytes()); }
            for cmd in &self.postlude { hash.update(&cmd.bytes()); }
        }
        hash.update(&self.repeats.to_le_bytes());
        Fingerprint(*hash.finalize().as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LmcList {
    pub layer: LayerKey,
    pub pass: u16,
    pub packets: Vec<Packet>,
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

pub fn encode_job(emit: &EmitPlan, m: &GalvoMachine) -> Result<LmcJob, Error> {
    if emit.sections().is_empty() { return Err(Error::Empty); }
    let fields = m.field_f();
    let mut lists = Vec::with_capacity(emit.sections().len());
    for section in emit.sections() {
        let CutQ::Galvo(q) = &section.cut else { return Err(Error::NotGalvo) };
        let pulse = q.pulse.word().ok_or(Error::InvalidPulse)?;
        let mut list = MarkingList::default();
        list.push(op::LIST_READY_MARK, [0; 5]);
        list.push(op::LIST_DELAY_TIME, [q.timings.mo_settle_10us, 0, 0, 0, 0]);
        list.push(op::LIST_WRITE_PORT, [bit(m.ports.laser_bit)?, 0, 0, 0, 0]);
        list.push(op::LIST_JUMP_SPEED, [q.timings.jump_speed_units, 0, 0, 0, 0]);
        list.push(op::LIST_FIBER_YLPM_PULSE_WIDTH, [pulse, 0, 0, 0, 0]);
        list.push(op::LIST_MARK_CURRENT, [q.mark_current, 0, 0, 0, 0]);
        list.push(op::LIST_Q_SWITCH_PERIOD, [q.q_period, 0, 0, 0, 0]);
        list.push(op::LIST_MARK_SPEED, [q.speed_units, 0, 0, 0, 0]);
        list.push(op::LIST_LASER_ON_DELAY, delay_params(q.timings.laser_on));
        list.push(op::LIST_LASER_OFF_DELAY, delay_params(q.timings.laser_off));
        list.push(op::LIST_POLYGON_DELAY, delay_params(q.timings.polygon));

        let mut last = (CENTER, CENTER);
        let mut wrote_path = false;
        for path in &section.paths {
            if path.0.len() < 2 { continue; }
            let start = point(path.0.first(), fields)?;
            let delay = if wrote_path { q.timings.jump_delay_short } else { q.timings.jump_delay_long };
            list.push(op::LIST_JUMP_DELAY, delay_params(delay));
            list.push(op::LIST_JUMP_TO, motion(start, last));
            last = start;
            if !wrote_path {
                list.push(op::LIST_DELAY_TIME, [q.timings.first_jump_settle_10us, 0, 0, 0, 0]);
            }
            for p in path.0.iter().skip(1) {
                let to = encode_point(*p, fields)?;
                list.push(op::LIST_MARK_TO, motion(to, last));
                last = to;
            }
            wrote_path = true;
        }
        if !wrote_path { return Err(Error::Empty); }
        let packets = list.finish()?;
        let each = divide_duration(section.estimate, packets.len());
        let packet_est = vec![each; packets.len()];
        lists.push(LmcList { layer: section.layer, pass: section.pass, packets, packet_est });
    }

    Ok(LmcJob {
        prelude: vec![
            Single([op::RESET_LIST, 0, 0, 0, 0, 0]),
            Single([op::WRITE_PORT, bit(m.ports.laser_bit)?, 0, 0, 0, 0]),
            Single([op::FIBER_SET_MO, 1, 0, 0, 0, 0]),
            Single([op::GET_VERSION, 0, 0, 0, 0, 0]),
        ],
        lists,
        repeats: emit.repeats(),
        postlude: vec![
            Single([op::FIBER_SET_MO, 0, 0, 0, 0, 0]),
            Single([op::WRITE_PORT, 0, 0, 0, 0, 0]),
            Single([op::GET_MARK_TIME, 3, 0, 0, 0, 0]),
        ],
        fingerprint: *emit.fingerprint(),
        machine: emit.machine().clone(),
    })
}

pub fn encode_frame(frame: &FramePlan, m: &GalvoMachine) -> Result<LmcFrameJob, Error> {
    let paths: Vec<Vec<[f64; 2]>> = frame.paths().iter().filter(|p| p.0.len() >= 2).map(|p| p.0.clone()).collect();
    if paths.is_empty() { return Err(Error::Empty); }
    let points = paths.iter().try_fold(0usize, |n, p| n.checked_add(p.len()).ok_or(Error::TooManyCommands))?;
    let length = paths.iter().flat_map(|p| p.windows(2)).filter_map(|w| Some((w.first()?, w.get(1)?))).map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1])).sum::<f64>();
    let pass_est = Duration::from_secs_f64(length / m.light_speed.get());
    let cap_by_points = u32::try_from(20_000usize / points.max(1)).map_err(|_| Error::TooManyCommands)?.clamp(1, 500);
    let desired = if pass_est.is_zero() { cap_by_points } else { ceil_u32(2.0 / pass_est.as_secs_f64())?.clamp(1, 500) };
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
    let light_speed = speed_to_units(m.light_speed.get(), fields[0])?;
    let red = bit(m.ports.red_bit)?;
    let long = openlaser_types::galvo::delay_word(m.first_jump.delay_us)?;
    let short = openlaser_types::galvo::delay_word(m.timings.jump_delay_short_us)?;
    let mut list = LightingList::default();
    list.push(op::LIST_READY_MARK, [0; 5]);
    list.push(op::LIST_WRITE_PORT, [red, 0, 0, 0, 0]);
    list.push(op::LIST_WRITE_PORT, [0, 0, 0, 0, 0]);
    list.push(op::LIST_JUMP_SPEED, [light_speed, 0, 0, 0, 0]);
    let mut last = (CENTER, CENTER);
    let mut first = true;
    for _ in 0..passes {
        for path in paths {
            let Some(first_point) = path.first() else { continue };
            let start = encode_light_point(*first_point, fields, m)?;
            list.push(op::LIST_WRITE_PORT, [0, 0, 0, 0, 0]);
            list.push(op::LIST_JUMP_DELAY, delay_params(if first { long } else { short }));
            list.push(op::LIST_JUMP_TO, motion(start, last));
            last = start;
            if first { list.push(op::LIST_DELAY_TIME, [m.first_jump.settle_10us, 0, 0, 0, 0]); }
            list.push(op::LIST_WRITE_PORT, [red, 0, 0, 0, 0]);
            list.push(op::LIST_JUMP_SPEED, [light_speed, 0, 0, 0, 0]);
            for p in path.iter().skip(1) {
                let to = encode_light_point(*p, fields, m)?;
                list.push(op::LIST_JUMP_TO, motion(to, last));
                last = to;
            }
            first = false;
        }
    }
    list.push(op::LIST_WRITE_PORT, [0, 0, 0, 0, 0]);
    list.finish()
}

fn point(p: Option<&[f64; 2]>, fields: [f64; 2]) -> Result<(u16, u16), Error> {
    p.copied().ok_or(Error::Empty).and_then(|p| encode_point(p, fields))
}

fn encode_point(p: [f64; 2], fields: [f64; 2]) -> Result<(u16, u16), Error> {
    let x = mm_to_galvo(p[0], fields[0]).map_err(|_| Error::NotEncodable { x: p[0], y: p[1] })?;
    let y = mm_to_galvo(p[1], fields[1]).map_err(|_| Error::NotEncodable { x: p[0], y: p[1] })?;
    Ok((x, y))
}

fn encode_light_point(p: [f64; 2], fields: [f64; 2], m: &GalvoMachine) -> Result<(u16, u16), Error> {
    encode_point([
        p[0] * m.red_dot.scale[0] + m.red_dot.offset_mm[0],
        p[1] * m.red_dot.scale[1] + m.red_dot.offset_mm[1],
    ], fields)
}

fn motion(to: (u16, u16), last: (u16, u16)) -> [u16; 5] { [to.0, to.1, 0, distance_word(last, to), 0] }
fn delay_params(d: (u16, u16)) -> [u16; 5] { [d.0, d.1, 0, 0, 0] }

fn bit(index: u8) -> Result<u16, Error> {
    1u16.checked_shl(u32::from(index)).ok_or(Error::InvalidPort(index))
}

fn divide_duration(d: Duration, count: usize) -> Duration {
    u32::try_from(count).ok().filter(|n| *n > 0).map_or(Duration::ZERO, |n| d / n)
}

fn ceil_u32(v: f64) -> Result<u32, Error> {
    if !v.is_finite() || v < 0.0 || v > f64::from(u32::MAX) { return Err(Error::TooManyCommands); }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(v.ceil() as u32)
}

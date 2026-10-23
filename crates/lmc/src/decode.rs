//! Decoding, disassembly and summaries derived from the exact bytes sent.
//!
//! Opcode layouts follow galvoplotter 0.2.0 `galvo/controller.py` (MIT).

use std::time::Duration;

use blake3::Hasher;
use openlaser_machine::GalvoMachine;
use openlaser_plan::JobPlan;
use openlaser_types::{Bounds, EmissionSummary, EmittedSegment, GalvoWords, Notice, PassRow};

use crate::Error;
use crate::encode::{LmcFrameJob, LmcJob};
use crate::op;
use crate::packet::Packet;

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

fn word_at(p: &[u16; 6], idx: usize) -> u16 {
    p.get(idx).copied().unwrap_or(0)
}

fn decode_words(w: [u16; 6]) -> Decoded {
    let a = word_at(&w, 1);
    let b = word_at(&w, 2);
    match w[0] {
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
        op::LIST_JUMP_TO => Decoded::JumpTo { x: a, y: b, dist: word_at(&w, 4) },
        op::LIST_MARK_TO => Decoded::MarkTo { x: a, y: b, dist: word_at(&w, 4) },
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

fn decoded_text(d: &Decoded, m: &GalvoMachine) -> String {
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
        Decoded::JumpTo { x, y, dist } => format!("JumpTo x={x} y={y} ({:.3}, {:.3} mm) dist={dist}", point_mm(x, m.field_f()[0]), point_mm(y, m.field_f()[1])),
        Decoded::MarkTo { x, y, dist } => format!("MarkTo x={x} y={y} ({:.3}, {:.3} mm) dist={dist}", point_mm(x, m.field_f()[0]), point_mm(y, m.field_f()[1])),
        Decoded::EndOfList => "EndOfList".into(),
        Decoded::Other(w) => format!("{}({:?})", op::list_name(w[0]), &w[1..]),
    }
}

fn add_packets(out: &mut Listing, packets: &[Packet], list_index: Option<u32>, m: &GalvoMachine) {
    for (pi, packet) in packets.iter().enumerate() {
        for (si, words) in packet.words().into_iter().enumerate() {
            if words[0] == op::LIST_END_OF_LIST {
                break;
            }
            let decoded = decode_words(words);
            out.lines.push(ListingLine {
                list: list_index,
                packet: u32::try_from(pi).ok(),
                slot: u16::try_from(si).ok(),
                words,
                text: decoded_text(&decoded, m),
            });
        }
    }
}

pub fn disassemble(job: &LmcJob, m: &GalvoMachine) -> Listing {
    let mut out = Listing::default();
    for (i, list) in job.lists.iter().enumerate() {
        add_packets(&mut out, &list.packets, u32::try_from(i).ok(), m);
    }
    out
}
pub fn disassemble_frame(job: &LmcFrameJob, m: &GalvoMachine) -> Listing {
    let mut out = Listing::default();
    add_packets(&mut out, &job.once, Some(0), m);
    add_packets(&mut out, &job.pass, Some(1), m);
    out
}

fn decoded_paths(job: &LmcJob, m: &GalvoMachine) -> (Vec<EmittedSegment>, Vec<PassRow>) {
    let mut segments = Vec::new();
    let mut rows = Vec::new();
    for (idx, list) in job.lists.iter().enumerate() {
        let mut current = None;
        let mut power = 0.0;
        let mut speed = 0.0;
        let mut freq = None;
        let mut pulse = None;
        let mut mark_mm = 0.0;
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let mut paths = 0u32;
        let mut was_mark = false;
        for packet in &list.packets {
            for d in decode_packet(packet) {
                match d {
                    Decoded::MarkCurrent(v) => power = f64::from(v) * 100.0 / 4095.0,
                    Decoded::QSwitchPeriod(v) if v > 0 => freq = Some(20000.0 / f64::from(v)),
                    Decoded::PulseWidth(v) => pulse = Some(v),
                    Decoded::MarkSpeed(v) => speed = f64::from(v) * m.field_f()[0] * 1000.0 / 65535.0,
                    Decoded::JumpTo { x, y, .. } => {
                        current = Some([point_mm(x, m.field_f()[0]), point_mm(y, m.field_f()[1])]);
                        was_mark = false;
                    }
                    Decoded::MarkTo { x, y, .. } => {
                        let b = [point_mm(x, m.field_f()[0]), point_mm(y, m.field_f()[1])];
                        if let Some(a) = current {
                            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                            mark_mm += len;
                            min[0] = min[0].min(a[0]).min(b[0]);
                            min[1] = min[1].min(a[1]).min(b[1]);
                            max[0] = max[0].max(a[0]).max(b[0]);
                            max[1] = max[1].max(a[1]).max(b[1]);
                            if !was_mark {
                                paths = paths.saturating_add(1);
                            }
                            segments.push(EmittedSegment { a, b, power_pct: power });
                        }
                        current = Some(b);
                        was_mark = true;
                    }
                    _ => {}
                }
            }
        }
        if paths > 0 {
            let section = idx;
            let (layer_name, sub, of, mode) = job_section_meta(section);
            rows.push(PassRow {
                layer: list.layer,
                layer_name,
                sub,
                pass: list.pass,
                of,
                mode,
                power_pct: power,
                frequency_khz: freq,
                pulse_ns: pulse,
                speed_mm_s: speed,
                galvo: Some(GalvoWords {
                    mark_current: crate_word(&list.packets, op::LIST_MARK_CURRENT),
                    q_period: crate_word(&list.packets, op::LIST_Q_SWITCH_PERIOD),
                    pulse_ns: pulse.unwrap_or(0),
                    speed_units: crate_word(&list.packets, op::LIST_MARK_SPEED),
                    jump_speed_units: crate_word(&list.packets, op::LIST_JUMP_SPEED),
                }),
                paths,
                mark_mm,
                bounds: Bounds { min, max },
                estimate: list.packet_est.iter().copied().sum(),
            });
        }
    }
    (segments, rows)
}

// The row labels are supplied by JobPlan in emission_summary; this default is only used by emitted_segments.
fn job_section_meta(_idx: usize) -> (String, u8, u16, openlaser_types::SectionMode) {
    (String::new(), 0, 0, openlaser_types::SectionMode::Line)
}
fn crate_word(packets: &[Packet], opcode: u16) -> u16 {
    packets.iter().flat_map(Packet::words).find(|w| w[0] == opcode).map_or(0, |w| w[1])
}

pub fn emitted_segments(job: &LmcJob, m: &GalvoMachine) -> Vec<EmittedSegment> {
    decoded_paths(job, m).0
}

/// Summary values are decoded from packet bytes. The plan contributes labels, section identity and notices only.
pub fn emission_summary(job: &LmcJob, m: &GalvoMachine, plan: &JobPlan, notices: &[Notice]) -> Result<EmissionSummary, Error> {
    if job.lists.len() != plan.sections.len() {
        return Err(Error::SummaryMismatch);
    }
    let (_, mut rows) = decoded_paths(job, m);
    if rows.len() != plan.sections.len() {
        return Err(Error::SummaryMismatch);
    }
    for (row, section) in rows.iter_mut().zip(&plan.sections) {
        row.layer = section.layer;
        row.layer_name = section.layer_name.clone();
        row.sub = section.sub;
        row.pass = section.pass;
        row.of = section.of;
        row.mode = section.mode;
    }
    let estimate = plan.estimate;
    let mut hash = Hasher::new();
    for list in &job.lists {
        for cmd in &job.prelude {
            hash.update(&cmd.bytes());
        }
        for packet in &list.packets {
            hash.update(packet.bytes());
        }
        for cmd in &job.postlude {
            hash.update(&cmd.bytes());
        }
    }
    hash.update(&job.repeats.to_le_bytes());
    let packets = job.lists.iter().flat_map(|l| &l.packets).count();
    let packets = u32::try_from(packets).map_err(|_| Error::TooManyCommands)?;
    Ok(EmissionSummary {
        fingerprint: job.fingerprint,
        bytes_hash: openlaser_types::Fingerprint(*hash.finalize().as_bytes()),
        machine: job.machine.clone(),
        rows,
        notices: notices.to_vec(),
        repeats: job.repeats,
        estimate,
        crash_bound: Duration::ZERO,
        packets,
        work_area: Bounds { min: m.work_area.min, max: m.work_area.max },
        negate: m.cal.negate,
        swap_xy: m.cal.swap_xy,
    })
}

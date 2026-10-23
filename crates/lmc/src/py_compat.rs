//! Test-only replay of OpenMopa @6aed8ffa8be5a938d195bf35198cfd168bb07f4e
//! `mopa_luiz/live.py` and galvoplotter 0.2.0 `galvo/controller.py` (MIT).
//! Ports sequencing, parameter caching and duplicate-motion elision, without I/O.
//! Copyright (c) MeerK40t contributors and OpenMopa contributors. MIT licensed.

//! Protocol source: meerk40t/galvoplotter@fc35136 galvo/controller.py (0.2.0), MIT.

/*
MIT License

Copyright (c) 2023 MeerK40t
Copyright (c) 2026 Luiz Bueno

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

use crate::{Error, Packet, Single, init_sequence, op};
use openlaser_types::galvo::{self, CENTER, JPT_M7_PULSES_NS};

#[derive(Debug, Clone, PartialEq)]
pub enum WireEvent {
    Open,
    Single { op: String, hex: String },
    Read { status: u16 },
    List { sha256: String, n_cmds: u16, bytes: Packet },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OmParams {
    pub power: f64,
    pub frequency_khz: f64,
    pub pulse_width_ns: f64,
    pub speed_mm_s: f64,
}

fn push(words: &mut Vec<[u16; 6]>, opcode: u16, a: u16) {
    words.push([opcode, a, 0, 0, 0, 0]);
}

fn paths(polys: &[Vec<[f64; 2]>], field: f64) -> Result<Vec<Vec<(u16, u16)>>, Error> {
    if !field.is_finite() || field <= 0.0 {
        return Err(Error::Empty);
    }
    polys
        .iter()
        .filter(|p| p.len() >= 2)
        .map(|p| {
            p.iter()
                .map(|p| {
                    if p.iter().any(|v| !v.is_finite() || v.abs() > field / 2.0) {
                        return Err(Error::NotEncodable { x: p[0], y: p[1] });
                    }
                    Ok((galvo::mm_to_galvo_om(p[0], field), galvo::mm_to_galvo_om(p[1], field)))
                })
                .collect()
        })
        .collect()
}

pub fn mark_polylines(polys: &[Vec<[f64; 2]>], p: &OmParams, field_mm: f64) -> Result<Vec<WireEvent>, Error> {
    let paths = paths(polys, field_mm)?;
    if paths.is_empty() {
        return Err(Error::Empty);
    }
    let pulse = galvo::nearest_pulse_width(p.pulse_width_ns, &JPT_M7_PULSES_NS).word().ok_or(Error::InvalidPulse)?;
    let mut words = Vec::new();
    for (opcode, value) in [
        (op::LIST_READY_MARK, 0),
        (op::LIST_DELAY_TIME, 800),
        (op::LIST_WRITE_PORT, 1),
        (op::LIST_JUMP_SPEED, galvo::speed_to_units(3000.0, field_mm)?),
        (op::LIST_FIBER_YLPM_PULSE_WIDTH, pulse),
        (op::LIST_MARK_CURRENT, galvo::power_to_mark_current(p.power)?),
        (op::LIST_Q_SWITCH_PERIOD, galvo::frequency_to_period_om(p.frequency_khz)?),
        (op::LIST_MARK_SPEED, galvo::speed_to_units(p.speed_mm_s, field_mm)?),
        (op::LIST_LASER_ON_DELAY, 100),
        (op::LIST_LASER_OFF_DELAY, 100),
        (op::LIST_POLYGON_DELAY, 100),
    ] {
        push(&mut words, opcode, value);
    }
    let mut last = (CENTER, CENTER);
    let mut delay_set = false;
    for path in paths {
        for (i, to) in path.into_iter().enumerate() {
            if to == last {
                continue;
            }
            if i == 0 && !delay_set {
                push(&mut words, op::LIST_JUMP_DELAY, 8);
                delay_set = true;
            }
            words.push([if i == 0 { op::LIST_JUMP_TO } else { op::LIST_MARK_TO }, to.0, to.1, 0, galvo::distance_word(last, to), 0]);
            last = to;
        }
    }
    replay(words, false)
}

/// OM's repeat calculation includes dark travel between paths and back to the first point.
pub fn frame_once_repeats(polys: &[Vec<[f64; 2]>]) -> u32 {
    let mut length = 0.0;
    let mut previous: Option<[f64; 2]> = None;
    let mut first = None;
    let mut points = 0usize;
    for p in polys.iter().flatten() {
        if let Some(a) = previous {
            length += (p[0] - a[0]).hypot(p[1] - a[1]);
        }
        first = first.or(Some(*p));
        previous = Some(*p);
        points = points.saturating_add(1);
    }
    if let (Some(a), Some(b)) = (previous, first) {
        length += (a[0] - b[0]).hypot(a[1] - b[1]);
    }
    let by_time = (2.0 / (length / 3000.0).max(0.001)).ceil().clamp(1.0, 500.0);
    // The clamped, finite count lies in 1..=500.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = by_time as u32;
    count.min(u32::try_from(20_000 / points.max(1)).unwrap_or(500).max(1))
}

pub fn frame_polylines(polys: &[Vec<[f64; 2]>], field_mm: f64) -> Result<Vec<WireEvent>, Error> {
    let paths = paths(polys, field_mm)?;
    if paths.is_empty() {
        return Err(Error::Empty);
    }
    let mut words = Vec::new();
    push(&mut words, op::LIST_READY_MARK, 0);
    push(&mut words, op::LIST_WRITE_PORT, 256);
    let mut last = (CENTER, CENTER);
    let mut red = true;
    let mut delay_set = false;
    let mut speed_set = false;
    for _ in 0..frame_once_repeats(polys) {
        for path in &paths {
            for (i, &to) in path.iter().enumerate() {
                if to == last {
                    continue;
                }
                let light = i != 0;
                if red != light {
                    push(&mut words, op::LIST_WRITE_PORT, if light { 256 } else { 0 });
                    red = light;
                }
                if light && !speed_set {
                    push(&mut words, op::LIST_JUMP_SPEED, galvo::speed_to_units(3000.0, field_mm)?);
                    speed_set = true;
                }
                if !delay_set {
                    push(&mut words, op::LIST_JUMP_DELAY, 8);
                    delay_set = true;
                }
                words.push([op::LIST_JUMP_TO, to.0, to.1, 0, galvo::distance_word(last, to), 0]);
                last = to;
            }
        }
    }
    replay(words, red)
}

fn init() -> Vec<WireEvent> {
    let mut events = vec![WireEvent::Open];
    for cmd in init_sequence() {
        add_single(&mut events, cmd);
    }
    events
}
fn single(events: &mut Vec<WireEvent>, opcode: u16, a: u16) {
    add_single(events, Single([opcode, a, 0, 0, 0, 0]));
}
fn packet(events: &mut Vec<WireEvent>, words: &[[u16; 6]]) -> Result<(), Error> {
    let bytes = Packet::from_words(words).map_err(Error::Packet)?;
    let n_cmds = u16::try_from(bytes.real_commands()).map_err(|_| Error::TooManyCommands)?;
    events.push(WireEvent::List { sha256: bytes.sha256_hex(), n_cmds, bytes });
    Ok(())
}
fn abort(events: &mut Vec<WireEvent>, port: u16) -> Result<(), Error> {
    for opcode in [op::STOP_EXECUTE, op::FIBER_SET_MO, op::RESET_LIST, op::GET_VERSION] {
        single(events, opcode, 0);
    }
    packet(events, &[])?;
    for opcode in [op::SET_END_OF_LIST, op::EXECUTE_LIST, op::FIBER_SET_MO] {
        single(events, opcode, 0);
    }
    single(events, op::WRITE_PORT, port);
    events.push(WireEvent::Close);
    Ok(())
}
fn replay(mut words: Vec<[u16; 6]>, red: bool) -> Result<Vec<WireEvent>, Error> {
    let lighting = words.get(1).is_some_and(|w| w[0] == op::LIST_WRITE_PORT);
    let mut events = init();
    single(&mut events, op::RESET_LIST, 0);
    if !lighting {
        single(&mut events, op::WRITE_PORT, 1);
        single(&mut events, op::FIBER_SET_MO, 1);
    }
    push(&mut words, op::LIST_END_OF_LIST, 0);
    let mut executing = false;
    for (i, chunk) in words.chunks(256).enumerate() {
        single(&mut events, op::GET_VERSION, 0);
        packet(&mut events, chunk)?;
        single(&mut events, op::SET_END_OF_LIST, 0);
        if i == 2 {
            single(&mut events, op::EXECUTE_LIST, 0);
            executing = true;
        }
    }
    if !executing {
        single(&mut events, op::EXECUTE_LIST, 0);
    }
    single(&mut events, op::GET_VERSION, 0);
    single(&mut events, op::FIBER_SET_MO, 0);
    let port = if red { 256 } else { 0 };
    single(&mut events, op::WRITE_PORT, port);
    single(&mut events, op::GET_MARK_TIME, 3);
    single(&mut events, op::GET_VERSION, 0);
    abort(&mut events, port)?;
    Ok(events)
}
pub fn init_abort() -> Vec<WireEvent> {
    let mut events = init();
    // Empty packet construction has a statically valid length.
    if abort(&mut events, 0).is_err() {
        return Vec::new();
    }
    events
}
fn add_single(events: &mut Vec<WireEvent>, cmd: Single) {
    let hex = cmd.bytes().iter().map(|b| format!("{b:02x}")).collect();
    events.push(WireEvent::Single { op: op::single_name(cmd.0[0]).to_owned(), hex });
    if cmd.reads_reply() {
        events.push(WireEvent::Read { status: 0x20 });
    }
}

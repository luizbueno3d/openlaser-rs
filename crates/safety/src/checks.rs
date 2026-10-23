//! The checks behind `validate` and `validate_frame`. Each collects; nothing stops at the first problem.

use std::collections::HashSet;

use openlaser_geom::{Bounds, Polyline, bounds_of};
use openlaser_machine::{GalvoMachine, GantryMachine, JobLimits, Kinematics, LaserSource, MachineProfile};
use openlaser_plan::quantise::{self, pulse_request, pulse_word, speed_field_mm};
use openlaser_plan::{CutQ, GalvoCutQ, GantryCutQ, JobPlan, PlanFacts, Section};
use openlaser_types::galvo::{self, PulseDecision};
use openlaser_types::{DeviceKind, LayerKey, Notice, NoticeCode, Ns, ObjectId, Refusal, RefusalCode, SectionMode};

/// Float noise from transforms must not refuse a shape drawn exactly on an edge (OM v1 `FIELD_EDGE_TOLERANCE_MM`).
/// Points are never moved: a point within the tolerance is sent as it is, and its encodability is checked exactly.
pub const FIELD_EDGE_TOLERANCE_MM: f64 = 1e-6;
/// `JobSettings.repeats` is 1..=999.
pub const MAX_REPEATS: u16 = 999;
/// A quantised speed this far from the requested one (relative) gets a `speed_quantized` notice.
pub const SPEED_NOTICE_REL: f64 = 0.01;

type NoticeKey = (NoticeCode, String, Option<ObjectId>, Option<LayerKey>);
/// A refusal's identity for de-duplication (`at` by bit pattern: `Refusal` holds f64, so it is not `Hash`).
type RefusalKey = (RefusalCode, String, Option<ObjectId>, Option<LayerKey>, Option<[u64; 2]>);

pub(crate) struct Out {
    pub refusals: Vec<Refusal>,
    refused: HashSet<RefusalKey>,
    notices: Vec<NoticeKey>,
    seen: HashSet<NoticeKey>,
}

impl Out {
    pub fn new() -> Self {
        Out { refusals: Vec::new(), refused: HashSet::new(), notices: Vec::new(), seen: HashSet::new() }
    }

    /// Records a refusal once (a set, not a scan: a forged plan with many orphans must not stall the UI thread).
    pub fn refuse(&mut self, r: Refusal) {
        let key = (r.code, r.message.clone(), r.object, r.layer, r.at.map(|[x, y]| [x.to_bits(), y.to_bits()]));
        if self.refused.insert(key) {
            self.refusals.push(r);
        }
    }

    fn notice(&mut self, code: NoticeCode, message: String, object: Option<ObjectId>, layer: Option<LayerKey>) {
        let n = (code, message, object, layer);
        if self.seen.insert(n.clone()) {
            self.notices.push(n);
        }
    }

    /// Notices numbered 1, 2, … in the order they were found (the confirmation and the canvas use the numbers).
    pub fn numbered_notices(&self) -> Vec<Notice> {
        (1u32..)
            .zip(&self.notices)
            .map(|(n, (code, message, object, layer))| Notice { n, code: *code, message: message.clone(), object: *object, layer: *layer })
            .collect()
    }
}

pub(crate) fn device_name(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::GalvoLmc => "galvo (LMC)",
        DeviceKind::Grbl => "GRBL gantry",
    }
}

fn usize_limit(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

fn finite_rect(min: [f64; 2], max: [f64; 2]) -> bool {
    min.iter().chain(&max).all(|v| v.is_finite()) && min[0] <= max[0] && min[1] <= max[1]
}

fn within(b: &Bounds, p: [f64; 2], tol: f64) -> bool {
    p[0] >= b.min[0] - tol && p[0] <= b.max[0] + tol && p[1] >= b.min[1] - tol && p[1] <= b.max[1] + tol
}

/// Where points may go, per machine kind.
pub(crate) enum Area {
    Galvo { work: Bounds, f: [f64; 2], valid: bool },
    Gantry { bed: Bounds, valid: bool },
}

impl Area {
    pub fn of(m: &MachineProfile) -> Area {
        match &m.kind {
            Kinematics::Galvo(g) => {
                let r = g.effective_work_area();
                let f = g.field_f();
                let valid = finite_rect(r.min, r.max) && f.iter().all(|v| v.is_finite() && *v > 0.0);
                Area::Galvo { work: Bounds { min: r.min, max: r.max }, f, valid }
            }
            Kinematics::Gantry(g) => {
                let bed = openlaser_plan::gantry_bed(g);
                let valid = finite_rect(bed.min, bed.max) && g.bed_mm.iter().all(|v| *v > 0.0);
                Area::Gantry { bed, valid }
            }
        }
    }
}

#[derive(Default)]
struct Tally {
    n: usize,
    first: Option<[f64; 2]>,
}

impl Tally {
    fn add(&mut self, p: [f64; 2]) {
        self.n += 1;
        if self.first.is_none() {
            self.first = Some(p);
        }
    }
}

fn mm(v: f64) -> String {
    format!("{v:.3}")
}

fn at_text(p: [f64; 2]) -> String {
    format!("({}, {}) mm", mm(p[0]), mm(p[1]))
}

/// Geometry rules for one layer-pass or for the frame: limits, finiteness, work area / bed, encodability.
/// `what` names it in messages ("C00", "the frame").
pub(crate) fn check_paths(out: &mut Out, area: &Area, limits: &JobLimits, what: &str, layer: Option<LayerKey>, paths: &[Polyline]) {
    let with_layer = |r: Refusal| match layer {
        Some(k) => r.layer(k),
        None => r,
    };
    if paths.len() > usize_limit(limits.max_paths) {
        out.refuse(with_layer(Refusal::new(
            RefusalCode::TooManyPaths,
            format!(
                "{what} has {} paths in one pass; the limit is {}. Split the job across layers, or use a larger Line Interval",
                paths.len(),
                limits.max_paths
            ),
        )));
    }
    let longest = paths.iter().map(Polyline::len).max().unwrap_or(0);
    if longest > usize_limit(limits.max_points_per_path) {
        out.refuse(with_layer(Refusal::new(
            RefusalCode::TooManyPointsPerPath,
            format!(
                "{what} has a path of {longest} points; the limit is {} per path. Simplify the shape or raise the curve tolerance",
                limits.max_points_per_path
            ),
        )));
    }
    let short = paths.iter().filter(|p| p.len() < 2).count();
    if short > 0 {
        out.refuse(with_layer(Refusal::new(
            RefusalCode::MalformedPoint,
            format!("{what} has {short} path(s) with fewer than 2 points, which the planner never makes; plan again"),
        )));
    }
    let total: usize = paths.iter().map(Polyline::len).sum();
    if total > usize_limit(limits.max_points_per_pass) {
        out.refuse(with_layer(Refusal::new(
            RefusalCode::TooManyPointsPerPass,
            format!("{what} has {total} points in one pass; the limit is {}. Split the job across layers, or simplify the artwork", limits.max_points_per_pass),
        )));
    }

    let (mut non_finite, mut outside, mut unencodable) = (Tally::default(), Tally::default(), Tally::default());
    for p in paths.iter().flat_map(|p| p.0.iter()) {
        if !(p[0].is_finite() && p[1].is_finite()) {
            non_finite.add(*p);
            continue;
        }
        match area {
            Area::Galvo { work, f, valid } => {
                if !*valid || !within(work, *p, FIELD_EDGE_TOLERANCE_MM) {
                    outside.add(*p);
                }
                if !*valid || galvo::mm_to_galvo(p[0], f[0]).is_err() || galvo::mm_to_galvo(p[1], f[1]).is_err() {
                    unencodable.add(*p);
                }
            }
            Area::Gantry { bed, valid } => {
                if !*valid || !within(bed, *p, FIELD_EDGE_TOLERANCE_MM) {
                    outside.add(*p);
                }
            }
        }
    }
    if non_finite.n > 0 {
        out.refuse(with_layer(Refusal::new(
            RefusalCode::NonFinite,
            format!("{what} has {} coordinate pair(s) that are not finite numbers; fix or delete the shape", non_finite.n),
        )));
    }
    if let Some(first) = outside.first {
        let r = match area {
            Area::Galvo { work, valid: true, .. } => Refusal::new(
                RefusalCode::OutsideWorkArea,
                format!(
                    "{} point(s) of {what} are outside the work area x {}..{}, y {}..{} mm (first at {}); move or scale the artwork inside it",
                    outside.n,
                    mm(work.min[0]),
                    mm(work.max[0]),
                    mm(work.min[1]),
                    mm(work.max[1]),
                    at_text(first)
                ),
            ),
            Area::Galvo { valid: false, .. } => Refusal::new(
                RefusalCode::OutsideWorkArea,
                format!("the machine's work area or field is not valid, so no point of {what} can be accepted; fix the machine profile"),
            ),
            Area::Gantry { bed, valid: true } => Refusal::new(
                RefusalCode::OutOfBed,
                format!(
                    "{} point(s) of {what} are off the bed x {}..{}, y {}..{} mm (first at {} in machine coordinates); move the artwork onto the bed",
                    outside.n,
                    mm(bed.min[0]),
                    mm(bed.max[0]),
                    mm(bed.min[1]),
                    mm(bed.max[1]),
                    at_text(first)
                ),
            ),
            Area::Gantry { valid: false, .. } => Refusal::new(
                RefusalCode::OutOfBed,
                format!("the machine's bed size is not valid, so no point of {what} can be accepted; fix the machine profile"),
            ),
        };
        out.refuse(with_layer(r.at(first)));
    }
    if let (Some(first), Area::Galvo { f, .. }) = (unencodable.first, area) {
        out.refuse(with_layer(
            Refusal::new(
                RefusalCode::NotEncodable,
                format!(
                    "{} point(s) of {what} cannot be encoded for the galvo (field F = {} × {} mm: each coordinate must stay strictly inside ±F/2; first at {}); move the artwork inward",
                    unencodable.n,
                    mm(f[0]),
                    mm(f[1]),
                    at_text(first)
                ),
            )
            .at(first),
        ));
    }
}

/// Defence in depth for frame ⊇ burn: every burn point lies within the bounds of what the frame traces.
pub(crate) fn check_frame_covers(out: &mut Out, frame: &[Polyline], sections: &[Section]) {
    let burning = sections.iter().any(|s| s.paths.iter().any(|p| p.len() >= 2));
    if !burning {
        return;
    }
    let Some(fb) = bounds_of(frame).filter(|b| finite_rect(b.min, b.max)) else {
        out.refuse(Refusal::new(RefusalCode::FrameDoesNotCoverJob, "the plan has no frame for its burn; plan again"));
        return;
    };
    for s in sections {
        let mut miss = Tally::default();
        for p in s.paths.iter().flat_map(|p| p.0.iter()) {
            if p[0].is_finite() && p[1].is_finite() && !within(&fb, *p, FIELD_EDGE_TOLERANCE_MM) {
                miss.add(*p);
            }
        }
        if let Some(first) = miss.first {
            out.refuse(
                Refusal::new(
                    RefusalCode::FrameDoesNotCoverJob,
                    format!("{} burn point(s) of {} lie outside the job frame (first at {}); plan again", miss.n, s.layer, at_text(first)),
                )
                .layer(s.layer)
                .at(first),
            );
        }
    }
}

/// The plan's facts → refusals and notices.
pub(crate) fn check_facts(out: &mut Out, facts: &PlanFacts) {
    for id in &facts.orphan_objects {
        out.refuse(
            Refusal::new(
                RefusalCode::OrphanObject,
                format!("object {} is on a layer that does not exist, so it would never be framed or burned; move it to a layer", id.0),
            )
            .object(*id),
        );
    }
    for k in &facts.hidden_output_layers {
        out.refuse(
            Refusal::new(
                RefusalCode::HiddenLayerWithOutput,
                format!("layer {k} is hidden but its Output is on, so it would burn without being seen; show the layer or turn its Output off"),
            )
            .layer(*k),
        );
    }
    for k in &facts.empty_fill_layers {
        out.refuse(
            Refusal::new(
                RefusalCode::FillWithoutHatch,
                format!("Fill layer {k} has shapes but no closed outline to fill, so nothing would burn; close the shapes or switch the layer to Line"),
            )
            .layer(*k),
        );
    }
    for id in &facts.non_finite_objects {
        out.refuse(
            Refusal::new(RefusalCode::NonFinite, format!("object {} has a coordinate or transform that is not a finite number; fix or delete it", id.0))
                .object(*id),
        );
    }
    for (k, why) in &facts.unsupported {
        out.refuse(Refusal::new(RefusalCode::UnsupportedMode, why.clone()).layer(*k));
    }
    if let Some(why) = &facts.device_mismatch {
        out.refuse(Refusal::new(RefusalCode::WrongDeviceKind, why.clone()));
    }
    for (id, k) in &facts.open_fill_shapes {
        out.notice(
            NoticeCode::OpenShapeUnhatched,
            format!("object {} on Fill layer {k} is an open shape with no inside to fill; it is framed but not burned", id.0),
            Some(*id),
            Some(*k),
        );
    }
}

/// `5000000000 mm`, or `1.000e15 mm` past what reads well as digits.
fn distance_text(d: f64) -> String {
    if d.is_finite() && d < 1e12 { format!("{d:.0} mm") } else { format!("{d:.3e} mm") }
}

/// Objects the planner left out whole because a point lies beyond `openlaser_plan::MAX_COORD_MM` (never planned in
/// part, never a stand-in): each is refused by name, with its farthest point's distance. `validate` and
/// `validate_frame` alike, whatever the frame source: no frame or burn of the job may go ahead without them.
pub(crate) fn check_far_objects(out: &mut Out, area: &Area, facts: &PlanFacts) {
    let (code, place) = match area {
        Area::Galvo { .. } => (RefusalCode::OutsideWorkArea, "the work area"),
        Area::Gantry { .. } => (RefusalCode::OutOfBed, "the bed"),
    };
    for (id, k, d) in &facts.far_objects {
        out.refuse(
            Refusal::new(
                code,
                format!(
                    "object {} on {k} has a point {} from the design origin, beyond the {:.0} mm this build plans and far outside {place}, so none of it was planned; move it into {place} or delete it",
                    id.0,
                    distance_text(*d),
                    openlaser_plan::MAX_COORD_MM
                ),
            )
            .object(*id)
            .layer(*k),
        );
    }
}

/// One notice for the objects on tool layers (never burned).
pub(crate) fn tool_layer_notice(out: &mut Out, facts: &PlanFacts) {
    let n = facts.tool_layer_objects.len();
    if n > 0 {
        out.notice(
            NoticeCode::ToolLayerNotBurned,
            format!("{n} object(s) on tool layers T1/T2 are never burned; frame them with \"Tool layers only\""),
            None,
            None,
        );
    }
}

fn label(s: &Section) -> String {
    format!("{} sub-layer {}", s.layer, u16::from(s.sub) + 1)
}

fn check_pulse(out: &mut Out, s: &Section, q: &GalvoCutQ, src: &LaserSource) {
    let k = s.layer;
    let what = label(s);
    let requested = pulse_request(&q.pulse);
    if !src.q_pulse_enabled {
        let msg = match requested {
            Some(ns) => format!("{what}: Q-Pulse Width {ns} ns is set, but this source has no Q-pulse control; clear it or use a MOPA source profile"),
            None => format!("{what}: this source has no Q-pulse control, and this build drives MOPA sources only; use a MOPA source profile"),
        };
        out.refuse(Refusal::new(RefusalCode::QPulseUnsupported, msg).layer(k));
        return;
    }
    let table = &src.pulse_table_ns;
    match q.pulse {
        PulseDecision::Exact(n) if table.contains(&n) => {}
        PulseDecision::Snapped { requested, to } if table.contains(&to) && (requested - f64::from(to)).abs() <= 0.5 => {
            out.notice(NoticeCode::PulseSnapped, format!("{what}: Q-Pulse Width {requested} ns is sent as the table's {to} ns"), None, Some(k))
        }
        PulseDecision::Refused { requested, nearest } if requested.is_finite() => out.refuse(
            Refusal::new(
                RefusalCode::PulseNotInTable,
                format!("{what}: Q-Pulse Width {requested} ns is not in the source's pulse table (nearest {nearest} ns); pick a width from the table"),
            )
            .layer(k),
        ),
        PulseDecision::Refused { .. } => out.refuse(
            Refusal::new(RefusalCode::PulseNotInTable, format!("{what}: Q-Pulse Width is not set or not a number; pick a width from the source's pulse table"))
                .layer(k),
        ),
        PulseDecision::Exact(n) | PulseDecision::Snapped { to: n, .. } => out.refuse(
            Refusal::new(RefusalCode::PulseNotInTable, format!("{what}: the planned Q-pulse word {n} ns does not fit the source's pulse table; plan again"))
                .layer(k),
        ),
    }
}

fn words_match(a: &GalvoCutQ, b: &GalvoCutQ) -> bool {
    a.speed_units == b.speed_units
        && a.actual_speed_mm_s.to_bits() == b.actual_speed_mm_s.to_bits()
        && a.mark_current == b.mark_current
        && a.q_period == b.q_period
        && pulse_word(&a.pulse) == pulse_word(&b.pulse)
        && a.timings == b.timings
}

pub(crate) fn check_galvo_cut(out: &mut Out, s: &Section, q: &GalvoCutQ, g: &GalvoMachine) {
    let k = s.layer;
    let what = label(s);
    let src = &g.source;
    let power = q.power.get();
    if !(src.max_power_pct.is_finite() && src.max_power_pct > 0.0) {
        out.refuse(Refusal::new(RefusalCode::PowerAboveMax, format!("{what}: the source's maximum power is not valid; fix the machine profile")).layer(k));
    } else if power > src.max_power_pct {
        out.refuse(
            Refusal::new(RefusalCode::PowerAboveMax, format!("{what}: power {power} % is above the source maximum {} %; lower Max Power", src.max_power_pct))
                .layer(k),
        );
    }
    let khz = q.frequency.khz();
    let [lo, hi] = src.freq_khz;
    if !(lo <= khz && khz <= hi) {
        out.refuse(
            Refusal::new(
                RefusalCode::FrequencyOutOfRange,
                format!("{what}: frequency {khz} kHz is outside the source range {lo}..{hi} kHz; pick a frequency inside it"),
            )
            .layer(k),
        );
    }
    check_pulse(out, s, q, src);
    // Defence in depth (B3): the first jump of every list waits the long delay and the settle, and the MO settles
    // before the first mark. A zero there (a bad profile) would slam the mirrors or fire before the MO is ready.
    let t = &q.timings;
    let zero: Vec<&str> = [
        (t.jump_delay_long.0 == 0, "first-jump long delay"),
        (t.first_jump_settle_10us == 0, "first-jump settle"),
        (t.mo_settle_10us == 0, "MO settle"),
        (t.jump_speed_units == 0, "jump speed"),
    ]
    .into_iter()
    .filter_map(|(z, name)| z.then_some(name))
    .collect();
    if !zero.is_empty() {
        out.refuse(
            Refusal::new(
                RefusalCode::NotEncodable,
                format!("{what}: the machine's {} encode(s) to 0, which the galvo must never be sent; fix the machine profile", zero.join(", ")),
            )
            .layer(k),
        );
    }
    // The delay word's sign is 0x0000 only for a positive delay (`delay_word`); 0x8000 (a zero or negative delay)
    // or any other value on the first jump's long delay is refused.
    if t.jump_delay_long.1 != 0 {
        out.refuse(
            Refusal::new(
                RefusalCode::NotEncodable,
                format!(
                    "{what}: the machine's first-jump long delay is sent with sign word {:#06x} (zero or negative); the first jump of every list must wait a positive delay, fix the machine profile",
                    t.jump_delay_long.1
                ),
            )
            .layer(k),
        );
    }
    // A jump or polygon delay is a wait at a jump's end or a corner: a negative one (sign word 0x8000 with a
    // magnitude) means nothing to the galvo. Zero keeps GP's 0x8000 sign word (D10) and is sent as it is; any other
    // sign word is malformed.
    for (name, (magnitude, sign)) in [("jump delay", t.jump_delay_short), ("polygon delay", t.polygon)] {
        if (sign == 0x8000 && magnitude != 0) || (sign != 0 && sign != 0x8000) {
            out.refuse(
                Refusal::new(
                    RefusalCode::NotEncodable,
                    format!(
                        "{what}: the machine's {name} is sent with sign word {sign:#06x} and magnitude {magnitude} (negative); a jump or polygon delay must be zero or positive, fix the machine profile"
                    ),
                )
                .layer(k),
            );
        }
    }
    let f = speed_field_mm(g);
    if q.speed_units == 0 {
        let min = (100_000.0 * f / 65535.0).ceil() / 100.0;
        out.refuse(
            Refusal::new(
                RefusalCode::SpeedZeroUnits,
                format!(
                    "{what}: speed {} mm/s is below one galvo unit per ms at the {} mm field and would be sent as 0; the minimum is {min:.2} mm/s",
                    q.requested_speed.get(),
                    mm(f)
                ),
            )
            .layer(k),
        );
    }
    let max = g.max_speed.get();
    if q.requested_speed.get() > max {
        out.refuse(
            Refusal::new(
                RefusalCode::SpeedAboveMax,
                format!("{what}: speed {} mm/s is above the machine maximum {max} mm/s; lower the speed", q.requested_speed.get()),
            )
            .layer(k),
        );
    }
    // `speed_to_units` caps the word at 65535 units (F × 1000 mm/s): a speed above that would run capped, so it is
    // refused, never sent as the cap (the jump speed likewise, from the profile).
    let top = galvo::units_to_speed(u16::MAX, f);
    if q.speed_units == u16::MAX && q.requested_speed.get() > top {
        out.refuse(
            Refusal::new(
                RefusalCode::SpeedAboveMax,
                format!(
                    "{what}: speed {} mm/s is above the fastest the galvo speed word can carry at the {} mm field ({top:.2} mm/s); lower the speed",
                    q.requested_speed.get(),
                    mm(f)
                ),
            )
            .layer(k),
        );
    }
    if t.jump_speed_units == u16::MAX && g.jump_speed.get() > top {
        out.refuse(
            Refusal::new(
                RefusalCode::NotEncodable,
                format!(
                    "{what}: the machine's jump speed {} mm/s is above the fastest the galvo speed word can carry at the {} mm field ({top:.2} mm/s); fix the machine profile",
                    g.jump_speed.get(),
                    mm(f)
                ),
            )
            .layer(k),
        );
    }
    // The words the encoder sends must be exactly what these settings quantise to.
    let q_pulse = pulse_request(&q.pulse).and_then(|v| Ns::new(v).ok());
    match quantise::quantise_galvo(q.requested_speed, q.power, q.frequency, q_pulse, g) {
        Ok(again) if words_match(q, &again) => {}
        Ok(_) => out.refuse(Refusal::new(RefusalCode::Stale, format!("{what}: the plan's galvo words do not match its settings; plan again")).layer(k)),
        Err(e) => out.refuse(Refusal::new(RefusalCode::NotEncodable, format!("{what}: {e}")).layer(k)),
    }
    let actual = q.actual_speed_mm_s;
    let requested = q.requested_speed.get();
    if q.speed_units > 0 && ((actual - requested).abs() > SPEED_NOTICE_REL * requested) {
        out.notice(
            NoticeCode::SpeedQuantized,
            format!("{what}: speed {requested} mm/s runs at {actual:.2} mm/s, the nearest whole galvo speed unit"),
            None,
            Some(k),
        );
    }
}

pub(crate) fn check_gantry_cut(out: &mut Out, s: &Section, q: &GantryCutQ, g: &GantryMachine) {
    let k = s.layer;
    let what = label(s);
    let max = g.max_speed.get();
    if q.speed.get() > max {
        out.refuse(
            Refusal::new(
                RefusalCode::SpeedAboveMax,
                format!("{what}: speed {} mm/s is above the machine maximum {max} mm/s (it is refused, never capped); lower the speed", q.speed.get()),
            )
            .layer(k),
        );
    }
    if quantise::s_value(q.power, g.s_max) != Some(q.s_value) {
        out.refuse(Refusal::new(RefusalCode::Stale, format!("{what}: the plan's spindle value S{} does not match its power; plan again", q.s_value)).layer(k));
    }
}

/// Section-level rules: cut kind, mode, parameters, geometry.
pub(crate) fn check_section(out: &mut Out, m: &MachineProfile, area: &Area, s: &Section) {
    let k = s.layer;
    match (&m.kind, &s.cut) {
        (Kinematics::Galvo(g), CutQ::Galvo(q)) => check_galvo_cut(out, s, q, g),
        (Kinematics::Gantry(g), CutQ::Gantry(q)) => check_gantry_cut(out, s, q, g),
        (Kinematics::Galvo(_), CutQ::Gantry(_)) | (Kinematics::Gantry(_), CutQ::Galvo(_)) => out.refuse(
            Refusal::new(
                RefusalCode::WrongDeviceKind,
                format!("{}: its cut settings are for a different kind of machine than {}; plan again", label(s), m.id.0),
            )
            .layer(k),
        ),
    }
    if k.is_tool() {
        out.refuse(Refusal::new(RefusalCode::UnsupportedMode, format!("{}: tool layers are framed, never burned; plan again", label(s))).layer(k));
    }
    if s.mode == SectionMode::Image {
        out.refuse(Refusal::new(RefusalCode::UnsupportedMode, format!("{}: burning images is not in this build", label(s))).layer(k));
    }
    check_paths(out, area, &m.limits, &k.to_string(), Some(k), &s.paths);
}

pub(crate) fn check_repeats(out: &mut Out, plan: &JobPlan) {
    if plan.repeats == 0 {
        out.refuse(Refusal::new(RefusalCode::NothingToMark, "the repeat count is 0, so nothing would burn; set 1 to 999"));
    } else if plan.repeats > MAX_REPEATS {
        out.refuse(Refusal::new(RefusalCode::UnsupportedMode, format!("the repeat count {} is above {MAX_REPEATS}; lower it", plan.repeats)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_deduplicated_by_value() {
        let mut out = Out::new();
        for _ in 0..3 {
            out.refuse(Refusal::new(RefusalCode::OrphanObject, "x").object(ObjectId(1)));
        }
        out.refuse(Refusal::new(RefusalCode::OrphanObject, "x").object(ObjectId(2)));
        out.refuse(Refusal::new(RefusalCode::OutsideWorkArea, "y").at([1.0, 2.0]));
        out.refuse(Refusal::new(RefusalCode::OutsideWorkArea, "y").at([1.0, 2.0]));
        out.refuse(Refusal::new(RefusalCode::OutsideWorkArea, "y").at([1.0, 3.0]));
        out.refuse(Refusal::new(RefusalCode::OutsideWorkArea, "y").layer(LayerKey::Color(1)).at([1.0, 3.0]));
        let got: Vec<(RefusalCode, Option<ObjectId>, Option<[f64; 2]>)> = out.refusals.iter().map(|r| (r.code, r.object, r.at)).collect();
        assert_eq!(got.len(), 5, "{got:?}");
        assert_eq!(got[0], (RefusalCode::OrphanObject, Some(ObjectId(1)), None), "first-seen order is kept");
    }
}

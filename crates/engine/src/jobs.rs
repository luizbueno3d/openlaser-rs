//! The job pipeline: plan → validate → encode → disassemble → summary (SPEC §3.16 `build_job`, §4.3 `job_plan`).
//!
//! Pure: nothing here touches a device. The ARM dialog, `job.plan`, `job.submit` and the live check all run this
//! same function; `request_arm` cold-builds it again and keeps the encoded object it returns (S22).

use std::sync::Arc;
use std::time::Duration;

use openlaser_doc::{Project, Selection, WorkspaceOrigin};
use openlaser_gcode::{GcodeFrame, GcodeProgram, Header};
use openlaser_lmc::{Listing, ListingLine, LmcFrameJob, LmcJob};
use openlaser_machine::{GantryMachine, Kinematics, MachineProfile};
use openlaser_plan::{CutQ, JobPlan, PlanOptions};
use openlaser_safety::Validated;
use openlaser_types::{
    Bounds, DeviceKind, EmissionSummary, EmittedSegment, Fingerprint, FrameSource, FrameStyle, MachineId, Notice, PassRow, Refusal, RefusalCode,
};
use serde::Serialize;

use crate::session::Session;

/// What generated a G-code program (deterministic, so a cold rebuild gives the same bytes).
pub const GENERATOR: &str = concat!("OpenLaser ", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobOptions {
    pub machine: Option<MachineId>,
    /// None = project setting.
    pub selected_only: Option<bool>,
}

pub enum Encoded {
    Lmc { job: Arc<LmcJob>, frame: Arc<LmcFrameJob> },
    Gcode { job: Arc<GcodeProgram>, frame: Arc<GcodeFrame> },
}

impl Encoded {
    /// S22: the hash of the bytes this job will send (recomputed from the object, never cached).
    pub fn bytes_hash(&self) -> Fingerprint {
        match self {
            Encoded::Lmc { job, .. } => job.bytes_hash(),
            Encoded::Gcode { job, .. } => job.bytes_hash(),
        }
    }

    pub fn fingerprint(&self) -> Fingerprint {
        match self {
            Encoded::Lmc { job, .. } => job.fingerprint,
            Encoded::Gcode { job, .. } => job.fingerprint,
        }
    }

    pub fn kind(&self) -> DeviceKind {
        match self {
            Encoded::Lmc { .. } => DeviceKind::GalvoLmc,
            Encoded::Gcode { .. } => DeviceKind::Grbl,
        }
    }
}

pub struct BuiltJob {
    pub plan: Arc<JobPlan>,
    pub validated: Validated,
    pub encoded: Encoded,
    pub summary: EmissionSummary,
    pub listing: Listing,
    pub burned: Vec<EmittedSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRefused {
    pub fingerprint: Option<Fingerprint>,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
    pub error: Option<String>,
}

impl JobRefused {
    /// A job that could not be built for a reason other than a safety rule (no document, a planner error…).
    pub fn error(message: impl Into<String>) -> Self {
        JobRefused { fingerprint: None, refusals: Vec::new(), notices: Vec::new(), error: Some(message.into()) }
    }

    pub(crate) fn refusal(fingerprint: Option<Fingerprint>, r: Refusal) -> Self {
        JobRefused { fingerprint, refusals: vec![r], notices: Vec::new(), error: None }
    }
}

/// Display only: never armed from.
pub struct LiveCheck {
    pub revision: u64,
    pub fingerprint: Option<Fingerprint>,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
    pub estimate: Option<Duration>,
    pub summary: Option<EmissionSummary>,
}

impl LiveCheck {
    fn from_result(revision: u64, r: &Result<BuiltJob, JobRefused>) -> (LiveCheck, Option<String>) {
        match r {
            Ok(b) => (
                LiveCheck {
                    revision,
                    fingerprint: Some(b.summary.fingerprint),
                    refusals: Vec::new(),
                    notices: b.validated.notices().to_vec(),
                    estimate: Some(b.summary.estimate),
                    summary: Some(b.summary.clone()),
                },
                None,
            ),
            Err(j) => (
                LiveCheck { revision, fingerprint: j.fingerprint, refusals: j.refusals.clone(), notices: j.notices.clone(), estimate: None, summary: None },
                j.error.clone(),
            ),
        }
    }
}

impl Session {
    /// plan → validate → encode → disassemble → summary. Pure; never touches a device.
    pub fn build_job(&self, index: Option<usize>, opts: &JobOptions) -> Result<BuiltJob, JobRefused> {
        let idx = index.or(self.active).ok_or_else(|| JobRefused::error("no document open"))?;
        let doc = self.docs.get(idx).ok_or_else(|| JobRefused::error(format!("no document {idx}")))?;
        let machine = opts.machine.as_ref().unwrap_or(&doc.machine);
        let profile = self.profile(machine).ok_or_else(|| JobRefused::error(format!("no machine profile `{}`", machine.0)))?;
        let selected_only = opts.selected_only.unwrap_or(doc.project.job.cut_selected);
        let popts = self.plan_options(&doc.project, &doc.selection, selected_only, None, None);
        build(&doc.project, profile, &popts)
    }

    /// Cached build_job of the active doc for the canvas, Cuts badges and status bar; recomputed when stale.
    pub fn live_check(&mut self) -> Option<&LiveCheck> {
        let idx = self.active?;
        let doc = self.docs.get(idx)?;
        let key = (idx, doc.revision, doc.machine.clone());
        if self.live_key.as_ref() != Some(&key) || self.live.is_none() {
            let result = self.build_job(Some(idx), &JobOptions::default());
            let (lc, err) = LiveCheck::from_result(key.1, &result);
            self.live = Some(lc);
            self.live_error = err;
            self.live_key = Some(key);
        }
        self.live.as_ref()
    }

    /// Why the live check could not build a job beyond the refusals (a planner or encoder error), if it could not.
    pub fn live_error(&self) -> Option<&str> {
        self.live_error.as_deref()
    }

    /// `{fingerprint, refusals, notices, error}` counts for `doc_inspect.live`, from the cache when it is current.
    pub(crate) fn live_json(&self, idx: usize) -> serde_json::Value {
        let current = self.docs.get(idx).is_some_and(|d| self.live_key.as_ref() == Some(&(idx, d.revision, d.machine.clone())));
        let (fp, refusals, notices, error) = match (&self.live, current) {
            (Some(lc), true) => (lc.fingerprint, lc.refusals.len(), lc.notices.len(), self.live_error.clone()),
            _ => {
                let r = self.build_job(Some(idx), &JobOptions::default());
                let (lc, err) = LiveCheck::from_result(0, &r);
                (lc.fingerprint, lc.refusals.len(), lc.notices.len(), err)
            }
        };
        serde_json::json!({ "fingerprint": fp.map(|f| f.hex()), "refusals": refusals, "notices": notices, "error": error })
    }

    /// The planner options for a document: Cut Selected Graphics, repeats, framing choices (S21: they never change the
    /// fingerprint), the curve tolerance pref.
    pub(crate) fn plan_options(
        &self,
        project: &Project,
        selection: &Selection,
        selected_only: bool,
        style: Option<FrameStyle>,
        source: Option<FrameSource>,
    ) -> PlanOptions {
        PlanOptions {
            selected: selected_only.then(|| selection.ids.clone()),
            repeats: project.job.repeats,
            frame_style: style.unwrap_or(project.job.frame_style),
            frame_source: source.unwrap_or(project.job.frame_source),
            flatten_tol_mm: self.prefs.curve_tolerance_mm(),
        }
    }
}

/// The pipeline for one project and machine.
pub(crate) fn build(project: &Project, profile: &MachineProfile, popts: &PlanOptions) -> Result<BuiltJob, JobRefused> {
    let plan = openlaser_plan::plan(project, profile, popts).map_err(|e| JobRefused::error(format!("plan: {e}")))?;
    let validated = openlaser_safety::validate(&plan, profile).map_err(|r| JobRefused {
        fingerprint: Some(r.fingerprint),
        refusals: r.refusals,
        notices: r.notices,
        error: None,
    })?;
    let fp = plan.fingerprint;
    let notices = validated.notices().to_vec();
    let (encoded, summary, listing, burned) = match &profile.kind {
        Kinematics::Galvo(g) => {
            let job = openlaser_lmc::encode_job(validated.emit(), g).map_err(|e| encode_refused(fp, &notices, lmc_refusal(&e)))?;
            let frame = openlaser_lmc::encode_frame(validated.frame(), g).map_err(|e| encode_refused(fp, &notices, lmc_refusal(&e)))?;
            let summary = openlaser_lmc::emission_summary(&job, g, &plan, &notices).map_err(|e| encode_refused(fp, &notices, lmc_refusal(&e)))?;
            let listing = openlaser_lmc::disassemble(&job, g);
            let burned = openlaser_lmc::emitted_segments(&job, g);
            (Encoded::Lmc { job: Arc::new(job), frame: Arc::new(frame) }, summary, listing, burned)
        }
        Kinematics::Gantry(g) => {
            let header = Header { machine_name: profile.name.clone(), generated: GENERATOR.to_owned() };
            let gcode_err = |e: openlaser_gcode::Error| encode_refused(fp, &notices, Refusal::new(RefusalCode::NotEncodable, format!("G-code: {e}")));
            let job = openlaser_gcode::compile(validated.emit(), g, &header).map_err(gcode_err)?;
            let frame = openlaser_gcode::frame_program(validated.frame(), g, &header).map_err(gcode_err)?;
            let summary = gantry_summary(&plan, &job, profile, g, &notices);
            let listing = gcode_listing(&job.text);
            let burns = gcode_decode(&job.text, g.s_max);
            // Bytes are the truth (§1.3 rule 3): every burn the text makes must be one the confirmation shows.
            check_gantry_bytes(&summary, &burns).map_err(|r| encode_refused(fp, &notices, r))?;
            let burned = burns.into_iter().map(|b| b.seg).collect();
            (Encoded::Gcode { job: Arc::new(job), frame: Arc::new(frame) }, summary, listing, burned)
        }
    };
    // Defence in depth (S22): the summary must describe these exact bytes and this exact plan.
    if summary.fingerprint != fp || summary.bytes_hash != encoded.bytes_hash() || encoded.fingerprint() != fp {
        return Err(JobRefused::refusal(
            Some(fp),
            Refusal::new(RefusalCode::BytesMismatch, "the emission summary does not match the encoded job; nothing can be armed from it"),
        ));
    }
    Ok(BuiltJob { plan: Arc::new(plan), validated, encoded, summary, listing, burned })
}

fn encode_refused(fp: Fingerprint, notices: &[Notice], r: Refusal) -> JobRefused {
    JobRefused { fingerprint: Some(fp), refusals: vec![r], notices: notices.to_vec(), error: None }
}

/// An encoder error after validation (it should not happen: safety judged the same plan) as a refusal.
fn lmc_refusal(e: &openlaser_lmc::Error) -> Refusal {
    match e {
        openlaser_lmc::Error::NotEncodable { x, y } => Refusal::new(RefusalCode::NotEncodable, format!("point ({x}, {y}) mm is not encodable")).at([*x, *y]),
        openlaser_lmc::Error::NotGalvo => Refusal::new(RefusalCode::WrongDeviceKind, "a section has no galvo parameters"),
        openlaser_lmc::Error::Empty => Refusal::new(RefusalCode::NothingToMark, "the job is empty"),
        openlaser_lmc::Error::SubUnitSection { layer, .. } => Refusal::new(RefusalCode::NothingToMark, e.to_string()).layer(*layer),
        other => Refusal::new(RefusalCode::NotEncodable, format!("LMC encoder: {other}")),
    }
}

/// The work area of a gantry bed in design mm, per its origin corner.
fn bed_bounds(g: &GantryMachine) -> Bounds {
    let [w, h] = g.bed_mm;
    match g.origin {
        WorkspaceOrigin::BottomLeft => Bounds { min: [0.0, 0.0], max: [w, h] },
        WorkspaceOrigin::TopLeft => Bounds { min: [0.0, -h], max: [w, 0.0] },
        WorkspaceOrigin::Center => Bounds { min: [-w / 2.0, -h / 2.0], max: [w / 2.0, h / 2.0] },
    }
}

/// The gantry summary. The bytes hash is computed from the program text (S22); the per-row power and speed are the
/// quantised words the encoder was asked to write (`GantryCutQ.s_value`, the requested speed), and
/// `check_gantry_bytes` then proves them against the S and F words decoded from the text itself. The crash bound is the whole estimate: GRBL keeps executing its planner and RX buffer
/// after a host crash, and this build does not model their depth.
fn gantry_summary(plan: &JobPlan, prog: &GcodeProgram, profile: &MachineProfile, g: &GantryMachine, notices: &[Notice]) -> EmissionSummary {
    let s_max = f64::from(g.s_max.max(1));
    let rows = plan
        .sections
        .iter()
        .map(|s| {
            let (power_pct, speed) = match &s.cut {
                CutQ::Gantry(q) => (f64::from(q.s_value) * 100.0 / s_max, q.speed.get()),
                CutQ::Galvo(q) => (q.power.get(), q.actual_speed_mm_s),
            };
            PassRow {
                layer: s.layer,
                layer_name: s.layer_name.clone(),
                sub: s.sub,
                pass: s.pass,
                of: s.of,
                mode: s.mode,
                power_pct,
                frequency_khz: None,
                pulse_ns: None,
                speed_mm_s: speed,
                galvo: None,
                paths: u32::try_from(s.paths.len()).unwrap_or(u32::MAX),
                mark_mm: s.paths.iter().map(|p| p.length_mm()).sum(),
                bounds: openlaser_geom::bounds_of(&s.paths).unwrap_or(Bounds { min: [0.0, 0.0], max: [0.0, 0.0] }),
                estimate: s.estimate,
            }
        })
        .collect();
    EmissionSummary {
        fingerprint: plan.fingerprint,
        bytes_hash: prog.bytes_hash(),
        machine: profile.id.clone(),
        rows,
        notices: notices.to_vec(),
        repeats: plan.repeats,
        estimate: plan.estimate,
        crash_bound: plan.estimate,
        packets: 0,
        work_area: bed_bounds(g),
        negate: [false, false],
        swap_xy: false,
    }
}

/// One listing line per G-code line.
fn gcode_listing(text: &str) -> Listing {
    Listing { lines: text.lines().map(|l| ListingLine { list: None, packet: None, slot: None, words: [0; 6], text: l.to_owned() }).collect() }
}

/// Burned segments decoded from G-code text: G1/G2/G3 moves (arcs as chords) while M3/M4 is on and S > 0.
#[cfg(test)]
pub(crate) fn gcode_burned(text: &str, s_max: u32) -> Vec<EmittedSegment> {
    gcode_decode(text, s_max).into_iter().map(|b| b.seg).collect()
}

/// One burning move decoded from G-code text, with the feed in effect for it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GcodeBurn {
    pub seg: EmittedSegment,
    /// mm/min; None when no feed is in effect (never set, not positive, or G93 inverse time).
    pub feed_mm_min: Option<f64>,
}

/// Decodes the burning moves of G-code text: G1/G2/G3 (arcs as chords) while M3/M4 is on and S > 0, each with its S
/// power (in % of `s_max`) and its modal F feed (in mm/min, inch feeds converted).
pub(crate) fn gcode_decode(text: &str, s_max: u32) -> Vec<GcodeBurn> {
    let s_max = f64::from(s_max.max(1));
    let mut out = Vec::new();
    let (mut pos, mut absolute, mut inch, mut motion, mut laser, mut s) = ([0.0f64, 0.0f64], true, false, 0u8, false, 0.0f64);
    let (mut feed, mut inverse_time): (Option<f64>, bool) = (None, false);
    for raw in text.lines() {
        let line = strip_gcode_comments(raw);
        let (mut x, mut y, mut f) = (None, None, None);
        for (letter, v) in gcode_words(&line) {
            // Whole G/M codes only; `as` is exact here (finite, whole, 0..=999).
            let code = (v.fract() == 0.0 && (0.0..1000.0).contains(&v)).then_some(v as u32);
            match (letter, code) {
                ('G', Some(0)) => motion = 0,
                ('G', Some(1)) => motion = 1,
                ('G', Some(2 | 3)) => motion = 2,
                ('G', Some(20)) => inch = true,
                ('G', Some(21)) => inch = false,
                ('G', Some(90)) => absolute = true,
                ('G', Some(91)) => absolute = false,
                ('G', Some(93)) => inverse_time = true,
                ('G', Some(94)) => inverse_time = false,
                ('M', Some(3 | 4)) => laser = true,
                ('M', Some(2 | 5 | 30)) => laser = false,
                ('S', _) => s = v,
                ('F', _) => f = Some(v),
                ('X', _) => x = Some(v),
                ('Y', _) => y = Some(v),
                _ => {}
            }
        }
        // Units apply to the whole block, so the line's F and X/Y are converted after all its words are read.
        let k = if inch { 25.4 } else { 1.0 };
        if let Some(f) = f {
            feed = (f > 0.0).then_some(f * k);
        }
        if x.is_none() && y.is_none() {
            continue;
        }
        let mut to = pos;
        if let Some(x) = x {
            to[0] = if absolute { x * k } else { pos[0] + x * k };
        }
        if let Some(y) = y {
            to[1] = if absolute { y * k } else { pos[1] + y * k };
        }
        if motion != 0 && laser && s > 0.0 {
            let feed_mm_min = if inverse_time { None } else { feed };
            out.push(GcodeBurn { seg: EmittedSegment { a: pos, b: to, power_pct: s * 100.0 / s_max }, feed_mm_min });
        }
        pos = to;
    }
    out
}

/// F is written with at most three decimals (T2 number format): a decoded feed matches a row within this, in mm/min.
const FEED_TOLERANCE_MM_MIN: f64 = 0.01;
/// S is an integer word: the decoded % matches a row's to rounding.
const POWER_TOLERANCE_PCT: f64 = 1e-6;

/// Bytes are the truth (§1.3 rule 3, AGENTS.md §1 rule 4): every burning move of the G-code text must run at the S
/// power and F feed of some row of the confirmation. A burn the confirmation does not show (another power, another
/// speed, no feed, inverse-time feed) refuses the job with `BytesMismatch`; nothing is clamped.
pub(crate) fn check_gantry_bytes(summary: &EmissionSummary, burns: &[GcodeBurn]) -> Result<(), Refusal> {
    for b in burns {
        let p = b.seg.power_pct;
        let Some(feed) = b.feed_mm_min.filter(|f| f.is_finite() && *f > 0.0) else {
            return Err(Refusal::new(
                RefusalCode::BytesMismatch,
                "the G-code burns without a feed rate the confirmation can show (no F, or inverse-time feed); nothing can be armed from it",
            )
            .at(b.seg.a));
        };
        let shown = summary.rows.iter().any(|r| {
            p.is_finite() && (r.power_pct - p).abs() <= POWER_TOLERANCE_PCT * p.abs().max(1.0) && (r.speed_mm_s * 60.0 - feed).abs() <= FEED_TOLERANCE_MM_MIN
        });
        if !shown {
            return Err(Refusal::new(
                RefusalCode::BytesMismatch,
                format!("the G-code burns at {p:.2} % and F{feed:.3} mm/min, which no row of the confirmation shows; nothing can be armed from it"),
            )
            .at(b.seg.a));
        }
    }
    Ok(())
}

fn strip_gcode_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0u32;
    for c in line.chars() {
        match c {
            ';' if depth == 0 => break,
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// `X10.5Y-3` → [('X', 10.5), ('Y', -3.0)]; non-finite or unparsable numbers are skipped.
fn gcode_words(line: &str) -> Vec<(char, f64)> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_ascii_alphabetic() {
            continue;
        }
        let mut num = String::new();
        while let Some(&d) = chars.peek() {
            if d.is_ascii_digit() || d == '.' || d == '-' || d == '+' || d == ' ' {
                if d != ' ' {
                    num.push(d);
                }
                chars.next();
            } else {
                break;
            }
        }
        if let Ok(v) = num.parse::<f64>()
            && v.is_finite()
        {
            out.push((c.to_ascii_uppercase(), v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcode_burn_decoding_follows_laser_state_and_modes() {
        let text = "G21 G90\nG0 X0 Y0\nM4 S500\nG1 X10 Y0 F1000\nG1 X10 Y10 S0\nM5\nG1 X0 Y10\n; comment X99\nM3 S1000 (on)\nG91\nG1 X-10 Y-10\n";
        let segs = gcode_burned(text, 1000);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!(segs[0].a, [0.0, 0.0]);
        assert_eq!(segs[0].b, [10.0, 0.0]);
        assert!((segs[0].power_pct - 50.0).abs() < 1e-9);
        assert_eq!(segs[1].a, [0.0, 10.0]);
        assert_eq!(segs[1].b, [-10.0, 0.0]);
        assert!((segs[1].power_pct - 100.0).abs() < 1e-9);
    }

    #[test]
    fn gcode_inches_and_garbage_never_panic() {
        let segs = gcode_burned("G20\nM3 S10\nG1 X1 Y0\nGXYZ\nX--1\nG1 Xnan\n", 10);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].b[0] - 25.4).abs() < 1e-9);
        assert!(gcode_burned("", 0).is_empty());
    }

    #[test]
    fn gcode_feeds_follow_units_and_modes() {
        let burns = gcode_decode("G21\nM3 S500\nG1 X1 Y0 F300\nG1 X2\nG20 G1 X1 F10\nG93 G1 X0 F2\nG94 F0 G1 X1\n", 1000);
        let feeds: Vec<Option<f64>> = burns.iter().map(|b| b.feed_mm_min).collect();
        assert_eq!(feeds.len(), 5);
        assert_eq!(feeds[0], Some(300.0));
        assert_eq!(feeds[1], Some(300.0), "F is modal");
        assert!((feeds[2].unwrap() - 254.0).abs() < 1e-9, "an inch feed is converted (G20 on the same line)");
        assert_eq!(feeds[3], None, "inverse time has no feed the confirmation can show");
        assert_eq!(feeds[4], None, "F0 is no feed");
    }

    fn row(power_pct: f64, speed_mm_s: f64) -> PassRow {
        PassRow {
            layer: openlaser_types::LayerKey::Color(0),
            layer_name: "C00".into(),
            sub: 0,
            pass: 1,
            of: 1,
            mode: openlaser_types::SectionMode::Line,
            power_pct,
            frequency_khz: None,
            pulse_ns: None,
            speed_mm_s,
            galvo: None,
            paths: 1,
            mark_mm: 1.0,
            bounds: Bounds { min: [0.0, 0.0], max: [1.0, 1.0] },
            estimate: Duration::ZERO,
        }
    }

    fn gantry_summary_with(rows: Vec<PassRow>) -> EmissionSummary {
        EmissionSummary {
            fingerprint: Fingerprint([1; 32]),
            bytes_hash: Fingerprint([2; 32]),
            machine: MachineId("diode".into()),
            rows,
            notices: Vec::new(),
            repeats: 1,
            estimate: Duration::ZERO,
            crash_bound: Duration::ZERO,
            packets: 0,
            work_area: Bounds { min: [0.0, 0.0], max: [400.0, 400.0] },
            negate: [false, false],
            swap_xy: false,
        }
    }

    #[test]
    fn gantry_bytes_must_match_the_confirmation() {
        // Rows: 80 % at 5 mm/s (F300) and 20 % at 33.3333 mm/s (F2000 written as F1999.998).
        let summary = gantry_summary_with(vec![row(80.0, 5.0), row(20.0, 33.333_33)]);
        let check = |text: &str| check_gantry_bytes(&summary, &gcode_decode(text, 1000));
        assert!(check("G21 G90\nM3 S800\nG1 X10 Y0 F300\nM5\nM3 S200\nG1 X0 F1999.998\nM5\n").is_ok());
        assert!(check("G0 X5 F9000\nM5\n").is_ok(), "travel never burns");
        let code = |text: &str| check(text).unwrap_err().code;
        // More power than any row shows.
        assert_eq!(code("M3 S900\nG1 X10 F300\n"), RefusalCode::BytesMismatch);
        // A shown power at a speed no row shows (slower = more energy per mm, faster = beyond what was confirmed).
        assert_eq!(code("M3 S800\nG1 X10 F150\n"), RefusalCode::BytesMismatch);
        assert_eq!(code("M3 S800\nG1 X10 F2000\n"), RefusalCode::BytesMismatch);
        // A power and a speed that are each shown, but not together.
        assert_eq!(code("M3 S800\nG1 X10 F1999.998\n"), RefusalCode::BytesMismatch);
        // No feed, inverse time.
        assert_eq!(code("M3 S800\nG1 X10\n"), RefusalCode::BytesMismatch);
        assert_eq!(code("G93\nM3 S800\nG1 X10 F300\n"), RefusalCode::BytesMismatch);
        // A program that burns while the confirmation shows nothing.
        let empty = gantry_summary_with(Vec::new());
        assert!(check_gantry_bytes(&empty, &gcode_decode("M3 S1\nG1 X1 F300\n", 1000)).is_err());
        assert!(check_gantry_bytes(&empty, &gcode_decode("G0 X1\n", 1000)).is_ok());
    }

    #[test]
    fn job_refused_shapes() {
        let j = JobRefused::error("no document open");
        assert!(j.refusals.is_empty() && j.error.is_some());
        let v = serde_json::to_value(&j).unwrap();
        assert!(v.get("fingerprint").is_some() && v.get("notices").is_some());
    }
}

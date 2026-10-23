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
            let burned = gcode_burned(&job.text, g.s_max);
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
/// quantised words the encoder wrote (`GantryCutQ.s_value`, the requested speed), the decoded burn comes from the
/// text (`gcode_burned`). The crash bound is the whole estimate: GRBL keeps executing its planner and RX buffer
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
pub(crate) fn gcode_burned(text: &str, s_max: u32) -> Vec<EmittedSegment> {
    let s_max = f64::from(s_max.max(1));
    let mut out = Vec::new();
    let (mut pos, mut absolute, mut inch, mut motion, mut laser, mut s) = ([0.0f64, 0.0f64], true, false, 0u8, false, 0.0f64);
    for raw in text.lines() {
        let line = strip_gcode_comments(raw);
        let (mut x, mut y) = (None, None);
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
                ('M', Some(3 | 4)) => laser = true,
                ('M', Some(2 | 5 | 30)) => laser = false,
                ('S', _) => s = v,
                ('X', _) => x = Some(v),
                ('Y', _) => y = Some(v),
                _ => {}
            }
        }
        if x.is_none() && y.is_none() {
            continue;
        }
        let k = if inch { 25.4 } else { 1.0 };
        let mut to = pos;
        if let Some(x) = x {
            to[0] = if absolute { x * k } else { pos[0] + x * k };
        }
        if let Some(y) = y {
            to[1] = if absolute { y * k } else { pos[1] + y * k };
        }
        if motion != 0 && laser && s > 0.0 {
            out.push(EmittedSegment { a: pos, b: to, power_pct: s * 100.0 / s_max });
        }
        pos = to;
    }
    out
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
    fn job_refused_shapes() {
        let j = JobRefused::error("no document open");
        assert!(j.refusals.is_empty() && j.error.is_some());
        let v = serde_json::to_value(&j).unwrap();
        assert!(v.get("fingerprint").is_some() && v.get("notices").is_some());
    }
}

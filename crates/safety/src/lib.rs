//! Safety validation (SPEC §3.8; owned by WP3, red zone; PLAN §3.6 layer 1).
//!
//! [`validate`] is the sole constructor of [`EmitPlan`] and [`FramePlan`]: it judges a [`JobPlan`] against the
//! machine profile, collects EVERY refusal (never warns and continues, never clamps or truncates), and numbers the
//! notices the confirmation shows. [`validate_frame`] applies only the geometry rules, for framing a job that
//! cannot burn yet. The table of rules is [`rules`].
//!
//! An object the planner could not plan as drawn is a fact, never stand-in geometry: a point beyond
//! `openlaser_plan::MAX_COORD_MM` refuses the job (`outside_work_area` / `out_of_bed`) by object, in both.
//!
//! Beyond the §3.8 table, both re-derive the plan's fingerprint and quantised words from its contents and refuse a
//! plan that does not match (`Stale`), so a plan edited after planning cannot reach an encoder, and both check that
//! the job frame's bounds contain every burn point (`FrameDoesNotCoverJob`, defence in depth for frame ⊇ burn).
//!
//! `testing` (feature `unchecked-for-tests`, `src/testing.rs`) is WP0's.
#![forbid(unsafe_code)]

#[cfg(all(feature = "unchecked-for-tests", not(debug_assertions)))]
compile_error!("openlaser-safety: the `unchecked-for-tests` feature is test-only (dev-dependencies) and needs debug assertions");

mod checks;
mod rules;

use serde::Serialize;

use checks::{Area, Out};

pub use checks::{FIELD_EDGE_TOLERANCE_MM, MAX_REPEATS, SPEED_NOTICE_REL};
pub use openlaser_doc::DeviceKind;
pub use openlaser_geom::Polyline;
pub use openlaser_machine::MachineProfile;
pub use openlaser_plan::{JobPlan, Section};
pub use openlaser_types::{Fingerprint, FrameSource, FrameStyle, MachineId, Notice, Refusal, RefusalCode};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The crate's error enum (SPEC §3.0); `validate` reports refusals through [`Refused`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// Validated job. (sealed) Built only by `validate`.
#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    emit: EmitPlan,
    frame: FramePlan,
    notices: Vec<Notice>,
}

impl Validated {
    pub fn emit(&self) -> &EmitPlan {
        &self.emit
    }

    pub fn frame(&self) -> &FramePlan {
        &self.frame
    }

    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    pub fn into_parts(self) -> (EmitPlan, FramePlan, Vec<Notice>) {
        (self.emit, self.frame, self.notices)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refused {
    pub fingerprint: Fingerprint,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
}

/// (sealed) Sections that passed every rule. Serialize only.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitPlan {
    sections: Vec<Section>,
    fingerprint: Fingerprint,
    machine: MachineId,
    device: DeviceKind,
    repeats: u16,
}

impl EmitPlan {
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn device(&self) -> DeviceKind {
        self.device
    }

    pub fn repeats(&self) -> u16 {
        self.repeats
    }
}

/// (sealed) What the red light may trace. Has no power, frequency or pulse fields.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    paths: Vec<Polyline>,
    fingerprint: Fingerprint,
    machine: MachineId,
    style: FrameStyle,
    source: FrameSource,
}

impl FramePlan {
    pub fn paths(&self) -> &[Polyline] {
        &self.paths
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn style(&self) -> FrameStyle {
        self.style
    }

    pub fn source(&self) -> FrameSource {
        self.source
    }
}

/// Identity rules shared by `validate` and `validate_frame`: the plan was made for this machine, for its kind, and
/// is exactly what its contents fingerprint to now.
fn check_identity(out: &mut Out, plan: &JobPlan, machine: &MachineProfile) {
    if plan.machine != machine.id {
        out.refuse(Refusal::new(
            RefusalCode::MachineMismatch,
            format!("this plan was made for machine {}, not {}; plan again for this machine", plan.machine.0, machine.id.0),
        ));
    }
    let kind = openlaser_plan::device_of(machine);
    if plan.device != kind {
        out.refuse(Refusal::new(
            RefusalCode::WrongDeviceKind,
            format!(
                "this plan is for a {}, but {} is a {}; plan again for this machine",
                checks::device_name(plan.device),
                machine.id.0,
                checks::device_name(kind)
            ),
        ));
    }
    if plan.machine == machine.id && openlaser_plan::fingerprint_of(machine, plan.device, &plan.sections, plan.repeats) != plan.fingerprint {
        out.refuse(Refusal::new(
            RefusalCode::Stale,
            format!("the plan's fingerprint {} does not match its contents or the machine's settings; plan again", plan.fingerprint.short()),
        ));
    }
}

fn refused(plan: &JobPlan, out: Out) -> Refused {
    let notices = out.numbered_notices();
    Refused { fingerprint: plan.fingerprint, refusals: out.refusals, notices }
}

/// Pure; runs live in the UI, again before ARM, and inside the engine before every encode. Collects ALL refusals.
pub fn validate(plan: &JobPlan, machine: &MachineProfile) -> Result<Validated, Refused> {
    let mut out = Out::new();
    check_identity(&mut out, plan, machine);
    checks::check_rotary(&mut out, plan, machine);
    checks::check_facts(&mut out, &plan.facts);
    checks::check_repeats(&mut out, plan);
    let area = Area::of(machine);
    let mut burning = false;
    for s in &plan.sections {
        checks::check_section(&mut out, machine, &area, s);
        burning |= s.paths.iter().any(|p| p.len() >= 2);
    }
    if !burning {
        out.refuse(Refusal::new(RefusalCode::NothingToMark, "no layer with Output on has anything to burn; put shapes on an output layer"));
    }
    // The frame `Validated` carries is what the light will trace for this job: it must be traceable and cover it.
    // Its points come from the burn's outlines, so a problem the burn already reports is not repeated for it.
    let mut frame_out = Out::new();
    checks::check_paths(&mut frame_out, &area, &machine.limits, "the frame", None, &plan.frame);
    for r in frame_out.refusals {
        if !out.refusals.iter().any(|x| x.code == r.code) {
            out.refuse(r);
        }
    }
    checks::check_frame_covers(&mut out, &plan.frame, &plan.sections);
    checks::check_far_objects(&mut out, &area, &plan.facts);
    checks::tool_layer_notice(&mut out, &plan.facts);

    if !out.refusals.is_empty() {
        return Err(refused(plan, out));
    }
    let notices = out.numbered_notices();
    let emit =
        EmitPlan { sections: plan.sections.clone(), fingerprint: plan.fingerprint, machine: plan.machine.clone(), device: plan.device, repeats: plan.repeats };
    let frame = FramePlan {
        paths: plan.frame.clone(),
        fingerprint: plan.fingerprint,
        machine: plan.machine.clone(),
        style: openlaser_plan::frame_style_of(&plan.frame),
        source: FrameSource::Job,
    };
    Ok(Validated { emit, frame, notices })
}

/// Framing a job that cannot burn (e.g. tool layers only, or to look at placement while a power refusal is open)
/// needs only the geometry rules. A receipt from it never covers a refused job (the fingerprint differs).
///
/// Checks: the plan's identity (machine, kind, fingerprint); a Hull frame (P1) is refused; an object the planner
/// left out (non-finite, or with a point beyond `MAX_COORD_MM`) is refused whatever the source; the traced paths
/// (`frame`, `tool_frame` or both, per `source`) must be finite, inside the work area or bed, encodable and
/// within the limits; a frame that stands for the job (`Job`, `JobAndToolLayers`) must cover every burn point.
pub fn validate_frame(plan: &JobPlan, machine: &MachineProfile, source: FrameSource) -> Result<FramePlan, Refused> {
    let mut out = Out::new();
    check_identity(&mut out, plan, machine);
    checks::check_rotary(&mut out, plan, machine);
    for (k, why) in &plan.facts.unsupported {
        if why == openlaser_plan::HULL_FRAME_UNSUPPORTED {
            out.refuse(Refusal::new(RefusalCode::UnsupportedMode, why.clone()).layer(*k));
        }
    }
    for id in &plan.facts.non_finite_objects {
        out.refuse(
            Refusal::new(RefusalCode::NonFinite, format!("object {} has a coordinate or transform that is not a finite number; fix or delete it", id.0))
                .object(*id),
        );
    }
    let paths: Vec<Polyline> = match source {
        FrameSource::Job => plan.frame.clone(),
        FrameSource::ToolLayersOnly => plan.tool_frame.clone(),
        FrameSource::JobAndToolLayers => plan.frame.iter().chain(&plan.tool_frame).cloned().collect(),
    };
    if !paths.iter().any(|p| p.len() >= 2) {
        let what = match source {
            FrameSource::Job => "the job has nothing to frame; put shapes on a layer with Output on",
            FrameSource::ToolLayersOnly => "the tool layers T1/T2 have nothing to frame; put shapes on a shown tool layer",
            FrameSource::JobAndToolLayers => "neither the job nor the tool layers have anything to frame",
        };
        out.refuse(Refusal::new(RefusalCode::NothingToMark, what));
    }
    let area = Area::of(machine);
    checks::check_far_objects(&mut out, &area, &plan.facts);
    checks::check_paths(&mut out, &area, &machine.limits, "the frame", None, &paths);
    if source != FrameSource::ToolLayersOnly {
        checks::check_frame_covers(&mut out, &paths, &plan.sections);
    }
    if !out.refusals.is_empty() {
        return Err(refused(plan, out));
    }
    let style = match source {
        FrameSource::ToolLayersOnly => openlaser_plan::frame_style_of(&plan.tool_frame),
        FrameSource::Job | FrameSource::JobAndToolLayers => openlaser_plan::frame_style_of(&plan.frame),
    };
    Ok(FramePlan { paths, fingerprint: plan.fingerprint, machine: plan.machine.clone(), style, source })
}

/// The rule table, for docs and the `rules_table_complete` test: every code `validate` and `validate_frame` can
/// return, with the rule in words.
pub fn rules() -> &'static [(RefusalCode, &'static str)] {
    rules::RULES
}

/// Dev-dependency only; `compile_error!` without debug_assertions. Implemented in full by WP0 (SPEC §3.0);
/// referenced only from tests (§2.3 rule 14).
#[cfg(feature = "unchecked-for-tests")]
pub mod testing;

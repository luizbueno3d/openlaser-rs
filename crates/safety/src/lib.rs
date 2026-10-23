//! Safety validation (SPEC §3.8; owned by WP3, red zone).
//!
//! WP0 stubs fail closed: `validate` and `validate_frame` refuse every plan until WP3 implements the rules.
//! `testing` (feature `unchecked-for-tests`, `src/testing.rs`) is WP0's and real.
#![forbid(unsafe_code)]

#[cfg(all(feature = "unchecked-for-tests", not(debug_assertions)))]
compile_error!("openlaser-safety: the `unchecked-for-tests` feature is test-only (dev-dependencies) and needs debug assertions");

use serde::Serialize;

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

fn not_implemented(plan: &JobPlan, what: &str) -> Refused {
    Refused {
        fingerprint: plan.fingerprint,
        refusals: vec![Refusal::new(RefusalCode::NotImplemented, format!("not implemented: {what}"))],
        notices: Vec::new(),
    }
}

/// Pure; runs live in the UI, again before ARM, and inside the engine before every encode. Collects ALL refusals.
pub fn validate(plan: &JobPlan, machine: &MachineProfile) -> Result<Validated, Refused> {
    let _ = machine;
    Err(not_implemented(plan, "openlaser_safety::validate"))
}

/// Framing a job that cannot burn (e.g. tool layers only, or to look at placement while a power refusal is open)
/// needs only the geometry rules. A receipt from it never covers a refused job (the fingerprint differs).
pub fn validate_frame(plan: &JobPlan, machine: &MachineProfile, source: FrameSource) -> Result<FramePlan, Refused> {
    let _ = (machine, source);
    Err(not_implemented(plan, "openlaser_safety::validate_frame"))
}

/// The rule table, for docs and the `rules_table_complete` test. WP0 stub: empty.
pub fn rules() -> &'static [(RefusalCode, &'static str)] {
    &[]
}

/// Dev-dependency only; `compile_error!` without debug_assertions. Implemented in full by WP0 (SPEC §3.0);
/// referenced only from tests (§2.3 rule 14).
#[cfg(feature = "unchecked-for-tests")]
pub mod testing;

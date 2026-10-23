//! AUT C3.4/C3.5 state machines (red zone review): agent frame requests and job submissions (SPEC §4.4).
//!
//! Nothing here touches a driver. A human acts on a request only through `approve_request` (which calls
//! `start_frame` or `request_arm` exactly as the human's own buttons do) or `decline_request`; both refuse every
//! origin but `Human`. An agent can create, read and withdraw requests, never advance them.
//!
//! ```text
//! frame_request ─► awaiting_human ─(human Approve)─► framing ─► done (Once) | until stop (Loop)
//!                    ├─(human Decline)─► declined     ├─(edit/undo)─► stale      └─(stop from anyone)─► stopped
//!                    └─(2 min)─► expired
//! job_submit ─(refusals)─► tool error, nothing created
//! awaiting_human ─(human Frame)─► framing ─(≥ 1 executed pass, S20)─► awaiting_arm ─(human ARM)─► running ─► finished | stopped | fault
//!    ├─(human Reject)─► rejected   ├─(job_withdraw)─► withdrawn   ├─(newer submit)─► superseded
//!    ├─(any edit/undo)─► stale     └─(15 min)─► expired           awaiting_arm ─(receipt > 10 min)─► awaiting_human
//! ```

use std::sync::Arc;
use std::time::Duration;

use openlaser_driver::{Outcome, StopReport};
use openlaser_types::{EmissionSummary, FRAME_RECEIPT_MAX_AGE, Fingerprint, FrameMode, FrameOrigin, FrameReceipt, MachineId, MonoTime, RefusalCode};
use serde::Serialize;

use crate::command::{EngineError, Origin};
use crate::jobs::{JobOptions, JobRefused};
use crate::session::Session;

/// A submission waits this long for a human (AUT C3.5).
pub const SUBMISSION_TTL: Duration = Duration::from_secs(15 * 60);
/// A frame request waits this long for a human (AUT C3.4).
pub const FRAME_REQUEST_TTL: Duration = Duration::from_secs(2 * 60);
/// The agent's note on a submission: plain text, at most this many characters.
pub const MAX_NOTE_CHARS: usize = 280;
/// Finished requests kept for `job.status` (oldest dropped first).
const MAX_KEPT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionState {
    AwaitingHuman,
    Framing,
    AwaitingArm,
    Running,
    Finished,
    Stopped,
    Fault,
    Rejected,
    Withdrawn,
    Superseded,
    Stale,
    Expired,
}

impl SubmissionState {
    /// Still waiting on a human (supersede, stale and expiry apply).
    pub fn pending(self) -> bool {
        matches!(self, SubmissionState::AwaitingHuman | SubmissionState::Framing | SubmissionState::AwaitingArm)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SubmissionState::AwaitingHuman => "awaiting_human",
            SubmissionState::Framing => "framing",
            SubmissionState::AwaitingArm => "awaiting_arm",
            SubmissionState::Running => "running",
            SubmissionState::Finished => "finished",
            SubmissionState::Stopped => "stopped",
            SubmissionState::Fault => "fault",
            SubmissionState::Rejected => "rejected",
            SubmissionState::Withdrawn => "withdrawn",
            SubmissionState::Superseded => "superseded",
            SubmissionState::Stale => "stale",
            SubmissionState::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameRequestState {
    AwaitingHuman,
    Framing,
    Done,
    Declined,
    Stale,
    Expired,
    Stopped,
}

impl FrameRequestState {
    pub fn pending(self) -> bool {
        matches!(self, FrameRequestState::AwaitingHuman | FrameRequestState::Framing)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FrameRequestState::AwaitingHuman => "awaiting_human",
            FrameRequestState::Framing => "framing",
            FrameRequestState::Done => "done",
            FrameRequestState::Declined => "declined",
            FrameRequestState::Stale => "stale",
            FrameRequestState::Expired => "expired",
            FrameRequestState::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionView {
    pub submission: RequestId,
    pub machine: MachineId,
    pub fingerprint: Fingerprint,
    pub state: SubmissionState,
    pub note: String,
    pub client: String,
    pub summary: EmissionSummary,
    pub expires_in_s: u64,
    pub outcome: Option<Outcome>,
    pub stop_report: Option<StopReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRequestView {
    pub request: RequestId,
    pub machine: MachineId,
    pub fingerprint: Fingerprint,
    pub mode: FrameMode,
    pub state: FrameRequestState,
    pub client: String,
    pub expires_in_s: u64,
}

struct SubmissionRec {
    view: SubmissionView,
    doc: usize,
    expires: MonoTime,
}

struct FrameRec {
    view: FrameRequestView,
    doc: usize,
    expires: MonoTime,
}

/// A state change to report as `Event::Request`.
pub(crate) type Moved = Vec<(RequestId, String)>;

/// ≤ 1 pending submission and ≤ 1 pending frame request per machine (a new one supersedes); submissions expire
/// after 15 min, frame requests after 2 min; any revision bump → Stale. Nothing here touches a driver except
/// approve_request (Human), which calls start_frame exactly as the human's own Frame button does.
#[derive(Default)]
pub struct Requests {
    submissions: Vec<SubmissionRec>,
    frame_requests: Vec<FrameRec>,
    next: u64,
}

impl Requests {
    pub fn submissions(&self) -> Vec<SubmissionView> {
        self.submissions.iter().map(|r| r.view.clone()).collect()
    }

    pub fn frame_requests(&self) -> Vec<FrameRequestView> {
        self.frame_requests.iter().map(|r| r.view.clone()).collect()
    }

    pub fn submission(&self, id: RequestId) -> Option<SubmissionView> {
        self.submissions.iter().find(|r| r.view.submission == id).map(|r| r.view.clone())
    }

    pub fn frame_request(&self, id: RequestId) -> Option<FrameRequestView> {
        self.frame_requests.iter().find(|r| r.view.request == id).map(|r| r.view.clone())
    }

    /// The pending submission for `machine`, if any.
    pub fn pending_submission(&self, machine: &MachineId) -> Option<RequestId> {
        self.submissions.iter().find(|r| &r.view.machine == machine && r.view.state.pending()).map(|r| r.view.submission)
    }

    /// The pending frame request for `machine`, if any.
    pub fn pending_frame_request(&self, machine: &MachineId) -> Option<RequestId> {
        self.frame_requests.iter().find(|r| &r.view.machine == machine && r.view.state.pending()).map(|r| r.view.request)
    }

    fn next_id(&mut self) -> RequestId {
        self.next = self.next.saturating_add(1);
        RequestId(self.next)
    }

    fn set_sub(r: &mut SubmissionRec, to: SubmissionState, moved: &mut Moved) {
        if r.view.state != to {
            r.view.state = to;
            moved.push((r.view.submission, to.as_str().to_owned()));
        }
    }

    fn set_frame(r: &mut FrameRec, to: FrameRequestState, moved: &mut Moved) {
        if r.view.state != to {
            r.view.state = to;
            moved.push((r.view.request, to.as_str().to_owned()));
        }
    }

    fn trim(&mut self) {
        while self.submissions.len() > MAX_KEPT {
            match self.submissions.iter().position(|r| !r.view.state.pending() && r.view.state != SubmissionState::Running) {
                Some(i) => {
                    self.submissions.remove(i);
                }
                None => break,
            }
        }
        while self.frame_requests.len() > MAX_KEPT {
            match self.frame_requests.iter().position(|r| !r.view.state.pending()) {
                Some(i) => {
                    self.frame_requests.remove(i);
                }
                None => break,
            }
        }
    }

    pub(crate) fn add_submission(&mut self, mut view: SubmissionView, doc: usize, now: MonoTime) -> (SubmissionView, Moved) {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| r.view.machine == view.machine && r.view.state.pending()) {
            Self::set_sub(r, SubmissionState::Superseded, &mut moved);
        }
        view.submission = self.next_id();
        view.state = SubmissionState::AwaitingHuman;
        view.expires_in_s = SUBMISSION_TTL.as_secs();
        moved.push((view.submission, view.state.as_str().to_owned()));
        self.submissions.push(SubmissionRec { view: view.clone(), doc, expires: now.plus(SUBMISSION_TTL) });
        self.trim();
        (view, moved)
    }

    pub(crate) fn add_frame_request(&mut self, mut view: FrameRequestView, doc: usize, now: MonoTime) -> (FrameRequestView, Moved) {
        let mut moved = Vec::new();
        for r in self.frame_requests.iter_mut().filter(|r| r.view.machine == view.machine && r.view.state == FrameRequestState::AwaitingHuman) {
            Self::set_frame(r, FrameRequestState::Declined, &mut moved);
        }
        view.request = self.next_id();
        view.state = FrameRequestState::AwaitingHuman;
        view.expires_in_s = FRAME_REQUEST_TTL.as_secs();
        moved.push((view.request, view.state.as_str().to_owned()));
        self.frame_requests.push(FrameRec { view: view.clone(), doc, expires: now.plus(FRAME_REQUEST_TTL) });
        self.trim();
        (view, moved)
    }

    /// Any revision bump of document `doc`: every pending request made from it is stale.
    pub(crate) fn mark_stale(&mut self, doc: usize) -> Moved {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| r.doc == doc && r.view.state.pending()) {
            Self::set_sub(r, SubmissionState::Stale, &mut moved);
        }
        for r in self.frame_requests.iter_mut().filter(|r| r.doc == doc && r.view.state.pending()) {
            Self::set_frame(r, FrameRequestState::Stale, &mut moved);
        }
        moved
    }

    /// Document `doc` was closed: its pending requests are stale, later documents shift down.
    pub(crate) fn close_doc(&mut self, doc: usize) -> Moved {
        let moved = self.mark_stale(doc);
        for r in &mut self.submissions {
            if r.doc > doc {
                r.doc -= 1;
            } else if r.doc == doc {
                r.doc = usize::MAX;
            }
        }
        for r in &mut self.frame_requests {
            if r.doc > doc {
                r.doc -= 1;
            } else if r.doc == doc {
                r.doc = usize::MAX;
            }
        }
        moved
    }

    /// A human started framing `fingerprint` on `machine` from document `doc` (own button or an approval).
    pub(crate) fn human_framed(&mut self, doc: usize, machine: &MachineId, fingerprint: Fingerprint) -> Moved {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| r.doc == doc && &r.view.machine == machine && r.view.fingerprint == fingerprint) {
            if r.view.state == SubmissionState::AwaitingHuman {
                Self::set_sub(r, SubmissionState::Framing, &mut moved);
            }
        }
        moved
    }

    /// The driver reported an executed frame pass (S20).
    pub(crate) fn frame_completed(&mut self, machine: &MachineId, receipt: &FrameReceipt) -> Moved {
        let mut moved = Vec::new();
        let counts = receipt.full_passes >= 1 && receipt.origin == FrameOrigin::Human && receipt.covers_job();
        for r in self.submissions.iter_mut().filter(|r| &r.view.machine == machine && r.view.fingerprint == receipt.fingerprint) {
            if counts && r.view.state == SubmissionState::Framing {
                Self::set_sub(r, SubmissionState::AwaitingArm, &mut moved);
            }
        }
        for r in self.frame_requests.iter_mut().filter(|r| &r.view.machine == machine && r.view.fingerprint == receipt.fingerprint) {
            if r.view.state == FrameRequestState::Framing && r.view.mode == FrameMode::Once {
                Self::set_frame(r, FrameRequestState::Done, &mut moved);
            }
        }
        moved
    }

    /// A human armed `fingerprint` on `machine`.
    pub(crate) fn armed(&mut self, machine: &MachineId, fingerprint: Fingerprint) -> Moved {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| &r.view.machine == machine && r.view.fingerprint == fingerprint) {
            if r.view.state == SubmissionState::AwaitingArm {
                Self::set_sub(r, SubmissionState::Running, &mut moved);
            }
        }
        moved
    }

    /// A stop reached `machine`: the running submission is stopped, framing ends.
    pub(crate) fn stopped(&mut self, machine: &MachineId, report: Option<StopReport>) -> Moved {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| &r.view.machine == machine) {
            match r.view.state {
                SubmissionState::Running => {
                    r.view.stop_report = report.clone();
                    Self::set_sub(r, SubmissionState::Stopped, &mut moved);
                }
                SubmissionState::Framing => Self::set_sub(r, SubmissionState::AwaitingHuman, &mut moved),
                _ => {}
            }
        }
        for r in self.frame_requests.iter_mut().filter(|r| &r.view.machine == machine && r.view.state == FrameRequestState::Framing) {
            Self::set_frame(r, FrameRequestState::Stopped, &mut moved);
        }
        moved
    }

    /// A stop of everything (control fast path, signal, `stop_all`).
    pub(crate) fn stopped_all(&mut self) -> Moved {
        let machines: Vec<MachineId> = self
            .submissions
            .iter()
            .filter(|r| matches!(r.view.state, SubmissionState::Running | SubmissionState::Framing))
            .map(|r| r.view.machine.clone())
            .chain(self.frame_requests.iter().filter(|r| r.view.state == FrameRequestState::Framing).map(|r| r.view.machine.clone()))
            .collect();
        let mut moved = Vec::new();
        for m in machines {
            moved.extend(self.stopped(&m, None));
        }
        moved
    }

    pub(crate) fn finished(&mut self, machine: &MachineId, outcome: &Outcome) -> Moved {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| &r.view.machine == machine && r.view.state == SubmissionState::Running) {
            r.view.outcome = Some(outcome.clone());
            let to = match outcome {
                Outcome::Finished { .. } => SubmissionState::Finished,
                Outcome::Stopped(report) => {
                    r.view.stop_report = Some(report.clone());
                    SubmissionState::Stopped
                }
            };
            Self::set_sub(r, to, &mut moved);
        }
        moved
    }

    pub(crate) fn faulted(&mut self, machine: &MachineId) -> Moved {
        let mut moved = Vec::new();
        for r in self.submissions.iter_mut().filter(|r| &r.view.machine == machine) {
            match r.view.state {
                SubmissionState::Running => Self::set_sub(r, SubmissionState::Fault, &mut moved),
                SubmissionState::Framing => Self::set_sub(r, SubmissionState::AwaitingHuman, &mut moved),
                _ => {}
            }
        }
        for r in self.frame_requests.iter_mut().filter(|r| &r.view.machine == machine && r.view.state == FrameRequestState::Framing) {
            Self::set_frame(r, FrameRequestState::Stopped, &mut moved);
        }
        moved
    }

    /// Time passes: expiry, `expiresInS`, and awaiting_arm → awaiting_human once the receipt is older than 10 min.
    pub(crate) fn tick(&mut self, now: MonoTime, receipts: &[(MachineId, Option<MonoTime>)]) -> Moved {
        let mut moved = Vec::new();
        for r in &mut self.submissions {
            if r.view.state.pending() && now >= r.expires {
                Self::set_sub(r, SubmissionState::Expired, &mut moved);
            }
            if r.view.state == SubmissionState::AwaitingArm {
                let receipt_at = receipts.iter().find(|(m, _)| m == &r.view.machine).and_then(|(_, at)| *at);
                if receipt_at.is_none_or(|at| now.saturating_since(at) > FRAME_RECEIPT_MAX_AGE) {
                    Self::set_sub(r, SubmissionState::AwaitingHuman, &mut moved);
                }
            }
            r.view.expires_in_s = if r.view.state.pending() { r.expires.saturating_since(now).as_secs() } else { 0 };
        }
        for r in &mut self.frame_requests {
            if r.view.state == FrameRequestState::AwaitingHuman && now >= r.expires {
                Self::set_frame(r, FrameRequestState::Expired, &mut moved);
            }
            r.view.expires_in_s = if r.view.state == FrameRequestState::AwaitingHuman { r.expires.saturating_since(now).as_secs() } else { 0 };
        }
        moved
    }

    fn sub_mut(&mut self, id: RequestId) -> Option<&mut SubmissionRec> {
        self.submissions.iter_mut().find(|r| r.view.submission == id)
    }

    fn frame_mut(&mut self, id: RequestId) -> Option<&mut FrameRec> {
        self.frame_requests.iter_mut().find(|r| r.view.request == id)
    }
}

fn not_awaiting(id: RequestId, state: &str) -> EngineError {
    EngineError::refused(RefusalCode::Stale, format!("request {} is {state}; it no longer waits for this", id.0))
}

fn unknown(id: RequestId) -> EngineError {
    EngineError::bad("job.status", "/submission", format!("no request {}", id.0))
}

/// Plain text for the agent's note: control characters become spaces, at most 280 characters.
pub(crate) fn clean_note(note: &str) -> Result<String, EngineError> {
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(EngineError::bad("job.submit", "/note", format!("the note is longer than {MAX_NOTE_CHARS} characters")));
    }
    Ok(note.chars().map(|c| if c.is_control() { ' ' } else { c }).collect())
}

impl Session {
    // ---- agent requests (bridge mode; NeedsBridge when !window) ---------------------------------------------
    pub fn submit_job(&mut self, client: &str, note: &str) -> Result<SubmissionView, EngineError> {
        if !self.cfg.window {
            return Err(EngineError::NeedsBridge);
        }
        let note = clean_note(note)?;
        let client: String = client.chars().filter(|c| !c.is_control()).take(64).collect();
        let idx = self.doc_index()?;
        let built = self.build_job(Some(idx), &JobOptions::default()).map_err(|r| EngineError::JobRefused(Box::new(r)))?;
        let now = self.cfg.clock.now();
        let view = SubmissionView {
            submission: RequestId(0),
            machine: built.summary.machine.clone(),
            fingerprint: built.summary.fingerprint,
            state: SubmissionState::AwaitingHuman,
            note,
            client,
            summary: built.summary,
            expires_in_s: SUBMISSION_TTL.as_secs(),
            outcome: None,
            stop_report: None,
        };
        let (view, moved) = self.requests.add_submission(view, idx, now);
        for (id, state) in moved {
            self.push_event(crate::devices::Event::Request { id, state });
        }
        Ok(view)
    }

    pub fn request_frame(&mut self, client: &str, mode: FrameMode) -> Result<FrameRequestView, EngineError> {
        if !self.cfg.window {
            return Err(EngineError::NeedsBridge);
        }
        let client: String = client.chars().filter(|c| !c.is_control()).take(64).collect();
        let idx = self.doc_index()?;
        let (project, selection, machine) = {
            let d = self.doc()?;
            (Arc::clone(&d.project), d.selection.clone(), d.machine.clone())
        };
        let profile = self.profile(&machine).ok_or_else(|| EngineError::refused(RefusalCode::NoDevice, format!("no machine profile `{}`", machine.0)))?;
        let popts = self.plan_options(&project, &selection, project.job.cut_selected, None, None);
        let plan = openlaser_plan::plan(&project, profile, &popts).map_err(|e| EngineError::JobRefused(Box::new(JobRefused::error(format!("plan: {e}")))))?;
        openlaser_safety::validate_frame(&plan, profile, popts.frame_source).map_err(|r| {
            EngineError::JobRefused(Box::new(JobRefused { fingerprint: Some(r.fingerprint), refusals: r.refusals, notices: r.notices, error: None }))
        })?;
        let now = self.cfg.clock.now();
        let view = FrameRequestView {
            request: RequestId(0),
            machine,
            fingerprint: plan.fingerprint,
            mode,
            state: FrameRequestState::AwaitingHuman,
            client,
            expires_in_s: FRAME_REQUEST_TTL.as_secs(),
        };
        let (view, moved) = self.requests.add_frame_request(view, idx, now);
        for (id, state) in moved {
            self.push_event(crate::devices::Event::Request { id, state });
        }
        Ok(view)
    }

    /// Human only. A frame request: frames exactly as the human's Frame button. A submission awaiting a human:
    /// frames it (Once); a submission awaiting ARM: opens the ARM dialog (`request_arm`, a cold rebuild).
    pub fn approve_request(&mut self, origin: Origin, id: RequestId) -> Result<(), EngineError> {
        if origin != Origin::Human {
            return Err(EngineError::human_only("request.approve"));
        }
        if let Some(r) = self.requests.frame_requests.iter().find(|r| r.view.request == id) {
            let (state, doc, mode, fingerprint) = (r.view.state, r.doc, r.view.mode, r.view.fingerprint);
            if state != FrameRequestState::AwaitingHuman {
                return Err(not_awaiting(id, state.as_str()));
            }
            if doc >= self.docs.len() {
                return Err(not_awaiting(id, "stale"));
            }
            self.select_doc(doc)?;
            // Exactly the job the agent asked about: a fingerprint that moved on is stale, whatever the revision says.
            self.start_frame_of(origin, mode, None, None, Some(fingerprint))?;
            let mut moved = Vec::new();
            if let Some(r) = self.requests.frame_mut(id) {
                Requests::set_frame(r, FrameRequestState::Framing, &mut moved);
            }
            for (id, state) in moved {
                self.push_event(crate::devices::Event::Request { id, state });
            }
            return Ok(());
        }
        let Some(r) = self.requests.submissions.iter().find(|r| r.view.submission == id) else { return Err(unknown(id)) };
        let (state, doc, fingerprint) = (r.view.state, r.doc, r.view.fingerprint);
        if doc >= self.docs.len() {
            return Err(not_awaiting(id, "stale"));
        }
        match state {
            SubmissionState::AwaitingHuman => {
                self.select_doc(doc)?;
                self.start_frame_of(origin, FrameMode::Once, None, None, Some(fingerprint))?;
                Ok(())
            }
            SubmissionState::AwaitingArm => {
                self.select_doc(doc)?;
                self.request_arm_of(origin, Some(fingerprint)).map(|_| ())
            }
            other => Err(not_awaiting(id, other.as_str())),
        }
    }

    /// Human only: Decline a frame request, Reject a submission.
    pub fn decline_request(&mut self, origin: Origin, id: RequestId) -> Result<(), EngineError> {
        if origin != Origin::Human {
            return Err(EngineError::human_only("request.decline"));
        }
        let mut moved = Vec::new();
        if let Some(r) = self.requests.frame_mut(id) {
            if r.view.state != FrameRequestState::AwaitingHuman {
                return Err(not_awaiting(id, r.view.state.as_str()));
            }
            Requests::set_frame(r, FrameRequestState::Declined, &mut moved);
        } else if let Some(r) = self.requests.sub_mut(id) {
            if !r.view.state.pending() {
                return Err(not_awaiting(id, r.view.state.as_str()));
            }
            Requests::set_sub(r, SubmissionState::Rejected, &mut moved);
        } else {
            return Err(unknown(id));
        }
        for (id, state) in moved {
            self.push_event(crate::devices::Event::Request { id, state });
        }
        Ok(())
    }

    /// The agent takes its request back (a pending submission → withdrawn; a frame request awaiting a human →
    /// declined).
    pub fn withdraw_request(&mut self, id: RequestId) -> Result<(), EngineError> {
        let mut moved = Vec::new();
        if let Some(r) = self.requests.sub_mut(id) {
            if !r.view.state.pending() {
                return Err(not_awaiting(id, r.view.state.as_str()));
            }
            Requests::set_sub(r, SubmissionState::Withdrawn, &mut moved);
        } else if let Some(r) = self.requests.frame_mut(id) {
            if r.view.state != FrameRequestState::AwaitingHuman {
                return Err(not_awaiting(id, r.view.state.as_str()));
            }
            Requests::set_frame(r, FrameRequestState::Declined, &mut moved);
        } else {
            return Err(unknown(id));
        }
        for (id, state) in moved {
            self.push_event(crate::devices::Event::Request { id, state });
        }
        Ok(())
    }

    pub fn requests(&self) -> &Requests {
        &self.requests
    }
}

#[cfg(test)]
mod tests {
    use openlaser_types::{FrameSource, FrameStyle, SessionId};

    use super::*;
    use crate::devices::tests::summary;

    fn view(machine: &str, fp: u8) -> SubmissionView {
        let fp = Fingerprint([fp; 32]);
        SubmissionView {
            submission: RequestId(0),
            machine: MachineId(machine.into()),
            fingerprint: fp,
            state: SubmissionState::AwaitingHuman,
            note: String::new(),
            client: "agent".into(),
            summary: summary(fp, machine, &[10.0]),
            expires_in_s: 0,
            outcome: None,
            stop_report: None,
        }
    }

    fn receipt(machine: &str, fp: u8, at: MonoTime, source: FrameSource) -> FrameReceipt {
        FrameReceipt {
            fingerprint: Fingerprint([fp; 32]),
            machine: MachineId(machine.into()),
            session: SessionId(1),
            full_passes: 1,
            at,
            style: FrameStyle::Contour,
            source,
            origin: FrameOrigin::Human,
        }
    }

    fn state(r: &Requests, id: RequestId) -> SubmissionState {
        r.submission(id).unwrap().state
    }

    #[test]
    fn submission_lifecycle_to_finished() {
        let mut r = Requests::default();
        let t0 = MonoTime::default();
        let (v, moved) = r.add_submission(view("m", 1), 0, t0);
        assert_eq!(moved, vec![(v.submission, "awaiting_human".to_owned())]);
        let m = MachineId("m".into());
        assert!(r.armed(&m, Fingerprint([1; 32])).is_empty(), "an agent submission cannot run before framing");
        r.human_framed(0, &m, Fingerprint([1; 32]));
        assert_eq!(state(&r, v.submission), SubmissionState::Framing);
        r.frame_completed(&m, &receipt("m", 1, t0, FrameSource::ToolLayersOnly));
        assert_eq!(state(&r, v.submission), SubmissionState::Framing, "a tool-layer frame does not cover the job");
        r.frame_completed(&m, &receipt("m", 1, t0, FrameSource::Job));
        assert_eq!(state(&r, v.submission), SubmissionState::AwaitingArm);
        r.armed(&m, Fingerprint([1; 32]));
        assert_eq!(state(&r, v.submission), SubmissionState::Running);
        r.finished(&m, &Outcome::Finished { duration: Duration::from_secs(3) });
        let s = r.submission(v.submission).unwrap();
        assert_eq!(s.state, SubmissionState::Finished);
        assert!(s.outcome.is_some());
    }

    #[test]
    fn supersede_stale_expire_and_receipt_age() {
        let mut r = Requests::default();
        let t0 = MonoTime::default();
        let m = MachineId("m".into());
        let (a, _) = r.add_submission(view("m", 1), 0, t0);
        let (b, _) = r.add_submission(view("m", 2), 0, t0);
        assert_eq!(state(&r, a.submission), SubmissionState::Superseded);
        assert_eq!(state(&r, b.submission), SubmissionState::AwaitingHuman);
        assert_eq!(r.pending_submission(&m), Some(b.submission));
        r.mark_stale(0);
        assert_eq!(state(&r, b.submission), SubmissionState::Stale);
        let (c, _) = r.add_submission(view("m", 3), 1, t0);
        r.mark_stale(0);
        assert_eq!(state(&r, c.submission), SubmissionState::AwaitingHuman, "another document's edit does not stale it");
        r.tick(t0.plus(SUBMISSION_TTL), &[]);
        assert_eq!(state(&r, c.submission), SubmissionState::Expired);
        // awaiting_arm falls back once the receipt is too old
        let (d, _) = r.add_submission(view("m", 4), 1, t0);
        r.human_framed(1, &m, Fingerprint([4; 32]));
        r.frame_completed(&m, &receipt("m", 4, t0, FrameSource::Job));
        assert_eq!(state(&r, d.submission), SubmissionState::AwaitingArm);
        r.tick(t0.plus(Duration::from_secs(60)), &[(m.clone(), Some(t0))]);
        assert_eq!(state(&r, d.submission), SubmissionState::AwaitingArm);
        r.tick(t0.plus(FRAME_RECEIPT_MAX_AGE + Duration::from_secs(1)), &[(m.clone(), Some(t0))]);
        assert_eq!(state(&r, d.submission), SubmissionState::AwaitingHuman);
    }

    #[test]
    fn stops_and_faults() {
        let mut r = Requests::default();
        let t0 = MonoTime::default();
        let m = MachineId("m".into());
        let (a, _) = r.add_submission(view("m", 1), 0, t0);
        r.human_framed(0, &m, Fingerprint([1; 32]));
        r.stopped(&m, None);
        assert_eq!(state(&r, a.submission), SubmissionState::AwaitingHuman, "a stopped frame goes back to the human");
        r.human_framed(0, &m, Fingerprint([1; 32]));
        r.frame_completed(&m, &receipt("m", 1, t0, FrameSource::Job));
        r.stopped(&m, None);
        assert_eq!(state(&r, a.submission), SubmissionState::AwaitingArm, "ending a loop frame keeps the receipt");
        r.armed(&m, Fingerprint([1; 32]));
        r.faulted(&m);
        assert_eq!(state(&r, a.submission), SubmissionState::Fault);
    }

    #[test]
    fn frame_requests() {
        let mut r = Requests::default();
        let t0 = MonoTime::default();
        let m = MachineId("m".into());
        let fv = |mode| FrameRequestView {
            request: RequestId(0),
            machine: m.clone(),
            fingerprint: Fingerprint([5; 32]),
            mode,
            state: FrameRequestState::AwaitingHuman,
            client: "agent".into(),
            expires_in_s: 0,
        };
        let (a, _) = r.add_frame_request(fv(FrameMode::Once), 0, t0);
        let (b, _) = r.add_frame_request(fv(FrameMode::Loop), 0, t0);
        assert_eq!(r.frame_request(a.request).unwrap().state, FrameRequestState::Declined, "superseded");
        assert_eq!(r.pending_frame_request(&m), Some(b.request));
        r.tick(t0.plus(FRAME_REQUEST_TTL), &[]);
        assert_eq!(r.frame_request(b.request).unwrap().state, FrameRequestState::Expired);
        let (c, _) = r.add_frame_request(fv(FrameMode::Once), 0, t0);
        let mut moved = Vec::new();
        Requests::set_frame(r.frame_mut(c.request).unwrap(), FrameRequestState::Framing, &mut moved);
        r.frame_completed(&m, &receipt("m", 5, t0, FrameSource::Job));
        assert_eq!(r.frame_request(c.request).unwrap().state, FrameRequestState::Done);
        let (d, _) = r.add_frame_request(fv(FrameMode::Loop), 0, t0);
        Requests::set_frame(r.frame_mut(d.request).unwrap(), FrameRequestState::Framing, &mut moved);
        r.frame_completed(&m, &receipt("m", 5, t0, FrameSource::Job));
        assert_eq!(r.frame_request(d.request).unwrap().state, FrameRequestState::Framing, "a loop frames until stopped");
        r.stopped_all();
        assert_eq!(r.frame_request(d.request).unwrap().state, FrameRequestState::Stopped);
    }

    #[test]
    fn notes_are_plain_and_bounded() {
        assert_eq!(clean_note("a\nb\u{7}c").unwrap(), "a b c");
        assert!(clean_note(&"x".repeat(281)).is_err());
        assert!(clean_note(&"é".repeat(280)).is_ok());
    }

    #[test]
    fn state_strings_match_serde() {
        for s in [SubmissionState::AwaitingHuman, SubmissionState::AwaitingArm, SubmissionState::Superseded] {
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        for s in [FrameRequestState::AwaitingHuman, FrameRequestState::Done] {
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
    }
}

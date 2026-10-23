//! AUT C3.4/C3.5 state machines (red zone review). WP0 stubs: no requests.

use openlaser_driver::{Outcome, StopReport};
use openlaser_types::{EmissionSummary, Fingerprint, FrameMode, MachineId};
use serde::Serialize;

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

/// ≤ 1 pending submission and ≤ 1 pending frame request per machine (a new one supersedes); submissions expire
/// after 15 min, frame requests after 2 min; any revision bump → Stale. Nothing here touches a driver except
/// approve_request (Human), which calls start_frame exactly as the human's own Frame button does.
#[derive(Default)]
pub struct Requests {
    submissions: Vec<SubmissionView>,
    frame_requests: Vec<FrameRequestView>,
}

impl Requests {
    pub fn submissions(&self) -> Vec<SubmissionView> {
        self.submissions.clone()
    }

    pub fn frame_requests(&self) -> Vec<FrameRequestView> {
        self.frame_requests.clone()
    }
}

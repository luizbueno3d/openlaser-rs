//! Framing: styles, sources and the receipt the driver issues after each fully acknowledged pass.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ids::{Fingerprint, MachineId, SessionId};
use crate::time::MonoTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameStyle {
    Bounds,
    Contour,
    /// P1: the planner refuses it in this build.
    Hull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameSource {
    Job,
    ToolLayersOnly,
    JobAndToolLayers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameMode {
    Once,
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameOrigin {
    Human,
    /// Reserved; nothing in this build creates it.
    Agent,
}

pub const FRAME_RECEIPT_MAX_AGE: Duration = Duration::from_secs(600);

/// Issued by the driver only after the board has EXECUTED the pass (S20): a status read shows !BUSY after the list
/// that carried it. Acknowledged-but-queued packets never count. `fingerprint` is the plan fingerprint (S21, style
/// and source excluded); `style` and `source` record what was traced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameReceipt {
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub session: SessionId,
    pub full_passes: u32,
    pub at: MonoTime,
    pub style: FrameStyle,
    pub source: FrameSource,
    pub origin: FrameOrigin,
}

impl FrameReceipt {
    /// `source != ToolLayersOnly`.
    pub fn covers_job(&self) -> bool {
        self.source != FrameSource::ToolLayersOnly
    }
}

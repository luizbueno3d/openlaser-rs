//! What will be emitted, decoded from the bytes that will be sent (SPEC §1.3 rule 3).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ids::{Fingerprint, LayerKey, MachineId};
use crate::refusal::Notice;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionMode {
    Line,
    Fill,
    Image,
}

/// Words read back from a marking list (galvo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalvoWords {
    pub mark_current: u16,
    pub q_period: u16,
    pub pulse_ns: u16,
    pub speed_units: u16,
    pub jump_speed_units: u16,
}

/// One emitting layer-pass. Every number is decoded from the bytes that will be sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassRow {
    pub layer: LayerKey,
    pub layer_name: String,
    pub sub: u8,
    pub pass: u16,
    pub of: u16,
    pub mode: SectionMode,
    /// galvo: mark_current × 100 / 4095; gantry: S / s_max × 100
    pub power_pct: f64,
    /// galvo: 20000 / q_period
    pub frequency_khz: Option<f64>,
    pub pulse_ns: Option<u16>,
    /// The ACTUAL speed (galvo: from speed units and F).
    pub speed_mm_s: f64,
    pub galvo: Option<GalvoWords>,
    pub paths: u32,
    pub mark_mm: f64,
    pub bounds: Bounds,
    #[serde(rename = "estimateS", with = "crate::serde_secs")]
    pub estimate: Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmissionSummary {
    pub fingerprint: Fingerprint,
    /// S22: blake3 over, in send order, every single command (12 bytes) and packet (3072 bytes) of one repeat,
    /// then the repeat count as u16 LE. Computed from the job's bytes, never from the plan.
    pub bytes_hash: Fingerprint,
    pub machine: MachineId,
    pub rows: Vec<PassRow>,
    pub notices: Vec<Notice>,
    pub repeats: u16,
    /// All repeats.
    #[serde(rename = "estimateS", with = "crate::serde_secs")]
    pub estimate: Duration,
    /// Emission that can still happen after a host crash (run-ahead bound, PLAN §3.5).
    #[serde(rename = "crashBoundS", with = "crate::serde_secs")]
    pub crash_bound: Duration,
    pub packets: u32,
    pub work_area: Bounds,
    /// As applied by the machine transform.
    pub negate: [bool; 2],
    pub swap_xy: bool,
}

/// A burned segment decoded from the bytes (MarkTo while MO is open and laser port bit 0 is set), machine mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EmittedSegment {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub power_pct: f64,
}

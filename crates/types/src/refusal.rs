//! Refusal and notice codes. The snake_case wire strings are stable forever.

use serde::{Deserialize, Serialize};

use crate::ids::{LayerKey, ObjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RefusalCode {
    // geometry
    OutsideWorkArea,
    NotEncodable,
    NonFinite,
    MalformedPoint,
    TooManyPaths,
    TooManyPointsPerPath,
    TooManyPointsPerPass,
    OutOfBed,
    // laser parameters
    PowerAboveMax,
    FrequencyOutOfRange,
    PulseNotInTable,
    QPulseUnsupported,
    SpeedZeroUnits,
    SpeedAboveMax,
    // objects and layers
    OrphanObject,
    HiddenLayerWithOutput,
    FillWithoutHatch,
    NothingToMark,
    UnsupportedMode,
    WrongDeviceKind,
    // job flow and ARM
    MachineMismatch,
    Stale,
    FrameRequired,
    FrameReceiptExpired,
    FrameNotHuman,
    FrameDoesNotCoverJob,
    ArmTextMismatch,
    ChecklistIncomplete,
    ArmExpired,
    ArmNonceUsed,
    ArmSessionMismatch,
    /// S22: the job handed to the driver is not the bytes the human confirmed.
    BytesMismatch,
    /// §5.3: a stop tripped after this command was queued cancelled it.
    StoppedAfterQueue,
    // access and devices
    HumanOnly,
    NeedsBridge,
    NoDevice,
    NotConnected,
    MachineBusy,
    HumanConfirmationOpen,
    HardwareDisabled,
    BenchSessionRequired,
    BoardLocked,
    CloneBoard,
    NotATty,
    GrblPreflight,
    /// S23: first emission of a bench session above 10 %.
    BenchPowerCap,
    /// `openlaser-hw run` without --allow-emission (red light only).
    EmissionNotAllowed,
    /// macOS: device unconfigured and the profile's usb_set_configuration is off (H12).
    UsbNotConfigured,
    /// Build-only (SPEC §3.0); removed at I3.
    NotImplemented,
}

impl RefusalCode {
    /// The snake_case wire string; stable forever.
    pub fn as_str(self) -> &'static str {
        use RefusalCode::*;
        match self {
            OutsideWorkArea => "outside_work_area",
            NotEncodable => "not_encodable",
            NonFinite => "non_finite",
            MalformedPoint => "malformed_point",
            TooManyPaths => "too_many_paths",
            TooManyPointsPerPath => "too_many_points_per_path",
            TooManyPointsPerPass => "too_many_points_per_pass",
            OutOfBed => "out_of_bed",
            PowerAboveMax => "power_above_max",
            FrequencyOutOfRange => "frequency_out_of_range",
            PulseNotInTable => "pulse_not_in_table",
            QPulseUnsupported => "q_pulse_unsupported",
            SpeedZeroUnits => "speed_zero_units",
            SpeedAboveMax => "speed_above_max",
            OrphanObject => "orphan_object",
            HiddenLayerWithOutput => "hidden_layer_with_output",
            FillWithoutHatch => "fill_without_hatch",
            NothingToMark => "nothing_to_mark",
            UnsupportedMode => "unsupported_mode",
            WrongDeviceKind => "wrong_device_kind",
            MachineMismatch => "machine_mismatch",
            Stale => "stale",
            FrameRequired => "frame_required",
            FrameReceiptExpired => "frame_receipt_expired",
            FrameNotHuman => "frame_not_human",
            FrameDoesNotCoverJob => "frame_does_not_cover_job",
            ArmTextMismatch => "arm_text_mismatch",
            ChecklistIncomplete => "checklist_incomplete",
            ArmExpired => "arm_expired",
            ArmNonceUsed => "arm_nonce_used",
            ArmSessionMismatch => "arm_session_mismatch",
            BytesMismatch => "bytes_mismatch",
            StoppedAfterQueue => "stopped_after_queue",
            HumanOnly => "human_only",
            NeedsBridge => "needs_bridge",
            NoDevice => "no_device",
            NotConnected => "not_connected",
            MachineBusy => "machine_busy",
            HumanConfirmationOpen => "human_confirmation_open",
            HardwareDisabled => "hardware_disabled",
            BenchSessionRequired => "bench_session_required",
            BoardLocked => "board_locked",
            CloneBoard => "clone_board",
            NotATty => "not_a_tty",
            GrblPreflight => "grbl_preflight",
            BenchPowerCap => "bench_power_cap",
            EmissionNotAllowed => "emission_not_allowed",
            UsbNotConfigured => "usb_not_configured",
            NotImplemented => "not_implemented",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub code: RefusalCode,
    /// One sentence the user can act on, naming the value and the limit.
    pub message: String,
    pub object: Option<ObjectId>,
    pub layer: Option<LayerKey>,
    /// mm, machine frame.
    pub at: Option<[f64; 2]>,
}

impl Refusal {
    pub fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Refusal { code, message: message.into(), object: None, layer: None, at: None }
    }

    pub fn object(mut self, id: ObjectId) -> Self {
        self.object = Some(id);
        self
    }

    pub fn layer(mut self, k: LayerKey) -> Self {
        self.layer = Some(k);
        self
    }

    pub fn at(mut self, p: [f64; 2]) -> Self {
        self.at = Some(p);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoticeCode {
    RotaryScanAngle,
    OpenShapeUnhatched,
    PulseSnapped,
    SpeedQuantized,
    ToolLayerNotBurned,
    ImportEntitySkipped,
    ImportColorUnmatched,
    ImportTextIgnored,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub n: u32,
    pub code: NoticeCode,
    pub message: String,
    pub object: Option<ObjectId>,
    pub layer: Option<LayerKey>,
}

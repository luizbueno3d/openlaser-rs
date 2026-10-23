//! The rule table (SPEC §3.8): every refusal `validate` and `validate_frame` can return, with the rule in words.
//! `rules_table_complete` checks it against the codes the checks emit.

use openlaser_types::RefusalCode;

pub(crate) static RULES: &[(RefusalCode, &str)] = &[
    // Geometry (burn sections and the frame alike; never clamped, never truncated).
    (
        RefusalCode::OutsideWorkArea,
        "Galvo: every point lies inside the machine's effective work area (the rated lens field intersected with the strictly encodable square |v| < F/2), within 1e-6 mm; B10. An object with any point beyond plan::MAX_COORD_MM (100 m) is not planned at all and is refused by name",
    ),
    (
        RefusalCode::NotEncodable,
        "Galvo: every point converts with mm_to_galvo on its axis (+F/2 exactly is refused, never clamped); every quantised word re-derives from its setting; the first-jump long delay, first-jump settle, MO settle and jump speed are never 0, and the first-jump long delay is never sent with a negative (0x8000) sign (B3)",
    ),
    (RefusalCode::NonFinite, "No NaN or infinite coordinate anywhere, and no object whose geometry or transform is not finite"),
    (
        RefusalCode::MalformedPoint,
        "Points have exactly two coordinates: refused at import (B9); a plan's Polyline cannot hold any other shape. Every path of a section and of a frame has at least 2 points",
    ),
    (RefusalCode::TooManyPaths, "At most JobLimits.max_paths (3000) paths in one layer-pass and in the frame"),
    (RefusalCode::TooManyPointsPerPath, "At most JobLimits.max_points_per_path (2000) points in one path"),
    (RefusalCode::TooManyPointsPerPass, "At most JobLimits.max_points_per_pass (20000) points in one layer-pass and in the frame"),
    (
        RefusalCode::OutOfBed,
        "Gantry: every point lies on the bed in machine coordinates, within 1e-6 mm. An object with any point beyond plan::MAX_COORD_MM (100 m) is not planned at all and is refused by name",
    ),
    // Laser parameters (judged on the values the quantised words carry).
    (RefusalCode::PowerAboveMax, "Power is at most the source's max_power_pct"),
    (RefusalCode::FrequencyOutOfRange, "Galvo frequency lies within the source's freq_khz range (1..4000 kHz for the JPT M7)"),
    (
        RefusalCode::PulseNotInTable,
        "The Q-pulse width is in the source's pulse table, or within 0.5 ns of an entry (then it snaps with a notice); NaN or unset is refused",
    ),
    (RefusalCode::QPulseUnsupported, "A galvo layer on a source whose Q-pulse control is off (this build drives MOPA sources only)"),
    (RefusalCode::SpeedZeroUnits, "Galvo mark speed encodes to at least 1 speed unit (B7: below 3.6115 mm/s at F = 236.68 it would be 0)"),
    (RefusalCode::SpeedAboveMax, "Speed is at most the machine's max_speed, galvo and gantry alike (never capped)"),
    // Objects and layers (from the plan's facts: nothing is dropped silently).
    (RefusalCode::OrphanObject, "Every object is on a layer that exists (B8)"),
    (RefusalCode::HiddenLayerWithOutput, "No layer is hidden while its Output is on (B6: it would burn without being seen)"),
    (RefusalCode::FillWithoutHatch, "A Fill layer with shapes yields at least one hatch segment; open shapes on it become numbered notices"),
    (
        RefusalCode::UnsupportedMode,
        "Nothing the build cannot do: Offset Fill, image burning, per-layer timings, wobble, Hull frames, out-of-range passes or repeats; no section on a tool layer",
    ),
    (RefusalCode::NothingToMark, "At least one section burns something, the repeat count is at least 1, and a frame has something to trace"),
    (RefusalCode::WrongDeviceKind, "The plan, every section's cut settings and the project's device hint match the machine's kind"),
    // Plan integrity.
    (RefusalCode::MachineMismatch, "The plan was made for the machine profile it is validated against"),
    (RefusalCode::Stale, "The plan's fingerprint and quantised words are exactly what its contents and the machine produce now"),
    (RefusalCode::FrameDoesNotCoverJob, "Every burn point lies within the bounds of the job frame (frame ⊇ burn, defence in depth)"),
];

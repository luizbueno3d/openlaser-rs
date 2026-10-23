//! Machine profiles (SPEC §3.4; owned by WP1). No I/O.
//!
#![forbid(unsafe_code)]

mod builtin;
pub mod markcfg;
pub mod om;
mod profile;

pub use builtin::{jpt_m7_200, jpt_m7_200_sim, om_gantries};
pub use profile::{
    FieldCalibration, FieldRect, FirstJump, GalvoLink, GalvoMachine, GalvoTimingDefaults, GantryLink, GantryMachine, GantryTool, JobLimits, Kinematics,
    LaserSource, LmcBoardFilter, MachineProfile, PortMap, ProfileStore, RedDotCal, RunAhead, SourceKind, validate_profile,
};

// Re-exports for callers that name these through the profile (SPEC §3.0).
pub use openlaser_doc::{LayerDefaults, WorkspaceOrigin};
pub use openlaser_types::{DeviceKind, MachineId, Mm, MmPerS, Us};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid machine profile: {0}")]
    InvalidProfile(String),
    #[error("invalid markcfg: {0}")]
    InvalidMarkCfg(String),
    #[error("invalid OM calibration: {0}")]
    InvalidCalibration(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpt_m7_matches_om_profile() {
        let profile = jpt_m7_200();
        let Kinematics::Galvo(ref galvo) = profile.kind else {
            panic!("expected galvo profile");
        };
        assert_eq!(profile.id.0, "jpt-m7-200");
        assert_eq!(galvo.board.vid, 0x9588);
        assert_eq!(galvo.board.pid, 0x9899);
        assert_eq!(galvo.encoding_field.get(), 200.0);
        assert_eq!(galvo.cal.scale, [1.1834, 1.1834]);
        assert_eq!(galvo.field_f(), [236.68, 236.68]);
        assert_eq!(galvo.source.freq_khz, [1.0, 4000.0]);
        assert_eq!(galvo.source.pulse_table_ns, openlaser_types::galvo::JPT_M7_PULSES_NS);
        assert_eq!(galvo.max_speed.get(), 6000.0);
        assert_eq!(galvo.first_jump.delay_us, 200.0);
        assert_eq!(galvo.first_jump.settle_10us, 100);
        assert!(validate_profile(&profile).is_ok());
    }

    #[test]
    fn markcfg_parses_om_profile() {
        let bytes = std::fs::read(openlaser_testkit::golden_dir().join("inputs/openmopa-machine.ini")).unwrap();
        let parsed = markcfg::parse(&bytes).unwrap();
        assert_eq!(parsed.values.get("FIELDSIZE").map(String::as_str), Some("200"));
        let (profile, applied) = markcfg::apply(&parsed, &jpt_m7_200()).unwrap();
        let Kinematics::Galvo(galvo) = profile.kind else {
            panic!("expected galvo profile");
        };
        assert_eq!(galvo.encoding_field.get(), 200.0);
        assert_eq!(galvo.source.freq_khz, [1.0, 4000.0]);
        assert!(applied.applied.iter().any(|key| key == "FIELDSIZE"));
    }

    #[test]
    fn calibration_out_of_range_refused() {
        let fixture = std::fs::read(openlaser_testkit::golden_dir().join("inputs/calibration.json")).unwrap();
        assert_eq!(om::parse_calibration(&fixture).unwrap(), 1.1834);
        assert!(om::parse_calibration(br#"{"scale_correction":2.01}"#).is_err());
    }

    #[test]
    fn effective_work_area_is_rated_field() {
        let profile = jpt_m7_200();
        let Kinematics::Galvo(galvo) = profile.kind else {
            panic!("expected galvo profile");
        };
        let area = galvo.effective_work_area();
        assert_eq!(area.min, [-100.0, -100.0]);
        assert_eq!(area.max, [100.0, 100.0]);
    }

    #[test]
    fn validate_profile_rejects_empty_checklist() {
        let mut profile = jpt_m7_200();
        profile.checklist.clear();
        assert!(validate_profile(&profile).is_err());
    }

    #[test]
    fn work_area_cannot_grow_past_h27_cleared() {
        let mut profile = jpt_m7_200();
        let Kinematics::Galvo(galvo) = &mut profile.kind else {
            panic!("expected galvo profile");
        };
        galvo.h27_cleared = Some(FieldRect { min: [-90.0, -90.0], max: [90.0, 90.0] });
        assert!(validate_profile(&profile).is_err());
    }
}

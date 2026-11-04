//! Machine profiles (SPEC §3.4; owned by WP1). No I/O.
//!
#![forbid(unsafe_code)]

mod builtin;
pub mod markcfg;
pub mod om;
mod profile;

pub use builtin::{jpt_m7_200, jpt_m7_200_sim, om_gantries};
pub use profile::{
    FieldCalibration, FieldRect, FirstJump, FootSwitchCfg, GalvoLink, GalvoMachine, GalvoTimingDefaults, GantryLink, GantryMachine, GantryTool, JobLimits,
    Kinematics, LaserSource, LmcBoardFilter, MachineProfile, PortMap, ProfileStore, RedDotCal, RotaryAxis, RotaryCfg, RotaryKind, RunAhead, SourceKind,
    validate_profile,
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
        assert_eq!(galvo.jump_speed.get(), 3000.0);
        assert_eq!(galvo.light_speed.get(), 3000.0);
        assert_eq!(galvo.max_speed.get(), 6000.0);
        assert_eq!(galvo.mo_settle.get(), 8000.0);
        assert_eq!(galvo.timings.laser_on_us, 100.0);
        assert_eq!(galvo.timings.laser_off_us, 100.0);
        assert_eq!(galvo.timings.polygon_us, 100.0);
        assert_eq!(galvo.timings.jump_delay_short_us, 8.0);
        assert_eq!(galvo.timings.jump_delay_long_us, 200.0);
        assert_eq!(galvo.first_jump.delay_us, 200.0);
        assert_eq!(galvo.first_jump.settle_10us, 100);
        assert_eq!(galvo.run_ahead.max_packets, 3);
        assert_eq!(galvo.run_ahead.max_emission_s, 2.0);
        assert_eq!(galvo.run_ahead.watchdog_factor, 1.5);
        assert_eq!(galvo.run_ahead.watchdog_slack_s, 2.0);
        assert_eq!(galvo.ports.laser_bit, 0);
        assert_eq!(galvo.ports.red_bit, 8);
        assert_eq!(galvo.transfer_timeout_ms, 250);
        assert!(!galvo.usb_set_configuration);
        assert_eq!(galvo.work_area.min, [-100.0, -100.0]);
        assert_eq!(galvo.work_area.max, [100.0, 100.0]);
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
    fn om_gantries_all_validate() {
        let profiles = om_gantries();
        assert_eq!(profiles.len(), 3);
        for profile in profiles {
            assert!(validate_profile(&profile).is_ok(), "{}", profile.id.0);
        }
    }

    #[test]
    fn markcfg_fieldsize_keeps_user_shrunk_work_area() {
        let mut base = jpt_m7_200();
        let Kinematics::Galvo(galvo) = &mut base.kind else { panic!("expected galvo profile") };
        galvo.work_area.min = [-50.0, -40.0];
        galvo.work_area.max = [50.0, 40.0];
        let cfg = markcfg::parse(b"[LMC_CFG]\nFIELDSIZE=200\n").unwrap();
        let (applied, _) = markcfg::apply(&cfg, &base).unwrap();
        let Kinematics::Galvo(galvo) = applied.kind else { panic!("expected galvo profile") };
        assert_eq!(galvo.work_area.min, [-50.0, -40.0]);
        assert_eq!(galvo.work_area.max, [50.0, 40.0]);
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

#[cfg(test)]
mod rotary_tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    #[test]
    fn legacy_profile_and_partial_rotary_defaults_load() {
        let mut json = serde_json::to_value(jpt_m7_200()).unwrap();
        json["kind"].as_object_mut().unwrap().remove("rotary");
        let old: MachineProfile = serde_json::from_value(json.clone()).unwrap();
        let Kinematics::Galvo(g) = old.kind else { panic!("galvo") };
        assert!(g.rotary.is_none());
        json["kind"]["rotary"] = serde_json::json!({});
        let profile: MachineProfile = serde_json::from_value(json).unwrap();
        let Kinematics::Galvo(g) = &profile.kind else { panic!("galvo") };
        assert_eq!(g.rotary, Some(RotaryCfg::default()));
        assert!(validate_profile(&profile).is_ok());
    }
    #[test]
    fn rotary_validation_refuses_each_invalid_range_even_when_disabled() {
        let mut cases = Vec::new();
        for ppr in [0, 1_000_001] {
            cases.push(RotaryCfg { pulses_per_rev: ppr, ..Default::default() });
        }
        for d in [f64::NAN, 0.0, -1.0, 500.1] {
            cases.push(RotaryCfg { drive_diameter_mm: d, ..Default::default() });
            cases.push(RotaryCfg { object_diameter_mm: d, ..Default::default() });
        }
        for split in [f64::NAN, 0.0, 0.001, 200.1] {
            cases.push(RotaryCfg { split_mm: split, ..Default::default() });
        }
        for overlap in [f64::NAN, -1.0, 5.0] {
            cases.push(RotaryCfg { overlap_mm: overlap, ..Default::default() });
        }
        for accel in [0, 10001] {
            cases.push(RotaryCfg { accel_ms: accel, ..Default::default() });
        }
        for turns in [f64::NAN, 0.0, -1.0, 20.1] {
            cases.push(RotaryCfg { max_turns_per_job: turns, ..Default::default() });
        }
        cases.push(RotaryCfg { min_speed_pps: 0, ..Default::default() });
        cases.push(RotaryCfg { min_speed_pps: 1001, max_speed_pps: 1000, ..Default::default() });
        for r in cases {
            let mut p = jpt_m7_200();
            let Kinematics::Galvo(g) = &mut p.kind else { panic!("galvo") };
            g.rotary = Some(r.clone());
            assert!(validate_profile(&p).is_err(), "{r:?}");
        }
        let area = FieldRect { min: [-100.0, -100.0], max: [100.0, 100.0] };
        assert!(
            RotaryCfg { split_mm: std::f64::consts::PI * 40.0 / 12800.0, max_turns_per_job: 20.0, accel_ms: 10000, ..Default::default() }
                .validate(area)
                .is_ok()
        );
    }
}

#[cfg(test)]
mod foot_switch_tests {
    use super::*;

    #[test]
    fn foot_switch_is_unset_by_default_and_not_serialized() {
        let p = jpt_m7_200();
        let Kinematics::Galvo(g) = &p.kind else { panic!("galvo") };
        assert_eq!(g.ports.foot_switch, None, "the pedal does nothing until a person sets the measured pin");
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("footSwitch"), "{json}");
    }

    #[test]
    fn foot_switch_levels_and_polarity() {
        let high = FootSwitchCfg::new(1, 15);
        assert!(high.pressed([0, 0x8000, 0, 0x20]));
        assert!(!high.pressed([0xffff, 0x7fff, 0xffff, 0xffff]), "only its own word and bit count");
        let low = FootSwitchCfg { active_low: true, ..FootSwitchCfg::new(2, 3) };
        assert!(low.pressed([0, 0, 0, 0]));
        assert!(!low.pressed([0, 0, 0x0008, 0]));
    }

    #[test]
    fn foot_switch_out_of_range_is_refused_by_profile_validation() {
        for bad in [
            FootSwitchCfg::new(3, 0),
            FootSwitchCfg::new(1, 16),
            FootSwitchCfg { debounce_ms: 5, ..FootSwitchCfg::new(1, 15) },
            FootSwitchCfg { debounce_ms: 2000, ..FootSwitchCfg::new(1, 15) },
        ] {
            let mut p = jpt_m7_200();
            let Kinematics::Galvo(g) = &mut p.kind else { panic!("galvo") };
            g.ports.foot_switch = Some(bad.clone());
            assert!(validate_profile(&p).is_err(), "{bad:?}");
        }
        let mut p = jpt_m7_200();
        let Kinematics::Galvo(g) = &mut p.kind else { panic!("galvo") };
        g.ports.foot_switch = Some(FootSwitchCfg::new(1, 15));
        assert!(validate_profile(&p).is_ok());
        let back: MachineProfile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}

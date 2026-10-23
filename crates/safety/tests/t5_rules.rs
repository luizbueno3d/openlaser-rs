//! T5 (SPEC §6.2, §7.4): one `safety_<code>` test per rule of §3.8, the named prototype regressions (B6, B7, B8,
//! B10), the notices, `validate_frame`, and `rules_table_complete`. Rust-authored from PLAN §3.6 and OM v1
//! (`5f7aa63` `safety.py`, `layers.py`, `live.py` `geometry_errors`): the same decision as OM v1, as a code.
//!
//! Plans are built by hand so each rule is tested on its own. Their galvo words come from the T0 formulas written
//! out below (not from `types::galvo`), so after I1 these tests also cross-check WP4's encoders through `safety`'s
//! word re-derivation. Tests whose verdict depends on another package's behaviour (the work area from WP1's
//! `effective_work_area`, the encoders from WP4) are `#[ignore = "needs I1"]` (§7.0 rule 6).

// An integration-test crate is test code; clippy's in-test allowances only reach `#[test]` functions, not helpers.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use openlaser_geom::Polyline;
use openlaser_machine::{GalvoMachine, GantryLink, GantryMachine, GantryTool, Kinematics, MachineProfile, WorkspaceOrigin, jpt_m7_200_sim};
use openlaser_plan::{CutQ, GalvoCutQ, GantryCutQ, JobPlan, PlanFacts, PulseDecision, ResolvedTimings, Section, fingerprint_of};
use openlaser_safety::{FramePlan, Refused, Validated, rules, validate, validate_frame};
use openlaser_types::{
    DeviceKind, Fingerprint, FrameSource, FrameStyle, Hz, LayerKey, MachineId, MmPerS, NoticeCode, ObjectId, Percent, RefusalCode, SectionMode,
};
use proptest::prelude::*;

// ---- the T0 formulas (SPEC §3.1 galvo.rs), written out ----------------------------------------------------------

fn units(v: f64, f: f64) -> u16 {
    let gpm = 65535.0 / f;
    (v * gpm / 1000.0).trunc().min(65535.0) as u16
}

fn delay(d: f64) -> (u16, u16) {
    (d.abs().trunc() as u16, if d > 0.0 { 0x0000 } else { 0x8000 })
}

fn jpt() -> MachineProfile {
    jpt_m7_200_sim()
}

fn galvo(m: &MachineProfile) -> &GalvoMachine {
    match &m.kind {
        Kinematics::Galvo(g) => g,
        Kinematics::Gantry(_) => panic!("expected a galvo profile"),
    }
}

/// The words `plan` quantises these settings to on `m` (JPT M7: 1000 mm/s → 276, 30 % → 1228, 30 kHz → 667).
fn words(m: &MachineProfile, speed: f64, power: f64, khz: u32, pulse: PulseDecision) -> GalvoCutQ {
    let g = galvo(m);
    let f = g.field_f()[0];
    let speed_units = units(speed, f);
    let t = &g.timings;
    GalvoCutQ {
        requested_speed: MmPerS::new(speed).unwrap(),
        speed_units,
        // `units_to_speed`: units × F × 1000 / 65535, in that order.
        actual_speed_mm_s: f64::from(speed_units) * f * 1000.0 / 65535.0,
        power: Percent::new(power).unwrap(),
        mark_current: (power * 4095.0 / 100.0).round_ties_even() as u16,
        frequency: Hz::new(khz * 1000).unwrap(),
        q_period: (20000.0 / f64::from(khz)).round_ties_even() as u16,
        pulse,
        timings: ResolvedTimings {
            laser_on: delay(t.laser_on_us),
            laser_off: delay(t.laser_off_us),
            polygon: delay(t.polygon_us),
            jump_delay_short: delay(t.jump_delay_short_us),
            jump_delay_long: delay(g.first_jump.delay_us),
            jump_speed_units: units(g.jump_speed.get(), f),
            mo_settle_10us: (g.mo_settle.get() / 10.0).trunc() as u16,
            first_jump_settle_10us: g.first_jump.settle_10us,
        },
    }
}

fn default_words(m: &MachineProfile) -> GalvoCutQ {
    words(m, 1000.0, 30.0, 30, PulseDecision::Exact(200))
}

fn line(pts: &[[f64; 2]]) -> Polyline {
    Polyline(pts.to_vec())
}

fn square(cx: f64, cy: f64, half: f64) -> Polyline {
    line(&[[cx - half, cy - half], [cx + half, cy - half], [cx + half, cy + half], [cx - half, cy + half], [cx - half, cy - half]])
}

fn section(layer: u8, paths: Vec<Polyline>, cut: CutQ) -> Section {
    Section {
        layer: LayerKey::Color(layer),
        layer_name: format!("C{layer:02}"),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        cut,
        paths,
        objects: vec![ObjectId(1)],
        estimate: Duration::from_millis(10),
    }
}

/// A plan for `m` whose fingerprint matches its contents and whose frame is the contour of its Line paths.
fn job(m: &MachineProfile, sections: Vec<Section>) -> JobPlan {
    let device = openlaser_plan::device_of(m);
    let frame: Vec<Polyline> = sections.iter().flat_map(|s| s.paths.clone()).collect();
    refingerprint(
        JobPlan {
            machine: m.id.clone(),
            device,
            sections,
            frame,
            tool_frame: Vec::new(),
            facts: PlanFacts::default(),
            repeats: 1,
            estimate: Duration::from_millis(10),
            fingerprint: Fingerprint([0; 32]),
        },
        m,
    )
}

fn refingerprint(mut p: JobPlan, m: &MachineProfile) -> JobPlan {
    p.fingerprint = fingerprint_of(m, p.device, &p.sections, p.repeats);
    p
}

fn galvo_job(paths: Vec<Polyline>) -> (MachineProfile, JobPlan) {
    let m = jpt();
    let p = job(&m, vec![section(0, paths, CutQ::Galvo(default_words(&m)))]);
    (m, p)
}

/// The refusal codes (empty when validated), after checking every refusal is a listed rule with a message.
fn codes(r: &Result<Validated, Refused>) -> Vec<RefusalCode> {
    match r {
        Ok(_) => Vec::new(),
        Err(e) => {
            for x in &e.refusals {
                assert!(rules().iter().any(|(c, _)| *c == x.code), "{:?} is not in rules()", x.code);
                assert!(!x.message.is_empty(), "{:?} has no message", x.code);
            }
            e.refusals.iter().map(|x| x.code).collect()
        }
    }
}

fn frame_codes(r: &Result<FramePlan, Refused>) -> Vec<RefusalCode> {
    match r {
        Ok(_) => Vec::new(),
        Err(e) => e.refusals.iter().map(|x| x.code).collect(),
    }
}

fn gantry_profile() -> MachineProfile {
    let mut m = jpt();
    m.id = MachineId("diode-test".into());
    m.kind = Kinematics::Gantry(GantryMachine {
        bed_mm: [400.0, 400.0],
        origin: WorkspaceOrigin::BottomLeft,
        max_speed: MmPerS::new(100.0).unwrap(),
        travel_speed: MmPerS::new(150.0).unwrap(),
        s_max: 1000,
        tool: GantryTool::Laser { dynamic: true },
        air_assist: false,
        frame_before_mark: true,
        home_on_connect: false,
        link: GantryLink::ExportOnly,
    });
    m
}

fn gantry_cut(speed: f64, power: f64) -> CutQ {
    CutQ::Gantry(GantryCutQ {
        speed: MmPerS::new(speed).unwrap(),
        power: Percent::new(power).unwrap(),
        s_value: (1000.0 * power / 100.0).round_ties_even() as u32,
        air_assist: false,
        dynamic: true,
    })
}

// ---- geometry ----------------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs I1"]
fn safety_outside_work_area_refused() {
    // B10: the work area is the lens's rated field (±100 mm), not the galvo's reach (±118.34 mm at scale 1.1834).
    let (m, ok) = galvo_job(vec![line(&[[100.0, 0.0], [-100.0, 0.0]]), line(&[[0.0, 100.0], [0.0, -100.0]])]);
    assert_eq!(codes(&validate(&ok, &m)), Vec::<RefusalCode>::new());
    let (m, bad) = galvo_job(vec![line(&[[0.0, 0.0], [100.5, 0.0]])]);
    let r = validate(&bad, &m);
    assert_eq!(codes(&r), vec![RefusalCode::OutsideWorkArea]);
    let refusal = &r.unwrap_err().refusals[0];
    assert_eq!(refusal.at, Some([100.5, 0.0]));
    assert_eq!(refusal.layer, Some(LayerKey::Color(0)));
    // The frame is judged the same way: framing it is refused too.
    assert!(frame_codes(&validate_frame(&bad, &m, FrameSource::Job)).contains(&RefusalCode::OutsideWorkArea));
}

#[test]
#[ignore = "needs I1"]
fn safety_plus_half_field_refused() {
    // A machine whose work area reaches the whole encodable field: +F/2 exactly is refused (OM clamps to 65535),
    // F/2 − 1 µm is accepted.
    let mut m = jpt();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.h27_cleared = Some(openlaser_machine::FieldRect { min: [-120.0, -120.0], max: [120.0, 120.0] });
        g.work_area = openlaser_machine::FieldRect { min: [-118.34, -118.34], max: [118.34, 118.34] };
    }
    let half = galvo(&m).field_f()[0] / 2.0;
    let edge = job(&m, vec![section(0, vec![line(&[[0.0, 0.0], [half, 0.0]])], CutQ::Galvo(default_words(&m)))]);
    assert!(codes(&validate(&edge, &m)).contains(&RefusalCode::NotEncodable));
    let inside = job(&m, vec![section(0, vec![line(&[[0.0, 0.0], [half - 0.001, 0.0]])], CutQ::Galvo(default_words(&m)))]);
    assert_eq!(codes(&validate(&inside, &m)), Vec::<RefusalCode>::new());
}

#[test]
fn safety_non_finite_refused() {
    let (m, p) = galvo_job(vec![line(&[[0.0, 0.0], [f64::NAN, 1.0]]), line(&[[f64::INFINITY, 0.0], [0.0, 0.0]])]);
    let c = codes(&validate(&p, &m));
    assert!(c.contains(&RefusalCode::NonFinite), "{c:?}");
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.non_finite_objects.push(ObjectId(9));
    let r = validate(&p, &m).unwrap_err();
    assert!(r.refusals.iter().any(|x| x.code == RefusalCode::NonFinite && x.object == Some(ObjectId(9))));
}

#[test]
fn safety_malformed_point_unrepresentable() {
    // B9 is refused at import; a plan cannot even hold such a point.
    assert!(serde_json::from_str::<Polyline>("[[1.0, 2.0, 3.0]]").is_err());
    assert!(serde_json::from_str::<Polyline>("[[1.0]]").is_err());
    assert!(serde_json::from_str::<Polyline>("[[1.0, 2.0]]").is_ok());
    assert!(rules().iter().any(|(c, _)| *c == RefusalCode::MalformedPoint));
}

#[test]
fn safety_too_many_paths_refused() {
    let paths: Vec<Polyline> = (0..3001).map(|i| line(&[[0.0, f64::from(i) * 0.01 - 15.0], [1.0, f64::from(i) * 0.01 - 15.0]])).collect();
    let (m, p) = galvo_job(paths);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::TooManyPaths));
}

#[test]
fn safety_too_many_points_per_path_refused() {
    let pts: Vec<[f64; 2]> = (0..2001).map(|i| [f64::from(i) * 0.01 - 10.0, 0.0]).collect();
    let (m, p) = galvo_job(vec![Polyline(pts)]);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::TooManyPointsPerPath));
}

#[test]
fn safety_too_many_points_per_pass_refused() {
    // 11 paths × 1900 points = 20900 > 20000, each path under the per-path limit.
    let paths: Vec<Polyline> = (0..11).map(|j| Polyline((0..1900).map(|i| [f64::from(i) * 0.01 - 9.5, f64::from(j)]).collect())).collect();
    let (m, p) = galvo_job(paths);
    let c = codes(&validate(&p, &m));
    assert!(c.contains(&RefusalCode::TooManyPointsPerPass), "{c:?}");
    assert!(!c.contains(&RefusalCode::TooManyPointsPerPath));
}

#[test]
fn safety_limits_refused_not_truncated() {
    let mk = |n: i32| -> Vec<Polyline> { (0..n).map(|i| line(&[[0.0, f64::from(i) * 0.01 - 15.0], [1.0, f64::from(i) * 0.01 - 15.0]])).collect() };
    let (m, p) = galvo_job(mk(3001));
    let r = validate(&p, &m).unwrap_err();
    let too_many = r.refusals.iter().find(|x| x.code == RefusalCode::TooManyPaths).unwrap();
    assert!(too_many.message.contains("3001") && too_many.message.contains("3000"), "{}", too_many.message);
    // The plan is untouched: nothing was cut down to fit.
    assert_eq!(p.sections[0].paths.len(), 3001);
    // At the limit the rule does not fire.
    let (m, p) = galvo_job(mk(3000));
    assert!(!codes(&validate(&p, &m)).contains(&RefusalCode::TooManyPaths));
}

#[test]
fn safety_out_of_bed_refused() {
    let m = gantry_profile();
    let ok = job(&m, vec![section(0, vec![square(200.0, 200.0, 50.0), line(&[[0.0, 0.0], [400.0, 400.0]])], gantry_cut(50.0, 30.0))]);
    assert_eq!(codes(&validate(&ok, &m)), Vec::<RefusalCode>::new());
    let bad = job(&m, vec![section(0, vec![line(&[[-0.5, 10.0], [10.0, 10.0]])], gantry_cut(50.0, 30.0))]);
    let r = validate(&bad, &m);
    assert_eq!(codes(&r), vec![RefusalCode::OutOfBed]);
    assert_eq!(r.unwrap_err().refusals[0].at, Some([-0.5, 10.0]));
}

// ---- laser parameters ----------------------------------------------------------------------------------------

#[test]
fn safety_power_above_max_refused() {
    let mut m = jpt();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.source.max_power_pct = 80.0;
    }
    let hot = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 85.0, 30, PulseDecision::Exact(200))))]);
    assert!(codes(&validate(&hot, &m)).contains(&RefusalCode::PowerAboveMax));
    let at_max = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 80.0, 30, PulseDecision::Exact(200))))]);
    assert!(!codes(&validate(&at_max, &m)).contains(&RefusalCode::PowerAboveMax));
}

#[test]
fn safety_frequency_out_of_range_refused() {
    let m = jpt();
    let at = |khz: u32| job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, khz, PulseDecision::Exact(200))))]);
    for khz in [4001, 5000] {
        assert!(codes(&validate(&at(khz), &m)).contains(&RefusalCode::FrequencyOutOfRange), "{khz} kHz");
    }
    for khz in [1, 30, 4000] {
        assert!(!codes(&validate(&at(khz), &m)).contains(&RefusalCode::FrequencyOutOfRange), "{khz} kHz");
    }
    // Below 1 kHz: 500 Hz.
    let mut q = default_words(&m);
    q.frequency = Hz::new(500).unwrap();
    q.q_period = 40000;
    let low = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(q))]);
    assert!(codes(&validate(&low, &m)).contains(&RefusalCode::FrequencyOutOfRange));
}

#[test]
fn safety_pulse_not_in_table_refused() {
    let m = jpt();
    let p = job(
        &m,
        vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Refused { requested: 170.0, nearest: 150 })))],
    );
    let r = validate(&p, &m).unwrap_err();
    let x = r.refusals.iter().find(|x| x.code == RefusalCode::PulseNotInTable).unwrap();
    assert!(x.message.contains("170") && x.message.contains("150"), "{}", x.message);
    // An exact word that is not in the table is refused as well (a forged decision).
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Exact(201))))]);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::PulseNotInTable));
}

#[test]
fn safety_nan_pulse_refused() {
    // OM snaps a NaN pulse to 2 ns (a listed divergence): here it is refused.
    let m = jpt();
    let p = job(
        &m,
        vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Refused { requested: f64::NAN, nearest: 2 })))],
    );
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::PulseNotInTable));
}

#[test]
fn safety_pulse_snap_notice() {
    let m = jpt();
    // 199.8 ns snaps to the table's 200 ns with a notice, never a refusal.
    let snapped =
        job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Snapped { requested: 199.8, to: 200 })))]);
    let r = validate(&snapped, &m);
    let c = codes(&r);
    assert!(!c.contains(&RefusalCode::PulseNotInTable), "{c:?}");
    let notices = match &r {
        Ok(v) => v.notices().to_vec(),
        Err(e) => e.notices.clone(),
    };
    assert!(notices.iter().any(|n| n.code == NoticeCode::PulseSnapped && n.message.contains("199.8") && n.layer == Some(LayerKey::Color(0))), "{notices:?}");
    // A snap further than 0.5 ns is a forged decision: refused.
    let far =
        job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Snapped { requested: 199.0, to: 200 })))]);
    assert!(codes(&validate(&far, &m)).contains(&RefusalCode::PulseNotInTable));
    // 170 ns is 20 ns from the table: refused.
    let refused = job(
        &m,
        vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Refused { requested: 170.0, nearest: 150 })))],
    );
    assert!(codes(&validate(&refused, &m)).contains(&RefusalCode::PulseNotInTable));
}

#[test]
fn safety_q_pulse_unsupported_refused() {
    let mut m = jpt();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.source.q_pulse_enabled = false;
    }
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(default_words(&m)))]);
    let c = codes(&validate(&p, &m));
    assert!(c.contains(&RefusalCode::QPulseUnsupported), "{c:?}");
    assert!(!c.contains(&RefusalCode::PulseNotInTable));
}

#[test]
fn safety_speed_zero_units_refused() {
    // B7: below one galvo unit per ms (3.6115 mm/s at F = 236.68) the speed would be sent as MarkSpeed(0).
    let m = jpt();
    let slow = words(&m, 3.0, 30.0, 30, PulseDecision::Exact(200));
    assert_eq!(slow.speed_units, 0);
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(slow))]);
    let r = validate(&p, &m).unwrap_err();
    let x = r.refusals.iter().find(|x| x.code == RefusalCode::SpeedZeroUnits).unwrap();
    assert!(x.message.contains("3.62"), "names the minimum: {}", x.message);
    let ok = words(&m, 3.62, 30.0, 30, PulseDecision::Exact(200));
    assert_eq!(ok.speed_units, 1);
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(ok))]);
    assert!(!codes(&validate(&p, &m)).contains(&RefusalCode::SpeedZeroUnits));
}

#[test]
fn safety_speed_above_max_refused() {
    let m = jpt();
    let fast = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 6000.5, 30.0, 30, PulseDecision::Exact(200))))]);
    assert!(codes(&validate(&fast, &m)).contains(&RefusalCode::SpeedAboveMax));
    let max = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 6000.0, 30.0, 30, PulseDecision::Exact(200))))]);
    assert!(!codes(&validate(&max, &m)).contains(&RefusalCode::SpeedAboveMax));
}

#[test]
fn safety_gantry_overspeed_refused() {
    // OM gcode.py caps the feed at the machine maximum; we refuse instead.
    let m = gantry_profile();
    let fast = job(&m, vec![section(0, vec![square(200.0, 200.0, 10.0)], gantry_cut(120.0, 30.0))]);
    let r = validate(&fast, &m);
    assert_eq!(codes(&r), vec![RefusalCode::SpeedAboveMax]);
    assert!(r.unwrap_err().refusals[0].message.contains("120"));
    let at_max = job(&m, vec![section(0, vec![square(200.0, 200.0, 10.0)], gantry_cut(100.0, 30.0))]);
    let v = validate(&at_max, &m).unwrap();
    assert_eq!(v.emit().sections()[0].cut, gantry_cut(100.0, 30.0));
}

// ---- objects and layers --------------------------------------------------------------------------------------

#[test]
fn safety_orphan_object_refused() {
    // B8: an object on a layer that does not exist is refused, never dropped.
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.orphan_objects = vec![ObjectId(41), ObjectId(42)];
    let r = validate(&p, &m).unwrap_err();
    let objects: Vec<_> = r.refusals.iter().filter(|x| x.code == RefusalCode::OrphanObject).map(|x| x.object).collect();
    assert_eq!(objects, vec![Some(ObjectId(41)), Some(ObjectId(42))]);
}

#[test]
fn safety_hidden_output_refused() {
    // B6: a hidden layer with Output on would burn without ever being seen.
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.hidden_output_layers = vec![LayerKey::Color(3)];
    let r = validate(&p, &m).unwrap_err();
    assert!(r.refusals.iter().any(|x| x.code == RefusalCode::HiddenLayerWithOutput && x.layer == Some(LayerKey::Color(3))));
}

#[test]
fn safety_empty_fill_layer_refused() {
    // OM skips a raster layer that yields no hatch silently; we refuse it.
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.empty_fill_layers = vec![LayerKey::Color(2)];
    let r = validate(&p, &m).unwrap_err();
    assert!(r.refusals.iter().any(|x| x.code == RefusalCode::FillWithoutHatch && x.layer == Some(LayerKey::Color(2))));
}

#[test]
fn safety_unsupported_mode_refused() {
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.unsupported = vec![(LayerKey::Color(1), "C01: Offset Fill is not in this build; use Line or Fill".into())];
    let r = validate(&p, &m).unwrap_err();
    assert!(r.refusals.iter().any(|x| x.code == RefusalCode::UnsupportedMode && x.message.contains("Offset Fill")));
    // An Image section (burning images is a stretch item, not built) is refused too.
    let m = jpt();
    let mut s = section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(default_words(&m)));
    s.mode = SectionMode::Image;
    let p = job(&m, vec![s]);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::UnsupportedMode));
    // Repeats outside 1..=999.
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.repeats = 1000;
    let p = refingerprint(p, &m);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::UnsupportedMode));
}

#[test]
fn safety_nothing_to_mark_refused() {
    let m = jpt();
    let empty = job(&m, Vec::new());
    assert!(codes(&validate(&empty, &m)).contains(&RefusalCode::NothingToMark));
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.repeats = 0;
    let p = refingerprint(p, &m);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::NothingToMark));
}

#[test]
fn safety_wrong_device_kind_refused() {
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.device = DeviceKind::Grbl;
    let p = refingerprint(p, &m);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::WrongDeviceKind));
    // A gantry cut on a galvo machine.
    let m = jpt();
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], gantry_cut(50.0, 30.0))]);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::WrongDeviceKind));
    // The project's device hint.
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.device_mismatch = Some("this project was made for a GRBL gantry".into());
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::WrongDeviceKind));
}

// ---- plan integrity --------------------------------------------------------------------------------------------

#[test]
fn safety_machine_mismatch_refused() {
    let (_, p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    let mut other = jpt();
    other.id = MachineId("jpt-m7-200".into());
    assert!(codes(&validate(&p, &other)).contains(&RefusalCode::MachineMismatch));
    assert!(frame_codes(&validate_frame(&p, &other, FrameSource::Job)).contains(&RefusalCode::MachineMismatch));
}

#[test]
fn safety_stale_refused() {
    // A plan edited after it was fingerprinted cannot reach an encoder.
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.sections[0].paths[0].0[1][0] += 1e-9;
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::Stale));
    assert!(frame_codes(&validate_frame(&p, &m, FrameSource::Job)).contains(&RefusalCode::Stale));
    // The machine's calibration is part of the fingerprint: planning on one calibration, validating on another.
    let (m, p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    let mut recal = m.clone();
    if let Kinematics::Galvo(g) = &mut recal.kind {
        g.cal.scale = [1.2, 1.2];
    }
    assert!(codes(&validate(&p, &recal)).contains(&RefusalCode::Stale));
}

#[test]
#[ignore = "needs I1"]
fn safety_stale_words_refused() {
    // Words that do not match the settings they claim (re-fingerprinted, so only the word check can catch it).
    let m = jpt();
    let mut q = default_words(&m);
    q.mark_current = 4095;
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(q))]);
    assert_eq!(codes(&validate(&p, &m)), vec![RefusalCode::Stale]);
}

#[test]
fn safety_frame_does_not_cover_job_refused() {
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.frame = vec![square(0.0, 0.0, 2.0)];
    let r = validate(&p, &m);
    assert!(codes(&r).contains(&RefusalCode::FrameDoesNotCoverJob));
    assert!(frame_codes(&validate_frame(&p, &m, FrameSource::Job)).contains(&RefusalCode::FrameDoesNotCoverJob));
    // Framing the tool layers alone does not stand for the job, so it is not judged against it.
    p.tool_frame = vec![square(0.0, 0.0, 1.0)];
    assert!(!frame_codes(&validate_frame(&p, &m, FrameSource::ToolLayersOnly)).contains(&RefusalCode::FrameDoesNotCoverJob));
    // No frame at all.
    p.frame.clear();
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::FrameDoesNotCoverJob));
}

#[test]
fn safety_collects_all_refusals() {
    let mut m = jpt();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.source.max_power_pct = 50.0;
    }
    let hot_slow = words(&m, 2.0, 90.0, 4500, PulseDecision::Refused { requested: 170.0, nearest: 150 });
    let mut paths: Vec<Polyline> = (0..3001).map(|i| line(&[[0.0, f64::from(i) * 0.01 - 15.0], [1.0, f64::from(i) * 0.01 - 15.0]])).collect();
    paths.push(line(&[[0.0, 0.0], [f64::NAN, 0.0]]));
    let mut p = job(&m, vec![section(0, paths, CutQ::Galvo(hot_slow))]);
    p.facts.orphan_objects.push(ObjectId(7));
    p.facts.hidden_output_layers.push(LayerKey::Color(4));
    p.facts.empty_fill_layers.push(LayerKey::Color(5));
    p.facts.unsupported.push((LayerKey::Color(6), "C06: wobble is not in this build; turn it off".into()));
    let c = codes(&validate(&p, &m));
    for want in [
        RefusalCode::PowerAboveMax,
        RefusalCode::FrequencyOutOfRange,
        RefusalCode::PulseNotInTable,
        RefusalCode::SpeedZeroUnits,
        RefusalCode::TooManyPaths,
        RefusalCode::NonFinite,
        RefusalCode::OrphanObject,
        RefusalCode::HiddenLayerWithOutput,
        RefusalCode::FillWithoutHatch,
        RefusalCode::UnsupportedMode,
    ] {
        assert!(c.contains(&want), "{want:?} missing from {c:?}");
    }
}

// ---- notices -------------------------------------------------------------------------------------------------

#[test]
fn notice_open_fill_shape_numbered() {
    // Open shapes on a Fill layer have no inside: numbered notices in the confirmation, never silently dropped.
    let m = jpt();
    let mut p =
        job(&m, vec![section(2, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(words(&m, 1000.0, 30.0, 30, PulseDecision::Snapped { requested: 199.8, to: 200 })))]);
    p.facts.open_fill_shapes = vec![(ObjectId(7), LayerKey::Color(2)), (ObjectId(9), LayerKey::Color(2))];
    p.facts.tool_layer_objects = vec![ObjectId(20)];
    let notices = match validate(&p, &m) {
        Ok(v) => v.notices().to_vec(),
        Err(e) => e.notices,
    };
    let numbered: Vec<(u32, NoticeCode, Option<ObjectId>)> = notices.iter().map(|n| (n.n, n.code, n.object)).collect();
    assert_eq!(
        numbered,
        vec![
            (1, NoticeCode::OpenShapeUnhatched, Some(ObjectId(7))),
            (2, NoticeCode::OpenShapeUnhatched, Some(ObjectId(9))),
            (3, NoticeCode::PulseSnapped, None),
            (4, NoticeCode::ToolLayerNotBurned, None),
        ]
    );
    assert!(notices.iter().take(2).all(|n| n.layer == Some(LayerKey::Color(2)) && !n.message.is_empty()));
}

// ---- validate_frame ----------------------------------------------------------------------------------------------

#[test]
fn validate_frame_refuses_hull_and_empty_frames() {
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    p.facts.unsupported.push((LayerKey::Color(0), openlaser_plan::HULL_FRAME_UNSUPPORTED.to_owned()));
    assert!(frame_codes(&validate_frame(&p, &m, FrameSource::Job)).contains(&RefusalCode::UnsupportedMode));
    let (m, p) = galvo_job(vec![square(0.0, 0.0, 5.0)]);
    assert!(frame_codes(&validate_frame(&p, &m, FrameSource::ToolLayersOnly)).contains(&RefusalCode::NothingToMark));
}

#[test]
#[ignore = "needs I1"]
fn validate_frame_traces_a_job_that_cannot_burn() {
    // A power refusal blocks the burn, not the framing (to look at placement).
    let mut m = jpt();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.source.max_power_pct = 10.0;
    }
    let p = job(&m, vec![section(0, vec![square(0.0, 0.0, 5.0)], CutQ::Galvo(default_words(&m)))]);
    assert!(codes(&validate(&p, &m)).contains(&RefusalCode::PowerAboveMax));
    let f = validate_frame(&p, &m, FrameSource::Job).unwrap();
    assert_eq!((f.paths(), f.fingerprint(), f.machine(), f.style(), f.source()), (&p.frame[..], &p.fingerprint, &m.id, FrameStyle::Contour, FrameSource::Job));
    // Tool layers only, and both together.
    let mut p = p;
    p.tool_frame = vec![openlaser_plan::bounds_rect(&openlaser_geom::Bounds { min: [-20.0, -20.0], max: [20.0, 20.0] })];
    let t = validate_frame(&p, &m, FrameSource::ToolLayersOnly).unwrap();
    assert_eq!((t.paths(), t.style(), t.source()), (&p.tool_frame[..], FrameStyle::Bounds, FrameSource::ToolLayersOnly));
    let both = validate_frame(&p, &m, FrameSource::JobAndToolLayers).unwrap();
    assert_eq!(both.paths().len(), p.frame.len() + p.tool_frame.len());
}

#[test]
#[ignore = "needs I1"]
fn validated_carries_the_plan_unchanged() {
    let (m, mut p) = galvo_job(vec![square(0.0, 0.0, 5.0), line(&[[-10.0, -10.0], [10.0, 10.0]])]);
    p.repeats = 3;
    let p = refingerprint(p, &m);
    let v = validate(&p, &m).unwrap();
    let e = v.emit();
    assert_eq!((e.sections(), e.fingerprint(), e.machine(), e.device(), e.repeats()), (&p.sections[..], &p.fingerprint, &p.machine, DeviceKind::GalvoLmc, 3));
    assert_eq!((v.frame().paths(), v.frame().style(), v.frame().source()), (&p.frame[..], FrameStyle::Contour, FrameSource::Job));
    assert!(v.notices().is_empty());
}

// ---- the rule table ----------------------------------------------------------------------------------------------

#[test]
fn rules_table_complete() {
    // Every row of SPEC §3.8's table, plus the plan-integrity rules this crate adds.
    let spec = [
        RefusalCode::OutsideWorkArea,
        RefusalCode::NotEncodable,
        RefusalCode::NonFinite,
        RefusalCode::MalformedPoint,
        RefusalCode::TooManyPaths,
        RefusalCode::TooManyPointsPerPath,
        RefusalCode::TooManyPointsPerPass,
        RefusalCode::PowerAboveMax,
        RefusalCode::FrequencyOutOfRange,
        RefusalCode::PulseNotInTable,
        RefusalCode::QPulseUnsupported,
        RefusalCode::SpeedZeroUnits,
        RefusalCode::SpeedAboveMax,
        RefusalCode::OrphanObject,
        RefusalCode::HiddenLayerWithOutput,
        RefusalCode::FillWithoutHatch,
        RefusalCode::UnsupportedMode,
        RefusalCode::NothingToMark,
        RefusalCode::WrongDeviceKind,
        RefusalCode::OutOfBed,
        RefusalCode::MachineMismatch,
        RefusalCode::Stale,
        RefusalCode::FrameDoesNotCoverJob,
    ];
    let table: Vec<RefusalCode> = rules().iter().map(|(c, _)| *c).collect();
    assert_eq!(table.len(), spec.len(), "{table:?}");
    for c in spec {
        assert_eq!(table.iter().filter(|t| **t == c).count(), 1, "{c:?} must appear exactly once");
    }
    assert!(rules().iter().all(|(_, d)| d.len() > 20));
    assert!(!table.contains(&RefusalCode::NotImplemented));
}

// ---- properties ----------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// SPEC §6.5: every point inside the effective work area encodes, and nothing inside is refused.
    #[test]
    #[ignore = "needs I1"]
    fn prop_points_inside_work_area_encode(x in -100.0f64..=100.0, y in -100.0f64..=100.0, x2 in -100.0f64..=100.0, y2 in -100.0f64..=100.0) {
        let (m, p) = galvo_job(vec![line(&[[x, y], [x2, y2]])]);
        let c = codes(&validate(&p, &m));
        prop_assert!(!c.contains(&RefusalCode::OutsideWorkArea) && !c.contains(&RefusalCode::NotEncodable), "{:?}", c);
        prop_assert!(c.is_empty(), "{:?}", c);
    }

    /// Outside the rated field is always refused, never clamped, whatever else is wrong.
    #[test]
    fn prop_points_outside_work_area_refused(x in 100.001f64..1e6, y in -100.0f64..=100.0, flip in any::<bool>()) {
        let p = if flip { [-x, y] } else { [x, y] };
        let (m, plan) = galvo_job(vec![line(&[[0.0, 0.0], p])]);
        let r = validate(&plan, &m);
        prop_assert!(codes(&r).contains(&RefusalCode::OutsideWorkArea));
        prop_assert!(validate_frame(&plan, &m, FrameSource::Job).is_err());
    }
}

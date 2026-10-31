//! Cross-crate image planning → safety → shipping encoder, with no hardware access.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
#[path = "../../plan/tests/common/image.rs"]
mod fixture;
use fixture::*;
use openlaser_lmc::{emission_summary, encode_job, op};
use openlaser_machine::{Kinematics, jpt_m7_200_sim};
use openlaser_types::{Deg, Mm, RefusalCode, SectionMode};

#[test]
fn image_job_validates_encodes_one_mark_per_run_and_one_power_per_section() {
    let m = jpt_m7_200_sim();
    let mut p = project(&m);
    layer(&mut p).image.passes = 2;
    let plan = plan(&p, &m);
    let validated = openlaser_safety::validate(&plan, &m).unwrap();
    let Kinematics::Galvo(g) = &m.kind else { panic!("galvo") };
    let job = encode_job(validated.emit(), g).unwrap();
    assert_eq!(job.lists.len(), 2);
    for (list, section) in job.lists.iter().zip(&plan.sections) {
        let words: Vec<_> = list.packets.iter().flat_map(|p| p.words().into_iter().take(p.real_commands())).collect();
        assert_eq!(words.iter().filter(|w| w[0] == op::LIST_MARK_TO).count(), section.paths.len());
        assert_eq!(words.iter().filter(|w| w[0] == op::LIST_MARK_CURRENT).count(), 1);
        let first_mark = words.iter().position(|w| w[0] == op::LIST_MARK_TO).unwrap();
        assert!(
            words[first_mark..].iter().all(|w| ![
                op::LIST_MARK_CURRENT,
                op::LIST_MARK_POWER_RATIO,
                op::LIST_SET_WELD_POWER_WAVE,
                op::LIST_ENABLE_WELD_POWER_WAVE
            ]
            .contains(&w[0]))
        );
    }
    let summary = emission_summary(&job, g, &plan, &[]).unwrap();
    assert!(summary.rows.iter().all(|r| r.mode == SectionMode::Image));
}

#[test]
fn image_unsupported_facts_cannot_pass_safety() {
    let m = jpt_m7_200_sim();
    let mut cases = Vec::new();
    let mut p = project(&m);
    layer(&mut p).image.angle = Deg::new(1.0).unwrap();
    cases.push(p);
    let mut p = project(&m);
    layer(&mut p).image.interval = Mm::new(0.001).unwrap();
    cases.push(p);
    let mut p = project(&m);
    bitmap(&mut p).size_mm = [100.0, 100.0];
    layer(&mut p).image.interval = Mm::new(0.01).unwrap();
    cases.push(p);
    let mut p = project(&m);
    std::sync::Arc::make_mut(&mut p.shapes[0]).power_scale = openlaser_types::Percent::new(50.0).unwrap();
    cases.push(p);
    for p in cases {
        let r = openlaser_safety::validate(&plan(&p, &m), &m).unwrap_err();
        assert!(r.refusals.iter().any(|r| r.code == RefusalCode::UnsupportedMode));
    }
}

//! Machine-space slab clipping. Targets are always rounded from absolute strip centres.
use crate::{CutQ, MAX_SECTION_POINTS, MAX_SECTIONS, RotaryStep, Section};
use openlaser_geom::Polyline;
use openlaser_machine::{GalvoMachine, RotaryCfg};

/// Clip each segment to a slab, retaining connected runs and their original direction.
/// Parallel segments on an upper boundary belong to the next strip (except the final edge).
fn clip(paths: &[Polyline], r: &RotaryCfg, lo: f64, hi: f64, centre: f64, last: bool) -> Vec<Polyline> {
    let axis = r.axis;
    let mut out = Vec::new();
    for path in paths {
        let mut run: Vec<[f64; 2]> = Vec::new();
        for &[a, b] in path.0.array_windows::<2>() {
            let delta = axis.coordinate(b) - axis.coordinate(a);
            let segment = if delta == 0.0 {
                (axis.coordinate(a) >= lo && (axis.coordinate(a) < hi || (last && axis.coordinate(a) == hi))).then_some((a, b))
            } else {
                let t0 = ((lo - axis.coordinate(a)) / delta).min((hi - axis.coordinate(a)) / delta).max(0.0);
                let t1 = ((lo - axis.coordinate(a)) / delta).max((hi - axis.coordinate(a)) / delta).min(1.0);
                (t1 > t0).then(|| {
                    let at = |t: f64| [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                    (at(t0), at(t1))
                })
            };
            if let Some((mut a, mut b)) = segment {
                axis.translate(&mut a, -centre);
                axis.translate(&mut b, -centre);
                if run.last() != Some(&a) {
                    if run.len() >= 2 {
                        out.push(Polyline(std::mem::take(&mut run)));
                    }
                    run.clear();
                    run.push(a);
                }
                run.push(b);
            } else if run.len() >= 2 {
                out.push(Polyline(std::mem::take(&mut run)));
            }
        }
        if run.len() >= 2 {
            out.push(Polyline(run));
        }
    }
    out
}

/// Closed outlines acquire the strip-edge connectors, so a centred contour frame
/// encloses its clipped fill/image region. Open line outlines remain open.
fn clip_frame(paths: &[Polyline], r: &RotaryCfg, lo: f64, hi: f64, centre: f64, last: bool) -> Vec<Polyline> {
    let mut out = Vec::new();
    for path in paths {
        if path.0.len() < 4 || path.0.first() != path.0.last() {
            out.extend(clip(std::slice::from_ref(path), r, lo, hi, centre, last));
            continue;
        }
        let mut polygon = path.0.clone();
        polygon.pop();
        for (edge, lower) in [(lo, true), (hi, false)] {
            let mut clipped = Vec::new();
            let Some(mut previous) = polygon.last().copied() else { break };
            for current in polygon {
                let a = r.axis.coordinate(previous);
                let b = r.axis.coordinate(current);
                let inside = |v: f64| if lower { v >= edge } else { v <= edge };
                if inside(a) != inside(b) {
                    let t = (edge - a) / (b - a);
                    let mut crossing = [previous[0] + t * (current[0] - previous[0]), previous[1] + t * (current[1] - previous[1])];
                    let correction = edge - r.axis.coordinate(crossing);
                    r.axis.translate(&mut crossing, correction);
                    clipped.push(crossing);
                }
                if inside(b) {
                    clipped.push(current);
                }
                previous = current;
            }
            polygon = clipped;
        }
        if polygon.len() >= 3 {
            for point in &mut polygon {
                r.axis.translate(point, -centre);
            }
            if let Some(first) = polygon.first().copied() {
                polygon.push(first);
            }
            out.push(Polyline(polygon));
        }
    }
    out
}

pub(crate) fn strips(sections: &[Section], outlines: &[Polyline], m: &GalvoMachine, repeats: u16) -> Result<(Vec<Section>, Vec<Polyline>), String> {
    let Some(r) = m.rotary.as_ref().filter(|r| r.enabled) else {
        return Ok((sections.to_vec(), outlines.to_vec()));
    };
    r.validate(m.work_area)?;
    let axis = r.axis;
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for p in outlines.iter().chain(sections.iter().flat_map(|s| &s.paths)).flat_map(|p| &p.0) {
        if !p.iter().all(|v| v.is_finite()) {
            return Err("rotary geometry must be finite".into());
        }
        lo = lo.min(axis.coordinate(*p));
        hi = hi.max(axis.coordinate(*p));
    }
    if !lo.is_finite() {
        return Ok((Vec::new(), Vec::new()));
    }
    let count = ((hi - lo) / r.split_mm).ceil().max(1.0);
    if count > MAX_SECTIONS as f64 {
        return Err("rotary job exceeds the strip budget; increase split size".into());
    }
    // The range is checked above before conversion; bounded by MAX_SECTIONS.
    let count = count as u32;
    let input_points = sections.iter().flat_map(|s| &s.paths).chain(outlines).fold(0usize, |n, p| n.saturating_add(p.0.len()));
    if input_points.saturating_mul(count as usize) > MAX_SECTION_POINTS.saturating_mul(4) {
        return Err("rotary clipping exceeds the geometry work budget; simplify paths or increase split size".into());
    }
    let s_ref = (lo + hi) / 2.0;
    let mut result = Vec::new();
    let mut frame = Vec::new();
    let mut points = 0usize;
    let mut targets = Vec::new();
    for strip in 0..count {
        let start = lo + f64::from(strip) * r.split_mm;
        let centre = start + r.split_mm / 2.0;
        let lower = start - r.overlap_mm;
        let upper = start + r.split_mm + r.overlap_mm;
        let area = m.effective_work_area();
        if -r.split_mm / 2.0 - r.overlap_mm < axis.coordinate(area.min) || r.split_mm / 2.0 + r.overlap_mm > axis.coordinate(area.max) {
            return Err("rotary strip including overlap exceeds the work area at the centre line; reduce split/overlap".into());
        }
        let target = ((centre - s_ref) / r.circumference() * f64::from(r.pulses_per_rev)).round() * if r.reverse { -1.0 } else { 1.0 };
        if target.abs() > r.max_turns_per_job * f64::from(r.pulses_per_rev) || target.abs() > f64::from(i32::MAX) {
            return Err("rotary target exceeds max turns per job".into());
        }
        let target_pulses = target as i32;
        let before = result.len();
        for s in sections {
            let paths = clip(&s.paths, r, lower, upper, centre, strip + 1 == count);
            if paths.is_empty() {
                continue;
            }
            points = points.saturating_add(paths.iter().map(|p| p.0.len()).sum::<usize>());
            if result.len() >= MAX_SECTIONS || points > MAX_SECTION_POINTS {
                return Err("rotary job exceeds the section/point budget".into());
            }
            let mut s = s.clone();
            s.paths = paths;
            s.rotary = Some(RotaryStep { strip, target_pulses });
            if let CutQ::Galvo(q) = &s.cut {
                s.estimate = crate::estimate::estimate_galvo(&s.paths, q, m);
            }
            result.push(s);
        }
        if result.len() > before {
            targets.push(target_pulses);
        }
        // Trace clipped outlines at the centre line. Bounds style is applied by the caller.
        frame.extend(clip_frame(outlines, r, lower, upper, centre, strip + 1 == count));
    }
    let mut position = 0i64;
    let mut travel = 0u64;
    for _ in 0..repeats {
        for &target in &targets {
            travel = travel.saturating_add((i64::from(target) - position).unsigned_abs());
            position = i64::from(target);
        }
    }
    if r.return_to_start {
        travel = travel.saturating_add(position.unsigned_abs());
    }
    if travel as f64 > r.max_turns_per_job * f64::from(r.pulses_per_rev) {
        return Err("total rotary travel, including initial positioning, repeats and return, exceeds max turns per job".into());
    }
    Ok((result, frame))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use openlaser_machine::{Kinematics, RotaryAxis, RotaryKind, jpt_m7_200_sim};
    use openlaser_types::{Hz, LayerKey, MmPerS, Percent, SectionMode};
    fn machine() -> GalvoMachine {
        let Kinematics::Galvo(mut m) = jpt_m7_200_sim().kind else { panic!("galvo") };
        m.rotary = Some(RotaryCfg { enabled: true, ..RotaryCfg::default() });
        m
    }
    fn section(m: &GalvoMachine, points: Vec<[f64; 2]>) -> Section {
        Section {
            rotary: None,
            layer: LayerKey::Color(0),
            layer_name: "C00".into(),
            sub: 0,
            pass: 1,
            of: 1,
            mode: SectionMode::Line,
            cut: CutQ::Galvo(
                crate::quantise::quantise_galvo(
                    MmPerS::new(1000.0).unwrap(),
                    Percent::new(1.0).unwrap(),
                    Hz::new(30000).unwrap(),
                    Some(openlaser_types::Ns::new(200.0).unwrap()),
                    m,
                )
                .unwrap(),
            ),
            paths: vec![Polyline(points)],
            objects: vec![],
            estimate: std::time::Duration::ZERO,
        }
    }
    fn run(m: &GalvoMachine, sections: &[Section]) -> Vec<Section> {
        strips(sections, &sections.iter().flat_map(|s| s.paths.clone()).collect::<Vec<_>>(), m, 1).unwrap().0
    }
    fn length(s: &[Section]) -> f64 {
        s.iter().flat_map(|s| &s.paths).flat_map(|p| p.0.array_windows::<2>()).map(|[a, b]| (a[0] - b[0]).hypot(a[1] - b[1])).sum()
    }
    #[test]
    fn thirty_mm_line_clips_into_three_exact_centred_strips() {
        let m = machine();
        let sections = run(&m, &[section(&m, vec![[0.0, 0.0], [0.0, 30.0]])]);
        assert_eq!(sections.len(), 3);
        for (i, s) in sections.iter().enumerate() {
            assert_eq!(s.paths, vec![Polyline(vec![[0.0, -5.0], [0.0, 5.0]])]);
            assert_eq!(s.rotary.unwrap().strip, i as u32);
        }
        assert_eq!(sections.iter().map(|s| s.rotary.unwrap().target_pulses).collect::<Vec<_>>(), vec![-1019, 0, 1019]);
        assert_eq!(length(&sections), 30.0);
    }
    #[test]
    fn overlap_duplicates_only_the_requested_boundary_intervals() {
        let mut m = machine();
        m.rotary.as_mut().unwrap().overlap_mm = 1.0;
        let s = run(&m, &[section(&m, vec![[0.0, 0.0], [0.0, 30.0]])]);
        assert_eq!(s[0].paths[0].0, vec![[0.0, -5.0], [0.0, 6.0]]);
        assert_eq!(s[1].paths[0].0, vec![[0.0, -6.0], [0.0, 6.0]]);
        assert_eq!(length(&s), 34.0);
    }
    #[test]
    fn diagonal_segments_are_split_and_parallel_boundary_has_one_owner() {
        let m = machine();
        let s = section(&m, vec![[0.0, 0.0], [30.0, 30.0]]);
        let out = run(&m, &[s]);
        assert!((length(&out) - 30.0_f64.hypot(30.0)).abs() < 1e-12);
        assert_eq!(out[1].paths[0].0, vec![[10.0, -5.0], [20.0, 5.0]]);
        let boundary = section(&m, vec![[1.0, 10.0], [2.0, 10.0]]);
        let outer = section(&m, vec![[0.0, 0.0], [0.0, 30.0]]);
        let out = run(&m, &[outer, boundary]);
        assert_eq!(out.len(), 4);
        assert_eq!(out[2].paths[0].0, vec![[1.0, -5.0], [2.0, -5.0]]);
    }
    #[test]
    fn roller_uses_drive_diameter_and_reverse_negates_absolute_targets() {
        let mut m = machine();
        let base = section(&m, vec![[0.0, 0.0], [0.0, 30.0]]);
        let chuck = run(&m, std::slice::from_ref(&base));
        let r = m.rotary.as_mut().unwrap();
        r.kind = RotaryKind::Roller;
        r.object_diameter_mm = 100.0;
        r.reverse = true;
        let roller = run(&m, std::slice::from_ref(&base));
        for (a, b) in chuck.iter().zip(&roller) {
            assert_eq!(a.rotary.unwrap().target_pulses, -b.rotary.unwrap().target_pulses);
        }
        m.rotary.as_mut().unwrap().drive_diameter_mm = 20.0;
        let out = run(&m, &[base]);
        assert_eq!(out[0].rotary.unwrap().target_pulses, 2037);
    }
    #[test]
    fn thousand_strips_round_from_absolute_centres_and_keep_layer_order() {
        let mut m = machine();
        let r = m.rotary.as_mut().unwrap();
        r.split_mm = 0.1;
        r.max_turns_per_job = 20.0;
        let a = section(&m, vec![[0.0, 0.0], [0.0, 100.0]]);
        let mut b = a.clone();
        b.layer = LayerKey::Color(2);
        let out = run(&m, &[a, b]);
        assert_eq!(out.len(), 2000);
        for (k, pair) in out.chunks(2).enumerate() {
            let expected = (((k as f64 + 0.5) * 0.1 - 50.0) / (40.0 * std::f64::consts::PI) * 12800.0).round() as i32;
            assert_eq!(pair[0].rotary.unwrap().target_pulses, expected);
            assert_eq!(pair[1].rotary, pair[0].rotary);
            assert_eq!(pair[0].layer, LayerKey::Color(0));
            assert_eq!(pair[1].layer, LayerKey::Color(2));
        }
    }
    #[test]
    fn x_wrap_centres_x_and_excess_travel_and_width_are_refused() {
        let mut m = machine();
        m.rotary.as_mut().unwrap().axis = RotaryAxis::X;
        let a = section(&m, vec![[0.0, 0.0], [30.0, 0.0]]);
        let out = run(&m, std::slice::from_ref(&a));
        assert_eq!(out[0].paths[0].0, vec![[-5.0, 0.0], [5.0, 0.0]]);
        m.rotary.as_mut().unwrap().max_turns_per_job = 0.01;
        assert!(strips(std::slice::from_ref(&a), &a.paths, &m, 1).is_err());
        let r = m.rotary.as_mut().unwrap();
        r.max_turns_per_job = 2.0;
        r.split_mm = 200.0;
        r.overlap_mm = 1.0;
        assert!(strips(std::slice::from_ref(&a), &a.paths, &m, 1).is_err());
    }
}

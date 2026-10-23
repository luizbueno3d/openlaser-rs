//! The plan fingerprint (S21): the emission identity of a job.
//!
//! blake3 over a canonical, length-prefixed little-endian encoding of: a domain tag; the machine id, its kinematics
//! settings that shape bytes (galvo: encoding field, calibration, red-dot calibration, timings, first jump, jump
//! and light speeds, MO settle, ports; gantry: bed, origin, travel speed, S max, tool, air assist); the device kind;
//! every section (layer, sub-layer, pass, of, mode, every field of its quantised cut, every path point's f64 bit
//! pattern); and the repeat count.
//!
//! Deliberately NOT in it: the frame, the tool frame, the frame style and source (framing choices: the receipt
//! records them and `covers_job` decides), layer names, object ids and estimates (they do not change a byte).
//! `safety` recomputes it and refuses a plan whose fingerprint does not match its contents (`Stale`).

use openlaser_doc::{DeviceKind, WorkspaceOrigin};
use openlaser_machine::{GantryTool, Kinematics, MachineProfile};
use openlaser_types::galvo::PulseDecision;
use openlaser_types::{Fingerprint, LayerKey, SectionMode};

use crate::{CutQ, GalvoCutQ, GantryCutQ, ResolvedTimings, Section};

const DOMAIN: &[u8] = b"openlaser/plan-fingerprint/v1";

struct Canon(blake3::Hasher);

impl Canon {
    fn new() -> Self {
        let mut h = blake3::Hasher::new();
        h.update(DOMAIN);
        Canon(h)
    }

    fn u8(&mut self, v: u8) {
        self.0.update(&[v]);
    }

    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    fn u16(&mut self, v: u16) {
        self.0.update(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.update(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.0.update(&v.to_le_bytes());
    }

    fn len(&mut self, n: usize) {
        // usize → u64 never loses bits on the 64-bit (and 32-bit) targets this builds for.
        self.u64(u64::try_from(n).unwrap_or(u64::MAX));
    }

    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.0.update(s.as_bytes());
    }

    fn pair(&mut self, w: (u16, u16)) {
        self.u16(w.0);
        self.u16(w.1);
    }

    fn finish(self) -> Fingerprint {
        Fingerprint(*self.0.finalize().as_bytes())
    }
}

fn origin_tag(o: WorkspaceOrigin) -> u8 {
    match o {
        WorkspaceOrigin::Center => 0,
        WorkspaceOrigin::BottomLeft => 1,
        WorkspaceOrigin::TopLeft => 2,
    }
}

fn machine(c: &mut Canon, m: &MachineProfile) {
    c.str(&m.id.0);
    match &m.kind {
        Kinematics::Galvo(g) => {
            c.u8(1);
            c.f64(g.encoding_field.get());
            for v in g.cal.scale.iter().chain(&g.cal.offset_mm) {
                c.f64(*v);
            }
            c.f64(g.cal.angle_deg);
            c.bool(g.cal.swap_xy);
            c.bool(g.cal.negate[0]);
            c.bool(g.cal.negate[1]);
            for v in g.red_dot.offset_mm.iter().chain(&g.red_dot.scale) {
                c.f64(*v);
            }
            let t = &g.timings;
            for v in [t.laser_on_us, t.laser_off_us, t.polygon_us, t.jump_delay_short_us, t.jump_delay_long_us] {
                c.f64(v);
            }
            c.f64(g.first_jump.delay_us);
            c.u16(g.first_jump.settle_10us);
            c.f64(g.jump_speed.get());
            c.f64(g.light_speed.get());
            c.f64(g.mo_settle.get());
            c.u8(g.ports.laser_bit);
            c.u8(g.ports.red_bit);
        }
        Kinematics::Gantry(g) => {
            c.u8(2);
            c.f64(g.bed_mm[0]);
            c.f64(g.bed_mm[1]);
            c.u8(origin_tag(g.origin));
            c.f64(g.travel_speed.get());
            c.u32(g.s_max);
            match g.tool {
                GantryTool::Laser { dynamic } => {
                    c.u8(1);
                    c.bool(dynamic);
                }
                GantryTool::Pen => c.u8(2),
            }
            c.bool(g.air_assist);
        }
    }
}

fn layer(c: &mut Canon, k: LayerKey) {
    match k {
        LayerKey::Color(i) => {
            c.u8(1);
            c.u8(i);
        }
        LayerKey::Tool(t) => {
            c.u8(2);
            c.u8(t);
        }
    }
}

fn timings(c: &mut Canon, t: &ResolvedTimings) {
    for w in [t.laser_on, t.laser_off, t.polygon, t.jump_delay_short, t.jump_delay_long] {
        c.pair(w);
    }
    c.u16(t.jump_speed_units);
    c.u16(t.mo_settle_10us);
    c.u16(t.first_jump_settle_10us);
}

fn pulse(c: &mut Canon, d: &PulseDecision) {
    match *d {
        PulseDecision::Exact(n) => {
            c.u8(1);
            c.u16(n);
        }
        PulseDecision::Snapped { requested, to } => {
            c.u8(2);
            c.f64(requested);
            c.u16(to);
        }
        PulseDecision::Refused { requested, nearest } => {
            c.u8(3);
            c.f64(requested);
            c.u16(nearest);
        }
    }
}

fn galvo_cut(c: &mut Canon, q: &GalvoCutQ) {
    c.u8(1);
    c.f64(q.requested_speed.get());
    c.u16(q.speed_units);
    c.f64(q.actual_speed_mm_s);
    c.f64(q.power.get());
    c.u16(q.mark_current);
    c.u32(q.frequency.get());
    c.u16(q.q_period);
    pulse(c, &q.pulse);
    timings(c, &q.timings);
}

fn gantry_cut(c: &mut Canon, q: &GantryCutQ) {
    c.u8(2);
    c.f64(q.speed.get());
    c.f64(q.power.get());
    c.u32(q.s_value);
    c.bool(q.air_assist);
    c.bool(q.dynamic);
}

fn section(c: &mut Canon, s: &Section) {
    layer(c, s.layer);
    c.u8(s.sub);
    c.u16(s.pass);
    c.u16(s.of);
    c.u8(match s.mode {
        SectionMode::Line => 1,
        SectionMode::Fill => 2,
        SectionMode::Image => 3,
    });
    match &s.cut {
        CutQ::Galvo(q) => galvo_cut(c, q),
        CutQ::Gantry(q) => gantry_cut(c, q),
    }
    c.len(s.paths.len());
    for p in &s.paths {
        c.len(p.0.len());
        for pt in &p.0 {
            c.f64(pt[0]);
            c.f64(pt[1]);
        }
    }
}

/// The canonical fingerprint of a plan's emission identity (S21). `plan()` stores it in `JobPlan.fingerprint`;
/// `safety` recomputes it from the plan it is handed.
pub fn fingerprint_of(m: &MachineProfile, device: DeviceKind, sections: &[Section], repeats: u16) -> Fingerprint {
    let mut c = Canon::new();
    machine(&mut c, m);
    c.u8(match device {
        DeviceKind::GalvoLmc => 1,
        DeviceKind::Grbl => 2,
    });
    c.len(sections.len());
    for s in sections {
        section(&mut c, s);
    }
    c.u16(repeats);
    c.finish()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use openlaser_geom::Polyline;
    use openlaser_machine::{Kinematics, jpt_m7_200_sim};
    use openlaser_types::{Hz, MmPerS, ObjectId, Percent};

    use super::*;

    fn cut(power: f64) -> CutQ {
        let zero = (100, 0);
        CutQ::Galvo(GalvoCutQ {
            requested_speed: MmPerS::new(1000.0).unwrap(),
            speed_units: 276,
            actual_speed_mm_s: 996.8,
            power: Percent::new(power).unwrap(),
            mark_current: 1228,
            frequency: Hz::new(30_000).unwrap(),
            q_period: 667,
            pulse: PulseDecision::Exact(200),
            timings: ResolvedTimings {
                laser_on: zero,
                laser_off: zero,
                polygon: zero,
                jump_delay_short: (8, 0),
                jump_delay_long: (200, 0),
                jump_speed_units: 830,
                mo_settle_10us: 800,
                first_jump_settle_10us: 100,
            },
        })
    }

    fn section(power: f64, x: f64) -> Section {
        Section {
            layer: LayerKey::Color(0),
            layer_name: "C00".into(),
            sub: 0,
            pass: 1,
            of: 1,
            mode: SectionMode::Line,
            cut: cut(power),
            paths: vec![Polyline(vec![[0.0, 0.0], [x, 0.0]])],
            objects: vec![ObjectId(1)],
            estimate: Duration::from_millis(5),
        }
    }

    fn fp(sections: &[Section]) -> Fingerprint {
        fingerprint_of(&jpt_m7_200_sim(), DeviceKind::GalvoLmc, sections, 1)
    }

    #[test]
    fn fingerprint_changes_with_power_paths_repeats_and_calibration() {
        let base = fp(&[section(30.0, 10.0)]);
        assert_eq!(base, fp(&[section(30.0, 10.0)]), "deterministic");
        assert_ne!(base, fp(&[section(31.0, 10.0)]), "power");
        assert_ne!(base, fp(&[section(30.0, 10.0 + 1e-12)]), "path bits");
        assert_ne!(base, fingerprint_of(&jpt_m7_200_sim(), DeviceKind::GalvoLmc, &[section(30.0, 10.0)], 2), "repeats");
        assert_ne!(base, fp(&[section(30.0, 10.0), section(30.0, 10.0)]), "a second pass");
        let mut recal = jpt_m7_200_sim();
        if let Kinematics::Galvo(g) = &mut recal.kind {
            g.cal.scale = [1.2, 1.2];
        }
        assert_ne!(base, fingerprint_of(&recal, DeviceKind::GalvoLmc, &[section(30.0, 10.0)], 1), "calibration");
        let mut red = jpt_m7_200_sim();
        if let Kinematics::Galvo(g) = &mut red.kind {
            g.red_dot.offset_mm = [0.1, 0.0];
        }
        assert_ne!(base, fingerprint_of(&red, DeviceKind::GalvoLmc, &[section(30.0, 10.0)], 1), "red-dot calibration");
        assert_ne!(base, fingerprint_of(&jpt_m7_200_sim(), DeviceKind::Grbl, &[section(30.0, 10.0)], 1), "device");
    }

    #[test]
    fn fingerprint_ignores_names_objects_and_estimates() {
        let base = fp(&[section(30.0, 10.0)]);
        let mut s = section(30.0, 10.0);
        s.layer_name = "Engrave".into();
        s.objects = vec![ObjectId(99)];
        s.estimate = Duration::from_secs(9);
        assert_eq!(base, fp(&[s]));
        // Profile fields that never reach the bytes: name, checklist, limits.
        let mut m = jpt_m7_200_sim();
        m.name = "renamed".into();
        m.checklist.push("one more".into());
        m.limits.max_paths = 10;
        assert_eq!(base, fingerprint_of(&m, DeviceKind::GalvoLmc, &[section(30.0, 10.0)], 1));
    }
}

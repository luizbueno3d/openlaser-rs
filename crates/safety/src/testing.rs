//! Test-only constructors (feature `unchecked-for-tests`; dev-dependency only; `compile_error!` without
//! debug_assertions). WP0's file (SPEC §7.12), implemented in full (§3.0) so WP2, WP4 and WP5 can build `EmitPlan`s
//! and `FramePlan`s before WP3's `validate` lands. Referenced only from `tests/` and `#[cfg(test)]` (§2.3 rule 14).

use crate::{DeviceKind, EmitPlan, Fingerprint, FramePlan, FrameSource, FrameStyle, MachineId, Polyline, Section};

pub fn emit_plan_unchecked(sections: Vec<Section>, fp: Fingerprint, machine: MachineId, device: DeviceKind, repeats: u16) -> EmitPlan {
    EmitPlan { sections, fingerprint: fp, machine, device, repeats }
}

pub fn frame_plan_unchecked(paths: Vec<Polyline>, fp: Fingerprint, machine: MachineId, style: FrameStyle, source: FrameSource) -> FramePlan {
    FramePlan { paths, fingerprint: fp, machine, style, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchecked_plans_keep_their_fields() {
        let e = emit_plan_unchecked(Vec::new(), Fingerprint([3; 32]), MachineId("m".into()), DeviceKind::GalvoLmc, 2);
        assert_eq!((e.fingerprint(), e.machine(), e.device(), e.repeats()), (&Fingerprint([3; 32]), &MachineId("m".into()), DeviceKind::GalvoLmc, 2));
        let f = frame_plan_unchecked(Vec::new(), Fingerprint([4; 32]), MachineId("m".into()), FrameStyle::Bounds, FrameSource::ToolLayersOnly);
        assert_eq!((f.style(), f.source()), (FrameStyle::Bounds, FrameSource::ToolLayersOnly));
    }
}

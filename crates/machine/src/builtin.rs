//! Built-in profiles.

use openlaser_doc::{FillParams, GalvoParams, GantryParams, LayerDefaults};
use openlaser_types::galvo::JPT_M7_PULSES_NS;
use openlaser_types::{Deg, Hz, MachineId, Mm, MmPerS, Ns, Percent, Us};

use crate::profile::{
    FieldCalibration, FieldRect, FirstJump, GalvoLink, GalvoMachine, GalvoTimingDefaults, GantryLink, GantryMachine, GantryTool, JobLimits, Kinematics,
    LaserSource, LmcBoardFilter, MachineProfile, PortMap, RedDotCal, RunAhead, SourceKind,
};

/// A unit literal checked at compile time: an out-of-range literal fails the build (the `None` arm is dead).
macro_rules! lit {
    ($t:ident, $v:expr) => {{
        const _: () = assert!($t::new_const($v).is_some(), "built-in profile literal out of range");
        const CHECKED: $t = match $t::new_const($v) {
            Some(v) => v,
            None => $t::MAX_VALID,
        };
        CHECKED
    }};
}

/// id "jpt-m7-200": OM profile + calibration 1.1834, link Usb.
pub fn jpt_m7_200() -> MachineProfile {
    jpt_m7(MachineId("jpt-m7-200".to_owned()), "JPT M7 · USB", GalvoLink::Usb)
}

/// id "jpt-m7-200-sim": identical, link Sim (default in non-hw builds).
pub fn jpt_m7_200_sim() -> MachineProfile {
    jpt_m7(MachineId("jpt-m7-200-sim".to_owned()), "JPT M7 · Simulator", GalvoLink::Sim)
}

/// co2-grbl-600x400, diode-grbl-400x400, plotter-grbl-a4 (OM machines.py, T2).
pub fn om_gantries() -> Vec<MachineProfile> {
    vec![
        gantry(
            "co2-grbl-600x400",
            "CO₂ cutter · GRBL 600 × 400 mm",
            [600.0, 400.0],
            lit!(MmPerS, 500.0),
            GantryTool::Laser { dynamic: false },
            true,
            lit!(MmPerS, 100.0),
            lit!(Percent, 20.0),
        ),
        gantry(
            "diode-grbl-400x400",
            "Diode laser · GRBL 400 × 400 mm",
            [400.0, 400.0],
            lit!(MmPerS, 250.0),
            GantryTool::Laser { dynamic: true },
            true,
            lit!(MmPerS, 50.0),
            lit!(Percent, 20.0),
        ),
        gantry(
            "plotter-grbl-a4",
            "Pen plotter · GRBL A4",
            [210.0, 297.0],
            lit!(MmPerS, 150.0),
            GantryTool::Pen,
            false,
            lit!(MmPerS, 50.0),
            lit!(Percent, 100.0),
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn gantry(
    id: &str,
    name: &str,
    bed_mm: [f64; 2],
    max_speed: MmPerS,
    tool: GantryTool,
    air_assist: bool,
    default_speed: MmPerS,
    default_power: Percent,
) -> MachineProfile {
    let dynamic = matches!(tool, GantryTool::Laser { dynamic: true });
    MachineProfile {
        id: MachineId(id.to_owned()),
        name: name.to_owned(),
        kind: Kinematics::Gantry(GantryMachine {
            bed_mm,
            origin: openlaser_doc::WorkspaceOrigin::BottomLeft,
            max_speed,
            travel_speed: max_speed,
            s_max: 1000,
            tool,
            air_assist,
            frame_before_mark: false,
            home_on_connect: false,
            link: GantryLink::ExportOnly,
        }),
        limits: JobLimits { max_paths: 3000, max_points_per_path: 2000, max_points_per_pass: 20000 },
        layer_defaults: LayerDefaults {
            galvo: GalvoParams {
                speed: lit!(MmPerS, 1000.0),
                power: lit!(Percent, 30.0),
                min_power: lit!(Percent, 0.0),
                frequency: lit!(Hz, 30_000),
                q_pulse: Some(lit!(Ns, 200.0)),
                timings: None,
                wobble: None,
            },
            gantry: GantryParams { speed: default_speed, power_max: default_power, power_min: lit!(Percent, 0.0), constant_power: !dynamic },
            fill: FillParams { interval: lit!(Mm, 0.1), angle: lit!(Deg, 0.0), angle_increment: lit!(Deg, 90.0), bidirectional: true, cross_hatch: false },
        },
        checklist: vec!["Machine area clear and material secured".to_owned()],
    }
}

fn jpt_m7(id: MachineId, name: &str, link: GalvoLink) -> MachineProfile {
    let galvo = GalvoMachine {
        rotary: None,
        board: LmcBoardFilter { vid: 0x9588, pid: 0x9899, serial: None },
        link,
        encoding_field: lit!(Mm, 200.0),
        work_area: FieldRect { min: [-100.0, -100.0], max: [100.0, 100.0] },
        h27_cleared: None,
        cal: FieldCalibration { scale: [1.1834, 1.1834], offset_mm: [0.0, 0.0], angle_deg: 0.0, swap_xy: false, negate: [false, false] },
        red_dot: RedDotCal { offset_mm: [0.0, 0.0], scale: [1.0, 1.0] },
        source: LaserSource {
            kind: SourceKind::MopaFiberJpt,
            pulse_table_ns: JPT_M7_PULSES_NS.to_vec(),
            freq_khz: [1.0, 4000.0],
            max_power_pct: 100.0,
            q_pulse_enabled: true,
            mo_only_when_running: true,
        },
        timings: GalvoTimingDefaults { laser_on_us: 100.0, laser_off_us: 100.0, polygon_us: 100.0, jump_delay_short_us: 8.0, jump_delay_long_us: 200.0 },
        first_jump: FirstJump { delay_us: 200.0, settle_10us: 100 },
        jump_speed: lit!(MmPerS, 3000.0),
        light_speed: lit!(MmPerS, 3000.0),
        max_speed: lit!(MmPerS, 6000.0),
        mo_settle: lit!(Us, 8000.0),
        ports: PortMap { laser_bit: 0, red_bit: 8, foot_switch: None },
        run_ahead: RunAhead { max_packets: 3, max_emission_s: 2.0, watchdog_factor: 1.5, watchdog_slack_s: 2.0 },
        transfer_timeout_ms: 250,
        usb_set_configuration: false,
    };
    let layer_defaults = LayerDefaults {
        galvo: GalvoParams {
            speed: lit!(MmPerS, 1000.0),
            power: lit!(Percent, 30.0),
            min_power: lit!(Percent, 0.0),
            frequency: lit!(Hz, 30_000),
            q_pulse: Some(lit!(Ns, 200.0)),
            timings: None,
            wobble: None,
        },
        gantry: GantryParams { speed: lit!(MmPerS, 100.0), power_max: lit!(Percent, 30.0), power_min: lit!(Percent, 0.0), constant_power: false },
        fill: FillParams { interval: lit!(Mm, 0.1), angle: lit!(Deg, 0.0), angle_increment: lit!(Deg, 90.0), bidirectional: true, cross_hatch: false },
    };
    MachineProfile {
        id,
        name: name.to_owned(),
        kind: Kinematics::Galvo(galvo),
        limits: JobLimits { max_paths: 3000, max_points_per_path: 2000, max_points_per_pass: 20000 },
        layer_defaults,
        checklist: vec![
            "Key switch and E-stop within reach".to_owned(),
            "Laser safety glasses on for everyone in the room".to_owned(),
            "Material in place and focus checked".to_owned(),
            "Fume extraction on".to_owned(),
        ],
    }
}

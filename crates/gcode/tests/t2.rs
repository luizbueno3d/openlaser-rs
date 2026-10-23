//! T2 (SPEC §6.2): G-code parity with OM `gcode.compile_gcode`. 100,019 number rows exact (and their inputs
//! proven parsed bit for bit); 44 cases: programs byte-exact after masking line 3, refusals with OM's exception
//! type and message. The three hatched scenarios wait for WP1's `hatch_om` (T3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use openlaser_gcode::number;
use openlaser_gcode::om::{OmLayer, OmMachine, OmObject, compile_gcode_om};
use openlaser_testkit::{sha256_hex, t2};

/// Scenarios whose programs need `openlaser_geom::hatch_om` (a raster layer with closed shapes).
const NEEDS_HATCH: [&str; 3] = ["raster_crosshatch", "mixed_layers", "business_card"];

pub fn om_machine(row: &t2::Machine) -> OmMachine {
    OmMachine {
        machine_id: row.machine_id.clone(),
        name: row.name.clone(),
        kind: row.kind.clone(),
        transport: row.transport.clone(),
        width_mm: row.width_mm,
        height_mm: row.height_mm,
        origin: row.origin.clone(),
        tool: row.tool.clone(),
        output_format: row.output_format.clone(),
        max_speed_mm_s: row.max_speed_mm_s,
        travel_speed_mm_s: row.travel_speed_mm_s,
        power_scale: row.power_scale,
        laser_command: row.laser_command.clone(),
        tool_on_command: row.tool_on_command.clone(),
        tool_off_command: row.tool_off_command.clone(),
        supports_air_assist: row.supports_air_assist,
        uses_frequency: row.uses_frequency,
        uses_pulse_width: row.uses_pulse_width,
        direct_control: row.direct_control,
        notes: row.notes.clone(),
        is_laser: row.is_laser,
        is_plotter: row.is_plotter,
    }
}

fn om_layer(l: &t2::Layer) -> OmLayer {
    OmLayer {
        layer_id: l.layer_id.clone(),
        name: l.name.clone(),
        operation: l.operation.clone(),
        power_percent: l.power_percent,
        frequency_khz: l.frequency_khz,
        pulse_width_ns: l.pulse_width_ns,
        speed_mm_s: l.speed_mm_s,
        passes: l.passes,
        visible: l.visible,
        output: l.output,
        color: l.color.clone(),
        air_assist: l.air_assist,
        raster_pitch_mm: l.raster_pitch_mm,
        raster_angle_deg: l.raster_angle_deg,
        raster_angle_step_deg: l.raster_angle_step_deg,
    }
}

/// A scenario's layers and objects; the business card's come from `inputs/` (`inputs_ref`).
fn scenario_inputs(name: &str, scenario: &t2::Scenario) -> (Vec<OmLayer>, Vec<OmObject>) {
    if name != "business_card" {
        let layers = scenario.layers.iter().map(om_layer).collect();
        let objects = scenario.objects.iter().map(|o| OmObject { layer_id: o.layer_id.clone(), polylines: o.polylines.clone() }).collect();
        return (layers, objects);
    }
    let read = |file: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(openlaser_testkit::golden_dir().join("inputs").join(file)).unwrap()).unwrap()
    };
    let layers: Vec<t2::Layer> = serde_json::from_value(read("layer_settings.json")["layers"].clone()).unwrap();
    let workspace = read("workspace.json");
    let objects = workspace["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            assert_eq!((o["x"].as_f64(), o["y"].as_f64(), o["scale"].as_f64(), o["rotation"].as_f64()), (Some(0.0), Some(0.0), Some(1.0), Some(0.0)));
            OmObject { layer_id: o["layer_id"].as_str().unwrap().to_owned(), polylines: serde_json::from_value(o["polylines"].clone()).unwrap() }
        })
        .collect();
    (layers.iter().map(om_layer).collect(), objects)
}

#[test]
fn t2_number_format() {
    let table = t2::load_numbers().expect("T2 number fixture");
    assert_eq!(table.n_rows, 100_019);
    assert_eq!(table.rows.len(), 100_019);
    let mut bits = Vec::with_capacity(table.rows.len() * 8);
    for (input, _) in &table.rows {
        bits.extend_from_slice(&input.to_bits().to_le_bytes());
    }
    assert_eq!(sha256_hex(&bits), table.inputs_f64_sha256, "every float input parsed exactly");
    for (input, expected) in &table.rows {
        assert_eq!(number(*input), *expected, "input {input:?}");
    }
}

#[test]
fn t2() {
    let index = t2::load().expect("T2 index");
    assert_eq!(index.cases.len(), 44);
    let (mut programs, mut refusals, mut deferred) = (0usize, 0usize, 0usize);
    for case in &index.cases {
        let row = index.machines.iter().find(|m| m.machine_id == case.machine_id).expect("fixture machine");
        let scenario = index.scenarios.get(&case.scenario).expect("fixture scenario");
        let label = format!("{} / {}", case.machine_id, case.scenario);
        if case.ok && NEEDS_HATCH.contains(&case.scenario.as_str()) {
            deferred += 1;
            continue;
        }
        let (layers, objects) = scenario_inputs(&case.scenario, scenario);
        let result = compile_gcode_om(&om_machine(row), &layers, &objects, "<MASKED>");
        if case.ok {
            let text = result.unwrap_or_else(|e| panic!("{label}: {e:?}"));
            let expected = t2::program(case.file.as_deref().expect("program file")).expect("program bytes");
            assert_eq!(text.as_bytes(), expected.as_slice(), "{label}");
            assert_eq!(text.lines().nth(2), Some("; Generated: <MASKED>"), "{label}: line 3 is the masked timestamp");
            assert_eq!(Some(sha256_hex(text.as_bytes())), case.sha256, "{label}");
            programs += 1;
        } else {
            let refusal = result.expect_err(&label);
            let expected = case.error.as_ref().expect("refusal row");
            assert_eq!((refusal.kind.as_str(), refusal.message.as_str()), (expected.kind.as_str(), expected.message.as_str()), "{label}");
            refusals += 1;
        }
    }
    assert_eq!((programs, refusals, deferred), (15, 20, 9), "T2 coverage");
}

/// The nine hatched programs (raster_crosshatch, mixed_layers, business_card on the three gantries).
#[test]
#[ignore = "needs WP1's openlaser_geom::hatch_om (T3); un-ignore at I1"]
fn t2_hatched_cases() {
    let index = t2::load().expect("T2 index");
    let mut checked = 0;
    for case in index.cases.iter().filter(|c| c.ok && NEEDS_HATCH.contains(&c.scenario.as_str())) {
        let row = index.machines.iter().find(|m| m.machine_id == case.machine_id).unwrap();
        let (layers, objects) = scenario_inputs(&case.scenario, index.scenarios.get(&case.scenario).unwrap());
        let text = compile_gcode_om(&om_machine(row), &layers, &objects, "<MASKED>").unwrap();
        assert_eq!(text.as_bytes(), t2::program(case.file.as_deref().unwrap()).unwrap().as_slice(), "{} / {}", case.machine_id, case.scenario);
        assert_eq!(Some(sha256_hex(text.as_bytes())), case.sha256);
        checked += 1;
    }
    assert_eq!(checked, 9);
}

//! T2 parity port of OM `gcode.compile_gcode` (MIT source: OpenMopa `mopa_luiz/gcode.py`, commit `6aed8ff`).

/// A machine row of `index.json` `machines`.
#[derive(Debug, Clone, PartialEq)]
pub struct OmMachine {
    pub machine_id: String,
    pub name: String,
    pub kind: String,
    pub transport: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub origin: String,
    pub tool: String,
    pub output_format: String,
    pub max_speed_mm_s: f64,
    pub travel_speed_mm_s: f64,
    pub power_scale: u32,
    pub laser_command: String,
    pub tool_on_command: String,
    pub tool_off_command: String,
    pub supports_air_assist: bool,
    pub uses_frequency: bool,
    pub uses_pulse_width: bool,
    pub direct_control: bool,
    pub notes: String,
    pub is_laser: bool,
    pub is_plotter: bool,
}

/// A layer of a scenario (OM `layer_settings.json` fields).
#[derive(Debug, Clone, PartialEq)]
pub struct OmLayer {
    pub layer_id: String,
    pub name: String,
    pub operation: String,
    pub power_percent: f64,
    pub frequency_khz: f64,
    pub pulse_width_ns: f64,
    pub speed_mm_s: f64,
    pub passes: u32,
    pub visible: bool,
    pub output: bool,
    pub color: String,
    pub air_assist: bool,
    pub raster_pitch_mm: f64,
    pub raster_angle_deg: f64,
    pub raster_angle_step_deg: f64,
}

/// An object of a scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct OmObject {
    pub layer_id: String,
    pub polylines: Vec<Vec<[f64; 2]>>,
}

/// OM's refusal (`index.json` case `error`: a Python exception type and message).
#[derive(Debug, Clone, PartialEq)]
pub struct OmRefusal {
    pub kind: String,
    pub message: String,
}

pub fn compile_gcode_om(machine: &OmMachine, layers: &[OmLayer], objects: &[OmObject], generated: &str) -> Result<String, OmRefusal> {
    if machine.output_format != "gcode" {
        return Err(refuse("ValueError", format!("{} does not use G-code output", machine.name)));
    }
    let mut lines = vec![
        "; OpenLaser G-code".to_owned(),
        format!("; Machine: {}", machine.name),
        format!("; Generated: {generated}"),
        "; Design origin: center; units: millimetres".to_owned(),
        "G21".to_owned(),
        "G90".to_owned(),
        machine.tool_off_command.clone(),
    ];
    let mut path_count = 0usize;
    for layer in layers {
        if !layer.output || matches!(layer.operation.as_str(), "disabled" | "frame_only") {
            continue;
        }
        let raster = layer.operation == "raster_engrave";
        let mut raw_paths: Vec<&Vec<[f64; 2]>> = Vec::new();
        for object in objects.iter().filter(|object| object.layer_id == layer.layer_id) {
            raw_paths.extend(object.polylines.iter());
        }
        if raw_paths.is_empty() {
            continue;
        }
        let speed = layer.speed_mm_s.min(machine.max_speed_mm_s);
        if !speed.is_finite() || speed <= 0.0 {
            return Err(refuse("ValueError", format!("layer {:?} speed must be greater than zero", layer.name)));
        }
        if !layer.power_percent.is_finite() || !(0.0..=100.0).contains(&layer.power_percent) {
            return Err(refuse("ValueError", format!("layer {:?} power must be between 0 and 100 percent", layer.name)));
        }
        let scaled = (f64::from(machine.power_scale) * layer.power_percent / 100.0).round_ties_even();
        if !scaled.is_finite() || scaled < 0.0 || scaled > f64::from(u32::MAX) {
            return Err(refuse("ValueError", "scaled power is outside u32 range"));
        }
        let power = format!("{scaled:.0}");
        let passes = if raster { 1 } else { layer.passes.max(1) };
        let mut paths: Vec<Vec<[f64; 2]>> = Vec::new();
        if raster {
            let polygons = raw_paths.iter().map(|raw| openlaser_geom::Polyline((*raw).clone())).collect::<Vec<_>>();
            let hatch = openlaser_geom::hatch_om(
                &polygons,
                layer.raster_angle_deg,
                layer.raster_pitch_mm,
                layer.passes.max(1),
                layer.raster_angle_step_deg,
                openlaser_geom::CLOSURE_TOL_MM,
            )
            .map_err(|e| refuse("ValueError", e.to_string()))?;
            paths.extend(hatch.segments.into_iter().map(|segment| vec![segment[0], segment[1]]));
        } else {
            for raw in raw_paths {
                if raw.len() >= 2 {
                    paths.push(raw.clone());
                }
            }
        }
        if paths.is_empty() {
            continue;
        }
        lines.push(format!("; Layer: {} · {}", layer.name, layer.operation));
        let air = machine.is_laser && layer.air_assist && machine.supports_air_assist;
        if air {
            lines.push("M8".to_owned());
        }
        for pass in 0..passes {
            if passes > 1 {
                lines.push(format!("; Pass {}/{}", pass + 1, passes));
            }
            for path in &paths {
                let transformed = path.iter().map(|&p| machine_xy(machine, p)).collect::<Result<Vec<_>, _>>()?;
                let Some(start) = transformed.first() else {
                    continue;
                };
                lines.push(machine.tool_off_command.clone());
                lines.push(format!("G0 X{} Y{}", crate::number(start[0]), crate::number(start[1])));
                if machine.is_plotter {
                    lines.push(machine.tool_on_command.clone());
                } else {
                    lines.push(format!("{} S{}", machine.laser_command, power));
                }
                for p in transformed.iter().skip(1) {
                    lines.push(format!("G1 X{} Y{} F{}", crate::number(p[0]), crate::number(p[1]), crate::number(speed * 60.0)));
                }
                lines.push(machine.tool_off_command.clone());
                path_count = path_count.saturating_add(1);
            }
        }
        if air {
            lines.push("M9".to_owned());
        }
    }
    if path_count == 0 {
        return Err(refuse("ValueError", "no output-enabled geometry for G-code export"));
    }
    lines.extend([machine.tool_off_command.clone(), "M9".to_owned(), "G0 X0 Y0".to_owned(), "; End OpenLaser job".to_owned(), String::new()]);
    Ok(lines.join("\n"))
}

fn refuse(kind: &str, message: impl Into<String>) -> OmRefusal {
    OmRefusal { kind: kind.to_owned(), message: message.into() }
}

fn machine_xy(machine: &OmMachine, point: [f64; 2]) -> Result<[f64; 2], OmRefusal> {
    let [x, y] = point;
    if !x.is_finite() || !y.is_finite() {
        return Err(refuse("ValueError", "job contains a non-finite coordinate"));
    }
    let (mx, my, minx, maxx, miny, maxy) = match machine.origin.as_str() {
        "center" => (x, y, -machine.width_mm / 2.0, machine.width_mm / 2.0, -machine.height_mm / 2.0, machine.height_mm / 2.0),
        "bottom_left" => (x + machine.width_mm / 2.0, y + machine.height_mm / 2.0, 0.0, machine.width_mm, 0.0, machine.height_mm),
        "top_left" => (x + machine.width_mm / 2.0, machine.height_mm / 2.0 - y, 0.0, machine.width_mm, 0.0, machine.height_mm),
        other => return Err(refuse("ValueError", format!("unsupported machine origin: {other}"))),
    };
    let tol = 1e-6;
    if mx < minx - tol || mx > maxx + tol || my < miny - tol || my > maxy + tol {
        return Err(refuse(
            "ValueError",
            format!("geometry point ({x:.3}, {y:.3}) mm is outside the {} × {} mm work area", machine.width_mm, machine.height_mm),
        ));
    }
    Ok([mx, my])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t2_vector_and_multipass_bytes_match() {
        let index = openlaser_testkit::t2::load().expect("T2 index");
        let mut checked = 0usize;
        for case in index
            .cases
            .iter()
            .filter(|c| c.ok && matches!(c.scenario.as_str(), "vector" | "multipass" | "air_assist" | "plotter_drawing" | "speed_power_edges"))
        {
            let Some(machine) = index.machines.iter().find(|m| m.machine_id == case.machine_id) else { continue };
            let Some(scenario) = index.scenarios.get(&case.scenario) else { continue };
            let machine = OmMachine {
                machine_id: machine.machine_id.clone(),
                name: machine.name.clone(),
                kind: machine.kind.clone(),
                transport: machine.transport.clone(),
                width_mm: machine.width_mm,
                height_mm: machine.height_mm,
                origin: machine.origin.clone(),
                tool: machine.tool.clone(),
                output_format: machine.output_format.clone(),
                max_speed_mm_s: machine.max_speed_mm_s,
                travel_speed_mm_s: machine.travel_speed_mm_s,
                power_scale: machine.power_scale,
                laser_command: machine.laser_command.clone(),
                tool_on_command: machine.tool_on_command.clone(),
                tool_off_command: machine.tool_off_command.clone(),
                supports_air_assist: machine.supports_air_assist,
                uses_frequency: machine.uses_frequency,
                uses_pulse_width: machine.uses_pulse_width,
                direct_control: machine.direct_control,
                notes: machine.notes.clone(),
                is_laser: machine.is_laser,
                is_plotter: machine.is_plotter,
            };
            let layers: Vec<OmLayer> = scenario
                .layers
                .iter()
                .map(|l| OmLayer {
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
                })
                .collect();
            let objects: Vec<OmObject> = scenario.objects.iter().map(|o| OmObject { layer_id: o.layer_id.clone(), polylines: o.polylines.clone() }).collect();
            if machine.output_format != "gcode" {
                assert!(compile_gcode_om(&machine, &layers, &objects, "<MASKED>").is_err());
                continue;
            }
            let actual = compile_gcode_om(&machine, &layers, &objects, "<MASKED>").expect("vector G-code compiles");
            let expected = openlaser_testkit::t2::program(case.file.as_deref().expect("program filename")).expect("golden program");
            assert_eq!(actual.as_bytes(), expected.as_slice(), "{} / {}", case.machine_id, case.scenario);
            checked += 1;
        }
        assert!(checked >= 10, "only {checked} T2 programs were checked");
    }

    #[test]
    #[ignore = "needs I1 hatch parity for raster scenarios"]
    fn t2_all_44_cases_match_when_hatch_parity_lands() {
        let index = openlaser_testkit::t2::load().expect("T2 index");
        assert_eq!(index.cases.len(), 44);
        for case in &index.cases {
            let machine_row = index.machines.iter().find(|m| m.machine_id == case.machine_id).expect("fixture machine");
            let machine = OmMachine {
                machine_id: machine_row.machine_id.clone(),
                name: machine_row.name.clone(),
                kind: machine_row.kind.clone(),
                transport: machine_row.transport.clone(),
                width_mm: machine_row.width_mm,
                height_mm: machine_row.height_mm,
                origin: machine_row.origin.clone(),
                tool: machine_row.tool.clone(),
                output_format: machine_row.output_format.clone(),
                max_speed_mm_s: machine_row.max_speed_mm_s,
                travel_speed_mm_s: machine_row.travel_speed_mm_s,
                power_scale: machine_row.power_scale,
                laser_command: machine_row.laser_command.clone(),
                tool_on_command: machine_row.tool_on_command.clone(),
                tool_off_command: machine_row.tool_off_command.clone(),
                supports_air_assist: machine_row.supports_air_assist,
                uses_frequency: machine_row.uses_frequency,
                uses_pulse_width: machine_row.uses_pulse_width,
                direct_control: machine_row.direct_control,
                notes: machine_row.notes.clone(),
                is_laser: machine_row.is_laser,
                is_plotter: machine_row.is_plotter,
            };
            let scenario = index.scenarios.get(&case.scenario).expect("fixture scenario");
            let layers: Vec<OmLayer> = scenario
                .layers
                .iter()
                .map(|l| OmLayer {
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
                })
                .collect();
            let objects: Vec<OmObject> = scenario.objects.iter().map(|o| OmObject { layer_id: o.layer_id.clone(), polylines: o.polylines.clone() }).collect();
            let actual = compile_gcode_om(&machine, &layers, &objects, "<MASKED>");
            if case.ok {
                let actual = actual.expect("successful golden scenario");
                let expected = openlaser_testkit::t2::program(case.file.as_deref().expect("program file")).expect("program bytes");
                assert_eq!(actual.as_bytes(), expected.as_slice(), "{} / {}", case.machine_id, case.scenario);
                if let Some(expected_hash) = &case.sha256 {
                    assert_eq!(openlaser_testkit::sha256_hex(actual.as_bytes()), *expected_hash);
                }
            } else {
                assert!(actual.is_err(), "{} / {} should refuse", case.machine_id, case.scenario);
            }
        }
    }
}

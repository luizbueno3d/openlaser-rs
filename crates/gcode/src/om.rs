//! T2 parity port of OM `gcode.compile_gcode` over `layers.group_by_layer` (py-compat: OM's bytes, including its
//! silent speed capping; the shipping encoder is [`crate::compile`]).
//!
//! Ported from OpenMopa `mopa_luiz/gcode.py` (`compile_gcode`, `_paths_for_job`, `_xy`, `_number`) and
//! `mopa_luiz/layers.py` (`group_by_layer`) at commit `6aed8ffa8be5a938d195bf35198cfd168bb07f4e`.
//! MIT © 2026 Luiz Bueno (see ATTRIBUTION.md). Refusals carry OM's exception type and message.
//!
//! The only difference from Python: a non-finite speed or coordinate refuses here, where Python would print `nan`.

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
    for (layer, polylines) in group_by_layer(objects, layers) {
        if !layer.output || matches!(layer.operation.as_str(), "disabled" | "frame_only") {
            continue;
        }
        // `_paths_for_job`: raster layers hatch their (already >= 2-point) polylines; others pass them through.
        let raster = layer.operation == "raster_engrave";
        let paths: Vec<Vec<[f64; 2]>> = if raster {
            let polygons = polylines.iter().map(|raw| openlaser_geom::Polyline(raw.clone())).collect::<Vec<_>>();
            let hatch = openlaser_geom::hatch_om(
                &polygons,
                layer.raster_angle_deg,
                layer.raster_pitch_mm,
                layer.passes.max(1),
                layer.raster_angle_step_deg,
                openlaser_geom::CLOSURE_TOL_MM,
            )
            .map_err(|e| refuse("ValueError", e.to_string()))?;
            hatch.segments.into_iter().map(|segment| vec![segment[0], segment[1]]).collect()
        } else {
            polylines
        };
        if paths.is_empty() {
            continue;
        }
        let passes = if raster { 1 } else { layer.passes.max(1) };
        // Python `min(speed, max)`: a NaN speed stays NaN (and is refused below), never becomes the maximum.
        let speed = if machine.max_speed_mm_s < layer.speed_mm_s { machine.max_speed_mm_s } else { layer.speed_mm_s };
        if !speed.is_finite() || speed <= 0.0 {
            return Err(refuse("ValueError", format!("layer {} speed must be greater than zero", py_repr(&layer.name))));
        }
        if !layer.power_percent.is_finite() || !(0.0..=100.0).contains(&layer.power_percent) {
            return Err(refuse("ValueError", format!("layer {} power must be between 0 and 100 percent", py_repr(&layer.name))));
        }
        let scaled = (f64::from(machine.power_scale) * layer.power_percent / 100.0).round_ties_even();
        if !scaled.is_finite() || scaled < 0.0 || scaled > f64::from(u32::MAX) {
            return Err(refuse("ValueError", "scaled power is outside u32 range"));
        }
        let power = format!("{scaled:.0}");
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
                if path.len() < 2 {
                    continue;
                }
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

/// OM `_xy`: centre-origin design mm → machine coordinates; outside the work area (1e-6 tolerance) refuses.
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

/// OM `layers.group_by_layer`: one job per distinct `layer_id` in first-seen order, carrying the last spec with
/// that id (Python's `dict` semantics); objects on unknown layers and polylines of fewer than 2 points are dropped
/// (OM's behaviour; the shipping planner refuses orphans, B8).
fn group_by_layer<'a>(objects: &[OmObject], layers: &'a [OmLayer]) -> Vec<(&'a OmLayer, Vec<Vec<[f64; 2]>>)> {
    let mut jobs: Vec<(&'a OmLayer, Vec<Vec<[f64; 2]>>)> = Vec::new();
    for layer in layers {
        match jobs.iter_mut().find(|(existing, _)| existing.layer_id == layer.layer_id) {
            Some(job) => job.0 = layer,
            None => jobs.push((layer, Vec::new())),
        }
    }
    for object in objects {
        let Some(job) = jobs.iter_mut().find(|(layer, _)| layer.layer_id == object.layer_id) else { continue };
        job.1.extend(object.polylines.iter().filter(|polyline| polyline.len() >= 2).cloned());
    }
    jobs
}

/// Python `repr(str)`: single quotes unless the text holds a single quote and no double quote; backslash, the
/// quote and control characters escaped.
pub fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if u32::from(c) < 0x20 || (0x7f..0xa0).contains(&u32::from(c)) => out.push_str(&format!("\\x{:02x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_repr_matches_python() {
        assert_eq!(py_repr("Vec"), "'Vec'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(py_repr("x\ny\\"), "'x\\ny\\\\'");
        assert_eq!(py_repr("\u{1}·"), "'\\x01·'");
    }
}

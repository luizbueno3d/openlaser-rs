//! DXF compatibility importer.
//!
//! Ported from OpenMopa `mopa_luiz/importers.py` at commit
//! `6aed8ffa8be5a938d195bf35198cfd168bb07f4e` (MIT). The operation order is
//! intentionally retained because T4 compares coordinates to 1e-9 mm.

use std::f64::consts::{PI, TAU};

use crate::{Error, Polyline};

type Pair = (String, String);
type Vertex = (f64, f64, f64);

pub fn parse_dxf_om(bytes: &[u8]) -> Result<Vec<Polyline>, Error> {
    parse_dxf_with_tolerance(bytes, None)
}

/// Shipping DXF parser. Compatibility rows intentionally keep SPLINE control/fit points;
/// the user-facing importer evaluates them into a deterministic polyline.
pub(crate) fn parse_dxf_shipping(bytes: &[u8], tolerance_mm: f64) -> Result<Vec<Polyline>, Error> {
    parse_dxf_with_tolerance(bytes, Some(tolerance_mm))
}

fn parse_dxf_with_tolerance(bytes: &[u8], spline_tolerance: Option<f64>) -> Result<Vec<Polyline>, Error> {
    let text = String::from_utf8_lossy(bytes);
    let pairs = dxf_pairs(&text);
    let mut polylines = Vec::new();
    let mut i = 0usize;
    while let Some((code, value)) = pairs.get(i) {
        if code == "0" && matches!(value.as_str(), "LINE" | "LWPOLYLINE" | "POLYLINE" | "CIRCLE" | "ARC" | "ELLIPSE" | "SPLINE") {
            let entity = value.clone();
            i += 1;
            let mut fields = Vec::new();
            while let Some((next_code, _)) = pairs.get(i) {
                if next_code == "0" {
                    break;
                }
                if let Some(pair) = pairs.get(i) {
                    fields.push(pair.clone());
                }
                i += 1;
            }
            let polyline = match entity.as_str() {
                "LINE" => {
                    let values = dxf_values(&fields)?;
                    Some(Polyline(vec![[value_or(&values, "10"), value_or(&values, "20")], [value_or(&values, "11"), value_or(&values, "21")]]))
                }
                "LWPOLYLINE" => Some(parse_lwpolyline(&fields)?),
                "POLYLINE" => {
                    let (points, next) = parse_old_polyline(&pairs, i, &fields)?;
                    i = next;
                    Some(points)
                }
                "CIRCLE" => {
                    let values = dxf_values(&fields)?;
                    Some(circle_points(value_or(&values, "10"), value_or(&values, "20"), value_or(&values, "40")))
                }
                "ARC" => {
                    let values = dxf_values(&fields)?;
                    Some(arc_points(
                        value_or(&values, "10"),
                        value_or(&values, "20"),
                        value_or(&values, "40"),
                        value_or(&values, "50"),
                        value_or(&values, "51"),
                    ))
                }
                "ELLIPSE" => Some(ellipse_points(&fields)?),
                "SPLINE" => Some(match spline_tolerance {
                    Some(tolerance) => evaluate_spline(&fields, tolerance)?,
                    None => {
                        let fit = repeated_xy(&fields, "11", "21")?;
                        let points = if fit.is_empty() { repeated_xy(&fields, "10", "20")? } else { fit };
                        if points.len() < 2 { Polyline(Vec::new()) } else { points }
                    }
                }),
                _ => None,
            };
            if let Some(polyline) = polyline.filter(|line| line.len() >= 2) {
                polylines.push(polyline);
            }
            continue;
        }
        i += 1;
    }
    Ok(polylines)
}

fn evaluate_spline(fields: &[Pair], tolerance: f64) -> Result<Polyline, Error> {
    let fit = repeated_xy(fields, "11", "21")?.0;
    let controls = repeated_xy(fields, "10", "20")?.0;
    let is_fit_spline = fit.len() >= 2;
    let points = if is_fit_spline { fit } else { controls };
    if points.len() < 2 {
        return Ok(Polyline(Vec::new()));
    }
    if (!is_fit_spline && points.len() > 128) || points.len() > 100_000 {
        return Err(Error::TooLarge(format!("DXF spline has {} fit/control points; limit is {}", points.len(), if is_fit_spline { 100_000 } else { 128 })));
    }
    // Many CAD writers omit the knot vector for a fit-point spline. Interpolate those
    // points with a Catmull-Rom curve. Control-point entities without a full knot vector
    // are represented as a clamped Bezier curve when the degree permits it.
    let degree = fields.iter().find(|(code, _)| code == "71").and_then(|(_, value)| value.parse::<usize>().ok());
    let knots: Vec<f64> = fields.iter().filter(|(code, _)| code == "40").map(|(_, value)| parse_f64(value)).collect::<Result<_, _>>()?;
    let weights: Vec<f64> = fields.iter().filter(|(code, _)| code == "41").map(|(_, value)| parse_f64(value)).collect::<Result<_, _>>()?;
    if !weights.is_empty() && weights.len() != points.len() {
        return Err(Error::Malformed { format: "DXF", message: "spline weight count does not match its points".to_owned() });
    }
    if is_fit_spline {
        return Ok(Polyline(interpolate_fit_points(&points, tolerance)?));
    }
    if let Some(degree) = degree
        && degree > 0
        && degree < points.len()
        && knots.len() == points.len() + degree + 1
    {
        return Ok(Polyline(sample_bspline(&points, &knots, &weights, degree, tolerance)?));
    }
    if !knots.is_empty() || degree.is_some_and(|degree| degree != points.len().saturating_sub(1)) {
        return Err(Error::Malformed { format: "DXF", message: "incomplete or inconsistent spline knot vector".to_owned() });
    }
    Ok(Polyline(sample_bezier(&points, &weights, tolerance)?))
}

fn sample_bezier(points: &[[f64; 2]], weights: &[f64], tolerance: f64) -> Result<Vec<[f64; 2]>, Error> {
    if weights.iter().any(|weight| !weight.is_finite() || *weight <= 0.0) {
        return Err(Error::Malformed { format: "DXF", message: "spline weights must be finite and positive".to_owned() });
    }
    let control_length = points.windows(2).try_fold(0.0, |length, pair| {
        let start = *pair.first().ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid Bezier spline controls".to_owned() })?;
        let end = *pair.get(1).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid Bezier spline controls".to_owned() })?;
        let dx = end.first().copied().unwrap_or(0.0) - start.first().copied().unwrap_or(0.0);
        let dy = end.get(1).copied().unwrap_or(0.0) - start.get(1).copied().unwrap_or(0.0);
        Ok::<f64, Error>(length + dx.hypot(dy))
    })?;
    let segments = (control_length / tolerance.sqrt().max(0.01)).ceil().clamp(16.0, 4096.0) as usize;
    (0..=segments)
        .map(|i| {
            let t = i as f64 / segments as f64;
            let mut work: Vec<[f64; 3]> = points
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    let weight = weights.get(index).copied().unwrap_or(1.0);
                    [point.first().copied().unwrap_or(0.0) * weight, point.get(1).copied().unwrap_or(0.0) * weight, weight]
                })
                .collect();
            for level in (1..work.len()).rev() {
                for index in 0..level {
                    let left = *work.get(index).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid Bezier spline controls".to_owned() })?;
                    let right = *work.get(index + 1).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid Bezier spline controls".to_owned() })?;
                    let current =
                        work.get_mut(index).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid Bezier spline controls".to_owned() })?;
                    *current = [
                        left.first().copied().unwrap_or(0.0) * (1.0 - t) + right.first().copied().unwrap_or(0.0) * t,
                        left.get(1).copied().unwrap_or(0.0) * (1.0 - t) + right.get(1).copied().unwrap_or(0.0) * t,
                        left.get(2).copied().unwrap_or(1.0) * (1.0 - t) + right.get(2).copied().unwrap_or(1.0) * t,
                    ];
                }
            }
            let point = work.first().copied().ok_or_else(|| Error::Malformed { format: "DXF", message: "spline has no control points".to_owned() })?;
            let weight = point.get(2).copied().unwrap_or(0.0);
            if weight <= 0.0 || !weight.is_finite() {
                return Err(Error::Malformed { format: "DXF", message: "invalid spline weight".to_owned() });
            }
            Ok([point.first().copied().unwrap_or(0.0) / weight, point.get(1).copied().unwrap_or(0.0) / weight])
        })
        .collect()
}

fn interpolate_fit_points(points: &[[f64; 2]], tolerance: f64) -> Result<Vec<[f64; 2]>, Error> {
    let mut result = Vec::new();
    let subdivisions = 16usize.max((1.0 / tolerance.sqrt()).ceil() as usize).min(128);
    for i in 0..points.len().saturating_sub(1) {
        let p1 = *points.get(i).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline fit points".to_owned() })?;
        let p2 = *points.get(i + 1).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline fit points".to_owned() })?;
        let p0 =
            if i == 0 { p1 } else { *points.get(i - 1).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline fit points".to_owned() })? };
        let p3 = points.get(i + 2).copied().unwrap_or(p2);
        for j in 0..subdivisions {
            let t = j as f64 / subdivisions as f64;
            let t2 = t * t;
            let t3 = t2 * t;
            result.push([
                0.5 * ((2.0 * p1[0])
                    + (-p0[0] + p2[0]) * t
                    + (2.0 * p0[0] - 5.0 * p1[0] + 4.0 * p2[0] - p3[0]) * t2
                    + (-p0[0] + 3.0 * p1[0] - 3.0 * p2[0] + p3[0]) * t3),
                0.5 * ((2.0 * p1[1])
                    + (-p0[1] + p2[1]) * t
                    + (2.0 * p0[1] - 5.0 * p1[1] + 4.0 * p2[1] - p3[1]) * t2
                    + (-p0[1] + 3.0 * p1[1] - 3.0 * p2[1] + p3[1]) * t3),
            ]);
        }
    }
    if let Some(last) = points.last() {
        result.push(*last);
    }
    Ok(result)
}

fn sample_bspline(points: &[[f64; 2]], knots: &[f64], weights: &[f64], degree: usize, tolerance: f64) -> Result<Vec<[f64; 2]>, Error> {
    let start = *knots.get(degree).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline knot vector".to_owned() })?;
    let end = *knots.get(points.len()).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline knot vector".to_owned() })?;
    if !start.is_finite() || !end.is_finite() || end <= start {
        return Err(Error::Malformed { format: "DXF", message: "invalid spline parameter range".to_owned() });
    }
    let segments = ((end - start) / tolerance.sqrt().max(0.01)).ceil().clamp(16.0, 4096.0) as usize;
    (0..=segments).map(|i| bspline_point(points, knots, weights, degree, start + (end - start) * i as f64 / segments as f64)).collect()
}

fn bspline_point(points: &[[f64; 2]], knots: &[f64], weights: &[f64], degree: usize, u: f64) -> Result<[f64; 2], Error> {
    let Some(n) = points.len().checked_sub(1) else {
        return Err(Error::Malformed { format: "DXF", message: "spline has no control points".to_owned() });
    };
    let final_knot = *knots.get(n + 1).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline knot vector".to_owned() })?;
    let span = if u >= final_knot {
        n
    } else {
        (degree..=n).find(|i| knots.get(*i).is_some_and(|a| u >= *a) && knots.get(*i + 1).is_some_and(|b| u < *b)).unwrap_or(n)
    };
    let mut d = Vec::with_capacity(degree + 1);
    for j in 0..=degree {
        let index = span - degree + j;
        let w = weights.get(index).copied().unwrap_or(1.0);
        if !w.is_finite() || w <= 0.0 {
            return Err(Error::Malformed { format: "DXF", message: "spline weights must be finite and positive".to_owned() });
        }
        let point = points.get(index).ok_or_else(|| Error::Malformed { format: "DXF", message: "spline control index is invalid".to_owned() })?;
        d.push([point[0] * w, point[1] * w, w]);
    }
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let right_knot =
                *knots.get(i + degree + 1 - r).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline knot vector".to_owned() })?;
            let left_knot = *knots.get(i).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline knot vector".to_owned() })?;
            let den = right_knot - left_knot;
            let alpha = if den == 0.0 { 0.0 } else { (u - left_knot) / den };
            let previous = *d.get(j - 1).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline recursion".to_owned() })?;
            let current = d.get_mut(j).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline recursion".to_owned() })?;
            let mix = |axis| {
                let previous = previous.get(axis).copied().ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline recursion".to_owned() })?;
                let current = current.get(axis).copied().ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline recursion".to_owned() })?;
                Ok::<f64, Error>((1.0 - alpha) * previous + alpha * current)
            };
            *current = [mix(0)?, mix(1)?, mix(2)?];
        }
    }
    let last = *d.get(degree).ok_or_else(|| Error::Malformed { format: "DXF", message: "invalid spline recursion".to_owned() })?;
    if last[2] == 0.0 || !last[2].is_finite() {
        return Err(Error::Malformed { format: "DXF", message: "invalid spline weight".to_owned() });
    }
    Ok([last[0] / last[2], last[1] / last[2]])
}

pub fn import_geometry_om(name: &str, bytes: &[u8]) -> Result<Vec<Polyline>, Error> {
    if !name.to_ascii_lowercase().ends_with(".dxf") {
        return Err(Error::UnknownFormat(name.to_owned()));
    }
    let mut polylines: Vec<Polyline> = parse_dxf_om(bytes)?.into_iter().filter(|line| line.len() >= 2).collect();
    if polylines.is_empty() {
        return Err(Error::Malformed { format: "DXF", message: format!("no usable geometry found in {name}") });
    }
    let scale = dxf_units_to_mm(bytes)?;
    let Some((min_x, min_y, max_x, max_y)) = raw_bounds(&polylines) else {
        return Err(Error::Malformed { format: "DXF", message: "no finite geometry".to_owned() });
    };
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    for line in &mut polylines {
        for point in &mut line.0 {
            let [x, y] = *point;
            *point = [(x - cx) * scale, (y - cy) * scale];
        }
    }
    Ok(polylines)
}

pub(crate) fn dxf_units_to_mm(bytes: &[u8]) -> Result<f64, Error> {
    let text = String::from_utf8_lossy(bytes);
    let pairs = dxf_pairs(&text);
    for window in pairs.windows(2) {
        let Some((code, value)) = window.first() else { continue };
        let Some((next_code, next_value)) = window.get(1) else { continue };
        if code == "9" && value.eq_ignore_ascii_case("$INSUNITS") && next_code == "70" {
            let unit = parse_f64(next_value)? as i32;
            return Ok(match unit {
                1 => 25.4,
                2 => 304.8,
                3 => 1_609_344.0,
                4 => 1.0,
                5 => 10.0,
                6 => 1000.0,
                7 => 1_000_000.0,
                8 => 0.000_025_4,
                9 => 0.0254,
                10 => 914.4,
                11 => 1e-7,
                12 => 1e-6,
                13 => 0.001,
                14 => 100.0,
                15 => 10_000.0,
                16 => 100_000.0,
                17 => 1_000_000_000.0,
                18 => 149_597_870_700_000.0,
                19 => 9.460_730_472_580_8e18,
                20 => 3.085_677_581_491_367e19,
                _ => 1.0,
            });
        }
    }
    Ok(1.0)
}

fn dxf_pairs(text: &str) -> Vec<Pair> {
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs = Vec::new();
    let mut i = 0usize;
    while let Some(line) = lines.get(i) {
        let code = line.trim();
        if code.parse::<i32>().is_ok() {
            let value = lines.get(i + 1).map_or("", |s| s.trim());
            pairs.push((code.to_owned(), value.to_owned()));
            i += 2;
        } else {
            i += 1;
        }
    }
    pairs
}

fn parse_old_polyline(pairs: &[Pair], index: usize, header: &[Pair]) -> Result<(Polyline, usize), Error> {
    let closed = flag_is_set(header, "70")?;
    let mut vertices = Vec::new();
    let mut i = index;
    while let Some((code, value)) = pairs.get(i) {
        if code == "0" && value == "SEQEND" {
            return Ok((flatten_bulged_vertices(&vertices, closed), i + 1));
        }
        if code == "0" && value == "VERTEX" {
            i += 1;
            let mut fields = Vec::new();
            while let Some((next_code, _)) = pairs.get(i) {
                if next_code == "0" {
                    break;
                }
                if let Some(pair) = pairs.get(i) {
                    fields.push(pair.clone());
                }
                i += 1;
            }
            let values = dxf_values(&fields)?;
            vertices.push((value_or(&values, "10"), value_or(&values, "20"), value_or(&values, "42")));
            continue;
        }
        if code == "0" {
            return Ok((flatten_bulged_vertices(&vertices, closed), i));
        }
        i += 1;
    }
    Ok((flatten_bulged_vertices(&vertices, closed), i))
}

fn parse_lwpolyline(fields: &[Pair]) -> Result<Polyline, Error> {
    let closed = flag_is_set(fields, "70")?;
    let mut vertices = Vec::new();
    let mut current: Option<Vertex> = None;
    for (code, value) in fields {
        match code.as_str() {
            "10" => {
                if let Some((x, y, bulge)) = current.take()
                    && y.is_finite()
                {
                    vertices.push((x, y, bulge));
                }
                current = Some((parse_f64(value)?, f64::NAN, 0.0));
            }
            "20" => {
                if let Some((x, _, bulge)) = current {
                    current = Some((x, parse_f64(value)?, bulge));
                }
            }
            "42" => {
                if let Some((x, y, _)) = current {
                    current = Some((x, y, parse_f64(value)?));
                }
            }
            _ => {}
        }
    }
    if let Some((x, y, bulge)) = current
        && y.is_finite()
    {
        vertices.push((x, y, bulge));
    }
    Ok(flatten_bulged_vertices(&vertices, closed))
}

fn flatten_bulged_vertices(vertices: &[Vertex], closed: bool) -> Polyline {
    let Some(&(first_x, first_y, _)) = vertices.first() else { return Polyline::default() };
    let mut points = vec![[first_x, first_y]];
    let last = if closed { vertices.len() } else { vertices.len().saturating_sub(1) };
    for i in 0..last {
        let Some(&(x1, y1, bulge)) = vertices.get(i) else { continue };
        let next = if i + 1 == vertices.len() { vertices.first() } else { vertices.get(i + 1) };
        let Some(&(x2, y2, _)) = next else { continue };
        let segment = bulge_segment_points([x1, y1], [x2, y2], bulge);
        points.extend(segment.0.into_iter().skip(1));
    }
    if closed
        && points.first() != points.last()
        && let Some(first) = points.first().copied()
    {
        points.push(first);
    }
    Polyline(points)
}

fn bulge_segment_points(start: [f64; 2], end: [f64; 2], bulge: f64) -> Polyline {
    if bulge.abs() < 1e-12 {
        return Polyline(vec![start, end]);
    }
    let [x1, y1] = start;
    let [x2, y2] = end;
    let dx = x2 - x1;
    let dy = y2 - y1;
    let chord = dx.hypot(dy);
    if chord < 1e-12 {
        return Polyline(vec![start, end]);
    }
    let theta = 4.0 * bulge.atan();
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let mid = [(x1 + x2) / 2.0, (y1 + y2) / 2.0];
    let center_offset = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
    let cx = mid[0] + (-dy / chord) * center_offset;
    let cy = mid[1] + (dx / chord) * center_offset;
    let start_angle = (y1 - cy).atan2(x1 - cx);
    let raw_steps = (theta.abs() / (PI / 24.0)).trunc() as i64 + 2;
    let steps = raw_steps.clamp(6, 96) as usize;
    Polyline(
        linspace(0.0, 1.0, steps).into_iter().map(|t| [cx + (start_angle + theta * t).cos() * radius, cy + (start_angle + theta * t).sin() * radius]).collect(),
    )
}

fn repeated_xy(fields: &[Pair], x_code: &str, y_code: &str) -> Result<Polyline, Error> {
    let mut points = Vec::new();
    let mut x = None;
    for (code, value) in fields {
        if code == x_code {
            x = Some(parse_f64(value)?);
        } else if code == y_code
            && let Some(x_value) = x.take()
        {
            points.push([x_value, parse_f64(value)?]);
        }
    }
    Ok(Polyline(points))
}

fn dxf_values(fields: &[Pair]) -> Result<Vec<(String, f64)>, Error> {
    let mut values = Vec::new();
    for (code, value) in fields {
        if matches!(code.as_str(), "10" | "20" | "11" | "21" | "40" | "41" | "42" | "50" | "51") {
            if let Some(existing) = values.iter_mut().find(|(key, _)| key == code) {
                existing.1 = parse_f64(value)?;
            } else {
                values.push((code.clone(), parse_f64(value)?));
            }
        }
    }
    Ok(values)
}

fn value_or(values: &[(String, f64)], code: &str) -> f64 {
    values.iter().find_map(|(key, value)| (key == code).then_some(*value)).unwrap_or(0.0)
}

fn flag_is_set(fields: &[Pair], code: &str) -> Result<bool, Error> {
    for (key, value) in fields {
        if key == code {
            return Ok((parse_f64(value)? as i64) & 1 != 0);
        }
    }
    Ok(false)
}

fn circle_points(cx: f64, cy: f64, radius: f64) -> Polyline {
    Polyline(linspace(0.0, TAU, 73).into_iter().map(|t| [cx + t.cos() * radius, cy + t.sin() * radius]).collect())
}

fn arc_points(cx: f64, cy: f64, radius: f64, start_deg: f64, mut end_deg: f64) -> Polyline {
    if end_deg < start_deg {
        end_deg += 360.0;
    }
    Polyline(
        linspace(start_deg, end_deg, 48)
            .into_iter()
            .map(|angle| {
                let radians = angle.to_radians();
                [cx + radians.cos() * radius, cy + radians.sin() * radius]
            })
            .collect(),
    )
}

fn ellipse_points(fields: &[Pair]) -> Result<Polyline, Error> {
    let values = dxf_values(fields)?;
    let cx = value_or(&values, "10");
    let cy = value_or(&values, "20");
    let major_x = value_or(&values, "11");
    let major_y = value_or(&values, "21");
    let ratio = values.iter().find_map(|(key, value)| (key == "40").then_some(*value)).unwrap_or(1.0);
    let start = value_or(&values, "41");
    let mut end = values.iter().find_map(|(key, value)| (key == "42").then_some(*value)).unwrap_or(TAU);
    if end <= start {
        end += TAU;
    }
    let minor_x = -major_y * ratio;
    let minor_y = major_x * ratio;
    Ok(Polyline(
        linspace(start, end, 96).into_iter().map(|t| [cx + major_x * t.cos() + minor_x * t.sin(), cy + major_y * t.cos() + minor_y * t.sin()]).collect(),
    ))
}

fn linspace(start: f64, end: f64, steps: usize) -> Vec<f64> {
    if steps <= 1 {
        return vec![start];
    }
    let denominator = (steps - 1) as f64;
    let step = (end - start) / denominator;
    (0..steps).map(|i| start + (i as f64) * step).collect()
}

fn parse_f64(text: &str) -> Result<f64, Error> {
    text.parse::<f64>().map_err(|e| Error::Malformed { format: "DXF", message: format!("invalid number {text:?}: {e}") })
}

fn raw_bounds(polylines: &[Polyline]) -> Option<(f64, f64, f64, f64)> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for line in polylines {
        for &[x, y] in &line.0 {
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            bounds = Some(match bounds {
                None => (x, y, x, y),
                Some((min_x, min_y, max_x, max_y)) => (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)),
            });
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t4_dxf_cases_match_golden() {
        let cases = openlaser_testkit::t4::load().expect("T4 fixtures load");
        assert_eq!(cases.len(), 30, "unexpected T4 fixture count");
        let mut checked = 0usize;
        for case in cases.iter().filter(|case| case.kind == "dxf") {
            let bytes = openlaser_testkit::t4::source_bytes(case).expect("T4 source");
            if case.function == "parse_dxf" {
                let actual = parse_dxf_om(&bytes).expect("DXF parses");
                let expected = case.output.as_ref().and_then(|v| v.get("polylines")).cloned().expect("expected polylines");
                assert_eq!(serde_json::to_value(actual).expect("serializable polylines"), expected, "T4 case {}", case.name);
            } else if case.function == "import_geometry" {
                let name = case.filename.as_str();
                let actual = import_geometry_om(name, &bytes);
                if let Some(expected) = case.output.as_ref().and_then(|v| v.get("polylines")) {
                    let actual = actual.expect("DXF geometry imports");
                    assert_eq!(serde_json::to_value(actual).expect("serializable polylines"), *expected, "T4 case {}", case.name);
                } else {
                    assert!(actual.is_err(), "T4 case {} must refuse", case.name);
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 25, "unexpected T4 DXF case coverage");
    }

    #[test]
    fn t4_non_dxf_cases_have_explicit_shipping_coverage() {
        let cases = openlaser_testkit::t4::load().expect("T4 fixtures load");
        let mut checked = 0usize;
        for case in cases.iter().filter(|case| case.kind != "dxf") {
            let bytes = openlaser_testkit::t4::source_bytes(case).expect("T4 source");
            match case.kind.as_str() {
                "svg" => {
                    let options = crate::ImportOptions { svg_units: crate::SvgUnits::Px96, ..crate::ImportOptions::default() };
                    let result = crate::import(&case.filename, &bytes, &options);
                    assert!(result.as_ref().is_ok_and(|imported| !imported.shapes.is_empty()), "T4 SVG {} imports: {result:?}", case.name);
                    assert!(!case.rust_must_match, "SVG rows are informational, not parity requirements");
                }
                "stl" | "other" => {
                    assert!(crate::import(&case.filename, &bytes, &crate::ImportOptions::default()).is_err(), "unsupported T4 case {} must refuse", case.name);
                }
                other => panic!("unexpected non-DXF T4 kind {other}"),
            }
            checked += 1;
        }
        assert_eq!(checked, 5, "all five non-DXF T4 rows must be covered");
    }

    #[test]
    fn shipping_splines_are_evaluated_while_compatibility_points_are_retained() {
        let case = openlaser_testkit::t4::load()
            .expect("T4 fixtures load")
            .into_iter()
            .find(|case| case.name == "spline_fit_and_control.parse_dxf")
            .expect("spline case");
        let bytes = openlaser_testkit::t4::source_bytes(&case).expect("spline source");
        let compatibility = parse_dxf_om(&bytes).expect("compatibility parser");
        let shipping = parse_dxf_shipping(&bytes, 0.05).expect("shipping parser");
        assert_eq!(compatibility.iter().map(Polyline::len).sum::<usize>(), 6);
        assert!(shipping.iter().map(Polyline::len).sum::<usize>() > 6);
    }
}

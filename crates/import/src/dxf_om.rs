//! DXF: the exact port of OM's parser (T4) and the shipping variant built on it.
//!
//! Ported from OpenMopa `mopa_luiz/importers.py` (`import_geometry`, `preserve_units_to_workspace`,
//! `dxf_units_to_mm`, `parse_dxf`, `dxf_pairs`, `parse_old_polyline`, `parse_lwpolyline`,
//! `flatten_bulged_vertices`, `bulge_segment_points`, `repeated_xy`, `dxf_values`, `circle_points`, `arc_points`,
//! `ellipse_points`, `linspace`) at commit `6aed8ffa8be5a938d195bf35198cfd168bb07f4e`.
//! MIT © 2026 Luiz Bueno (see ATTRIBUTION.md). The operation order is kept because T4 compares coordinates to
//! 1e-9 mm.
//!
//! Python behaviour that is reproduced: `decode("utf-8", errors="ignore")`, `str.splitlines()` line breaks (CR-only
//! files included) and `str.strip()` whitespace, polylines of fewer than 2 points kept by `parse_dxf` (dropped by
//! `import_geometry`), `any()` over every group-70 flag, `int(float(v))` refusing NaN and infinity, and a literal
//! `nan` coordinate kept as a coordinate (no sentinel).
//!
//! Deliberate differences (none reachable by T4):
//! - the input, the number of group-code pairs and the output points are capped ([`crate::Error::TooLarge`]);
//! - a group-code line is `-?[0-9]+` (Python's `\d` also matches non-ASCII digits);
//! - numbers use Rust's `f64` grammar (Python's `float()` also takes `_` separators and non-ASCII digits): such a
//!   value is refused, never misread;
//! - `import_geometry_om` refuses non-finite coordinates instead of returning them.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use crate::{Budget, Error, MAX_ENTITY_POINTS, Polyline, check_entity_points, check_vector_size};

/// Most group-code/value pairs one DXF may hold (bounds the pair table: 32 bytes a pair).
pub const MAX_DXF_PAIRS: usize = 4_000_000;

pub(crate) type Pair<'a> = (&'a str, &'a str);
type Vertex = (f64, f64, f64);

const ENTITIES: [&str; 7] = ["LINE", "LWPOLYLINE", "POLYLINE", "CIRCLE", "ARC", "ELLIPSE", "SPLINE"];

/// How SPLINE entities become polylines.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Splines {
    /// OM: the fit points, else the control points, as given (T4).
    Om,
    /// Shipping: the curve is evaluated with this chord tolerance (in file units).
    Evaluate(f64),
}

pub(crate) struct Entities {
    pub polylines: Vec<Polyline>,
    /// Shipping only: entities that gave fewer than 2 points (skipped, reported in a notice).
    pub degenerate: usize,
}

/// Exact port of OM `importers.parse_dxf` (T4: counts exact, coordinates within 1e-9).
/// SPLINE keeps OM's control/fit points; the shipping importer evaluates the curve (listed divergence).
pub fn parse_dxf_om(bytes: &[u8]) -> Result<Vec<Polyline>, Error> {
    let text = decode_ignore(check_vector_size("DXF", bytes)?);
    let pairs = dxf_pairs(&text)?;
    let mut budget = Budget::new();
    Ok(parse_entities(&pairs, Splines::Om, &mut budget)?.polylines)
}

/// Exact port of OM `importers.import_geometry` for `.dxf` (`preserve_units_to_workspace`: centred on the bounds,
/// scaled to mm, Y kept).
pub fn import_geometry_om(name: &str, bytes: &[u8]) -> Result<Vec<Polyline>, Error> {
    let lower = name.to_lowercase();
    let suffix = if name.contains('.') { lower.rsplit('.').next().unwrap_or_default() } else { "" };
    if suffix != "dxf" {
        return Err(Error::UnknownFormat(name.to_owned()));
    }
    let text = decode_ignore(check_vector_size("DXF", bytes)?);
    let pairs = dxf_pairs(&text)?;
    let (scale, _) = dxf_units(&pairs)?;
    let mut budget = Budget::new();
    let mut polylines: Vec<Polyline> = parse_entities(&pairs, Splines::Om, &mut budget)?.polylines.into_iter().filter(|line| line.len() >= 2).collect();
    if polylines.is_empty() {
        return Err(Error::Malformed { format: "DXF", message: format!("no usable geometry found in {name}") });
    }
    let Some((min_x, min_y, max_x, max_y)) = raw_bounds(&polylines) else {
        return Err(Error::Malformed { format: "DXF", message: format!("{name} has a coordinate that is not finite") });
    };
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    for line in &mut polylines {
        for point in &mut line.0 {
            let [x, y] = *point;
            *point = [(x - cx) * scale, (y - cy) * scale];
            if !point[0].is_finite() || !point[1].is_finite() {
                return Err(Error::Malformed { format: "DXF", message: format!("{name}: point ({x}, {y}) is not finite in millimetres") });
            }
        }
    }
    Ok(polylines)
}

/// `bytes.decode("utf-8", errors="ignore")`: invalid sequences are dropped (not replaced).
pub(crate) fn decode_ignore(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => {
            let mut text = String::with_capacity(bytes.len());
            for chunk in bytes.utf8_chunks() {
                text.push_str(chunk.valid());
            }
            Cow::Owned(text)
        }
    }
}

/// OM `dxf_units_to_mm`: (mm per file unit, assumed). `assumed` is true when `$INSUNITS` is missing, 0 or unknown
/// (OM's "unitless_assumed_mm").
pub(crate) fn dxf_units(pairs: &[Pair<'_>]) -> Result<(f64, bool), Error> {
    for window in pairs.windows(2) {
        let (Some(&(code, value)), Some(&(next_code, next_value))) = (window.first(), window.get(1)) else { continue };
        if code == "9" && value.to_uppercase() == "$INSUNITS" {
            if next_code != "70" {
                continue;
            }
            let unit = py_int(parse_f64(next_value)?)?;
            let scale = match unit {
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
                _ => return Ok((1.0, true)),
            };
            return Ok((scale, false));
        }
    }
    Ok((1.0, true))
}

/// Python `str.splitlines()`.
struct PyLines<'a> {
    rest: &'a str,
}

impl<'a> Iterator for PyLines<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.rest.is_empty() {
            return None;
        }
        for (i, c) in self.rest.char_indices() {
            if matches!(c, '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}') {
                let line = self.rest.get(..i).unwrap_or_default();
                let mut next = i + c.len_utf8();
                if c == '\r' && self.rest.get(next..).is_some_and(|rest| rest.starts_with('\n')) {
                    next += 1;
                }
                self.rest = self.rest.get(next..).unwrap_or_default();
                return Some(line);
            }
        }
        let line = self.rest;
        self.rest = "";
        Some(line)
    }
}

/// Python `str.strip()` (`str.isspace` = Unicode White_Space plus U+001C..U+001F).
fn py_strip(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

fn is_group_code(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// OM `dxf_pairs`: integer group-code lines paired with the following line, extra blank lines tolerated.
pub(crate) fn dxf_pairs(text: &str) -> Result<Vec<Pair<'_>>, Error> {
    let mut lines = PyLines { rest: text };
    let mut pairs = Vec::new();
    while let Some(line) = lines.next() {
        let code = py_strip(line);
        if !is_group_code(code) {
            continue;
        }
        let value = lines.next().map_or("", py_strip);
        if pairs.len() >= MAX_DXF_PAIRS {
            return Err(Error::TooLarge(format!("DXF has more than {MAX_DXF_PAIRS} group-code pairs")));
        }
        pairs.push((code, value));
    }
    Ok(pairs)
}

/// OM `parse_dxf` over the pairs, with every entity accounted in `budget`.
pub(crate) fn parse_entities(pairs: &[Pair<'_>], splines: Splines, budget: &mut Budget) -> Result<Entities, Error> {
    let mut out = Entities { polylines: Vec::new(), degenerate: 0 };
    let mut i = 0usize;
    while let Some(&(code, value)) = pairs.get(i) {
        if code == "0" && ENTITIES.contains(&value) {
            i += 1;
            let start = i;
            while pairs.get(i).is_some_and(|(next_code, _)| *next_code != "0") {
                i += 1;
            }
            let fields = pairs.get(start..i).unwrap_or_default();
            let polyline = match value {
                "LINE" => {
                    let v = DxfValues::new(fields)?;
                    Some(Polyline(vec![[v.get("10", 0.0), v.get("20", 0.0)], [v.get("11", 0.0), v.get("21", 0.0)]]))
                }
                "LWPOLYLINE" => Some(parse_lwpolyline(fields)?),
                "POLYLINE" => {
                    let (points, next) = parse_old_polyline(pairs, i, fields)?;
                    i = next;
                    Some(points)
                }
                "CIRCLE" => {
                    let v = DxfValues::new(fields)?;
                    Some(circle_points(v.get("10", 0.0), v.get("20", 0.0), v.get("40", 0.0)))
                }
                "ARC" => {
                    let v = DxfValues::new(fields)?;
                    Some(arc_points(v.get("10", 0.0), v.get("20", 0.0), v.get("40", 0.0), v.get("50", 0.0), v.get("51", 0.0)))
                }
                "ELLIPSE" => Some(ellipse_points(fields)?),
                _ => match splines {
                    Splines::Om => {
                        let fit = repeated_xy(fields, "11", "21")?;
                        let points = if fit.is_empty() { repeated_xy(fields, "10", "20")? } else { fit };
                        (points.len() >= 2).then_some(points)
                    }
                    Splines::Evaluate(tolerance) => Some(evaluate_spline(fields, tolerance, budget)?),
                },
            };
            match (polyline, splines) {
                (Some(polyline), Splines::Om) => {
                    budget.entity(polyline.len())?;
                    out.polylines.push(polyline);
                }
                (Some(polyline), Splines::Evaluate(_)) if polyline.len() >= 2 => {
                    budget.entity(polyline.len())?;
                    out.polylines.push(polyline);
                }
                (Some(_), Splines::Evaluate(_)) => out.degenerate += 1,
                (None, _) => {}
            }
            continue;
        }
        i += 1;
    }
    Ok(out)
}

/// Entity types in the ENTITIES section that neither parser reads, with their counts (shipping notice).
pub(crate) fn unsupported_entities<'a>(pairs: &[Pair<'a>]) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    let mut in_entities = false;
    for (index, &(code, value)) in pairs.iter().enumerate() {
        if code != "0" {
            continue;
        }
        match value {
            "SECTION" => in_entities = pairs.get(index + 1).is_some_and(|&(c, v)| c == "2" && v == "ENTITIES"),
            "ENDSEC" => in_entities = false,
            "VERTEX" | "SEQEND" => {}
            other if in_entities && !ENTITIES.contains(&other) => *counts.entry(other).or_insert(0) += 1,
            _ => {}
        }
    }
    counts
}

fn evaluate_spline(fields: &[Pair<'_>], tolerance: f64, budget: &mut Budget) -> Result<Polyline, Error> {
    let fit = repeated_xy(fields, "11", "21")?.0;
    let controls = repeated_xy(fields, "10", "20")?.0;
    let is_fit_spline = fit.len() >= 2;
    let points = if is_fit_spline { fit } else { controls };
    if points.len() < 2 {
        return Ok(Polyline(Vec::new()));
    }
    check_entity_points(points.len())?;
    if points.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return Err(malformed("spline has a point that is not finite"));
    }
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(malformed("spline tolerance must be finite and positive"));
    }
    // Many CAD writers omit the knot vector for a fit-point spline. Interpolate those points with a Catmull-Rom
    // curve. Control-point entities with a full knot vector are sampled as a (rational) B-spline; without one, as a
    // clamped Bezier curve when the degree permits it.
    let degree = fields.iter().find(|(code, _)| *code == "71").and_then(|(_, value)| value.parse::<usize>().ok());
    let knots: Vec<f64> = fields.iter().filter(|(code, _)| *code == "40").map(|(_, value)| parse_f64(value)).collect::<Result<_, _>>()?;
    let weights: Vec<f64> = fields.iter().filter(|(code, _)| *code == "41").map(|(_, value)| parse_f64(value)).collect::<Result<_, _>>()?;
    if !weights.is_empty() && weights.len() != points.len() {
        return Err(malformed("spline weight count does not match its points"));
    }
    if weights.iter().any(|weight| !weight.is_finite() || *weight <= 0.0) {
        return Err(malformed("spline weights must be finite and positive"));
    }
    if is_fit_spline {
        return Ok(Polyline(interpolate_fit_points(&points, tolerance, budget)?));
    }
    if let Some(degree) = degree
        && degree > 0
        && degree < points.len()
        && knots.len() == points.len() + degree + 1
    {
        return Ok(Polyline(sample_bspline(&points, &knots, &weights, degree, tolerance, budget)?));
    }
    if !knots.is_empty() || degree.is_some_and(|degree| degree != points.len().saturating_sub(1)) {
        return Err(malformed("incomplete or inconsistent spline knot vector"));
    }
    if points.len() > 128 {
        return Err(Error::TooLarge(format!("a DXF Bezier spline has {} control points; the limit is 128", points.len())));
    }
    Ok(Polyline(sample_bezier(&points, &weights, tolerance, budget)?))
}

fn malformed(message: &str) -> Error {
    Error::Malformed { format: "DXF", message: message.to_owned() }
}

fn polygon_length(points: &[[f64; 2]]) -> f64 {
    points.windows(2).filter_map(|pair| Some((pair.first()?, pair.get(1)?))).map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1])).sum()
}

/// Sample count for a curve of control-polygon length `length` (file units): one chord per √tolerance, 16..=cap.
fn sample_segments(length: f64, tolerance: f64, minimum: usize) -> Result<usize, Error> {
    let raw = (length / tolerance.sqrt().max(1e-6)).ceil();
    if !raw.is_finite() {
        return Err(malformed("spline is too long to sample"));
    }
    let cap = MAX_ENTITY_POINTS - 1;
    // Clamped to `16..=cap` before the conversion, so the cast is exact.
    let segments = raw.clamp(16.0, cap as f64) as usize;
    Ok(segments.max(minimum).min(cap))
}

fn sample_bezier(points: &[[f64; 2]], weights: &[f64], tolerance: f64, budget: &mut Budget) -> Result<Vec<[f64; 2]>, Error> {
    let segments = sample_segments(polygon_length(points), tolerance, 16)?.min(4096);
    let n = points.len() as u64;
    budget.work((segments as u64 + 1).saturating_mul(n.saturating_mul(n) / 2 + 1))?;
    let homogeneous: Vec<[f64; 3]> = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let weight = weights.get(index).copied().unwrap_or(1.0);
            [point[0] * weight, point[1] * weight, weight]
        })
        .collect();
    let mut work = homogeneous.clone();
    (0..=segments)
        .map(|i| {
            let t = i as f64 / segments as f64;
            work.clone_from(&homogeneous);
            for level in (1..work.len()).rev() {
                for index in 0..level {
                    let (Some(left), Some(right)) = (work.get(index).copied(), work.get(index + 1).copied()) else {
                        return Err(malformed("invalid Bezier spline controls"));
                    };
                    if let Some(current) = work.get_mut(index) {
                        *current = [left[0] * (1.0 - t) + right[0] * t, left[1] * (1.0 - t) + right[1] * t, left[2] * (1.0 - t) + right[2] * t];
                    }
                }
            }
            let point = work.first().copied().ok_or_else(|| malformed("spline has no control points"))?;
            if point[2] <= 0.0 || !point[2].is_finite() {
                return Err(malformed("invalid spline weight"));
            }
            Ok([point[0] / point[2], point[1] / point[2]])
        })
        .collect()
}

fn interpolate_fit_points(points: &[[f64; 2]], tolerance: f64, budget: &mut Budget) -> Result<Vec<[f64; 2]>, Error> {
    let raw = (1.0 / tolerance.sqrt()).ceil();
    // Clamped to 16..=128 before the conversion, so the cast is exact.
    let subdivisions = if raw.is_finite() { raw.clamp(16.0, 128.0) as usize } else { 128 };
    let total = points.len().saturating_sub(1).saturating_mul(subdivisions).saturating_add(1);
    check_entity_points(total)?;
    budget.work(total as u64)?;
    let mut result = Vec::with_capacity(total);
    for i in 0..points.len().saturating_sub(1) {
        let (Some(&p1), Some(&p2)) = (points.get(i), points.get(i + 1)) else { return Err(malformed("invalid spline fit points")) };
        let p0 = if i == 0 { p1 } else { points.get(i - 1).copied().unwrap_or(p1) };
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

fn sample_bspline(points: &[[f64; 2]], knots: &[f64], weights: &[f64], degree: usize, tolerance: f64, budget: &mut Budget) -> Result<Vec<[f64; 2]>, Error> {
    let start = *knots.get(degree).ok_or_else(|| malformed("invalid spline knot vector"))?;
    let end = *knots.get(points.len()).ok_or_else(|| malformed("invalid spline knot vector"))?;
    if knots.iter().any(|k| !k.is_finite()) || end <= start || knots.windows(2).any(|pair| pair.get(1) < pair.first()) {
        return Err(malformed("invalid spline parameter range"));
    }
    // At least 8 samples a knot span, and one chord per √tolerance of control polygon.
    let spans = points.len() - degree;
    let segments = sample_segments(polygon_length(points), tolerance, spans.saturating_mul(8))?;
    let order = degree as u64 + 1;
    budget.work((segments as u64 + 1).saturating_mul(order.saturating_mul(order)))?;
    (0..=segments).map(|i| bspline_point(points, knots, weights, degree, start + (end - start) * i as f64 / segments as f64)).collect()
}

fn bspline_point(points: &[[f64; 2]], knots: &[f64], weights: &[f64], degree: usize, u: f64) -> Result<[f64; 2], Error> {
    let Some(n) = points.len().checked_sub(1) else {
        return Err(malformed("spline has no control points"));
    };
    let final_knot = *knots.get(n + 1).ok_or_else(|| malformed("invalid spline knot vector"))?;
    // The knots are finite and non-decreasing (checked by the caller), so the span is a binary search: the last
    // index i in degree..=n with knots[i] <= u.
    let span = if u >= final_knot { n } else { knots.get(..=n).unwrap_or_default().partition_point(|k| *k <= u).saturating_sub(1).clamp(degree, n) };
    let mut d = Vec::with_capacity(degree + 1);
    for j in 0..=degree {
        let index = span - degree + j;
        let w = weights.get(index).copied().unwrap_or(1.0);
        let point = points.get(index).ok_or_else(|| malformed("spline control index is invalid"))?;
        d.push([point[0] * w, point[1] * w, w]);
    }
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let right_knot = *knots.get(i + degree + 1 - r).ok_or_else(|| malformed("invalid spline knot vector"))?;
            let left_knot = *knots.get(i).ok_or_else(|| malformed("invalid spline knot vector"))?;
            let den = right_knot - left_knot;
            let alpha = if den == 0.0 { 0.0 } else { (u - left_knot) / den };
            let previous = *d.get(j - 1).ok_or_else(|| malformed("invalid spline recursion"))?;
            let current = d.get_mut(j).ok_or_else(|| malformed("invalid spline recursion"))?;
            *current = [
                (1.0 - alpha) * previous[0] + alpha * current[0],
                (1.0 - alpha) * previous[1] + alpha * current[1],
                (1.0 - alpha) * previous[2] + alpha * current[2],
            ];
        }
    }
    let last = *d.get(degree).ok_or_else(|| malformed("invalid spline recursion"))?;
    if last[2] == 0.0 || !last[2].is_finite() {
        return Err(malformed("invalid spline weight"));
    }
    Ok([last[0] / last[2], last[1] / last[2]])
}

fn parse_old_polyline<'a>(pairs: &[Pair<'a>], index: usize, header: &[Pair<'a>]) -> Result<(Polyline, usize), Error> {
    let closed = flag_any(header)?;
    let mut vertices = Vec::new();
    let mut i = index;
    while let Some(&(code, value)) = pairs.get(i) {
        if code == "0" && value == "SEQEND" {
            return Ok((flatten_bulged_vertices(&vertices, closed)?, i + 1));
        }
        if code == "0" && value == "VERTEX" {
            i += 1;
            let start = i;
            while pairs.get(i).is_some_and(|(next_code, _)| *next_code != "0") {
                i += 1;
            }
            let v = DxfValues::new(pairs.get(start..i).unwrap_or_default())?;
            vertices.push((v.get("10", 0.0), v.get("20", 0.0), v.get("42", 0.0)));
            check_entity_points(vertices.len())?;
            continue;
        }
        if code == "0" {
            return Ok((flatten_bulged_vertices(&vertices, closed)?, i));
        }
        i += 1;
    }
    Ok((flatten_bulged_vertices(&vertices, closed)?, i))
}

fn parse_lwpolyline(fields: &[Pair<'_>]) -> Result<Polyline, Error> {
    let closed = flag_any(fields)?;
    let mut vertices = Vec::new();
    // Python's `current` dict: x, then y and bulge once given (no sentinel: a literal `nan` y is a coordinate).
    let mut current: Option<(f64, Option<f64>, f64)> = None;
    for &(code, value) in fields {
        match code {
            "10" => {
                if let Some((x, Some(y), bulge)) = current {
                    vertices.push((x, y, bulge));
                    check_entity_points(vertices.len())?;
                }
                current = Some((parse_f64(value)?, None, 0.0));
            }
            "20" => {
                if let Some((x, _, bulge)) = current {
                    current = Some((x, Some(parse_f64(value)?), bulge));
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
    if let Some((x, Some(y), bulge)) = current {
        vertices.push((x, y, bulge));
    }
    flatten_bulged_vertices(&vertices, closed)
}

fn flatten_bulged_vertices(vertices: &[Vertex], closed: bool) -> Result<Polyline, Error> {
    let Some(&(first_x, first_y, _)) = vertices.first() else { return Ok(Polyline::default()) };
    let mut points = vec![[first_x, first_y]];
    let last = if closed { vertices.len() } else { vertices.len().saturating_sub(1) };
    for i in 0..last {
        let Some(&(x1, y1, bulge)) = vertices.get(i) else { continue };
        let next = if i + 1 == vertices.len() { vertices.first() } else { vertices.get(i + 1) };
        let Some(&(x2, y2, _)) = next else { continue };
        let segment = bulge_segment_points([x1, y1], [x2, y2], bulge)?;
        points.extend(segment.0.into_iter().skip(1));
        check_entity_points(points.len())?;
    }
    if closed
        && points.first() != points.last()
        && let Some(first) = points.first().copied()
    {
        points.push(first);
    }
    Ok(Polyline(points))
}

fn bulge_segment_points(start: [f64; 2], end: [f64; 2], bulge: f64) -> Result<Polyline, Error> {
    if bulge.abs() < 1e-12 {
        return Ok(Polyline(vec![start, end]));
    }
    let [x1, y1] = start;
    let [x2, y2] = end;
    let dx = x2 - x1;
    let dy = y2 - y1;
    let chord = dx.hypot(dy);
    if chord < 1e-12 {
        return Ok(Polyline(vec![start, end]));
    }
    let theta = 4.0 * bulge.atan();
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let mid = [(x1 + x2) / 2.0, (y1 + y2) / 2.0];
    let center_offset = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
    let cx = mid[0] + (-dy / chord) * center_offset;
    let cy = mid[1] + (dx / chord) * center_offset;
    let start_angle = (y1 - cy).atan2(x1 - cx);
    // Python `int(abs(theta) / (pi / 24)) + 2`: `int()` of NaN raises.
    let raw_steps = (theta.abs() / (PI / 24.0)).trunc();
    if !raw_steps.is_finite() {
        return Err(malformed("bulge is not a number"));
    }
    // |theta| <= 2π, so raw_steps <= 48 and the cast is exact.
    let steps = (raw_steps as usize + 2).clamp(6, 96);
    Ok(Polyline(
        linspace(0.0, 1.0, steps).into_iter().map(|t| [cx + (start_angle + theta * t).cos() * radius, cy + (start_angle + theta * t).sin() * radius]).collect(),
    ))
}

fn repeated_xy(fields: &[Pair<'_>], x_code: &str, y_code: &str) -> Result<Polyline, Error> {
    let mut points = Vec::new();
    let mut x = None;
    for &(code, value) in fields {
        if code == x_code {
            x = Some(parse_f64(value)?);
        } else if code == y_code
            && let Some(x_value) = x.take()
        {
            points.push([x_value, parse_f64(value)?]);
            check_entity_points(points.len())?;
        }
    }
    Ok(Polyline(points))
}

/// OM `dxf_values`: the last value of each coordinate code, every one parsed.
struct DxfValues<'a>(Vec<(&'a str, f64)>);

impl<'a> DxfValues<'a> {
    fn new(fields: &[Pair<'a>]) -> Result<Self, Error> {
        let mut values: Vec<(&'a str, f64)> = Vec::new();
        for &(code, value) in fields {
            if matches!(code, "10" | "20" | "11" | "21" | "40" | "41" | "42" | "50" | "51") {
                let parsed = parse_f64(value)?;
                match values.iter_mut().find(|(key, _)| *key == code) {
                    Some(existing) => existing.1 = parsed,
                    None => values.push((code, parsed)),
                }
            }
        }
        Ok(DxfValues(values))
    }

    fn get(&self, code: &str, default: f64) -> f64 {
        self.0.iter().find_map(|(key, value)| (*key == code).then_some(*value)).unwrap_or(default)
    }
}

/// Python `any(code == "70" and int(float(val)) & 1 for code, val in fields)`.
fn flag_any(fields: &[Pair<'_>]) -> Result<bool, Error> {
    for &(code, value) in fields {
        if code == "70" && py_int(parse_f64(value)?)? & 1 != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Python `int(v)` for a float: truncates; NaN and infinity raise. Floats of magnitude >= 2^53 are even integers,
/// so their low bit (all that callers read) is 0, and they compare unequal to every unit code.
fn py_int(v: f64) -> Result<i64, Error> {
    if !v.is_finite() {
        return Err(Error::Malformed { format: "DXF", message: format!("cannot convert {v} to an integer") });
    }
    let t = v.trunc();
    // In range, so the cast is exact.
    Ok(if t.abs() < 9_007_199_254_740_992.0 { t as i64 } else { 0 })
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

fn ellipse_points(fields: &[Pair<'_>]) -> Result<Polyline, Error> {
    let v = DxfValues::new(fields)?;
    let cx = v.get("10", 0.0);
    let cy = v.get("20", 0.0);
    let major_x = v.get("11", 0.0);
    let major_y = v.get("21", 0.0);
    let ratio = v.get("40", 1.0);
    let start = v.get("41", 0.0);
    let mut end = v.get("42", TAU);
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

    fn dxf(lines: &[&str], eol: &str) -> Vec<u8> {
        let mut text = lines.join(eol);
        text.push_str(eol);
        text.into_bytes()
    }

    const LWPOLY_ONE_VERTEX: [&str; 8] = ["0", "SECTION", "2", "ENTITIES", "0", "LWPOLYLINE", "10", "1.5"];

    #[test]
    fn shipping_splines_are_evaluated_while_compatibility_points_are_retained() {
        let case = openlaser_testkit::t4::load()
            .expect("T4 fixtures load")
            .into_iter()
            .find(|case| case.name == "spline_fit_and_control.parse_dxf")
            .expect("spline case");
        let bytes = openlaser_testkit::t4::source_bytes(&case).expect("spline source");
        let compatibility = parse_dxf_om(&bytes).expect("compatibility parser");
        let text = decode_ignore(&bytes);
        let pairs = dxf_pairs(&text).expect("pairs");
        let shipping = parse_entities(&pairs, Splines::Evaluate(0.05), &mut Budget::new()).expect("shipping parser");
        assert_eq!(compatibility.iter().map(Polyline::len).sum::<usize>(), 6);
        assert!(shipping.polylines.iter().map(Polyline::len).sum::<usize>() > 6);
    }

    #[test]
    fn cr_only_line_endings_parse_like_python_splitlines() {
        let lines = ["0", "LINE", "10", "1", "20", "2", "11", "3", "21", "4", "0", "EOF"];
        let lf = parse_dxf_om(&dxf(&lines, "\n")).expect("LF");
        assert_eq!(parse_dxf_om(&dxf(&lines, "\r")).expect("CR"), lf);
        assert_eq!(parse_dxf_om(&dxf(&lines, "\r\n")).expect("CRLF"), lf);
        assert_eq!(parse_dxf_om(&dxf(&lines, "\u{2028}")).expect("LS"), lf);
        assert_eq!(lf, vec![Polyline(vec![[1.0, 2.0], [3.0, 4.0]])]);
    }

    #[test]
    fn parse_dxf_keeps_short_polylines_like_python_and_import_geometry_drops_them() {
        let mut lines = LWPOLY_ONE_VERTEX.to_vec();
        lines.extend(["20", "2.5", "0", "LINE", "10", "0", "20", "0", "11", "1", "21", "1", "0", "ENDSEC", "0", "EOF"]);
        let bytes = dxf(&lines, "\n");
        let parsed = parse_dxf_om(&bytes).expect("parses");
        assert_eq!(parsed.first(), Some(&Polyline(vec![[1.5, 2.5]])), "a one-vertex LWPOLYLINE is kept by parse_dxf");
        assert_eq!(parsed.len(), 2);
        assert_eq!(import_geometry_om("p.dxf", &bytes).expect("imports").len(), 1);
    }

    #[test]
    fn group_70_flag_is_any_of_all_values() {
        let lines = ["0", "LWPOLYLINE", "70", "0", "70", "1", "10", "0", "20", "0", "10", "1", "20", "0", "10", "1", "20", "1", "0", "EOF"];
        let parsed = parse_dxf_om(&dxf(&lines, "\n")).expect("parses");
        let line = parsed.first().expect("polyline");
        assert_eq!(line.0.first(), line.0.last(), "the second 70 closes it");
        assert_eq!(line.len(), 4);
        let nan_flag = ["0", "LWPOLYLINE", "70", "nan", "10", "0", "20", "0", "0", "EOF"];
        assert!(parse_dxf_om(&dxf(&nan_flag, "\n")).is_err(), "int(float('nan')) raises in Python");
    }

    #[test]
    fn literal_nan_y_is_a_coordinate_not_a_missing_value() {
        let lines = ["0", "LWPOLYLINE", "10", "0", "20", "nan", "10", "1", "20", "1", "0", "EOF"];
        let parsed = parse_dxf_om(&dxf(&lines, "\n")).expect("parses");
        let line = parsed.first().expect("polyline");
        assert_eq!(line.len(), 2, "Python keeps the NaN vertex");
        assert!(line.0.first().is_some_and(|p| p[1].is_nan()));
        assert!(import_geometry_om("nan.dxf", &dxf(&lines, "\n")).is_err(), "non-finite coordinates are refused, never returned");
    }

    #[test]
    fn invalid_utf8_is_dropped_like_errors_ignore() {
        let mut bytes = b"0\nLINE\n10\n1".to_vec();
        bytes.push(0xff);
        bytes.extend_from_slice(b"0\n20\n2\n11\n3\n21\n4\n0\nEOF\n");
        let parsed = parse_dxf_om(&bytes).expect("parses");
        assert_eq!(parsed, vec![Polyline(vec![[10.0, 2.0], [3.0, 4.0]])], "the 0xff byte is dropped, not replaced");
    }

    #[test]
    fn entity_and_total_point_caps_refuse() {
        let mut lines = vec!["0", "SPLINE"];
        let n = MAX_ENTITY_POINTS + 1;
        for _ in 0..n {
            lines.extend(["11", "1", "21", "2"]);
        }
        lines.extend(["0", "EOF"]);
        assert!(matches!(parse_dxf_om(&dxf(&lines, "\n")), Err(Error::TooLarge(_))), "per-entity cap");

        let circle = ["0", "CIRCLE", "10", "0", "20", "0", "40", "1"];
        let count = crate::MAX_IMPORT_POINTS / 73 + 1;
        let mut lines = Vec::with_capacity(count * circle.len() + 2);
        for _ in 0..count {
            lines.extend(circle);
        }
        lines.extend(["0", "EOF"]);
        assert!(matches!(parse_dxf_om(&dxf(&lines, "\n")), Err(Error::TooLarge(_))), "total cap stops CIRCLE amplification");
    }

    #[test]
    fn unsupported_entities_are_counted_in_the_entities_section_only() {
        let lines = [
            "0", "SECTION", "2", "BLOCKS", "0", "TEXT", "0", "ENDSEC", "0", "SECTION", "2", "ENTITIES", "0", "INSERT", "0", "TEXT", "0", "TEXT", "0", "LINE",
            "0", "ENDSEC",
        ];
        let text = String::from_utf8(dxf(&lines, "\n")).expect("utf-8");
        let pairs = dxf_pairs(&text).expect("pairs");
        let counts = unsupported_entities(&pairs);
        assert_eq!(counts.get("TEXT"), Some(&2));
        assert_eq!(counts.get("INSERT"), Some(&1));
        assert_eq!(counts.len(), 2);
    }
}

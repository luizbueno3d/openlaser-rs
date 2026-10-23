//! T3 hatch cases (`tests/golden/t3_hatch/hatch.json`): name, polylines, params, output.
//! Owned by WP1 (SPEC §7.12).
//!
//! Cases whose polylines live in `inputs/workspace.json` (`polylines_ref`) are resolved here: the business-card
//! raster layer is every object with `layer_id == "raster"`, in file order, keeping polylines of at least two
//! points (OM `layers.group_by_layer`). The generator refuses un-baked object transforms, so none are applied.

use serde_json::Value;

/// OM `hatch_polylines` defaults, used when a case omits a parameter.
pub const DEFAULT_ANGLE_DEG: f64 = 0.0;
pub const DEFAULT_PITCH_MM: f64 = 0.1;
pub const DEFAULT_PASSES: u32 = 1;
pub const DEFAULT_ANGLE_STEP_DEG: f64 = 90.0;
pub const DEFAULT_CLOSURE_TOL_MM: f64 = 0.05;

#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub angle_deg: f64,
    pub pitch_mm: f64,
    pub passes: u32,
    pub angle_step_deg: f64,
    pub closure_tol_mm: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub segments: Vec<[[f64; 2]; 2]>,
    pub segment_count: usize,
    pub closed_input_count: usize,
    pub skipped_open_count: usize,
    pub passes: u32,
    pub pitch_mm: f64,
    pub angles_deg: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expected {
    Output(Output),
    /// OM raised (`type`, `message`); Rust compares the decision and the category.
    Error {
        kind: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub name: String,
    pub polylines: Vec<Vec<[f64; 2]>>,
    pub params: Params,
    pub expected: Expected,
}

/// All cases of `t3_hatch/hatch.json`, in file order.
pub fn load() -> Result<Vec<Case>, String> {
    let path = crate::golden_dir().join("t3_hatch").join("hatch.json");
    let doc = read_json(&path)?;
    let cases = doc.get("cases").and_then(Value::as_array).ok_or("hatch.json has no `cases` array")?;
    let mut workspace: Option<Value> = None;
    cases.iter().map(|c| parse_case(c, &mut workspace)).collect()
}

fn read_json(path: &std::path::Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn f64_at(v: &Value, key: &str, default: f64) -> Result<f64, String> {
    match v.get(key) {
        None => Ok(default),
        Some(x) => x.as_f64().ok_or_else(|| format!("`{key}` is not a number")),
    }
}

fn point(v: &Value) -> Result<[f64; 2], String> {
    match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => Ok([x.as_f64().ok_or("x is not a number")?, y.as_f64().ok_or("y is not a number")?]),
        _ => Err(format!("not an [x, y] point: {v}")),
    }
}

fn polylines(v: &Value) -> Result<Vec<Vec<[f64; 2]>>, String> {
    v.as_array().ok_or("polylines is not an array")?.iter().map(|p| p.as_array().ok_or("polyline is not an array")?.iter().map(point).collect()).collect()
}

fn business_card_raster(workspace: &mut Option<Value>) -> Result<Vec<Vec<[f64; 2]>>, String> {
    if workspace.is_none() {
        *workspace = Some(read_json(&crate::golden_dir().join("inputs").join("workspace.json"))?);
    }
    let ws = workspace.as_ref().ok_or("workspace.json not loaded")?;
    let objects = ws.get("objects").and_then(Value::as_array).ok_or("workspace.json has no `objects`")?;
    let mut out = Vec::new();
    for o in objects.iter().filter(|o| o.get("layer_id").and_then(Value::as_str) == Some("raster")) {
        for p in polylines(o.get("polylines").ok_or("object without polylines")?)? {
            if p.len() >= 2 {
                out.push(p);
            }
        }
    }
    Ok(out)
}

fn parse_case(c: &Value, workspace: &mut Option<Value>) -> Result<Case, String> {
    let name = c.get("name").and_then(Value::as_str).ok_or("case without a name")?.to_owned();
    let polys = if let Some(p) = c.get("polylines") {
        polylines(p)?
    } else if c.get("polylines_ref").is_some() {
        let p = business_card_raster(workspace)?;
        let want = c.get("polylines_count").and_then(Value::as_u64).ok_or("polylines_ref without polylines_count")?;
        if u64::try_from(p.len()).ok() != Some(want) {
            return Err(format!("{name}: resolved {} polylines, the fixture says {want}", p.len()));
        }
        p
    } else {
        return Err(format!("{name}: no polylines"));
    };
    let params = c.get("params").ok_or_else(|| format!("{name}: no params"))?;
    let passes = match params.get("passes") {
        None => DEFAULT_PASSES,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| format!("{name}: bad passes"))?,
    };
    let params = Params {
        angle_deg: f64_at(params, "angle_deg", DEFAULT_ANGLE_DEG)?,
        pitch_mm: f64_at(params, "pitch_mm", DEFAULT_PITCH_MM)?,
        passes,
        angle_step_deg: f64_at(params, "angle_step_deg", DEFAULT_ANGLE_STEP_DEG)?,
        closure_tol_mm: f64_at(params, "closure_tol_mm", DEFAULT_CLOSURE_TOL_MM)?,
    };
    let expected = if let Some(e) = c.get("error") {
        Expected::Error {
            kind: e.get("type").and_then(Value::as_str).unwrap_or_default().to_owned(),
            message: e.get("message").and_then(Value::as_str).unwrap_or_default().to_owned(),
        }
    } else {
        let o = c.get("output").ok_or_else(|| format!("{name}: neither output nor error"))?;
        let count = |k: &str| o.get(k).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| format!("{name}: bad `{k}`"));
        let segments = o
            .get("segments")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{name}: no segments"))?
            .iter()
            .map(|s| match s.as_array().map(Vec::as_slice) {
                Some([a, b]) => Ok([point(a)?, point(b)?]),
                _ => Err(format!("{name}: a segment is not two points")),
            })
            .collect::<Result<Vec<_>, String>>()?;
        Expected::Output(Output {
            segments,
            segment_count: count("segment_count")?,
            closed_input_count: count("closed_input_count")?,
            skipped_open_count: count("skipped_open_count")?,
            passes: u32::try_from(count("passes")?).map_err(|e| e.to_string())?,
            pitch_mm: f64_at(o, "pitch_mm", f64::NAN)?,
            angles_deg: o
                .get("angles_deg")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{name}: no angles_deg"))?
                .iter()
                .map(|a| a.as_f64().ok_or_else(|| format!("{name}: bad angle")))
                .collect::<Result<_, _>>()?,
        })
    };
    Ok(Case { name, polylines: polys, params, expected })
}

#[cfg(test)]
mod tests {
    #[test]
    fn loads_all_46_cases() {
        let cases = super::load().unwrap();
        assert_eq!(cases.len(), 46);
        let bc = cases.iter().find(|c| c.name == "business_card_raster_saved_settings").unwrap();
        assert_eq!(bc.polylines.len(), 99);
    }
}

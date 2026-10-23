//! Hatch: the exact OM `raster.hatch_polylines` port (T3) and LightBurn Fill. WP0 stubs.

use crate::{Error, P2, Polyline};

/// Exact port of OM `raster.hatch_polylines` (T3: segment count and order exact, coordinates within 1e-9).
pub fn hatch_om(polys: &[Polyline], angle_deg: f64, pitch_mm: f64, passes: u32, angle_step_deg: f64, closure_tol_mm: f64) -> Result<OmHatch, Error> {
    let _ = (polys, angle_deg, pitch_mm, passes, angle_step_deg, closure_tol_mm);
    Err(Error::NotImplemented("openlaser_geom::hatch_om"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct OmHatch {
    pub segments: Vec<[P2; 2]>,
    pub closed_input_count: usize,
    pub skipped_open_count: usize,
    pub angles_deg: Vec<f64>,
}

/// LightBurn Fill. Pass i uses angle + i × angle_increment; `bidirectional` = OM boustrophedon (identical
/// segments to `hatch_om` for that pass); one-way flips every other row back; `cross_hatch` adds a +90° pass
/// after each pass. Even-odd over all closed inputs; open inputs are reported, never dropped silently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HatchParams {
    pub interval_mm: f64,
    pub angle_deg: f64,
    pub passes: u16,
    pub angle_increment_deg: f64,
    pub bidirectional: bool,
    pub cross_hatch: bool,
    pub closure_tol_mm: f64,
    pub max_lines: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchResult {
    pub passes: Vec<Vec<[P2; 2]>>,
    pub angles_deg: Vec<f64>,
    pub closed_used: usize,
    pub open_inputs: Vec<usize>,
}

pub fn hatch(polys: &[Polyline], p: &HatchParams) -> Result<HatchResult, Error> {
    let _ = (polys, p);
    Err(Error::NotImplemented("openlaser_geom::hatch"))
}

/// Even-odd point-in-region test over closed polylines (used by the frame ⊇ burn property test).
pub fn inside_even_odd(closed: &[Polyline], p: P2) -> bool {
    let _ = (closed, p);
    false
}

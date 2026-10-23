//! Geometry (SPEC §3.2; owned by WP1).
//!
//! Polylines in f64 millimetres (Y up), kurbo flattening, transforms, bounds, the clean-up operations (Auto-Join
//! as an exact OM `geometry.join_lines` port, Close Path, Delete Duplicates), the hatch generators (`hatch_om`,
//! the exact OM `raster.hatch_polylines` port checked by T3, and LightBurn Fill `hatch`), the even-odd region
//! test and the canvas `HitIndex`.
#![forbid(unsafe_code)]

mod hatch;
mod hit;
mod ops;
#[cfg(test)]
mod props;

pub use kurbo::{Affine, BezPath, PathEl, Point, Rect, Vec2};
pub use openlaser_types::emission::Bounds;
use serde::{Deserialize, Serialize};

pub use hatch::{HATCH_OM_MAX_LINES, HatchParams, HatchResult, OmHatch, hatch, hatch_om, inside_even_odd};
pub use hit::{HIT_MAX_COORD_MM, HitIndex};
pub use openlaser_types::ids::ObjectId;
pub use ops::{JoinReport, MAX_CURVE_SEGMENTS, MIN_FLATTEN_TOL_MM, bounds_of, close_paths, duplicates, flatten, join_lines, join_lines_report};

pub type P2 = [f64; 2];
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Polyline(pub Vec<P2>);

impl Polyline {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn length_mm(&self) -> f64 {
        self.0.windows(2).filter_map(|w| Some((w.first()?, w.get(1)?))).map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1])).sum()
    }

    pub fn bounds(&self) -> Option<Bounds> {
        bounds_of(std::slice::from_ref(self))
    }

    /// OM `is_closed`: at least 3 points and |first − last| <= tol.
    pub fn is_closed(&self, tol_mm: f64) -> bool {
        match (self.0.first(), self.0.last()) {
            (Some(a), Some(b)) if self.0.len() >= 3 => (a[0] - b[0]).hypot(a[1] - b[1]) <= tol_mm,
            _ => false,
        }
    }

    pub fn transformed(&self, a: &Affine) -> Polyline {
        Polyline(
            self.0
                .iter()
                .map(|p| {
                    let q = *a * Point::new(p[0], p[1]);
                    [q.x, q.y]
                })
                .collect(),
        )
    }

    pub fn all_finite(&self) -> bool {
        self.0.iter().all(|p| p[0].is_finite() && p[1].is_finite())
    }
}

/// OM raster closure tolerance.
pub const CLOSURE_TOL_MM: f64 = 0.05;
/// Pref `output.curveToleranceMm`.
pub const DEFAULT_FLATTEN_TOL_MM: f64 = 0.01;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("line interval must be > 0 (got {0})")]
    BadInterval(f64),
    #[error("non-finite coordinate")]
    NonFinite,
    #[error("hatch would produce {0} lines, above the {1} limit")]
    TooManyLines(usize, usize),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

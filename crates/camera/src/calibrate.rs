//! Camera↔bed calibration (phase 6b): N ≥ 3 point correspondences `{camera px ↔ bed mm}` fitted to a 2-D map.
//!
//! Three points fit an affine transform exactly; four or more fit a homography by normalised DLT — the
//! smallest eigenvector of `AᵀA` found by inverse iteration over a 9 × 9 Gaussian-elimination solve (both
//! written here in pure safe Rust — no external linear algebra). The inverse direction is the analytic
//! inverse of the fitted map, so [`Calibration::bed_to_px`] and [`Calibration::px_to_bed`] always agree.
//!
//! Everything degenerate is a typed [`Error::Calibrate`], never a panic: non-finite or duplicate points,
//! collinear triples, under-determined sets and singular fits are all refused.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Above this many pairs a fit is refused: a camera calibration never needs more, and the degenerate
/// checks are O(n³) in the pair count.
pub const MAX_PAIRS: usize = 64;

/// Two coordinates closer than this (px for the camera side, mm for the bed side) count as duplicates.
const DUPLICATE_EPS: f64 = 1e-6;

/// One point correspondence: `cam` is a position in the camera image in pixels, `bed` the same physical
/// point measured on the machine bed in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub cam: [f64; 2],
    pub bed: [f64; 2],
}

/// A fitted camera↔bed map. `fwd` maps bed mm → camera px, `inv` camera px → bed mm; `rms_px` is the
/// reprojection RMS error of `fwd` over the fitted pairs, in pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Calibration {
    /// Exact fit through three pairs: `fwd`/`inv` are row-major `[a b tx; c d ty]` affines.
    Affine { fwd: [f64; 6], inv: [f64; 6], rms_px: f64 },
    /// Least-squares fit through four or more pairs: `fwd`/`inv` are row-major 3 × 3 homographies.
    Homography { fwd: [f64; 9], inv: [f64; 9], rms_px: f64 },
}

impl Calibration {
    /// `"affine"` or `"homography"`, for status rows.
    pub fn kind(&self) -> &'static str {
        match self {
            Calibration::Affine { .. } => "affine",
            Calibration::Homography { .. } => "homography",
        }
    }

    /// The reprojection RMS error over the fitted pairs, in pixels.
    pub fn rms_error_px(&self) -> f64 {
        match self {
            Calibration::Affine { rms_px, .. } | Calibration::Homography { rms_px, .. } => *rms_px,
        }
    }

    /// bed mm → camera px. `None` on non-finite input or a projective point at infinity.
    pub fn bed_to_px(&self, bed: [f64; 2]) -> Option<[f64; 2]> {
        match self {
            Calibration::Affine { fwd, .. } => affine_apply(fwd, bed),
            Calibration::Homography { fwd, .. } => homography_apply(fwd, bed),
        }
    }

    /// camera px → bed mm. `None` on non-finite input or a projective point at infinity.
    pub fn px_to_bed(&self, px: [f64; 2]) -> Option<[f64; 2]> {
        match self {
            Calibration::Affine { inv, .. } => affine_apply(inv, px),
            Calibration::Homography { inv, .. } => homography_apply(inv, px),
        }
    }
}

/// A persisted calibration: the setup it belongs to (the camera the pairs were clicked on), the pairs it
/// was fitted from, the fit and when it was made. Machine-local setup data, never part of a project file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCalibration {
    /// The OS device index the pairs were clicked on, when a capture was open at fit time.
    pub device_index: Option<u32>,
    pub device_name: String,
    pub pairs: Vec<Pair>,
    pub calibration: Calibration,
    /// Wall-clock seconds since the Unix epoch at fit time (informational only).
    pub unix_secs: i64,
}

/// Where a [`StoredCalibration`] lives between runs (S24-style): writes are atomic and the reader ignores
/// a file that does not parse. Implemented by `openlaser-format`'s `DirCalibrationStore`; the engine holds
/// whatever the app injects and stays filesystem-free itself.
pub trait CalibrationStore: Send + Sync {
    /// The stored calibration, or `None` when the file is absent or unreadable.
    fn load(&self) -> Option<StoredCalibration>;
    fn save(&self, calibration: &StoredCalibration) -> std::result::Result<(), String>;
    /// Remove the stored calibration; an absent file is success.
    fn clear(&self) -> std::result::Result<(), String>;
}

/// Fit `pairs` to a [`Calibration`]: exact affine at 3 pairs, normalised-DLT homography at 4 or more.
/// Refuses (never clamps or silently proceeds): fewer than 3 or more than [`MAX_PAIRS`] pairs, non-finite
/// coordinates, duplicate points, collinear triples and singular results.
pub fn fit(pairs: &[Pair]) -> Result<Calibration> {
    if pairs.len() < 3 {
        return Err(Error::Calibrate(format!("at least 3 point pairs are needed, got {}", pairs.len())));
    }
    if pairs.len() > MAX_PAIRS {
        return Err(Error::Calibrate(format!("at most {MAX_PAIRS} point pairs, got {}", pairs.len())));
    }
    for (i, p) in pairs.iter().enumerate() {
        if !p.cam.iter().chain(p.bed.iter()).all(|v| v.is_finite()) {
            return Err(Error::Calibrate(format!("pair {i} contains a non-finite coordinate")));
        }
    }
    for (i, a) in pairs.iter().enumerate() {
        for (j, b) in pairs.iter().enumerate().skip(i + 1) {
            if dist(a.cam, b.cam) < DUPLICATE_EPS || dist(a.bed, b.bed) < DUPLICATE_EPS {
                return Err(Error::Calibrate(format!("pairs {i} and {j} are duplicates")));
            }
        }
    }
    if pairs.len() == 3 { fit_affine(pairs) } else { fit_homography(pairs) }
}

/// The exact affine through three pairs. Refuses a collinear triple on either side (the affine is then
/// under-determined or non-invertible).
fn fit_affine(pairs: &[Pair]) -> Result<Calibration> {
    let Some((p0, p1, p2)) = pairs.first().zip(pairs.get(1)).zip(pairs.get(2)).map(|((a, b), c)| (a.bed, b.bed, c.bed)) else {
        return Err(Error::Calibrate("at least 3 point pairs are needed".to_owned()));
    };
    let Some((q0, q1, q2)) = pairs.first().zip(pairs.get(1)).zip(pairs.get(2)).map(|((a, b), c)| (a.cam, b.cam, c.cam)) else {
        return Err(Error::Calibrate("at least 3 point pairs are needed".to_owned()));
    };
    let bx = [p1[0] - p0[0], p2[0] - p0[0]];
    let by = [p1[1] - p0[1], p2[1] - p0[1]];
    let det_b = bx[0] * by[1] - bx[1] * by[0];
    let scale_b = (bx[0] * bx[0] + by[0] * by[0]).max(bx[1] * bx[1] + by[1] * by[1]);
    if det_b.abs() <= 1e-9 * scale_b || !det_b.is_finite() {
        return Err(Error::Calibrate("the three bed points are collinear or coincident".to_owned()));
    }
    // A_lin = Q · B⁻¹ with B = [bx; by] (bed deltas as columns), Q = cam deltas as columns.
    let qx = [q1[0] - q0[0], q2[0] - q0[0]];
    let qy = [q1[1] - q0[1], q2[1] - q0[1]];
    let inv_det = 1.0 / det_b;
    let a = (qx[0] * by[1] - qx[1] * by[0]) * inv_det;
    let b = (qx[1] * bx[0] - qx[0] * bx[1]) * inv_det;
    let c = (qy[0] * by[1] - qy[1] * by[0]) * inv_det;
    let d = (qy[1] * bx[0] - qy[0] * bx[1]) * inv_det;
    let tx = q0[0] - a * p0[0] - b * p0[1];
    let ty = q0[1] - c * p0[0] - d * p0[1];
    let fwd = [a, b, tx, c, d, ty];
    let det_a = a * d - b * c;
    let norm_a = a * a + b * b + c * c + d * d;
    if det_a.abs() <= 1e-9 * norm_a || !det_a.is_finite() {
        return Err(Error::Calibrate("the three camera points are collinear or coincident".to_owned()));
    }
    let ia = d / det_a;
    let ib = -b / det_a;
    let ic = -c / det_a;
    let id = a / det_a;
    let inv = [ia, ib, -(ia * tx + ib * ty), ic, id, -(ic * tx + id * ty)];
    let rms = rms_error(&Calibration::Affine { fwd, inv, rms_px: 0.0 }, pairs);
    Ok(Calibration::Affine { fwd, inv, rms_px: rms })
}

/// Least-squares homography through four or more pairs: Hartley-normalised DLT. Refuses under-determined
/// configurations — the points must contain a spanning 4-subset with no three collinear on each side —
/// and singular results.
fn fit_homography(pairs: &[Pair]) -> Result<Calibration> {
    if !spans4(pairs.iter().map(|p| p.bed)) {
        return Err(Error::Calibrate("the bed points do not span the field (any 4 targets must not have 3 on one line)".to_owned()));
    }
    if !spans4(pairs.iter().map(|p| p.cam)) {
        return Err(Error::Calibrate("the camera points do not span the image (any 4 clicks must not have 3 on one line)".to_owned()));
    }
    // Hartley normalisation: centre each set, scale so the mean distance to the centroid is √2.
    let Some(bed_t) = similarity(pairs.iter().map(|p| p.bed)) else {
        return Err(Error::Calibrate("the bed points are degenerate".to_owned()));
    };
    let Some(cam_t) = similarity(pairs.iter().map(|p| p.cam)) else {
        return Err(Error::Calibrate("the camera points are degenerate".to_owned()));
    };
    // A·h = 0, one 2-row block per pair on the normalised points.
    let mut m = [[0.0f64; 9]; 9]; // AᵀA
    for pair in pairs {
        let [x, y] = affine_apply(&bed_t, pair.bed).unwrap_or([0.0; 2]);
        let [u, v] = affine_apply(&cam_t, pair.cam).unwrap_or([0.0; 2]);
        for row in [[-x, -y, -1.0, 0.0, 0.0, 0.0, u * x, u * y, u], [0.0, 0.0, 0.0, -x, -y, -1.0, v * x, v * y, v]] {
            for (i, ri) in row.iter().enumerate() {
                if let Some(mrow) = m.get_mut(i) {
                    for (v, rj) in mrow.iter_mut().zip(row.iter()) {
                        *v += ri * rj;
                    }
                }
            }
        }
    }
    let h = smallest_eigenvector(&m).ok_or_else(|| Error::Calibrate("the point configuration is singular".to_owned()))?;
    // Denormalise: H = T_cam⁻¹ · H_norm · T_bed.
    let cam_t_inv = affine_inverse(&cam_t).ok_or_else(|| Error::Calibrate("the camera points are degenerate".to_owned()))?;
    let mut h9 = [0.0f64; 9];
    for i in 0..3 {
        for j in 0..3 {
            // (T_cam⁻¹ · H_norm · T_bed)[i][j]
            let mut acc = 0.0;
            for r in 0..3 {
                for s in 0..3 {
                    acc += mat3(&cam_t_inv, i, r) * h.get(r * 3 + s).copied().unwrap_or(0.0) * mat3(&bed_t, s, j);
                }
            }
            if let Some(v) = h9.get_mut(i * 3 + j) {
                *v = acc;
            }
        }
    }
    let norm: f64 = h9.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !norm.is_finite() || norm <= f64::MIN_POSITIVE {
        return Err(Error::Calibrate("the fitted homography is degenerate".to_owned()));
    }
    for v in &mut h9 {
        *v /= norm;
    }
    let inv9 = homography_inverse(&h9).ok_or_else(|| Error::Calibrate("the fitted homography is not invertible".to_owned()))?;
    let rms = rms_error(&Calibration::Homography { fwd: h9, inv: inv9, rms_px: 0.0 }, pairs);
    if !rms.is_finite() {
        return Err(Error::Calibrate("the fitted homography is degenerate".to_owned()));
    }
    Ok(Calibration::Homography { fwd: h9, inv: inv9, rms_px: rms })
}

/// The reprojection RMS error of `cal`'s forward map over `pairs`, in pixels.
fn rms_error(cal: &Calibration, pairs: &[Pair]) -> f64 {
    let mut sum = 0.0;
    let mut n = 0usize;
    for pair in pairs {
        if let Some(px) = cal.bed_to_px(pair.bed) {
            sum += dist(px, pair.cam).powi(2);
            n += 1;
        }
    }
    if n == 0 { f64::NAN } else { (sum / n as f64).sqrt() }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

/// Does a point set contain a projective basis — some 4-subset with no three collinear? Without one the
/// homography is under-determined (all points on one line, or all-but-one on one line, leave a 1-parameter
/// family of solutions). Collinear triples inside a larger spanning set are fine.
fn spans4(points: impl Iterator<Item = [f64; 2]>) -> bool {
    let pts: Vec<[f64; 2]> = points.collect();
    let n = pts.len();
    if n < 4 {
        return false;
    }
    for a in 0..n {
        for b in (a + 1)..n {
            for c in (b + 1)..n {
                for d in (c + 1)..n {
                    let quad = [a, b, c, d].map(|i| pts.get(i).copied().unwrap_or([f64::NAN; 2]));
                    let any3 = (0..4).any(|drop| {
                        let tri: Vec<[f64; 2]> = quad.iter().enumerate().filter(|(i, _)| *i != drop).map(|(_, p)| *p).collect();
                        matches!(tri.as_slice(), [p, q, r] if collinear(*p, *q, *r))
                    });
                    if !any3 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Collinearity test: twice the triangle area relative to the longest side squared. Scale-free, so 1e-9 is
/// "on one line" for px and mm alike.
fn collinear(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let area2 = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs();
    let side = dist(a, b).max(dist(b, c)).max(dist(a, c));
    side <= 0.0 || area2 <= 1e-9 * side * side
}

/// Hartley similarity for a point set: `[sx 0 tx; 0 sy ty]` mapping the centroid to the origin and the mean
/// radius to √2. `None` when the set has zero spread (every point the same — duplicates are refused earlier,
/// so this only guards pathological input).
fn similarity(points: impl Iterator<Item = [f64; 2]>) -> Option<[f64; 6]> {
    let mut cx = 0.0;
    let mut cy = 0.0;
    let mut n = 0usize;
    let mut pts = Vec::new();
    for p in points {
        cx += p[0];
        cy += p[1];
        n += 1;
        pts.push(p);
    }
    if n == 0 {
        return None;
    }
    cx /= n as f64;
    cy /= n as f64;
    let mean: f64 = pts.iter().map(|p| dist(*p, [cx, cy])).sum::<f64>() / n as f64;
    if mean <= 0.0 || !mean.is_finite() {
        return None;
    }
    let s = std::f64::consts::SQRT_2 / mean;
    Some([s, 0.0, -s * cx, 0.0, s, -s * cy])
}

/// Element `[r][c]` of a row-major `[a b tx; c d ty]` affine.
fn mat3(t: &[f64; 6], r: usize, c: usize) -> f64 {
    match (r, c) {
        (0, 0) => t.first().copied().unwrap_or(0.0),
        (0, 1) => t.get(1).copied().unwrap_or(0.0),
        (0, 2) => t.get(2).copied().unwrap_or(0.0),
        (1, 0) => t.get(3).copied().unwrap_or(0.0),
        (1, 1) => t.get(4).copied().unwrap_or(0.0),
        (1, 2) => t.get(5).copied().unwrap_or(0.0),
        (2, 2) => 1.0,
        _ => 0.0,
    }
}

fn affine_apply(t: &[f64; 6], p: [f64; 2]) -> Option<[f64; 2]> {
    if !p.iter().all(|v| v.is_finite()) {
        return None;
    }
    let [a, b, tx, c, d, ty] = *t;
    let out = [a * p[0] + b * p[1] + tx, c * p[0] + d * p[1] + ty];
    out.iter().all(|v| v.is_finite()).then_some(out)
}

fn affine_inverse(t: &[f64; 6]) -> Option<[f64; 6]> {
    let [ta, tb, tx, tc, td, ty] = *t;
    let det = ta * td - tb * tc;
    let norm = ta * ta + tb * tb + tc * tc + td * td;
    if !det.is_finite() || det.abs() <= 1e-30 * norm.max(1e-300) {
        return None;
    }
    let (a, b, c, d) = (td / det, -tb / det, -tc / det, ta / det);
    Some([a, b, -(a * tx + b * ty), c, d, -(c * tx + d * ty)])
}

fn homography_apply(h: &[f64; 9], p: [f64; 2]) -> Option<[f64; 2]> {
    if !p.iter().all(|v| v.is_finite()) {
        return None;
    }
    let [h0, h1, h2, h3, h4, h5, h6, h7, h8] = *h;
    let w = h6 * p[0] + h7 * p[1] + h8;
    if !(w.is_finite() && w.abs() > 1e-300) {
        return None;
    }
    let out = [(h0 * p[0] + h1 * p[1] + h2) / w, (h3 * p[0] + h4 * p[1] + h5) / w];
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// The adjugate inverse of a row-major 3 × 3 homography. `None` when the determinant is degenerate
/// relative to the matrix norm.
fn homography_inverse(h: &[f64; 9]) -> Option<[f64; 9]> {
    let [h0, h1, h2, h3, h4, h5, h6, h7, h8] = *h;
    let det = h0 * (h4 * h8 - h5 * h7) - h1 * (h3 * h8 - h5 * h6) + h2 * (h3 * h7 - h4 * h6);
    let norm: f64 = h.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !det.is_finite() || !norm.is_finite() || det.abs() <= 1e-12 * norm.powi(3).max(1e-300) {
        return None;
    }
    let inv = [
        (h4 * h8 - h5 * h7) / det,
        (h2 * h7 - h1 * h8) / det,
        (h1 * h5 - h2 * h4) / det,
        (h5 * h6 - h3 * h8) / det,
        (h0 * h8 - h2 * h6) / det,
        (h2 * h3 - h0 * h5) / det,
        (h3 * h7 - h4 * h6) / det,
        (h1 * h6 - h0 * h7) / det,
        (h0 * h4 - h1 * h3) / det,
    ];
    inv.iter().all(|v| v.is_finite()).then_some(inv)
}

/// The smallest eigenvector of a symmetric positive semidefinite 9 × 9 matrix, by inverse iteration:
/// repeatedly solve `(M + σI) x = h` and normalise — the iterates converge on the eigenvector of the
/// smallest eigenvalue. Nine seed axes are tried and the best Rayleigh quotient wins, so a degenerate
/// direction in one seed can never lose the answer. `None` when every solve is singular or non-finite.
fn smallest_eigenvector(m: &[[f64; 9]; 9]) -> Option<[f64; 9]> {
    let trace: f64 = m.iter().enumerate().map(|(i, row)| row.get(i).copied().unwrap_or(0.0)).sum();
    if !trace.is_finite() {
        return None;
    }
    let sigma = 1e-12 * (trace / 9.0).abs() + 1e-300;
    let mut reg = *m;
    for (i, row) in reg.iter_mut().enumerate() {
        if let Some(v) = row.get_mut(i) {
            *v += sigma;
        }
    }
    let mut best: Option<([f64; 9], f64)> = None;
    for seed in 0..9 {
        let mut h = [0.0f64; 9];
        if let Some(v) = h.get_mut(seed) {
            *v = 1.0;
        }
        for _ in 0..64 {
            let Some(x) = solve9(&reg, &h) else { break };
            let norm: f64 = x.iter().map(|v| v * v).sum::<f64>().sqrt();
            if !norm.is_finite() || norm <= f64::MIN_POSITIVE {
                break;
            }
            h = x;
            for v in &mut h {
                *v /= norm;
            }
        }
        // Rayleigh quotient hᵀ M h / hᵀh — the DLT residual this vector would have.
        let mut mh = [0.0f64; 9];
        for (i, row) in m.iter().enumerate() {
            let dot: f64 = row.iter().zip(h.iter()).map(|(a, b)| a * b).sum();
            if let Some(v) = mh.get_mut(i) {
                *v = dot;
            }
        }
        let rho: f64 = h.iter().zip(mh.iter()).map(|(a, b)| a * b).sum::<f64>() / h.iter().map(|v| v * v).sum::<f64>().max(1e-300);
        if rho.is_finite() && best.as_ref().is_none_or(|(_, r)| rho < *r) {
            best = Some((h, rho));
        }
    }
    best.map(|(h, _)| h)
}

/// Gaussian elimination with partial pivoting on a 9 × 9 system. `None` when a pivot is too small to
/// divide by (a singular or numerically singular matrix) — never divides by ~0, never panics.
fn solve9(a: &[[f64; 9]; 9], b: &[f64; 9]) -> Option<[f64; 9]> {
    let mut m = *a;
    let mut x = *b;
    let at = |m: &[[f64; 9]; 9], r: usize, c: usize| m.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0);
    for col in 0..9 {
        // Partial pivot.
        let mut pivot = col;
        for row in col..9 {
            if at(&m, row, col).abs() > at(&m, pivot, col).abs() {
                pivot = row;
            }
        }
        let mag = at(&m, pivot, col).abs();
        let scale: f64 = m.get(col).map(|row| row.iter().map(|v| v.abs()).sum::<f64>()).unwrap_or(0.0).max(1e-300);
        if !mag.is_finite() || mag <= 1e-12 * scale {
            return None;
        }
        if pivot != col {
            m.swap(col, pivot);
            x.swap(col, pivot);
        }
        let pivot_row = m.get(col).copied().unwrap_or([0.0; 9]);
        let pv = pivot_row.get(col).copied().unwrap_or(0.0);
        let x_col = x.get(col).copied().unwrap_or(0.0);
        for (xr, row) in x.iter_mut().skip(col + 1).zip(m.iter_mut().skip(col + 1)) {
            let f = row.get(col).copied().unwrap_or(0.0) / pv;
            for (v, s) in row.iter_mut().skip(col).zip(pivot_row.iter().skip(col)) {
                *v -= f * s;
            }
            *xr -= f * x_col;
        }
    }
    let mut out = [0.0f64; 9];
    for col in (0..9).rev() {
        let row = m.get(col)?;
        let mut acc = x.get(col).copied().unwrap_or(0.0);
        for (mv, ov) in row.iter().skip(col + 1).zip(out.iter().skip(col + 1)) {
            acc -= mv * ov;
        }
        let d = row.get(col).copied().unwrap_or(0.0);
        if !d.is_finite() || d.abs() <= 1e-300 {
            return None;
        }
        if let Some(v) = out.get_mut(col) {
            *v = acc / d;
        }
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bed_grid() -> Vec<[f64; 2]> {
        let mut pts = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                pts.push([(x as f64 - 1.0) * 50.0, (y as f64 - 1.0) * 50.0]);
            }
        }
        pts
    }

    /// A known projective transform: a plane viewed obliquely (strong perspective, like a bench camera).
    fn project(p: [f64; 2]) -> [f64; 2] {
        let h = [1.1, 0.25, 640.0, -0.05, 0.9, 340.0, 0.0018, -0.0011, 1.0];
        homography_apply(&h, p).unwrap()
    }

    #[test]
    fn affine_is_exact_through_three_points() {
        // bed → px: scale 3.2/2.7, swap-ish rotation, translation.
        let map = |p: [f64; 2]| [3.2 * p[0] + 0.4 * p[1] + 512.0, 0.3 * p[0] + 2.7 * p[1] + 384.0];
        let pairs: Vec<Pair> = [[-50.0, -40.0], [30.0, -10.0], [10.0, 60.0]].iter().map(|&b| Pair { cam: map(b), bed: b }).collect();
        let cal = fit(&pairs).unwrap();
        assert_eq!(cal.kind(), "affine");
        assert!(cal.rms_error_px() < 1e-9, "{}", cal.rms_error_px());
        for pair in &pairs {
            let px = cal.bed_to_px(pair.bed).unwrap();
            assert!(dist(px, pair.cam) < 1e-9);
            let back = cal.px_to_bed(px).unwrap();
            assert!(dist(back, pair.bed) < 1e-9);
        }
        // And an arbitrary point round-trips.
        let p = [12.345, -67.89];
        assert!(dist(cal.px_to_bed(cal.bed_to_px(p).unwrap()).unwrap(), p) < 1e-9);
    }

    #[test]
    fn homography_recovers_a_known_projective_map() {
        let pairs: Vec<Pair> = bed_grid().iter().map(|&b| Pair { cam: project(b), bed: b }).collect();
        let cal = fit(&pairs).unwrap();
        assert_eq!(cal.kind(), "homography");
        assert!(cal.rms_error_px() < 1e-6, "{}", cal.rms_error_px());
        // Round-trip anywhere in the field, far off the fitted grid too.
        for p in [[-90.0, 88.0], [0.0, 0.0], [55.5, -42.25], [97.0, 97.0]] {
            let back = cal.px_to_bed(cal.bed_to_px(p).unwrap()).unwrap();
            assert!(dist(back, p) < 1e-6, "{p:?} -> {back:?}");
            assert!(dist(cal.bed_to_px(p).unwrap(), project(p)) < 1e-3);
        }
    }

    #[test]
    fn homography_four_points_minimum() {
        let pts = [[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0]];
        let pairs: Vec<Pair> = pts.iter().map(|&b| Pair { cam: project(b), bed: b }).collect();
        let cal = fit(&pairs).unwrap();
        assert!(cal.rms_error_px() < 1e-6);
        for pair in &pairs {
            assert!(dist(cal.bed_to_px(pair.bed).unwrap(), pair.cam) < 1e-4);
        }
    }

    #[test]
    fn noisier_measurements_report_rms() {
        let mut pairs: Vec<Pair> = bed_grid().iter().map(|&b| Pair { cam: project(b), bed: b }).collect();
        // 1 px of deterministic noise on one point: the fit stays sane and the rms reports it.
        if let Some(p) = pairs.get_mut(4) {
            p.cam[0] += 1.0;
        }
        let cal = fit(&pairs).unwrap();
        assert!(cal.rms_error_px() > 0.01 && cal.rms_error_px() < 0.5, "{}", cal.rms_error_px());
    }

    #[test]
    fn refuses_bad_inputs() {
        let good = |b: [f64; 2]| Pair { cam: project(b), bed: b };
        // Too few.
        assert!(matches!(fit(&[good([-10.0, 0.0]), good([10.0, 5.0])]), Err(Error::Calibrate(_))));
        // Non-finite.
        let mut bad = vec![good([-10.0, 0.0]), good([10.0, 5.0]), good([0.0, 30.0])];
        bad[0].cam[0] = f64::NAN;
        assert!(fit(&bad).is_err());
        bad[0].cam[0] = 0.0;
        bad[1].bed[1] = f64::INFINITY;
        assert!(fit(&bad).is_err());
        // Duplicates (either side).
        let mut dup = vec![good([-10.0, 0.0]), good([10.0, 5.0]), good([0.0, 30.0])];
        dup[2].cam = dup[0].cam;
        assert!(fit(&dup).is_err());
        let mut dup = vec![good([-10.0, 0.0]), good([10.0, 5.0]), good([0.0, 30.0])];
        dup[2].bed = dup[1].bed;
        assert!(fit(&dup).is_err());
        // Collinear affine triple.
        let line: Vec<Pair> = [[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]].iter().map(|&b| good(b)).collect();
        assert!(fit(&line).is_err());
        // Collinear triple inside a larger set (under-determined homography).
        let mut spread: Vec<Pair> = bed_grid().iter().map(|&b| good(b)).collect();
        spread.truncate(4);
        spread[2] = good([0.0, -50.0]); // bed 0,1,2 now collinear
        assert!(fit(&spread).is_err());
        // Empty.
        assert!(fit(&[]).is_err());
    }

    #[test]
    fn stored_calibration_roundtrips_json() {
        let pairs: Vec<Pair> = bed_grid().iter().map(|&b| Pair { cam: project(b), bed: b }).collect();
        let cal = fit(&pairs).unwrap();
        let stored = StoredCalibration { device_index: Some(0), device_name: "Desk Cam".into(), pairs, calibration: cal, unix_secs: 1_760_000_000 };
        let json = serde_json::to_string(&stored).unwrap();
        let back: StoredCalibration = serde_json::from_str(&json).unwrap();
        assert_eq!(stored, back);
    }

    /// A memory `CalibrationStore` for engine/UI tests.
    #[test]
    fn memory_store_shape() {
        struct Mem(std::sync::Mutex<Option<StoredCalibration>>);
        impl CalibrationStore for Mem {
            fn load(&self) -> Option<StoredCalibration> {
                self.0.lock().ok().and_then(|g| g.clone())
            }
            fn save(&self, c: &StoredCalibration) -> std::result::Result<(), String> {
                if let Ok(mut g) = self.0.lock() {
                    *g = Some(c.clone());
                }
                Ok(())
            }
            fn clear(&self) -> std::result::Result<(), String> {
                if let Ok(mut g) = self.0.lock() {
                    *g = None;
                }
                Ok(())
            }
        }
        let store = Mem(std::sync::Mutex::new(None));
        assert!(store.load().is_none());
        let pairs: Vec<Pair> = bed_grid().iter().map(|&b| Pair { cam: project(b), bed: b }).collect();
        let stored = StoredCalibration {
            device_index: None,
            device_name: String::new(),
            pairs,
            calibration: fit(&bed_grid().iter().map(|&b| Pair { cam: project(b), bed: b }).collect::<Vec<_>>()).unwrap(),
            unix_secs: 0,
        };
        store.save(&stored).unwrap();
        assert_eq!(store.load().as_ref().map(|s| s.pairs.len()), Some(9));
        store.clear().unwrap();
        assert!(store.load().is_none());
    }
}

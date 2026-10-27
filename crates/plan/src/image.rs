//! Box-filter resampling and maximal on/off runs, in image-local millimetres.
use openlaser_doc::{BitmapData, ImageCut};
use openlaser_geom::Polyline;

use crate::MAX_SECTION_POINTS;
use crate::dither::dither;

pub const MIN_IMAGE_INTERVAL_MM: f64 = 0.01;
pub const MAX_IMAGE_INTERVAL_MM: f64 = 2.0;
pub const MAX_IMAGE_CELLS: usize = 25_000_000;

pub(crate) fn runs(bitmap: &BitmapData, cut: &ImageCut) -> Result<Vec<Polyline>, String> {
    let interval = cut.interval.get();
    if !interval.is_finite() || !(MIN_IMAGE_INTERVAL_MM..=MAX_IMAGE_INTERVAL_MM).contains(&interval) {
        return Err(format!("image Line Interval {interval} mm must be finite and in {MIN_IMAGE_INTERVAL_MM}..={MAX_IMAGE_INTERVAL_MM} mm"));
    }
    let [width, height] = bitmap.size_mm.map(f64::abs);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("image dimensions must be finite and above 0 mm".into());
    }
    let cols_f = (width / interval).ceil();
    let rows_f = (height / interval).ceil();
    if cols_f < 1.0 || rows_f < 1.0 {
        return Err("image dimensions are too small for a scan cell".into());
    }
    let count = cols_f * rows_f;
    if !count.is_finite() || count > MAX_IMAGE_CELLS as f64 {
        return Err(format!("image needs {count:.0} cells, above {MAX_IMAGE_CELLS}; raise the interval"));
    }
    let (cols, rows) = (cols_f as usize, rows_f as usize);
    let [sw, sh] = bitmap.px.map(|n| n as usize);
    if sw == 0 || sh == 0 || sw.checked_mul(sh) != Some(bitmap.gray.len()) {
        return Err("image grey pixels do not match its source dimensions".into());
    }
    let mut gray = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        let y0 = (r as f64 * interval / height * sh as f64).min(sh as f64);
        let y1 = (((r + 1) as f64 * interval).min(height) / height * sh as f64).min(sh as f64);
        for c in 0..cols {
            let x0 = (c as f64 * interval / width * sw as f64).min(sw as f64);
            let x1 = (((c + 1) as f64 * interval).min(width) / width * sw as f64).min(sw as f64);
            let mut sum = 0.0;
            let mut area = 0.0;
            for sy in y0.floor() as usize..(y1.ceil() as usize).min(sh) {
                let dy = (y1.min((sy + 1) as f64) - y0.max(sy as f64)).max(0.0);
                for sx in x0.floor() as usize..(x1.ceil() as usize).min(sw) {
                    let dx = (x1.min((sx + 1) as f64) - x0.max(sx as f64)).max(0.0);
                    let Some(&v) = bitmap.gray.get(sy * sw + sx) else { return Err("missing source image pixel".into()) };
                    sum += f64::from(v) * dx * dy;
                    area += dx * dy;
                }
            }
            if area <= 0.0 || !area.is_finite() {
                return Err("image scan cell has no source area; adjust dimensions or interval".into());
            }
            gray.push((sum / area).round() as u8);
        }
    }
    let burn = dither(&gray, cols, rows, cut.mode, cut.threshold, cut.negative)?;
    let mut paths = Vec::new();
    for (r, row) in burn.chunks_exact(cols).enumerate() {
        // The centre of the row's band clipped to the rectangle: a short final row stays inside the image (and so
        // inside the frame) instead of being scanned at a nominal centre below it.
        let top = height / 2.0 - r as f64 * interval;
        let bottom = (height / 2.0 - (r + 1) as f64 * interval).max(-height / 2.0);
        let y = (top + bottom) / 2.0;
        let row_start = paths.len();
        let mut start = None;
        // One sentinel white cell closes a run touching the right edge.
        for (c, on) in row.iter().copied().chain(std::iter::once(false)).enumerate() {
            if on {
                start.get_or_insert(c);
            } else if let Some(i0) = start.take() {
                if paths.len() >= MAX_SECTION_POINTS / 2 {
                    return Err(format!("image runs exceed the {MAX_SECTION_POINTS} section-point budget; raise the interval"));
                }
                let x0 = -width / 2.0 + i0 as f64 * interval;
                let x1 = (-width / 2.0 + c as f64 * interval).min(width / 2.0);
                let points = if cut.bidirectional && r % 2 == 1 { vec![[x1, y], [x0, y]] } else { vec![[x0, y], [x1, y]] };
                paths.push(Polyline(points));
            }
        }
        if cut.bidirectional
            && r % 2 == 1
            && let Some(row_paths) = paths.get_mut(row_start..)
        {
            row_paths.reverse();
        }
    }
    Ok(paths)
}

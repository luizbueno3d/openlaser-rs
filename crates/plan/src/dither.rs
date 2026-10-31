//! Deterministic on/off dithering. Row zero is the top; diffusion always scans left to right.
use openlaser_doc::ImageMode;

// (column offset, row offset, numerator), followed by the kernel's divisor.
const FLOYD: &[(isize, usize, i32)] = &[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)];
const JARVIS: &[(isize, usize, i32)] =
    &[(1, 0, 7), (2, 0, 5), (-2, 1, 3), (-1, 1, 5), (0, 1, 7), (1, 1, 5), (2, 1, 3), (-2, 2, 1), (-1, 2, 3), (0, 2, 5), (1, 2, 3), (2, 2, 1)];
const STUCKI: &[(isize, usize, i32)] =
    &[(1, 0, 8), (2, 0, 4), (-2, 1, 2), (-1, 1, 4), (0, 1, 8), (1, 1, 4), (2, 1, 2), (-2, 2, 1), (-1, 2, 2), (0, 2, 4), (1, 2, 2), (2, 2, 1)];
const ATKINSON: &[(isize, usize, i32)] = &[(1, 0, 1), (2, 0, 1), (-1, 1, 1), (0, 1, 1), (1, 1, 1), (0, 2, 1)];

/// Midpoints of the 64 Bayer bins, scaled to 0..255 (2, 6, …, 254).
fn bayer(x: usize, y: usize) -> i32 {
    let mut rank = 0;
    for bit in 0..3 {
        let xb = (x >> bit) & 1;
        let yb = (y >> bit) & 1;
        rank += (((xb ^ yb) * 2 + yb) << (2 * (2 - bit))) as i32;
    }
    rank * 4 + 2
}

/// Rejects inconsistent dimensions rather than dropping pixels. Diffusion uses 8 fractional bits, integer
/// division truncating toward zero, a fixed 128 quantiser, and discards only error beyond the image boundary.
pub(crate) fn dither(gray: &[u8], w: usize, h: usize, mode: ImageMode, threshold: u8, negative: bool) -> Result<Vec<bool>, &'static str> {
    if w == 0 || h == 0 || w.checked_mul(h) != Some(gray.len()) {
        return Err("image grey grid does not match its dimensions");
    }
    let value = |v: u8| i32::from(if negative { 255 - v } else { v });
    if mode == ImageMode::Threshold {
        return Ok(gray.iter().map(|&v| value(v) < i32::from(threshold)).collect());
    }
    if mode == ImageMode::Ordered {
        return Ok(gray.iter().enumerate().map(|(i, &v)| value(v) < bayer(i % w, i / w)).collect());
    }
    let (kernel, divisor) = match mode {
        ImageMode::Jarvis => (JARVIS, 48),
        ImageMode::FloydSteinberg => (FLOYD, 16),
        ImageMode::Stucki => (STUCKI, 42),
        ImageMode::Atkinson => (ATKINSON, 8),
        ImageMode::Threshold | ImageMode::Ordered => return Err("invalid diffusion mode"),
    };
    let mut work: Vec<i32> = gray.iter().map(|&v| value(v) * 256).collect();
    let mut out = Vec::with_capacity(gray.len());
    for i in 0..gray.len() {
        let Some(&v) = work.get(i) else { return Err("missing diffusion pixel") };
        let burn = v < 128 * 256;
        out.push(burn);
        let error = v - if burn { 0 } else { 255 * 256 };
        let (x, y) = (i % w, i / w);
        for &(dx, dy, weight) in kernel {
            let Some(nx) = x.checked_add_signed(dx).filter(|&n| n < w) else { continue };
            let Some(ny) = y.checked_add(dy).filter(|&n| n < h) else { continue };
            if let Some(pixel) = work.get_mut(ny * w + nx) {
                *pixel += error * weight / divisor;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MODES: [ImageMode; 6] =
        [ImageMode::Threshold, ImageMode::Jarvis, ImageMode::FloydSteinberg, ImageMode::Stucki, ImageMode::Atkinson, ImageMode::Ordered];

    #[test]
    fn extremes_negative_and_determinism() {
        for mode in MODES {
            for (v, burn) in [(0, true), (255, false)] {
                assert_eq!(dither(&[v; 256], 16, 16, mode, 128, false).unwrap(), vec![burn; 256]);
                assert_eq!(dither(&[v; 256], 16, 16, mode, 128, true).unwrap(), vec![!burn; 256]);
            }
            let gray: Vec<u8> = (0..256).map(|i| i as u8).collect();
            let a = dither(&gray, 16, 16, mode, 128, false).unwrap();
            assert_eq!(a, dither(&gray, 16, 16, mode, 128, false).unwrap());
            let inverse: Vec<u8> = gray.iter().map(|&v| 255 - v).collect();
            assert_eq!(a, dither(&inverse, 16, 16, mode, 128, true).unwrap());
        }
    }

    #[test]
    fn half_gray_has_half_coverage() {
        for mode in MODES.into_iter().skip(1) {
            let grid = dither(&vec![128; 256 * 256], 256, 256, mode, 128, false).unwrap();
            let fraction = grid.iter().filter(|&&v| v).count() as f64 / grid.len() as f64;
            assert!((fraction - 0.5).abs() <= 0.02, "{mode:?}: {fraction}");
        }
    }

    #[test]
    fn threshold_boundary_and_bad_dimensions() {
        assert_eq!(dither(&[127, 128, 129], 3, 1, ImageMode::Threshold, 128, false).unwrap(), vec![true, false, false]);
        assert!(dither(&[0], usize::MAX, 2, ImageMode::Jarvis, 128, false).is_err());
        assert!(dither(&[0], 0, 1, ImageMode::Threshold, 128, false).is_err());
    }
}

#[cfg(test)]
mod golden_tests {
    use super::*;
    #[test]
    fn golden_threshold() {
        let gray = [0, 36, 73, 109, 146, 182, 219, 255].repeat(4);
        let bits = dither(&gray, 8, 4, ImageMode::Threshold, 128, false).unwrap();
        let got = bits.chunks(8).map(|row| row.iter().map(|&b| if b { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(got, "####....\n####....\n####....\n####....");
    }
    #[test]
    fn golden_jarvis() {
        let gray = [0, 36, 73, 109, 146, 182, 219, 255].repeat(4);
        let bits = dither(&gray, 8, 4, ImageMode::Jarvis, 128, false).unwrap();
        let got = bits.chunks(8).map(|row| row.iter().map(|&b| if b { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(got, "####....\n###.....\n####.#..\n###.#...");
    }
    #[test]
    fn golden_floydsteinberg() {
        let gray = [0, 36, 73, 109, 146, 182, 219, 255].repeat(4);
        let bits = dither(&gray, 8, 4, ImageMode::FloydSteinberg, 128, false).unwrap();
        let got = bits.chunks(8).map(|row| row.iter().map(|&b| if b { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(got, "###.#...\n###.#...\n##.#....\n###.#.#.");
    }
    #[test]
    fn golden_stucki() {
        let gray = [0, 36, 73, 109, 146, 182, 219, 255].repeat(4);
        let bits = dither(&gray, 8, 4, ImageMode::Stucki, 128, false).unwrap();
        let got = bits.chunks(8).map(|row| row.iter().map(|&b| if b { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(got, "####....\n###..#..\n###.#...\n###.....");
    }
    #[test]
    fn golden_atkinson() {
        let gray = [0, 36, 73, 109, 146, 182, 219, 255].repeat(4);
        let bits = dither(&gray, 8, 4, ImageMode::Atkinson, 128, false).unwrap();
        let got = bits.chunks(8).map(|row| row.iter().map(|&b| if b { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(got, "####....\n###.....\n###.#...\n####....");
    }
    #[test]
    fn golden_ordered() {
        let gray = [0, 36, 73, 109, 146, 182, 219, 255].repeat(4);
        let bits = dither(&gray, 8, 4, ImageMode::Ordered, 128, false).unwrap();
        let got = bits.chunks(8).map(|row| row.iter().map(|&b| if b { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(got, "##.#....\n###.#.#.\n##.#.#..\n###.#...");
    }
}

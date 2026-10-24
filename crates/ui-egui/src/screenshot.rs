//! Bounded PNG screenshots without adding an image dependency to the shell.
const PNG_LIMIT: usize = 5 * 1024 * 1024;

pub fn encode(image: &egui::ColorImage, max_side: u32) -> Result<serde_json::Value, String> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || max_side == 0 || max_side > 2048 || width.checked_mul(height) != Some(image.pixels.len()) {
        return Err("invalid screenshot dimensions".into());
    }
    let scale = (max_side as f64 / width.max(height) as f64).min(1.0);
    let w = ((width as f64 * scale).floor() as usize).max(1);
    let h = ((height as f64 * scale).floor() as usize).max(1);
    w.checked_mul(4)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(h))
        .filter(|n| *n + 1024 < PNG_LIMIT)
        .ok_or("screenshot exceeds 5 MiB; request a smaller maxSide")?;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let source = (y * height / h).checked_mul(width).and_then(|n| n.checked_add(x * width / w)).ok_or("screenshot dimensions overflow")?;
            pixels.push(*image.pixels.get(source).ok_or("missing screenshot pixel")?);
        }
    }
    let png = png_bytes(&egui::ColorImage::new([w, h], pixels))?;
    Ok(serde_json::json!({"mime":"image/png","pngBase64":base64(&png),"width":w,"height":h}))
}

/// An uncompressed (stored-deflate) RGBA PNG, at most 5 MiB.
pub fn png_bytes(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let [w, h] = image.size;
    if w == 0 || h == 0 || w.checked_mul(h) != Some(image.pixels.len()) {
        return Err("invalid image dimensions".into());
    }
    let size = w.checked_mul(4).and_then(|n| n.checked_add(1)).and_then(|n| n.checked_mul(h)).filter(|n| *n + 1024 < PNG_LIMIT).ok_or("image exceeds 5 MiB")?;
    let mut raw = Vec::with_capacity(size);
    for row in image.pixels.chunks(w) {
        raw.push(0);
        for color in row {
            raw.extend(color.to_srgba_unmultiplied());
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let count = raw.len().div_ceil(65535);
    for (i, block) in raw.chunks(65535).enumerate() {
        zlib.push(u8::from(i + 1 == count));
        let n = u16::try_from(block.len()).map_err(|_| "PNG block too large")?;
        zlib.extend(n.to_le_bytes());
        zlib.extend((!n).to_le_bytes());
        zlib.extend(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend(((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend(u32::try_from(w).map_err(|_| "image too wide")?.to_be_bytes());
    header.extend(u32::try_from(h).map_err(|_| "image too tall")?.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header)?;
    chunk(&mut png, b"IDAT", &zlib)?;
    chunk(&mut png, b"IEND", &[])?;
    if png.len() > PNG_LIMIT {
        return Err("image exceeds 5 MiB".into());
    }
    Ok(png)
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}
fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) -> Result<(), String> {
    png.extend(u32::try_from(data.len()).map_err(|_| "PNG chunk too large")?.to_be_bytes());
    let start = png.len();
    png.extend(kind);
    png.extend(data);
    let bytes = png.get(start..).ok_or("invalid PNG chunk")?;
    let crc = crc32(bytes);
    png.extend(crc.to_be_bytes());
    Ok(())
}
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for block in bytes.chunks(3) {
        let a = block.first().copied().unwrap_or(0);
        let b = block.get(1).copied().unwrap_or(0);
        let c = block.get(2).copied().unwrap_or(0);
        for (index, present) in [(a >> 2, true), (((a & 3) << 4) | (b >> 4), true), (((b & 15) << 2) | (c >> 6), block.len() > 1), (c & 63, block.len() > 2)] {
            result.push(if present { char::from(ALPHABET.get(usize::from(index)).copied().unwrap_or(b'A')) } else { '=' });
        }
    }
    result
}

/// Test-only CPU rasteriser of egui's tessellated output, for review screenshots without a GPU (kittest's `wgpu`
/// feature is not in this workspace). Premultiplied-alpha "over" blending in gamma space, nearest texel sampling.
#[cfg(test)]
pub(crate) mod tests_raster {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    pub(crate) type Textures = HashMap<egui::TextureId, egui::ColorImage>;

    /// A kittest renderer that only records texture uploads (font atlas, icons).
    pub(crate) struct Recorder(pub Arc<Mutex<Textures>>);

    impl egui_kittest::TestRenderer for Recorder {
        fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
            let mut delta = std::mem::take(delta);
            let (set, free) = (std::mem::take(&mut delta.set), std::mem::take(&mut delta.free));
            let Ok(mut textures) = self.0.lock() else { return };
            for (id, change) in set.into_iter().flat_map(|(id, changes)| changes.into_iter().map(move |c| (id, c))) {
                let egui::ImageData::Color(image) = change.image;
                match change.pos {
                    None => {
                        textures.insert(id, (*image).clone());
                    }
                    Some([x0, y0]) => {
                        if let Some(target) = textures.get_mut(&id) {
                            let [tw, _] = target.size;
                            for y in 0..image.size[1] {
                                for x in 0..image.size[0] {
                                    if let (Some(dst), Some(src)) = (target.pixels.get_mut((y0 + y) * tw + x0 + x), image.pixels.get(y * image.size[0] + x)) {
                                        *dst = *src;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for id in free {
                textures.remove(&id);
            }
        }
    }

    fn mul(a: egui::Color32, b: egui::Color32) -> [f32; 4] {
        let f = |x: u8, y: u8| f32::from(x) * f32::from(y) / 255.0;
        [f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a())]
    }

    pub(crate) fn rasterize(primitives: &[egui::ClippedPrimitive], textures: &Textures, size: egui::Vec2, ppp: f32) -> egui::ColorImage {
        let (w, h) = ((size.x * ppp) as usize, (size.y * ppp) as usize);
        let mut px = vec![[0.0f32, 0.0, 0.0, 255.0]; w * h];
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else { continue };
            let clip = primitive.clip_rect;
            let (cx0, cy0) = ((clip.min.x * ppp).max(0.0) as usize, (clip.min.y * ppp).max(0.0) as usize);
            let (cx1, cy1) = (((clip.max.x * ppp).ceil() as usize).min(w), ((clip.max.y * ppp).ceil() as usize).min(h));
            let texture = textures.get(&mesh.texture_id);
            for tri in mesh.indices.as_chunks::<3>().0 {
                let v: Vec<&egui::epaint::Vertex> = tri.iter().filter_map(|i| mesh.vertices.get(*i as usize)).collect();
                let [a, b, c] = [v[0], v[1], v[2]];
                let (pa, pb, pc) = (a.pos * ppp, b.pos * ppp, c.pos * ppp);
                let area = (pb.x - pa.x) * (pc.y - pa.y) - (pb.y - pa.y) * (pc.x - pa.x);
                if area.abs() < 1e-6 {
                    continue;
                }
                let x0 = (pa.x.min(pb.x).min(pc.x).floor().max(cx0 as f32)) as usize;
                let x1 = (pa.x.max(pb.x).max(pc.x).ceil().min(cx1 as f32)) as usize;
                let y0 = (pa.y.min(pb.y).min(pc.y).floor().max(cy0 as f32)) as usize;
                let y1 = (pa.y.max(pb.y).max(pc.y).ceil().min(cy1 as f32)) as usize;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                        let w0 = ((pb.x - p.x) * (pc.y - p.y) - (pb.y - p.y) * (pc.x - p.x)) / area;
                        let w1 = ((pc.x - p.x) * (pa.y - p.y) - (pc.y - p.y) * (pa.x - p.x)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                            continue;
                        }
                        let lerp = |f: &dyn Fn(&egui::epaint::Vertex) -> f32| w0 * f(a) + w1 * f(b) + w2 * f(c);
                        let color = egui::Color32::from_rgba_premultiplied(
                            lerp(&|v| f32::from(v.color.r())) as u8,
                            lerp(&|v| f32::from(v.color.g())) as u8,
                            lerp(&|v| f32::from(v.color.b())) as u8,
                            lerp(&|v| f32::from(v.color.a())) as u8,
                        );
                        let texel = texture
                            .and_then(|t| {
                                let u = (lerp(&|v| v.uv.x) * t.size[0] as f32) as usize;
                                let v = (lerp(&|v| v.uv.y) * t.size[1] as f32) as usize;
                                t.pixels.get(v.min(t.size[1].saturating_sub(1)) * t.size[0] + u.min(t.size[0].saturating_sub(1))).copied()
                            })
                            .unwrap_or(egui::Color32::WHITE);
                        let src = mul(color, texel);
                        if let Some(dst) = px.get_mut(y * w + x) {
                            let k = 1.0 - src[3] / 255.0;
                            for i in 0..4 {
                                dst[i] = src[i] + dst[i] * k;
                            }
                        }
                    }
                }
            }
        }
        let pixels =
            px.iter().map(|p| egui::Color32::from_rgb(p[0].clamp(0.0, 255.0) as u8, p[1].clamp(0.0, 255.0) as u8, p[2].clamp(0.0, 255.0) as u8)).collect();
        egui::ColorImage::new([w, h], pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_checksums_base64_and_bounds() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
        assert_eq!(crc32(b"IEND"), 0xae426082);
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        let image = egui::ColorImage::filled([4, 2], egui::Color32::RED);
        let value = encode(&image, 2).unwrap();
        assert_eq!(value["width"], 2);
        assert_eq!(value["height"], 1);
        assert!(value["pngBase64"].as_str().unwrap().starts_with("iVBORw0KGgo"));
        assert!(encode(&image, 0).is_err());
        assert!(encode(&image, 2049).is_err());
        assert!(encode(&egui::ColorImage::filled([2048, 2048], egui::Color32::WHITE), 2048).is_err());
    }
}

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
    let size = w
        .checked_mul(4)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(h))
        .filter(|n| *n + 1024 < PNG_LIMIT)
        .ok_or("screenshot exceeds 5 MiB; request a smaller maxSide")?;
    let mut raw = Vec::with_capacity(size);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let source = (y * height / h).checked_mul(width).and_then(|n| n.checked_add(x * width / w)).ok_or("screenshot dimensions overflow")?;
            let color = image.pixels.get(source).ok_or("missing screenshot pixel")?;
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
    header.extend((w as u32).to_be_bytes());
    header.extend((h as u32).to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header)?;
    chunk(&mut png, b"IDAT", &zlib)?;
    chunk(&mut png, b"IEND", &[])?;
    if png.len() > PNG_LIMIT {
        return Err("screenshot exceeds 5 MiB; request a smaller maxSide".into());
    }
    Ok(serde_json::json!({"mime":"image/png","pngBase64":base64(&png),"width":w,"height":h}))
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
fn base64(bytes: &[u8]) -> String {
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

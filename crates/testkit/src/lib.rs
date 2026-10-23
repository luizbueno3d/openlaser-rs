//! Dev-only test support (SPEC §3.22). Other crates use it only as a dev-dependency (§2.3 rule 10).
//!
//! WP0 implements the path and hash helpers; each loader module is owned by the work package that uses it
//! (§7.12): `t0`/`t1` WP4, `t2`/`t4` WP2, `t3` WP1, `adversarial` WP6.
#![forbid(unsafe_code)]

use std::path::PathBuf;

use sha2::{Digest, Sha256};

pub mod adversarial;
pub mod t0;
pub mod t1;
pub mod t2;
pub mod t3;
pub mod t4;

/// The repository root, from this crate's `CARGO_MANIFEST_DIR` (`<repo>/crates/testkit`).
pub fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(|crates| crates.parent()).map(PathBuf::from).unwrap_or(manifest)
}

/// `<repo>/tests/golden`.
pub fn golden_dir() -> PathBuf {
    repo_root().join("tests").join("golden")
}

/// `<repo>/tests/fixtures/<name>`; `name` may contain `/`-separated components.
pub fn fixture(name: &str) -> PathBuf {
    let mut p = repo_root().join("tests").join("fixtures");
    for part in name.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Lowercase hex sha256 (golden packet file names are sha256).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Decodes hex (either case; ASCII whitespace ignored, as in the `.hex` packet files).
pub fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return Err(format!("odd number of hex digits ({})", digits.len()));
    }
    let mut out = Vec::with_capacity(digits.len() / 2);
    let (pairs, _) = digits.as_chunks::<2>();
    for &[hi, lo] in pairs {
        let hi = nibble(hi).ok_or_else(|| format!("not a hex digit: {:?}", char::from(hi)))?;
        let lo = nibble(lo).ok_or_else(|| format!("not a hex digit: {:?}", char::from(lo)))?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_empty_input() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn hex_roundtrip_and_errors() {
        assert_eq!(hex_decode("00ff 8002\nAB").unwrap(), vec![0x00, 0xff, 0x80, 0x02, 0xab]);
        assert!(hex_decode("abc").is_err());
        assert!(hex_decode("zz").is_err());
    }

    #[test]
    fn golden_dir_has_the_manifest() {
        assert!(golden_dir().join("manifest.json").is_file());
        assert!(fixture("square10mm.svg").is_file());
    }
}

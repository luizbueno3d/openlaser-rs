//! `cargo xtask golden-verify`: sha256 and size of every fixture in `tests/golden/manifest.json`.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::files::{join_rel, rel, walk};

/// The tier directories whose every file must be listed in the manifest.
const TIERS: &[&str] = &["rust-v1", "t0_numeric", "t1_lmc", "t2_gcode", "t3_hatch", "t4_import"];

pub fn run(root: &Path) -> Result<(), String> {
    let golden = root.join("tests").join("golden");
    let manifest_path = golden.join("manifest.json");
    let text = std::fs::read(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;

    let mut problems = Vec::new();
    let mut checked = 0usize;
    let mut listed = BTreeSet::new();

    for (section, base) in [("files", golden.clone()), ("inputs", golden.join("inputs"))] {
        let Some(entries) = manifest.get(section).and_then(Value::as_object) else {
            problems.push(format!("manifest has no `{section}` object"));
            continue;
        };
        for (name, entry) in entries {
            let path = join_rel(&base, name);
            listed.insert(rel(root, &path));
            let want_sha = entry.get("sha256").and_then(Value::as_str).unwrap_or_default();
            let want_bytes = entry.get("bytes").and_then(Value::as_u64);
            match std::fs::read(&path) {
                Err(e) => problems.push(format!("{}: {e}", rel(root, &path))),
                Ok(bytes) => {
                    checked += 1;
                    let got = sha256_hex(&bytes);
                    if got != want_sha {
                        problems.push(format!("{}: sha256 {got} != manifest {want_sha}", rel(root, &path)));
                    }
                    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    if want_bytes.is_some_and(|b| b != len) {
                        problems.push(format!("{}: {len} bytes != manifest {}", rel(root, &path), want_bytes.unwrap_or_default()));
                    }
                }
            }
        }
    }

    let declared_total = manifest.get("totals").and_then(|t| t.get("files")).and_then(Value::as_u64);
    let files_listed = manifest.get("files").and_then(Value::as_object).map(|m| m.len());
    if let (Some(t), Some(n)) = (declared_total, files_listed)
        && u64::try_from(n).ok() != Some(t)
    {
        problems.push(format!("manifest totals.files = {t} but {n} files are listed"));
    }

    // Every file in a tier directory must be in the manifest (fixtures are never edited or added by hand).
    for tier in TIERS {
        let dir = golden.join(tier);
        for f in walk(&dir, &|_| true) {
            let r = rel(root, &f);
            if !listed.contains(&r) {
                problems.push(format!("{r}: not listed in tests/golden/manifest.json"));
            }
        }
    }

    if problems.is_empty() {
        println!("golden-verify: {checked} fixtures match tests/golden/manifest.json");
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

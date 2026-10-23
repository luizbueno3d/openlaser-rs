//! Small file helpers shared by the tasks.

use std::path::{Path, PathBuf};

/// Directories never scanned.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".ai-router", "golden"];

/// Every file under `dir` (recursively) whose name satisfies `keep`, skipping build output, VCS data and goldens.
pub fn walk(dir: &Path, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if !SKIP_DIRS.iter().any(|s| name == *s) {
                    stack.push(path);
                }
            } else if kind.is_file() && keep(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// `path` relative to `root`, with `/` separators (stable on every OS).
pub fn rel(root: &Path, path: &Path) -> String {
    let r = path.strip_prefix(root).unwrap_or(path);
    r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

pub fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().is_some_and(|e| e == ext)
}

/// Joins a `/`-separated relative path onto `base` (never builds paths from `/` literals, SPEC §2.6).
pub fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

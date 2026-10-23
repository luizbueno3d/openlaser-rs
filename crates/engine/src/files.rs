//! File access for `Roots` commands (SPEC §3.16 pipeline step 4): a human reads and writes what they pick; every
//! other origin goes through the session's `FsAuthority` (automation adapts `control::AuthorizedWorkspace`).

use std::path::Path;

use crate::command::{EngineError, Origin};
use crate::session::Session;

/// Automation adapts control::AuthorizedWorkspace to this.
pub trait FileAccess: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), String>;
}

pub enum FsAuthority {
    Denied,
    Scoped(Box<dyn FileAccess>),
    TrustedLocal,
}

pub(crate) const FS_DENIED: &str = "automation filesystem access is not granted";

/// Largest file a command reads (imports, projects, profiles): 256 MiB.
pub const MAX_READ_BYTES: u64 = 256 << 20;

impl Session {
    /// Reads `path` with the rights of the running command's origin.
    pub(crate) fn read_file(&self, path: &str) -> Result<Vec<u8>, EngineError> {
        check_path(path)?;
        if self.effective_origin() == Origin::Human {
            return read_local(path);
        }
        match &self.cfg.files {
            crate::files::FsAuthority::Denied => Err(EngineError::Io(FS_DENIED.to_owned())),
            crate::files::FsAuthority::Scoped(fa) => {
                let bytes = fa.read(path).map_err(EngineError::Io)?;
                if bytes.len() as u64 > MAX_READ_BYTES {
                    return Err(EngineError::Io(format!("{path} is larger than {} MiB", MAX_READ_BYTES >> 20)));
                }
                Ok(bytes)
            }
            crate::files::FsAuthority::TrustedLocal => read_local(path),
        }
    }

    /// Writes `path` atomically with the rights of the running command's origin.
    pub(crate) fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), EngineError> {
        check_path(path)?;
        if self.effective_origin() == Origin::Human {
            return write_local(path, bytes);
        }
        match &self.cfg.files {
            FsAuthority::Denied => Err(EngineError::Io(FS_DENIED.to_owned())),
            FsAuthority::Scoped(fa) => fa.write_atomic(path, bytes).map_err(EngineError::Io),
            FsAuthority::TrustedLocal => write_local(path, bytes),
        }
    }
}

fn check_path(path: &str) -> Result<(), EngineError> {
    if path.trim().is_empty() {
        return Err(EngineError::Io("the path is empty".to_owned()));
    }
    if path.contains('\0') {
        return Err(EngineError::Io("the path contains a NUL character".to_owned()));
    }
    Ok(())
}

fn read_local(path: &str) -> Result<Vec<u8>, EngineError> {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).map_err(|e| EngineError::Io(format!("{path}: {e}")))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("{path} is not a file")));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(EngineError::Io(format!("{path} is larger than {} MiB", MAX_READ_BYTES >> 20)));
    }
    std::fs::read(p).map_err(|e| EngineError::Io(format!("{path}: {e}")))
}

/// Temp file in the same folder, then rename (atomic on the same volume, replacing on Windows too).
fn write_local(path: &str, bytes: &[u8]) -> Result<(), EngineError> {
    let target = Path::new(path);
    let Some(name) = target.file_name() else { return Err(EngineError::Io(format!("{path} has no file name"))) };
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".openlaser-tmp");
    let tmp = target.with_file_name(tmp_name);
    std::fs::write(&tmp, bytes).map_err(|e| EngineError::Io(format!("{}: {e}", tmp.display())))?;
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(EngineError::Io(format!("{path}: {e}")));
    }
    Ok(())
}

/// The sibling `name` of `path` (same folder), as a string for `FileAccess`.
pub(crate) fn sibling(path: &str, name: &str) -> String {
    Path::new(path).with_file_name(name).to_string_lossy().into_owned()
}

/// The file name of `path` ("card.svg").
pub(crate) fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Lower-case extension without the dot ("olp").
pub(crate) fn extension(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_write_then_read_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        let ps = p.to_string_lossy().into_owned();
        write_local(&ps, b"hello").unwrap();
        assert_eq!(read_local(&ps).unwrap(), b"hello");
        write_local(&ps, b"again").unwrap();
        assert_eq!(read_local(&ps).unwrap(), b"again");
        assert!(read_local(&dir.path().to_string_lossy()).is_err(), "a folder is not a file");
    }

    #[test]
    fn path_helpers() {
        let p = Path::new("jobs").join("card.svg").to_string_lossy().into_owned();
        assert_eq!(file_name(&p), "card.svg");
        assert_eq!(extension(&p), "svg");
        assert_eq!(extension("X.OLP"), "olp");
        assert_eq!(sibling(&p, "layer_settings.json"), Path::new("jobs").join("layer_settings.json").to_string_lossy());
        assert!(check_path("").is_err());
        assert!(check_path("a\0b").is_err());
    }
}

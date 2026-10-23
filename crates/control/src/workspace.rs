//! cap-std directory capabilities; separate read and write roots; missing root = denied (fail closed).
//! Path grammar: non-empty, '/'-separated, relative; no '\', ':', leading '/', empty, '.', '..' component,
//! no component ending in '.' or space; an absolute path is accepted only beneath the root and re-checked.
//! All paths are resolved through directory capabilities. The grammar is deliberately portable.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use cap_std::fs::Dir;

use crate::Error;

pub struct AuthorizedWorkspace {
    read: Option<Dir>,
    write: Option<Dir>,
    read_root: Option<PathBuf>,
    write_root: Option<PathBuf>,
}

impl AuthorizedWorkspace {
    pub fn denied() -> Self {
        AuthorizedWorkspace { read: None, write: None, read_root: None, write_root: None }
    }

    pub fn open(read: Option<&Path>, write: Option<&Path>) -> Result<Self, Error> {
        let read_root = read.map(Path::canonicalize).transpose()?;
        let write_root = write.map(Path::canonicalize).transpose()?;
        let read = read_root.as_deref().map(|path| cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())).transpose().map_err(Error::Io)?;
        let write = write_root.as_deref().map(|path| cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())).transpose().map_err(Error::Io)?;
        Ok(Self { read, write, read_root, write_root })
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let dir = self.read.as_ref().ok_or_else(|| Error::PathRefused("read access is not granted".to_owned()))?;
        let path = resolve_path(path, self.read_root.as_deref())?;
        let mut file = dir.open(path).map_err(Error::Io)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let dir = self.write.as_ref().ok_or_else(|| Error::PathRefused("write access is not granted".to_owned()))?;
        let path = resolve_path(path, self.write_root.as_deref())?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        if !parent.as_os_str().is_empty() {
            dir.create_dir_all(parent).map_err(Error::Io)?;
        }
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).map_err(|error| Error::PathRefused(format!("cannot allocate temporary name: {error}")))?;
        let temp_name = format!(".openlaser-{:016x}.tmp", u64::from_le_bytes(random));
        let temp = parent.join(&temp_name);
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = dir.open_with(&temp, &options).map_err(Error::Io)?;
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = dir.remove_file(&temp);
            return Err(Error::Io(error));
        }
        if let Err(error) = dir.rename(&temp, dir, &path) {
            let _ = dir.remove_file(&temp);
            return Err(Error::Io(error));
        }
        Ok(())
    }
}

fn resolve_path(path: &str, root: Option<&Path>) -> Result<PathBuf, Error> {
    if path.is_empty() || path.contains('\\') || path.contains(':') {
        return Err(Error::PathRefused("path must be non-empty and use portable '/' separators".to_owned()));
    }
    let candidate = Path::new(path);
    let relative = if candidate.is_absolute() {
        // Absolute paths are accepted only when they are descendants of this exact root.
        let root = root.ok_or_else(|| Error::PathRefused("no root is granted".to_owned()))?;
        let mut existing = candidate.to_path_buf();
        let mut suffix = Vec::new();
        while !existing.exists() {
            let Some(name) = existing.file_name() else { return Err(Error::PathRefused("absolute path has no existing ancestor".to_owned())) };
            suffix.push(name.to_os_string());
            let Some(parent) = existing.parent() else { return Err(Error::PathRefused("absolute path has no existing ancestor".to_owned())) };
            if parent == existing {
                return Err(Error::PathRefused("absolute path has no existing ancestor".to_owned()));
            }
            existing = parent.to_path_buf();
        }
        let mut canonical = existing.canonicalize().map_err(Error::Io)?;
        for component in suffix.into_iter().rev() {
            canonical.push(component);
        }
        canonical.strip_prefix(root).map_err(|_| Error::PathRefused("absolute path is outside the granted root".to_owned()))?.to_path_buf()
    } else {
        candidate.to_path_buf()
    };
    let text = relative.to_str().ok_or_else(|| Error::PathRefused("path is not valid Unicode".to_owned()))?;
    if text.starts_with('/') || text.is_empty() {
        return Err(Error::PathRefused("absolute or empty path refused".to_owned()));
    }
    for component in text.split('/') {
        if component.is_empty() || component == "." || component == ".." || component.ends_with('.') || component.ends_with(' ') {
            return Err(Error::PathRefused(format!("invalid path component `{component}`")));
        }
    }
    if relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Error::PathRefused("path contains a non-normal component".to_owned()));
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_scoped_and_atomic_writes_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let read = temp.path().join("read");
        let write = temp.path().join("write");
        std::fs::create_dir_all(&read).unwrap();
        std::fs::create_dir_all(&write).unwrap();
        std::fs::write(read.join("source.txt"), b"safe").unwrap();
        let ws = AuthorizedWorkspace::open(Some(&read), Some(&write)).unwrap();
        assert_eq!(ws.read("source.txt").unwrap(), b"safe");
        ws.write_atomic("nested/result.txt", b"result").unwrap();
        assert_eq!(std::fs::read(write.join("nested/result.txt")).unwrap(), b"result");
        assert_eq!(ws.read(read.join("source.txt").to_str().unwrap()).unwrap(), b"safe");
    }

    #[test]
    fn path_grammar_and_missing_roots_fail_closed() {
        let ws = AuthorizedWorkspace::denied();
        assert!(ws.read("file").is_err());
        let temp = tempfile::tempdir().unwrap();
        let ws = AuthorizedWorkspace::open(Some(temp.path()), Some(temp.path())).unwrap();
        for path in ["", "../escape", "a//b", "a/./b", "a\\b", "C:/x", "/etc/passwd", "trailing.", "trailing "] {
            assert!(ws.read(path).is_err(), "accepted {path:?}");
        }
    }
}

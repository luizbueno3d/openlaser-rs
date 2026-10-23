//! cap-std directory capabilities; separate read and write roots; missing root = denied (fail closed).
//! Path grammar: non-empty, '/'-separated, relative; no '\', ':', leading '/', empty, '.', '..' component,
//! no component ending in '.' or space, no Windows device name (`CON`, `NUL`, `COM1`, `LPT1.txt`, …); an absolute
//! path is accepted only when its text lies beneath the root (component-wise, drive letter without case) and the
//! rest is re-checked as relative.
//!
//! Nothing is resolved against the real filesystem: request paths are compared as text and every open goes through
//! the held `Dir`, so links cannot escape a root. Reads return regular files only (a FIFO or device is refused
//! before it is opened, and again on the opened handle), at most `MAX_READ_BYTES`, and never past the length seen
//! when the file was opened. Writes need the parent folder to exist (they never create folders) and are atomic.
//!
//! Follows PhotoCraft's `crates/automation/src/workspace.rs` (MIT OR Apache-2.0) at
//! c699e3cda69f4852b43c0b191ab0091aaeff9d6a, adapted to OpenLaser's error type.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use cap_std::fs::Dir;

use crate::{Error, budgets};

struct Root {
    dir: Dir,
    /// The root as given, made absolute textually (no link resolution), to recognise absolute request paths.
    path: PathBuf,
}

pub struct AuthorizedWorkspace {
    read: Option<Root>,
    write: Option<Root>,
}

impl AuthorizedWorkspace {
    pub fn denied() -> Self {
        AuthorizedWorkspace { read: None, write: None }
    }

    pub fn open(read: Option<&Path>, write: Option<&Path>) -> Result<Self, Error> {
        Ok(Self { read: open_root(read, "read")?, write: open_root(write, "write")? })
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let root = self.read.as_ref().ok_or_else(|| Error::PathRefused("automation filesystem access is not granted: no read root".to_owned()))?;
        let relative = request_path(path, &root.path)?;
        // Before opening: opening a FIFO for reading would block until a writer appears.
        let before = root.dir.metadata(&relative).map_err(|e| file_error("read", path, e))?;
        check_regular(path, &before)?;
        let mut file = root.dir.open(&relative).map_err(|e| file_error("read", path, e))?;
        let meta = file.metadata().map_err(|e| file_error("read", path, e))?;
        check_regular(path, &meta)?;
        read_to_observed_length(&mut file, meta.len(), path)
    }

    pub fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let root = self.write.as_ref().ok_or_else(|| Error::PathRefused("automation filesystem access is not granted: no write root".to_owned()))?;
        let relative = request_path(path, &root.path)?;
        let parent = relative.parent().map(Path::to_path_buf).unwrap_or_default();
        if !parent.as_os_str().is_empty() {
            match root.dir.metadata(&parent) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => return Err(Error::PathRefused(format!("`{path}`: the parent is not a folder"))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(Error::PathRefused(format!("`{path}`: the folder `{}` does not exist (folders are never created)", parent.display())));
                }
                Err(e) => return Err(file_error("write", path, e)),
            }
        }
        if let Ok(existing) = root.dir.symlink_metadata(&relative)
            && existing.is_dir()
        {
            return Err(Error::PathRefused(format!("`{path}` is a folder")));
        }
        let leaf = relative.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let temp = parent.join(craft_atomic::temp_name(&leaf));
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let written = root.dir.open_with(&temp, &options).and_then(|mut file| {
            file.write_all(bytes)?;
            craft_atomic::sync_file(&file.into_std())
        });
        let renamed = written.and_then(|()| craft_atomic::retry_rename(craft_atomic::RenameRetry::platform(), || root.dir.rename(&temp, &root.dir, &relative)));
        if let Err(e) = renamed {
            let _ = root.dir.remove_file(&temp);
            return Err(file_error("write", path, e));
        }
        Ok(())
    }
}

fn open_root(path: Option<&Path>, authority: &str) -> Result<Option<Root>, Error> {
    let Some(path) = path else { return Ok(None) };
    if path.as_os_str().is_empty() {
        return Err(Error::PathRefused(format!("automation {authority} root is empty")));
    }
    let dir = Dir::open_ambient_dir(path, cap_std::ambient_authority())
        .map_err(|e| Error::Io(std::io::Error::new(e.kind(), format!("cannot open automation {authority} root `{}`: {e}", path.display()))))?;
    // Textual (no link resolution), like the request paths it is compared with.
    let absolute = std::path::absolute(path)?;
    Ok(Some(Root { dir, path: absolute }))
}

fn check_regular(path: &str, meta: &cap_std::fs::Metadata) -> Result<(), Error> {
    if !meta.is_file() {
        return Err(Error::PathRefused(format!("`{path}` is not a regular file")));
    }
    if meta.len() > budgets::MAX_READ_BYTES {
        return Err(Error::PathRefused(format!("`{path}` is {} bytes; automation reads at most {}", meta.len(), budgets::MAX_READ_BYTES)));
    }
    Ok(())
}

/// Reads in chunks without reserving from metadata, and refuses growth past the length seen at open.
fn read_to_observed_length(reader: &mut impl Read, observed: u64, path: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; 64 << 10];
    loop {
        let remaining = observed.saturating_sub(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        let ask = usize::try_from(remaining.saturating_add(1)).unwrap_or(usize::MAX).min(chunk.len());
        let n = match reader.read(chunk.get_mut(..ask).unwrap_or_default()) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(file_error("read", path, e)),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if u64::try_from(n).unwrap_or(u64::MAX) > remaining {
            return Err(Error::PathRefused(format!("`{path}` grew beyond the {observed} bytes it had when opened")));
        }
        bytes.extend_from_slice(chunk.get(..n).unwrap_or_default());
    }
}

/// A request path as the relative path the root's capability resolves. An absolute path is accepted only when its
/// text names a file beneath `root`: the root's components are stripped (a Windows prefix compared without case) and
/// the rest must pass [`relative_path`]. Nothing touches the filesystem.
fn request_path(raw: &str, root: &Path) -> Result<PathBuf, Error> {
    if !Path::new(raw).is_absolute() {
        return relative_path(raw);
    }
    let mut request = Path::new(raw).components();
    for want in root.components() {
        let same = match (request.next(), want) {
            (Some(Component::Prefix(a)), Component::Prefix(b)) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
            (Some(got), want) => got == want,
            (None, _) => false,
        };
        if !same {
            return Err(path_error(raw, "absolute paths must be inside the automation root"));
        }
    }
    let rest: Vec<&str> = request.map(|c| c.as_os_str().to_str().unwrap_or("\u{FFFD}")).collect();
    relative_path(&rest.join("/"))
}

fn relative_path(raw: &str) -> Result<PathBuf, Error> {
    if raw.is_empty() {
        return Err(path_error(raw, "path is empty"));
    }
    if raw.contains('\\') {
        return Err(path_error(raw, "alternate separators are not allowed; use `/`"));
    }
    if raw.starts_with('/') {
        return Err(path_error(raw, "absolute paths are not allowed"));
    }
    if raw.contains(':') {
        return Err(path_error(raw, "drive, device and stream prefixes are not allowed"));
    }
    for component in raw.split('/') {
        if component.is_empty() {
            return Err(path_error(raw, "empty path components are not allowed"));
        }
        if component == "." {
            return Err(path_error(raw, "`.` path components are not allowed"));
        }
        if component == ".." {
            return Err(path_error(raw, "parent traversal is not allowed"));
        }
        if component.ends_with(['.', ' ']) {
            return Err(path_error(raw, "path components ending in a dot or space are not allowed"));
        }
        if component.chars().any(char::is_control) {
            return Err(path_error(raw, "control characters are not allowed"));
        }
        if is_windows_device_name(component) {
            return Err(path_error(raw, "reserved device names are not allowed"));
        }
    }
    let path = PathBuf::from(raw);
    if path.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(path_error(raw, "path contains a non-normal component"));
    }
    Ok(path)
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9` (also with superscript digits) and the console names,
/// with or without an extension, in any case: Windows opens the device whatever folder they are in.
fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end().to_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

fn path_error(path: &str, reason: &str) -> Error {
    Error::PathRefused(format!("automation path rejected: {reason}: `{path}`"))
}

fn file_error(operation: &str, path: &str, error: std::io::Error) -> Error {
    Error::Io(std::io::Error::new(error.kind(), format!("automation {operation} `{path}` failed: {error}")))
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
        std::fs::create_dir_all(write.join("nested")).unwrap();
        std::fs::write(read.join("source.txt"), b"safe").unwrap();
        let ws = AuthorizedWorkspace::open(Some(&read), Some(&write)).unwrap();
        assert_eq!(ws.read("source.txt").unwrap(), b"safe");
        ws.write_atomic("nested/result.txt", b"result").unwrap();
        assert_eq!(std::fs::read(write.join("nested").join("result.txt")).unwrap(), b"result");
        assert_eq!(ws.read(read.join("source.txt").to_str().unwrap()).unwrap(), b"safe");
        assert!(ws.read(write.join("nested").join("result.txt").to_str().unwrap()).is_err(), "absolute path outside the read root");
    }

    #[test]
    fn workspace_path_grammar() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("ok.txt"), b"x").unwrap();
        let ws = AuthorizedWorkspace::open(Some(temp.path()), Some(temp.path())).unwrap();
        for path in [
            "",
            "../escape",
            "a//b",
            "a/./b",
            "a\\b",
            "C:/x",
            "/etc/passwd",
            "trailing.",
            "trailing ",
            "CON",
            "nul.txt",
            "sub/COM1",
            "lpt9.log",
            "Aux",
            "COM¹",
            "bell\u{7}",
        ] {
            assert!(ws.read(path).is_err(), "read accepted {path:?}");
            assert!(ws.write_atomic(path, b"x").is_err(), "write accepted {path:?}");
        }
        assert!(ws.read("ok.txt").is_ok());
        assert!(!is_windows_device_name("console.txt") && !is_windows_device_name("COM10") && !is_windows_device_name("nullable"));
    }

    #[test]
    fn roots_fail_closed() {
        let ws = AuthorizedWorkspace::denied();
        assert!(ws.read("file").is_err());
        assert!(ws.write_atomic("file", b"x").is_err());
        let temp = tempfile::tempdir().unwrap();
        let read_only = AuthorizedWorkspace::open(Some(temp.path()), None).unwrap();
        assert!(read_only.write_atomic("file", b"x").is_err());
        assert!(!temp.path().join("file").exists());
    }

    #[test]
    fn writes_never_create_folders() {
        let temp = tempfile::tempdir().unwrap();
        let ws = AuthorizedWorkspace::open(None, Some(temp.path())).unwrap();
        assert!(ws.write_atomic("missing/out.txt", b"x").is_err());
        assert!(!temp.path().join("missing").exists());
    }

    #[test]
    fn reads_only_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("folder")).unwrap();
        let ws = AuthorizedWorkspace::open(Some(temp.path()), None).unwrap();
        assert!(matches!(ws.read("folder"), Err(Error::PathRefused(message)) if message.contains("regular file")));
    }

    #[test]
    fn growth_past_the_observed_length_is_refused() {
        let mut source = std::io::Cursor::new(vec![b'a'; 10]);
        assert!(read_to_observed_length(&mut source, 4, "f").is_err());
        let mut source = std::io::Cursor::new(vec![b'a'; 4]);
        assert_eq!(read_to_observed_length(&mut source, 4, "f").unwrap().len(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_escape_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(temp.path().join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(temp.path().join("secret"), root.join("link")).unwrap();
        let ws = AuthorizedWorkspace::open(Some(&root), None).unwrap();
        assert!(ws.read("link").is_err());
    }
}

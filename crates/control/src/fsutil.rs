//! Private-file helpers shared by the token and the discovery file (SPEC §2.6: unix `0700` dir + `0600` file;
//! Windows: the per-user AppData folder plus `FILE_ATTRIBUTE_HIDDEN`).

use std::fs::{DirBuilder, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::Error;

/// `FILE_ATTRIBUTE_HIDDEN` (winnt.h).
#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

/// Creates `dir` and every missing ancestor, each private (`0700` on unix). A folder that already exists is left
/// exactly as it is: a token path such as `~/token` must never chmod the home folder.
pub(crate) fn create_private_dirs(dir: &Path) -> Result<(), Error> {
    let mut missing: Vec<PathBuf> = Vec::new();
    let mut current = Some(dir);
    while let Some(path) = current.filter(|p| !p.as_os_str().is_empty()) {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => return Err(Error::PathRefused(format!("`{}` exists and is not a folder", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(path.to_path_buf());
                current = path.parent();
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
    for path in missing.iter().rev() {
        let builder = private_dir_builder();
        match builder.create(path) {
            Ok(()) => {
                // The umask can only remove bits, but say exactly what we mean for the folders we made.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
                }
            }
            // Someone else made it between the check and here: it is theirs, leave it alone.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(())
}

/// `0700` on unix; on Windows the per-user AppData folder's own ACL applies.
fn private_dir_builder() -> DirBuilder {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        builder
    }
    #[cfg(not(unix))]
    DirBuilder::new()
}

/// `create_new` + write, `0600` on unix, hidden on Windows.
pub(crate) fn private_new_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.attributes(FILE_ATTRIBUTE_HIDDEN);
    }
    options
}

/// Reads a small secret file: a regular file of at most `max` bytes that no other user can read or write (unix).
pub(crate) fn read_private_file(path: &Path, max: u64, what: &str) -> Result<Vec<u8>, Error> {
    // Checked before opening too: opening a FIFO for reading blocks until a writer appears.
    if !std::fs::metadata(path)?.is_file() {
        return Err(Error::Insecure(format!("{what} `{}` is not a regular file", path.display())));
    }
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(Error::Insecure(format!("{what} `{}` is not a regular file", path.display())));
    }
    check_private_mode(path, &meta, what)?;
    if meta.len() > max {
        return Err(Error::Insecure(format!("{what} `{}` is {} bytes; at most {max} are expected", path.display(), meta.len())));
    }
    let mut bytes = Vec::new();
    // `take(max + 1)`: a file that grows after the metadata check is still refused, never read without end.
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(Error::Insecure(format!("{what} `{}` is longer than {max} bytes", path.display())));
    }
    Ok(bytes)
}

/// Unix: refuse a secret that group or other users can read or write (mode bits `0o077`).
fn check_private_mode(path: &Path, meta: &std::fs::Metadata, what: &str) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(Error::Insecure(format!(
                "{what} `{}` has mode {mode:o}, so other users can read or change it; run `chmod 600` on it (or delete it to get a new one)",
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = (path, meta, what);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn only_created_folders_are_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join("shared");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
        let nested = existing.join("a").join("b");
        create_private_dirs(&nested).unwrap();
        assert_eq!(std::fs::metadata(&existing).unwrap().permissions().mode() & 0o777, 0o755, "an existing folder was chmodded");
        assert_eq!(std::fs::metadata(existing.join("a")).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn oversized_secret_file_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("big");
        let mut file = private_new_file_options().open(&path).unwrap();
        std::io::Write::write_all(&mut file, &[b'a'; 2048]).unwrap();
        assert!(matches!(read_private_file(&path, 1024, "token file"), Err(Error::Insecure(_))));
    }
}

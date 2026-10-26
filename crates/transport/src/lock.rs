//! The board lock: `<data_dir>/OpenLaser/board.lock` (§2.6 paths), shared with the prototype (OM v1 patch 12,
//! `flock`); `File::try_lock`, which is `flock(LOCK_EX | LOCK_NB)` on Unix and `LockFileEx` on Windows, so the two
//! programs lock each other out.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use openlaser_types::{Refusal, RefusalCode};

/// The lock file name inside the data folder.
pub const BOARD_LOCK_FILE: &str = "board.lock";

/// `<data_dir>/OpenLaser/board.lock` (§2.6 paths), shared with the prototype (OM v1 patch 12, `flock`); File::try_lock.
/// Held for as long as the value lives (the transport keeps it with the open device).
#[derive(Debug)]
pub struct BoardLock {
    _file: File,
    path: PathBuf,
}

impl BoardLock {
    /// Takes the lock in the per-user data folder, or refuses `BoardLocked` (another OpenLaser or OpenMopa holds
    /// the board).
    pub fn acquire() -> Result<Self, Refusal> {
        let dir = crate::data_dir()?;
        BoardLock::acquire_in(&dir)
    }

    /// Takes the lock in `dir` (created if missing); tests use a temporary folder.
    pub fn acquire_in(dir: &Path) -> Result<Self, Refusal> {
        std::fs::create_dir_all(dir).map_err(|e| Refusal::new(RefusalCode::BoardLocked, format!("cannot create {}: {e}", dir.display())))?;
        let path = dir.join(BOARD_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| Refusal::new(RefusalCode::BoardLocked, format!("cannot open the board lock {}: {e}", path.display())))?;
        match file.try_lock() {
            Ok(()) => Ok(BoardLock { _file: file, path }),
            Err(TryLockError::WouldBlock) => Err(Refusal::new(
                RefusalCode::BoardLocked,
                format!("another program holds the laser board ({}): close OpenMopa or the other OpenLaser window first", path.display()),
            )),
            Err(TryLockError::Error(e)) => Err(Refusal::new(RefusalCode::BoardLocked, format!("cannot lock {}: {e}", path.display()))),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_lock_second_holder_refused() {
        let dir = tempfile::tempdir().unwrap();
        let first = BoardLock::acquire_in(dir.path()).unwrap();
        let err = BoardLock::acquire_in(dir.path()).unwrap_err();
        assert_eq!(err.code, RefusalCode::BoardLocked);
        assert!(err.message.contains("another program holds the laser board"), "{}", err.message);
        drop(first);
        let again = BoardLock::acquire_in(dir.path()).unwrap();
        assert!(again.path().ends_with(BOARD_LOCK_FILE));
    }
}

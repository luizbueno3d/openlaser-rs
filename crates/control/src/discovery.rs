//! The discovery file (S12): the token inline, 0600 file in a 0700 folder (Windows: per-user AppData). WP0 stubs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Discovery {
    pub version: u32,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub started: String,
    pub roots: Roots,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Roots {
    pub read: Option<String>,
    pub write: Option<String>,
}

/// `~/Library/Application Support/OpenLaser/automation/control.json` (`%APPDATA%\OpenLaser\automation\control.json`
/// on Windows). WP0 stub: empty path.
pub fn default_path() -> PathBuf {
    PathBuf::new()
}

/// Dir 0700, file 0600, atomic.
pub fn write(path: &Path, d: &Discovery) -> Result<(), Error> {
    let _ = (path, d);
    Err(Error::NotImplemented("openlaser_control::discovery::write"))
}

/// Err if the pid is gone (stale).
pub fn read(path: &Path) -> Result<Discovery, Error> {
    let _ = path;
    Err(Error::NotImplemented("openlaser_control::discovery::read"))
}

pub fn remove(path: &Path) {
    let _ = path;
}

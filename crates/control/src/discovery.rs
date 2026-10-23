//! The discovery file (S12): the token inline, 0600 file in a 0700 folder (Windows: per-user AppData).

use std::io::Write;
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
/// on Windows).
pub fn default_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support"));
    base.unwrap_or_default().join("OpenLaser").join("automation").join("control.json")
}

/// Dir 0700, file 0600, atomic.
pub fn write(path: &Path, d: &Discovery) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| Error::PathRefused("discovery path has no parent".to_owned()))?;
    std::fs::create_dir_all(parent)?;
    set_private_dir(parent)?;
    let bytes = serde_json::to_vec(d)?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|e| Error::PathRefused(format!("cannot allocate temporary file: {e}")))?;
    let temp = parent.join(format!(".control-{:016x}.tmp", u64::from_le_bytes(random)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<(), Error> {
        let mut file = options.open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Err if the pid is gone (stale).
pub fn read(path: &Path) -> Result<Discovery, Error> {
    let bytes = std::fs::read(path)?;
    let d: Discovery = serde_json::from_slice(&bytes)?;
    if d.version != 1 {
        return Err(Error::PathRefused(format!("unsupported discovery version {}", d.version)));
    }
    if !pid_is_alive(d.pid, d.port) {
        return Err(Error::PathRefused(format!("stale discovery: process {} is not running", d.pid)));
    }
    Ok(d)
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn pid_is_alive(pid: u32, port: u16) -> bool {
    if pid == std::process::id() {
        return true;
    }
    if port == 0 {
        return false;
    }
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), std::time::Duration::from_millis(100)).is_ok()
}

fn set_private_dir(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_pid_is_refused_and_private_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("automation").join("control.json");
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let discovery =
            Discovery { version: 1, pid: std::process::id(), port, token: "a".repeat(64), started: "now".to_owned(), roots: Roots { read: None, write: None } };
        write(&file, &discovery).unwrap();
        assert_eq!(read(&file).unwrap(), discovery);
        drop(listener);
        let stale = Discovery { pid: u32::MAX, ..discovery };
        write(&file, &stale).unwrap();
        assert!(read(&file).is_err());
    }
}

//! The discovery file (S12, AUT B10.2): the token inline, 0600 file in a 0700 folder (Windows: per-user AppData,
//! hidden).
//!
//! Before a client sends the token it proves the listener on `port` is the OpenLaser process `pid` that wrote the
//! file: the control server answers a pre-auth `hello` with its pid, and a mismatch (or any other answer) is a
//! stale file. A port that another program took over after OpenLaser exited therefore never sees the token.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Error, budgets, fsutil, token::Token, wire};

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

/// The pre-auth method that identifies the server (answered before `auth`, at most once per connection).
pub const HELLO_METHOD: &str = "hello";
/// How long a liveness probe may take to connect and to hear the `hello` reply.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// The per-user configuration folder's `OpenLaser/automation/control.json`, by `craft-settings`' rules (which this
/// leaf crate mirrors rather than links, §2.3): `OPENLASER_CONFIG_DIR` if set (empty = isolated: no discovery),
/// else `~/Library/Application Support/OpenLaser` (macOS), `%APPDATA%\OpenLaser` (Windows),
/// `$XDG_CONFIG_HOME/OpenLaser` or `~/.config/OpenLaser` (other unix). It never falls back to the current folder:
/// with no home folder it is an error.
pub fn default_path() -> Result<PathBuf, Error> {
    default_path_from(&|name| std::env::var_os(name))
}

/// [`default_path`] with the environment injected (tests cannot change the real one: `set_var` is unsafe).
fn default_path_from(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    Ok(config_dir(env)?.join("automation").join("control.json"))
}

fn config_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    if let Some(value) = env("OPENLASER_CONFIG_DIR") {
        if value.is_empty() {
            return Err(Error::PathRefused("OPENLASER_CONFIG_DIR is empty (isolated): there is no discovery file".to_owned()));
        }
        return absolute_dir(PathBuf::from(value), "OPENLASER_CONFIG_DIR");
    }
    platform_config_dir(env).map(|dir| dir.join("OpenLaser"))
}

#[cfg(windows)]
fn platform_config_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let appdata = env("APPDATA").filter(|v| !v.is_empty()).ok_or_else(|| Error::PathRefused("APPDATA is not set".to_owned()))?;
    absolute_dir(PathBuf::from(appdata), "APPDATA")
}

#[cfg(target_os = "macos")]
fn platform_config_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    Ok(home(env)?.join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_config_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    if let Some(xdg) = env("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Ok(xdg);
    }
    Ok(home(env)?.join(".config"))
}

#[cfg(not(any(unix, windows)))]
fn platform_config_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let _ = env;
    Err(Error::PathRefused("no per-user configuration folder on this platform".to_owned()))
}

#[cfg(unix)]
fn home(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let home = env("HOME").filter(|v| !v.is_empty()).ok_or_else(|| Error::PathRefused("HOME is not set".to_owned()))?;
    absolute_dir(PathBuf::from(home), "HOME")
}

fn absolute_dir(path: PathBuf, what: &str) -> Result<PathBuf, Error> {
    if path.is_absolute() { Ok(path) } else { Err(Error::PathRefused(format!("{what} is not an absolute path: `{}`", path.display()))) }
}

/// Dir 0700 (only the folders it creates), file 0600 (Windows: hidden), atomic.
pub fn write(path: &Path, d: &Discovery) -> Result<(), Error> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| Error::PathRefused("discovery path has no parent".to_owned()))?;
    fsutil::create_private_dirs(parent)?;
    let bytes = serde_json::to_vec(d)?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|e| Error::PathRefused(format!("cannot allocate temporary file: {e}")))?;
    let temp = parent.join(format!(".control-{:016x}.tmp", u64::from_le_bytes(random)));
    let result = (|| -> Result<(), Error> {
        let mut file = fsutil::private_new_file_options().open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        craft_atomic::retry_rename(craft_atomic::RenameRetry::platform(), || std::fs::rename(&temp, path))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Err if the file is not private, malformed, holds a malformed token, or the pid is gone or is not the process
/// listening on `port` (stale).
pub fn read(path: &Path) -> Result<Discovery, Error> {
    let bytes = fsutil::read_private_file(path, budgets::DISCOVERY_MAX_BYTES, "discovery file")?;
    let d: Discovery = serde_json::from_slice(&bytes)?;
    if d.version != 1 {
        return Err(Error::PathRefused(format!("unsupported discovery version {}", d.version)));
    }
    Token::from_hex(&d.token)?;
    probe(d.port, d.pid)?;
    Ok(d)
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Connects to `127.0.0.1:port` and checks, with `hello`, that the listener is OpenLaser running as `pid`. Sends no
/// secret.
pub fn probe(port: u16, pid: u32) -> Result<(), Error> {
    if port == 0 || pid == 0 {
        return Err(Error::Stale(format!("process {pid} on port {port} is not a live OpenLaser")));
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT)
        .map_err(|e| Error::Stale(format!("nothing is listening on port {port} ({e}); process {pid} is gone")))?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    check_hello(&mut stream, &mut reader, pid)
}

/// The `hello` exchange on an open connection, before any token is sent: the server must say it is OpenLaser and
/// report exactly `pid`.
pub fn check_hello(stream: &mut TcpStream, reader: &mut impl BufRead, pid: u32) -> Result<(), Error> {
    let mut line = serde_json::to_vec(&wire::Request { id: serde_json::json!(0), method: HELLO_METHOD.to_owned(), params: serde_json::json!({}) })?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    let reply =
        wire::read_line_bounded(reader, budgets::AUTH_MAX_BYTES)?.ok_or_else(|| Error::Stale("the listener closed without answering hello".to_owned()))?;
    let reply: wire::Reply = serde_json::from_slice(&reply).map_err(|_| Error::Stale("the listener is not an OpenLaser control server".to_owned()))?;
    let result = reply.result.filter(|_| reply.ok).ok_or_else(|| Error::Stale("the listener is not an OpenLaser control server".to_owned()))?;
    let server = result.get("server").and_then(serde_json::Value::as_str);
    let reported = result.get("pid").and_then(serde_json::Value::as_u64);
    if server != Some("openlaser") || reported != Some(u64::from(pid)) {
        return Err(Error::Stale(format!(
            "the listener is not OpenLaser process {pid} (it reports {}); the token was not sent",
            reported.map_or_else(|| "nothing".to_owned(), |p| p.to_string())
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::server::{ControlServer, ServerConfig};

    fn server() -> ControlServer {
        let token = Token::generate().unwrap();
        ControlServer::start(ServerConfig { port: 0, token, fast_stop: Arc::new(|_| serde_json::json!({"latched":true})), wake: Arc::new(|| {}) }).unwrap().0
    }

    fn discovery(port: u16, pid: u32) -> Discovery {
        Discovery { version: 1, pid, port, token: "a".repeat(64), started: "now".to_owned(), roots: Roots { read: None, write: None } }
    }

    #[test]
    fn discovery_stale_pid_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("automation").join("control.json");
        let live = server();
        let good = discovery(live.port(), std::process::id());
        write(&file, &good).unwrap();
        assert_eq!(read(&file).unwrap(), good);
        // The port is live, but it is not that pid: refused (the pid is really checked, not just the port).
        write(&file, &discovery(live.port(), std::process::id().wrapping_add(1))).unwrap();
        assert!(matches!(read(&file), Err(Error::Stale(_))));
        // Nothing listening any more: refused.
        let port = live.port();
        live.shutdown();
        write(&file, &discovery(port, std::process::id())).unwrap();
        assert!(matches!(read(&file), Err(Error::Stale(_))));
    }

    #[test]
    fn malformed_token_in_discovery_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("control.json");
        let live = server();
        let mut bad = discovery(live.port(), std::process::id());
        bad.token = "not-a-token".to_owned();
        write(&file, &bad).unwrap();
        assert!(matches!(read(&file), Err(Error::BadToken(_))));
        live.shutdown();
    }

    #[test]
    fn a_squatter_on_the_port_never_receives_anything_secret() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let squatter = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.write_all(b"{\"id\":0,\"ok\":true,\"result\":{\"server\":\"openlaser\",\"pid\":1}}\n");
                let mut buf = [0_u8; 4096];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    record.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            }
        });
        assert!(matches!(probe(port, std::process::id()), Err(Error::Stale(_))));
        squatter.join().unwrap();
        let seen = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
        assert!(seen.contains("hello") && !seen.contains("token"), "{seen}");
    }

    #[cfg(unix)]
    #[test]
    fn discovery_file_and_new_folder_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("automation").join("control.json");
        write(&file, &discovery(1, 1)).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn default_path_never_falls_back_to_the_current_folder() {
        let empty = |_: &str| None;
        assert!(default_path_from(&empty).is_err(), "no HOME/APPDATA must be an error, not ./OpenLaser/…");
        let relative = |name: &str| (name != "OPENLASER_CONFIG_DIR").then(|| OsString::from("relative/home"));
        assert!(default_path_from(&relative).is_err());
        let isolated = |name: &str| (name == "OPENLASER_CONFIG_DIR").then(OsString::new);
        assert!(default_path_from(&isolated).is_err());
        let root = std::env::temp_dir();
        let overridden = |name: &str| (name == "OPENLASER_CONFIG_DIR").then(|| root.clone().into_os_string());
        assert_eq!(default_path_from(&overridden).unwrap(), root.join("automation").join("control.json"));
        if let Ok(path) = default_path() {
            assert!(path.is_absolute(), "{}", path.display());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_path_is_application_support_on_macos() {
        let home = |name: &str| (name == "HOME").then(|| OsString::from("/Users/someone"));
        assert_eq!(default_path_from(&home).unwrap(), Path::new("/Users/someone/Library/Application Support/OpenLaser/automation/control.json"));
    }
}

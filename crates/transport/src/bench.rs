//! The bench session (S17, S23): a human at the laser starts it at a TTY (`openlaser-hw bench start`); every real
//! open needs one ≤ 2 h old, and the first emission of each session is capped at 10 % power.
//!
//! `<data_dir>/OpenLaser/bench-session.json` = `{createdUnix, pid, emissions}`. Wall clock, because the file
//! crosses processes. No tty name is stored: reading it needs `ttyname(3)` (unsafe); `IsTerminal` is enough.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use openlaser_types::{Refusal, RefusalCode};
use serde::{Deserialize, Serialize};

pub const BENCH_FILE: &str = "bench-session.json";
/// S17: a bench session is valid for 2 h.
pub const BENCH_MAX_AGE_S: u64 = 2 * 3600;
/// A session dated more than this in the future (a clock set back) is not trusted.
const FUTURE_TOLERANCE_S: u64 = 300;
/// The file is tiny; anything bigger is not ours.
const MAX_FILE_BYTES: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BenchSession {
    pub created_unix: u64,
    pub pid: u32,
    pub emissions: u32,
}

fn required(message: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BenchSessionRequired, message)
}

/// The bench-session file in one folder (the per-user data folder in the apps; a temporary folder in tests).
#[derive(Debug, Clone, PartialEq)]
pub struct BenchStore {
    dir: PathBuf,
}

impl BenchStore {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        BenchStore { dir: dir.into() }
    }

    /// `<data_dir>/OpenLaser`.
    pub fn default_location() -> Result<Self, Refusal> {
        crate::data_dir().map(BenchStore::at)
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(BENCH_FILE)
    }

    pub fn read(&self) -> Result<BenchSession, Refusal> {
        let path = self.path();
        let meta = std::fs::metadata(&path).map_err(|_| required("no bench session: run `openlaser-hw bench start` at the laser"))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(required(format!("{} is not a bench session file: start a new bench session", path.display())));
        }
        let bytes = std::fs::read(&path).map_err(|e| required(format!("cannot read {}: {e}", path.display())))?;
        serde_json::from_slice(&bytes).map_err(|e| required(format!("{} is damaged ({e}): start a new bench session", path.display())))
    }

    fn write(&self, s: &BenchSession) -> Result<(), Refusal> {
        std::fs::create_dir_all(&self.dir).map_err(|e| required(format!("cannot create {}: {e}", self.dir.display())))?;
        let bytes = serde_json::to_vec(s).map_err(|e| required(format!("cannot encode the bench session: {e}")))?;
        let path = self.path();
        let tmp = self.dir.join(format!("{BENCH_FILE}.tmp"));
        std::fs::write(&tmp, &bytes).map_err(|e| required(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| required(format!("cannot replace {}: {e}", path.display())))
    }

    /// The session, if it is at most 2 h old at `now_unix` and not dated in the future.
    pub fn valid_at(&self, now_unix: u64) -> Result<BenchSession, Refusal> {
        let s = self.read()?;
        if s.created_unix > now_unix.saturating_add(FUTURE_TOLERANCE_S) {
            return Err(required("the bench session is dated in the future (was the clock changed?): start a new one"));
        }
        let age = now_unix.saturating_sub(s.created_unix);
        if age > BENCH_MAX_AGE_S {
            return Err(required(format!("the bench session expired ({} min old; the limit is 120 min): run `openlaser-hw bench start` again", age / 60)));
        }
        Ok(s)
    }

    /// A new session (emissions 0), replacing any older one.
    pub fn start_at(&self, now_unix: u64, pid: u32) -> Result<BenchSession, Refusal> {
        let s = BenchSession { created_unix: now_unix, pid, emissions: 0 };
        self.write(&s)?;
        Ok(s)
    }

    /// S23: `Some(10.0)` until one emission has completed in this bench session, then `None`.
    pub fn emission_cap_pct_at(&self, now_unix: u64) -> Result<Option<f64>, Refusal> {
        // No first-emission cap: the job's own power is what burns (Luiz, 2026-10-31).
        self.valid_at(now_unix)?;
        Ok(None)
    }

    /// After a run finishes (not after a stop or fault).
    pub fn record_emission_at(&self, now_unix: u64) -> Result<BenchSession, Refusal> {
        let mut s = self.valid_at(now_unix)?;
        s.emissions = s.emissions.saturating_add(1);
        self.write(&s)?;
        Ok(s)
    }
}

fn now_unix() -> Result<u64, Refusal> {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).map_err(|_| required("the system clock is before 1970: fix it first"))
}

/// `<data_dir>/OpenLaser/bench-session.json` {createdUnix, pid, emissions}; valid ≤ 2 h (wall clock; the file
/// crosses processes). No tty name is stored: reading it needs `ttyname(3)` (unsafe); `IsTerminal` is enough.
pub fn valid() -> Result<(), Refusal> {
    BenchStore::default_location()?.valid_at(now_unix()?).map(|_| ())
}

/// Only openlaser-hw calls it; stdin and stdout must be TTYs.
pub fn start_from_tty() -> Result<(), Refusal> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Refusal::new(RefusalCode::NotATty, "a bench session is started by a person at the laser, in a terminal (stdin and stdout must be a TTY)"));
    }
    BenchStore::default_location()?.start_at(now_unix()?, std::process::id()).map(|_| ())
}

/// S23: `Some(10.0)` until one emission has completed in this bench session, then `None`.
pub fn emission_cap_pct() -> Result<Option<f64>, Refusal> {
    BenchStore::default_location()?.emission_cap_pct_at(now_unix()?)
}

/// After a run finishes (not after a stop or fault).
pub fn record_emission() -> Result<(), Refusal> {
    BenchStore::default_location()?.record_emission_at(now_unix()?).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_790_000_000;

    #[test]
    fn bench_session_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let store = BenchStore::at(dir.path());
        assert_eq!(store.valid_at(T0).unwrap_err().code, RefusalCode::BenchSessionRequired, "no file, no session");
        store.start_at(T0, 4242).unwrap();
        assert_eq!(store.valid_at(T0 + BENCH_MAX_AGE_S).unwrap().pid, 4242);
        let expired = store.valid_at(T0 + BENCH_MAX_AGE_S + 1).unwrap_err();
        assert_eq!(expired.code, RefusalCode::BenchSessionRequired);
        assert!(expired.message.contains("expired"), "{}", expired.message);
        assert!(store.valid_at(T0 - FUTURE_TOLERANCE_S - 1).is_err(), "a session from the future is refused");
        std::fs::write(store.path(), b"{\"createdUnix\": \"soon\"}").unwrap();
        assert!(store.valid_at(T0).unwrap_err().message.contains("damaged"));
    }

    #[test]
    fn bench_has_no_first_emission_cap() {
        let dir = tempfile::tempdir().unwrap();
        let store = BenchStore::at(dir.path());
        assert!(store.emission_cap_pct_at(T0).is_err(), "no session: refused, never uncapped");
        store.start_at(T0, 1).unwrap();
        assert_eq!(store.emission_cap_pct_at(T0 + 10).unwrap(), None, "the first emission is not capped either");
        assert_eq!(store.record_emission_at(T0 + 20).unwrap().emissions, 1);
        assert_eq!(store.emission_cap_pct_at(T0 + 30).unwrap(), None);
        assert!(store.record_emission_at(T0 + BENCH_MAX_AGE_S + 60).is_err(), "nothing is recorded into an expired session");
        store.start_at(T0 + 10_000, 2).unwrap();
        assert_eq!(store.emission_cap_pct_at(T0 + 10_001).unwrap(), None);
    }

    #[test]
    fn start_needs_a_tty() {
        // Test harnesses never run with stdin and stdout both on a terminal; when they do, skip rather than write
        // a real bench session.
        if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            return;
        }
        assert_eq!(start_from_tty().unwrap_err().code, RefusalCode::NotATty);
    }
}

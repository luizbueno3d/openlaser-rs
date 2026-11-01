//! `--trace <f>` (SPEC §3.21, §8.2): every wire event as one JSON line in the T1 event format of
//! `tests/golden/t1_lmc/*/*.json` (`kind`, and `op` + `hex` for singles, `status` for reads, `sha256` (+ `n_cmds`
//! when known) for list packets) plus a timestamp `atS` in seconds since the session's first event. HIL sessions
//! commit the file; it becomes a FakeLmc fixture.
//!
//! Single-command names follow galvoplotter's controller (meerk40t/galvoplotter, MIT), as R05 Appendix B lists them.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};

use openlaser_lmc_sim::SimWire;
use openlaser_types::MonoTime;
use serde_json::{Map, Value, json};

/// One event on the USB wire.
#[derive(Debug, Clone, PartialEq)]
pub enum WireEvent {
    Open,
    /// A 12-byte single command: six little-endian words.
    Single {
        words: [u16; 6],
    },
    /// Word 3 of an 8-byte reply.
    Read {
        status: u16,
    },
    /// A 3072-byte list packet, by sha256 (the golden packet file name).
    List {
        sha256: String,
        n_cmds: Option<u16>,
    },
    Close,
}

/// The single-command name for an opcode (word 0), as the T1 goldens spell it; unknown opcodes print as hex.
pub fn single_name(op: u16) -> String {
    let name = match op {
        0x0004 => "EnableLaser",
        0x0005 => "ExecuteList",
        0x0006 => "SetPwmPulseWidth",
        0x0007 => "GetVersion",
        0x0009 => "GetSerialNo",
        0x000A => "GetListStatus",
        0x0010 => "WriteCorLine",
        0x0012 => "ResetList",
        0x0015 => "WriteCorTable",
        0x0016 => "SetControlMode",
        0x0017 => "SetDelayMode",
        0x0019 => "SetEndOfList",
        0x001A => "SetFirstPulseKiller",
        0x001B => "SetLaserMode",
        0x001C => "SetTiming",
        0x001D => "SetStandby",
        0x001E => "SetPwmHalfPeriod",
        0x001F => "StopExecute",
        0x0021 => "WritePort",
        0x0022 => "WriteAnalogPort1",
        0x0025 => "ReadPort",
        0x002E => "SetFpkParam2",
        0x0032 => "SetFlyRes",
        0x0033 => "Fiber_SetMo",
        0x003A => "EnableZ",
        0x0040 => "Reset",
        0x0041 => "GetMarkTime",
        other => return format!("0x{other:04X}"),
    };
    name.to_owned()
}

/// Lowercase hex of the 12 bytes a single command puts on the wire.
pub fn single_hex(words: &[u16; 6]) -> String {
    words.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

/// The JSON object of one event, without the timestamp (exactly a T1 `events[]` element).
pub fn event_json(ev: &WireEvent) -> Value {
    match ev {
        WireEvent::Open => json!({ "kind": "open" }),
        WireEvent::Single { words } => json!({ "kind": "single", "op": single_name(words[0]), "hex": single_hex(words) }),
        WireEvent::Read { status } => json!({ "kind": "read", "status": status }),
        WireEvent::List { sha256, n_cmds: Some(n) } => json!({ "kind": "list", "sha256": sha256, "n_cmds": n }),
        WireEvent::List { sha256, n_cmds: None } => json!({ "kind": "list", "sha256": sha256 }),
        WireEvent::Close => json!({ "kind": "close" }),
    }
}

/// One trace line: the T1 object plus `atS`.
pub fn line(ev: &WireEvent, at_s: f64) -> String {
    let mut v = event_json(ev);
    if let Value::Object(m) = &mut v {
        m.insert("atS".to_owned(), json!(at_s));
    }
    v.to_string()
}

/// Parses one T1 golden event object (for fixtures and tests).
pub fn from_t1(v: &Value) -> Option<WireEvent> {
    let m: &Map<String, Value> = v.as_object()?;
    match m.get("kind")?.as_str()? {
        "open" => Some(WireEvent::Open),
        "close" => Some(WireEvent::Close),
        "read" => Some(WireEvent::Read { status: u16::try_from(m.get("status")?.as_u64()?).ok()? }),
        "list" => Some(WireEvent::List {
            sha256: m.get("sha256")?.as_str()?.to_owned(),
            n_cmds: m.get("n_cmds").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()),
        }),
        "single" => {
            let hex = m.get("hex")?.as_str()?;
            let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect::<Option<_>>()?;
            if bytes.len() != 12 {
                return None;
            }
            let mut words = [0u16; 6];
            let (pairs, _) = bytes.as_chunks::<2>();
            for (w, pair) in words.iter_mut().zip(pairs) {
                *w = u16::from_le_bytes(*pair);
            }
            Some(WireEvent::Single { words })
        }
        _ => None,
    }
}

/// The FakeLmc recorder's view of the wire (SimWire carries no open/close).
pub fn from_sim(w: &SimWire) -> (MonoTime, WireEvent) {
    match w {
        SimWire::Single { at, words } => (*at, WireEvent::Single { words: *words }),
        SimWire::Packet { at, sha256 } => (*at, WireEvent::List { sha256: sha256.clone(), n_cmds: None }),
        SimWire::Read { at, status } => (*at, WireEvent::Read { status: *status }),
    }
}

/// Writes trace lines as they come; every line is flushed so a crash or a signal loses nothing already seen.
pub struct TraceWriter {
    out: BufWriter<File>,
    path: PathBuf,
    origin: Option<MonoTime>,
    written: u64,
    /// Bytes were handed to the file (even if the write then failed): it is no longer empty.
    touched: bool,
}

/// A trace that recorded nothing (the command failed before anything reached the wire) is removed, so the same name
/// can be used again; this writer created the file itself (`create_new`), so it never removes anyone else's.
impl Drop for TraceWriter {
    fn drop(&mut self) {
        if !self.touched {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl TraceWriter {
    /// Creates a new trace file. An existing file is never overwritten: a HIL trace is the record of a session.
    pub fn create(path: &Path) -> Result<Self, String> {
        let f = OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| {
            if e.kind() == ErrorKind::AlreadyExists {
                format!("the trace file {} already exists; traces are never overwritten, so give a new file name", path.display())
            } else {
                format!("cannot create the trace file {}: {e}", path.display())
            }
        })?;
        Ok(TraceWriter { out: BufWriter::new(f), path: path.to_path_buf(), origin: None, written: 0, touched: false })
    }

    pub fn write(&mut self, ev: &WireEvent, at: MonoTime) -> Result<(), String> {
        let origin = *self.origin.get_or_insert(at);
        let at_s = at.saturating_since(origin).as_secs_f64();
        // `\n` on every OS (SPEC §2.6): goldens and fixtures are compared as bytes.
        self.touched = true;
        self.out
            .write_all(line(ev, at_s).as_bytes())
            .and_then(|()| self.out.write_all(b"\n"))
            .and_then(|()| self.out.flush())
            .map_err(|e| format!("trace write failed: {e}"))?;
        self.written = self.written.saturating_add(1);
        Ok(())
    }

    pub fn written(&self) -> u64 {
        self.written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_six_little_endian_words() {
        assert_eq!(single_hex(&[0x0009, 0, 0, 0, 0, 0]), "090000000000000000000000");
        assert_eq!(single_hex(&[0x0021, 0x0100, 0, 0, 0, 0]), "210000010000000000000000");
        assert_eq!(single_name(0x0033), "Fiber_SetMo");
        assert_eq!(single_name(0x7777), "0x7777");
    }

    #[test]
    fn lines_carry_a_timestamp_and_nothing_else_extra() {
        let l = line(&WireEvent::Read { status: 32 }, 0.25);
        let v: Value = serde_json::from_str(&l).unwrap();
        assert_eq!(v, json!({ "kind": "read", "status": 32, "atS": 0.25 }));
        assert!(!l.contains('\n'));
    }

    #[test]
    fn writer_appends_flushed_lines_with_relative_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut w = TraceWriter::create(&path).unwrap();
        let t0 = MonoTime(std::time::Duration::from_secs(5));
        w.write(&WireEvent::Open, t0).unwrap();
        w.write(&WireEvent::Single { words: [7, 0, 0, 0, 0, 0] }, t0.plus(std::time::Duration::from_millis(1500))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(text.ends_with('\n') && !text.contains('\r'));
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["atS"], json!(1.5));
        assert_eq!(second["op"], json!("GetVersion"));
        assert_eq!(w.written(), 2);
        // A second trace with the same name is refused and the first is kept.
        drop(w);
        assert!(path.exists(), "a trace with events is kept");
        let err = TraceWriter::create(&path).err().unwrap();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        // One that recorded nothing is removed, so the name can be used again.
        let empty = dir.path().join("empty.jsonl");
        drop(TraceWriter::create(&empty).unwrap());
        assert!(!empty.exists());
        assert!(TraceWriter::create(&empty).is_ok());
    }
}

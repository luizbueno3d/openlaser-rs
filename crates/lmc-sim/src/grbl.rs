//! Deterministic GRBL 1.1 byte-link simulator for fake-only driver tests.

use std::sync::Arc;
use std::time::Duration;

use openlaser_driver::{ByteLink, LinkError};
use openlaser_types::Clock;

pub struct FakeGrbl {
    clock: Arc<dyn Clock>,
    log: Vec<String>,
    rx: Vec<u8>,
    planner: u8,
    alarm: bool,
    paused: bool,
    x: f64,
    y: f64,
}

impl FakeGrbl {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, log: Vec::new(), rx: b"Grbl 1.1h ['$' for help]\r\n".to_vec(), planner: 0, alarm: false, paused: false, x: 0.0, y: 0.0 }
    }
    pub fn log(&self) -> Vec<String> {
        self.log.clone()
    }
    fn status_report(&mut self) {
        let state = if self.alarm {
            "Alarm"
        } else if self.paused {
            "Hold:0"
        } else if self.planner > 0 {
            "Run"
        } else {
            "Idle"
        };
        let free = 15u8.saturating_sub(self.planner);
        self.rx.extend_from_slice(format!("<{state}|MPos:{:.3},{:.3},0.000|FS:0,0|Bf:{free},128>\r\n", self.x, self.y).as_bytes());
    }
    fn motion(&mut self, line: &str) {
        for item in line.split_ascii_whitespace() {
            if let Some(v) = item.strip_prefix('X').and_then(|s| s.parse::<f64>().ok()) {
                self.x = v;
            }
            if let Some(v) = item.strip_prefix('Y').and_then(|s| s.parse::<f64>().ok()) {
                self.y = v;
            }
        }
        if line.starts_with("G0") || line.starts_with("G1") || line.starts_with("G2") || line.starts_with("G3") {
            self.planner = self.planner.saturating_add(1).min(15);
            self.clock.sleep(Duration::from_millis(1));
            self.planner = self.planner.saturating_sub(1);
        }
    }
}

impl ByteLink for FakeGrbl {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        self.clock.sleep(slice.min(Duration::from_millis(2)));
        let input = String::from_utf8_lossy(bytes);
        for line in input.split('\n').map(str::trim).filter(|s| !s.is_empty()) {
            self.log.push(line.to_owned());
            if self.alarm && line != "$X" {
                self.rx.extend_from_slice(b"ALARM:1\r\n");
                continue;
            }
            match line {
                "$$" => self.rx.extend_from_slice(b"$0=10\r\n$10=1\r\nok\r\n"),
                "$I" => self.rx.extend_from_slice(b"[VER:1.1h.20240601:]\r\n[OPT:V,15,128]\r\nok\r\n"),
                "$G" => self.rx.extend_from_slice(b"[GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0]\r\nok\r\n"),
                "$#" => self.rx.extend_from_slice(b"[G54:0.000,0.000,0.000]\r\nok\r\n"),
                "$H" => {
                    self.alarm = false;
                    self.x = 0.0;
                    self.y = 0.0;
                    self.rx.extend_from_slice(b"ok\r\n");
                }
                "$X" => {
                    self.alarm = false;
                    self.rx.extend_from_slice(b"ok\r\n");
                }
                _ if line.starts_with("$J=") => self.rx.extend_from_slice(b"ok\r\n"),
                _ => {
                    self.motion(line);
                    self.rx.extend_from_slice(b"ok\r\n");
                }
            }
        }
        Ok(())
    }
    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        self.log.push(format!("<RT:{byte:02x}>"));
        match byte {
            b'?' => self.status_report(),
            b'!' => self.paused = true,
            b'~' => self.paused = false,
            0x18 => {
                self.alarm = false;
                self.paused = false;
                self.rx.extend_from_slice(b"Grbl 1.1h ['$' for help]\r\n");
            }
            0x85 => self.rx.extend_from_slice(b"[MSG:Pgm End]\r\n"),
            _ => {}
        }
        Ok(())
    }
    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        self.clock.sleep(slice);
        let n = self.rx.len();
        buf.append(&mut self.rx);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_types::VirtualClock;

    #[test]
    fn fake_grbl_streams_a_square_and_replies() {
        let mut fake = FakeGrbl::new(Arc::new(VirtualClock::new()));
        let mut reply = Vec::new();
        assert!(fake.read_available(&mut reply, Duration::ZERO).unwrap_or(0) > 0);
        assert!(String::from_utf8_lossy(&reply).starts_with("Grbl 1.1h"));
        assert!(fake.write_all(b"G21 G90\nG0 X0 Y0\nM3 S100\nG1 X10 Y0 F300\nG1 X10 Y10\nG1 X0 Y10\nG1 X0 Y0\nM5\n", Duration::ZERO).is_ok());
        assert_eq!(fake.log().len(), 8);
        assert!(fake.write_realtime(b'?').is_ok());
        let mut status = Vec::new();
        assert!(fake.read_available(&mut status, Duration::ZERO).unwrap_or(0) > 0);
        assert!(String::from_utf8_lossy(&status).contains("<Idle|MPos:0.000,0.000"));
    }
}

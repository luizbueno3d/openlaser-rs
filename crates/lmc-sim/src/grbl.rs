//! Fake-only GRBL 1.1 model, following `openlaser-rust-plan/rayforge-test/fake_grbl.py`
//! (OpenLaser research, MIT; unversioned source snapshot, see source SHA below).
//! Source SHA-256: 916dd1f6924c0868a71a3fe7a67102471b2389d89872d9045065db8b8244ff9f.
//! The Rust byte parser, bounded RX and deterministic clock executor are independently implemented.

//! Behavioural source attribution: OpenLaser research rayforge-test/fake_grbl.py, MIT (snapshot SHA-256 above; no git commit available).

/*
MIT License

Copyright (c) 2026 Luiz Bueno

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use openlaser_driver::{ByteLink, LinkError};
use openlaser_types::{Clock, MonoTime};

const BANNER: &[u8] = b"Grbl 1.1h ['$' for help]\r\n";
const SETTINGS: &[(u16, &str)] = &[
    (0, "10"),
    (1, "25"),
    (2, "0"),
    (3, "0"),
    (4, "0"),
    (5, "0"),
    (6, "0"),
    (10, "1"),
    (11, "0.010"),
    (12, "0.002"),
    (13, "0"),
    (20, "0"),
    (21, "0"),
    (22, "1"),
    (23, "3"),
    (24, "25.0"),
    (25, "500.0"),
    (26, "250"),
    (27, "1.0"),
    (30, "1000"),
    (31, "0"),
    (32, "1"),
    (100, "80.0"),
    (101, "80.0"),
    (102, "80.0"),
    (110, "6000.0"),
    (111, "6000.0"),
    (112, "500.0"),
    (120, "1000.0"),
    (121, "1000.0"),
    (122, "100.0"),
    (130, "400.0"),
    (131, "400.0"),
    (132, "50.0"),
];
struct Motion {
    target: [f64; 3],
    remaining: Duration,
    jog: bool,
}

pub struct FakeGrbl {
    clock: Arc<dyn Clock>,
    last: MonoTime,
    log: Vec<String>,
    output: Vec<u8>,
    input: VecDeque<u8>,
    queue: VecDeque<Motion>,
    alarm: Option<u8>,
    next_error: Option<u8>,
    paused: bool,
    home_until: Option<MonoTime>,
    pos: [f64; 3],
    target: [f64; 3],
    feed: f64,
    spindle: f64,
    laser_on: bool,
    absolute: bool,
    linear: bool,
    speedup: f64,
}
impl FakeGrbl {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let last = clock.now();
        Self {
            clock,
            last,
            log: vec![],
            output: BANNER.to_vec(),
            input: VecDeque::new(),
            queue: VecDeque::new(),
            alarm: None,
            next_error: None,
            paused: false,
            home_until: None,
            pos: [0.0; 3],
            target: [0.0; 3],
            feed: 1000.0,
            spindle: 0.0,
            laser_on: false,
            absolute: true,
            linear: false,
            speedup: 5.0,
        }
    }
    pub fn log(&self) -> Vec<String> {
        self.log.clone()
    }
    /// Current planner blocks and RX characters; query after a ByteLink operation to synchronise time.
    pub fn buffer_usage(&self) -> (usize, usize) {
        (self.queue.len(), self.input.len())
    }
    pub fn position(&self) -> [f64; 3] {
        self.pos
    }
    pub fn laser_on(&self) -> bool {
        self.laser_on
    }
    pub fn set_speedup(&mut self, value: f64) -> Result<(), LinkError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(LinkError::Io("speedup must be finite and positive".into()));
        }
        self.advance();
        self.speedup = value;
        Ok(())
    }
    pub fn inject_alarm(&mut self, code: u8) {
        self.advance();
        self.queue.clear();
        self.target = self.pos;
        self.alarm = Some(code);
        self.laser_on = false;
        self.output.extend_from_slice(format!("ALARM:{code}\r\n").as_bytes());
    }
    /// Return a deterministic controller error for the next complete command.
    pub fn inject_error_next(&mut self, code: u8) {
        self.next_error = Some(code);
    }
    fn reply(&mut self, text: &str) {
        self.output.extend_from_slice(text.as_bytes());
    }
    fn error(&mut self, code: u8) {
        self.reply(&format!("error:{code}\r\n"));
    }
    fn status_report(&mut self) {
        let state = if self.home_until.is_some() {
            "Home"
        } else if self.alarm.is_some() {
            "Alarm"
        } else if self.paused && !self.queue.is_empty() {
            "Hold:0"
        } else if self.queue.front().is_some_and(|m| m.jog) {
            "Jog"
        } else if !self.queue.is_empty() {
            "Run"
        } else {
            "Idle"
        };
        self.reply(&format!(
            "<{state}|MPos:{:.3},{:.3},{:.3}|Bf:{},{}|FS:{:.0},{:.0}|WCO:0.000,0.000,0.000>\r\n",
            self.pos[0],
            self.pos[1],
            self.pos[2],
            15 - self.queue.len(),
            128 - self.input.len(),
            if self.queue.is_empty() { 0.0 } else { self.feed },
            self.spindle
        ));
    }
    fn advance(&mut self) {
        let now = self.clock.now();
        let mut budget = now.saturating_since(self.last);
        self.last = now;
        if let Some(until) = self.home_until {
            if now < until {
                return;
            }
            self.home_until = None;
            self.pos = [0.0; 3];
            self.target = self.pos;
            budget = now.saturating_since(until);
            self.reply("ok\r\n");
        }
        if self.paused || self.alarm.is_some() {
            return;
        }
        // Interleave pending RX parsing and execution so a full planner withholds ok without dropping lines.
        loop {
            self.process_input();
            let Some(front) = self.queue.front_mut() else {
                break;
            };
            if front.remaining > budget {
                front.remaining = front.remaining.saturating_sub(budget);
                break;
            }
            budget = budget.saturating_sub(front.remaining);
            if let Some(done) = self.queue.pop_front() {
                self.pos = done.target;
            }
        }
    }
    fn process_input(&mut self) {
        while self.queue.len() < 15 && self.home_until.is_none() {
            let Some(end) = self.input.iter().position(|b| matches!(b, b'\n' | b'\r')) else {
                break;
            };
            let bytes: Vec<_> = self.input.drain(..=end).collect();
            let line = String::from_utf8_lossy(&bytes).trim().to_ascii_uppercase();
            if line.is_empty() {
                continue;
            }
            self.log.push(line.clone());
            if let Some(code) = self.next_error.take() {
                self.error(code);
                continue;
            }
            if self.alarm.is_some() && !matches!(line.as_str(), "$X" | "$H" | "$$" | "$I" | "$G" | "$#") {
                self.error(9);
                continue;
            }
            match self.command(&line) {
                Ok(()) if self.home_until.is_none() => self.reply("ok\r\n"),
                Ok(()) => {}
                Err(code) => self.error(code),
            }
        }
    }
    fn command(&mut self, line: &str) -> Result<(), u8> {
        match line {
            "$$" => {
                for &(key, value) in SETTINGS {
                    self.reply(&format!("${key}={value}\r\n"));
                }
                return Ok(());
            }
            "$I" => {
                self.reply("[VER:1.1h.20190825:fake-grbl]\r\n[OPT:VML,15,128]\r\n");
                return Ok(());
            }
            "$G" => {
                self.reply(&format!(
                    "[GC:{} G54 G17 G21 {} G94 {} M9 T0 F{:.0} S{:.0}]\r\n",
                    if self.linear { "G1" } else { "G0" },
                    if self.absolute { "G90" } else { "G91" },
                    if self.laser_on { "M4" } else { "M5" },
                    self.feed,
                    self.spindle
                ));
                return Ok(());
            }
            "$#" => {
                for key in ["G54", "G55", "G56", "G57", "G58", "G59", "G28", "G30", "G92"] {
                    self.reply(&format!("[{key}:0.000,0.000,0.000]\r\n"));
                }
                self.reply("[TLO:0.000]\r\n[PRB:0.000,0.000,0.000:0]\r\n");
                return Ok(());
            }
            "$X" => {
                self.alarm = None;
                return Ok(());
            }
            "$H" => {
                self.alarm = None;
                self.queue.clear();
                self.home_until = Some(self.clock.now().plus(Duration::try_from_secs_f64(0.3 / self.speedup).map_err(|_| 2u8)?));
                self.laser_on = false;
                return Ok(());
            }
            _ => {}
        }
        let jog = line.starts_with("$J=");
        if line.starts_with('$') && !jog {
            return Err(3);
        }
        let line = if jog { line.get(3..).ok_or(2u8)? } else { line };
        let words = parse_words(line)?;
        let mut absolute = self.absolute;
        let mut linear = self.linear;
        let mut laser = self.laser_on;
        let mut feed = self.feed;
        let mut spindle = self.spindle;
        for &(letter, value) in &words {
            match letter {
                b'G' if value == 0.0 => linear = false,
                b'G' if value == 1.0 => linear = true,
                b'G' if value == 90.0 => absolute = true,
                b'G' if value == 91.0 => absolute = false,
                b'G' if [17.0, 21.0, 54.0, 94.0].contains(&value) => {}
                b'M' if [3.0, 4.0].contains(&value) => laser = true,
                b'M' if value == 5.0 => laser = false,
                b'M' if [8.0, 9.0].contains(&value) => {}
                b'F' if value > 0.0 => feed = value,
                b'S' if value >= 0.0 => spindle = value,
                b'X' | b'Y' | b'Z' => {}
                _ => return Err(20),
            }
        }
        let mut to = self.target;
        let mut moved = false;
        for &(letter, value) in &words {
            if let Some(axis) = b"XYZ".iter().position(|b| *b == letter)
                && let Some(coordinate) = to.get_mut(axis)
            {
                *coordinate = if absolute { value } else { *coordinate + value };
                moved = true;
            }
        }
        if moved {
            if to.iter().any(|v| !v.is_finite()) {
                return Err(2);
            }
            let distance = to.iter().zip(self.target).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
            let rate = if linear || jog { feed } else { 6000.0 };
            let duration = Duration::try_from_secs_f64(distance / (rate / 60.0) / self.speedup).map_err(|_| 2u8)?;
            self.queue.push_back(Motion { target: to, remaining: duration, jog });
            self.target = to;
        }
        if !jog {
            self.absolute = absolute;
            self.linear = linear;
            self.laser_on = laser;
            self.spindle = spindle;
            self.feed = feed;
        }
        Ok(())
    }
}
fn parse_words(line: &str) -> Result<Vec<(u8, f64)>, u8> {
    let mut clean = Vec::new();
    let mut comment = false;
    for b in line.bytes() {
        if b == b';' {
            break;
        }
        if b == b'(' {
            comment = true;
            continue;
        }
        if b == b')' {
            comment = false;
            continue;
        }
        if !comment && !b.is_ascii_whitespace() {
            clean.push(b);
        }
    }
    let mut words = Vec::new();
    let mut i = 0;
    while let Some(&letter) = clean.get(i) {
        if !letter.is_ascii_uppercase() {
            return Err(2);
        }
        i += 1;
        let start = i;
        while clean.get(i).is_some_and(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+')) {
            i += 1;
        }
        let text = std::str::from_utf8(clean.get(start..i).ok_or(2u8)?).map_err(|_| 2u8)?;
        let value = text.parse::<f64>().map_err(|_| 2u8)?;
        if !value.is_finite() {
            return Err(2);
        }
        words.push((letter, value));
    }
    Ok(words)
}
impl ByteLink for FakeGrbl {
    fn write_all(&mut self, bytes: &[u8], _slice: Duration) -> Result<(), LinkError> {
        self.advance();
        for &b in bytes {
            if matches!(b, b'?' | b'!' | b'~' | 0x18 | 0x85) {
                self.write_realtime(b)?;
                continue;
            }
            if self.input.len() >= 128 {
                return Err(LinkError::Timeout);
            }
            self.input.push_back(b);
            self.process_input();
        }
        Ok(())
    }
    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        self.advance();
        self.log.push(format!("<RT:{byte:02x}>"));
        match byte {
            b'?' => self.status_report(),
            b'!' => self.paused = true,
            b'~' => self.paused = false,
            0x18 => {
                self.queue.clear();
                self.input.clear();
                self.paused = false;
                self.home_until = None;
                self.alarm = None;
                self.laser_on = false;
                self.spindle = 0.0;
                self.absolute = true;
                self.linear = false;
                self.target = self.pos;
                self.output.clear();
                self.output.extend_from_slice(BANNER);
            }
            0x85 => {
                self.queue.retain(|m| !m.jog);
                self.target = self.queue.back().map_or(self.pos, |m| m.target);
            }
            _ => {}
        }
        Ok(())
    }
    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        self.clock.sleep(slice);
        self.advance();
        let n = self.output.len();
        buf.append(&mut self.output);
        Ok(n)
    }
}

#[cfg(test)]
#[path = "grbl_tests.rs"]
mod tests;

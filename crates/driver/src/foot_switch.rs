//! The foot switch (SPEC M3, design C3.8): the driver thread reads the board's input port (`ReadPort`) while the
//! machine is idle and turns the samples into presses. A press is never a level: the pedal must first be seen
//! released, then pressed, then (with `full_pulse`, the default) released again, each level held for the debounce
//! time. Any stop, fault, frame or run resets it, so a pedal held through a job, or still down after a stop, has to
//! be let go before it counts again. A press only reports; what it may do (Start, ARM) is decided above the driver.

use std::time::Duration;

use openlaser_machine::FootSwitchCfg;
use openlaser_types::MonoTime;

/// How often the idle driver thread reads the input port.
pub const INPUT_POLL: Duration = Duration::from_millis(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for a debounced release (after start, a reset or a completed press).
    NeedRelease,
    /// Released and steady: waiting for a debounced press.
    Released,
    /// Pressed and steady since `at` (full pulse: waiting for the debounced release that completes the press).
    Pressed,
}

#[derive(Debug, Clone)]
pub struct PedalDetector {
    cfg: FootSwitchCfg,
    phase: Phase,
    /// The level being debounced and when it was first seen.
    candidate: Option<(bool, MonoTime)>,
}

impl PedalDetector {
    pub fn new(cfg: FootSwitchCfg) -> Self {
        PedalDetector { cfg, phase: Phase::NeedRelease, candidate: None }
    }

    /// Forget everything: the pedal must be seen released before the next press counts.
    pub fn reset(&mut self) {
        self.phase = Phase::NeedRelease;
        self.candidate = None;
    }

    /// Whether the pedal reads pressed in this reply (display only).
    pub fn pressed(&self, reply: [u16; 4]) -> bool {
        self.cfg.pressed(reply)
    }

    /// One `ReadPort` reply read at `at`. Returns the time of a completed press (once per press).
    pub fn sample(&mut self, reply: [u16; 4], at: MonoTime) -> Option<MonoTime> {
        let level = self.cfg.pressed(reply);
        let since = match self.candidate {
            Some((l, since)) if l == level => since,
            _ => {
                self.candidate = Some((level, at));
                at
            }
        };
        let debounce = Duration::from_millis(u64::from(self.cfg.debounce_ms));
        if at.saturating_since(since) < debounce {
            return None;
        }
        // `level` has held for the debounce time.
        match (self.phase, level) {
            (Phase::NeedRelease, false) => self.phase = Phase::Released,
            (Phase::Released, true) => {
                self.phase = Phase::Pressed;
                if !self.cfg.full_pulse {
                    self.phase = Phase::NeedRelease;
                    return Some(at);
                }
            }
            (Phase::Pressed, false) => {
                self.phase = Phase::Released;
                return Some(at);
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> MonoTime {
        MonoTime(Duration::from_millis(v))
    }

    const UP: [u16; 4] = [0, 0, 0, 0x20];
    const DOWN: [u16; 4] = [0, 0x8000, 0, 0x20];

    /// Feeds `level` every 10 ms from `from` to `to` (exclusive) and collects the presses.
    fn feed(d: &mut PedalDetector, reply: [u16; 4], from: u64, to: u64) -> Vec<MonoTime> {
        (from..to).step_by(10).filter_map(|t| d.sample(reply, ms(t))).collect()
    }

    #[test]
    fn a_full_pulse_counts_on_the_debounced_release_once() {
        let mut d = PedalDetector::new(FootSwitchCfg::new(1, 15));
        assert!(feed(&mut d, UP, 0, 100).is_empty());
        assert!(feed(&mut d, DOWN, 100, 400).is_empty(), "a press alone never counts with full pulse");
        assert_eq!(feed(&mut d, UP, 400, 600), vec![ms(430)], "counts once the release held 30 ms");
        assert!(feed(&mut d, UP, 600, 1000).is_empty(), "never twice for one press");
    }

    #[test]
    fn a_pedal_down_at_start_must_be_released_first() {
        let mut d = PedalDetector::new(FootSwitchCfg::new(1, 15));
        assert!(feed(&mut d, DOWN, 0, 500).is_empty());
        assert!(feed(&mut d, UP, 500, 700).is_empty(), "the release of a press that began before watching is not a press");
        assert!(feed(&mut d, DOWN, 700, 800).is_empty());
        assert_eq!(feed(&mut d, UP, 800, 900).len(), 1);
    }

    #[test]
    fn bounce_and_noise_shorter_than_the_debounce_never_count() {
        let mut d = PedalDetector::new(FootSwitchCfg::new(1, 15));
        feed(&mut d, UP, 0, 100);
        // 20 ms glitches (2 samples) down, then up again: never a press.
        for k in 0..10u64 {
            let t = 100 + k * 60;
            assert!(feed(&mut d, DOWN, t, t + 20).is_empty());
            assert!(feed(&mut d, UP, t + 20, t + 60).is_empty());
        }
    }

    #[test]
    fn without_full_pulse_the_press_counts_and_needs_a_release_before_the_next() {
        let cfg = FootSwitchCfg { full_pulse: false, ..FootSwitchCfg::new(1, 15) };
        let mut d = PedalDetector::new(cfg);
        feed(&mut d, UP, 0, 100);
        assert_eq!(feed(&mut d, DOWN, 100, 2000), vec![ms(130)], "a held pedal counts once");
        feed(&mut d, UP, 2000, 2100);
        assert_eq!(feed(&mut d, DOWN, 2100, 2200).len(), 1);
    }

    #[test]
    fn active_low_and_reset() {
        let cfg = FootSwitchCfg { active_low: true, ..FootSwitchCfg::new(1, 15) };
        let mut d = PedalDetector::new(cfg);
        // Active low: DOWN (bit set) is released, UP (bit clear) is pressed.
        feed(&mut d, DOWN, 0, 100);
        feed(&mut d, UP, 100, 300);
        d.reset();
        assert!(feed(&mut d, DOWN, 300, 400).is_empty(), "a reset mid-press forgets it");
        feed(&mut d, UP, 400, 500);
        assert_eq!(feed(&mut d, DOWN, 500, 600).len(), 1);
    }
}

//! Monotonic time. Never wall-clock; virtual in tests.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Monotonic time from a `Clock`; virtual in tests. Never wall-clock. Serialize/Deserialize are manual (f64 s, §3.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MonoTime(pub Duration);

impl Serialize for MonoTime {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        crate::serde_secs::serialize(&self.0, s)
    }
}

impl<'de> Deserialize<'de> for MonoTime {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::serde_secs::deserialize(d).map(MonoTime)
    }
}

impl MonoTime {
    pub fn saturating_since(self, earlier: MonoTime) -> Duration {
        self.0.saturating_sub(earlier.0)
    }

    pub fn plus(self, d: Duration) -> MonoTime {
        MonoTime(self.0.saturating_add(d))
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> MonoTime;
    /// Waits at most `d`. A virtual clock advances by `d` instead of sleeping.
    fn sleep(&self, d: Duration);
}

pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        SystemClock { origin: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> MonoTime {
        MonoTime(self.origin.elapsed())
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

pub struct VirtualClock {
    /// A poisoned lock is recovered with `into_inner`.
    now: Mutex<Duration>,
}

impl VirtualClock {
    pub fn new() -> Self {
        VirtualClock { now: Mutex::new(Duration::ZERO) }
    }

    pub fn advance(&self, d: Duration) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        *now = now.saturating_add(d);
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> MonoTime {
        MonoTime(*self.now.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn sleep(&self, d: Duration) {
        self.advance(d);
    }
}

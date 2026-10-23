//! Unit newtypes. Every constructor validates; nothing clamps (SPEC §1.3 rule 2).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UnitError {
    #[error("{what} must be a finite number")]
    NonFinite { what: &'static str },
    #[error("{what} = {value} is outside {min}..={max}")]
    OutOfRange { what: &'static str, value: f64, min: f64, max: f64 },
    #[error("{what} = {value} is not a whole number of hertz")]
    NotWholeHz { what: &'static str, value: f64 },
    /// WP0 addition (docs/SPEC-CHANGES.md): text that does not parse (layer keys, fingerprint hex).
    #[error("{what}: `{text}` is malformed")]
    Malformed { what: &'static str, text: String },
    /// Build-only (SPEC §3.0); removed at I3.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// Range rule of a unit: `lo < v` or `lo <= v`, and `v <= hi`.
const fn in_range(v: f64, lo: f64, lo_exclusive: bool, hi: f64) -> bool {
    v.is_finite() && (if lo_exclusive { v > lo } else { v >= lo }) && v <= hi
}

macro_rules! f64_unit {
    ($(#[$doc:meta])* $name:ident, $what:literal, lo = $lo:expr, lo_exclusive = $lox:literal, hi = $hi:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "f64", into = "f64")]
        pub struct $name(f64);

        impl $name {
            /// The largest valid value (WP0 addition: a const anchor for compile-time literals).
            pub const MAX_VALID: Self = Self($hi);

            pub fn new(v: f64) -> Result<Self, UnitError> {
                if !v.is_finite() {
                    return Err(UnitError::NonFinite { what: $what });
                }
                if !in_range(v, $lo, $lox, $hi) {
                    return Err(UnitError::OutOfRange { what: $what, value: v, min: $lo, max: $hi });
                }
                Ok(Self(v))
            }

            /// The same validation as [`Self::new`], usable in `const` items (WP0 addition).
            pub const fn new_const(v: f64) -> Option<Self> {
                if in_range(v, $lo, $lox, $hi) { Some(Self(v)) } else { None }
            }

            pub fn get(self) -> f64 {
                self.0
            }
        }

        impl TryFrom<f64> for $name {
            type Error = UnitError;
            fn try_from(v: f64) -> Result<Self, UnitError> {
                Self::new(v)
            }
        }

        impl From<$name> for f64 {
            fn from(v: $name) -> f64 {
                v.0
            }
        }
    };
}

f64_unit!(
    /// Millimetres; finite, any sign.
    Mm, "length (mm)", lo = f64::MIN, lo_exclusive = false, hi = f64::MAX
);
f64_unit!(
    /// Millimetres per second; finite, 0 < v <= 100 000.
    MmPerS, "speed (mm/s)", lo = 0.0, lo_exclusive = true, hi = 100_000.0
);
f64_unit!(
    /// Percent; finite, 0..=100.
    Percent, "percent", lo = 0.0, lo_exclusive = false, hi = 100.0
);
f64_unit!(
    /// Nanoseconds; finite, 0 < v <= 100 000. A requested pulse width, snapped by the planner.
    Ns, "pulse width (ns)", lo = 0.0, lo_exclusive = true, hi = 100_000.0
);
f64_unit!(
    /// Microseconds; finite, -100 000..=100 000. Delays (the laser-on TC may be negative).
    Us, "delay (µs)", lo = -100_000.0, lo_exclusive = false, hi = 100_000.0
);
f64_unit!(
    /// Milliseconds; finite, 0..=600 000.
    Ms, "duration (ms)", lo = 0.0, lo_exclusive = false, hi = 600_000.0
);
f64_unit!(
    /// Degrees; finite.
    Deg, "angle (°)", lo = f64::MIN, lo_exclusive = false, hi = f64::MAX
);

/// Frequency in whole hertz (as `.lbrn2` stores it); the UI shows kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Hz(u32); // 1..=10_000_000

impl Hz {
    pub const MIN_HZ: u32 = 1;
    pub const MAX_HZ: u32 = 10_000_000;
    /// The largest valid value (WP0 addition: a const anchor for compile-time literals).
    pub const MAX_VALID: Self = Self(Self::MAX_HZ);

    pub fn new(hz: u32) -> Result<Self, UnitError> {
        Self::new_const(hz).ok_or(UnitError::OutOfRange {
            what: "frequency (Hz)",
            value: f64::from(hz),
            min: f64::from(Self::MIN_HZ),
            max: f64::from(Self::MAX_HZ),
        })
    }

    /// The same validation as [`Self::new`], usable in `const` items (WP0 addition).
    pub const fn new_const(hz: u32) -> Option<Self> {
        if hz >= Self::MIN_HZ && hz <= Self::MAX_HZ { Some(Self(hz)) } else { None }
    }

    /// Refuses non-finite, <= 0 and non-whole Hz; never rounds.
    pub fn from_khz(khz: f64) -> Result<Self, UnitError> {
        const WHAT: &str = "frequency (kHz)";
        if !khz.is_finite() {
            return Err(UnitError::NonFinite { what: WHAT });
        }
        let hz = khz * 1000.0;
        let (lo, hi) = (f64::from(Self::MIN_HZ), f64::from(Self::MAX_HZ));
        if !(lo..=hi).contains(&hz) {
            return Err(UnitError::OutOfRange { what: WHAT, value: khz, min: lo / 1000.0, max: hi / 1000.0 });
        }
        if hz.fract() != 0.0 {
            return Err(UnitError::NotWholeHz { what: WHAT, value: khz });
        }
        // Explicit rule: `hz` is finite, whole and inside 1..=10_000_000, so the cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = hz as u32;
        Self::new(whole)
    }

    pub fn get(self) -> u32 {
        self.0
    }

    /// `hz as f64 / 1000.0`: the value the encoders take.
    pub fn khz(self) -> f64 {
        f64::from(self.0) / 1000.0
    }
}

impl TryFrom<u32> for Hz {
    type Error = UnitError;
    fn try_from(hz: u32) -> Result<Self, UnitError> {
        Self::new(hz)
    }
}

impl From<Hz> for u32 {
    fn from(v: Hz) -> u32 {
        v.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb(pub [u8; 3]);

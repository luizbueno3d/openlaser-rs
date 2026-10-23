//! Ids: objects, LightBurn layers, machines, sessions and plan fingerprints.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::units::UnitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub u64);

/// A LightBurn layer: colour 00..=29 or tool layer T1/T2. JSON: 0..=29, or "C05", "05", "T1", "T2"
/// (case-insensitive). Display: "C05", "T1".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LayerKey {
    Color(u8),
    Tool(u8),
}

impl LayerKey {
    pub const COLORS: u8 = 30;

    /// 0..=29.
    pub fn color(index: u8) -> Result<Self, UnitError> {
        if index < Self::COLORS {
            Ok(LayerKey::Color(index))
        } else {
            Err(UnitError::OutOfRange { what: "colour layer", value: f64::from(index), min: 0.0, max: 29.0 })
        }
    }

    /// 1 | 2.
    pub fn tool(which: u8) -> Result<Self, UnitError> {
        if which == 1 || which == 2 {
            Ok(LayerKey::Tool(which))
        } else {
            Err(UnitError::OutOfRange { what: "tool layer", value: f64::from(which), min: 1.0, max: 2.0 })
        }
    }

    pub fn is_tool(self) -> bool {
        matches!(self, LayerKey::Tool(_))
    }

    /// 0..=31 (T1 = 30, T2 = 31).
    pub fn palette_slot(self) -> usize {
        match self {
            LayerKey::Color(i) => usize::from(i),
            LayerKey::Tool(t) => usize::from(Self::COLORS) + usize::from(t.saturating_sub(1)),
        }
    }
}

impl fmt::Display for LayerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayerKey::Color(i) => write!(f, "C{i:02}"),
            LayerKey::Tool(t) => write!(f, "T{t}"),
        }
    }
}

impl FromStr for LayerKey {
    type Err = UnitError;

    fn from_str(s: &str) -> Result<Self, UnitError> {
        let malformed = || UnitError::Malformed { what: "layer key", text: s.to_owned() };
        let t = s.trim();
        let (tool, digits) = match t.chars().next() {
            Some('T' | 't') => (true, t.get(1..).unwrap_or_default()),
            Some('C' | 'c') => (false, t.get(1..).unwrap_or_default()),
            _ => (false, t),
        };
        if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(malformed());
        }
        let n: u8 = digits.parse().map_err(|_| malformed())?;
        if tool { LayerKey::tool(n) } else { LayerKey::color(n) }
    }
}

impl Serialize for LayerKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LayerKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = LayerKey;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a layer key: 0..=29, \"C05\", \"05\", \"T1\" or \"T2\"")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<LayerKey, E> {
                let i = u8::try_from(v).map_err(|_| E::custom(format!("colour layer {v} is outside 0..=29")))?;
                LayerKey::color(i).map_err(E::custom)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<LayerKey, E> {
                let u = u64::try_from(v).map_err(|_| E::custom(format!("colour layer {v} is outside 0..=29")))?;
                self.visit_u64(u)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<LayerKey, E> {
                v.parse().map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineId(pub String);

/// Moved here from `doc` (driver, plan, safety and lmc all name it; driver has no `doc` edge). `doc` re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    GalvoLmc,
    Grbl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// blake3 of a canonical encoding (plan fingerprint, S21) or of wire bytes (`bytes_hash`, S22). JSON: 64 lowercase
/// hex. Display, Debug and `short()`: first 6 hex digits. Debug/Display/Serialize/Deserialize are MANUAL impls (§3.0).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(hex_digit(b >> 4));
            s.push(hex_digit(b & 0x0f));
        }
        s
    }

    pub fn short(&self) -> String {
        self.hex().chars().take(6).collect()
    }

    pub fn from_hex(s: &str) -> Result<Self, UnitError> {
        let malformed = || UnitError::Malformed { what: "fingerprint (64 lowercase hex)", text: s.to_owned() };
        let bytes = s.as_bytes();
        if bytes.len() != 64 {
            return Err(malformed());
        }
        let mut out = [0u8; 32];
        let (pairs, _) = bytes.as_chunks::<2>();
        for (o, &[hi, lo]) in out.iter_mut().zip(pairs) {
            let (Some(hi), Some(lo)) = (hex_value(hi), hex_value(lo)) else { return Err(malformed()) };
            *o = (hi << 4) | lo;
        }
        Ok(Fingerprint(out))
    }
}

fn hex_digit(n: u8) -> char {
    char::from_digit(u32::from(n), 16).unwrap_or('0')
}

/// Lowercase only: the wire form is canonical.
fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Fingerprint::from_hex(&s).map_err(de::Error::custom)
    }
}

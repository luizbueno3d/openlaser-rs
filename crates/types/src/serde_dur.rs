//! `Duration` in JSON (SPEC §3.0, S2): f64 seconds under keys ending in `S`, u64 milliseconds under keys ending in
//! `Ms`; never serde's default `{secs, nanos}`. Use with `#[serde(rename = "estimateS", with = "openlaser_types::serde_secs")]`.

/// f64 seconds. Deserialize refuses negative, non-finite and out-of-range values.
pub mod serde_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(d.as_secs_f64())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = f64::deserialize(d)?;
        Duration::try_from_secs_f64(secs).map_err(|_| de::Error::custom(format!("{secs} s is not a valid duration (finite, >= 0)")))
    }
}

/// u64 milliseconds (sub-millisecond parts are truncated on output).
pub mod serde_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct T {
        #[serde(rename = "estimateS", with = "crate::serde_secs")]
        estimate: Duration,
        #[serde(rename = "elapsedMs", with = "crate::serde_ms")]
        elapsed: Duration,
    }

    #[test]
    fn durations_use_unit_keys() {
        let t = T { estimate: Duration::from_millis(3100), elapsed: Duration::from_millis(141) };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#"{"estimateS":3.1,"elapsedMs":141}"#);
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), t);
        assert!(serde_json::from_str::<T>(r#"{"estimateS":-1.0,"elapsedMs":0}"#).is_err());
    }
}

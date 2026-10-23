//! OM prototype imports.

use serde::Deserialize;

use crate::Error;

/// OM calibration.json → scale_correction; refuses outside 0.5..=2.0 (OM clamps; we refuse).
pub fn parse_calibration(bytes: &[u8]) -> Result<f64, Error> {
    #[derive(Deserialize)]
    struct Calibration {
        scale_correction: f64,
    }
    let value: Calibration = openlaser_doc::deserialize_json(bytes).map_err(Error::InvalidCalibration)?;
    if !value.scale_correction.is_finite() || !(0.5..=2.0).contains(&value.scale_correction) {
        return Err(Error::InvalidCalibration(format!("scale_correction {} is outside 0.5..=2.0", value.scale_correction)));
    }
    Ok(value.scale_correction)
}

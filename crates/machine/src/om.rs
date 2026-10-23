//! OM prototype imports. WP0 stub.

use crate::Error;

/// OM calibration.json → scale_correction; refuses outside 0.5..=2.0 (OM clamps; we refuse).
pub fn parse_calibration(bytes: &[u8]) -> Result<f64, Error> {
    let _ = bytes;
    Err(Error::NotImplemented("openlaser_machine::om::parse_calibration"))
}

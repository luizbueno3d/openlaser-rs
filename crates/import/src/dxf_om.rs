//! Exact port of OM `importers.parse_dxf` / `import_geometry` (T4: counts exact, coords within 1e-9).
//! SPLINE: OM uses control/fit points; `parse_dxf_om` keeps that, the shipping importer evaluates the spline
//! (listed divergence; T4 SPLINE rows are compared against `parse_dxf_om`). WP0 stubs.

use crate::{Error, Polyline};

pub fn parse_dxf_om(bytes: &[u8]) -> Result<Vec<Polyline>, Error> {
    let _ = bytes;
    Err(Error::NotImplemented("openlaser_import::dxf_om::parse_dxf_om"))
}

pub fn import_geometry_om(name: &str, bytes: &[u8]) -> Result<Vec<Polyline>, Error> {
    let _ = (name, bytes);
    Err(Error::NotImplemented("openlaser_import::dxf_om::import_geometry_om"))
}

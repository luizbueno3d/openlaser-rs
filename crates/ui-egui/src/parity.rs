//! Menu parity: built commands against the catalogue. `cargo xtask parity` writes docs/parity-checklist.md.
//! WP0 stub.

use openlaser_engine::Registry;

/// The parity floor only rises.
pub const FLOOR: usize = 0;

/// (live, catalogued). WP0 stub: nothing is live.
pub fn live_count(reg: &Registry) -> (usize, usize) {
    let _ = reg;
    (0, crate::menu_catalog::CATALOG.len())
}

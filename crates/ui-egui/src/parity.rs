//! Menu parity: built commands against the catalogue. `cargo xtask parity` writes docs/parity-checklist.md.
//! Live command coverage against the frozen menu catalogue.

use openlaser_engine::Registry;

/// Current live engine and shell menu rows. This floor only rises.
pub const FLOOR: usize = 20;

/// (live, catalogued). WP0 stub: nothing is live.
pub fn live_count(reg: &Registry) -> (usize, usize) {
    let rows: Vec<_> = crate::menu_catalog::CATALOG.iter().filter(|row| !crate::menu_catalog::is_separator(row)).collect();
    let live = rows.iter().filter(|row| !crate::menu_catalog::is_hw_row(row) && (reg.find(row.id).is_some() || crate::menu_catalog::is_shell_row(row))).count();
    (live, rows.len())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parity_floor() {
        let (live, catalogued) = super::live_count(openlaser_engine::registry());
        assert!(live >= super::FLOOR, "live menu count regressed: {live} < {}", super::FLOOR);
        assert!(catalogued >= live);
    }

    #[test]
    fn menu_catalog_has_no_human_only_ids() {
        for row in crate::menu_catalog::CATALOG.iter().filter(|row| !crate::menu_catalog::is_separator(row)) {
            assert!(row.shortcut.is_none() || !crate::menu_catalog::is_hw_row(row), "human-only row has shortcut: {row:?}");
            assert!(!row.id.starts_with("arm:"), "ARM id in menu: {row:?}");
            if let Some(spec) = openlaser_engine::registry().find(row.id) {
                assert_ne!(spec.access, openlaser_engine::Access::Human, "human-only command in menu: {row:?}");
            }
        }
    }
}

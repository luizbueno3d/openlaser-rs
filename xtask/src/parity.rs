//! `cargo xtask parity`: writes `docs/parity-checklist.md` from ui-egui's menu catalogue and the engine registry.
//!
//! WP0 stub: the catalogue is empty until WP8 fills `crates/ui-egui/src/menu_catalog.rs`; WP8 wires this task
//! (SPEC §1.1, §7.9). It reports and succeeds so `cargo xtask ci` stays green meanwhile.

use std::path::Path;

pub fn run(root: &Path) -> Result<(), String> {
    let catalog = root.join("crates").join("ui-egui").join("src").join("menu_catalog.rs");
    if !catalog.is_file() {
        return Err(format!("{} is missing", catalog.display()));
    }
    println!("parity: the menu catalogue is filled by WP8; nothing to report yet");
    Ok(())
}

//! Toolbar and palette icons: SVGs embedded with `include_bytes!`, recoloured to white by `craft_ui::icons::SvgIconSet`,
//! rasterised by `egui_extras` at the exact pixel size and tinted per use. Pattern from PhotoCraft
//! `crates/ui-egui/src/icons.rs` (MIT OR Apache-2.0). Every file has a row in `crates/ui-egui/ATTRIBUTION.md`
//! (`every_icon_has_an_attribution_row`). No LightBurn icon is used (AGENTS.md §3).

/// (name, SVG bytes).
pub static ICONS: &[(&str, &[u8])] = &[
    ("camera", include_bytes!("../assets/icons/camera.svg")),
    ("circle", include_bytes!("../assets/icons/circle.svg")),
    ("clipboard-paste", include_bytes!("../assets/icons/clipboard-paste.svg")),
    ("copy", include_bytes!("../assets/icons/copy.svg")),
    ("eye", include_bytes!("../assets/icons/eye.svg")),
    ("file-input", include_bytes!("../assets/icons/file-input.svg")),
    ("file-plus", include_bytes!("../assets/icons/file-plus.svg")),
    ("flip-horizontal-2", include_bytes!("../assets/icons/flip-horizontal-2.svg")),
    ("flip-vertical-2", include_bytes!("../assets/icons/flip-vertical-2.svg")),
    ("folder-open", include_bytes!("../assets/icons/folder-open.svg")),
    ("hand", include_bytes!("../assets/icons/hand.svg")),
    ("layers", include_bytes!("../assets/icons/layers.svg")),
    ("link-2", include_bytes!("../assets/icons/link-2.svg")),
    ("lock-open", include_bytes!("../assets/icons/lock-open.svg")),
    ("lock", include_bytes!("../assets/icons/lock.svg")),
    ("maximize", include_bytes!("../assets/icons/maximize.svg")),
    ("mouse-pointer-2", include_bytes!("../assets/icons/mouse-pointer-2.svg")),
    ("move", include_bytes!("../assets/icons/move.svg")),
    ("pen-tool", include_bytes!("../assets/icons/pen-tool.svg")),
    ("pentagon", include_bytes!("../assets/icons/pentagon.svg")),
    ("rectangle-horizontal", include_bytes!("../assets/icons/rectangle-horizontal.svg")),
    ("redo-2", include_bytes!("../assets/icons/redo-2.svg")),
    ("rotate-ccw", include_bytes!("../assets/icons/rotate-ccw.svg")),
    ("rotate-cw", include_bytes!("../assets/icons/rotate-cw.svg")),
    ("ruler", include_bytes!("../assets/icons/ruler.svg")),
    ("save", include_bytes!("../assets/icons/save.svg")),
    ("scan", include_bytes!("../assets/icons/scan.svg")),
    ("scissors", include_bytes!("../assets/icons/scissors.svg")),
    ("settings-2", include_bytes!("../assets/icons/settings-2.svg")),
    ("settings", include_bytes!("../assets/icons/settings.svg")),
    ("spline", include_bytes!("../assets/icons/spline.svg")),
    ("square-terminal", include_bytes!("../assets/icons/square-terminal.svg")),
    ("swatch-book", include_bytes!("../assets/icons/swatch-book.svg")),
    ("trash-2", include_bytes!("../assets/icons/trash-2.svg")),
    ("triangle-alert", include_bytes!("../assets/icons/triangle-alert.svg")),
    ("type", include_bytes!("../assets/icons/type.svg")),
    ("undo-2", include_bytes!("../assets/icons/undo-2.svg")),
    ("unlink", include_bytes!("../assets/icons/unlink.svg")),
    ("wand-sparkles", include_bytes!("../assets/icons/wand-sparkles.svg")),
    ("zoom-in", include_bytes!("../assets/icons/zoom-in.svg")),
    ("zoom-out", include_bytes!("../assets/icons/zoom-out.svg")),
];

#[cfg(test)]
mod tests {
    #[test]
    fn every_icon_has_an_attribution_row() {
        let attribution = include_str!("../ATTRIBUTION.md");
        for (name, bytes) in super::ICONS {
            assert!(!bytes.is_empty());
            assert!(attribution.contains(&format!("`assets/icons/{name}.svg`")), "missing attribution for {name}");
        }
    }
}

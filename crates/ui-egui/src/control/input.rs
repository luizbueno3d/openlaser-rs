//! The only builder of synthetic `egui::Event`s (SPEC §2.3 rule 16): control methods `ui.click`, `ui.key`,
//! `ui.type`, `ui.pointer`, `ui.drag`… become a `Vec<egui::Event>` here, handed to `HumanSurfaces::inject`, which
//! taints the frame. No other path pushes synthetic input into egui. WP0 stub: builds nothing.

/// The events one control method injects. WP0 stub: refuses every method.
pub fn events_for(method: &str, params: &serde_json::Value) -> Result<Vec<egui::Event>, String> {
    let _ = params;
    Err(format!("not implemented: synthetic input for `{method}`"))
}

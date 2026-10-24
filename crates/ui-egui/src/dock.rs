//! Application-owned panel registry and validated craft-ui docking persistence.
use craft_ui::docking::{Layout, Node, Permissions};
use craft_ui::layout::{SplitAxis, SplitSize};

pub const PANELS: &[&str] = &["canvas", "cuts", "laser", "library", "move", "console"];
pub type DockLayout = Layout<String>;

pub fn default_layout() -> DockLayout {
    let tabs = |names: &[&str]| Node::Tabs { panels: names.iter().map(|name| (*name).to_owned()).collect(), active: 0 };
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::Ratio(0.72),
            first: Box::new(tabs(&["canvas"])),
            second: Box::new(Node::Split {
                axis: SplitAxis::Vertical,
                size: SplitSize::Ratio(0.45),
                first: Box::new(tabs(&["cuts"])),
                second: Box::new(tabs(&["laser", "library", "move", "console"])),
            }),
        }),
        floating: Vec::new(),
    }
}
pub fn permissions(panel: &String) -> Permissions {
    if panel == "canvas" { Permissions::PROTECTED } else { Permissions::default() }
}
pub fn decode(value: &str) -> DockLayout {
    if value.len() <= 64 * 1024
        && let Ok(layout) = serde_json::from_str::<DockLayout>(value)
        && layout.validate().is_ok()
        && layout.contains(&"canvas".to_owned())
        && layout.panels().iter().all(|p| PANELS.contains(&p.as_str()))
    {
        return layout;
    }
    default_layout()
}
pub fn encode(layout: &DockLayout) -> String {
    serde_json::to_string(layout).unwrap_or_default()
}
pub fn label(panel: &str) -> String {
    match panel {
        "canvas" => "Design",
        "cuts" => "Cuts / Layers",
        "laser" => "Laser",
        "library" => "Library",
        "move" => "Move",
        "console" => "Console",
        _ => panel,
    }
    .into()
}

pub fn visible_panels(layout: &DockLayout) -> Vec<String> {
    fn collect(node: &Node<String>, out: &mut Vec<String>) {
        match node {
            Node::Tabs { panels, active } => {
                if let Some(panel) = panels.get(*active) {
                    out.push(panel.clone());
                }
            }
            Node::Stack { entries } => out.extend(entries.iter().filter(|e| e.open).map(|e| e.panel.clone())),
            Node::Split { first, second, .. } => {
                collect(first, out);
                collect(second, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = &layout.root {
        collect(root, &mut out);
    }
    for floating in &layout.floating {
        if let Some(panel) = floating.panels.get(floating.active) {
            out.push(panel.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::docking::{Action, Placement, Zone};
    #[test]
    fn dock_layout_roundtrip_and_fallback() {
        let mut moved = default_layout();
        moved.apply_with_permissions(Action::Float { panel: "laser".into(), rect: [80.0, 90.0, 300.0, 400.0] }, permissions).unwrap();
        assert_eq!(decode(&encode(&moved)), moved);
        assert_ne!(moved, default_layout());
        moved
            .apply_with_permissions(Action::Move { panel: "laser".into(), anchor: "cuts".into(), placement: Placement::Split(Zone::Bottom) }, permissions)
            .unwrap();
        assert!(moved.floating.is_empty());
        assert_eq!(decode(&encode(&moved)), moved);
        assert_eq!(decode("garbage"), default_layout());
        assert_eq!(decode(r#"{"root":{"Tabs":{"panels":["canvas","unknown"],"active":0}},"floating":[]}"#), default_layout());
        assert_eq!(decode(r#"{"root":{"Tabs":{"panels":["canvas","canvas"],"active":0}},"floating":[]}"#), default_layout());
        assert!(moved.apply_with_permissions(Action::Close { panel: "canvas".into() }, permissions).is_err());
    }
}

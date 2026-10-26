//! The only builder of synthetic events (§2.3 rule 16). Events always enter HumanSurfaces::inject.
pub fn events_for(method: &str, params: &serde_json::Value) -> Result<Vec<egui::Event>, String> {
    let position = || -> Result<egui::Pos2, String> {
        let x = params.get("x").and_then(serde_json::Value::as_f64).ok_or("x is required")?;
        let y = params.get("y").and_then(serde_json::Value::as_f64).ok_or("y is required")?;
        if !x.is_finite() || !y.is_finite() || x.abs() > 16384.0 || y.abs() > 16384.0 {
            return Err("input position outside viewport limits".into());
        }
        Ok(egui::pos2(x as f32, y as f32))
    };
    match method {
        "ui.type" => Ok(vec![egui::Event::Text(params.get("text").and_then(serde_json::Value::as_str).ok_or("text is required")?.into())]),
        "ui.key" => {
            let key = params.get("key").and_then(serde_json::Value::as_str).and_then(egui::Key::from_name).ok_or("unknown or missing key")?;
            Ok([true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }).to_vec())
        }
        "ui.move" | "ui.pointer" => Ok(vec![egui::Event::PointerMoved(position()?)]),
        "ui.click" => {
            let pos = position()?;
            Ok(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ])
        }
        "ui.drag" => {
            let from = params.get("from").ok_or("from {x,y} is required")?;
            let to = params.get("to").ok_or("to {x,y} is required")?;
            let start = events_for("ui.move", from)?;
            let end = events_for("ui.move", to)?;
            let (Some(egui::Event::PointerMoved(from)), Some(egui::Event::PointerMoved(to))) = (start.first(), end.first()) else {
                return Err("invalid drag endpoints".into());
            };
            Ok(vec![
                egui::Event::PointerMoved(*from),
                egui::Event::PointerButton { pos: *from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerMoved(*to),
                egui::Event::PointerButton { pos: *to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ])
        }
        _ => Err(format!("unknown input method `{method}`")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn drag_has_validated_endpoints_press_motion_release() {
        let events = super::events_for("ui.drag", &serde_json::json!({"from":{"x":10,"y":20},"to":{"x":30,"y":40}})).unwrap();
        assert!(matches!(
            events.as_slice(),
            [
                egui::Event::PointerMoved(_),
                egui::Event::PointerButton { pressed: true, .. },
                egui::Event::PointerMoved(_),
                egui::Event::PointerButton { pressed: false, .. }
            ]
        ));
        assert!(super::events_for("ui.drag", &serde_json::json!({"from":{"x":10,"y":20},"to":{"x":30000,"y":40}})).is_err());
    }
}

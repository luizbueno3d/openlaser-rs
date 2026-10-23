//! GUI-thread control dispatch. Human-only commands are fenced before reaching the engine.

pub mod input;

use std::sync::mpsc::Sender;

use openlaser_control::server::ControlRequest;
use openlaser_control::wire::Reply;
use openlaser_engine::{Access, Origin};

use crate::app::OpenLaserApp;

pub enum Outcome {
    Done(Reply),
    AfterInput(Reply),
    Screenshot { max_side: u32, reply: Sender<Reply> },
}

pub fn handle(app: &mut OpenLaserApp, ctx: &egui::Context, req: ControlRequest) -> Outcome {
    let _ = ctx;
    let id = req.request.id.clone();
    let method = req.request.method.as_str();
    let params = &req.request.params;
    let result: Result<serde_json::Value, String> = match method {
        "methods" => Ok(serde_json::json!({"methods": openlaser_control::methods::METHODS})),
        "machine.stop" => {
            app.stop_from_control();
            Ok(serde_json::json!({"stopped": true}))
        }
        "ui.inspect" => Ok(serde_json::json!({
            "activeLayer": app.ui_state().active_layer,
            "activePanel": app.ui_state().active_panel,
            "tool": app.ui_state().selected_tool,
            "armDialog": "closed",
            "window": "OpenLaser"
        })),
        "ui.elements" => Ok(serde_json::json!({"elements": ["Stop laser", "Cuts / Layers", "Laser", "Layer 00", "Layer 29", "Layer T1", "Layer T2"]})),
        "ui.menu.list" => Ok(
            serde_json::json!({"items": crate::menu_catalog::CATALOG.iter().map(|r| serde_json::json!({"path": r.path, "label": r.label, "shortcut": r.shortcut, "id": r.id})).collect::<Vec<_>>() }),
        ),
        "ui.menu.invoke" => {
            let command = params.get("id").or_else(|| params.get("command")).and_then(serde_json::Value::as_str).unwrap_or_default();
            if let Some(row) = crate::menu_catalog::CATALOG.iter().find(|row| row.id == command) {
                if crate::menu_catalog::is_hw_row(row) {
                    Err(format!("human-only: {command} drives laser hardware; ask the operator"))
                } else {
                    execute_command(app, command, params.get("params").cloned().unwrap_or(serde_json::Value::Null))
                }
            } else {
                Err(format!("unknown menu command `{command}`"))
            }
        }
        "engine.execute" => {
            let command = params.get("command").and_then(serde_json::Value::as_str).unwrap_or_default();
            execute_command(app, command, params.get("params").cloned().unwrap_or(serde_json::Value::Null))
        }
        "engine.commands" => Ok(serde_json::json!({"commands": app.session().command_list(Origin::Synthetic, &Default::default())})),
        "machine.list" => {
            Ok(serde_json::json!({"machines": app.session().machines().iter().map(|m| serde_json::json!({"id": m.id.0, "name": m.name})).collect::<Vec<_>>() }))
        }
        "ui.screenshot" => Err("screenshot rendering is unavailable".to_owned()),
        method if openlaser_control::methods::METHODS.contains(&method) => Err(format!("method `{method}` is not available in this build")),
        _ => Err(format!("unknown method `{method}`")),
    };
    let reply = match result {
        Ok(value) => Reply { id, ok: true, result: Some(value), error: None },
        Err(error) => Reply { id, ok: false, result: None, error: Some(error) },
    };
    Outcome::Done(reply)
}

fn execute_command(app: &mut OpenLaserApp, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(spec) = openlaser_engine::registry().find(command) else {
        return Err(format!("unknown command `{command}`"));
    };
    if spec.access == Access::Human {
        return Err(format!("human-only: {command} drives laser hardware; ask the operator"));
    }
    app.session_mut().execute_as(Origin::Synthetic, command, params).map_err(|error| error.to_string())
}

pub(crate) fn error_reply(id: serde_json::Value, error: &str) -> Reply {
    Reply { id, ok: false, result: None, error: Some(error.to_owned()) }
}

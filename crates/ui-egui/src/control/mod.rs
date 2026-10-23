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
    Screenshot { id: serde_json::Value, max_side: u32, reply: Sender<Reply> },
}

/// Frozen §4.5 routes, also exercised while WP7's exported table is still empty.
pub const ROUTES: &[&str] = &[
    "methods",
    "machine.stop",
    "ui.inspect",
    "ui.elements",
    "ui.menu.list",
    "ui.menu.invoke",
    "engine.execute",
    "engine.commands",
    "engine.schema",
    "machine.list",
    "machine.status",
    "ui.screenshot",
    "jobs.list",
    "jobs.cancel",
    "app.open",
    "app.import",
    "app.save",
    "app.export",
    "doc.inspect",
    "doc.render",
    "doc.select",
    "doc.close",
    "session.list",
    "job.plan",
    "job.preview",
    "job.estimate",
    "job.simulate",
    "job.export",
    "gcode.export",
    "frame.request",
    "frame.requests",
    "job.submit",
    "job.status",
    "job.withdraw",
    "ui.focus",
    "ui.resize",
    "ui.set",
    "ui.dialog.open",
    "ui.dialog.set",
    "ui.dialog.confirm",
    "ui.dialog.cancel",
    "ui.pointer",
    "ui.click",
    "ui.move",
    "ui.drag",
    "ui.key",
    "ui.type",
    "ui.dock.move",
    "ui.dock.float",
    "ui.dock.close",
    "ui.dock.open",
    "ui.dock.resize",
    "app.quit",
];

fn mutates(method: &str, params: &serde_json::Value) -> bool {
    if matches!(method, "engine.execute" | "ui.menu.invoke") {
        let id = params.get("command").or_else(|| params.get("id")).and_then(serde_json::Value::as_str).unwrap_or_default();
        return !openlaser_engine::registry().find(id).is_some_and(|spec| matches!(spec.kind, openlaser_engine::Kind::Query) || spec.access == Access::Stop);
    }
    !matches!(
        method,
        "methods"
            | "machine.stop"
            | "engine.commands"
            | "engine.schema"
            | "jobs.list"
            | "doc.inspect"
            | "doc.render"
            | "session.list"
            | "machine.list"
            | "machine.status"
            | "job.plan"
            | "job.preview"
            | "job.estimate"
            | "job.simulate"
            | "frame.requests"
            | "job.status"
            | "ui.inspect"
            | "ui.elements"
            | "ui.menu.list"
            | "ui.screenshot"
    )
}

pub fn handle(app: &mut OpenLaserApp, ctx: &egui::Context, req: ControlRequest) -> Outcome {
    let id = req.request.id.clone();
    let method = req.request.method.as_str();
    let params = &req.request.params;
    if !ROUTES.contains(&method) {
        return Outcome::Done(error_reply(id, &format!("unknown method `{method}`")));
    }
    if matches!(method, "engine.execute" | "ui.menu.invoke") && contains_human_command(params) {
        return Outcome::Done(error_reply(id, "human-only: hardware actions require the operator"));
    }
    if mutates(method, params) {
        let refusal = if app.is_machine_busy() {
            Some("machine_busy")
        } else if app.human_surface_open() {
            Some("human_confirmation_open")
        } else {
            None
        };
        if let Some(refusal) = refusal {
            return Outcome::Done(error_reply(id, refusal));
        }
    }
    let result: Result<serde_json::Value, String> = match method {
        "methods" => Ok(serde_json::json!({"methods": ROUTES})),
        "machine.stop" => {
            app.stop_from_control();
            Ok(serde_json::json!({"stopped": true}))
        }
        "ui.inspect" => Ok(serde_json::json!({
            "activeLayer": app.ui_state().active_layer,
            "activePanel": app.ui_state().active_panel,
            "tool": app.ui_state().selected_tool,
            "armDialog": if app.arm_dialog_open() { "open" } else { "closed" },
            "window": "OpenLaser"
        })),
        "ui.elements" => Ok(serde_json::json!({"elements": app.element_labels()})),
        "ui.menu.list" => Ok(
            serde_json::json!({"items": crate::menu_catalog::CATALOG.iter().map(|r| serde_json::json!({"path": r.path, "label": r.label, "shortcut": r.shortcut, "id": r.id,"params": r.panel.map(|panel|serde_json::json!({"panel":panel,"visible":true}))})).collect::<Vec<_>>() }),
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
        "ui.screenshot" => {
            let side = params.get("maxSide").and_then(serde_json::Value::as_u64).unwrap_or(2048);
            if side == 0 || side > 2048 {
                Err("maxSide must be between 1 and 2048".to_owned())
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                return Outcome::Screenshot { id, max_side: side as u32, reply: req.reply };
            }
        }
        "engine.schema" => app.session().command_schema(params.get("id").and_then(serde_json::Value::as_str).unwrap_or_default()).map_err(|e| e.to_string()),
        "doc.inspect" => app
            .session()
            .inspect(
                params.get("index").and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok()),
                params.get("objects").and_then(serde_json::Value::as_bool).unwrap_or(false),
            )
            .map_err(|e| e.to_string()),
        "frame.requests" => Ok(serde_json::json!({"requests": app.session().requests().frame_requests()})),
        "job.status" => Ok(serde_json::json!({"submissions": app.session().requests().submissions()})),
        "job.submit" => app
            .session_mut()
            .submit_job(&req.client, params.get("note").and_then(serde_json::Value::as_str).unwrap_or_default())
            .map(|v| serde_json::json!(v))
            .map_err(|e| e.to_string()),
        "frame.request" => {
            let mode = match params.get("mode").and_then(serde_json::Value::as_str).unwrap_or("once") {
                "once" => Ok(openlaser_types::FrameMode::Once),
                "loop" => Ok(openlaser_types::FrameMode::Loop),
                _ => Err("mode must be once or loop".to_owned()),
            };
            mode.and_then(|mode| app.session_mut().request_frame(&req.client, mode).map(|v| serde_json::json!(v)).map_err(|e| e.to_string()))
        }
        "job.withdraw" => params
            .get("submission")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "submission is required".to_owned())
            .and_then(|id| app.session_mut().withdraw_request(openlaser_engine::RequestId(id)).map(|_| serde_json::json!({})).map_err(|e| e.to_string())),
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            Ok(serde_json::json!({}))
        }
        "ui.resize" => app.resize_window(ctx, params),
        "app.quit" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            Ok(serde_json::json!({}))
        }
        "ui.click" | "ui.pointer" | "ui.move" | "ui.key" | "ui.type" => input::events_for(method, params).map(|events| {
            app.queue_input(events);
            serde_json::json!({})
        }),
        "ui.drag" => input::events_for(method, params).map(|events| {
            app.queue_drag(events);
            serde_json::json!({})
        }),
        "ui.dialog.open" | "ui.dialog.set" | "ui.dialog.confirm" | "ui.dialog.cancel" => app.control_dialog(method, params),
        "ui.dock.move" | "ui.dock.float" | "ui.dock.close" | "ui.dock.open" | "ui.dock.resize" => app.control_dock(method, params),
        "ui.set" => app.control_set(params),
        "jobs.list" | "jobs.cancel" | "app.open" | "app.import" | "app.save" | "app.export" | "doc.render" | "doc.select" | "doc.close" | "session.list"
        | "machine.status" | "job.plan" | "job.preview" | "job.estimate" | "job.simulate" | "job.export" | "gcode.export" => {
            let command = match method {
                "app.open" => "file.open",
                "app.import" => "file.import",
                "app.save" => "file.save",
                "app.export" => "file.export",
                "doc.select" => "document.select",
                "doc.close" => "file.close",
                "job.export" => "job.dump",
                method => method,
            };
            app.session_mut().execute_as(Origin::Synthetic, command, params.clone()).map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown method `{method}`")),
    };
    let reply = match result {
        Ok(value) => Reply { id, ok: true, result: Some(value), error: None },
        Err(error) => Reply { id, ok: false, result: None, error: Some(error) },
    };
    Outcome::Done(reply)
}

fn execute_command(app: &mut OpenLaserApp, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    if human_only_command(command) {
        return Err(format!("human-only: {command}; ask the operator"));
    }
    if command == "window.show" {
        return app.show_panel(&params);
    }
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

pub fn human_only_command(id: &str) -> bool {
    id.starts_with("hw:")
        || matches!(
            id,
            "device.connect"
                | "device.disconnect"
                | "device.detect"
                | "device.settings.set"
                | "device.create"
                | "device.importEzcad"
                | "device.calibrateAxis"
                | "laser.frame"
                | "laser.start"
                | "laser.arm"
                | "laser.mark"
                | "laser.run"
                | "laser.redLight"
                | "laser.home"
                | "laser.jog"
        )
        || openlaser_engine::registry().find(id).is_some_and(|spec| spec.access == Access::Human)
}
fn contains_human_command(params: &serde_json::Value) -> bool {
    match params {
        serde_json::Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| ((key == "command" || key == "id") && value.as_str().is_some_and(human_only_command)) || contains_human_command(value)),
        serde_json::Value::Array(values) => values.iter().any(contains_human_command),
        _ => false,
    }
}

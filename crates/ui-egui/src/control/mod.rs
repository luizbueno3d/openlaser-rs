//! The GUI-thread control dispatcher (SPEC §4.5). Requests from the loopback control server arrive here on the UI
//! thread; `machine.stop` is also answered on the transport thread by the app's fast path.
//!
//! Fences, in order: (1) only the exact §4.5 method table is routed (anything else: `unknown method`); (1b) a
//! request carrying both command-identifying fields (`command` and `id`) is refused as ambiguous (`conflicting
//! command identifiers`); (2) any
//! human-only command id anywhere in the params, through `engine.execute`, `ui.menu.invoke` or a batch, is refused
//! (`human-only:`); (3) while a machine is framing, running or stopping, or a human-only surface (the ARM dialog) is
//! open, every mutating method is refused (`machine_busy` / `human_confirmation_open`); reads, `ui.screenshot` and
//! `machine.stop` are always served. Everything the dispatcher runs is `Origin::Synthetic`.

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

/// The §4.5 method table, exactly (`control_routes_match_spec_table`).
pub const ROUTES: &[&str] = &[
    "engine.execute",
    "engine.commands",
    "engine.schema",
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
    "machine.list",
    "machine.status",
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
    "machine.stop",
    "ui.inspect",
    "ui.elements",
    "ui.screenshot",
    "ui.focus",
    "ui.resize",
    "ui.set",
    "ui.menu.list",
    "ui.menu.invoke",
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
    "methods",
];

/// Methods that change nothing: always served, even while a machine is busy or a human-only surface is open.
pub const READS: &[&str] = &[
    "methods",
    "machine.stop",
    "engine.commands",
    "engine.schema",
    "jobs.list",
    "doc.inspect",
    "doc.render",
    "session.list",
    "machine.list",
    "machine.status",
    "job.plan",
    "job.preview",
    "job.estimate",
    "job.simulate",
    "frame.requests",
    "job.status",
    "ui.inspect",
    "ui.elements",
    "ui.menu.list",
    "ui.screenshot",
];

/// The one command id a request names. `engine.execute` and `ui.menu.invoke` accept `command` or `id`;
/// a request that carries both is ambiguous — whichever field a check read, the other could be the one that
/// executes — so it is refused and authorization and execution always see the same single resolved id.
fn command_id(params: &serde_json::Value) -> Result<Option<&str>, String> {
    let mut resolved: Option<&str> = None;
    for key in ["command", "id"] {
        if params.get(key).is_some() {
            if resolved.is_some() {
                return Err("conflicting command identifiers: send only one of `command` or `id`".to_owned());
            }
            resolved = Some(params.get(key).and_then(serde_json::Value::as_str).unwrap_or_default());
        }
    }
    Ok(resolved)
}

/// Does this request change anything? `engine.execute` / `ui.menu.invoke` of a Query or Stop command does not.
/// A conflicting identifier fails closed (mutating); `handle` refuses it before this is consulted anyway.
pub fn mutates(method: &str, params: &serde_json::Value) -> bool {
    if matches!(method, "engine.execute" | "ui.menu.invoke") {
        let Ok(id) = command_id(params) else { return true };
        let id = id.unwrap_or_default();
        return !openlaser_engine::registry().find(id).is_some_and(|spec| matches!(spec.kind, openlaser_engine::Kind::Query) || spec.access == Access::Stop);
    }
    !READS.contains(&method)
}

pub fn handle(app: &mut OpenLaserApp, ctx: &egui::Context, req: ControlRequest) -> Outcome {
    let id = req.request.id.clone();
    let method = req.request.method.as_str();
    let params = &req.request.params;
    if !ROUTES.contains(&method) {
        return Outcome::Done(error_reply(id, &format!("unknown method `{method}`")));
    }
    // Resolve the single command id up front: a request naming both `command` and `id` is refused, and the
    // resolved id below is the only one authorization and execution ever see.
    let command = if matches!(method, "engine.execute" | "ui.menu.invoke") {
        match command_id(params) {
            Ok(command) => command.unwrap_or_default(),
            Err(error) => return Outcome::Done(error_reply(id, &error)),
        }
    } else {
        ""
    };
    if matches!(method, "engine.execute" | "ui.menu.invoke") && contains_human_command(params) {
        return Outcome::Done(error_reply(id, "human-only: hardware actions are started by a person in the OpenLaser window; ask the operator"));
    }
    if mutates(method, params) {
        if app.is_machine_busy() {
            return Outcome::Done(error_reply(id, "machine_busy: a machine is framing, running or stopping; reads and machine.stop are still served"));
        }
        if app.human_surface_open() || app.arm_dialog_open() {
            return Outcome::Done(error_reply(id, "human_confirmation_open: a human-only surface is open; reads and machine.stop are still served"));
        }
    }
    // The dispatcher's own provenance: never Human.
    app.frame_synthetic = true;
    let result: Result<serde_json::Value, String> = match method {
        "methods" => Ok(serde_json::json!({ "methods": ROUTES })),
        "machine.stop" => {
            // Every machine, whatever `machine` names (an unknown id must still stop the one that is running).
            let acks = app.stop_from_control();
            Ok(serde_json::json!({
                "latched": acks.iter().all(|(_, ack)| ack.latched),
                "confirmed": !acks.is_empty() && acks.iter().all(|(_, ack)| ack.report.as_ref().is_some_and(|r| r.confirmed)),
                "stoppedAll": true,
                "requested": params.get("machine"),
                "machines": acks.iter().map(|(id, ack)| serde_json::json!({ "machine": id.0, "latched": ack.latched, "elapsedMs": ack.elapsed_ms })).collect::<Vec<_>>(),
            }))
        }
        "ui.inspect" => Ok(inspect(app)),
        "ui.elements" => Ok(elements(app, params)),
        "ui.menu.list" => Ok(serde_json::json!({
            "items": crate::menu_catalog::CATALOG.iter().map(|r| serde_json::json!({
                "path": r.path,
                "label": r.label,
                "shortcut": r.shortcut,
                "id": r.id,
                "live": crate::menu_catalog::is_live(r),
                "humanOnly": crate::menu_catalog::is_hw_row(r),
                "params": r.panel.map(|panel| serde_json::json!({ "panel": panel, "visible": true })),
            })).collect::<Vec<_>>()
        })),
        "ui.menu.invoke" => menu_invoke(app, command, params),
        "engine.execute" => execute_command(app, command, params.get("params").cloned().unwrap_or(serde_json::Value::Null)),
        "engine.commands" => Ok(serde_json::json!({ "commands": app.session().command_list(Origin::Synthetic, &Default::default()) })),
        "engine.schema" => app.session().command_schema(params.get("id").and_then(serde_json::Value::as_str).unwrap_or_default()).map_err(|e| e.to_string()),
        "machine.list" => Ok(serde_json::json!({
            "machines": app.session().machines().iter().map(|m| serde_json::json!({ "id": m.id.0, "name": m.name })).collect::<Vec<_>>()
        })),
        "machine.status" => Ok(serde_json::json!({
            "machines": app.machine_states().iter().map(|(id, state)| serde_json::json!({ "machine": id.0, "state": format!("{state:?}") })).collect::<Vec<_>>(),
            "active": app.safety_active(),
        })),
        "ui.screenshot" => {
            let side = params.get("maxSide").and_then(serde_json::Value::as_u64).unwrap_or(u64::from(openlaser_control::budgets::PREVIEW_DEFAULT_SIDE));
            if side == 0 || side > u64::from(openlaser_control::budgets::PREVIEW_MAX_SIDE) {
                Err("maxSide must be between 1 and 2048".to_owned())
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                ctx.request_repaint();
                return Outcome::Screenshot { id, max_side: u32::try_from(side).unwrap_or(2048), reply: req.reply };
            }
        }
        "doc.inspect" => app
            .session()
            .inspect(
                params.get("index").and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok()),
                params.get("objects").and_then(serde_json::Value::as_str) == Some("all")
                    || params.get("objects").and_then(serde_json::Value::as_bool) == Some(true),
            )
            .map_err(|e| e.to_string()),
        "doc.render" => doc_render(app, params),
        "job.preview" => job_preview(app, params),
        "job.simulate" => app.session_mut().execute_as(Origin::Synthetic, "job.simulate", params.clone()).map_err(|e| e.to_string()).and_then(png_reply),
        "frame.requests" => Ok(serde_json::json!({ "requests": app.session().requests().frame_requests() })),
        "job.status" => Ok(serde_json::json!({ "submissions": app.session().requests().submissions() })),
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
        "ui.resize" => resize_window(ctx, params),
        "app.quit" => {
            (app.services.quit)();
            Ok(serde_json::json!({ "closing": true }))
        }
        "ui.click" | "ui.pointer" | "ui.move" | "ui.key" | "ui.type" => input::events_for(method, params).map(|events| {
            app.queue_input(events);
            serde_json::json!({ "queued": true })
        }),
        "ui.drag" => input::events_for(method, params).map(|events| {
            app.queue_drag(events);
            serde_json::json!({ "queued": true })
        }),
        "ui.dialog.open" | "ui.dialog.set" | "ui.dialog.confirm" | "ui.dialog.cancel" => dialog(app, method, params),
        "ui.dock.move" | "ui.dock.float" | "ui.dock.close" | "ui.dock.open" | "ui.dock.resize" => dock(app, method, params),
        "ui.set" => ui_set(app, params),
        "jobs.list" | "jobs.cancel" | "app.open" | "app.import" | "app.save" | "app.export" | "doc.select" | "doc.close" | "session.list" | "job.plan"
        | "job.estimate" | "job.export" | "gcode.export" => {
            let command = match method {
                "app.open" => "file.open",
                "app.import" => "file.import",
                "app.save" => "file.save",
                "app.export" => "file.export",
                "doc.select" => "document.select",
                "doc.close" => "file.close",
                "job.export" => "job.dump",
                other => other,
            };
            app.session_mut().execute_as(Origin::Synthetic, command, params.clone()).map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown method `{method}`")),
    };
    Outcome::Done(match result {
        Ok(value) => Reply { id, ok: true, result: Some(value), error: None },
        Err(error) => Reply { id, ok: false, result: None, error: Some(error) },
    })
}

fn inspect(app: &OpenLaserApp) -> serde_json::Value {
    let ui = app.ui_state();
    let plan = app.plan();
    serde_json::json!({
        "window": "OpenLaser",
        "tool": ui.selected_tool,
        "activeLayer": ui.active_layer.to_string(),
        "selection": app.selection(),
        "view": { "zoom": app.view.zoom, "center": app.view.center },
        "visiblePanels": crate::dock::visible_panels(&app.dock),
        "dock": app.dock,
        "dialogs": {
            "cutSettings": ui.cut_settings_open,
            "settings": ui.settings_open,
            "deviceSettings": ui.device_settings_open,
            "liveFraming": ui.live_framing_open,
            "preview": ui.preview_open,
        },
        // The webcam is view state too (phase 6a): whether a capture is open/streaming, never its frames.
        "camera": { "open": app.session.camera_index().is_some(), "running": app.camera_running(), "calibrated": app.session.camera_calibration().is_some(), "background": ui.camera_background },
        // Human-only surfaces are reported read-only, with no nonce or checklist state (§4.6 mechanism 3).
        "armDialog": if app.arm_dialog_open() { "open" } else { "closed" },
        "humanSurfaces": { "open": app.human_surface_open(), "armDialog": app.arm_dialog_open(), "readOnly": true },
        "machines": app.machine_states().iter().map(|(id, state)| serde_json::json!({ "machine": id.0, "state": format!("{state:?}") })).collect::<Vec<_>>(),
        "machineActive": app.safety_active(),
        "device": app.active_machine().map(|m| m.id.0.clone()),
        "plan": plan.map(|p| serde_json::json!({
            "ok": p.summary().is_some(),
            "refusals": p.refusals().len(),
            "notices": p.notices().len(),
            "estimateS": p.summary().map(|s| s.estimate.as_secs_f64()),
        })),
        "inWindowMenus": app.in_window_menus,
        "status": ui.status_message,
        "agent": app.services.automation.info().map(|i| serde_json::json!({ "port": i.port, "clients": i.clients })),
    })
}

fn elements(app: &OpenLaserApp, params: &serde_json::Value) -> serde_json::Value {
    let query = params.get("query").and_then(serde_json::Value::as_str).map(str::to_lowercase);
    let role = params.get("role").and_then(serde_json::Value::as_str);
    let limit = params.get("limit").and_then(serde_json::Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(500).min(2000);
    let items: Vec<serde_json::Value> = app
        .last_frame_elements()
        .iter()
        .enumerate()
        .filter(|(_, e)| query.as_ref().is_none_or(|q| e.label.to_lowercase().contains(q.as_str())))
        .filter(|(_, e)| role.is_none_or(|r| e.role == r))
        .take(limit)
        .map(|(n, e)| serde_json::json!({ "id": n, "label": e.label, "role": e.role, "rect": e.rect, "enabled": e.enabled, "humanOnly": e.human_only }))
        .collect();
    serde_json::json!({ "elements": items, "total": app.last_frame_elements().len() })
}

fn menu_invoke(app: &mut OpenLaserApp, command: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(row) = crate::menu_catalog::CATALOG.iter().find(|row| row.id == command || crate::menu_catalog::native_id(row) == command) else {
        return Err(format!("unknown menu command `{command}`"));
    };
    if crate::menu_catalog::is_hw_row(row) {
        return Err(format!("human-only: `{command}` drives laser hardware; ask the operator"));
    }
    if !crate::menu_catalog::is_live(row) {
        return Err(format!("`{command}` is not in OpenLaser yet"));
    }
    match row.id {
        // Panel rows open the panel here: the engine's `window.show` is a UI hint, not the panel action, and a
        // `window.show:<panel>` row supplies the panel when `params` does not.
        "window.show" => app.show_panel(&params.get("params").cloned().unwrap_or_else(|| serde_json::json!({ "panel": row.panel, "visible": true }))),
        "file.open" | "file.import" => Err("this menu item opens a file dialog for a person; use app.open / app.import with a path".into()),
        _ if openlaser_engine::registry().find(row.id).is_some() => {
            execute_command(app, row.id, params.get("params").cloned().unwrap_or(serde_json::Value::Null))
        }
        _ => {
            app.invoke_ui_action(&crate::menu_catalog::native_id(row));
            Ok(serde_json::json!({ "invoked": row.id }))
        }
    }
}

fn execute_command(app: &mut OpenLaserApp, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    if human_only_command(command) {
        return Err(format!("human-only: `{command}`; ask the operator"));
    }
    if command == "window.show" {
        return app.show_panel(&params);
    }
    if openlaser_engine::registry().find(command).is_none() && command != "command.batch" {
        return Err(format!("unknown command `{command}`"));
    }
    app.session_mut().execute_as(Origin::Synthetic, command, params).map_err(|error| error.to_string())
}

fn resize_window(ctx: &egui::Context, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let width = params.get("width").and_then(serde_json::Value::as_f64).ok_or("width is required")?;
    let height = params.get("height").and_then(serde_json::Value::as_f64).ok_or("height is required")?;
    if !(760.0..=16384.0).contains(&width) || !(480.0..=16384.0).contains(&height) {
        return Err("window size outside 760×480 to 16384×16384".into());
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width as f32, height as f32)));
    Ok(serde_json::json!({ "width": width, "height": height }))
}

/// `ui.set`: a closed list of fields (anything else is an error, never ignored).
fn ui_set(app: &mut OpenLaserApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let fields = params.get("fields").unwrap_or(params);
    let object = fields.as_object().ok_or("fields must be an object")?;
    for (field, value) in object {
        match field.as_str() {
            "tool" => {
                app.ui.selected_tool = match value.as_str() {
                    Some("Select") => "Select",
                    Some("Pan") => "Pan",
                    _ => return Err("tool must be Select or Pan".into()),
                }
            }
            "activeLayer" => {
                let key: openlaser_types::LayerKey = serde_json::from_value(value.clone()).map_err(|e| format!("activeLayer: {e}"))?;
                app.ui.active_layer = key;
            }
            "zoom" => {
                let zoom = value
                    .as_f64()
                    .filter(|z| z.is_finite() && (crate::view::ZOOM_MIN..=crate::view::ZOOM_MAX).contains(z))
                    .ok_or("zoom must be 0.2 to 5000 pt/mm")?;
                app.view.zoom = Some(zoom);
            }
            "center" => {
                let c: [f64; 2] = serde_json::from_value(value.clone()).map_err(|_| "center must be [x, y] in mm")?;
                if !c.iter().all(|v| v.is_finite() && v.abs() <= 1e6) {
                    return Err("center must be finite".into());
                }
                app.view.center = c;
            }
            "fit" => {
                if value.as_bool() == Some(true) {
                    app.view.fit();
                }
            }
            other => return Err(format!("unknown or human-only UI field `{other}`")),
        }
    }
    Ok(serde_json::json!({ "set": object.keys().collect::<Vec<_>>() }))
}

/// Ordinary dialogs only (§4.5): the Cut Settings Editor. Human-only surfaces have no dialog kind here.
fn dialog(app: &mut OpenLaserApp, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    if params.get("dialog").or_else(|| params.get("kind")).and_then(serde_json::Value::as_str) != Some("cutSettings") {
        return Err("unknown or human-only dialog (the only control dialog is cutSettings)".into());
    }
    match method {
        "ui.dialog.open" => {
            if let Some(layer) = params.get("layer") {
                app.ui.active_layer = serde_json::from_value(layer.clone()).map_err(|e| format!("layer: {e}"))?;
            }
            app.ui.cut_settings_open = true;
            app.open_cut_draft();
            if app.cut_draft.is_none() {
                app.ui.cut_settings_open = false;
                return Err("that layer has no cut settings (tool layers and unused layers have none)".into());
            }
        }
        "ui.dialog.cancel" => {
            app.ui.cut_settings_open = false;
            app.cut_draft = None;
        }
        "ui.dialog.set" => {
            let draft = app.cut_draft.as_mut().filter(|_| app.ui.cut_settings_open).ok_or("the Cut Settings Editor is not open")?;
            let field = params.get("field").and_then(serde_json::Value::as_str).ok_or("field is required")?;
            draft.set_field(field, params.get("value").ok_or("value is required")?)?;
        }
        "ui.dialog.confirm" => {
            let draft = app.cut_draft.as_ref().filter(|_| app.ui.cut_settings_open).ok_or("the Cut Settings Editor is not open")?;
            draft.validate()?;
            let params = draft.params();
            app.session_mut().execute_as(Origin::Synthetic, "layer.cutSettings.set", params).map_err(|e| e.to_string())?;
            app.ui.cut_settings_open = false;
            app.cut_draft = None;
        }
        _ => return Err("unknown dialog operation".into()),
    }
    Ok(serde_json::json!({ "open": app.ui.cut_settings_open, "draft": app.cut_draft.as_ref().map(crate::cut_editor::Draft::params) }))
}

fn dock(app: &mut OpenLaserApp, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    use craft_ui::docking::{Action, Placement, Zone};
    let panel = params.get("panel").and_then(serde_json::Value::as_str).ok_or("panel is required")?.to_owned();
    if !crate::dock::PANELS.contains(&panel.as_str()) {
        return Err(format!("unknown panel `{panel}`"));
    }
    let action = match method {
        "ui.dock.close" => Action::Close { panel },
        "ui.dock.open" => Action::Open { panel, anchor: Some("cuts".into()) },
        "ui.dock.float" => Action::Float { panel, rect: [80.0, 80.0, 320.0, 420.0] },
        "ui.dock.resize" => {
            let rect = params.get("rect").and_then(serde_json::Value::as_array).filter(|a| a.len() == 4).ok_or("rect must be [x, y, width, height]")?;
            let mut values = [0.0f32; 4];
            for (out, value) in values.iter_mut().zip(rect) {
                let v = value.as_f64().filter(|v| v.is_finite() && v.abs() <= 16384.0).ok_or("rect must contain finite numbers")?;
                *out = v as f32;
            }
            Action::MoveFloating { panel, rect: values }
        }
        _ => Action::Move {
            panel,
            anchor: params.get("anchor").and_then(serde_json::Value::as_str).ok_or("anchor is required")?.into(),
            placement: Placement::Split(match params.get("zone").and_then(serde_json::Value::as_str).unwrap_or("center") {
                "left" => Zone::Left,
                "right" => Zone::Right,
                "top" => Zone::Top,
                "bottom" => Zone::Bottom,
                "center" => Zone::Center,
                _ => return Err("zone must be left, right, top, bottom or center".into()),
            }),
        },
    };
    app.dock.apply_with_permissions(action, crate::dock::permissions).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "layout": app.dock }))
}

/// A PNG reply: `{mime, pngBase64, width, height, …}` (the bridge's image shape; §4.1 PNG tools).
fn png_value(png: &[u8], extra: serde_json::Value) -> serde_json::Value {
    let be = |range: std::ops::Range<usize>| png.get(range).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes).unwrap_or(0);
    let (width, height) = (be(16..20), be(20..24));
    let mut value = serde_json::json!({ "mime": "image/png", "pngBase64": crate::screenshot::base64(png), "width": width, "height": height });
    if let (Some(map), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    value
}

/// Normalises an engine result that carries a PNG (`png` as bytes or base64) into the `{pngBase64, …}` shape.
fn png_reply(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value.get("pngBase64").is_some() {
        return Ok(value);
    }
    let bytes: Option<Vec<u8>> = value.get("png").and_then(|p| serde_json::from_value(p.clone()).ok());
    match (bytes, value.as_object_mut()) {
        (Some(bytes), Some(map)) => {
            map.remove("png");
            Ok(png_value(&bytes, serde_json::Value::Object(map.clone())))
        }
        _ => Err("the engine returned no image".into()),
    }
}

fn max_side(params: &serde_json::Value) -> Result<u32, String> {
    let side = params.get("maxSide").and_then(serde_json::Value::as_u64).unwrap_or(u64::from(openlaser_control::budgets::PREVIEW_DEFAULT_SIDE));
    if side == 0 || side > u64::from(openlaser_control::budgets::PREVIEW_MAX_SIDE) {
        return Err("maxSide must be between 1 and 2048".into());
    }
    u32::try_from(side).map_err(|_| "maxSide must be between 1 and 2048".to_owned())
}

/// `doc.render`: the document (never the window), rendered by `openlaser_render` from the same display list.
fn doc_render(app: &OpenLaserApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let side = max_side(params)?;
    let project = app.project().ok_or("no document open")?;
    let machine = app.active_machine();
    let opts =
        openlaser_render::ViewOpts { dark: false, selection: Vec::new(), refusals: Vec::new(), notices: Vec::new(), show_field: true, show_work_area: true };
    let list = openlaser_render::document_list(project, machine, &opts);
    let extent = crate::view::MachineGeometry::of(machine).extent();
    let world = openlaser_render::fit(&list, 0.05).unwrap_or(openlaser_types::Bounds { min: extent[0], max: extent[1] });
    let png = openlaser_render::render_png(&list, &openlaser_render::RenderView { world, max_side: side, background: openlaser_render::Rgba([255; 4]) })
        .map_err(|e| e.to_string())?;
    Ok(png_value(&png, serde_json::json!({ "bounds": world })))
}

/// `job.preview`: the planned job (burn, travel, frame), from the same plan the burn would use.
fn job_preview(app: &OpenLaserApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let side = max_side(params)?;
    let show = params.get("show").and_then(serde_json::Value::as_array);
    let has = |what: &str| show.is_none_or(|s| s.iter().any(|v| v.as_str() == Some(what)));
    let job = app
        .session()
        .build_job(None, &openlaser_engine::JobOptions::default())
        .map_err(|refused| format!("refused: {}", refused.refusals.iter().map(|r| r.message.as_str()).collect::<Vec<_>>().join("; ")))?;
    let machine = app.active_machine().ok_or("no machine")?;
    let list = openlaser_render::plan_list(
        &job.plan,
        machine,
        &openlaser_render::PlanShow { burn: has("burn"), travel: has("travel"), frame: has("frame"), order: has("order") },
    );
    let extent = crate::view::MachineGeometry::of(Some(machine)).extent();
    let world = openlaser_render::fit(&list, 0.05).unwrap_or(openlaser_types::Bounds { min: extent[0], max: extent[1] });
    let png = openlaser_render::render_png(&list, &openlaser_render::RenderView { world, max_side: side, background: openlaser_render::Rgba([255; 4]) })
        .map_err(|e| e.to_string())?;
    Ok(png_value(&png, serde_json::json!({ "fingerprint": job.summary.fingerprint, "bounds": world })))
}

pub(crate) fn error_reply(id: serde_json::Value, error: &str) -> Reply {
    Reply { id, ok: false, result: None, error: Some(error.to_owned()) }
}

/// Ids that drive hardware or bound safety (S18, §4.5 "deliberately absent", §4.6): never from the control channel.
pub fn human_only_command(id: &str) -> bool {
    id.starts_with(crate::menu_catalog::HW_PREFIX)
        || matches!(
            id,
            "device.connect"
                | "device.disconnect"
                | "device.reconnect"
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
                | "laser.goToOrigin"
                | "laser.jog"
                | "laser.pause"
                | "laser.resume"
                | "frame.approve"
                | "frame.decline"
                | "automation.set"
        )
        || openlaser_engine::registry().find(id).is_some_and(|spec| spec.access == Access::Human)
}

fn contains_human_command(params: &serde_json::Value) -> bool {
    match params {
        serde_json::Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| (matches!(key.as_str(), "command" | "id") && value.as_str().is_some_and(human_only_command)) || contains_human_command(value)),
        serde_json::Value::Array(values) => values.iter().any(contains_human_command),
        serde_json::Value::String(s) => human_only_command(s),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4.5's table, read from the spec itself: ROUTES is exactly it (no more, no fewer).
    #[test]
    fn control_routes_match_spec_table() {
        let spec = include_str!("../../../../docs/SPEC.md");
        let table = spec.split("### 4.5 Control methods").nth(1).and_then(|s| s.split("**Deliberately absent**").next()).unwrap_or_default();
        let mut expected: Vec<String> = Vec::new();
        for line in table.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| Group") && !l.starts_with("|---")) {
            let methods = line.split('|').nth(2).unwrap_or_default();
            for quoted in methods.split('`').skip(1).step_by(2) {
                let name = quoted.split_whitespace().next().unwrap_or_default();
                let parts: Vec<&str> = name.split('/').collect();
                let first = parts.first().copied().unwrap_or_default();
                expected.push(first.to_owned());
                if let Some((prefix, _)) = first.rsplit_once('.') {
                    for alt in parts.iter().skip(1) {
                        expected.push(format!("{prefix}.{alt}"));
                    }
                }
            }
        }
        let mut routes: Vec<String> = ROUTES.iter().map(|s| (*s).to_owned()).collect();
        expected.sort();
        routes.sort();
        assert_eq!(routes, expected);
        assert!(expected.len() >= 50, "{expected:?}");
        for method in openlaser_control::methods::METHODS {
            assert!(ROUTES.contains(method), "WP7's table has `{method}` but the dispatcher does not route it");
        }
    }

    #[test]
    fn reads_are_routes_and_mutations_are_the_rest() {
        for read in READS {
            assert!(ROUTES.contains(read), "{read}");
            assert!(!mutates(read, &serde_json::json!({})), "{read}");
        }
        for method in [
            "engine.execute",
            "ui.menu.invoke",
            "ui.click",
            "ui.key",
            "ui.dialog.confirm",
            "ui.dock.close",
            "app.quit",
            "app.open",
            "job.submit",
            "frame.request",
        ] {
            assert!(mutates(method, &serde_json::json!({ "command": "file.new" })), "{method}");
        }
    }

    #[test]
    fn human_only_ids_are_found_anywhere_in_the_params() {
        assert!(contains_human_command(&serde_json::json!({ "command": "laser.start" })));
        assert!(contains_human_command(&serde_json::json!({ "command": "command.batch", "params": { "steps": [{ "id": "device.connect" }] } })));
        assert!(contains_human_command(&serde_json::json!({ "id": "hw:tools.positionLaser" })));
        assert!(!contains_human_command(&serde_json::json!({ "command": "layer.setOutput", "params": { "layer": "C00", "on": true } })));
    }

    #[test]
    fn png_replies_carry_png_base64() {
        let png = [0x89u8, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 7, 0, 0, 0, 5];
        let value = png_value(&png, serde_json::json!({ "fingerprint": "ab" }));
        assert_eq!(value["width"], 7);
        assert_eq!(value["height"], 5);
        assert_eq!(value["fingerprint"], "ab");
        assert!(value["pngBase64"].as_str().unwrap().starts_with("iVBORw0KGgo"));
        let normalised = png_reply(serde_json::json!({ "png": png.to_vec(), "summary": {} })).unwrap();
        assert!(normalised.get("png").is_none());
        assert!(normalised["pngBase64"].is_string());
        assert!(normalised.get("summary").is_some());
        assert!(png_reply(serde_json::json!({})).is_err());
    }
}

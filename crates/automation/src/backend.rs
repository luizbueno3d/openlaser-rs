//! The two tool backends. WP0 stubs.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::bridge::BridgeClient;
use crate::tools::{ClientInfo, Mode};
use crate::{BridgeTarget, Error};

pub enum ToolOutput {
    Json(serde_json::Value),
    Png { png: Vec<u8>, note: serde_json::Value },
    Error { message: String, data: Option<serde_json::Value> },
}

/// The MCP layer is identical over both backends. Tool calls run one at a time behind a mutex (PhotoCraft), EXCEPT
/// `stop`, which the server routes to `stop_handle()` without taking that mutex, so a long `job_simulate` or a
/// 55 s bridge timeout can never delay a stop.
pub trait ToolBackend: Send {
    fn mode(&self) -> Mode;
    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput;
    /// Bridge: a handle with its own authenticated connection, used only for `machine.stop`. Headless: None (the
    /// `stop` tool then returns the `needs_bridge:` error).
    fn stop_handle(&self) -> Option<StopHandle>;
}

/// Arc<Mutex<dedicated BridgeClient connection>>.
#[derive(Clone)]
pub struct StopHandle {
    conn: Arc<Mutex<Option<BridgeClient>>>,
}

impl StopHandle {
    /// WP0 addition: wraps the dedicated stop connection (`BridgeClient::stop_handle` builds it).
    pub(crate) fn new(conn: Option<BridgeClient>) -> Self {
        StopHandle { conn: Arc::new(Mutex::new(conn)) }
    }

    /// Waits ≤ STOP_WAIT for the StopAck. WP0 stub: reports that nothing was sent.
    pub fn stop(&self, machine: Option<&str>) -> ToolOutput {
        let Ok(mut guard) = self.conn.lock() else { return ToolOutput::Error { message: "transport: stop connection unavailable".to_owned(), data: None } };
        let Some(connection) = guard.as_mut() else { return ToolOutput::Error { message: "needs_bridge: stop requires a running GUI".to_owned(), data: None } };
        match connection.call_stop(machine) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.entry("latched").or_insert(serde_json::Value::Bool(true));
                }
                ToolOutput::Json(value)
            }
            Err(error) => ToolOutput::Error { message: format_bridge_error(error), data: None },
        }
    }
}

/// engine::Session with origin Headless, window false, DeviceAccess::None; files through AuthorizedWorkspace.
pub struct Headless {
    session: openlaser_engine::Session,
}

impl Headless {
    pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, Error> {
        let workspace = openlaser_control::workspace::AuthorizedWorkspace::open(read_root, write_root)?;
        let files = EngineWorkspace { inner: workspace };
        let session = openlaser_engine::Session::new(openlaser_engine::SessionConfig {
            origin: openlaser_engine::Origin::Headless,
            window: false,
            devices: openlaser_engine::DeviceAccess::None,
            machines: vec![openlaser_machine::jpt_m7_200_sim()],
            profiles: None,
            files: openlaser_engine::FsAuthority::Scoped(Box::new(files)),
            clock: std::sync::Arc::new(openlaser_types::SystemClock::default()),
        });
        Ok(Self { session })
    }

    /// Dispatch a raw control method through the same headless tool implementation as MCP.
    pub fn dispatch_control(&mut self, method: &str, params: serde_json::Value) -> ToolOutput {
        let tool = match method {
            "engine.execute" => "command_run",
            "engine.commands" => "command_list",
            "engine.schema" => "command_schema",
            "jobs.list" => "jobs_list",
            "jobs.cancel" => "jobs_cancel",
            "app.open" => "doc_open",
            "app.import" => "doc_import",
            "app.save" => "doc_save",
            "app.export" => "doc_export",
            "doc.inspect" => "doc_inspect",
            "doc.render" => "render_preview",
            "doc.select" => "doc_select",
            "doc.close" => "doc_close",
            "session.list" => "session_list",
            "machine.list" => "machine_list",
            "machine.status" => "machine_status",
            "job.plan" => "job_plan",
            "job.preview" => "job_preview",
            "job.estimate" => "job_estimate",
            "job.simulate" => "job_simulate",
            "job.export" => "job_export",
            "gcode.export" => "gcode_export",
            "frame.request" => "frame_request",
            "job.submit" => "job_submit",
            "job.status" => "job_status",
            "job.withdraw" => "job_withdraw",
            "machine.stop" => "stop",
            "ui.inspect" => "ui_inspect",
            "ui.elements" => "ui_elements",
            "ui.screenshot" => "ui_screenshot",
            "ui.click" => "ui_click",
            "ui.drag" => "ui_drag",
            "ui.key" => "ui_key",
            "ui.type" => "ui_type",
            "ui.pointer" => "ui_pointer",
            "ui.menu.invoke" => "ui_menu_invoke",
            "ui.set" => "ui_set",
            "methods" => return ToolOutput::Json(serde_json::to_value(openlaser_control::methods::METHODS).unwrap_or(serde_json::Value::Null)),
            _ if openlaser_control::methods::METHODS.contains(&method) => {
                return ToolOutput::Error { message: format!("not implemented: headless control method `{method}`"), data: None };
            }
            _ => return ToolOutput::Error { message: format!("unknown method `{method}`"), data: None },
        };
        let mut args = params;
        if let Some(object) = args.as_object_mut()
            && method == "engine.execute"
            && let Some(command) = object.remove("command")
        {
            object.insert("id".to_owned(), command);
        }
        self.call(tool, args, &ClientInfo { name: "openlaser-cli".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() })
    }
}

impl ToolBackend for Headless {
    fn mode(&self) -> Mode {
        Mode::Headless
    }

    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput {
        let _ = client;
        if matches!(
            tool,
            "frame_request"
                | "job_submit"
                | "job_status"
                | "job_withdraw"
                | "ui_inspect"
                | "ui_elements"
                | "ui_screenshot"
                | "ui_click"
                | "ui_drag"
                | "ui_key"
                | "ui_type"
                | "ui_pointer"
                | "ui_menu_invoke"
                | "ui_set"
        ) {
            return ToolOutput::Error { message: format!("needs_bridge: `{tool}` requires the running OpenLaser window"), data: None };
        }
        let result = match tool {
            "command_list" => Ok(serde_json::to_value(self.session.command_list(openlaser_engine::Origin::Headless, &openlaser_engine::ListFilter::default()))
                .unwrap_or(serde_json::Value::Array(Vec::new()))),
            "command_schema" => string_arg(&args, "id").and_then(|id| self.session.command_schema(id).map_err(|e| e.to_string())),
            "command_run" => string_arg(&args, "id")
                .and_then(|id| self.session.execute(id, args.get("params").cloned().unwrap_or(serde_json::json!({}))).map_err(|e| e.to_string())),
            "command_batch" => Err("not implemented: command_batch requires engine batch API support".to_owned()),
            "doc_inspect" => {
                self.session.inspect(optional_index(&args), args.get("objects").and_then(serde_json::Value::as_str) == Some("all")).map_err(|e| e.to_string())
            }
            "machine_status" => Ok(serde_json::json!({"connected":false,"backend":"headless"})),
            "machine_list" => Ok(serde_json::json!([{"id":"jpt-m7-200-sim","name":"JPT M7 · Simulator","kind":"galvo_lmc","connected":false}])),
            "job_plan" => match self.session.build_job(optional_index(&args), &openlaser_engine::JobOptions::default()) {
                Ok(built) => Ok(serde_json::json!({"ok":true,"fingerprint":built.plan.fingerprint,"summary":built.summary,"refusals":[]})),
                Err(refused) => Ok(
                    serde_json::json!({"ok":false,"fingerprint":refused.fingerprint,"refusals":refused.refusals,"notices":refused.notices,"error":refused.error}),
                ),
            },
            "job_preview" | "job_estimate" | "job_simulate" => Err("not implemented: job pipeline is not available in this engine build".to_owned()),
            "stop" => Err("needs_bridge: stop requires the running OpenLaser window".to_owned()),
            _ => Err(format!("not implemented: headless tool `{tool}`")),
        };
        result.map(ToolOutput::Json).unwrap_or_else(|message| ToolOutput::Error { message, data: None })
    }

    fn stop_handle(&self) -> Option<StopHandle> {
        None
    }
}

pub struct Bridge {
    client: BridgeClient,
    stop: Option<StopHandle>,
}

impl ToolBackend for Bridge {
    fn mode(&self) -> Mode {
        Mode::Bridge
    }

    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput {
        let _ = client;
        if tool == "stop" {
            return self.stop.as_ref().map_or_else(
                || ToolOutput::Error { message: "needs_bridge: dedicated stop channel unavailable".to_owned(), data: None },
                |h| h.stop(args.get("machine").and_then(serde_json::Value::as_str)),
            );
        }
        let method = match tool {
            "command_list" => "engine.commands",
            "command_schema" => "engine.schema",
            "command_run" => "engine.execute",
            "doc_open" => "app.open",
            "doc_import" => "app.import",
            "doc_save" => "app.save",
            "doc_export" => "app.export",
            "doc_inspect" => "doc.inspect",
            "render_preview" => "doc.render",
            "doc_select" => "doc.select",
            "doc_close" => "doc.close",
            "session_list" => "session.list",
            "jobs_list" => "jobs.list",
            "jobs_cancel" => "jobs.cancel",
            "machine_list" => "machine.list",
            "machine_status" => "machine.status",
            "job_plan" => "job.plan",
            "job_preview" => "job.preview",
            "job_estimate" => "job.estimate",
            "job_simulate" => "job.simulate",
            "job_export" => "job.export",
            "gcode_export" => "gcode.export",
            "frame_request" => "frame.request",
            "job_submit" => "job.submit",
            "job_status" => "job.status",
            "job_withdraw" => "job.withdraw",
            "ui_inspect" => "ui.inspect",
            "ui_elements" => "ui.elements",
            "ui_screenshot" => "ui.screenshot",
            "ui_click" => "ui.click",
            "ui_drag" => "ui.drag",
            "ui_key" => "ui.key",
            "ui_type" => "ui.type",
            "ui_pointer" => "ui.pointer",
            "ui_menu_invoke" => "ui.menu.invoke",
            "ui_set" => "ui.set",
            "control_call" => return self.raw_control(args),
            _ => return ToolOutput::Error { message: format!("not implemented: bridge tool `{tool}`"), data: None },
        };
        let read = openlaser_control::methods::is_read_only(method);
        let params = args_to_control(tool, args);
        let result = if read { self.client.call_read(method, params) } else { self.client.call(method, params) };
        result.map(ToolOutput::Json).unwrap_or_else(|e| ToolOutput::Error { message: format_bridge_error(e), data: None })
    }

    fn stop_handle(&self) -> Option<StopHandle> {
        self.stop.clone()
    }
}

impl Bridge {
    pub fn connect(target: &BridgeTarget, client: &ClientInfo) -> Result<Self, Error> {
        let connection = BridgeClient::connect(target, client).map_err(Error::Bridge)?;
        let stop = connection.stop_handle().ok();
        Ok(Self { client: connection, stop })
    }

    fn raw_control(&mut self, args: serde_json::Value) -> ToolOutput {
        let Some(method) = args.get("method").and_then(serde_json::Value::as_str) else {
            return ToolOutput::Error { message: "invalid params: method is required".to_owned(), data: None };
        };
        self.client
            .call(method, args.get("params").cloned().unwrap_or(serde_json::json!({})))
            .map(ToolOutput::Json)
            .unwrap_or_else(|e| ToolOutput::Error { message: format_bridge_error(e), data: None })
    }
}

struct EngineWorkspace {
    inner: openlaser_control::workspace::AuthorizedWorkspace,
}
impl openlaser_engine::FileAccess for EngineWorkspace {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.inner.read(path).map_err(|e| e.to_string())
    }
    fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.inner.write_atomic(path, bytes).map_err(|e| e.to_string())
    }
}

fn string_arg<'a>(args: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    args.get(name).and_then(serde_json::Value::as_str).ok_or_else(|| format!("invalid params: {name} is required"))
}
fn optional_index(args: &serde_json::Value) -> Option<usize> {
    args.get("index").and_then(serde_json::Value::as_u64).and_then(|v| usize::try_from(v).ok())
}
fn args_to_control(tool: &str, args: serde_json::Value) -> serde_json::Value {
    let mut args = args;
    if let Some(obj) = args.as_object_mut()
        && let Some(id) = obj.remove("id")
    {
        if tool == "command_run" {
            obj.insert("command".to_owned(), id);
        } else {
            obj.insert("id".to_owned(), id);
        }
    }
    args
}
fn format_bridge_error(error: crate::BridgeError) -> String {
    match error {
        crate::BridgeError::NotSent(text) => format!("not_connected: the request was not sent: {text}"),
        crate::BridgeError::Transport(text) => format!("transport: operation may have completed; inspect state before retrying: {text}"),
        crate::BridgeError::TimedOut { read: true } => "timeout: read may be retried safely".to_owned(),
        crate::BridgeError::TimedOut { read: false } => "timeout: write may have completed; inspect state before retrying".to_owned(),
        crate::BridgeError::Definite(text) => text,
        crate::BridgeError::NotImplemented(text) => format!("not implemented: {text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_failure_wording() {
        assert!(format_bridge_error(crate::BridgeError::NotSent("connection refused".to_owned())).contains("the request was not sent"));
        assert!(format_bridge_error(crate::BridgeError::Transport("reset".to_owned())).contains("operation may have completed; inspect state before retrying"));
        assert!(format_bridge_error(crate::BridgeError::TimedOut { read: true }).contains("may be retried safely"));
        assert!(format_bridge_error(crate::BridgeError::TimedOut { read: false }).contains("may have completed"));
    }

    #[test]
    fn headless_control_dispatch_uses_engine_tool_routes() {
        let mut backend = Headless::new(None, None).unwrap();
        let output = backend.dispatch_control("methods", serde_json::json!({}));
        assert!(matches!(output, ToolOutput::Json(value) if value.as_array().is_some_and(|items| items.iter().any(|item| item == "engine.execute"))));
        let unknown = backend.dispatch_control("made.up", serde_json::json!({}));
        assert!(matches!(unknown, ToolOutput::Error { message, .. } if message.starts_with("unknown method")));
    }

    #[test]
    fn headless_device_tools_need_bridge() {
        let mut backend = Headless::new(None, None).unwrap();
        let output = backend.call("frame_request", serde_json::json!({}), &ClientInfo { name: "test".into(), version: "0".into() });
        assert!(matches!(output, ToolOutput::Error { message, .. } if message.starts_with("needs_bridge:")));
    }

    #[test]
    fn job_plan_refusal_is_a_successful_tool_result_with_ok_false() {
        let mut backend = Headless::new(None, None).unwrap();
        let output = backend.call("job_plan", serde_json::json!({}), &ClientInfo { name: "test".into(), version: "0".into() });
        assert!(matches!(output, ToolOutput::Json(value) if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false)));
    }

    #[test]
    #[ignore = "needs I1"]
    fn job_simulate_business_card() {
        let mut backend = Headless::new(None, None).unwrap();
        let output = backend.call("job_simulate", serde_json::json!({"index":0}), &ClientInfo { name: "test".into(), version: "0".into() });
        assert!(matches!(output, ToolOutput::Png { .. }));
    }

    #[test]
    #[ignore = "needs I2"]
    fn render_preview_is_document() {
        let mut backend = Headless::new(None, None).unwrap();
        let output = backend.call("render_preview", serde_json::json!({}), &ClientInfo { name: "test".into(), version: "0".into() });
        assert!(matches!(output, ToolOutput::Png { .. }));
    }
}

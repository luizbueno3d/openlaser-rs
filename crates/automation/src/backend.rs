//! The headless engine backend and authenticated GUI bridge.

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
            "app.new" => "doc_new",
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
            "doc_new" => self.session.execute("file.new", args.clone()).map_err(|e| e.to_string()),
            "doc_open" => self.session.execute("file.open", args.clone()).map_err(|e| e.to_string()),
            "doc_import" => self.session.execute("file.import", args.clone()).map_err(|e| e.to_string()),
            "doc_save" => if args.get("path").and_then(serde_json::Value::as_str).is_some() {
                self.session.execute("file.saveAs", args.clone())
            } else {
                self.session.execute("file.save", serde_json::json!({}))
            }
            .map_err(|e| e.to_string()),
            "doc_export" => self.session.execute("file.export", args.clone()).map_err(|e| e.to_string()),
            "doc_select" => optional_index(&args)
                .ok_or_else(|| "invalid params: index is required".to_owned())
                .and_then(|index| self.session.select_doc(index).map(|_| session_list(&self.session)).map_err(|e| e.to_string())),
            "doc_close" => self.session.close_doc(optional_index(&args)).map(|_| session_list(&self.session)).map_err(|e| e.to_string()),
            "session_list" => Ok(session_list(&self.session)),
            "jobs_list" => Ok(serde_json::json!([])),
            "jobs_cancel" => Err("not supported: headless sessions have no background jobs".to_owned()),
            "command_list" => Ok(serde_json::to_value(self.session.command_list(
                openlaser_engine::Origin::Headless,
                &openlaser_engine::ListFilter {
                    filter: args.get("filter").and_then(serde_json::Value::as_str).map(str::to_owned),
                    enabled_only: args.get("enabled_only").or_else(|| args.get("enabledOnly")).and_then(serde_json::Value::as_bool).unwrap_or(false),
                    agent: None,
                    schema: args.get("schema").and_then(serde_json::Value::as_bool).unwrap_or(false),
                },
            ))
            .unwrap_or(serde_json::Value::Array(Vec::new()))),
            "command_schema" => string_arg(&args, "id").and_then(|id| self.session.command_schema(id).map_err(|e| e.to_string())),
            "command_run" => string_arg(&args, "id")
                .and_then(|id| self.session.execute(id, args.get("params").cloned().unwrap_or(serde_json::json!({}))).map_err(|e| e.to_string())),
            "command_batch" => {
                let steps =
                    args.get("steps").and_then(serde_json::Value::as_array).ok_or_else(|| "invalid params: steps is required".to_owned()).and_then(|items| {
                        items
                            .iter()
                            .map(|item| {
                                let id =
                                    item.get("id").and_then(serde_json::Value::as_str).ok_or_else(|| "invalid params: every step needs an id".to_owned())?;
                                Ok((id.to_owned(), item.get("params").cloned().unwrap_or_else(|| serde_json::json!({}))))
                            })
                            .collect::<Result<Vec<_>, String>>()
                    });
                steps.map(|steps| {
                    let atomic = args
                        .get("atomic")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                        .then(|| args.get("label").and_then(serde_json::Value::as_str).unwrap_or("Automation batch").to_owned());
                    let result = self.session.batch(
                        openlaser_engine::Origin::Headless,
                        steps,
                        args.get("stop_on_error").or_else(|| args.get("stopOnError")).and_then(serde_json::Value::as_bool).unwrap_or(true),
                        atomic,
                    );
                    serde_json::to_value(result).unwrap_or(serde_json::Value::Null)
                })
            }
            "doc_inspect" => {
                self.session.inspect(optional_index(&args), args.get("objects").and_then(serde_json::Value::as_str) == Some("all")).map_err(|e| e.to_string())
            }
            "machine_status" => Ok(serde_json::json!({"connected":false,"backend":"headless"})),
            "machine_list" => Ok(serde_json::to_value(self.session.machines()).unwrap_or(serde_json::Value::Array(Vec::new()))),
            "job_plan" => match self.session.build_job(optional_index(&args), &job_options(&args)) {
                Ok(built) => Ok(serde_json::json!({"ok":true,"fingerprint":built.plan.fingerprint,"summary":built.summary,"refusals":[]})),
                Err(refused) => Ok(
                    serde_json::json!({"ok":false,"fingerprint":refused.fingerprint,"refusals":refused.refusals,"notices":refused.notices,"error":refused.error}),
                ),
            },
            "job_estimate" => match self.session.build_job(optional_index(&args), &job_options(&args)) {
                Ok(built) => Ok(serde_json::json!({
                    "fingerprint":built.summary.fingerprint.hex(), "estimateS":built.summary.estimate.as_secs_f64(),
                    "crashBoundS":built.summary.crash_bound.as_secs_f64(),
                    "perSection":built.summary.rows.iter().map(|row| serde_json::json!({"layer":row.layer,"pass":row.pass,"of":row.of,"estimateS":row.estimate.as_secs_f64(),"markMm":row.mark_mm,"paths":row.paths})).collect::<Vec<_>>()
                })),
                Err(refused) => Ok(serde_json::json!({"ok":false,"fingerprint":refused.fingerprint,"refusals":refused.refusals,"error":refused.error})),
            },
            "job_preview" => {
                let built = self
                    .session
                    .build_job(optional_index(&args), &job_options(&args))
                    .map_err(|refusal| serde_json::to_string(&refusal).unwrap_or_else(|_| "job refused".to_owned()));
                built.and_then(|built| {
                    let max_side = max_side(&args)?;
                    let Some(profile) = self.session.machine_for(optional_index(&args)) else { return Err("no machine profile for document".to_owned()) };
                    let names = args
                        .get("show")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| items.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>());
                    let show = openlaser_render::PlanShow {
                        burn: names.as_ref().is_none_or(|items| items.contains(&"burn")),
                        travel: names.as_ref().is_none_or(|items| items.contains(&"travel")),
                        frame: names.as_ref().is_some_and(|items| items.contains(&"frame")),
                        order: names.as_ref().is_some_and(|items| items.contains(&"order")),
                    };
                    let list = openlaser_render::plan_list(&built.plan, profile, &show);
                    render_list(list, max_side, serde_json::json!({"fingerprint":built.summary.fingerprint.hex()}))
                })
            }
            "job_simulate" => {
                let built = self
                    .session
                    .build_job(optional_index(&args), &job_options(&args))
                    .map_err(|refusal| serde_json::to_string(&refusal).unwrap_or_else(|_| "job refused".to_owned()));
                built.and_then(|built| {
                    let max_side = max_side(&args)?;
                    let Some(profile) = self.session.machine_for(optional_index(&args)) else { return Err("no machine profile for document".to_owned()) };
                    let (job, galvo) = match (&built.encoded, &profile.kind) {
                        (openlaser_engine::Encoded::Lmc { job, .. }, openlaser_machine::Kinematics::Galvo(galvo)) => (job, galvo),
                        _ => return Err("offline simulation currently supports galvo LMC jobs".to_owned()),
                    };
                    let recording = openlaser_lmc_sim::simulate_job(job, galvo);
                    let light = recording.lit.to_vec();
                    let list = openlaser_render::emission_list(&recording.burned, &light, profile);
                    let mut note = serde_json::json!({
                        "fingerprint":built.summary.fingerprint.hex(), "summary":built.summary,
                        "wireEvents":recording.wire.len(), "burnedSegments":recording.burned.len()
                    });
                    if args.get("listing").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                        let mut lines = Vec::new();
                        let mut bytes = 0_usize;
                        let mut truncated = false;
                        for line in built.listing.lines.iter().take(2000) {
                            if bytes.saturating_add(line.text.len()) > 256 * 1024 {
                                truncated = true;
                                break;
                            }
                            bytes = bytes.saturating_add(line.text.len());
                            lines.push(line.text.clone());
                        }
                        truncated |= built.listing.lines.len() > lines.len();
                        if let Some(object) = note.as_object_mut() {
                            object.insert("listing".to_owned(), serde_json::json!({"lines":lines,"truncated":truncated}));
                        }
                    }
                    render_list(list, max_side, note)
                })
            }
            "job_export" => self.session.execute("job.dump", args.clone()).map_err(|e| e.to_string()),
            "gcode_export" => self.session.execute("file.saveMachineFile", args.clone()).map_err(|e| e.to_string()),
            "render_preview" => {
                let max_side = match max_side(&args) {
                    Ok(side) => side,
                    Err(message) => return ToolOutput::Error { message, data: None },
                };
                let Some(doc) = optional_index(&args).and_then(|index| self.session.docs().get(index)).or_else(|| self.session.active()) else {
                    return ToolOutput::Error { message: "no document open".to_owned(), data: None };
                };
                let profile = self.session.machines().iter().find(|machine| machine.id == doc.machine);
                let opts = openlaser_render::ViewOpts {
                    dark: false,
                    selection: Vec::new(),
                    refusals: Vec::new(),
                    notices: Vec::new(),
                    show_field: true,
                    show_work_area: true,
                };
                let list = if args.get("view").and_then(serde_json::Value::as_str) == Some("toolpath") {
                    let built = match self.session.build_job(optional_index(&args), &openlaser_engine::JobOptions::default()) {
                        Ok(built) => built,
                        Err(refused) => {
                            return ToolOutput::Error { message: serde_json::to_string(&refused).unwrap_or_else(|_| "job refused".to_owned()), data: None };
                        }
                    };
                    let Some(machine) = profile else { return ToolOutput::Error { message: "no machine profile for document".to_owned(), data: None } };
                    openlaser_render::plan_list(&built.plan, machine, &openlaser_render::PlanShow { burn: true, travel: true, frame: false, order: false })
                } else {
                    openlaser_render::document_list(&doc.project, profile, &opts)
                };
                render_list(list, max_side, serde_json::json!({"document":doc.project.name}))
            }
            "stop" => Err("needs_bridge: stop requires the running OpenLaser window".to_owned()),
            _ => Err(format!("not implemented: headless tool `{tool}`")),
        };
        result.map(tool_output).unwrap_or_else(|message| ToolOutput::Error { message, data: None })
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

fn max_side(args: &serde_json::Value) -> Result<u32, String> {
    let Some(value) = args.get("max_side") else { return Ok(1024) };
    let Some(side) = value.as_u64().and_then(|value| u32::try_from(value).ok()) else {
        return Err("invalid params: max_side must be an integer from 1 to 2048".to_owned());
    };
    if (1..=2048).contains(&side) { Ok(side) } else { Err("invalid params: max_side must be an integer from 1 to 2048".to_owned()) }
}

fn session_list(session: &openlaser_engine::Session) -> serde_json::Value {
    serde_json::json!({
        "documents": session.docs().iter().enumerate().map(|(index, doc)| serde_json::json!({
            "index":index, "name":doc.project.name, "path":doc.path, "dirty":doc.dirty(),
            "revision":doc.revision, "machine":doc.machine.0
        })).collect::<Vec<_>>(), "active":session.active_index()
    })
}

fn job_options(args: &serde_json::Value) -> openlaser_engine::JobOptions {
    openlaser_engine::JobOptions {
        machine: args.get("machine").and_then(serde_json::Value::as_str).map(|id| openlaser_types::MachineId(id.to_owned())),
        selected_only: args.get("selected_only").or_else(|| args.get("selectedOnly")).and_then(serde_json::Value::as_bool),
    }
}

fn render_list(list: openlaser_render::DisplayList, max_side: u32, note: serde_json::Value) -> Result<serde_json::Value, String> {
    let world = openlaser_render::fit(&list, 0.05).ok_or_else(|| "preview has no renderable geometry".to_owned())?;
    let png = openlaser_render::render_png(&list, &openlaser_render::RenderView { world, max_side, background: openlaser_render::Rgba([255, 255, 255, 255]) })
        .map_err(|error| error.to_string())?;
    let mut note = note;
    if let Some(object) = note.as_object_mut() {
        object.insert("bounds".to_owned(), serde_json::to_value(world).unwrap_or(serde_json::Value::Null));
    }
    Ok(serde_json::json!({"__png":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png), "note":note}))
}

fn tool_output(value: serde_json::Value) -> ToolOutput {
    if let Some(encoded) = value.get("__png").and_then(serde_json::Value::as_str) {
        use base64::Engine as _;
        if let Ok(png) = base64::engine::general_purpose::STANDARD.decode(encoded) {
            return ToolOutput::Png { png, note: value.get("note").cloned().unwrap_or(serde_json::Value::Null) };
        }
    }
    ToolOutput::Json(value)
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
    fn headless_document_create_and_list() {
        let temp = tempfile::tempdir().unwrap();
        let mut backend = Headless::new(Some(temp.path()), Some(temp.path())).unwrap();
        let client = ClientInfo { name: "test".into(), version: "0".into() };
        assert!(matches!(backend.call("doc_new", serde_json::json!({"name":"WP7"}), &client), ToolOutput::Json(_)));
        assert!(
            matches!(backend.call("session_list", serde_json::json!({}), &client), ToolOutput::Json(value) if value["documents"].as_array().is_some_and(|docs| docs.len() == 1))
        );
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

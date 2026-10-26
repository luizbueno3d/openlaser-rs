//! The stdio MCP server (JSON-RPC 2.0, one message per line) with PhotoCraft's `serve_io` admission rules: a line is
//! read whole, within 1 MiB, before any of it is acted on; an oversized line gets an error and ends the session; a
//! malformed line gets `-32700` and the next line is served; one writer serialises every response.
//!
//! Stop is never queued (§1.3 rule 5, §3.18). The reader thread never takes the backend mutex: the mode and the
//! stop handle are taken once before the loop, `initialize`, `tools/list` and the catalogues are answered from the
//! table, and every call that needs the backend runs on a worker thread behind the mutex. A `stop` (also through
//! `tool_call` or `control_call machine.stop`, however nested) is handled on the reader thread itself, through the
//! dedicated stop connection, before its arguments are even checked: it trips first and reports bad arguments
//! afterwards. A stop sent as a notification (no `id`) trips too; it just gets no answer.
//!
//! Panics are contained: a worker's panic is a tool error, the poisoned mutex is recovered for the next call, and a
//! panic while handling a line on the reader thread is a `-32603` reply; the session goes on.

use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use base64::Engine as _;
use openlaser_control::budgets;
use serde_json::{Value, json};

use crate::Error;
use crate::backend::{ToolBackend, ToolOutput};
use crate::bridge::StopHandle;
use crate::tools::{self, ClientInfo, Mode};

/// Protocol versions with an `initialize` handshake that this server speaks (rmcp 3.5's set).
const SUPPORTED_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const LATEST_WITH_INITIALIZE: &str = "2025-11-25";
/// The 2026-07-28 style: the version (and the client) travel in each request's `_meta`.
const MODERN: &str = "2026-07-28";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
/// Worker threads waiting on or holding the backend; past this a call is answered with an error, not queued.
const MAX_IN_FLIGHT: usize = 16;

/// AUT B4, verbatim; the backend is appended.
const INSTRUCTIONS: &str = "OpenLaser designs and runs laser jobs, LightBurn-style. Units are millimetres, Y up; on a galvo the origin is the field centre. Start with doc_new, doc_open or doc_import; discover command ids and their parameter schemas with command_list; edit with command_run or command_batch (each step undoable with edit.undo). Check a job with job_plan (refusals plus the summary decoded from the bytes that would be sent), job_preview and job_simulate. You cannot fire the laser or move the machine: job_submit hands the job to the person at the machine, who frames and arms it in the OpenLaser window; poll job_status. stop always works in bridge mode. File paths are relative to the read/write roots granted at launch. machine_status, frame_request, job_submit, stop and the ui_* tools need bridge mode (openlaser-cli mcp --bridge auto).";

const PROMPTS: &[(&str, &str, &str)] = &[
    (
        "prepare-job",
        "Import a design, assign layers, plan, fix refusals, simulate, and submit it for the person at the machine.",
        "Prepare a laser job. 1) doc_import the design (or doc_open a project). 2) doc_inspect, then assign objects to layers and set cut settings with command_run (command_list and command_schema give ids and parameters). 3) job_plan: if ok is false, fix every refusal it lists (move or scale objects into the work area, change settings) and plan again. 4) job_preview and job_simulate to check the burn decoded from the exact bytes. 5) job_submit with a short note; the person frames and arms it in the OpenLaser window. Poll job_status. You cannot start the laser yourself, and stop always works.",
    ),
    (
        "mopa-color-test",
        "Build a MOPA frequency × pulse test grid and plan it.",
        "Build a MOPA colour test: doc_new, then draw a grid of small filled squares with tools.rectangle, one layer per square, each with its own frequency (kHz) and pulse width (ns) set through layer.cutSettings.set, and a text-free legend. Keep everything inside the work area from machine_list. Then job_plan and job_simulate, and report the grid of settings.",
    ),
    (
        "fix-refusals",
        "Read job_plan's refusals and fix them.",
        "Run job_plan. For each refusal, read its code, object and message; doc_inspect the object; fix the cause with command_run (move it into the work area, close an open shape to fill it, change a layer setting within the source limits). Never try to bypass a refusal: there is no override. Plan again until ok is true.",
    ),
];

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

struct Server {
    backend: Mutex<Box<dyn ToolBackend>>,
    mode: Mode,
    stop: Option<StopHandle>,
    compact: bool,
    client: Mutex<ClientInfo>,
    out: Out,
}

/// Serves MCP on `input`/`output` until end of input (Ok) or an oversized line (an error reply, then Err).
pub(crate) fn serve(
    backend: Box<dyn ToolBackend>,
    compact: bool,
    client: ClientInfo,
    mut input: impl BufRead,
    output: impl Write + Send + 'static,
) -> Result<(), Error> {
    // Taken once, here: the reader never locks the backend.
    let mode = backend.mode();
    let stop = backend.stop_handle();
    let out: Out = Arc::new(Mutex::new(Box::new(output)));
    let server = Arc::new(Server { backend: Mutex::new(backend), mode, stop, compact, client: Mutex::new(client), out });
    let mut calls: Vec<JoinHandle<()>> = Vec::new();
    let result = loop {
        calls.retain(|call| !call.is_finished());
        match openlaser_control::wire::read_line_bounded(&mut input, budgets::MAX_REQUEST_BYTES) {
            Ok(Some(line)) => {
                if catch_unwind(AssertUnwindSafe(|| server.handle_line(&line, &mut calls))).is_err() {
                    server.write(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal error: handling the request panicked; the session continues"}}));
                }
            }
            Ok(None) => break Ok(()),
            Err(openlaser_control::Error::LineTooLong { max }) => {
                server.write(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":format!("request exceeds {max} bytes; the session ends")}}));
                break Err(Error::Control(openlaser_control::Error::LineTooLong { max }));
            }
            Err(e) => break Err(Error::Control(e)),
        }
    };
    for call in calls {
        let _ = call.join();
    }
    result
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.into()}})
}

fn tool_error_result(message: &str) -> Value {
    json!({"content":[{"type":"text","text":message}],"isError":true})
}

/// How deep `tool_call` wrappers are followed looking for a stop (JSON nesting is bounded anyway; this bounds the
/// recursion on the reader thread).
const STOP_NESTING_MAX: usize = 8;

/// `stop` reached any way: the stop tool, `control_call {method:"machine.stop"}`, or either of them wrapped in
/// `tool_call` (also `tool_call` → `control_call` → `machine.stop`, and `tool_call` inside `tool_call`, which would
/// otherwise be refused: a stop is never refused for its wrapping).
fn stop_arguments(name: &str, args: &Value) -> Option<Value> {
    stop_arguments_within(name, args, STOP_NESTING_MAX)
}

fn stop_arguments_within(name: &str, args: &Value, depth: usize) -> Option<Value> {
    match name {
        "stop" => Some(args.clone()),
        "control_call" if args.get("method").and_then(Value::as_str) == Some("machine.stop") => Some(args.get("params").cloned().unwrap_or_else(|| json!({}))),
        "tool_call" if depth > 0 => {
            let inner = args.get("name").and_then(Value::as_str)?;
            let inner_args = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
            stop_arguments_within(inner, &inner_args, depth.saturating_sub(1))
        }
        _ => None,
    }
}

/// The stop arguments of a `tools/call` that is a stop, however it is wrapped.
fn stop_call_arguments(params: &Value) -> Option<Value> {
    let name = params.get("name").and_then(Value::as_str)?;
    stop_arguments(name, &params.get("arguments").cloned().unwrap_or_else(|| json!({})))
}

impl Server {
    fn write(&self, response: Value) {
        let response = match serde_json::to_vec(&response) {
            Ok(bytes) if bytes.len() < budgets::MAX_RESPONSE_BYTES => bytes,
            _ => {
                let id = response.get("id").cloned().unwrap_or(Value::Null);
                let replacement = json!({"jsonrpc":"2.0","id":id,"result":tool_error_result("response exceeds 8388608 bytes; operation may have completed; inspect state before retrying")});
                serde_json::to_vec(&replacement).unwrap_or_default()
            }
        };
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = out.write_all(&response).and_then(|()| out.write_all(b"\n")).and_then(|()| out.flush());
    }

    fn client(&self) -> ClientInfo {
        self.client.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The MCP client's identity: shown by the GUI ("Agent connected: …") and recorded on submissions.
    fn set_client(&self, info: &Value) {
        let text = |key: &str| info.get(key).and_then(Value::as_str).unwrap_or("unknown").to_owned();
        let client = ClientInfo { name: text("name"), version: text("version") };
        if let Some(stop) = &self.stop {
            stop.set_client(&client);
        }
        *self.client.lock().unwrap_or_else(PoisonError::into_inner) = client;
    }

    fn handle_line(self: &Arc<Self>, line: &[u8], calls: &mut Vec<JoinHandle<()>>) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let request: Value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(e) => return self.write(rpc_error(&Value::Null, -32700, format!("Parse error: {e}"))),
        };
        let Some(object) = request.as_object() else { return self.write(rpc_error(&Value::Null, -32600, "Invalid Request: not an object")) };
        let id = object.get("id").cloned();
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            if let Some(id) = id {
                self.write(rpc_error(&id, -32600, "Invalid Request: no method"));
            }
            return;
        };
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        let meta = params.get("_meta").or_else(|| object.get("_meta"));
        let modern = meta.and_then(|m| m.get(META_VERSION)).and_then(Value::as_str) == Some(MODERN);
        if let Some(info) = meta.and_then(|m| m.get(META_CLIENT)) {
            self.set_client(info);
        }
        // Notifications (initialized, cancelled, …) need no answer; a stop sent as one still trips.
        let Some(id) = id else {
            if method == "tools/call"
                && let Some(stop_args) = stop_call_arguments(&params)
            {
                let _unanswered = self.stop_call(&Value::Null, &stop_args, modern);
            }
            return;
        };
        let response = match method {
            "initialize" => self.initialize(&id, &params),
            "ping" => self.result(&id, json!({}), modern),
            "tools/list" => self.tools_list(&id, modern),
            "tools/call" => {
                if let Some(stop_args) = stop_call_arguments(&params) {
                    self.stop_call(&id, &stop_args, modern)
                } else {
                    let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    return self.spawn(calls, id, move |server, id| server.tool_call(id, &name, args, modern));
                }
            }
            "resources/list" => self.resources_list(&id, modern),
            "resources/templates/list" => self.templates_list(&id, modern),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or_default().to_owned();
                return self.spawn(calls, id, move |server, id| server.resource_read(id, &uri, modern));
            }
            "prompts/list" => self.result(
                &id,
                json!({"prompts": PROMPTS.iter().map(|(name, description, _)| json!({"name":name,"description":description})).collect::<Vec<_>>()}),
                modern,
            ),
            "prompts/get" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
                match PROMPTS.iter().find(|(n, _, _)| *n == name) {
                    Some((_, description, text)) => {
                        self.result(&id, json!({"description":description,"messages":[{"role":"user","content":{"type":"text","text":text}}]}), modern)
                    }
                    None => rpc_error(&id, -32602, format!("unknown prompt `{name}`")),
                }
            }
            _ => rpc_error(&id, -32601, format!("Method not found: {method}")),
        };
        self.write(response);
    }

    fn result(&self, id: &Value, mut result: Value, modern: bool) -> Value {
        if modern && let Some(object) = result.as_object_mut() {
            object.insert("resultType".to_owned(), json!("complete"));
        }
        json!({"jsonrpc":"2.0","id":id,"result":result})
    }

    /// A catalogue result: the 2026-07-28 list fields when the request asked for that version (PhotoCraft).
    fn list_result(&self, id: &Value, key: &str, items: Value, modern: bool) -> Value {
        if modern {
            json!({"jsonrpc":"2.0","id":id,"result":{key: items, "resultType":"complete","ttlMs":600000,"cacheScope":"private"}})
        } else {
            json!({"jsonrpc":"2.0","id":id,"result":{key: items}})
        }
    }

    fn initialize(&self, id: &Value, params: &Value) -> Value {
        if let Some(info) = params.get("clientInfo") {
            self.set_client(info);
        }
        let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
        let version = SUPPORTED_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(LATEST_WITH_INITIALIZE);
        let backend = if self.mode == Mode::Headless { "headless" } else { "bridge" };
        json!({"jsonrpc":"2.0","id":id,"result":{
            "protocolVersion": version,
            "capabilities": {"tools":{"listChanged":false},"resources":{"subscribe":false,"listChanged":false},"prompts":{"listChanged":false}},
            "serverInfo": {"name":"openlaser","title":"OpenLaser","version":env!("CARGO_PKG_VERSION")},
            "instructions": format!("{INSTRUCTIONS} Backend: {backend}"),
        }})
    }

    fn listed(&self) -> impl Iterator<Item = &'static tools::ToolDef> + '_ {
        // `stop` is always listed, in both modes and under --compact.
        tools().iter().filter(|def| (def.modes.contains(&self.mode) || def.name == "stop") && (!self.compact || def.compact))
    }

    fn tools_list(&self, id: &Value, modern: bool) -> Value {
        self.list_result(id, "tools", Value::Array(self.listed().map(tools::ToolDef::listing).collect()), modern)
    }

    fn resources_list(&self, id: &Value, modern: bool) -> Value {
        let resources = json!([
            {"uri":"openlaser://document","name":"document","title":"Active document","mimeType":"application/json","description":"Live doc_inspect of the active document."},
            {"uri":"openlaser://commands","name":"commands","title":"Command catalog","mimeType":"application/json","description":"Live command_list, with enabled state and agent class."},
            {"uri":"openlaser://machines","name":"machines","title":"Machine profiles","mimeType":"application/json","description":"Live machine_list."},
            {"uri":"openlaser://job","name":"job","title":"Job check","mimeType":"application/json","description":"job_plan of the active document: refusals and the decoded summary."},
        ]);
        self.list_result(id, "resources", resources, modern)
    }

    fn templates_list(&self, id: &Value, modern: bool) -> Value {
        let templates = json!([
            {"uriTemplate":"openlaser://layer/{key}","name":"layer","mimeType":"application/json","description":"One layer of the active document (doc_inspect)."},
            {"uriTemplate":"openlaser://object/{id}","name":"object","mimeType":"application/json","description":"One object of the active document (doc_inspect)."},
            {"uriTemplate":"openlaser://command/{id}","name":"command","mimeType":"application/json","description":"One command's parameter schema."},
            {"uriTemplate":"openlaser://submission/{id}","name":"submission","mimeType":"application/json","description":"One job submission's state (bridge)."},
        ]);
        self.list_result(id, "resourceTemplates", templates, modern)
    }

    /// Runs `work` on a worker thread (it may wait for the backend mutex); the reader goes on reading.
    fn spawn(self: &Arc<Self>, calls: &mut Vec<JoinHandle<()>>, id: Value, work: impl FnOnce(&Server, &Value) -> Value + Send + 'static) {
        if calls.len() >= MAX_IN_FLIGHT {
            return self.write(json!({"jsonrpc":"2.0","id":id,"result":tool_error_result("too many calls in flight; wait for their replies and retry")}));
        }
        let server = Arc::clone(self);
        let worker_id = id.clone();
        let spawned = std::thread::Builder::new().name("openlaser-mcp-call".to_owned()).spawn(move || {
            let response = catch_unwind(AssertUnwindSafe(|| work(&server, &worker_id))).unwrap_or_else(
                |_| json!({"jsonrpc":"2.0","id":worker_id,"result":tool_error_result("internal error: the tool call panicked; the session continues")}),
            );
            server.write(response);
        });
        match spawned {
            Ok(handle) => calls.push(handle),
            Err(e) => self.write(json!({"jsonrpc":"2.0","id":id,"result":tool_error_result(&format!("internal error: cannot start the call ({e}); retry"))})),
        }
    }

    /// On the reader thread, without the backend mutex: trip first, then report anything wrong with the arguments.
    fn stop_call(&self, id: &Value, args: &Value, modern: bool) -> Value {
        let machine = args.get("machine").and_then(Value::as_str);
        let output = match &self.stop {
            Some(handle) => handle.stop(machine),
            None => ToolOutput::Error {
                message: "needs_bridge: stop needs the running OpenLaser window (openlaser-cli mcp --bridge auto); a headless session has no machine to stop"
                    .to_owned(),
                data: None,
            },
        };
        let problem = tools::find("stop").and_then(|def| tools::check(&(def.input_schema)(), args).err());
        let mut response = self.render(id, output, modern);
        if let Some(problem) = problem
            && let Some(content) = response.pointer_mut("/result/content").and_then(Value::as_array_mut)
        {
            let target = machine.map_or_else(|| "every machine".to_owned(), |m| format!("machine `{m}`"));
            content.push(json!({"type":"text","text":format!("note: {problem} (ignored: stop never refuses its arguments; it was sent for {target})")}));
        }
        response
    }

    fn tool_call(&self, id: &Value, name: &str, args: Value, modern: bool) -> Value {
        let Some(def) = tools::find(name) else { return rpc_error(id, -32602, format!("unknown tool `{name}`")) };
        if !def.modes.contains(&self.mode) {
            let message = format!(
                "needs_bridge: `{name}` needs the running OpenLaser window: turn on Automation › Allow Agent Connections, then use `openlaser-cli mcp --bridge auto`"
            );
            return self.render(id, ToolOutput::Error { message, data: None }, modern);
        }
        if let Err(reason) = tools::check(&(def.input_schema)(), &args) {
            return rpc_error(id, -32602, format!("{reason} for {name}"));
        }
        match name {
            "tool_search" => self.render(id, self.search(&args), modern),
            "tool_call" => {
                let inner = args.get("name").and_then(Value::as_str).unwrap_or_default();
                if inner == "tool_call" {
                    return rpc_error(id, -32602, "tool_call cannot call tool_call");
                }
                self.tool_call(id, inner, args.get("arguments").cloned().unwrap_or_else(|| json!({})), modern)
            }
            _ => {
                let client = self.client();
                let output = self.backend.lock().unwrap_or_else(PoisonError::into_inner).call(name, args, &client);
                self.render(id, output, modern)
            }
        }
    }

    fn search(&self, args: &Value) -> ToolOutput {
        let available = |def: &tools::ToolDef| def.modes.contains(&self.mode) || def.name == "stop";
        if let Some(name) = args.get("name").and_then(Value::as_str) {
            return match tools::find(name).filter(|def| available(def)) {
                Some(def) => ToolOutput::Json(def.listing()),
                None => ToolOutput::Error { message: format!("no tool `{name}` in this mode"), data: None },
            };
        }
        let query = args.get("query").and_then(Value::as_str).map(str::to_lowercase);
        let category = args.get("category").and_then(Value::as_str);
        let rows: Vec<Value> = tools()
            .iter()
            .filter(|def| available(def))
            .filter(|def| category.is_none_or(|c| def.name.split('_').next() == Some(c)))
            .filter(|def| query.as_ref().is_none_or(|q| def.name.contains(q.as_str()) || def.description.to_lowercase().contains(q.as_str())))
            .map(|def| json!({"name":def.name,"title":def.title(),"description":def.description,"readOnly":def.hints.read_only,"listed":!self.compact || def.compact}))
            .collect();
        ToolOutput::Json(Value::Array(rows))
    }

    fn render(&self, id: &Value, output: ToolOutput, modern: bool) -> Value {
        let result = match output {
            ToolOutput::Json(value) => json!({"content":[{"type":"text","text":value.to_string()}],"isError":false}),
            ToolOutput::Png { png, note } => {
                if png.len() > budgets::MAX_PNG_BYTES {
                    tool_error_result(&format!("image is {} bytes, over the {} byte PNG budget; ask for a smaller max_side", png.len(), budgets::MAX_PNG_BYTES))
                } else {
                    let data = base64::engine::general_purpose::STANDARD.encode(png);
                    json!({"content":[{"type":"text","text":note.to_string()},{"type":"image","data":data,"mimeType":"image/png"}],"isError":false})
                }
            }
            ToolOutput::Error { message, data } => {
                let mut content = vec![json!({"type":"text","text":message})];
                if let Some(data) = data {
                    content.push(json!({"type":"text","text":data.to_string()}));
                }
                json!({"content":content,"isError":true})
            }
        };
        self.result(id, result, modern)
    }

    fn resource_read(&self, id: &Value, uri: &str, modern: bool) -> Value {
        let rest = |prefix: &str| uri.strip_prefix(prefix).filter(|r| !r.is_empty()).map(str::to_owned);
        let (tool, args, pick): (&str, Value, Option<(&str, String)>) = match uri {
            "openlaser://document" => ("doc_inspect", json!({}), None),
            "openlaser://commands" => ("command_list", json!({}), None),
            "openlaser://machines" => ("machine_list", json!({}), None),
            "openlaser://job" => ("job_plan", json!({}), None),
            _ => {
                if let Some(command) = rest("openlaser://command/") {
                    ("command_schema", json!({"id": command}), None)
                } else if let Some(submission) = rest("openlaser://submission/") {
                    ("job_status", json!({"submission": submission}), None)
                } else if let Some(key) = rest("openlaser://layer/") {
                    ("doc_inspect", json!({}), Some(("layer", key)))
                } else if let Some(object) = rest("openlaser://object/") {
                    ("doc_inspect", json!({"objects": "all"}), Some(("object", object)))
                } else {
                    return rpc_error(id, -32002, format!("unknown resource `{uri}`"));
                }
            }
        };
        if !tools::find(tool).is_some_and(|def| def.modes.contains(&self.mode)) {
            return rpc_error(id, -32002, format!("`{uri}` needs bridge mode (openlaser-cli mcp --bridge auto)"));
        }
        let client = self.client();
        let output = self.backend.lock().unwrap_or_else(PoisonError::into_inner).call(tool, args, &client);
        let value = match output {
            ToolOutput::Json(value) => value,
            ToolOutput::Png { note, .. } => note,
            ToolOutput::Error { message, .. } => return rpc_error(id, -32603, message),
        };
        let value = match pick {
            None => value,
            Some(("layer", key)) => match value
                .get("layers")
                .and_then(Value::as_array)
                .and_then(|layers| layers.iter().find(|l| l.get("key").and_then(Value::as_str) == Some(key.as_str())))
            {
                Some(layer) => layer.clone(),
                None => return rpc_error(id, -32002, format!("no layer `{key}` in the active document")),
            },
            Some((_, wanted)) => {
                let items = value.pointer("/objects/items").and_then(Value::as_array);
                match items.and_then(|items| items.iter().find(|o| o.get("id").map(|v| v.to_string().trim_matches('"').to_owned()) == Some(wanted.clone()))) {
                    Some(object) => object.clone(),
                    None => return rpc_error(id, -32002, format!("no object `{wanted}` in the active document")),
                }
            }
        };
        let contents = json!([{"uri":uri,"mimeType":"application/json","text":value.to_string()}]);
        if modern {
            json!({"jsonrpc":"2.0","id":id,"result":{"contents":contents,"resultType":"complete","ttlMs":0,"cacheScope":"private"}})
        } else {
            json!({"jsonrpc":"2.0","id":id,"result":{"contents":contents}})
        }
    }
}

fn tools() -> &'static [tools::ToolDef] {
    tools::tools()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::BufReader;
    use std::net::SocketAddr;
    use std::sync::mpsc::{Receiver, Sender};
    use std::time::{Duration, Instant};

    use openlaser_control::server::{ControlRequest, ControlServer, FastStop, ServerConfig};
    use openlaser_control::token::Token;
    use openlaser_control::wire::Reply;

    use super::*;
    use crate::backend::{Bridge, Headless};
    use crate::{BridgeTarget, TokenSource};

    /// Collects complete output lines as JSON values.
    struct LineSink {
        tx: Sender<Value>,
        buf: Vec<u8>,
    }

    impl Write for LineSink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.buf.extend_from_slice(bytes);
            while let Some(end) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=end).collect();
                if let Ok(value) = serde_json::from_slice(&line) {
                    let _ = self.tx.send(value);
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The real read loop on a pipe, as the CLI runs it on stdin/stdout.
    struct Harness {
        input: Option<std::io::PipeWriter>,
        lines: Receiver<Value>,
        early: HashMap<String, Value>,
        thread: Option<std::thread::JoinHandle<Result<(), Error>>>,
    }

    impl Harness {
        fn start(backend: Box<dyn ToolBackend>, compact: bool) -> Self {
            let (reader, writer) = std::io::pipe().unwrap();
            let (tx, lines) = std::sync::mpsc::channel();
            let client = ClientInfo { name: "openlaser-cli".to_owned(), version: "test".to_owned() };
            let thread = std::thread::spawn(move || serve(backend, compact, client, BufReader::new(reader), LineSink { tx, buf: Vec::new() }));
            Harness { input: Some(writer), lines, early: HashMap::new(), thread: Some(thread) }
        }

        fn send(&mut self, message: Value) {
            self.send_raw(format!("{message}\n").as_bytes());
        }

        fn send_raw(&mut self, bytes: &[u8]) {
            self.input.as_mut().unwrap().write_all(bytes).unwrap();
        }

        fn call(&mut self, id: u64, name: &str, arguments: Value) {
            self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}}));
        }

        /// The response with `id`, within `within`; responses to other ids are kept for later.
        fn response(&mut self, id: Value, within: Duration) -> Option<Value> {
            let key = id.to_string();
            if let Some(found) = self.early.remove(&key) {
                return Some(found);
            }
            let deadline = Instant::now() + within;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let line = self.lines.recv_timeout(left).ok()?;
                let line_id = line.get("id").map(Value::to_string).unwrap_or_default();
                if line_id == key {
                    return Some(line);
                }
                self.early.insert(line_id, line);
            }
        }

        fn finish(mut self) -> Result<(), Error> {
            drop(self.input.take());
            self.thread.take().unwrap().join().unwrap()
        }
    }

    fn text(response: &Value) -> String {
        response
            .pointer("/result/content")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    fn is_error(response: &Value) -> bool {
        response.pointer("/result/isError").and_then(Value::as_bool) == Some(true)
    }

    struct Gui {
        server: ControlServer,
        requests: Receiver<ControlRequest>,
        stops: Arc<Mutex<Vec<Option<String>>>>,
        target: BridgeTarget,
    }

    fn gui_on(port: u16, report: impl Fn(Option<&str>) -> Value + Send + Sync + 'static) -> Gui {
        let token = Token::generate().unwrap();
        let hex = token.to_hex();
        let stops = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&stops);
        let fast_stop: FastStop = Arc::new(move |machine| {
            seen.lock().unwrap().push(machine.map(str::to_owned));
            report(machine)
        });
        let (server, requests) = ControlServer::start(ServerConfig { port, token, fast_stop, wake: Arc::new(|| {}) }).unwrap();
        let target = BridgeTarget::Addr { addr: SocketAddr::from(([127, 0, 0, 1], server.port())), token: TokenSource::Inline(hex) };
        Gui { server, requests, stops, target }
    }

    fn gui() -> Gui {
        gui_on(0, |_| json!({"latched": true, "confirmed": true, "elapsedMs": 2}))
    }

    fn bridge(target: &BridgeTarget) -> Box<dyn ToolBackend> {
        Box::new(Bridge::new(target.clone(), &ClientInfo { name: "openlaser-cli".to_owned(), version: "test".to_owned() }))
    }

    fn stops(gui: &Gui) -> Vec<Option<String>> {
        gui.stops.lock().unwrap().clone()
    }

    const STOP_WITHIN: Duration = Duration::from_secs(3);

    /// The stop delivery contract (bridge.rs module docs): at least one FastStop call since `before`, every one of them
    /// for `machine`. Not exactly one: a stop whose connection is dropped under it may reach the GUI and still be
    /// resent on a fresh connection (Windows delivers bytes to a read already waiting on a socket shut down under it).
    fn assert_stopped_since(gui: &Gui, before: usize, machine: Option<&str>, context: &str) {
        let seen = stops(gui);
        let new = seen.get(before..).unwrap_or_default();
        assert!(!new.is_empty(), "{context}: the stop never reached FastStop ({seen:?})");
        assert!(new.iter().all(|m| m.as_deref() == machine), "{context}: a delivery was for another machine than {machine:?}: {seen:?}");
    }

    fn wait_live(gui: &Gui, n: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        while gui.server.live_connections() != n && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(gui.server.live_connections(), n, "the GUI's connections did not end");
    }

    /// B1: the GUI drops the stop connection (its 30 s idle timeout, or every agent connection at once); the next stop
    /// through the MCP loop reconnects, resends, reaches FastStop for the requested machine, and reports success.
    /// Once with the stop sent while the drop is still under way, once after the dropped connection is gone.
    #[test]
    fn stop_reconnects_after_idle_drop() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "stop", json!({}));
        let first = mcp.response(json!(1), STOP_WITHIN).unwrap();
        assert!(!is_error(&first), "{first}");
        assert_eq!(stops(&gui), [None]);
        let machine = "jpt-m7-200-sim";
        for (id, settle) in [(2, false), (3, true)] {
            gui.server.disconnect_all();
            if settle {
                // Every connection thread ends promptly on every OS, so the stop meets a connection that is gone.
                wait_live(&gui, 0);
            }
            let before = stops(&gui).len();
            mcp.call(id, "stop", json!({"machine": machine}));
            let response = mcp.response(json!(id), STOP_WITHIN).unwrap();
            assert!(!is_error(&response), "the stop after an idle drop failed (settled: {settle}): {response}");
            let report: Value = serde_json::from_str(&text(&response)).unwrap();
            assert_eq!(report["latched"], json!(true), "{response}");
            assert_stopped_since(&gui, before, Some(machine), &format!("settled: {settle}"));
        }
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    /// B1: the MCP server starts before the GUI listens; the stop handle is made on use, so a stop still reaches it.
    #[test]
    fn stop_handle_is_created_lazily() {
        let port = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let token = Token::generate().unwrap();
        let target = BridgeTarget::Addr { addr: SocketAddr::from(([127, 0, 0, 1], port)), token: TokenSource::Inline(token.to_hex()) };
        let mut mcp = Harness::start(bridge(&target), false);
        mcp.call(1, "stop", json!({}));
        let early = mcp.response(json!(1), STOP_WITHIN).unwrap();
        assert!(is_error(&early) && text(&early).starts_with("not_connected:"), "{early}");
        let stops = Arc::new(Mutex::new(0_usize));
        let seen = Arc::clone(&stops);
        let (server, _requests) = ControlServer::start(ServerConfig {
            port,
            token,
            fast_stop: Arc::new(move |_| {
                *seen.lock().unwrap() += 1;
                json!({"latched": true})
            }),
            wake: Arc::new(|| {}),
        })
        .unwrap();
        mcp.call(2, "stop", json!({}));
        let late = mcp.response(json!(2), STOP_WITHIN).unwrap();
        assert!(!is_error(&late), "{late}");
        assert_eq!(*stops.lock().unwrap(), 1);
        mcp.finish().unwrap();
        server.shutdown();
    }

    /// B2: a call parked in the GUI holds the backend; tools/list, initialize and stop are still answered at once.
    #[test]
    fn mcp_stop_not_blocked_by_inflight_call() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "doc_inspect", json!({}));
        let parked = gui.requests.recv_timeout(Duration::from_secs(5)).expect("the call reached the GUI");
        assert!(parked.dispatch.try_dispatch());
        let started = Instant::now();
        mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        mcp.send(json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"t","version":"1"}}}));
        mcp.call(4, "stop", json!({}));
        assert!(mcp.response(json!(2), Duration::from_secs(1)).is_some(), "tools/list waited behind the in-flight call");
        assert!(mcp.response(json!(3), Duration::from_secs(1)).is_some(), "initialize waited behind the in-flight call");
        let stop = mcp.response(json!(4), STOP_WITHIN).expect("stop waited behind the in-flight call");
        assert!(!is_error(&stop), "{stop}");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(stops(&gui).len(), 1);
        parked.reply.send(Reply { id: parked.request.id, ok: true, result: Some(json!({"name":"card"})), error: None }).unwrap();
        let inspected = mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        assert!(text(&inspected).contains("card"));
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    /// B3: a stop with bad arguments trips first (every machine when `machine` is not a string) and reports them.
    #[test]
    fn stop_with_bad_arguments_still_trips() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "stop", json!({"machine": 5, "bogus": true}));
        let response = mcp.response(json!(1), STOP_WITHIN).unwrap();
        assert!(response.get("error").is_none() && !is_error(&response), "{response}");
        assert!(text(&response).contains("unknown argument `bogus`"), "{response}");
        mcp.call(2, "stop", json!({"machine": "m1", "extra": 1}));
        assert!(!is_error(&mcp.response(json!(2), STOP_WITHIN).unwrap()));
        mcp.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"stop","arguments":"everything"}}));
        assert!(!is_error(&mcp.response(json!(3), STOP_WITHIN).unwrap()));
        mcp.call(4, "tool_call", json!({"name":"stop","arguments":{"bogus":1}}));
        assert!(!is_error(&mcp.response(json!(4), STOP_WITHIN).unwrap()));
        mcp.call(5, "control_call", json!({"method":"machine.stop","params":{"machine":"m2","x":1}}));
        assert!(!is_error(&mcp.response(json!(5), STOP_WITHIN).unwrap()));
        assert_eq!(stops(&gui), [None, Some("m1".to_owned()), None, None, Some("m2".to_owned())]);
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    /// Keeps `n` silent sockets open on `port`, reopening each as soon as the server closes it (X1's flood).
    struct Flood {
        stop: Arc<std::sync::atomic::AtomicBool>,
        reopened: Arc<std::sync::atomic::AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Flood {
        fn start(port: u16, n: usize) -> Self {
            use std::io::Read;
            use std::sync::atomic::Ordering;
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reopened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (flag, count) = (Arc::clone(&stop), Arc::clone(&reopened));
            let thread = std::thread::spawn(move || {
                let open = || {
                    let stream = std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).ok()?;
                    stream.set_nonblocking(true).ok()?;
                    Some(stream)
                };
                let mut sockets: Vec<Option<std::net::TcpStream>> = (0..n).map(|_| open()).collect();
                let mut byte = [0_u8; 1];
                while !flag.load(Ordering::Acquire) {
                    for socket in &mut sockets {
                        let closed = match socket {
                            Some(stream) => match stream.read(&mut byte) {
                                Ok(_) => true,
                                Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
                            },
                            None => true,
                        };
                        if closed {
                            *socket = open();
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    std::thread::yield_now();
                }
            });
            Flood { stop, reopened, thread: Some(thread) }
        }

        fn finish(mut self) -> usize {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
            self.reopened.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    /// An authenticated raw client of the GUI (another agent holding one of its 16 slots).
    fn agent(gui: &Gui, name: &str) -> std::net::TcpStream {
        use std::io::BufRead;
        let BridgeTarget::Addr { addr, token: TokenSource::Inline(token) } = &gui.target else { panic!("addr target") };
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        writeln!(stream, "{}", json!({"id": 0, "method": "auth", "params": {"token": token, "client": {"name": name, "version": "1"}}})).unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
        let reply: Reply = serde_json::from_str(&line).unwrap();
        assert!(reply.ok, "{reply:?}");
        stream
    }

    /// X1 regression: 16 authenticated agents fill every agent slot and ≥ 40 silent sockets (reopened as they close)
    /// flood the GUI; a stop through the real MCP read loop and bridge still reaches FastStop within the delivery
    /// budget, round after round, and again once the agents leave (then on a full, kept-warm connection).
    #[test]
    fn stop_reaches_fast_stop_through_a_silent_socket_flood() {
        let gui = gui();
        let agents: Vec<std::net::TcpStream> = (0..budgets::MAX_CONNECTIONS).map(|index| agent(&gui, &format!("agent-{index}"))).collect();
        assert_eq!(gui.server.clients().len(), budgets::MAX_CONNECTIONS);
        let flood = Flood::start(gui.server.port(), 48);
        // Whatever the server's design, the flood now holds every socket it is allowed (with 16 agents, that is at
        // least the stop reserve): wait for that, then let it churn a little.
        let until = Instant::now() + Duration::from_secs(5);
        while gui.server.live_connections() < budgets::MAX_CONNECTIONS + budgets::STOP_ONLY_RESERVE && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(gui.server.live_connections() >= budgets::MAX_CONNECTIONS + budgets::STOP_ONLY_RESERVE, "the flood did not start");
        std::thread::sleep(Duration::from_millis(200));
        let mut mcp = Harness::start(bridge(&gui.target), false);
        let budget = crate::bridge::STOP_DELIVERY_BUDGET;
        let mut id = 0_u64;
        let mut round = |mcp: &mut Harness| {
            id += 1;
            let before = stops(&gui).len();
            let started = Instant::now();
            mcp.call(id, "stop", json!({}));
            let response = mcp.response(json!(id), budget + Duration::from_millis(500)).expect("no answer to the stop");
            assert!(!is_error(&response), "round {id}: {response}");
            // The handle's own budget, plus the stdio pipe around it.
            assert!(started.elapsed() < budget + Duration::from_millis(100), "round {id}: {:?}", started.elapsed());
            assert_stopped_since(&gui, before, None, &format!("round {id}"));
        };
        for _ in 1..=3 {
            round(&mut mcp);
        }
        drop(agents);
        for _ in 4..=6 {
            round(&mut mcp);
        }
        assert!(flood.finish() > 0, "the flood never churned");
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    /// `tool_call` → `control_call` → `machine.stop` (and `tool_call` → `tool_call` → `stop`) take the stop fast path
    /// on the reader thread: they trip while a parked call holds the backend.
    #[test]
    fn nested_stop_takes_the_fast_path() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "doc_inspect", json!({}));
        let parked = gui.requests.recv_timeout(Duration::from_secs(5)).expect("the call reached the GUI");
        assert!(parked.dispatch.try_dispatch());
        mcp.call(2, "tool_call", json!({"name":"control_call","arguments":{"method":"machine.stop","params":{"machine":"m3"}}}));
        let nested = mcp.response(json!(2), STOP_WITHIN).expect("the nested stop waited behind the in-flight call");
        assert!(!is_error(&nested), "{nested}");
        mcp.call(3, "tool_call", json!({"name":"tool_call","arguments":{"name":"stop","arguments":{"machine":"m4"}}}));
        let twice = mcp.response(json!(3), STOP_WITHIN).expect("the doubly wrapped stop waited behind the in-flight call");
        assert!(!is_error(&twice), "{twice}");
        assert_eq!(stops(&gui), [Some("m3".to_owned()), Some("m4".to_owned())]);
        parked.reply.send(Reply { id: parked.request.id, ok: true, result: Some(json!({"name":"card"})), error: None }).unwrap();
        assert!(mcp.response(json!(1), Duration::from_secs(5)).is_some());
        // Not a stop, however wrapped: still the usual refusal.
        mcp.call(4, "tool_call", json!({"name":"tool_call","arguments":{"name":"session_list"}}));
        assert_eq!(mcp.response(json!(4), Duration::from_secs(5)).unwrap().pointer("/error/code"), Some(&json!(-32602)));
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    /// A stop sent as a notification (no `id`) trips, and gets no answer; the session goes on.
    #[test]
    fn stop_notification_still_trips() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.send(json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"stop","arguments":{}}}));
        mcp.send(json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"tool_call","arguments":{"name":"control_call","arguments":{"method":"machine.stop","params":{"machine":"m5"}}}}}));
        mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}));
        assert!(mcp.response(json!(1), STOP_WITHIN).is_some());
        assert_eq!(stops(&gui), [None, Some("m5".to_owned())]);
        assert!(mcp.response(Value::Null, Duration::from_millis(200)).is_none(), "a notification was answered");
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    /// B4: the GUI's report reaches the agent unchanged; a reply without `latched`, or with `latched:false`, is an
    /// error, never a success.
    #[test]
    fn stop_report_is_never_fabricated() {
        let gui = gui_on(0, |_| json!({"confirmed": false, "elapsedMs": 7}));
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "stop", json!({}));
        let response = mcp.response(json!(1), STOP_WITHIN).unwrap();
        assert!(is_error(&response), "{response}");
        assert!(!text(&response).contains("\"latched\":true"), "{response}");
        mcp.finish().unwrap();
        gui.server.shutdown();
        let gui = gui_on(0, |_| json!({"latched": false, "confirmed": false, "elapsedMs": 0, "stopped": [], "note": "no machine is connected"}));
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "stop", json!({}));
        let response = mcp.response(json!(1), STOP_WITHIN).unwrap();
        assert!(is_error(&response) && text(&response).starts_with("not_connected:"), "latched:false was a success: {response}");
        mcp.finish().unwrap();
        gui.server.shutdown();
        let gui = gui_on(0, |m| json!({"latched": true, "confirmed": true, "elapsedMs": 9, "machine": m}));
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.call(1, "stop", json!({"machine":"m"}));
        let response = mcp.response(json!(1), STOP_WITHIN).unwrap();
        let report: Value = serde_json::from_str(&text(&response)).unwrap();
        assert_eq!(report, json!({"latched": true, "confirmed": true, "elapsedMs": 9, "machine": "m"}));
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    struct PanickingBackend;

    impl ToolBackend for PanickingBackend {
        fn mode(&self) -> Mode {
            Mode::Headless
        }
        fn call(&mut self, tool: &str, _: Value, _: &ClientInfo) -> ToolOutput {
            assert_ne!(tool, "doc_new", "backend panic");
            ToolOutput::Json(json!({"documents":[],"active":null}))
        }
        fn stop_handle(&self) -> Option<StopHandle> {
            None
        }
    }

    /// B5: a panicking tool call is a tool error, the mutex it poisoned is recovered, and the next call works.
    #[test]
    fn panic_contained() {
        let mut mcp = Harness::start(Box::new(PanickingBackend), false);
        mcp.call(1, "doc_new", json!({}));
        let panicked = mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        assert!(is_error(&panicked) && text(&panicked).contains("panicked"), "{panicked}");
        mcp.call(2, "session_list", json!({}));
        let next = mcp.response(json!(2), Duration::from_secs(5)).unwrap();
        assert!(!is_error(&next), "the call after a panic failed: {next}");
        mcp.finish().unwrap();
    }

    #[test]
    fn malformed_line_recovers_and_unknown_argument_is_32602() {
        let mut mcp = Harness::start(Box::new(Headless::new(None, None).unwrap()), false);
        mcp.send_raw(b"not-json\n");
        assert_eq!(mcp.response(Value::Null, Duration::from_secs(5)).unwrap().pointer("/error/code"), Some(&json!(-32700)));
        mcp.call(1, "session_list", json!({"force": true}));
        let refused = mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        assert_eq!(refused.pointer("/error/code"), Some(&json!(-32602)));
        assert!(refused.pointer("/error/message").and_then(Value::as_str).unwrap().contains("`force`"));
        mcp.call(2, "frame_request", json!({}));
        let needs = mcp.response(json!(2), Duration::from_secs(5)).unwrap();
        assert!(is_error(&needs) && text(&needs).starts_with("needs_bridge:"), "{needs}");
        mcp.finish().unwrap();
    }

    #[test]
    fn oversized_line_ends_session_with_an_error_reply() {
        let mut mcp = Harness::start(Box::new(Headless::new(None, None).unwrap()), false);
        let mut line = vec![b' '; budgets::MAX_REQUEST_BYTES + 1];
        line.push(b'\n');
        mcp.send_raw(&line);
        let reply = mcp.response(Value::Null, Duration::from_secs(5)).unwrap();
        assert!(reply.pointer("/error/message").and_then(Value::as_str).unwrap().contains("exceeds"));
        assert!(matches!(mcp.finish(), Err(Error::Control(openlaser_control::Error::LineTooLong { .. }))));
    }

    #[test]
    fn modern_list_fields_and_protocol_version() {
        let mut mcp = Harness::start(Box::new(Headless::new(None, None).unwrap()), false);
        mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","clientInfo":{"name":"c","version":"1"}}}));
        let init = mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        assert_eq!(init.pointer("/result/protocolVersion"), Some(&json!("2025-03-26")));
        assert!(init.pointer("/result/instructions").and_then(Value::as_str).unwrap().ends_with("Backend: headless"));
        mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}));
        assert_eq!(mcp.response(json!(2), Duration::from_secs(5)).unwrap().pointer("/result/protocolVersion"), Some(&json!(LATEST_WITH_INITIALIZE)));
        mcp.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}));
        let modern = mcp.response(json!(3), Duration::from_secs(5)).unwrap();
        assert_eq!(modern.pointer("/result/resultType"), Some(&json!("complete")));
        assert_eq!(modern.pointer("/result/ttlMs"), Some(&json!(600000)));
        assert_eq!(modern.pointer("/result/cacheScope"), Some(&json!("private")));
        mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}));
        assert!(mcp.response(json!(4), Duration::from_secs(5)).unwrap().pointer("/result/resultType").is_none());
        mcp.finish().unwrap();
    }

    #[test]
    fn compact_lists_exactly_the_spec_set_and_stop() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), true);
        mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
        let listed = mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        let names: Vec<&str> = listed.pointer("/result/tools").and_then(Value::as_array).unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        let expected: Vec<&str> = tools().iter().filter(|t| t.compact).map(|t| t.name).collect();
        assert_eq!(names, expected);
        assert!(names.contains(&"stop"));
        mcp.call(2, "tool_search", json!({"name":"machine_list"}));
        let found = mcp.response(json!(2), Duration::from_secs(5)).unwrap();
        assert!(text(&found).contains("inputSchema"), "{found}");
        mcp.finish().unwrap();
        gui.server.shutdown();
        let mut headless = Harness::start(Box::new(Headless::new(None, None).unwrap()), false);
        headless.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
        let listed = headless.response(json!(1), Duration::from_secs(5)).unwrap();
        let names: Vec<&str> = listed.pointer("/result/tools").and_then(Value::as_array).unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"stop") && !names.contains(&"job_submit") && !names.contains(&"ui_click"));
        headless.finish().unwrap();
    }

    /// Item 4: the MCP client's name reaches the GUI in the auth frame of the (lazily opened) connections.
    #[test]
    fn client_info_is_forwarded_to_the_gui() {
        let gui = gui();
        let mut mcp = Harness::start(bridge(&gui.target), false);
        mcp.send(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"claude-code","version":"2.1"}}}),
        );
        mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        mcp.call(2, "session_list", json!({}));
        let request = gui.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.client, "claude-code 2.1");
        assert!(request.dispatch.try_dispatch());
        request.reply.send(Reply { id: request.request.id, ok: true, result: Some(json!({"documents":[]})), error: None }).unwrap();
        mcp.response(json!(2), Duration::from_secs(5)).unwrap();
        mcp.finish().unwrap();
        gui.server.shutdown();
    }

    #[test]
    fn resources_are_live() {
        let mut mcp = Harness::start(Box::new(Headless::new(None, None).unwrap()), false);
        mcp.call(1, "doc_new", json!({"name":"live"}));
        mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"openlaser://document"}}));
        let document = mcp.response(json!(2), Duration::from_secs(5)).unwrap();
        let body: Value = serde_json::from_str(document.pointer("/result/contents/0/text").and_then(Value::as_str).unwrap()).unwrap();
        assert_eq!(body["name"], "live", "{body}");
        mcp.send(json!({"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"openlaser://command/file.new"}}));
        assert!(mcp.response(json!(3), Duration::from_secs(5)).unwrap().pointer("/result/contents").is_some());
        mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"openlaser://nothing"}}));
        assert_eq!(mcp.response(json!(4), Duration::from_secs(5)).unwrap().pointer("/error/code"), Some(&json!(-32002)));
        mcp.send(json!({"jsonrpc":"2.0","id":5,"method":"prompts/get","params":{"name":"prepare-job"}}));
        assert!(mcp.response(json!(5), Duration::from_secs(5)).unwrap().pointer("/result/messages/0/content/text").is_some());
        mcp.finish().unwrap();
    }

    /// AUT B10.2: `--bridge auto` without a running window answers every call with the reason and never falls back
    /// to a headless session (edits must not land in an invisible document).
    #[test]
    fn bridge_without_gui_says_so_and_never_falls_back() {
        let temp = tempfile::tempdir().unwrap();
        let target = BridgeTarget::File(temp.path().join("control.json"));
        let mut mcp = Harness::start(bridge(&target), false);
        mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}));
        let init = mcp.response(json!(1), Duration::from_secs(5)).unwrap();
        assert!(init.pointer("/result/instructions").and_then(Value::as_str).unwrap().ends_with("Backend: bridge"));
        for (id, tool) in [(2, "doc_new"), (3, "session_list"), (4, "stop")] {
            mcp.call(id, tool, json!({}));
            let reply = mcp.response(json!(id), STOP_WITHIN).unwrap();
            assert!(is_error(&reply), "{tool}: {reply}");
            assert!(text(&reply).contains(crate::bridge::NOT_RUNNING), "{tool}: {reply}");
        }
        mcp.finish().unwrap();
    }
}

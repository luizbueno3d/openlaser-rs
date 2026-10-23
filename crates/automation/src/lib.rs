//! Agent automation (SPEC §3.18, §4; owned by WP7): the MCP server, the headless backend and the GUI bridge.
//! This crate can never link `arm`, `human`, `transport` or a USB crate (§2.3 rule 5).
//!
#![forbid(unsafe_code)]

mod backend;
mod bridge;
mod tools;

use std::path::PathBuf;

pub use backend::{Bridge, Headless, StopHandle, ToolBackend, ToolOutput};
pub use bridge::{BridgeClient, BridgeError};
pub use tools::{ClientInfo, Hints, Mode, ToolAccess, ToolDef, strict_schema, tools};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control: {0}")]
    Control(#[from] openlaser_control::Error),
    #[error("bridge: {0:?}")]
    Bridge(BridgeError),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub struct McpConfig {
    pub backend: BackendChoice,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    pub compact: bool,
}

#[derive(Clone)]
pub enum BackendChoice {
    Headless,
    Bridge(BridgeTarget),
}

#[derive(Clone)]
pub enum BridgeTarget {
    /// Discovery file.
    Auto,
    File(PathBuf),
    Addr {
        addr: std::net::SocketAddr,
        token: TokenSource,
    },
}

#[derive(Clone)]
pub enum TokenSource {
    Inline(String),
    File(PathBuf),
    /// OPENLASER_CONTROL_TOKEN[_FILE].
    Env,
}

/// stdio MCP (rmcp 3.5) behind PhotoCraft's `serve_io` pre-reader: 1 MiB line cap ends the session, -32700
/// recovery, single writer, `_meta` 2026-07-28 list fields. Server info {name: "openlaser", version}.
pub fn run_mcp_stdio(cfg: McpConfig) -> Result<(), Error> {
    let client = ClientInfo { name: "openlaser-cli".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() };
    let backend: Box<dyn ToolBackend> = match cfg.backend {
        BackendChoice::Headless => Box::new(Headless::new(cfg.read_root.as_deref(), cfg.write_root.as_deref())?),
        BackendChoice::Bridge(target) => Box::new(Bridge::connect(&target, &client)?),
    };
    let stdin = std::io::stdin();
    let backend = std::sync::Arc::new(std::sync::Mutex::new(backend));
    let stop = backend.lock().ok().and_then(|backend| backend.stop_handle());
    let stdout = std::sync::Arc::new(std::sync::Mutex::new(()));
    let mut reader = stdin.lock();
    let mut calls: Vec<std::thread::JoinHandle<()>> = Vec::new();
    loop {
        let line = match openlaser_control::wire::read_line_bounded(&mut reader, openlaser_control::budgets::MAX_REQUEST_BYTES) {
            Ok(Some(line)) => line,
            Ok(None) => {
                for call in calls {
                    let _ = call.join();
                }
                return Ok(());
            }
            Err(openlaser_control::Error::LineTooLong { .. }) => {
                for call in calls {
                    let _ = call.join();
                }
                return Ok(());
            }
            Err(error) => return Err(Error::Control(error)),
        };
        let request: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(error) => {
                write_mcp_response(&stdout, serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("Parse error: {error}")}}))?;
                continue;
            }
        };
        let Some(method) = request.get("method").and_then(serde_json::Value::as_str) else { continue };
        let id = request.get("id").cloned();
        let Some(id) = id else { continue };
        let params = request.get("params").cloned().unwrap_or_else(|| serde_json::json!({}));
        let modern =
            request.get("_meta").and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion")).and_then(serde_json::Value::as_str) == Some("2026-07-28");
        if method == "tools/call" {
            let params = params.clone();
            let id = id.clone();
            let backend = std::sync::Arc::clone(&backend);
            let stdout = std::sync::Arc::clone(&stdout);
            let stop = stop.clone();
            let call = std::thread::Builder::new().name("openlaser-mcp-call".to_owned()).spawn(move || {
                let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mcp_tool_call(&backend, stop.as_ref(), &id, &params)))
                    .unwrap_or_else(|_| mcp_tool_error(&id, "internal error: tool call panicked"));
                let response = bounded_mcp_response(response, &id);
                let _ = write_mcp_response(&stdout, response);
            })?;
            calls.push(call);
            continue;
        }
        let response = match method {
            "initialize" => {
                let mode = backend.lock().map(|backend| backend.mode()).unwrap_or(Mode::Headless);
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":false},"resources":{"subscribe":false,"listChanged":false},"prompts":{"listChanged":false}},"serverInfo":{"name":"openlaser","version":env!("CARGO_PKG_VERSION")},"instructions":format!("OpenLaser laser design automation. Units are millimetres, Y up; galvo origin is the field centre. Backend: {}", if mode==Mode::Headless {"headless"} else {"bridge"})}})
            }
            "ping" => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => {
                let mode = backend.lock().map(|backend| backend.mode()).unwrap_or(Mode::Headless);
                let listed: Vec<_> = tools().iter().filter(|def| (def.modes.contains(&mode) || def.name == "stop") && (!cfg.compact || def.compact || def.name == "stop" || matches!(def.name, "tool_search"|"tool_call"))).map(|def| serde_json::json!({"name":def.name,"description":def.description,"inputSchema":(def.input_schema)(),"annotations":{"readOnlyHint":def.hints.read_only,"destructiveHint":def.hints.destructive,"idempotentHint":def.hints.idempotent}})).collect();
                if modern {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"tools":listed,"resultType":"complete","ttlMs":600000,"cacheScope":"private"}})
                } else {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"tools":listed}})
                }
            }
            "resources/list" => {
                let resources = [
                    serde_json::json!({"uri":"openlaser://document","name":"document","mimeType":"application/json"}),
                    serde_json::json!({"uri":"openlaser://commands","name":"commands","mimeType":"application/json"}),
                    serde_json::json!({"uri":"openlaser://machines","name":"machines","mimeType":"application/json"}),
                    serde_json::json!({"uri":"openlaser://job","name":"job","mimeType":"application/json"}),
                ];
                if modern {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"resources":resources,"resultType":"complete","ttlMs":600000,"cacheScope":"private"}})
                } else {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"resources":resources}})
                }
            }
            "resources/read" => {
                let uri = params.get("uri").and_then(serde_json::Value::as_str).unwrap_or("");
                let text = match uri {
                    "openlaser://commands" => serde_json::to_string(&tools().iter().map(|t| t.name).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".to_owned()),
                    "openlaser://machines" => "[{\"id\":\"jpt-m7-200-sim\",\"connected\":false}]".to_owned(),
                    "openlaser://document" | "openlaser://job" => "null".to_owned(),
                    _ => String::new(),
                };
                if text.is_empty() {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"unknown resource URI"}})
                } else if modern {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":text}],"ttlMs":0}})
                } else {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":text}]}})
                }
            }
            "prompts/list" => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"prompts":[{"name":"prepare-job","description":"Plan, review and submit a job for a person"},{"name":"mopa-color-test","description":"Prepare a MOPA layer test design"},{"name":"fix-refusals","description":"Inspect and address job planning refusals"}]}})
            }
            "tools/call" => continue,
            _ => serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("Method not found: {method}")}}),
        };
        write_mcp_response(&stdout, bounded_mcp_response(response, &id))?;
    }
}

fn mcp_tool_call(
    backend: &std::sync::Arc<std::sync::Mutex<Box<dyn ToolBackend>>>,
    stop: Option<&StopHandle>,
    id: &serde_json::Value,
    params: &serde_json::Value,
) -> serde_json::Value {
    let name = params.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));
    let is_stop = name == "stop";
    let mode = if is_stop {
        Mode::Bridge
    } else {
        let Ok(guard) = backend.lock() else { return mcp_tool_error(id, "internal error: backend unavailable") };
        guard.mode()
    };
    let Some(definition) = tools().iter().find(|tool| tool.name == name && tool.modes.contains(&mode)) else {
        return serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":format!("unknown tool `{name}`")}});
    };
    if let Some(reason) = validate_arguments(&(definition.input_schema)(), &args) {
        return serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":reason}});
    }
    let info = ClientInfo { name: "mcp-client".to_owned(), version: String::new() };
    let output = if is_stop {
        stop.map_or_else(
            || ToolOutput::Error { message: "needs_bridge: stop requires bridge mode".to_owned(), data: None },
            |handle| handle.stop(args.get("machine").and_then(serde_json::Value::as_str)),
        )
    } else {
        let Ok(mut guard) = backend.lock() else { return mcp_tool_error(id, "internal error: backend unavailable") };
        guard.call(name, args, &info)
    };
    match output {
        ToolOutput::Json(value) => {
            let text = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_owned());
            serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":text}],"isError":false}})
        }
        ToolOutput::Png { png, note } => {
            use base64::Engine as _;
            let text = serde_json::to_string(&note).unwrap_or_else(|_| "{}".to_owned());
            let image = base64::engine::general_purpose::STANDARD.encode(png);
            serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":text},{"type":"image","data":image,"mimeType":"image/png"}],"isError":false}})
        }
        ToolOutput::Error { message, .. } => {
            serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":message}],"isError":true}})
        }
    }
}

fn mcp_tool_error(id: &serde_json::Value, message: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":message}],"isError":true}})
}

fn bounded_mcp_response(response: serde_json::Value, id: &serde_json::Value) -> serde_json::Value {
    let too_large = serde_json::to_vec(&response).map(|bytes| bytes.len() > openlaser_control::budgets::MAX_RESPONSE_BYTES).unwrap_or(true);
    if too_large { mcp_tool_error(id, "response exceeds 8388608 bytes; operation may have completed; inspect state before retrying") } else { response }
}

fn write_mcp_response(lock: &std::sync::Arc<std::sync::Mutex<()>>, response: serde_json::Value) -> Result<(), Error> {
    let Ok(_guard) = lock.lock() else { return Err(Error::NotImplemented("MCP response writer is unavailable")) };
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    write_json_line(&mut writer, &response)
}

fn validate_arguments(schema: &serde_json::Value, args: &serde_json::Value) -> Option<String> {
    let Some(object) = args.as_object() else { return Some("arguments must be an object".to_owned()) };
    let Some(properties) = schema.get("properties").and_then(serde_json::Value::as_object) else { return Some("invalid tool schema".to_owned()) };
    if let Some(name) = object.keys().find(|key| !properties.contains_key(*key)) {
        return Some(format!("unknown argument `{name}`"));
    }
    if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
        for key in required.iter().filter_map(serde_json::Value::as_str) {
            if !object.contains_key(key) {
                return Some(format!("missing required argument `{key}`"));
            }
        }
    }
    None
}

fn write_json_line(writer: &mut impl std::io::Write, value: &serde_json::Value) -> Result<(), Error> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub struct ServeConfig {
    pub port: Option<u16>,
    pub token: Option<TokenSource>,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
}

/// `openlaser-cli serve`: the control protocol over a headless Session (stdio: no auth; TCP: auth frame).
pub fn run_serve(cfg: ServeConfig) -> Result<(), Error> {
    let mut backend = Headless::new(cfg.read_root.as_deref(), cfg.write_root.as_deref())?;
    if let Some(port) = cfg.port {
        let token = match cfg.token {
            Some(TokenSource::Inline(value)) => openlaser_control::token::Token::from_hex(&value)?,
            Some(TokenSource::File(path)) => openlaser_control::token::read_file(&path)?,
            Some(TokenSource::Env) => {
                let path = std::env::var_os("OPENLASER_CONTROL_TOKEN_FILE")
                    .ok_or(Error::NotImplemented("OPENLASER_CONTROL_TOKEN_FILE is required for serve --port"))?;
                openlaser_control::token::read_file(&PathBuf::from(path))?
            }
            None => openlaser_control::token::Token::generate()?,
        };
        let fast_stop: openlaser_control::server::FastStop =
            std::sync::Arc::new(|machine| serde_json::json!({"latched":true,"machine":machine,"confirmed":false,"elapsedMs":0}));
        let (server, requests) = openlaser_control::server::ControlServer::start(openlaser_control::server::ServerConfig {
            port,
            token,
            fast_stop,
            wake: std::sync::Arc::new(|| {}),
        })?;
        eprintln!("openlaser-cli serve listening on 127.0.0.1:{}", server.port());
        for request in requests {
            if !request.dispatch.try_dispatch() {
                continue;
            }
            let method = request.request.method.as_str();
            let output = backend.dispatch_control(method, request.request.params);
            let reply = control_reply(request.request.id, output);
            let _ = request.reply.send(reply);
        }
        server.shutdown();
        return Ok(());
    }
    use std::io::Write;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    loop {
        let Some(line) = openlaser_control::wire::read_line_bounded(&mut input, openlaser_control::budgets::MAX_REQUEST_BYTES)? else { return Ok(()) };
        let value: serde_json::Value = serde_json::from_slice(&line)?;
        let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = value.get("method").and_then(serde_json::Value::as_str).unwrap_or("");
        let params = value.get("params").cloned().unwrap_or_else(|| serde_json::json!({}));
        let reply = serde_json::to_value(control_reply(id, backend.dispatch_control(method, params))).unwrap_or(serde_json::Value::Null);
        serde_json::to_writer(&mut output, &reply)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

fn control_reply(id: serde_json::Value, output: ToolOutput) -> openlaser_control::wire::Reply {
    match output {
        ToolOutput::Json(result) => openlaser_control::wire::Reply { id, ok: true, result: Some(result), error: None },
        ToolOutput::Png { note, .. } => openlaser_control::wire::Reply { id, ok: true, result: Some(note), error: None },
        ToolOutput::Error { message, .. } => openlaser_control::wire::Reply { id, ok: false, result: None, error: Some(message) },
    }
}

#[cfg(test)]
mod mcp_tests {
    use super::*;

    struct PanickingBackend;

    impl ToolBackend for PanickingBackend {
        fn mode(&self) -> Mode {
            Mode::Headless
        }
        fn call(&mut self, _: &str, _: serde_json::Value, _: &ClientInfo) -> ToolOutput {
            panic!("backend panic")
        }
        fn stop_handle(&self) -> Option<StopHandle> {
            None
        }
    }

    #[test]
    fn panic_contained() {
        let backend: std::sync::Arc<std::sync::Mutex<Box<dyn ToolBackend>>> = std::sync::Arc::new(std::sync::Mutex::new(Box::new(PanickingBackend)));
        let id = serde_json::json!(5);
        let params = serde_json::json!({"name":"session_list","arguments":{}});
        let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mcp_tool_call(&backend, None, &id, &params)))
            .unwrap_or_else(|_| mcp_tool_error(&id, "internal error: tool call panicked"));
        assert_eq!(response.pointer("/result/isError").and_then(serde_json::Value::as_bool), Some(true));
        assert!(response.pointer("/result/content/0/text").and_then(serde_json::Value::as_str).unwrap_or_default().contains("panicked"));
    }
}

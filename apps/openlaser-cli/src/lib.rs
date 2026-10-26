//! `openlaser-cli`: the agent CLI and MCP server (SPEC §3.21). Behaviour lives here so tests run it in process.
//!
//! `mcp` and `serve` confine files to the automation roots (none = denied). The other subcommands are trusted-local:
//! the person typed the paths, so they are used as given, relative to the current folder. Every subcommand runs the
//! engine with `Origin::Headless`, so human-only commands are refused by the engine's policy, whatever is asked; and
//! nothing here can connect a machine, frame or emit (§2.3 rule 5).
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::Write;

use openlaser_automation::{BackendChoice, BridgeClient, BridgeTarget, ClientInfo, Headless, McpConfig, ServeConfig, TokenSource, ToolBackend, ToolOutput};
use serde_json::{Value, json};

const USAGE: &str = "usage:
  openlaser-cli mcp   [--bridge auto|<127.0.0.1:port> | --bridge-file <control.json>]
                      [--control-token <hex> | --control-token-file <path>]
                      [--automation-read-root <dir>] [--automation-write-root <dir>] [--root <dir>] [--compact]
  openlaser-cli serve [--port <port>] [--control-token <hex> | --control-token-file <path>] [root flags]
  openlaser-cli ui    (--bridge-file <f> | --bridge auto | --bridge <addr> --control-token-file <f>) <method> [key=value…] [--out shot.png]
  openlaser-cli commands [--json] [--filter <text>]
  openlaser-cli info  <project.olp|file.svg|file.dxf|workspace.json> [--compact]
  openlaser-cli convert <in> <out.olp>
  openlaser-cli run   (<project> | --new <json>) --cmd <id> [--params <json>] … [--out <file.olp>]
  openlaser-cli plan  <project> [--machine <id>] [--json]          exit 1 on refusal
  openlaser-cli preview <project> --out <png> [--show burn,frame,travel,order] [--max-side N]
  openlaser-cli simulate <project> --out <png> [--listing <txt>]
  openlaser-cli estimate <project>
  openlaser-cli gcode <project> --machine <gantry id> --out <file.gcode>
  openlaser-cli job-export <project> --out <job.json>
  openlaser-cli frame|mark|run-hw …   hardware commands live in openlaser-hw
exit codes: 0 ok, 1 failure or refusal, 2 usage";

pub fn run(input: impl IntoIterator<Item = OsString>, out: &mut impl Write, err: &mut impl Write) -> u8 {
    let mut args = Vec::new();
    for arg in input {
        match arg.into_string() {
            Ok(arg) => args.push(arg),
            Err(_) => return usage_error(err, "arguments must be valid Unicode"),
        }
    }
    let Some((command, rest)) = args.split_first() else {
        let _ = writeln!(err, "{USAGE}");
        return 2;
    };
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        let _ = writeln!(out, "{USAGE}");
        return 0;
    }
    match command.as_str() {
        "frame" | "mark" | "run-hw" => {
            let _ = writeln!(err, "hardware commands live in openlaser-hw and are run by a person at a terminal");
            2
        }
        "mcp" => run_mcp(rest, err),
        "serve" => run_serve(rest, err),
        "ui" => run_ui(rest, out, err),
        "commands" => commands(rest, out, err),
        "info" => info(rest, out, err),
        "convert" => convert(rest, out, err),
        "run" => run_commands(rest, out, err),
        "plan" => plan(rest, out, err),
        "preview" | "simulate" => image(command, rest, out, err),
        "estimate" | "gcode" | "job-export" => project_tool(command, rest, out, err),
        "--help" | "-h" | "help" => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        "--version" | "-V" => {
            let _ = writeln!(out, "openlaser-cli {}", env!("CARGO_PKG_VERSION"));
            0
        }
        other => usage_error(err, &format!("unknown subcommand `{other}`\n{USAGE}")),
    }
}

/// Parsed flags of one subcommand: positionals in order, flags in order (a flag may repeat).
struct Parsed {
    positional: Vec<String>,
    flags: Vec<(&'static str, Option<String>)>,
}

impl Parsed {
    fn get(&self, name: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(flag, _)| *flag == name).and_then(|(_, value)| value.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(flag, _)| *flag == name)
    }
}

/// `flags`: (name, takes a value). Unknown flags and missing values are usage errors (exit 2).
fn parse(args: &[String], flags: &[(&'static str, bool)], max_positional: usize) -> Result<Parsed, String> {
    let mut parsed = Parsed { positional: Vec::new(), flags: Vec::new() };
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        if arg.starts_with('-') && arg.len() > 1 {
            let (name, inline) = arg.split_once('=').map_or((arg.as_str(), None), |(n, v)| (n, Some(v.to_owned())));
            let Some(&(flag, takes_value)) = flags.iter().find(|(f, _)| *f == name) else { return Err(format!("unknown flag `{name}`")) };
            let value = if takes_value {
                match inline {
                    Some(value) => Some(value),
                    None => {
                        let value = args.get(i).cloned().ok_or_else(|| format!("`{flag}` needs a value"))?;
                        i += 1;
                        Some(value)
                    }
                }
            } else if inline.is_some() {
                return Err(format!("`{flag}` takes no value"));
            } else {
                None
            };
            parsed.flags.push((flag, value));
        } else {
            if parsed.positional.len() >= max_positional {
                return Err(format!("unexpected argument `{arg}`"));
            }
            parsed.positional.push(arg.clone());
        }
    }
    Ok(parsed)
}

fn usage_error(err: &mut impl Write, message: &str) -> u8 {
    let _ = writeln!(err, "openlaser-cli: {message}");
    2
}

fn failure(err: &mut impl Write, message: &str) -> u8 {
    let _ = writeln!(err, "openlaser-cli: {message}");
    1
}

/// `value[key]` without the panicking `Index` (never-crash rule): Null when absent.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn client() -> ClientInfo {
    ClientInfo { name: "openlaser-cli".into(), version: env!("CARGO_PKG_VERSION").into() }
}

const ROOT_FLAGS: [(&str, bool); 3] = [("--automation-read-root", true), ("--automation-write-root", true), ("--root", true)];

fn roots(parsed: &Parsed) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let both = parsed.get("--root").map(std::path::PathBuf::from);
    (parsed.get("--automation-read-root").map(Into::into).or_else(|| both.clone()), parsed.get("--automation-write-root").map(Into::into).or(both))
}

fn token_source(parsed: &Parsed) -> Result<Option<TokenSource>, String> {
    match (parsed.get("--control-token"), parsed.get("--control-token-file")) {
        (Some(_), Some(_)) => Err("give --control-token or --control-token-file, not both".to_owned()),
        (Some(token), None) => Ok(Some(TokenSource::Inline(token.to_owned()))),
        (None, Some(path)) => Ok(Some(TokenSource::File(path.into()))),
        (None, None) => Ok(None),
    }
}

/// `--bridge auto|<addr>` / `--bridge-file <f>` and the token flags (an address needs a token; auto and the file
/// carry their own).
fn bridge_target(parsed: &Parsed) -> Result<Option<BridgeTarget>, String> {
    let token = token_source(parsed)?;
    let target = match (parsed.get("--bridge"), parsed.get("--bridge-file")) {
        (Some(_), Some(_)) => return Err("give --bridge or --bridge-file, not both".to_owned()),
        (Some("auto"), None) => Some(BridgeTarget::Auto),
        (Some(addr), None) => {
            let addr: std::net::SocketAddr =
                addr.parse().map_err(|_| format!("--bridge expects auto or a loopback address such as 127.0.0.1:7878, got `{addr}`"))?;
            if !addr.ip().is_loopback() {
                return Err(format!("--bridge must be a loopback address, got `{addr}`"));
            }
            return Ok(Some(BridgeTarget::Addr { addr, token: token.unwrap_or(TokenSource::Env) }));
        }
        (None, Some(path)) => Some(BridgeTarget::File(path.into())),
        (None, None) => None,
    };
    if token.is_some() {
        return Err("token flags go with --bridge <addr> (auto and --bridge-file read the token from the discovery file)".to_owned());
    }
    Ok(target)
}

fn run_mcp(args: &[String], err: &mut impl Write) -> u8 {
    let flags = [("--bridge", true), ("--bridge-file", true), ("--control-token", true), ("--control-token-file", true), ("--compact", false)];
    let parsed = match parse(args, &[&flags[..], &ROOT_FLAGS[..]].concat(), 0) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let target = match bridge_target(&parsed) {
        Ok(target) => target,
        Err(message) => return usage_error(err, &message),
    };
    let (read_root, write_root) = roots(&parsed);
    if target.is_some() && (read_root.is_some() || write_root.is_some()) {
        let _ = writeln!(err, "openlaser-cli: note: with --bridge the OpenLaser window's own roots apply; the root flags are ignored");
    }
    let backend = target.map_or(BackendChoice::Headless, BackendChoice::Bridge);
    match openlaser_automation::run_mcp_stdio(McpConfig { backend, read_root, write_root, compact: parsed.has("--compact") }) {
        Ok(()) => 0,
        Err(e) => failure(err, &e.to_string()),
    }
}

fn run_serve(args: &[String], err: &mut impl Write) -> u8 {
    let flags = [("--port", true), ("--control-token", true), ("--control-token-file", true)];
    let parsed = match parse(args, &[&flags[..], &ROOT_FLAGS[..]].concat(), 0) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let port = match parsed.get("--port").map(str::parse::<u16>) {
        None => None,
        Some(Ok(port)) => Some(port),
        Some(Err(_)) => return usage_error(err, "--port must be a port number (0 = any free port)"),
    };
    let token = match token_source(&parsed) {
        Ok(token) => token,
        Err(message) => return usage_error(err, &message),
    };
    if port.is_none() && token.is_some() {
        return usage_error(err, "token flags go with --port (stdio needs no authentication)");
    }
    let (read_root, write_root) = roots(&parsed);
    let token = if port.is_some() { token.or(Some(TokenSource::Env)) } else { None };
    match openlaser_automation::run_serve(ServeConfig { port, token, read_root, write_root }) {
        Ok(()) => 0,
        Err(e) => failure(err, &e.to_string()),
    }
}

/// `ui <method> key=value…`: one control method on the running window. Values are JSON when they parse as JSON
/// (`count=2`, `fields={"a":1}`), else text.
fn run_ui(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let flags = [("--bridge", true), ("--bridge-file", true), ("--control-token", true), ("--control-token-file", true), ("--out", true)];
    let parsed = match parse(args, &flags, usize::MAX) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let target = match bridge_target(&parsed) {
        Ok(Some(target)) => target,
        Ok(None) => return usage_error(err, "ui needs --bridge-file <f>, --bridge auto or --bridge <addr> --control-token-file <f>"),
        Err(message) => return usage_error(err, &message),
    };
    let Some((method, pairs)) = parsed.positional.split_first() else { return usage_error(err, "ui needs a control method, for example ui.inspect") };
    let mut params = serde_json::Map::new();
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else { return usage_error(err, &format!("expected key=value, got `{pair}`")) };
        params.insert(key.to_owned(), serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned())));
    }
    let mut bridge = match BridgeClient::connect(&target, &client()) {
        Ok(bridge) => bridge,
        Err(e) => return failure(err, &format!("{e:?}")),
    };
    let params = Value::Object(params);
    let result = if openlaser_control::methods::is_read_only(method) { bridge.call_read(method, params) } else { bridge.call(method, params) };
    let result = match result {
        Ok(result) => result,
        Err(e) => return failure(err, &format!("{e:?}")),
    };
    let (png, rest) = match openlaser_automation::png_from_result(result) {
        Ok(split) => split,
        Err(message) => return failure(err, &message),
    };
    match (png, parsed.get("--out")) {
        (Some(png), Some(path)) => {
            if let Err(e) = std::fs::write(path, png) {
                return failure(err, &format!("{path}: {e}"));
            }
        }
        (Some(_), None) => return failure(err, "the result is an image; pass --out <file.png>"),
        (None, Some(_)) => return failure(err, "--out was given but the result has no image"),
        (None, None) => {}
    }
    let _ = writeln!(out, "{rest}");
    0
}

fn commands(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let parsed = match parse(args, &[("--json", false), ("--filter", true)], 0) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let filter = parsed.get("--filter").map(str::to_lowercase);
    let rows: Vec<Value> = openlaser_engine::registry()
        .specs()
        .iter()
        .filter(|row| filter.as_ref().is_none_or(|f| row.id.to_lowercase().contains(f) || row.label.to_lowercase().contains(f)))
        .map(|row| json!({"id": row.id, "label": row.label, "params": row.doc}))
        .collect();
    if parsed.has("--json") {
        let _ = writeln!(out, "{}", Value::Array(rows));
    } else {
        for row in rows {
            let _ = writeln!(out, "{}\t{}", field(&row, "id").as_str().unwrap_or_default(), field(&row, "label").as_str().unwrap_or_default());
        }
    }
    0
}

/// A trusted-local session with `path` open.
fn open_project(path: &str, err: &mut impl Write) -> Result<Headless, u8> {
    let mut backend = Headless::trusted_local();
    match backend.call("doc_open", json!({"path": path}), &client()) {
        ToolOutput::Error { message, .. } => Err(failure(err, &message)),
        _ => Ok(backend),
    }
}

fn print_output(output: ToolOutput, out: &mut impl Write, err: &mut impl Write) -> u8 {
    match output {
        ToolOutput::Json(value) => {
            let _ = writeln!(out, "{value}");
            0
        }
        ToolOutput::Png { note, .. } => {
            let _ = writeln!(out, "{note}");
            0
        }
        ToolOutput::Error { message, .. } => failure(err, &message),
    }
}

fn info(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let parsed = match parse(args, &[("--compact", false)], 1) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let Some(path) = parsed.positional.first() else { return usage_error(err, "info needs a file") };
    let mut backend = match open_project(path, err) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let compact = parsed.has("--compact");
    match backend.call("doc_inspect", json!({"objects": if compact { "summary" } else { "all" }}), &client()) {
        ToolOutput::Json(value) if !compact => {
            let _ = writeln!(out, "{}", serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()));
            0
        }
        output => print_output(output, out, err),
    }
}

fn convert(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let parsed = match parse(args, &[], 2) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let [input, output] = parsed.positional.as_slice() else { return usage_error(err, "convert needs <in> <out.olp>") };
    if !std::path::Path::new(output).extension().is_some_and(|e| e.eq_ignore_ascii_case("olp")) {
        return usage_error(err, "convert writes OpenLaser projects: the output must end in .olp");
    }
    let mut backend = match open_project(input, err) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    print_output(backend.call("doc_save", json!({"path": output}), &client()), out, err)
}

fn run_commands(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let parsed = match parse(args, &[("--cmd", true), ("--params", true), ("--new", true), ("--out", true)], 1) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let mut commands: Vec<(String, Value)> = Vec::new();
    for (flag, value) in &parsed.flags {
        let value = value.clone().unwrap_or_default();
        match *flag {
            "--cmd" => commands.push((value, json!({}))),
            "--params" => {
                let Some(last) = commands.last_mut() else { return usage_error(err, "--params must follow the --cmd it belongs to") };
                last.1 = match serde_json::from_str(&value) {
                    Ok(params) => params,
                    Err(e) => return usage_error(err, &format!("invalid --params JSON: {e}")),
                };
            }
            _ => {}
        }
    }
    let mut backend = match (parsed.positional.first(), parsed.get("--new")) {
        (Some(_), Some(_)) => return usage_error(err, "give a project or --new, not both"),
        (None, None) => return usage_error(err, "run needs a project or --new <json>"),
        (Some(project), None) => match open_project(project, err) {
            Ok(backend) => backend,
            Err(code) => return code,
        },
        (None, Some(new)) => {
            let params: Value = match serde_json::from_str(new) {
                Ok(params) => params,
                Err(e) => return usage_error(err, &format!("invalid --new JSON: {e}")),
            };
            let mut backend = Headless::trusted_local();
            if let ToolOutput::Error { message, .. } = backend.call("doc_new", params, &client()) {
                return failure(err, &message);
            }
            backend
        }
    };
    // Every command goes to the engine as Origin::Headless: its policy refuses human-only commands (§2.3 rule 13).
    for (id, params) in commands {
        if print_output(backend.call("command_run", json!({"id": id, "params": params}), &client()), out, err) != 0 {
            return 1;
        }
    }
    match parsed.get("--out") {
        Some(path) => print_output(backend.call("doc_save", json!({"path": path}), &client()), out, err),
        None => 0,
    }
}

fn plan(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let parsed = match parse(args, &[("--machine", true), ("--json", false)], 1) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let Some(path) = parsed.positional.first() else { return usage_error(err, "plan needs a project") };
    let mut backend = match open_project(path, err) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let mut request = json!({});
    if let (Some(machine), Some(object)) = (parsed.get("--machine"), request.as_object_mut()) {
        object.insert("machine".to_owned(), json!(machine));
    }
    let ToolOutput::Json(value) = backend.call("job_plan", request, &client()) else { return failure(err, "job_plan failed") };
    let ok = value.get("ok").and_then(Value::as_bool) == Some(true);
    if parsed.has("--json") {
        let _ = writeln!(out, "{value}");
    } else if ok {
        let summary = field(&value, "summary");
        let _ = writeln!(
            out,
            "ok: fingerprint {}, {} row(s), estimate {} s, {} notice(s)",
            field(&value, "short").as_str().unwrap_or_default(),
            field(summary, "rows").as_array().map_or(0, Vec::len),
            field(summary, "estimateS"),
            field(&value, "notices").as_array().map_or(0, Vec::len)
        );
    } else {
        let _ = writeln!(err, "refused:");
        for refusal in field(&value, "refusals").as_array().into_iter().flatten() {
            let _ = writeln!(err, "  {}: {}", field(refusal, "code").as_str().unwrap_or("refused"), field(refusal, "message").as_str().unwrap_or_default());
        }
        if let Some(error) = field(&value, "error").as_str() {
            let _ = writeln!(err, "  {error}");
        }
    }
    if ok { 0 } else { 1 }
}

fn write_png(output: ToolOutput, path: &str, listing: Option<&str>, out: &mut impl Write, err: &mut impl Write) -> u8 {
    match output {
        ToolOutput::Png { png, note } => {
            if let Err(e) = std::fs::write(path, png) {
                return failure(err, &format!("{path}: {e}"));
            }
            if let Some(listing_path) = listing {
                let lines: Vec<&str> = field(field(&note, "listing"), "lines").as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let mut text = lines.join("\n");
                text.push('\n');
                if let Err(e) = std::fs::write(listing_path, text) {
                    return failure(err, &format!("{listing_path}: {e}"));
                }
            }
            let _ = writeln!(out, "{note}");
            0
        }
        ToolOutput::Json(value) => failure(err, &format!("expected an image, got {value}")),
        ToolOutput::Error { message, .. } => failure(err, &message),
    }
}

fn image(command: &str, args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let flags: &[(&'static str, bool)] =
        if command == "preview" { &[("--out", true), ("--show", true), ("--max-side", true)] } else { &[("--out", true), ("--listing", true)] };
    let parsed = match parse(args, flags, 1) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let Some(path) = parsed.positional.first() else { return usage_error(err, &format!("{command} needs a project")) };
    let Some(png_path) = parsed.get("--out") else { return usage_error(err, &format!("{command} needs --out <file.png>")) };
    let mut request = serde_json::Map::new();
    if let Some(side) = parsed.get("--max-side") {
        match side.parse::<u32>() {
            Ok(side) if (1..=2048).contains(&side) => request.insert("max_side".to_owned(), json!(side)),
            _ => return usage_error(err, "--max-side must be from 1 to 2048"),
        };
    }
    let tool = if command == "simulate" {
        request.insert("listing".to_owned(), json!(parsed.has("--listing")));
        "job_simulate"
    } else if let Some(show) = parsed.get("--show") {
        let items: Vec<&str> = show.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if let Some(bad) = items.iter().find(|s| !matches!(**s, "burn" | "frame" | "travel" | "order")) {
            return usage_error(err, &format!("--show takes burn, frame, travel, order; got `{bad}`"));
        }
        request.insert("show".to_owned(), json!(items));
        "job_preview"
    } else {
        "render_preview"
    };
    let mut backend = match open_project(path, err) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let output = backend.call(tool, Value::Object(request), &client());
    write_png(output, png_path, parsed.get("--listing"), out, err)
}

fn project_tool(command: &str, args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let flags: &[(&'static str, bool)] = match command {
        "gcode" => &[("--machine", true), ("--out", true)],
        "job-export" => &[("--out", true)],
        _ => &[],
    };
    let parsed = match parse(args, flags, 1) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(err, &message),
    };
    let Some(path) = parsed.positional.first() else { return usage_error(err, &format!("{command} needs a project")) };
    let (tool, request) = match command {
        "estimate" => ("job_estimate", json!({})),
        "gcode" => {
            let (Some(machine), Some(output)) = (parsed.get("--machine"), parsed.get("--out")) else {
                return usage_error(err, "gcode needs --machine <gantry id> and --out <file.gcode>");
            };
            ("gcode_export", json!({"path": output, "machine": machine}))
        }
        _ => {
            let Some(output) = parsed.get("--out") else { return usage_error(err, "job-export needs --out <job.json>") };
            ("job_export", json!({"path": output}))
        }
    };
    let mut backend = match open_project(path, err) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    print_output(backend.call(tool, request, &client()), out, err)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn invoke(args: &[&str]) -> (u8, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(args.iter().map(OsString::from), &mut out, &mut err);
        (code, String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned())
    }

    #[test]
    fn cli_unknown_flag_exit_2() {
        for args in [
            &["commands", "--mystery"][..],
            &["info", "a.olp", "--mystery"],
            &["plan", "a.olp", "--force"],
            &["serve", "--token", "x"],
            &["mcp", "--bridge"],
            &["run", "--new", "{}", "--params", "{}"],
            &["convert", "a.svg", "b.svg"],
            &["serve", "--control-token-file", "t"],
            &["mcp", "--bridge", "10.0.0.1:7878"],
            &["mystery"],
        ] {
            assert_eq!(invoke(args).0, 2, "{args:?}");
        }
    }

    #[test]
    fn cli_hw_subcommands_redirect() {
        for command in ["frame", "mark", "run-hw"] {
            let (code, _, err) = invoke(&[command, "x.olp"]);
            assert_eq!(code, 2);
            assert!(err.contains("openlaser-hw"));
        }
    }

    /// The CLI has no list of its own: the engine's policy refuses every human-only command for Origin::Headless,
    /// with the engine's wording (the CLI's old hardcoded text said "command is started by a person").
    #[test]
    fn cli_run_is_headless_origin() {
        for command in ["laser.start", "device.settings.set", "device.create", "laser.frame"] {
            let (code, out, err) = invoke(&["run", "--new", "{}", "--cmd", command]);
            assert_eq!(code, 1, "{command}");
            assert!(out.is_empty(), "{command} ran: {out}");
            assert!(err.contains(&format!("human-only: `{command}` is started by a person in the OpenLaser window")), "{command}: {err}");
        }
    }

    #[test]
    fn cli_run_creates_a_headless_document() {
        let (code, out, err) =
            invoke(&["run", "--new", r#"{"name":"CLI project"}"#, "--cmd", "tools.rectangle", "--params", r#"{"x_mm":0,"y_mm":0,"w_mm":5,"h_mm":5}"#]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("\"ids\""), "{out}");
    }

    /// Relative paths are read once, from the current folder (cargo runs tests in the package folder). The old
    /// code made the file's folder the root and then resolved the same relative path again inside it.
    #[test]
    fn cli_relative_paths_resolve_once() {
        let (code, _, err) = invoke(&["info", "src/main.rs"]);
        assert_eq!(code, 1);
        assert!(!err.contains("os error 2") && !err.to_lowercase().contains("no such file"), "src/main.rs was not found: {err}");
        let (code, _, err) = invoke(&["info", "src/missing.olp", "--compact"]);
        assert_eq!(code, 1, "--compact is accepted");
        assert!(err.contains("src/missing.olp") && !err.contains("src/src/"), "{err}");
    }

    #[test]
    fn cli_plan_missing_project_exit_1() {
        let (code, _, err) = invoke(&["plan", "no-such-project.olp"]);
        assert_eq!(code, 1);
        assert!(err.contains("no-such-project.olp"));
    }

    /// A real refusal: a rectangle outside the ±100 mm field. Needs I1 (format, plan and safety are stubs here).
    #[test]
    fn cli_plan_exit_1_on_refusal() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("outside.olp").to_string_lossy().into_owned();
        let (code, _, err) =
            invoke(&["run", "--new", "{}", "--cmd", "tools.rectangle", "--params", r#"{"x_mm":150,"y_mm":150,"w_mm":20,"h_mm":20}"#, "--out", &project]);
        assert_eq!(code, 0, "{err}");
        let (code, out, _) = invoke(&["plan", &project, "--json"]);
        assert_eq!(code, 1);
        let plan: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(plan["ok"], false);
        assert!(plan["refusals"].as_array().unwrap().iter().any(|r| r["code"] == "outside_work_area"), "{plan}");
    }

    #[test]
    fn cli_convert_writes_olp() {
        let temp = tempfile::tempdir().unwrap();
        let inputs = openlaser_testkit::golden_dir().join("inputs");
        let out = temp.path().join("workspace.olp").to_string_lossy().into_owned();
        let (code, _, err) = invoke(&["convert", &inputs.join("workspace.json").to_string_lossy(), &out]);
        assert_eq!(code, 0, "{err}");
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
    }

    #[test]
    fn cli_simulate_writes_png() {
        let temp = tempfile::tempdir().unwrap();
        let png = temp.path().join("simulation.png");
        let listing = temp.path().join("listing.txt");
        let inputs = openlaser_testkit::golden_dir().join("inputs");
        let workspace = temp.path().join("workspace.json");
        assert!(std::fs::copy(inputs.join("workspace.json"), &workspace).is_ok());
        assert!(std::fs::copy(inputs.join("layer_settings.json"), temp.path().join("layer_settings.json")).is_ok());
        let (code, _, err) = invoke(&["simulate", &workspace.to_string_lossy(), "--out", &png.to_string_lossy(), "--listing", &listing.to_string_lossy()]);
        assert_eq!(code, 0, "{err}");
        assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!std::fs::read_to_string(&listing).unwrap().is_empty());
    }

    /// `ui` speaks to a running window (here an in-process control server standing in for it).
    #[test]
    fn cli_ui_calls_the_running_window() {
        use openlaser_control::server::{ControlServer, ServerConfig};
        let temp = tempfile::tempdir().unwrap();
        let token_file = temp.path().join("token");
        let token = openlaser_control::token::create_or_reuse_file(&token_file).unwrap();
        let (server, requests) = ControlServer::start(ServerConfig {
            port: 0,
            token,
            fast_stop: Arc::new(|machine| json!({"latched": true, "machine": machine})),
            wake: Arc::new(|| {}),
        })
        .unwrap();
        let gui = std::thread::spawn(move || {
            let request = requests.recv().unwrap();
            assert!(request.dispatch.try_dispatch());
            let reply = json!({"method": request.request.method, "params": request.request.params});
            request.reply.send(openlaser_control::wire::Reply { id: request.request.id, ok: true, result: Some(reply), error: None }).unwrap();
        });
        let addr = format!("127.0.0.1:{}", server.port());
        let token_path = token_file.to_string_lossy().into_owned();
        let (code, out, err) = invoke(&["ui", "--bridge", &addr, "--control-token-file", &token_path, "ui.click", "id=save", "count=2"]);
        assert_eq!(code, 0, "{err}");
        let echoed: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(echoed, json!({"method":"ui.click","params":{"id":"save","count":2}}));
        let (code, out, err) = invoke(&["ui", "--bridge", &addr, "--control-token-file", &token_path, "machine.stop"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("\"latched\":true"));
        gui.join().unwrap();
        server.shutdown();
        assert_eq!(invoke(&["ui", "ui.inspect"]).0, 2, "ui without a bridge target is a usage error");
    }
}

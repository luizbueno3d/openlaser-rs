//! In-process CLI entry point so command behavior can be tested without spawning a process.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use openlaser_automation::{BackendChoice, BridgeTarget, ClientInfo, McpConfig, ServeConfig, TokenSource, ToolBackend};

pub fn run(input: impl IntoIterator<Item = OsString>, out: &mut impl Write, err: &mut impl Write) -> u8 {
    let mut args = Vec::new();
    for arg in input {
        match arg.into_string() {
            Ok(arg) => args.push(arg),
            Err(_) => {
                let _ = writeln!(err, "openlaser-cli: arguments must be valid Unicode");
                return 2;
            }
        }
    }
    let Some((command, rest)) = args.split_first() else {
        usage(err);
        return 2;
    };
    match command.as_str() {
        "frame" | "mark" | "run-hw" => {
            let _ = writeln!(err, "hardware commands live in openlaser-hw and are run by a person at a terminal");
            2
        }
        "mcp" => run_mcp(rest, err),
        "serve" => run_serve(rest, err),
        "commands" => commands(rest, out, err),
        "plan" => run_plan(rest, out, err),
        "run" => run_commands(rest, out, err),
        "info" | "convert" | "preview" | "simulate" | "estimate" | "gcode" | "job-export" => run_project_command(command, rest, out, err),
        "ui" => {
            let _ = writeln!(err, "openlaser-cli: `ui` requires the running OpenLaser window; use `mcp --bridge auto`");
            1
        }
        "--help" | "help" => {
            usage(out);
            0
        }
        command => {
            let _ = writeln!(err, "openlaser-cli: unknown subcommand `{command}`");
            usage(err);
            2
        }
    }
}

fn usage(out: &mut impl Write) {
    let _ = writeln!(out, "usage: openlaser-cli <mcp|serve|ui|commands|info|convert|run|plan|preview|simulate|estimate|gcode|job-export|frame|mark|run-hw> …");
}

fn run_mcp(args: &[String], err: &mut impl Write) -> u8 {
    let mut bridge = None;
    let mut token = None;
    let mut read_root = None;
    let mut write_root = None;
    let mut compact = false;
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).map(String::as_str);
        match args.get(i).map(String::as_str).unwrap_or("") {
            "--bridge" => {
                let Some(value) = value else { return usage_error(err, "--bridge needs a value") };
                bridge = Some(if value == "auto" {
                    BridgeTarget::Auto
                } else {
                    match value.parse() {
                        Ok(addr) => BridgeTarget::Addr { addr, token: TokenSource::Env },
                        Err(_) => return usage_error(err, "--bridge expects auto or a loopback address"),
                    }
                });
                i += 2;
            }
            "--bridge-file" => {
                let Some(value) = value else { return usage_error(err, "--bridge-file needs a path") };
                bridge = Some(BridgeTarget::File(value.into()));
                i += 2;
            }
            "--control-token" => {
                let Some(value) = value else { return usage_error(err, "--control-token needs a value") };
                token = Some(TokenSource::Inline(value.to_owned()));
                i += 2;
            }
            "--control-token-file" => {
                let Some(value) = value else { return usage_error(err, "--control-token-file needs a path") };
                token = Some(TokenSource::File(value.into()));
                i += 2;
            }
            "--automation-read-root" => {
                let Some(value) = value else { return usage_error(err, "--automation-read-root needs a path") };
                read_root = Some(value.into());
                i += 2;
            }
            "--automation-write-root" => {
                let Some(value) = value else { return usage_error(err, "--automation-write-root needs a path") };
                write_root = Some(value.into());
                i += 2;
            }
            "--root" => {
                let Some(value) = value else { return usage_error(err, "--root needs a path") };
                read_root = Some(value.into());
                write_root = Some(value.into());
                i += 2;
            }
            "--compact" => {
                compact = true;
                i += 1;
            }
            flag => return usage_error(err, &format!("unknown flag `{flag}`")),
        }
    }
    if let Some(source) = token {
        if let Some(BridgeTarget::Addr { token, .. }) = bridge.as_mut() {
            *token = source;
        } else {
            let _ = writeln!(err, "openlaser-cli: token flag requires --bridge <addr>");
            return 2;
        }
    }
    let backend = bridge.map_or(BackendChoice::Headless, BackendChoice::Bridge);
    match openlaser_automation::run_mcp_stdio(McpConfig { backend, read_root, write_root, compact }) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "openlaser-cli: {e}");
            1
        }
    }
}

fn run_serve(args: &[String], err: &mut impl Write) -> u8 {
    let mut port = None;
    let mut read_root = None;
    let mut write_root = None;
    let mut token = None;
    let mut i = 0;
    while i < args.len() {
        let Some(value) = args.get(i + 1) else { return usage_error(err, "missing flag value") };
        match args.get(i).map(String::as_str).unwrap_or("") {
            "--port" => {
                port = match value.parse() {
                    Ok(p) => Some(p),
                    Err(_) => return usage_error(err, "invalid port"),
                };
                i += 2;
            }
            "--token" => {
                token = Some(TokenSource::Inline(value.clone()));
                i += 2;
            }
            "--token-file" => {
                token = Some(TokenSource::File(value.into()));
                i += 2;
            }
            "--automation-read-root" => {
                read_root = Some(value.into());
                i += 2;
            }
            "--automation-write-root" => {
                write_root = Some(value.into());
                i += 2;
            }
            "--root" => {
                read_root = Some(value.into());
                write_root = Some(value.into());
                i += 2;
            }
            flag => return usage_error(err, &format!("unknown flag `{flag}`")),
        }
    }
    match openlaser_automation::run_serve(ServeConfig { port, token, read_root, write_root }) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "openlaser-cli: {e}");
            1
        }
    }
}

fn commands(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let mut json = false;
    let mut filter = None;
    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str).unwrap_or("") {
            "--json" => {
                json = true;
                i += 1;
            }
            "--filter" => {
                let Some(value) = args.get(i + 1) else { return usage_error(err, "--filter needs text") };
                filter = Some(value.to_lowercase());
                i += 2;
            }
            flag => return usage_error(err, &format!("unknown flag `{flag}`")),
        }
    }
    let list = openlaser_engine::registry()
        .specs()
        .iter()
        .filter(|row| filter.as_ref().is_none_or(|f| row.id.to_lowercase().contains(f) || row.label.to_lowercase().contains(f)))
        .map(|row| serde_json::json!({"id":row.id,"label":row.label}))
        .collect::<Vec<_>>();
    if json {
        let _ = serde_json::to_writer(&mut *out, &list);
        let _ = writeln!(out);
    } else {
        for row in list {
            let id = row.get("id").and_then(serde_json::Value::as_str).unwrap_or("");
            let label = row.get("label").and_then(serde_json::Value::as_str).unwrap_or("");
            let _ = writeln!(out, "{id}\t{label}");
        }
    }
    0
}

fn run_plan(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    if args.is_empty() {
        return usage_error(err, "plan needs a project path");
    }
    let mut machine = None;
    let mut json = false;
    let mut i = 1;
    while i < args.len() {
        match args.get(i).map(String::as_str).unwrap_or("") {
            "--machine" => {
                let Some(value) = args.get(i + 1) else { return usage_error(err, "--machine needs an id") };
                machine = Some(value.clone());
                i += 2;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            flag => return usage_error(err, &format!("unknown flag `{flag}`")),
        }
    }
    let path = args.first().map(String::as_str).unwrap_or_default();
    match project_backend(Some(path), None) {
        Ok(mut backend) => {
            if let openlaser_automation::ToolOutput::Error { message, .. } = backend.call("doc_open", serde_json::json!({"path":path}), &cli_client()) {
                let _ = writeln!(err, "{message}");
                return 1;
            }
            match backend.call("job_plan", serde_json::json!({"machine":machine}), &cli_client()) {
                openlaser_automation::ToolOutput::Json(value) if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => {
                    if json {
                        let _ = writeln!(out, "{value}");
                    }
                    0
                }
                output => {
                    let text = output_text(output);
                    if json {
                        let _ = writeln!(out, "{text}");
                    } else {
                        let _ = writeln!(err, "{text}");
                    }
                    1
                }
            }
        }
        Err(e) => {
            let _ = writeln!(err, "{e}");
            1
        }
    }
}

fn run_commands(args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    if args.is_empty() {
        return usage_error(err, "run needs a project or --new JSON");
    }
    let mut commands = Vec::new();
    let mut params = serde_json::json!({});
    let mut new_doc = None;
    let mut project = None;
    let mut output = None;
    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str).unwrap_or("") {
            "--cmd" => {
                let Some(id) = args.get(i + 1) else { return usage_error(err, "--cmd needs an id") };
                commands.push((id.clone(), serde_json::json!({})));
                i += 2;
            }
            "--params" => {
                let Some(value) = args.get(i + 1) else {
                    return usage_error(err, "--params needs JSON");
                };
                params = match serde_json::from_str(value) {
                    Ok(value) => value,
                    Err(error) => return usage_error(err, &format!("invalid --params JSON: {error}")),
                };
                if let Some(last) = commands.last_mut() {
                    last.1 = params.clone();
                }
                i += 2;
            }
            flag @ ("--out" | "--new") => {
                let Some(value) = args.get(i + 1) else {
                    return usage_error(err, "flag needs a value");
                };
                if flag == "--new" {
                    new_doc = match serde_json::from_str(value) {
                        Ok(value) => Some(value),
                        Err(error) => return usage_error(err, &format!("invalid --new JSON: {error}")),
                    };
                } else {
                    output = Some(value.clone());
                }
                i += 2;
            }
            arg if arg.starts_with('-') => return usage_error(err, &format!("unknown flag `{arg}`")),
            arg => {
                project = Some(arg.to_owned());
                i += 1;
            }
        }
    }
    if commands.iter().any(|(id, _)| matches!(id.as_str(), "laser.start" | "device.settings.set" | "laser.startJob")) {
        let _ = writeln!(err, "human-only: command is started by a person in the OpenLaser window");
        return 1;
    }
    let mut backend = match project_backend(project.as_deref(), output.as_deref()) {
        Ok(backend) => backend,
        Err(error) => {
            let _ = writeln!(err, "{error}");
            return 1;
        }
    };
    if let Some(path) = project.as_deref() {
        if let openlaser_automation::ToolOutput::Error { message, .. } = backend.call("doc_open", serde_json::json!({"path":path}), &cli_client()) {
            let _ = writeln!(err, "{message}");
            return 1;
        }
    } else if let Some(params) = new_doc {
        if let openlaser_automation::ToolOutput::Error { message, .. } = backend.call("doc_new", params, &cli_client()) {
            let _ = writeln!(err, "{message}");
            return 1;
        }
    } else {
        return usage_error(err, "run needs a project or --new JSON");
    }
    for (id, params) in commands {
        match backend.call("command_run", serde_json::json!({"id":id,"params":params}), &cli_client()) {
            openlaser_automation::ToolOutput::Json(value) => {
                let _ = writeln!(out, "{value}");
            }
            openlaser_automation::ToolOutput::Png { note, .. } => {
                let _ = writeln!(out, "{note}");
            }
            openlaser_automation::ToolOutput::Error { message, .. } => {
                let _ = writeln!(err, "{message}");
                return 1;
            }
        }
    }
    if let Some(path) = output {
        match backend.call("doc_save", serde_json::json!({"path":path}), &cli_client()) {
            openlaser_automation::ToolOutput::Json(value) => {
                let _ = writeln!(out, "{value}");
            }
            openlaser_automation::ToolOutput::Error { message, .. } => {
                let _ = writeln!(err, "{message}");
                return 1;
            }
            openlaser_automation::ToolOutput::Png { .. } => return 1,
        }
    }
    0
}

fn cli_client() -> ClientInfo {
    ClientInfo { name: "openlaser-cli".into(), version: env!("CARGO_PKG_VERSION").into() }
}

fn project_backend(input: Option<&str>, output: Option<&str>) -> Result<openlaser_automation::Headless, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let read = input.map(|path| workspace_root(path, &cwd));
    let write = output.map(|path| workspace_root(path, &cwd));
    openlaser_automation::Headless::new(read.as_deref().or(Some(cwd.as_path())), write.as_deref().or(Some(cwd.as_path()))).map_err(|error| error.to_string())
}

fn workspace_root(path: &str, cwd: &Path) -> PathBuf {
    let candidate = Path::new(path);
    let absolute = if candidate.is_absolute() { candidate.to_path_buf() } else { cwd.join(candidate) };
    absolute.parent().map_or_else(|| cwd.to_path_buf(), Path::to_path_buf)
}

fn run_project_command(command: &str, args: &[String], out: &mut impl Write, err: &mut impl Write) -> u8 {
    let Some(path) = args.first().filter(|arg| !arg.starts_with('-')).map(String::as_str) else {
        return usage_error(err, "command needs a project path");
    };
    let mut output = None;
    let mut listing = None;
    let mut machine = None;
    let mut show = None;
    let mut max_side = None;
    let mut i = 1;
    while i < args.len() {
        let flag = args.get(i).map(String::as_str).unwrap_or("");
        match flag {
            "--out" | "--listing" | "--machine" | "--show" | "--max-side" => {
                let Some(value) = args.get(i + 1) else { return usage_error(err, &format!("{flag} needs a value")) };
                match flag {
                    "--out" => output = Some(value.clone()),
                    "--listing" => listing = Some(value.clone()),
                    "--machine" => machine = Some(value.clone()),
                    "--show" => show = Some(value.split(',').map(str::to_owned).collect::<Vec<_>>()),
                    "--max-side" => {
                        max_side = match value.parse::<u32>() {
                            Ok(side) if (1..=2048).contains(&side) => Some(side),
                            _ => return usage_error(err, "--max-side must be from 1 to 2048"),
                        }
                    }
                    _ => {}
                }
                i += 2;
            }
            flag => return usage_error(err, &format!("unknown flag `{flag}`")),
        }
    }
    if matches!(command, "preview" | "simulate" | "gcode" | "job-export") && output.is_none() {
        return usage_error(err, "command needs --out <path>");
    }
    let mut backend = match project_backend(Some(path), output.as_deref()) {
        Ok(backend) => backend,
        Err(error) => {
            let _ = writeln!(err, "{error}");
            return 1;
        }
    };
    if let openlaser_automation::ToolOutput::Error { message, .. } = backend.call("doc_open", serde_json::json!({"path":path}), &cli_client()) {
        let _ = writeln!(err, "{message}");
        return 1;
    }
    let (tool, mut params) = match command {
        "info" => ("doc_inspect", serde_json::json!({"objects":"all"})),
        "preview" => ("render_preview", serde_json::json!({})),
        "simulate" => ("job_simulate", serde_json::json!({"listing":listing.is_some()})),
        "estimate" => ("job_estimate", serde_json::json!({"machine":machine})),
        "gcode" => ("gcode_export", serde_json::json!({"path":output.as_deref().unwrap_or("output.gcode"),"machine":machine})),
        "job-export" => ("job_export", serde_json::json!({"path":output.as_deref().unwrap_or("output.openlaser-job.json")})),
        "convert" => ("doc_export", serde_json::json!({"path":output.as_deref().unwrap_or("output.svg"),"format":"svg"})),
        _ => return usage_error(err, "unknown project command"),
    };
    if let Some(object) = params.as_object_mut() {
        if let Some(side) = max_side {
            object.insert("max_side".to_owned(), serde_json::json!(side));
        }
        if let Some(show) = show {
            object.insert("show".to_owned(), serde_json::json!(show));
        }
    }
    match backend.call(tool, params, &cli_client()) {
        openlaser_automation::ToolOutput::Json(value) => {
            let _ = writeln!(out, "{value}");
            0
        }
        openlaser_automation::ToolOutput::Png { png, note } => {
            let Some(path) = output.as_deref() else {
                let _ = writeln!(err, "command requires --out <path>");
                return 2;
            };
            match std::fs::write(path, png) {
                Ok(()) => {
                    if let Some(listing_path) = listing.as_deref() {
                        let lines = note.get("listing").and_then(|value| value.get("lines")).and_then(serde_json::Value::as_array);
                        let body = lines.map(|lines| lines.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                        if let Err(error) = std::fs::write(listing_path, body) {
                            let _ = writeln!(err, "{error}");
                            return 1;
                        }
                    }
                    let _ = writeln!(out, "{note}");
                    0
                }
                Err(error) => {
                    let _ = writeln!(err, "{error}");
                    1
                }
            }
        }
        openlaser_automation::ToolOutput::Error { message, .. } => {
            let _ = writeln!(err, "{message}");
            1
        }
    }
}

fn output_text(output: openlaser_automation::ToolOutput) -> String {
    match output {
        openlaser_automation::ToolOutput::Error { message, .. } => message,
        openlaser_automation::ToolOutput::Json(value) => value.to_string(),
        openlaser_automation::ToolOutput::Png { note, .. } => note.to_string(),
    }
}

fn usage_error(err: &mut impl Write, message: &str) -> u8 {
    let _ = writeln!(err, "openlaser-cli: {message}");
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke(args: &[&str]) -> (u8, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(args.iter().map(OsString::from), &mut out, &mut err);
        (code, String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned())
    }

    #[test]
    fn cli_unknown_flag_exit_2() {
        assert_eq!(invoke(&["commands", "--mystery"]).0, 2);
    }

    #[test]
    fn cli_hw_subcommands_redirect() {
        let (code, _, err) = invoke(&["frame"]);
        assert_eq!(code, 2);
        assert!(err.contains("openlaser-hw"));
    }

    #[test]
    fn cli_run_is_headless_origin_and_refuses_human_only_commands() {
        for command in ["laser.start", "device.settings.set"] {
            let (code, _, err) = invoke(&["run", "--new", "{}", "--cmd", command]);
            assert_eq!(code, 1);
            assert!(err.starts_with("human-only:"));
        }
    }

    #[test]
    fn cli_run_creates_a_headless_document() {
        let (code, out, err) = invoke(&["run", "--new", r#"{"name":"CLI project"}"#]);
        assert_eq!(code, 0, "{err}");
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn cli_plan_exit_1_on_refusal() {
        let (code, _, err) = invoke(&["plan", "empty.olp"]);
        assert_eq!(code, 1);
        assert!(!err.is_empty());
    }

    #[test]
    #[ignore = "needs I1"]
    fn cli_simulate_writes_png() {
        let temp = tempfile::tempdir().unwrap();
        let png = temp.path().join("simulation.png");
        let inputs = openlaser_testkit::golden_dir().join("inputs");
        let workspace = temp.path().join("workspace.json");
        assert!(std::fs::copy(inputs.join("workspace.json"), &workspace).is_ok());
        assert!(std::fs::copy(inputs.join("layer_settings.json"), temp.path().join("layer_settings.json")).is_ok());
        let workspace_path = workspace.to_string_lossy().into_owned();
        let png_path = png.to_string_lossy().into_owned();
        let (code, _, _) = invoke(&["simulate", &workspace_path, "--out", &png_path]);
        assert_eq!(code, 0);
        assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}

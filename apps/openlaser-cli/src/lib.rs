//! In-process CLI entry point so command behavior can be tested without spawning a process.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::Write;

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
        "plan" => run_plan(rest, err),
        "run" => run_commands(rest, err),
        "info" | "convert" | "preview" | "simulate" | "estimate" | "gcode" | "job-export" | "ui" => {
            let _ = writeln!(err, "openlaser-cli: `{command}` requires the completed engine pipeline");
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

fn run_plan(args: &[String], err: &mut impl Write) -> u8 {
    if args.is_empty() {
        return usage_error(err, "plan needs a project path");
    }
    if let Some(flag) = args.iter().find(|a| a.starts_with('-')) {
        return usage_error(err, &format!("unknown flag `{flag}`"));
    }
    match openlaser_automation::Headless::new(None, None) {
        Ok(mut backend) => {
            match backend.call("job_plan", serde_json::json!({}), &ClientInfo { name: "openlaser-cli".into(), version: env!("CARGO_PKG_VERSION").into() }) {
                openlaser_automation::ToolOutput::Json(value) if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => 0,
                output => {
                    let _ = writeln!(err, "{}", output_text(output));
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

fn run_commands(args: &[String], err: &mut impl Write) -> u8 {
    if args.is_empty() {
        return usage_error(err, "run needs a project or --new JSON");
    }
    let mut commands = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str).unwrap_or("") {
            "--cmd" => {
                let Some(id) = args.get(i + 1) else { return usage_error(err, "--cmd needs an id") };
                commands.push(id.clone());
                i += 2;
            }
            "--params" => {
                if args.get(i + 1).is_none() {
                    return usage_error(err, "--params needs JSON");
                }
                i += 2;
            }
            "--out" | "--new" => {
                if args.get(i + 1).is_none() {
                    return usage_error(err, "flag needs a value");
                }
                i += 2;
            }
            arg if arg.starts_with('-') => return usage_error(err, &format!("unknown flag `{arg}`")),
            _ => i += 1,
        }
    }
    if commands.iter().any(|id| matches!(id.as_str(), "laser.start" | "device.settings.set" | "laser.startJob")) {
        let _ = writeln!(err, "human-only: command is started by a person in the OpenLaser window");
        return 1;
    }
    let _ = writeln!(err, "not implemented: headless command pipeline is unavailable in this engine build");
    1
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
}

//! OpenLaser's job commands (Appendix A): plan, validate, summary, dump, preview statistics, and the agent requests
//! (bridge only). `job.submit` and `job.frameRequest` create requests a human must act on; they never reach a driver.

use openlaser_types::MachineId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{needs_doc, needs_doc_and_window, needs_window};
use crate::command;
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::jobs::{Encoded, JobOptions};
use crate::params::{FrameModeP, NoParams, PathParams};
use crate::requests::RequestId;
use crate::session::Session;

/// The client name a submission made through the command path carries (the GUI's control dispatcher calls
/// `Session::submit_job` with the connected client's own name).
pub const COMMAND_CLIENT: &str = "agent";

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("job.plan", "Plan Job", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: JobParams, run: plan, doc: "{selectedOnly?, machine?}"),
        command!("job.validate", "Validate Job", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: JobParams, run: validate, doc: "{selectedOnly?, machine?}"),
        command!("job.summary", "Emission Summary", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: JobParams, run: summary, doc: "{selectedOnly?, machine?}"),
        command!("job.dump", "Export Job", menu: [], shortcut: None, kind: Kind::Io, access: Access::Roots,
            enabled: needs_doc, params: PathParams, run: dump, doc: "{path} (packets hex + listing + summary JSON)"),
        command!("preview.stats", "Preview Statistics", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: NoParams, run: stats, doc: "{}"),
        command!("job.submit", "Submit Job", menu: [], shortcut: None, kind: Kind::Request, access: Access::Agent,
            enabled: needs_doc_and_window, params: SubmitParams, run: submit, doc: "{note?} (bridge only; a person frames and arms it)"),
        command!("job.status", "Job Status", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_window, params: StatusParams, run: status, doc: "{submission?}"),
        command!("job.withdraw", "Withdraw Job", menu: [], shortcut: None, kind: Kind::Request, access: Access::Agent,
            enabled: needs_window, params: WithdrawParams, run: withdraw, doc: "{submission}"),
        command!("job.frameRequest", "Request Framing", menu: [], shortcut: None, kind: Kind::Request, access: Access::Agent,
            enabled: needs_doc_and_window, params: FrameRequestParams, run: frame_request, doc: "{mode?:\"once\"|\"loop\"} (bridge only; a person approves it)"),
    ]
}

/// `job.plan`, `job.validate`, `job.summary`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobParams {
    /// Only the selected objects. Default: the project's Cut Selected Graphics setting.
    #[serde(default)]
    pub selected_only: Option<bool>,
    /// Plan for this machine profile. Default: the document's machine.
    #[serde(default)]
    pub machine: Option<String>,
}

fn opts(p: JobParams) -> JobOptions {
    JobOptions { machine: p.machine.map(MachineId), selected_only: p.selected_only }
}

fn plan(s: &mut Session, p: JobParams) -> Result<Value, EngineError> {
    Ok(s.job_plan_json(None, &opts(p)))
}

fn validate(s: &mut Session, p: JobParams) -> Result<Value, EngineError> {
    Ok(match s.build_job(None, &opts(p)) {
        Ok(b) => json!({ "ok": true, "fingerprint": b.summary.fingerprint.hex(), "refusals": [], "notices": b.validated.notices() }),
        Err(r) => json!({ "ok": false, "fingerprint": r.fingerprint.map(|f| f.hex()), "refusals": r.refusals, "notices": r.notices, "error": r.error }),
    })
}

fn summary(s: &mut Session, p: JobParams) -> Result<Value, EngineError> {
    Ok(match s.build_job(None, &opts(p)) {
        Ok(b) => json!({ "ok": true, "summary": b.summary }),
        Err(r) => {
            json!({ "ok": false, "summary": null, "fingerprint": r.fingerprint.map(|f| f.hex()), "refusals": r.refusals, "notices": r.notices, "error": r.error })
        }
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        for nibble in [b >> 4, b & 0x0f] {
            if let Some(d) = DIGITS.get(usize::from(nibble)) {
                s.push(char::from(*d));
            }
        }
    }
    s
}

fn dump(s: &mut Session, p: PathParams) -> Result<Value, EngineError> {
    let built = s.build_job(None, &JobOptions::default()).map_err(|r| EngineError::JobRefused(Box::new(r)))?;
    let body = match &built.encoded {
        Encoded::Lmc { job, .. } => json!({
            "kind": "galvo_lmc",
            "bytesHash": job.bytes_hash().hex(),
            "repeats": job.repeats,
            "lists": job.lists.iter().map(|l| json!({
                "layer": l.layer.to_string(),
                "pass": l.pass,
                "packets": l.packets.iter().map(|pk| hex(pk.bytes())).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
        Encoded::Gcode { job, .. } => json!({ "kind": "grbl", "bytesHash": job.bytes_hash().hex(), "gcode": job.text }),
    };
    let doc = json!({
        "format": "openlaser-job",
        "formatVersion": 1,
        "generator": crate::jobs::GENERATOR,
        "fingerprint": built.summary.fingerprint.hex(),
        "summary": built.summary,
        "listing": built.listing.lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
        "job": body,
    });
    let mut text = serde_json::to_string_pretty(&doc).map_err(|_| EngineError::Internal("job.dump".to_owned()))?;
    text.push('\n');
    s.write_file(&p.path, text.as_bytes())?;
    Ok(json!({ "path": p.path, "fingerprint": built.summary.fingerprint.hex() }))
}

fn stats(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    Ok(match s.build_job(None, &JobOptions::default()) {
        Ok(b) => {
            let rows: Vec<Value> = b
                .summary
                .rows
                .iter()
                .map(|r| json!({ "layer": r.layer, "layerName": r.layer_name, "pass": r.pass, "of": r.of, "markMm": r.mark_mm, "paths": r.paths, "estimateS": r.estimate.as_secs_f64() }))
                .collect();
            json!({
                "ok": true,
                "fingerprint": b.summary.fingerprint.hex(),
                "estimateS": b.summary.estimate.as_secs_f64(),
                "crashBoundS": b.summary.crash_bound.as_secs_f64(),
                "markMm": b.summary.rows.iter().map(|r| r.mark_mm).sum::<f64>(),
                "paths": b.summary.rows.iter().map(|r| u64::from(r.paths)).sum::<u64>(),
                "packets": b.summary.packets,
                "repeats": b.summary.repeats,
                "rows": rows,
            })
        }
        Err(r) => json!({ "ok": false, "refusals": r.refusals, "notices": r.notices, "error": r.error }),
    })
}

/// `job.submit`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubmitParams {
    /// A note for the person (plain text, ≤ 280 characters), shown outside the confirmation.
    #[serde(default)]
    pub note: Option<String>,
}

fn submit(s: &mut Session, p: SubmitParams) -> Result<Value, EngineError> {
    let v = s.submit_job(COMMAND_CLIENT, p.note.as_deref().unwrap_or(""))?;
    serde_json::to_value(v).map_err(|_| EngineError::Internal("job.submit".to_owned()))
}

/// `job.status`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StatusParams {
    /// Submission id. Default: every submission and frame request.
    #[serde(default)]
    pub submission: Option<u64>,
}

fn status(s: &mut Session, p: StatusParams) -> Result<Value, EngineError> {
    match p.submission {
        Some(id) => {
            let r = s.requests();
            if let Some(v) = r.submission(RequestId(id)) {
                return serde_json::to_value(v).map_err(|_| EngineError::Internal("job.status".to_owned()));
            }
            if let Some(v) = r.frame_request(RequestId(id)) {
                return serde_json::to_value(v).map_err(|_| EngineError::Internal("job.status".to_owned()));
            }
            Err(EngineError::bad("job.status", "/submission", format!("no request {id}")))
        }
        None => Ok(json!({ "submissions": s.requests().submissions(), "frameRequests": s.requests().frame_requests() })),
    }
}

/// `job.withdraw`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WithdrawParams {
    /// Submission (or frame request) id.
    pub submission: u64,
}

fn withdraw(s: &mut Session, p: WithdrawParams) -> Result<Value, EngineError> {
    s.withdraw_request(RequestId(p.submission))?;
    let state = s
        .requests()
        .submission(RequestId(p.submission))
        .map(|v| v.state.as_str())
        .or_else(|| s.requests().frame_request(RequestId(p.submission)).map(|v| v.state.as_str()));
    Ok(json!({ "submission": p.submission, "state": state }))
}

/// `job.frameRequest`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FrameRequestParams {
    /// "once" (default) or "loop".
    #[serde(default)]
    pub mode: Option<FrameModeP>,
}

fn frame_request(s: &mut Session, p: FrameRequestParams) -> Result<Value, EngineError> {
    let v = s.request_frame(COMMAND_CLIENT, p.mode.unwrap_or(FrameModeP::Once).into())?;
    serde_json::to_value(v).map_err(|_| EngineError::Internal("job.frameRequest".to_owned()))
}

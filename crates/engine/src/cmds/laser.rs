//! Laser panel and Live Framing (Appendix A). `laser.frame` and `laser.start` are human-only (S18: no menu, no
//! shortcut); `laser.start` never emits, it opens the ARM dialog. `laser.stop` / `laser.frameStop` are never
//! disabled or refused: they trip the latches before they look at their params (a machine that is not connected or
//! params that do not decode stop every machine, and only then is the problem reported); without device access they
//! fail with `NoDevice` after tripping whatever is registered.

use openlaser_types::{FrameMode, MachineId};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{always, needs_doc};
use crate::command::{Access, CommandSpec, DeviceClass, EngineError, Kind};
use crate::params::{FrameSourceP, FrameStyleP, MachineParams, NoParams};
use crate::session::{Label, Session};
use crate::{StopAck, command};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("laser.status", "Machine Status", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: MachineParams, run: status, doc: "{machine?} (cached; never device I/O)"),
        command!("laser.liveFraming", "Live Framing", menu: [], shortcut: Some("F1"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "laser.liveFraming" })), doc: "{}"),
        command!("laser.frame", "Frame", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Light), access: Access::Human,
            enabled: needs_doc, params: FrameParams, run: frame, doc: "{style?:\"bounds\"|\"contour\", source?:\"job\"|\"toolLayersOnly\"|\"jobAndToolLayers\", loop?}"),
        stop_spec("laser.frameStop", "Stop Framing", |s, v| stop_command("laser.frameStop", s, v)),
        command!("laser.start", "Start", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Emit), access: Access::Human,
            enabled: needs_doc, params: NoParams, run: start, doc: "{} (opens the ARM dialog; never emits by itself)"),
        stop_spec("laser.stop", "Stop", |s, v| stop_command("laser.stop", s, v)),
        command!("laser.clearFault", "Clear Fault", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Connect), access: Access::Human,
            enabled: always, params: MachineParams, run: clear_fault, doc: "{machine?}"),
        command!("laser.cutSelected", "Cut Selected Graphics", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: OnOff, run: cut_selected, doc: "{on}",
            checked: |s: &Session| s.active().is_some_and(|d| d.project.job.cut_selected)),
        command!("laser.repeat", "Repeat", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: RepeatParams, run: repeat, doc: "{count} (1..=999)"),
    ]
}

fn status(s: &mut Session, p: MachineParams) -> Result<Value, EngineError> {
    let m = p.machine.map(MachineId);
    Ok(s.machine_status_json(m.as_ref()))
}

/// `laser.frame`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FrameParams {
    /// Outline style. Default: the project's setting. Never changes the fingerprint (S21).
    #[serde(default)]
    pub style: Option<FrameStyleP>,
    /// What to trace. Default: the project's setting. A tool-layers-only frame never counts toward Start.
    #[serde(default)]
    pub source: Option<FrameSourceP>,
    /// Loop until stopped (Live Framing). Default false: frame once.
    #[serde(default, rename = "loop")]
    pub looping: Option<bool>,
}

fn frame(s: &mut Session, p: FrameParams) -> Result<Value, EngineError> {
    let origin = s.current_origin();
    let mode = if p.looping.unwrap_or(false) { FrameMode::Loop } else { FrameMode::Once };
    let fp = s.start_frame(origin, mode, p.style.map(Into::into), p.source.map(Into::into))?;
    Ok(json!({ "fingerprint": fp.hex(), "mode": mode, "framing": true }))
}

fn start(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    let origin = s.current_origin();
    let pending = s.request_arm(origin)?;
    Ok(json!({
        "armRequired": true,
        "fingerprint": pending.fingerprint.hex(),
        "machine": pending.machine.0,
        "summary": pending.summary,
        "checklist": pending.checklist,
    }))
}

fn ack_json(machine: &MachineId, a: &StopAck) -> Value {
    json!({
        "machine": machine.0,
        "latched": a.latched,
        "confirmed": a.report.as_ref().is_some_and(|r| r.confirmed),
        "report": a.report,
        "elapsedMs": a.elapsed_ms,
    })
}

/// A Stop command's spec. Not built by `command!`: its `run` takes the raw params (dispatch hands them over without
/// the strict decode, §1.3 rule 5), so nothing about them can keep it from tripping.
fn stop_spec(id: &'static str, label: &'static str, run: crate::command::RunFn) -> CommandSpec {
    CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: Some("Escape"),
        kind: Kind::Device(DeviceClass::Stop),
        access: Access::Stop,
        doc: "{machine?}",
        schema: crate::params_schema::<MachineParams>,
        enabled: always,
        run,
        checked: None,
    }
}

/// `{machine?}`, strictly (the universal keys checked as everywhere else; a stop is never a dry run).
fn stop_params(id: &str, raw: Value) -> Result<Option<MachineId>, EngineError> {
    let (params, _coalesce, dry_run) = crate::session::split_universal(id, raw)?;
    if dry_run {
        return Err(EngineError::bad(id, "/dryRun", "a stop is never a dry run"));
    }
    let p: MachineParams = crate::params::decode(id, params)?;
    Ok(p.machine.map(MachineId))
}

/// Trip first, then look at the params. The named machine is stopped alone only when it is connected; an unknown or
/// unconnected machine, or params that do not decode, stop every machine. `latched` / `confirmed` are true only when
/// something was stopped and every latch / report says so.
fn stop_command(id: &'static str, s: &mut Session, raw: Value) -> Result<Value, EngineError> {
    let parsed = stop_params(id, raw);
    let wanted = parsed.as_ref().ok().and_then(Option::as_ref);
    let target = wanted.filter(|m| s.stop_registry().machines().contains(m)).cloned();
    let acks: Vec<(MachineId, StopAck)> = match &target {
        Some(m) => match s.stop_machine(m) {
            Some(a) => vec![(m.clone(), a)],
            None => s.stop_all(),
        },
        None => s.stop_all(),
    };
    s.provider()?;
    let requested = match parsed {
        Ok(m) => m,
        Err(EngineError::BadParams { id, pointer, message }) => {
            return Err(EngineError::BadParams { id, pointer, message: format!("{message} (every machine was stopped anyway)") });
        }
        Err(e) => return Err(e),
    };
    let any = !acks.is_empty();
    let latched = any && acks.iter().all(|(_, a)| a.latched);
    let confirmed = any && acks.iter().all(|(_, a)| a.report.as_ref().is_some_and(|r| r.confirmed));
    let elapsed = acks.iter().map(|(_, a)| a.elapsed_ms).max().unwrap_or(0);
    let note = match (&requested, &target, any) {
        (Some(m), None, true) => Some(format!("{} is not connected; every connected machine was stopped", m.0)),
        (Some(m), None, false) => Some(format!("{} is not connected, and no machine is connected", m.0)),
        (None, _, false) => Some("no machine is connected".to_owned()),
        _ => None,
    };
    Ok(json!({
        "latched": latched,
        "confirmed": confirmed,
        "elapsedMs": elapsed,
        "stopped": acks.iter().map(|(m, a)| ack_json(m, a)).collect::<Vec<_>>(),
        "requested": requested.map(|m| m.0),
        "note": note,
    }))
}

fn clear_fault(s: &mut Session, p: MachineParams) -> Result<Value, EngineError> {
    let origin = s.current_origin();
    let m = p.machine.map(MachineId).or_else(|| s.current_machine()).ok_or(EngineError::NoDevice)?;
    s.acknowledge_fault(origin, &m)?;
    Ok(json!({ "machine": m.0, "fault": null }))
}

/// `{on}`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OnOff {
    /// On or off.
    pub on: bool,
}

fn cut_selected(s: &mut Session, p: OnOff) -> Result<Value, EngineError> {
    s.edit(Label::new("Cut Selected Graphics"), None, |project, _| {
        project.job.cut_selected = p.on;
        Ok(())
    })?;
    Ok(json!({ "cutSelected": p.on }))
}

/// `laser.repeat`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RepeatParams {
    /// How many times the whole job runs (1..=999); shown at confirmation.
    pub count: u16,
}

fn repeat(s: &mut Session, p: RepeatParams) -> Result<Value, EngineError> {
    if !(1..=999).contains(&p.count) {
        return Err(EngineError::bad("laser.repeat", "/count", format!("{} is outside 1..=999", p.count)));
    }
    s.edit(Label::with("Repeat {name}", p.count.to_string()), None, |project, _| {
        project.job.repeats = p.count;
        Ok(())
    })?;
    Ok(json!({ "repeats": p.count }))
}

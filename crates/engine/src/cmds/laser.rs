//! Laser panel and Live Framing (Appendix A). `laser.frame` and `laser.start` are human-only (S18: no menu, no
//! shortcut); `laser.start` never emits, it opens the ARM dialog. `laser.stop` / `laser.frameStop` are never
//! disabled or refused; without device access they fail with `NoDevice`.

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
        command!("laser.frameStop", "Stop Framing", menu: [], shortcut: Some("Escape"), kind: Kind::Device(DeviceClass::Stop), access: Access::Stop,
            enabled: always, params: MachineParams, run: stop, doc: "{machine?}"),
        command!("laser.start", "Start", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Emit), access: Access::Human,
            enabled: needs_doc, params: NoParams, run: start, doc: "{} (opens the ARM dialog; never emits by itself)"),
        command!("laser.stop", "Stop", menu: [], shortcut: Some("Escape"), kind: Kind::Device(DeviceClass::Stop), access: Access::Stop,
            enabled: always, params: MachineParams, run: stop, doc: "{machine?}"),
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

fn stop(s: &mut Session, p: MachineParams) -> Result<Value, EngineError> {
    s.provider()?;
    let acks: Vec<(MachineId, StopAck)> = match p.machine.map(MachineId) {
        Some(m) => s.stop_machine(&m).map(|a| vec![(m, a)]).unwrap_or_default(),
        None => s.stop_all(),
    };
    let latched = acks.iter().all(|(_, a)| a.latched);
    let confirmed = acks.iter().all(|(_, a)| a.report.as_ref().is_some_and(|r| r.confirmed));
    let elapsed = acks.iter().map(|(_, a)| a.elapsed_ms).max().unwrap_or(0);
    Ok(json!({
        "latched": latched,
        "confirmed": confirmed,
        "elapsedMs": elapsed,
        "stopped": acks.iter().map(|(m, a)| ack_json(m, a)).collect::<Vec<_>>(),
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

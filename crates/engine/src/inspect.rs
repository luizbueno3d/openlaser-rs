//! JSON views (SPEC §4.3): `doc_inspect`, layers, `machine_list`, `machine_status`, `session_list`, `job_plan`.
//! Read-only; `machine_status` reads the driver's cached events and never causes device I/O.

use openlaser_doc::{ColorLayer, CutMode, CutOp, ImageMode, Project};
use openlaser_driver::MachineState;
use openlaser_machine::{GalvoLink, GantryLink, Kinematics, MachineProfile};
use openlaser_types::{Bounds, LayerKey, MachineId};
use serde_json::{Value, json};

use crate::command::EngineError;
use crate::jobs::JobOptions;
use crate::session::Session;

/// `doc_inspect` lists at most this many objects unless `objects:"all"`.
pub const INSPECT_OBJECTS: usize = 200;

pub(crate) fn mode_str(m: CutMode) -> &'static str {
    match m {
        CutMode::Line => "line",
        CutMode::Fill => "fill",
        CutMode::OffsetFill => "offsetFill",
    }
}

fn image_mode_str(m: ImageMode) -> &'static str {
    match m {
        ImageMode::Threshold => "threshold",
        ImageMode::Jarvis => "jarvis",
    }
}

fn bounds_json(b: &Bounds) -> Value {
    json!({ "min": b.min, "max": b.max })
}

/// Cuts / Layers order: `layers.order`, then any used layer the order misses (in draw order).
pub(crate) fn layer_order(project: &Project) -> Vec<LayerKey> {
    let mut keys = project.layers.order.clone();
    for k in project.used_layers().into_iter().chain(project.shapes.iter().map(|s| s.layer)) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys
}

/// The actual galvo speed for a requested one (display only; the planner quantises the same way).
fn actual_speed(speed: f64, m: Option<&MachineProfile>) -> Option<f64> {
    let Some(Kinematics::Galvo(g)) = m.map(|m| &m.kind) else { return None };
    let f = g.field_f()[0];
    let units = openlaser_types::galvo::speed_to_units(speed, f).ok()?;
    (units > 0).then(|| openlaser_types::galvo::units_to_speed(units, f))
}

fn op_json(op: &CutOp, m: Option<&MachineProfile>) -> Value {
    json!({
        "name": op.name,
        "enabled": op.enabled,
        "mode": mode_str(op.mode),
        "passes": op.passes,
        "galvo": {
            "speedMmS": op.galvo.speed.get(),
            "actualSpeedMmS": actual_speed(op.galvo.speed.get(), m),
            "powerPct": op.galvo.power.get(),
            "minPowerPct": op.galvo.min_power.get(),
            "frequencyKhz": op.galvo.frequency.khz(),
            "qPulseNs": op.galvo.q_pulse.map(|n| n.get()),
            "timings": op.galvo.timings.is_some(),
            "wobble": op.galvo.wobble.is_some(),
        },
        "gantry": {
            "speedMmS": op.gantry.speed.get(),
            "powerMaxPct": op.gantry.power_max.get(),
            "powerMinPct": op.gantry.power_min.get(),
            "constantPower": op.gantry.constant_power,
        },
        "fill": {
            "intervalMm": op.fill.interval.get(),
            "angleDeg": op.fill.angle.get(),
            "angleIncrementDeg": op.fill.angle_increment.get(),
            "bidirectional": op.fill.bidirectional,
            "crossHatch": op.fill.cross_hatch,
        },
    })
}

/// One layer: the `doc_inspect` summary (`full` = false) or `layer.cutSettings.get` (every sub-layer, image).
pub(crate) fn layer_json(project: &Project, key: LayerKey, m: Option<&MachineProfile>, full: bool) -> Value {
    match key {
        LayerKey::Tool(which) => {
            let t = project.layers.tools.iter().find(|t| t.which == which);
            json!({ "key": key.to_string(), "name": t.map(|t| t.name.clone()), "tool": true, "show": t.map(|t| t.show), "output": false })
        }
        LayerKey::Color(i) => match project.color_layer(i) {
            None => json!({ "key": key.to_string(), "missing": true }),
            Some(l) => color_layer_json(key, l, m, full),
        },
    }
}

fn color_layer_json(key: LayerKey, l: &ColorLayer, m: Option<&MachineProfile>, full: bool) -> Value {
    let first = l.ops.first();
    let mut v = json!({
        "key": key.to_string(),
        "name": l.name,
        "mode": first.map(|o| mode_str(o.mode)),
        "output": l.output,
        "show": l.show,
        "passes": first.map(|o| o.passes),
        "globalPasses": l.global_passes,
        "subLayers": l.ops.len(),
    });
    if let (Some(obj), Some(op)) = (v.as_object_mut(), first) {
        let o = op_json(op, m);
        for k in ["galvo", "gantry", "fill"] {
            if let Some(x) = o.get(k) {
                obj.insert(k.to_owned(), x.clone());
            }
        }
        if full {
            obj.insert("ops".to_owned(), Value::Array(l.ops.iter().map(|op| op_json(op, m)).collect()));
            obj.insert("airAssist".to_owned(), Value::Bool(l.air_assist));
            obj.insert(
                "image".to_owned(),
                json!({
                    "mode": image_mode_str(l.image.mode),
                    "intervalMm": l.image.interval.get(),
                    "angleDeg": l.image.angle.get(),
                    "bidirectional": l.image.bidirectional,
                    "negative": l.image.negative,
                    "threshold": l.image.threshold,
                    "passes": l.image.passes,
                    "galvo": { "speedMmS": l.image.galvo.speed.get(), "powerPct": l.image.galvo.power.get(), "frequencyKhz": l.image.galvo.frequency.khz(), "qPulseNs": l.image.galvo.q_pulse.map(|n| n.get()) },
                    "gantry": { "speedMmS": l.image.gantry.speed.get(), "powerMaxPct": l.image.gantry.power_max.get(), "powerMinPct": l.image.gantry.power_min.get() },
                }),
            );
        }
    }
    v
}

fn kind_str(m: &MachineProfile) -> &'static str {
    match m.kind {
        Kinematics::Galvo(_) => "galvo_lmc",
        Kinematics::Gantry(_) => "grbl",
    }
}

fn work_area(m: &MachineProfile) -> Value {
    match &m.kind {
        Kinematics::Galvo(g) => {
            let w = g.effective_work_area();
            json!({ "min": w.min, "max": w.max })
        }
        Kinematics::Gantry(g) => json!({ "min": [0.0, 0.0], "max": g.bed_mm }),
    }
}

fn link_str(m: &MachineProfile) -> &'static str {
    match &m.kind {
        Kinematics::Galvo(g) => match g.link {
            GalvoLink::Usb => "usb",
            GalvoLink::Sim => "sim",
        },
        Kinematics::Gantry(g) => match g.link {
            GantryLink::ExportOnly => "export_only",
            GantryLink::Serial { .. } => "serial",
            GantryLink::Tcp { .. } => "tcp",
            GantryLink::Sim => "sim",
        },
    }
}

pub(crate) fn state_str(s: &MachineState) -> &'static str {
    match s {
        MachineState::Disconnected => "disconnected",
        MachineState::Connecting => "connecting",
        MachineState::Idle => "idle",
        MachineState::Framing => "framing",
        MachineState::Running { .. } => "running",
        MachineState::Stopping => "stopping",
        MachineState::Stopped => "stopped",
        MachineState::Fault => "fault",
        MachineState::StopNotConfirmed => "stop_not_confirmed",
    }
}

impl Session {
    /// doc_inspect JSON (§4.3).
    pub fn inspect(&self, index: Option<usize>, all_objects: bool) -> Result<Value, EngineError> {
        let idx = index.or(self.active).ok_or(EngineError::NoDocument)?;
        let doc = self.docs.get(idx).ok_or_else(|| EngineError::bad("document.inspect", "/index", format!("no document {idx} ({} open)", self.docs.len())))?;
        let p = &doc.project;
        let m = self.profile(&doc.machine);
        let tol = self.prefs.curve_tolerance_mm();
        let count = p.shapes.len();
        let shown = if all_objects { count } else { count.min(INSPECT_OBJECTS) };
        let items: Vec<Value> = p
            .shapes
            .iter()
            .take(shown)
            .map(|s| -> Result<Value, EngineError> {
                let (paths, closed) = crate::geomx::path_stats(p, s.id, tol)?;
                Ok(json!({
                    "id": s.id.0,
                    "name": s.name,
                    "layer": s.layer.to_string(),
                    "bounds": crate::geomx::bounds(p, &[s.id], tol)?.map(|b| bounds_json(&b)),
                    "paths": paths,
                    "closed": closed,
                    "locked": s.locked,
                }))
            })
            .collect::<Result<_, _>>()?;
        Ok(json!({
            "index": idx,
            "name": doc.display_name(),
            "path": doc.path,
            "dirty": doc.dirty(),
            "revision": doc.revision,
            "units": "mm",
            "axes": "y-up; galvo origin at field centre",
            "machine": m.map(|m| json!({ "id": m.id.0, "kind": kind_str(m), "workArea": work_area(m) })),
            "layers": layer_order(p).into_iter().map(|k| layer_json(p, k, m, false)).collect::<Vec<_>>(),
            "currentLayer": doc.current_layer.to_string(),
            "job": { "repeats": p.job.repeats, "cutSelected": p.job.cut_selected, "frameStyle": p.job.frame_style, "frameSource": p.job.frame_source },
            "objects": { "count": count, "shown": shown, "truncated": count - shown, "items": items },
            "selection": doc.selection.ids.iter().map(|i| i.0).collect::<Vec<_>>(),
            "history": doc.history.labels(),
            "canUndo": doc.history.can_undo(),
            "canRedo": doc.history.can_redo(),
            "live": self.live_json(idx),
        }))
    }

    /// `session_list`: `{documents:[{index,name,path,dirty,revision,machine}], active}`.
    pub fn session_list_json(&self) -> Value {
        let docs: Vec<Value> = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| json!({ "index": i, "name": d.display_name(), "path": d.path, "dirty": d.dirty(), "revision": d.revision, "machine": d.machine.0 }))
            .collect();
        json!({ "documents": docs, "active": self.active })
    }

    /// `machine_list`: every profile with its work area, source limits, link and connection.
    pub fn machine_list_json(&self) -> Value {
        let rows: Vec<Value> = self
            .cfg
            .machines
            .iter()
            .map(|m| {
                let source = match &m.kind {
                    Kinematics::Galvo(g) => json!({ "maxPowerPct": g.source.max_power_pct, "freqKhz": g.source.freq_khz, "pulsesNs": g.source.pulse_table_ns }),
                    Kinematics::Gantry(g) => json!({ "sMax": g.s_max, "maxSpeedMmS": g.max_speed.get() }),
                };
                json!({
                    "id": m.id.0,
                    "name": m.name,
                    "kind": kind_str(m),
                    "workArea": work_area(m),
                    "source": source,
                    "link": link_str(m),
                    "connected": self.slot(&m.id).is_some_and(|s| s.status.connected),
                })
            })
            .collect();
        Value::Array(rows)
    }

    /// `machine_status` (§4.3): cached; never causes device I/O. Headless: `{connected:false, backend:"headless"}`.
    pub fn machine_status_json(&self, machine: Option<&MachineId>) -> Value {
        if matches!(self.cfg.devices, crate::devices::DeviceAccess::None) {
            return json!({ "machine": machine.map(|m| m.0.clone()).or_else(|| self.current_machine().map(|m| m.0)), "connected": false, "backend": "headless" });
        }
        let Some(m) = machine.cloned().or_else(|| self.current_machine()) else {
            return json!({ "machine": null, "connected": false });
        };
        let now = self.cfg.clock.now();
        let pending = json!({
            "submission": self.requests.pending_submission(&m).map(|r| r.0.to_string()),
            "frameRequest": self.requests.pending_frame_request(&m).map(|r| r.0.to_string()),
        });
        let Some(slot) = self.slot(&m) else {
            return json!({ "machine": m.0, "connected": false, "state": "disconnected", "pending": pending });
        };
        let st = &slot.status;
        let bits = st.status_bits;
        json!({
            "machine": m.0,
            "connected": st.connected,
            "provider": st.provider,
            "state": state_str(&st.state),
            "statusBits": bits.map(|b| format!("0x{b:04x}")),
            "ready": bits.map(|b| b & 0x20 != 0),
            "busy": bits.map(|b| b & 0x04 != 0),
            "fault": st.fault.as_ref().map(|f| f.to_string()),
            "progress": st.progress.map(|(a, t)| json!({ "packetsAcked": a, "total": t })),
            "lastStop": st.last_stop.as_ref().map(|r| json!({ "elapsedMs": u64::try_from(r.elapsed.as_millis()).unwrap_or(u64::MAX), "confirmed": r.confirmed, "possibleDesync": r.possible_desync })),
            "frameReceipt": st.receipt.as_ref().map(|r| json!({
                "fingerprint": r.fingerprint.hex(),
                "fullPasses": r.full_passes,
                "ageS": now.saturating_since(r.at).as_secs(),
                "origin": r.origin,
                "style": r.style,
                "source": r.source,
            })),
            "pending": pending,
        })
    }

    /// `job_plan` (§4.3): the same pipeline the ARM dialog runs. A refusing plan is a successful call with `ok:false`.
    pub fn job_plan_json(&self, index: Option<usize>, opts: &JobOptions) -> Value {
        let machine = opts.machine.clone().or_else(|| index.or(self.active).and_then(|i| self.docs.get(i)).map(|d| d.machine.clone()));
        match self.build_job(index, opts) {
            Ok(b) => {
                let frame = b.validated.frame();
                json!({
                    "fingerprint": b.summary.fingerprint.hex(),
                    "short": b.summary.fingerprint.short(),
                    "machine": b.summary.machine.0,
                    "ok": true,
                    "refusals": [],
                    "notices": b.validated.notices(),
                    "summary": b.summary,
                    "frame": {
                        "style": frame.style(),
                        "source": frame.source(),
                        "bounds": openlaser_geom::bounds_of(frame.paths()).map(|b| bounds_json(&b)),
                        "paths": frame.paths().len(),
                    },
                })
            }
            Err(r) => json!({
                "fingerprint": r.fingerprint.map(|f| f.hex()),
                "short": r.fingerprint.map(|f| f.short()),
                "machine": machine.map(|m| m.0),
                "ok": false,
                "refusals": r.refusals,
                "notices": r.notices,
                "summary": null,
                "error": r.error,
            }),
        }
    }
}

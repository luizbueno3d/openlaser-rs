//! Devices and Device Settings (Appendix A, CAT §7). Profile writes are human-only (S9) and go only through the
//! injected `ProfileStore` (S24): with `profiles: None` they fail even for a human. Reads are open to agents.

use openlaser_machine::{GalvoMachine, Kinematics, MachineProfile, validate_profile};
use openlaser_types::{MachineId, MmPerS, RefusalCode, Us};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{always, human_here};
use crate::command;
use crate::command::{Access, CommandSpec, DeviceClass, EngineError, Kind};
use crate::params::{DeviceParams, NoParams, finite, name, positive};
use crate::session::Session;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("device.list", "Devices", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: list, doc: "{}"),
        command!("device.select", "Select Device", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: super::needs_doc, params: SelectDevice, run: select, doc: "{device} (refused while a machine is busy)"),
        command!("device.connect", "Connect", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Connect), access: Access::Human,
            enabled: always, params: DeviceParams, run: connect, doc: "{device?}"),
        command!("device.disconnect", "Disconnect", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Connect), access: Access::Human,
            enabled: always, params: DeviceParams, run: disconnect, doc: "{device?}"),
        command!("device.detect", "Find My Laser", menu: [], shortcut: None, kind: Kind::Device(DeviceClass::Connect), access: Access::Human,
            enabled: always, params: NoParams, run: detect, doc: "{} (read-only enumeration)"),
        command!("device.settings.get", "Device Settings", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: DeviceParams, run: settings_get, doc: "{device?}"),
        command!("device.settings.set", "Save Device Settings", menu: [], shortcut: None, kind: Kind::Settings, access: Access::Human,
            enabled: always, params: SettingsParams, run: settings_set, doc: "{device?, …galvo keys of CAT §7 (fieldWidthMm, scaleXPct, negateX, …)}"),
        command!("device.create", "New Device", menu: [], shortcut: None, kind: Kind::Settings, access: Access::Human,
            enabled: always, params: CreateParams, run: create, doc: "{id, name, from?}"),
        command!("device.importEzcad", "Import EZCAD Settings", menu: [], shortcut: None, kind: Kind::Settings, access: Access::Human,
            enabled: always, params: ImportEzcadParams, run: import_ezcad, doc: "{path, device?}"),
        command!("device.calibrateAxis", "Calibrate Axis", menu: [], shortcut: None, kind: Kind::Settings, access: Access::Human,
            enabled: always, params: CalibrateParams, run: calibrate_axis, doc: "{axis:1|2, requestedMm, measuredMm, device?}"),
        command!("device.inspectProfile", "Inspect Device Profile", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: DeviceParams, run: inspect_profile, doc: "{device?}"),
    ]
}

fn target(s: &Session, id: &str, device: Option<String>) -> Result<MachineProfile, EngineError> {
    let m = match device {
        Some(d) => MachineId(d),
        None => s.current_machine().ok_or_else(|| EngineError::disabled(id, "this session has no machine profiles"))?,
    };
    s.profile(&m).cloned().ok_or_else(|| EngineError::bad(id, "/device", format!("no machine profile `{}`", m.0)))
}

fn list(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    Ok(s.machine_list_json())
}

/// `device.select`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SelectDevice {
    /// Machine profile id (`device.list`).
    pub device: String,
}

fn select(s: &mut Session, p: SelectDevice) -> Result<Value, EngineError> {
    const ID: &str = "device.select";
    let m = MachineId(p.device);
    if s.profile(&m).is_none() {
        return Err(EngineError::bad(ID, "/device", format!("no machine profile `{}`", m.0)));
    }
    if let Some(busy) = s.busy_machine() {
        return Err(EngineError::refused(RefusalCode::MachineBusy, format!("{} is framing or running; stop it first", busy.0)));
    }
    let idx = s.doc_index()?;
    if s.doc()?.machine == m {
        return Ok(json!({ "device": m.0, "changed": false }));
    }
    let d = s.doc_mut()?;
    d.machine = m.clone();
    d.revision = d.revision.saturating_add(1);
    s.after_change(idx, "the machine changed");
    Ok(json!({ "device": m.0, "changed": true }))
}

fn connect(s: &mut Session, p: DeviceParams) -> Result<Value, EngineError> {
    let origin = s.current_origin();
    let m = target(s, "device.connect", p.device)?.id;
    s.connect(origin, &m)?;
    Ok(json!({ "device": m.0, "state": "connecting" }))
}

fn disconnect(s: &mut Session, p: DeviceParams) -> Result<Value, EngineError> {
    let origin = s.current_origin();
    let m = target(s, "device.disconnect", p.device)?.id;
    s.disconnect(origin, &m)?;
    Ok(json!({ "device": m.0, "state": "disconnected" }))
}

fn detect(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    human_here(s, "device.detect")?;
    let provider = s.provider()?;
    let found = provider.detect().map_err(|e| match e {
        openlaser_driver::DriverError::Refused(r) => EngineError::Refused(r),
        other => EngineError::refused(RefusalCode::NoDevice, format!("detect: {other}")),
    })?;
    Ok(json!({ "provider": provider.name(), "devices": found }))
}

fn profile_json(m: &MachineProfile) -> Value {
    let problems = validate_profile(m).err().unwrap_or_default();
    let galvo = match &m.kind {
        Kinematics::Galvo(g) => {
            let w = g.effective_work_area();
            json!({ "fieldF": g.field_f(), "effectiveWorkArea": { "min": w.min, "max": w.max } })
        }
        Kinematics::Gantry(_) => Value::Null,
    };
    json!({ "profile": m, "valid": problems.is_empty(), "problems": problems, "galvo": galvo })
}

fn settings_get(s: &mut Session, p: DeviceParams) -> Result<Value, EngineError> {
    Ok(profile_json(&target(s, "device.settings.get", p.device)?))
}

fn inspect_profile(s: &mut Session, p: DeviceParams) -> Result<Value, EngineError> {
    let m = target(s, "device.inspectProfile", p.device)?;
    let mut v = profile_json(&m);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("connected".into(), Value::Bool(s.status(&m.id).is_some_and(|st| st.connected)));
    }
    Ok(v)
}

/// The human-only write path (S24): validate, save through the store, then use the new profile in this session.
fn save_profile(s: &mut Session, id: &str, new: MachineProfile, replace: bool) -> Result<Value, EngineError> {
    let store = s.cfg.profiles.clone().ok_or_else(|| EngineError::refused(RefusalCode::NoDevice, "profiles are read-only in this session"))?;
    if s.slot(&new.id).is_some() {
        return Err(EngineError::refused(RefusalCode::MachineBusy, format!("disconnect {} before changing its profile", new.id.0)));
    }
    if let Err(problems) = validate_profile(&new) {
        return Err(EngineError::bad(id, "", format!("the profile is not valid: {}", problems.join("; "))));
    }
    store.save(&new).map_err(|e| EngineError::Io(format!("saving profile {}: {e}", new.id.0)))?;
    let mid = new.id.clone();
    match s.cfg.machines.iter_mut().find(|m| m.id == mid) {
        Some(slot) if replace => *slot = new.clone(),
        Some(_) => return Err(EngineError::bad(id, "/id", format!("a profile `{}` already exists", mid.0))),
        None => s.cfg.machines.push(new.clone()),
    }
    s.bump_docs(Some(&mid), "the machine profile changed");
    Ok(profile_json(&new))
}

fn require_profiles(s: &Session, id: &str) -> Result<(), EngineError> {
    human_here(s, id)?;
    if s.cfg.profiles.is_none() {
        return Err(EngineError::refused(RefusalCode::NoDevice, "profiles are read-only in this session"));
    }
    Ok(())
}

fn galvo_mut<'a>(id: &str, m: &'a mut MachineProfile) -> Result<&'a mut GalvoMachine, EngineError> {
    match &mut m.kind {
        Kinematics::Galvo(g) => Ok(g),
        Kinematics::Gantry(_) => Err(EngineError::bad(id, "", format!("{} is a GRBL profile; these are galvo settings", m.id.0))),
    }
}

/// `device.settings.set`: Device Settings (CAT §7) for a galvo profile. Every key optional.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettingsParams {
    #[serde(default)]
    pub work_area: Option<serde_json::Value>,
    #[serde(default)]
    pub cal: Option<serde_json::Value>,
    #[serde(default)]
    pub red_dot: Option<serde_json::Value>,
    #[serde(default)]
    pub rotary: Option<serde_json::Value>,
    /// Profile id. Default: the active document's machine.
    #[serde(default)]
    pub device: Option<String>,
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Field (work area) width, mm. Grows only as far as hardware item H27 cleared.
    #[serde(default)]
    pub field_width_mm: Option<f64>,
    /// Field (work area) height, mm.
    #[serde(default)]
    pub field_height_mm: Option<f64>,
    /// Field offset X, mm.
    #[serde(default)]
    pub field_offset_x_mm: Option<f64>,
    /// Field offset Y, mm.
    #[serde(default)]
    pub field_offset_y_mm: Option<f64>,
    /// Field angle, degrees.
    #[serde(default)]
    pub field_angle_deg: Option<f64>,
    /// "Galvo 1 is X axis".
    #[serde(default)]
    pub galvo1_is_x: Option<bool>,
    /// Reverse direction of X.
    #[serde(default)]
    pub negate_x: Option<bool>,
    /// Reverse direction of Y.
    #[serde(default)]
    pub negate_y: Option<bool>,
    /// Axis 1 scale, percent (50..=200).
    #[serde(default)]
    pub scale_x_pct: Option<f64>,
    /// Axis 2 scale, percent (50..=200).
    #[serde(default)]
    pub scale_y_pct: Option<f64>,
    /// Red dot offset X, mm.
    #[serde(default)]
    pub red_dot_offset_x_mm: Option<f64>,
    /// Red dot offset Y, mm.
    #[serde(default)]
    pub red_dot_offset_y_mm: Option<f64>,
    /// Red dot scale X (factor).
    #[serde(default)]
    pub red_dot_scale_x: Option<f64>,
    /// Red dot scale Y (factor).
    #[serde(default)]
    pub red_dot_scale_y: Option<f64>,
    /// Frame (red light) speed, mm/s.
    #[serde(default)]
    pub frame_speed_mm_s: Option<f64>,
    /// Max layer speed, mm/s (above it is refused).
    #[serde(default)]
    pub max_speed_mm_s: Option<f64>,
    /// Jump speed, mm/s.
    #[serde(default)]
    pub jump_speed_mm_s: Option<f64>,
    /// Min jump delay, µs.
    #[serde(default)]
    pub jump_min_delay_us: Option<f64>,
    /// Max jump delay, µs.
    #[serde(default)]
    pub jump_max_delay_us: Option<f64>,
    /// Laser On TC, µs.
    #[serde(default)]
    pub laser_on_tc_us: Option<f64>,
    /// Laser Off TC, µs.
    #[serde(default)]
    pub laser_off_tc_us: Option<f64>,
    /// Polygon TC, µs.
    #[serde(default)]
    pub polygon_tc_us: Option<f64>,
    /// Enable the Q-pulse width setting (MOPA).
    #[serde(default)]
    pub q_pulse_enabled: Option<bool>,
    /// Accepted pulse widths, ns, ascending.
    #[serde(default)]
    pub pulse_table_ns: Option<Vec<u16>>,
    /// Open-MO delay, µs.
    #[serde(default)]
    pub open_mo_delay_us: Option<f64>,
    /// Lowest pulse frequency, kHz.
    #[serde(default)]
    pub pwm_min_khz: Option<f64>,
    /// Highest pulse frequency, kHz.
    #[serde(default)]
    pub pwm_max_khz: Option<f64>,
    /// Source power ceiling, percent.
    #[serde(default)]
    pub max_power_pct: Option<f64>,
    /// The job checklist ticked at ARM (≥ 1 item).
    #[serde(default)]
    pub checklist: Option<Vec<String>>,
}

fn speed(id: &str, pointer: &str, v: f64) -> Result<MmPerS, EngineError> {
    MmPerS::new(v).map_err(|e| EngineError::bad(id, pointer, e.to_string()))
}

fn us(id: &str, pointer: &str, v: f64) -> Result<Us, EngineError> {
    Us::new(v).map_err(|e| EngineError::bad(id, pointer, e.to_string()))
}

#[allow(clippy::too_many_lines)]
fn apply_settings(id: &str, p: SettingsParams, m: &mut MachineProfile) -> Result<(), EngineError> {
    if let Some(n) = &p.name {
        m.name = name(id, "/name", n.trim(), 128)?;
    }
    if let Some(list) = &p.checklist {
        if list.is_empty() || list.len() > 32 {
            return Err(EngineError::bad(id, "/checklist", "the checklist has 1..=32 items"));
        }
        m.checklist = list.iter().enumerate().map(|(i, t)| name(id, &format!("/checklist/{i}"), t.trim(), 200)).collect::<Result<_, _>>()?;
    }
    let galvo_keys = p.work_area.is_some()
        || p.cal.is_some()
        || p.red_dot.is_some()
        || p.rotary.is_some()
        || p.field_width_mm.is_some()
        || p.field_height_mm.is_some()
        || p.field_offset_x_mm.is_some()
        || p.field_offset_y_mm.is_some()
        || p.field_angle_deg.is_some()
        || p.galvo1_is_x.is_some()
        || p.negate_x.is_some()
        || p.negate_y.is_some()
        || p.scale_x_pct.is_some()
        || p.scale_y_pct.is_some()
        || p.red_dot_offset_x_mm.is_some()
        || p.red_dot_offset_y_mm.is_some()
        || p.red_dot_scale_x.is_some()
        || p.red_dot_scale_y.is_some()
        || p.frame_speed_mm_s.is_some()
        || p.max_speed_mm_s.is_some()
        || p.jump_speed_mm_s.is_some()
        || p.jump_min_delay_us.is_some()
        || p.jump_max_delay_us.is_some()
        || p.laser_on_tc_us.is_some()
        || p.laser_off_tc_us.is_some()
        || p.polygon_tc_us.is_some()
        || p.q_pulse_enabled.is_some()
        || p.pulse_table_ns.is_some()
        || p.open_mo_delay_us.is_some()
        || p.pwm_min_khz.is_some()
        || p.pwm_max_khz.is_some()
        || p.max_power_pct.is_some();
    if !galvo_keys {
        return Ok(());
    }
    let g = galvo_mut(id, m)?;
    if let Some(v) = p.work_area {
        g.work_area = serde_json::from_value(v).map_err(|e| EngineError::bad(id, "/workArea", e.to_string()))?;
    }
    if let Some(v) = p.cal {
        g.cal = serde_json::from_value(v).map_err(|e| EngineError::bad(id, "/cal", e.to_string()))?;
    }
    if let Some(v) = p.red_dot {
        g.red_dot = serde_json::from_value(v).map_err(|e| EngineError::bad(id, "/redDot", e.to_string()))?;
    }
    if let Some(v) = p.rotary {
        g.rotary = Some(serde_json::from_value(v).map_err(|e| EngineError::bad(id, "/rotary", e.to_string()))?);
    }
    if p.field_width_mm.is_some() || p.field_height_mm.is_some() {
        let cur_w = g.work_area.max[0] - g.work_area.min[0];
        let cur_h = g.work_area.max[1] - g.work_area.min[1];
        let w = positive(id, "/fieldWidthMm", p.field_width_mm.unwrap_or(cur_w), 10_000.0)?;
        let h = positive(id, "/fieldHeightMm", p.field_height_mm.unwrap_or(cur_h), 10_000.0)?;
        g.work_area.min = [-w / 2.0, -h / 2.0];
        g.work_area.max = [w / 2.0, h / 2.0];
    }
    if let Some(v) = p.field_offset_x_mm {
        g.cal.offset_mm[0] = finite(id, "/fieldOffsetXMm", v)?;
    }
    if let Some(v) = p.field_offset_y_mm {
        g.cal.offset_mm[1] = finite(id, "/fieldOffsetYMm", v)?;
    }
    if let Some(v) = p.field_angle_deg {
        g.cal.angle_deg = finite(id, "/fieldAngleDeg", v)?;
    }
    if let Some(v) = p.galvo1_is_x {
        g.cal.swap_xy = !v;
    }
    if let Some(v) = p.negate_x {
        g.cal.negate[0] = v;
    }
    if let Some(v) = p.negate_y {
        g.cal.negate[1] = v;
    }
    if let Some(v) = p.scale_x_pct {
        g.cal.scale[0] = positive(id, "/scaleXPct", v, 1000.0)? / 100.0;
    }
    if let Some(v) = p.scale_y_pct {
        g.cal.scale[1] = positive(id, "/scaleYPct", v, 1000.0)? / 100.0;
    }
    if let Some(v) = p.red_dot_offset_x_mm {
        g.red_dot.offset_mm[0] = finite(id, "/redDotOffsetXMm", v)?;
    }
    if let Some(v) = p.red_dot_offset_y_mm {
        g.red_dot.offset_mm[1] = finite(id, "/redDotOffsetYMm", v)?;
    }
    if let Some(v) = p.red_dot_scale_x {
        g.red_dot.scale[0] = positive(id, "/redDotScaleX", v, 10.0)?;
    }
    if let Some(v) = p.red_dot_scale_y {
        g.red_dot.scale[1] = positive(id, "/redDotScaleY", v, 10.0)?;
    }
    if let Some(v) = p.frame_speed_mm_s {
        g.light_speed = speed(id, "/frameSpeedMmS", v)?;
    }
    if let Some(v) = p.max_speed_mm_s {
        g.max_speed = speed(id, "/maxSpeedMmS", v)?;
    }
    if let Some(v) = p.jump_speed_mm_s {
        g.jump_speed = speed(id, "/jumpSpeedMmS", v)?;
    }
    if let Some(v) = p.jump_min_delay_us {
        g.timings.jump_delay_short_us = us(id, "/jumpMinDelayUs", v)?.get();
    }
    if let Some(v) = p.jump_max_delay_us {
        g.timings.jump_delay_long_us = us(id, "/jumpMaxDelayUs", v)?.get();
    }
    if let Some(v) = p.laser_on_tc_us {
        g.timings.laser_on_us = us(id, "/laserOnTcUs", v)?.get();
    }
    if let Some(v) = p.laser_off_tc_us {
        g.timings.laser_off_us = us(id, "/laserOffTcUs", v)?.get();
    }
    if let Some(v) = p.polygon_tc_us {
        g.timings.polygon_us = us(id, "/polygonTcUs", v)?.get();
    }
    if let Some(v) = p.q_pulse_enabled {
        g.source.q_pulse_enabled = v;
    }
    if let Some(v) = p.pulse_table_ns {
        g.source.pulse_table_ns = v;
    }
    if let Some(v) = p.open_mo_delay_us {
        g.mo_settle = us(id, "/openMoDelayUs", v)?;
    }
    if let Some(v) = p.pwm_min_khz {
        g.source.freq_khz[0] = positive(id, "/pwmMinKhz", v, 4000.0)?;
    }
    if let Some(v) = p.pwm_max_khz {
        g.source.freq_khz[1] = positive(id, "/pwmMaxKhz", v, 4000.0)?;
    }
    if let Some(v) = p.max_power_pct {
        g.source.max_power_pct = positive(id, "/maxPowerPct", v, 100.0)?;
    }
    Ok(())
}

fn settings_set(s: &mut Session, p: SettingsParams) -> Result<Value, EngineError> {
    const ID: &str = "device.settings.set";
    require_profiles(s, ID)?;
    let mut m = target(s, ID, p.device.clone())?;
    apply_settings(ID, p, &mut m)?;
    save_profile(s, ID, m, true)
}

/// `device.create`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateParams {
    /// New profile id: [a-z0-9-]{1,64}.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Profile to copy. Default: the active document's machine.
    #[serde(default)]
    pub from: Option<String>,
}

fn create(s: &mut Session, p: CreateParams) -> Result<Value, EngineError> {
    const ID: &str = "device.create";
    require_profiles(s, ID)?;
    let ok = !p.id.is_empty() && p.id.len() <= 64 && p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(EngineError::bad(ID, "/id", "profile ids are 1..=64 characters of a-z, 0-9 and -"));
    }
    let mid = MachineId(p.id.clone());
    if s.profile(&mid).is_some() {
        return Err(EngineError::bad(ID, "/id", format!("a profile `{}` already exists", p.id)));
    }
    let mut m = target(s, ID, p.from)?;
    m.id = mid;
    m.name = name(ID, "/name", p.name.trim(), 128)?;
    save_profile(s, ID, m, false)
}

/// `device.importEzcad`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ImportEzcadParams {
    /// An EZCAD `markcfg` file.
    pub path: String,
    /// Profile to update. Default: the active document's machine.
    #[serde(default)]
    pub device: Option<String>,
}

fn import_ezcad(s: &mut Session, p: ImportEzcadParams) -> Result<Value, EngineError> {
    const ID: &str = "device.importEzcad";
    require_profiles(s, ID)?;
    let base = target(s, ID, p.device)?;
    let bytes = s.read_file(&p.path)?;
    let cfg = openlaser_machine::markcfg::parse(&bytes).map_err(|e| EngineError::Io(format!("{}: {e}", p.path)))?;
    let (m, applied) = openlaser_machine::markcfg::apply(&cfg, &base).map_err(|e| EngineError::Io(format!("{}: {e}", p.path)))?;
    let mut v = save_profile(s, ID, m, true)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("applied".into(), json!(applied.applied));
        obj.insert("ignored".into(), json!(applied.ignored));
    }
    Ok(v)
}

/// `device.calibrateAxis`: new scale = old × requested / measured.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CalibrateParams {
    /// Galvo axis: 1 or 2.
    pub axis: u8,
    /// The length that was marked, mm.
    pub requested_mm: f64,
    /// The length measured on the material, mm.
    pub measured_mm: f64,
    /// Profile. Default: the active document's machine.
    #[serde(default)]
    pub device: Option<String>,
}

fn calibrate_axis(s: &mut Session, p: CalibrateParams) -> Result<Value, EngineError> {
    const ID: &str = "device.calibrateAxis";
    require_profiles(s, ID)?;
    let axis = match p.axis {
        1 => 0usize,
        2 => 1usize,
        _ => return Err(EngineError::bad(ID, "/axis", "axis is 1 or 2")),
    };
    let req = positive(ID, "/requestedMm", p.requested_mm, 10_000.0)?;
    let meas = positive(ID, "/measuredMm", p.measured_mm, 10_000.0)?;
    let mut m = target(s, ID, p.device)?;
    let g = galvo_mut(ID, &mut m)?;
    let Some(scale) = g.cal.scale.get_mut(axis) else { return Err(EngineError::bad(ID, "/axis", "axis is 1 or 2")) };
    *scale *= req / meas;
    let new_scale = *scale;
    let mut v = save_profile(s, ID, m, true)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("scale".into(), json!(new_scale));
    }
    Ok(v)
}

//! `openlaser-hw profile show|set|import-om|import-ezcad` (SPEC §3.21, S9, S24): the only CLI path that writes a
//! machine profile. TTY only. It prints every changed value as old → new, runs `validate_profile`, and writes through
//! `DirProfileStore` only after the operator types `SAVE <id>`.
//!
//! Keys (galvo profiles): cal.negate.x, cal.negate.y, cal.swapXy, cal.offsetMm, cal.scale, runAhead.maxPackets,
//! runAhead.maxEmissionS, runAhead.watchdogFactor, runAhead.watchdogSlackS, transferTimeoutMs, usbSetConfiguration,
//! redDot.offsetMm, redDot.scale, workArea (within h27Cleared), h27Cleared.

use std::path::Path;

use openlaser_format::machines::DirProfileStore;
use openlaser_machine::{FieldRect, GalvoMachine, Kinematics, MachineId, MachineProfile, ProfileStore, validate_profile};
use openlaser_types::RefusalCode;
use serde_json::Value;

use crate::args::{ProfileAction, ProfileOpts};
use crate::rig::DEFAULT_USB_MACHINE;
use crate::{Env, HwError, Terminal, require_tty};

pub const KEYS: [&str; 29] = [
    "cal.negate.x",
    "cal.negate.y",
    "cal.swapXy",
    "cal.offsetMm",
    "cal.scale",
    "runAhead.maxPackets",
    "runAhead.maxEmissionS",
    "runAhead.watchdogFactor",
    "runAhead.watchdogSlackS",
    "transferTimeoutMs",
    "usbSetConfiguration",
    "redDot.offsetMm",
    "redDot.scale",
    "workArea",
    "h27Cleared",
    "rotary.enabled",
    "rotary.kind",
    "rotary.pulses_per_rev",
    "rotary.drive_diameter_mm",
    "rotary.object_diameter_mm",
    "rotary.axis",
    "rotary.split_mm",
    "rotary.overlap_mm",
    "rotary.reverse",
    "rotary.min_speed_pps",
    "rotary.max_speed_pps",
    "rotary.accel_ms",
    "rotary.return_to_start",
    "rotary.max_turns_per_job",
];

fn bad(key: &str, value: &str, want: &str) -> HwError {
    HwError::Usage(format!("profile set {key}: `{value}` is not {want}"))
}

fn parse_bool(key: &str, v: &str) -> Result<bool, HwError> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(bad(key, v, "true or false")),
    }
}

fn parse_f64(key: &str, v: &str) -> Result<f64, HwError> {
    match v.trim().parse::<f64>() {
        Ok(x) if x.is_finite() => Ok(x),
        _ => Err(bad(key, v, "a finite number")),
    }
}

fn parse_list(key: &str, v: &str) -> Result<Vec<f64>, HwError> {
    v.split(',').map(|p| parse_f64(key, p)).collect()
}

/// "x,y", or one number for both axes.
fn parse_pair(key: &str, v: &str) -> Result<[f64; 2], HwError> {
    match parse_list(key, v)?.as_slice() {
        [a] => Ok([*a, *a]),
        [a, b] => Ok([*a, *b]),
        _ => Err(bad(key, v, "one number or `x,y`")),
    }
}

/// "minX,minY,maxX,maxY", or one half-size `e` for ±e.
fn parse_rect(key: &str, v: &str) -> Result<FieldRect, HwError> {
    let r = match parse_list(key, v)?.as_slice() {
        [e] if *e > 0.0 => FieldRect { min: [-*e, -*e], max: [*e, *e] },
        [a, b, c, d] => FieldRect { min: [*a, *b], max: [*c, *d] },
        _ => return Err(bad(key, v, "a positive half-size `e` (for ±e) or `minX,minY,maxX,maxY`")),
    };
    if r.min[0] >= r.max[0] || r.min[1] >= r.max[1] {
        return Err(bad(key, v, "a rectangle with min below max"));
    }
    Ok(r)
}

/// Applies one `key value` to a galvo profile. Range checks beyond parsing are `validate_profile`'s.
pub fn apply(p: &mut MachineProfile, key: &str, value: &str) -> Result<(), HwError> {
    let Kinematics::Galvo(g) = &mut p.kind else {
        return Err(HwError::refused(RefusalCode::WrongDeviceKind, format!("`{}` is not a galvo profile; its settings are edited in the GUI", p.id.0)));
    };
    apply_galvo(g, key, value)
}

fn apply_galvo(g: &mut GalvoMachine, key: &str, v: &str) -> Result<(), HwError> {
    if let Some(field) = key.strip_prefix("rotary.") {
        let mut r = g.rotary.clone().unwrap_or_default();
        match field {
            "enabled" => r.enabled = parse_bool(key, v)?,
            "kind" => {
                r.kind = match v.to_ascii_lowercase().as_str() {
                    "chuck" => openlaser_machine::RotaryKind::Chuck,
                    "roller" => openlaser_machine::RotaryKind::Roller,
                    _ => return Err(bad(key, v, "chuck or roller")),
                }
            }
            "pulses_per_rev" => r.pulses_per_rev = v.parse::<u32>().map_err(|_| bad(key, v, "an unsigned integer"))?,
            "drive_diameter_mm" => r.drive_diameter_mm = parse_f64(key, v)?,
            "object_diameter_mm" => r.object_diameter_mm = parse_f64(key, v)?,
            "axis" => {
                r.axis = match v.to_ascii_lowercase().as_str() {
                    "x" => openlaser_machine::RotaryAxis::X,
                    "y" => openlaser_machine::RotaryAxis::Y,
                    _ => return Err(bad(key, v, "X or Y")),
                }
            }
            "split_mm" => r.split_mm = parse_f64(key, v)?,
            "overlap_mm" => r.overlap_mm = parse_f64(key, v)?,
            "reverse" => r.reverse = parse_bool(key, v)?,
            "min_speed_pps" => r.min_speed_pps = v.parse::<u16>().map_err(|_| bad(key, v, "an unsigned 16-bit integer"))?,
            "max_speed_pps" => r.max_speed_pps = v.parse::<u16>().map_err(|_| bad(key, v, "an unsigned 16-bit integer"))?,
            "accel_ms" => r.accel_ms = v.parse::<u16>().map_err(|_| bad(key, v, "an unsigned 16-bit integer"))?,
            "return_to_start" => r.return_to_start = parse_bool(key, v)?,
            "max_turns_per_job" => r.max_turns_per_job = parse_f64(key, v)?,
            _ => return Err(bad(key, v, "a supported rotary field")),
        }
        g.rotary = Some(r);
        return Ok(());
    }
    match key {
        "cal.negate.x" => g.cal.negate[0] = parse_bool(key, v)?,
        "cal.negate.y" => g.cal.negate[1] = parse_bool(key, v)?,
        "cal.swapXy" => g.cal.swap_xy = parse_bool(key, v)?,
        "cal.offsetMm" => g.cal.offset_mm = parse_pair(key, v)?,
        "cal.scale" => g.cal.scale = parse_pair(key, v)?,
        "runAhead.maxPackets" => g.run_ahead.max_packets = v.trim().parse::<u8>().map_err(|_| bad(key, v, "a whole number from 0 to 255"))?,
        "runAhead.maxEmissionS" => g.run_ahead.max_emission_s = parse_f64(key, v)?,
        "runAhead.watchdogFactor" => g.run_ahead.watchdog_factor = parse_f64(key, v)?,
        "runAhead.watchdogSlackS" => g.run_ahead.watchdog_slack_s = parse_f64(key, v)?,
        "transferTimeoutMs" => g.transfer_timeout_ms = v.trim().parse::<u32>().map_err(|_| bad(key, v, "a whole number of milliseconds"))?,
        "usbSetConfiguration" => g.usb_set_configuration = parse_bool(key, v)?,
        "redDot.offsetMm" => g.red_dot.offset_mm = parse_pair(key, v)?,
        "redDot.scale" => g.red_dot.scale = parse_pair(key, v)?,
        "workArea" => g.work_area = parse_rect(key, v)?,
        "h27Cleared" => {
            g.h27_cleared = if v.trim().eq_ignore_ascii_case("none") { None } else { Some(parse_rect(key, v)?) };
        }
        other => return Err(HwError::Usage(format!("profile set: unknown key `{other}`; keys: {}", KEYS.join(", ")))),
    }
    Ok(())
}

/// Every changed leaf as `path: old → new` (JSON values; the `kind.` prefix of galvo fields is dropped).
pub fn diff(old: &MachineProfile, new: &MachineProfile) -> Vec<String> {
    let (Ok(a), Ok(b)) = (serde_json::to_value(old), serde_json::to_value(new)) else {
        return vec!["(the profiles could not be compared)".to_owned()];
    };
    let mut out = Vec::new();
    walk("", &a, &b, &mut out);
    out
}

fn walk(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                walk(&p, x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), out);
            }
        }
        _ => {
            let shown = path.strip_prefix("kind.").unwrap_or(path);
            out.push(format!("{shown}: {a} → {b}"));
        }
    }
}

fn pick(machines: &[MachineProfile], id: &MachineId) -> Result<MachineProfile, HwError> {
    machines.iter().find(|m| &m.id == id).cloned().ok_or_else(|| HwError::refused(RefusalCode::NoDevice, format!("no machine profile `{}`", id.0)))
}

fn read_file(path: &Path) -> Result<Vec<u8>, HwError> {
    std::fs::read(path).map_err(|e| HwError::Failed(format!("cannot read {}: {e}", path.display())))
}

/// A profile file that did not load leaves its machine on the built-in settings: the diff would compare the
/// built-in (not the file) with the new values, and SAVE would replace the file with them. Every write refuses until
/// each file loads again (`profile show` still lists them).
pub fn refuse_writes_over_skipped(skipped: &[String]) -> Result<(), HwError> {
    if skipped.is_empty() {
        return Ok(());
    }
    Err(HwError::Failed(format!(
        "machine profile files were skipped: {}. Fix or remove them before changing a profile (SAVE would replace a skipped file \
         with the built-in settings plus this change); nothing was written",
        crate::plain::field(&skipped.join("; "), crate::plain::MESSAGE_MAX_CHARS)
    )))
}

pub fn run(o: &ProfileOpts, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    require_tty(term, "profile")?;
    let dir = env.profiles_dir.clone().ok_or_else(|| HwError::Failed("no per-user data folder for machine profiles on this system".to_owned()))?;
    let (machines, skipped) = openlaser_format::machines::load_all(Some(&dir));
    for s in &skipped {
        term.write(&format!("note: {}\n", crate::plain::field(s, crate::plain::MESSAGE_MAX_CHARS)));
    }
    if !matches!(o.action, ProfileAction::Show) {
        refuse_writes_over_skipped(&skipped)?;
    }
    let id = o.machine.clone().unwrap_or_else(|| MachineId(DEFAULT_USB_MACHINE.to_owned()));
    let old = pick(&machines, &id)?;
    let new = match &o.action {
        ProfileAction::Show => {
            let text = serde_json::to_string_pretty(&old).map_err(|e| HwError::Failed(e.to_string()))?;
            term.write(&format!("{text}\n"));
            return Ok(());
        }
        ProfileAction::Set { key, value } => {
            let mut p = old.clone();
            apply(&mut p, key, value)?;
            p
        }
        ProfileAction::ImportOm(path) => {
            let scale = openlaser_machine::om::parse_calibration(&read_file(path)?).map_err(|e| HwError::Failed(format!("{}: {e}", path.display())))?;
            let mut p = old.clone();
            apply(&mut p, "cal.scale", &format!("{scale},{scale}"))?;
            p
        }
        ProfileAction::ImportEzcad(path) => {
            let cfg = openlaser_machine::markcfg::parse(&read_file(path)?).map_err(|e| HwError::Failed(format!("{}: {e}", path.display())))?;
            let (p, applied) = openlaser_machine::markcfg::apply(&cfg, &old).map_err(|e| HwError::Failed(format!("{}: {e}", path.display())))?;
            term.write(&format!("markcfg: applied {}; ignored {}\n", applied.applied.join(", "), applied.ignored.join(", ")));
            p
        }
    };
    save_confirmed(&old, &new, &dir, term)
}

/// Prints old → new, validates, asks `SAVE <id>`, writes.
fn save_confirmed(old: &MachineProfile, new: &MachineProfile, dir: &Path, term: &mut dyn Terminal) -> Result<(), HwError> {
    let changes = diff(old, new);
    if changes.is_empty() {
        term.write("Nothing changes; nothing was written.\n");
        return Ok(());
    }
    term.write(&format!("Machine profile `{}`:\n", new.id.0));
    for c in &changes {
        term.write(&format!("  {c}\n"));
    }
    if let Err(problems) = validate_profile(new) {
        return Err(HwError::Failed(format!("the new profile is not valid, nothing was written: {}", problems.join("; "))));
    }
    let expect = format!("SAVE {}", new.id.0);
    term.write(&format!("Type `{expect}` to write it (anything else cancels): "));
    match term.read_line() {
        Some(line) if line.trim() == expect => {}
        _ => return Err(HwError::Failed("not saved".to_owned())),
    }
    DirProfileStore::new(dir.to_path_buf()).save(new).map_err(|e| HwError::Failed(format!("not saved: {e}")))?;
    term.write("Saved.\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use openlaser_machine::jpt_m7_200;

    use super::*;

    #[test]
    fn writes_refuse_while_a_profile_file_is_skipped() {
        assert!(refuse_writes_over_skipped(&[]).is_ok());
        let err = refuse_writes_over_skipped(&["machines/jpt-m7-200.json: cal.scale must be finite\u{1b}[8m".to_owned()]).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("machines/jpt-m7-200.json: cal.scale must be finite[8m") && text.contains("nothing was written"), "{text}");
        assert!(!text.chars().any(crate::plain::hidden), "{text:?}");
    }

    fn galvo(p: &MachineProfile) -> &GalvoMachine {
        match &p.kind {
            Kinematics::Galvo(g) => g,
            Kinematics::Gantry(_) => panic!("galvo"),
        }
    }

    #[test]
    fn every_key_applies() {
        let base = jpt_m7_200();
        for (key, value) in [
            ("cal.negate.x", "true"),
            ("cal.negate.y", "on"),
            ("cal.swapXy", "yes"),
            ("cal.offsetMm", "0.5,-0.25"),
            ("cal.scale", "1.2"),
            ("runAhead.maxPackets", "2"),
            ("runAhead.maxEmissionS", "1.5"),
            ("runAhead.watchdogFactor", "2"),
            ("runAhead.watchdogSlackS", "3"),
            ("transferTimeoutMs", "300"),
            ("usbSetConfiguration", "true"),
            ("redDot.offsetMm", "0.1,0.2"),
            ("redDot.scale", "1.01,0.99"),
            ("workArea", "90"),
            ("h27Cleared", "-110,-110,110,110"),
        ] {
            assert!(KEYS.contains(&key));
            let mut p = base.clone();
            apply(&mut p, key, value).unwrap_or_else(|e| panic!("{key} {value}: {e}"));
            assert_ne!(p, base, "{key} changes the profile");
            assert!(!diff(&base, &p).is_empty(), "{key} shows in the diff");
        }
        let mut p = base.clone();
        apply(&mut p, "cal.scale", "1.2").unwrap();
        assert_eq!(galvo(&p).cal.scale, [1.2, 1.2]);
        apply(&mut p, "workArea", "-50,-40,50,40").unwrap();
        assert_eq!(galvo(&p).work_area, FieldRect { min: [-50.0, -40.0], max: [50.0, 40.0] });
        apply(&mut p, "h27Cleared", "none").unwrap();
        assert_eq!(galvo(&p).h27_cleared, None);
    }

    #[test]
    fn bad_values_are_refused() {
        let mut p = jpt_m7_200();
        for (key, value) in [
            ("cal.negate.x", "maybe"),
            ("cal.scale", "NaN"),
            ("cal.scale", "1,2,3"),
            ("cal.offsetMm", "inf,0"),
            ("runAhead.maxPackets", "-1"),
            ("transferTimeoutMs", "1.5"),
            ("workArea", "0"),
            ("workArea", "10,10,-10,-10"),
            ("h27Cleared", "a,b,c,d"),
            ("cal.angleDeg", "5"),
        ] {
            assert!(apply(&mut p, key, value).is_err(), "{key} {value}");
        }
        assert_eq!(p, jpt_m7_200(), "a refused value changes nothing");
    }

    #[test]
    fn diff_lists_old_and_new() {
        let base = jpt_m7_200();
        let mut p = base.clone();
        apply(&mut p, "cal.negate.x", "true").unwrap();
        apply(&mut p, "transferTimeoutMs", "300").unwrap();
        let d = diff(&base, &p);
        assert_eq!(d, vec!["cal.negate: [false,false] → [true,false]".to_owned(), "transferTimeoutMs: 250 → 300".to_owned()]);
    }
    #[test]
    fn every_rotary_field_applies_without_bypassing_profile_validation() {
        let mut p = jpt_m7_200();
        for (key, value) in [
            ("enabled", "true"),
            ("kind", "roller"),
            ("pulses_per_rev", "6400"),
            ("drive_diameter_mm", "20"),
            ("object_diameter_mm", "35"),
            ("axis", "x"),
            ("split_mm", "8"),
            ("overlap_mm", "1"),
            ("reverse", "true"),
            ("min_speed_pps", "200"),
            ("max_speed_pps", "1200"),
            ("accel_ms", "150"),
            ("return_to_start", "false"),
            ("max_turns_per_job", "3"),
        ] {
            apply(&mut p, &format!("rotary.{key}"), value).unwrap();
            assert!(validate_profile(&p).is_ok(), "{key}");
        }
        let r = galvo(&p).rotary.as_ref().unwrap();
        assert!(r.enabled && r.reverse);
        assert!(!r.return_to_start);
        assert_eq!(r.axis, openlaser_machine::RotaryAxis::X);
        apply(&mut p, "rotary.pulses_per_rev", "0").unwrap();
        assert!(validate_profile(&p).is_err());
        assert!(apply(&mut p, "rotary.kind", "belt").is_err());
    }
}

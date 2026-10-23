//! Preferences (`prefs.get` / `prefs.set`): machine-local, not project data, not in history.
//!
//! Keys under `machine.`, `safety.`, `automation.` and `humanOnly.` are refused for every origin (§4.5, AUT C3.6):
//! they would bound safety or automation access, which only a person changes, in their own dialogs. Keys the
//! planner reads (`output.*`) bump every document's revision, so a pending ARM is voided and requests go stale.

use std::collections::BTreeMap;

use openlaser_types::RefusalCode;
use serde_json::Value;

use crate::command::{Access, CommandSpec, EngineError, registry};

/// Prefix families no `prefs.set` may touch.
pub const FORBIDDEN_PREFIXES: [&str; 4] = ["machine.", "safety.", "automation.", "humanOnly."];

pub const CURVE_TOLERANCE: &str = "output.curveToleranceMm";
pub const MAX_STATES: &str = "history.maxStates";

/// Largest serialised value a pref may hold.
const MAX_VALUE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Prefs {
    values: BTreeMap<String, Value>,
}

/// What a successful `set` changed, so the session can react.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefEffect {
    None,
    /// A key the planner reads: bump every document's revision.
    Planner,
    /// The undo depth changed.
    History,
}

impl Prefs {
    /// The stored value, or the built-in default for a known key.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.values.get(key).cloned().or_else(|| default_of(key))
    }

    /// Every known default overlaid with the stored values.
    pub fn all(&self) -> BTreeMap<String, Value> {
        let mut m: BTreeMap<String, Value> = [CURVE_TOLERANCE, MAX_STATES].iter().filter_map(|k| default_of(k).map(|v| ((*k).to_owned(), v))).collect();
        for (k, v) in &self.values {
            m.insert(k.clone(), v.clone());
        }
        m
    }

    /// Flattening tolerance the planner uses (mm).
    pub fn curve_tolerance_mm(&self) -> f64 {
        self.get(CURVE_TOLERANCE).and_then(|v| v.as_f64()).unwrap_or(openlaser_geom::DEFAULT_FLATTEN_TOL_MM)
    }

    /// Undo depth per document.
    pub fn max_states(&self) -> usize {
        self.get(MAX_STATES).and_then(|v| v.as_u64()).and_then(|v| usize::try_from(v).ok()).unwrap_or(100)
    }

    /// The effective shortcut of a command: a `shortcuts.<id>` override (string, or null for none) or the default.
    /// Human-only commands never have one (S18).
    pub fn shortcut(&self, spec: &CommandSpec) -> Option<String> {
        if spec.access == Access::Human {
            return None;
        }
        match self.values.get(&format!("shortcuts.{}", spec.id)) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => None,
            None => spec.shortcut.map(str::to_owned),
        }
    }

    /// Validates and stores one key.
    pub(crate) fn set(&mut self, key: &str, value: Value) -> Result<PrefEffect, EngineError> {
        const ID: &str = "prefs.set";
        if let Some(prefix) = FORBIDDEN_PREFIXES.iter().find(|p| key.starts_with(*p)) {
            return Err(EngineError::refused(
                RefusalCode::HumanOnly,
                format!("`{key}`: `{prefix}*` settings bound safety or automation access and are changed only by a person in their own OpenLaser dialog"),
            ));
        }
        if !key_grammar_ok(key) {
            return Err(EngineError::bad(ID, "/key", format!("`{key}` is not a preference key (dot-separated camelCase segments)")));
        }
        if serde_json::to_string(&value).map_or(usize::MAX, |s| s.len()) > MAX_VALUE_BYTES {
            return Err(EngineError::bad(ID, "/value", format!("values are at most {MAX_VALUE_BYTES} bytes")));
        }
        let effect = match key {
            CURVE_TOLERANCE => {
                let v = value.as_f64().filter(|v| v.is_finite() && (0.001..=1.0).contains(v));
                if v.is_none() {
                    return Err(EngineError::bad(ID, "/value", format!("`{CURVE_TOLERANCE}` takes a number in 0.001..=1.0 mm")));
                }
                PrefEffect::Planner
            }
            MAX_STATES => {
                if !value.as_u64().is_some_and(|v| (1..=1000).contains(&v)) {
                    return Err(EngineError::bad(ID, "/value", format!("`{MAX_STATES}` takes a whole number in 1..=1000")));
                }
                PrefEffect::History
            }
            k if k.starts_with("shortcuts.") => {
                let id = k.strip_prefix("shortcuts.").unwrap_or_default();
                let Some(spec) = registry().find(id) else {
                    return Err(EngineError::bad(ID, "/key", format!("no command `{id}` to bind")));
                };
                if spec.access == Access::Human {
                    return Err(EngineError::refused(RefusalCode::HumanOnly, format!("`{}` is human-only and never has a shortcut", spec.id)));
                }
                if !(value.is_null() || value.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 64)) {
                    return Err(EngineError::bad(ID, "/value", "a shortcut is a string such as \"Cmd+Shift+H\", or null for none"));
                }
                PrefEffect::None
            }
            k if k.starts_with("output.") => PrefEffect::Planner,
            _ => PrefEffect::None,
        };
        self.values.insert(key.to_owned(), value);
        Ok(effect)
    }
}

fn default_of(key: &str) -> Option<Value> {
    match key {
        CURVE_TOLERANCE => Some(Value::from(openlaser_geom::DEFAULT_FLATTEN_TOL_MM)),
        MAX_STATES => Some(Value::from(100u64)),
        _ => None,
    }
}

/// `^[a-z][a-zA-Z0-9]*(\.[a-zA-Z0-9]+)+$`, ≤ 128 chars (shortcut keys embed a command id).
fn key_grammar_ok(key: &str) -> bool {
    if key.len() > 128 {
        return false;
    }
    let mut segments = key.split('.');
    let Some(first) = segments.next() else { return false };
    let first_ok = first.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && first.chars().all(|c| c.is_ascii_alphanumeric());
    let rest: Vec<&str> = segments.collect();
    first_ok && !rest.is_empty() && rest.iter().all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn forbidden_prefixes_are_refused() {
        let mut p = Prefs::default();
        for k in ["machine.maxPowerPct", "safety.workArea", "automation.allowAgents", "humanOnly.arm"] {
            let e = p.set(k, json!(1)).unwrap_err();
            assert_eq!(e.code(), "human_only", "{k}");
        }
        assert!(p.all().keys().all(|k| !FORBIDDEN_PREFIXES.iter().any(|f| k.starts_with(f))));
    }

    #[test]
    fn known_keys_are_validated() {
        let mut p = Prefs::default();
        assert_eq!(p.set(CURVE_TOLERANCE, json!(0.05)).unwrap(), PrefEffect::Planner);
        assert_eq!(p.curve_tolerance_mm(), 0.05);
        assert!(p.set(CURVE_TOLERANCE, json!(5)).is_err());
        assert!(p.set(CURVE_TOLERANCE, json!("x")).is_err());
        assert_eq!(p.set(MAX_STATES, json!(10)).unwrap(), PrefEffect::History);
        assert_eq!(p.max_states(), 10);
        assert!(p.set(MAX_STATES, json!(0)).is_err());
        assert_eq!(p.set("ui.theme", json!("dark")).unwrap(), PrefEffect::None);
        assert!(p.set("Bad", json!(1)).is_err());
        assert!(p.set("a..b", json!(1)).is_err());
    }

    #[test]
    fn shortcuts_cannot_bind_human_only_commands() {
        let mut p = Prefs::default();
        assert!(p.set("shortcuts.laser.start", json!("Cmd+Enter")).is_err());
        assert!(p.set("shortcuts.edit.undo", json!("Cmd+Y")).is_ok());
        let spec = registry().find("edit.undo").unwrap();
        assert_eq!(p.shortcut(spec).as_deref(), Some("Cmd+Y"));
        assert!(p.set("shortcuts.edit.undo", Value::Null).is_ok());
        assert_eq!(p.shortcut(spec), None);
        assert!(p.set("shortcuts.no.suchCommand", json!("X")).is_err());
    }
}

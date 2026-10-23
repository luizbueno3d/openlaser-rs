//! The registry (ENG §2.2, with typed params): command specs, the `command!` macro, the access × origin policy and
//! the `command_list` rows.

use std::collections::HashMap;
use std::sync::OnceLock;

use openlaser_types::{ObjectId, Refusal, RefusalCode};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::params::{decode, normalise_id};
use crate::session::{JobRefused, Session};

pub type RunFn = fn(&mut Session, serde_json::Value) -> Result<serde_json::Value, EngineError>;

#[derive(Clone, Copy)]
pub struct CommandSpec {
    /// "layer.cutSettings.set" (camelCase segments, S2).
    pub id: &'static str,
    /// English; also the i18n key.
    pub label: &'static str,
    /// LightBurn menu path; [] = not in menus.
    pub menu: &'static [&'static str],
    /// "Cmd+Shift+H"; Cmd = ⌘ on macOS, Ctrl elsewhere.
    pub shortcut: Option<&'static str>,
    pub kind: Kind,
    pub access: Access,
    /// One-line params summary for command_list.
    pub doc: &'static str,
    /// From the typed params struct (doc comments become descriptions).
    pub schema: fn() -> schemars::Schema,
    pub enabled: fn(&Session) -> Result<(), Disabled>,
    pub run: RunFn,
    pub checked: Option<fn(&Session) -> bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Edit,
    View,
    Query,
    Io,
    Settings,
    Device(DeviceClass),
    Request,
}

impl Kind {
    /// The `kind` string of a `command_list` row.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Edit => "edit",
            Kind::View => "view",
            Kind::Query => "query",
            Kind::Io => "io",
            Kind::Settings => "settings",
            Kind::Device(_) => "device",
            Kind::Request => "request",
        }
    }

    /// Journaled (and so replayable): `Edit` and `Io` only, never `Device` (pipeline step 9).
    pub fn journaled(self) -> bool {
        matches!(self, Kind::Edit | Kind::Io)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Stop,
    Connect,
    Light,
    Emit,
}

/// Who may run it; checked in dispatch before params are decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Any origin.
    Agent,
    /// Any origin; for Synthetic/Headless/Replay every path param resolves inside the granted roots.
    Roots,
    /// Needs a window: Human or Synthetic; Headless → Disabled "needs the OpenLaser window (bridge mode)".
    Ui,
    /// Origin::Human only; anything else → Refused(HumanOnly). No menu path, no shortcut (S18).
    Human,
    /// Any origin; never disabled, never refused, never journaled.
    Stop,
}

impl Access {
    pub fn agent_class(self) -> AgentClass {
        match self {
            Access::Agent => AgentClass::Allowed,
            Access::Roots => AgentClass::RootsOnly,
            Access::Ui => AgentClass::Ui,
            Access::Human => AgentClass::HumanOnly,
            Access::Stop => AgentClass::Stop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Human,
    Synthetic,
    Headless,
    Replay,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disabled {
    pub reason: String,
    pub needs_document: bool,
}

impl Disabled {
    pub fn new(reason: impl Into<String>) -> Self {
        Disabled { reason: reason.into(), needs_document: false }
    }

    pub fn needs_document() -> Self {
        Disabled { reason: "no document open".to_owned(), needs_document: true }
    }
}

fn suggest(suggestions: &[String]) -> String {
    if suggestions.is_empty() { String::new() } else { format!("; did you mean {}?", suggestions.join(", ")) }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{id}`{}", suggest(.suggestions))]
    UnknownCommand { id: String, suggestions: Vec<String> },
    #[error("`{id}`: {pointer}: {message}")]
    BadParams { id: String, pointer: String, message: String },
    #[error("`{id}` is disabled: {reason}")]
    Disabled { id: String, reason: String, needs_document: bool },
    /// Policy and per-action safety.
    #[error("refused: {}", .0.message)]
    Refused(Refusal),
    #[error("job refused ({} refusals)", .0.refusals.len())]
    JobRefused(Box<JobRefused>),
    #[error("no document open")]
    NoDocument,
    #[error("no object {0:?}")]
    NoShape(ObjectId),
    #[error("this session has no device access")]
    NoDevice,
    #[error("needs bridge mode: turn on Settings › Automation › Allow agent connections in OpenLaser, then `openlaser-cli mcp --bridge auto`")]
    NeedsBridge,
    #[error("cancelled")]
    Cancelled,
    #[error("file: {0}")]
    Io(String),
    #[error("`{0}` failed with an internal error; the project is unchanged")]
    Internal(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

impl EngineError {
    /// A refusal with `code` and a message the user can act on.
    pub fn refused(code: RefusalCode, message: impl Into<String>) -> Self {
        EngineError::Refused(Refusal::new(code, message))
    }

    /// The policy refusal for a human-only command reached from any other origin.
    pub fn human_only(id: &str) -> Self {
        EngineError::refused(RefusalCode::HumanOnly, format!("`{id}` is started by a person in the OpenLaser window"))
    }

    pub(crate) fn bad(id: &str, pointer: &str, message: impl Into<String>) -> Self {
        EngineError::BadParams { id: id.to_owned(), pointer: pointer.to_owned(), message: message.into() }
    }

    pub(crate) fn disabled(id: &str, reason: impl Into<String>) -> Self {
        EngineError::Disabled { id: id.to_owned(), reason: reason.into(), needs_document: false }
    }

    /// A stable machine-readable code (snake_case), the same in every transport.
    pub fn code(&self) -> &'static str {
        match self {
            EngineError::UnknownCommand { .. } => "unknown_command",
            EngineError::BadParams { .. } => "bad_params",
            EngineError::Disabled { .. } => "disabled",
            EngineError::Refused(r) => refusal_prefix(r.code).1,
            EngineError::JobRefused(_) => "refused",
            EngineError::NoDocument => "no_document",
            EngineError::NoShape(_) => "no_shape",
            EngineError::NoDevice => "no_device",
            EngineError::NeedsBridge => "needs_bridge",
            EngineError::Cancelled => "cancelled",
            EngineError::Io(_) => "io",
            EngineError::Internal(_) => "internal",
            EngineError::NotImplemented(_) => "not_implemented",
        }
    }

    /// The message agents see, with AUT B13's stable prefix (`refused:`, `human-only:`, `machine_busy:`,
    /// `needs_bridge:`, `stale:`, `not_connected:`, `human_confirmation_open:`).
    pub fn agent_message(&self) -> String {
        match self {
            EngineError::Refused(r) => format!("{}: {}", refusal_prefix(r.code).0, r.message),
            EngineError::NeedsBridge => format!("needs_bridge: {self}"),
            other => other.to_string(),
        }
    }

    /// Structured form for batch results and tool errors: `{code, message, pointer?, suggestions?, refusals?, notices?}`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        obj.insert("code".into(), self.code().into());
        obj.insert("message".into(), self.agent_message().into());
        match self {
            EngineError::BadParams { pointer, .. } => {
                obj.insert("pointer".into(), pointer.clone().into());
            }
            EngineError::UnknownCommand { suggestions, .. } => {
                obj.insert("suggestions".into(), suggestions.clone().into());
            }
            EngineError::Disabled { needs_document, .. } => {
                obj.insert("needsDocument".into(), (*needs_document).into());
            }
            EngineError::Refused(r) => {
                obj.insert("refusals".into(), serde_json::to_value(vec![r]).unwrap_or_default());
            }
            EngineError::JobRefused(j) => {
                obj.insert("refusals".into(), serde_json::to_value(&j.refusals).unwrap_or_default());
                obj.insert("notices".into(), serde_json::to_value(&j.notices).unwrap_or_default());
                if let Some(fp) = &j.fingerprint {
                    obj.insert("fingerprint".into(), fp.hex().into());
                }
                if let Some(e) = &j.error {
                    obj.insert("error".into(), e.clone().into());
                }
            }
            _ => {}
        }
        serde_json::Value::Object(obj)
    }
}

/// (agent message prefix, error code) of a refusal.
fn refusal_prefix(code: RefusalCode) -> (&'static str, &'static str) {
    match code {
        RefusalCode::HumanOnly => ("human-only", "human_only"),
        RefusalCode::MachineBusy => ("machine_busy", "machine_busy"),
        RefusalCode::NeedsBridge => ("needs_bridge", "needs_bridge"),
        RefusalCode::Stale => ("stale", "stale"),
        RefusalCode::NotConnected => ("not_connected", "not_connected"),
        RefusalCode::HumanConfirmationOpen => ("human_confirmation_open", "human_confirmation_open"),
        _ => ("refused", "refused"),
    }
}

pub struct Registry {
    specs: Vec<CommandSpec>,
    by_id: HashMap<&'static str, usize>,
    duplicates: Vec<&'static str>,
}

impl Registry {
    /// Builds the index. A duplicate id keeps the first spec and is recorded (`registry_ids_unique` fails on it).
    pub(crate) fn build(specs: Vec<CommandSpec>) -> Registry {
        let mut by_id = HashMap::with_capacity(specs.len());
        let mut duplicates = Vec::new();
        for (i, s) in specs.iter().enumerate() {
            if by_id.contains_key(s.id) {
                duplicates.push(s.id);
            } else {
                by_id.insert(s.id, i);
            }
        }
        Registry { specs, by_id, duplicates }
    }

    /// Exact, then snake_case alias (`flip_h` → `flipH`).
    pub fn find(&self, id: &str) -> Option<&CommandSpec> {
        if let Some(&i) = self.by_id.get(id) {
            return self.specs.get(i);
        }
        let alias = normalise_id(id);
        self.by_id.get(alias.as_str()).and_then(|&i| self.specs.get(i))
    }

    /// Up to three ids close to `id` (edit distance over the whole id and over its last segment, plus substring
    /// matches), best first.
    pub fn suggestions(&self, id: &str) -> Vec<&'static str> {
        let wanted = normalise_id(id).to_ascii_lowercase();
        if wanted.is_empty() {
            return Vec::new();
        }
        let wanted_last = wanted.rsplit('.').next().unwrap_or(&wanted).to_owned();
        let limit = (wanted.chars().count() / 3).max(2);
        let mut scored: Vec<(usize, &'static str)> = Vec::new();
        for s in &self.specs {
            let cand = s.id.to_ascii_lowercase();
            let last = cand.rsplit('.').next().unwrap_or(&cand);
            let mut d = levenshtein(&wanted, &cand).min(levenshtein(&wanted_last, last).saturating_add(1));
            if cand.contains(&wanted) || (wanted_last.len() >= 4 && last.contains(&wanted_last)) {
                d = d.min(1);
            }
            if d <= limit {
                scored.push((d, s.id));
            }
        }
        scored.sort();
        scored.into_iter().map(|(_, id)| id).take(3).collect()
    }

    pub fn specs(&self) -> &[CommandSpec] {
        &self.specs
    }

    /// Ids registered more than once (must stay empty).
    pub fn duplicates(&self) -> &[&'static str] {
        &self.duplicates
    }
}

/// Classic Levenshtein distance over chars (ids are short).
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        if let Some(first) = cur.first_mut() {
            *first = i + 1;
        }
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(usize::MAX).saturating_add(usize::from(ca != *cb));
            let del = prev.get(j + 1).copied().unwrap_or(usize::MAX).saturating_add(1);
            let ins = cur.get(j).copied().unwrap_or(usize::MAX).saturating_add(1);
            if let Some(slot) = cur.get_mut(j + 1) {
                *slot = sub.min(del).min(ins);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.last().copied().unwrap_or(0)
}

/// OnceLock; duplicate ids fail `registry_ids_unique`.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::build(crate::cmds::all()))
}

/// `command!("arrange.flipH", "Flip Horizontal", menu: ["Arrange"], shortcut: Some("Cmd+Shift+H"),
///   kind: Kind::Edit, access: Access::Agent, enabled: needs_selection, params: IdsParams, run: arrange::flip_h)`
/// expands to a CommandSpec whose `run` decodes `IdsParams` strictly, calls the typed fn and serialises the result.
///
/// Optional trailing `doc: "…"` (the `command_list` params summary; default `"{}"`) and `checked: fn(&Session) ->
/// bool`. The generated `run` performs pipeline steps 5–7 (strict decode, `enabled`, device busy) before the typed
/// function, so the order of §3.16 holds whoever calls it.
#[macro_export]
macro_rules! command {
    (@doc) => { "{}" };
    (@doc $doc:expr) => { $doc };
    (@checked) => { ::core::option::Option::None };
    (@checked $ck:expr) => { ::core::option::Option::Some($ck) };
    ($id:literal, $label:literal, menu: [$($m:literal),* $(,)?], shortcut: $sc:expr, kind: $kind:expr, access: $access:expr,
     enabled: $en:expr, params: $P:ty, run: $f:expr $(, doc: $doc:expr)? $(, checked: $ck:expr)? $(,)?) => {
        $crate::CommandSpec {
            id: $id,
            label: $label,
            menu: &[$($m),*],
            shortcut: $sc,
            kind: $kind,
            access: $access,
            doc: $crate::command!(@doc $($doc)?),
            schema: $crate::params_schema::<$P>,
            enabled: $en,
            run: |session, value| $crate::run_typed::<$P, _, _>($id, session, value, $f),
            checked: $crate::command!(@checked $($ck)?),
        }
    };
}

/// The strict JSON Schema of a params struct, subschemas inlined (AUT B6).
pub fn params_schema<P: JsonSchema>() -> schemars::Schema {
    let generator = schemars::generate::SchemaSettings::draft2020_12().with(|s| s.inline_subschemas = true).into_generator();
    generator.into_root_schema_for::<P>()
}

/// The body of every `command!` `run`: strict decode (step 5), `enabled` and device busy (steps 6, 7), the typed
/// function, then the result as JSON.
#[doc(hidden)]
pub fn run_typed<P, R, F>(id: &'static str, session: &mut Session, value: serde_json::Value, f: F) -> Result<serde_json::Value, EngineError>
where
    P: DeserializeOwned,
    R: Serialize,
    F: FnOnce(&mut Session, P) -> Result<R, EngineError>,
{
    let params: P = decode(id, value)?;
    session.pre_run(id)?;
    let result = f(session, params)?;
    serde_json::to_value(result).map_err(|e| {
        log::error!("{id}: result does not serialise: {e}");
        EngineError::Internal(id.to_owned())
    })
}

/// Pipeline step 3: `access × origin` (SPEC §3.16 table). `effective` is the origin a Replay step inherits from its
/// caller (a batch, a macro); for any other origin it equals `origin`. Ok means "go on"; roots are step 4.
pub(crate) fn policy(spec: &CommandSpec, origin: Origin, effective: Origin, window: bool) -> Result<(), EngineError> {
    match spec.access {
        Access::Agent | Access::Roots | Access::Stop => Ok(()),
        Access::Human => {
            if origin == Origin::Human {
                Ok(())
            } else {
                Err(EngineError::human_only(spec.id))
            }
        }
        Access::Ui => match effective {
            Origin::Human | Origin::Synthetic => Ok(()),
            Origin::Replay if window => Ok(()),
            Origin::Headless | Origin::Replay => Err(EngineError::disabled(spec.id, UI_NEEDS_WINDOW)),
        },
    }
}

pub(crate) const UI_NEEDS_WINDOW: &str = "needs the OpenLaser window (bridge mode)";
pub(crate) const HUMAN_ONLY_ROW: &str = "human-only: started by a person in the OpenLaser window";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRow {
    pub id: String,
    pub label: String,
    pub menu: Vec<String>,
    pub shortcut: Option<String>,
    pub default_shortcut: Option<String>,
    pub kind: String,
    pub agent: AgentClass,
    pub params: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub needs_document: bool,
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentClass {
    Allowed,
    RootsOnly,
    Ui,
    HumanOnly,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListFilter {
    pub filter: Option<String>,
    pub enabled_only: bool,
    pub agent: Option<AgentClass>,
    pub schema: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("flip", "flip"), 0);
    }

    #[test]
    fn suggestions_find_near_ids() {
        let r = registry();
        let s = r.suggestions("edit.undoo");
        assert!(s.contains(&"edit.undo"), "{s:?}");
        let s = r.suggestions("layer.cutSetings.set");
        assert!(s.contains(&"layer.cutSettings.set"), "{s:?}");
        assert!(r.suggestions("").is_empty());
    }

    #[test]
    fn error_codes_and_prefixes() {
        let e = EngineError::human_only("laser.start");
        assert_eq!(e.code(), "human_only");
        assert!(e.agent_message().starts_with("human-only: "), "{}", e.agent_message());
        assert_eq!(EngineError::refused(RefusalCode::MachineBusy, "x").agent_message(), "machine_busy: x");
        assert!(EngineError::NeedsBridge.agent_message().starts_with("needs_bridge: "));
        let j = EngineError::bad("a.b", "/x", "nope").to_json();
        assert_eq!(j["pointer"], "/x");
        assert_eq!(j["code"], "bad_params");
    }

    #[test]
    fn kind_strings_and_journal() {
        assert_eq!(Kind::Device(DeviceClass::Stop).as_str(), "device");
        assert!(Kind::Edit.journaled() && Kind::Io.journaled());
        assert!(!Kind::Device(DeviceClass::Emit).journaled() && !Kind::Query.journaled() && !Kind::View.journaled());
    }
}

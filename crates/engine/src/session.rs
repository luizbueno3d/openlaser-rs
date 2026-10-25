//! The session: documents, history, dispatch, batches and the command list (SPEC §3.16, ENG §2.4, §2.8).
//!
//! The job pipeline lives in `jobs.rs`, the device hub and the ARM path in `devices.rs`, the agent requests in
//! `requests.rs`; they are `impl Session` blocks over the fields declared here.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use openlaser_doc::{DeviceHint, Project, Selection, Workspace, WorkspaceOrigin};
use openlaser_machine::{Kinematics, MachineProfile, ProfileStore};
use openlaser_types::{Clock, DeviceKind, LayerKey, MachineId, SessionId};
use serde::Serialize;
use serde_json::Value;

use crate::command::{Access, CommandRow, EngineError, HUMAN_ONLY_ROW, Kind, ListFilter, Origin, policy, registry};
use crate::devices::{PendingArm, Slot, StopRegistry};
use crate::files::{FS_DENIED, FsAuthority};
use crate::jobs::LiveCheck;
use crate::prefs::Prefs;
use crate::requests::Requests;

pub use crate::devices::{CachedStatus, DeviceAccess, Event};
pub use crate::files::FileAccess;
pub use crate::jobs::{BuiltJob, Encoded, JobOptions, JobRefused};

/// The most steps one `batch` call runs (MCP `command_batch`, §4.1).
pub const MAX_BATCH_STEPS: usize = 256;
/// Journal entries kept (oldest dropped first).
pub const MAX_JOURNAL: usize = 10_000;
/// Events kept between two `poll` calls (oldest dropped first; driver events are folded on every poll).
pub const MAX_EVENTS: usize = 10_000;

pub struct SessionConfig {
    /// Default origin of execute(): Human (GUI, openlaser-hw), Headless (CLI/MCP).
    pub origin: Origin,
    /// A GUI hosts this session (bridge requests possible).
    pub window: bool,
    /// None in openlaser-cli; Sim (+ Usb with `hw`) in the GUI and openlaser-hw.
    pub devices: DeviceAccess,
    /// format::machines::load_all (built-ins shadowed by saved profiles).
    pub machines: Vec<MachineProfile>,
    /// S24: Some only in the GUI and openlaser-hw. `device.settings.set`/`create`/`importEzcad`/`calibrateAxis`
    /// write through it and fail with NoDevice-style "profiles are read-only in this session" when None.
    pub profiles: Option<Arc<dyn ProfileStore>>,
    pub files: FsAuthority,
    pub clock: Arc<dyn Clock>,
}

/// One journaled command (Edit and Io only, never Device): enough to replay it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub id: String,
    pub params: Value,
}

/// What the running command was called with (set by dispatch, read by the typed command functions).
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    pub origin: Origin,
    /// The origin a Replay step inherits from its caller; equals `origin` otherwise.
    pub effective: Origin,
    pub coalesce: Option<String>,
    pub dry_run: bool,
}

/// An atomic batch in progress: edits record no history of their own; one state is recorded at the end.
pub(crate) struct Group;

/// The app-wide clipboard (`edit.copy`/`edit.cut`): deep copies of the top-level targets plus their combined
/// world bounds, shared by the GUI, CLI and MCP because it lives on the Session, not the OS clipboard. Not part
/// of a document, so undo and `file.new`/`file.open` never touch it.
#[derive(Debug, Clone)]
pub(crate) struct Clipboard {
    pub shapes: Vec<Arc<openlaser_doc::Shape>>,
    pub bounds: Option<openlaser_types::Bounds>,
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    docs: Vec<DocState>,
    active: Option<usize>,
    prefs: Prefs,
}

pub struct Session {
    pub(crate) cfg: SessionConfig,
    pub(crate) id: SessionId,
    pub(crate) docs: Vec<DocState>,
    pub(crate) active: Option<usize>,
    pub(crate) prefs: Prefs,
    /// The lazily-built font index for text layout; tests point it at a fixture dir.
    fonts: openlaser_text::FontLibrary,
    pub(crate) journal: VecDeque<JournalEntry>,
    pub(crate) events: VecDeque<Event>,
    pub(crate) hub: Vec<Slot>,
    pub(crate) stop_registry: Arc<StopRegistry>,
    pub(crate) seen_stop_trips: u64,
    pub(crate) pending_arm: Option<PendingArm>,
    /// The document the pending ARM was built from.
    pub(crate) pending_doc: Option<usize>,
    pub(crate) requests: Requests,
    pub(crate) live: Option<LiveCheck>,
    pub(crate) live_error: Option<String>,
    pub(crate) live_key: Option<(usize, u64, MachineId)>,
    pub(crate) ctx: Option<Ctx>,
    pub(crate) group: Option<Group>,
    pub(crate) clipboard: Option<Clipboard>,
}

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

impl Session {
    pub fn new(cfg: SessionConfig) -> Self {
        let id = SessionId(NEXT_SESSION.fetch_add(1, Ordering::Relaxed));
        Session {
            cfg,
            id,
            docs: Vec::new(),
            active: None,
            prefs: Prefs::default(),
            fonts: openlaser_text::FontLibrary::new(),
            journal: VecDeque::new(),
            events: VecDeque::new(),
            hub: Vec::new(),
            stop_registry: Arc::new(StopRegistry::default()),
            seen_stop_trips: 0,
            pending_arm: None,
            pending_doc: None,
            requests: Requests::default(),
            live: None,
            live_error: None,
            live_key: None,
            ctx: None,
            group: None,
            clipboard: None,
        }
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    /// The default origin of `execute` (SessionConfig).
    pub fn origin(&self) -> Origin {
        self.cfg.origin
    }

    /// A GUI hosts this session (bridge requests possible).
    pub fn has_window(&self) -> bool {
        self.cfg.window
    }

    // ---- dispatch (pipeline: SPEC §3.16) --------------------------------------------------------------------
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value, EngineError> {
        let origin = self.cfg.origin;
        self.execute_as(origin, id, params)
    }

    pub fn execute_as(&mut self, origin: Origin, id: &str, params: Value) -> Result<Value, EngineError> {
        self.dispatch(origin, None, id, params)
    }

    /// The pipeline. `parent` is the origin a `Replay` step inherits (the batch caller's).
    fn dispatch(&mut self, origin: Origin, parent: Option<Origin>, id: &str, params: Value) -> Result<Value, EngineError> {
        // 1. find (exact, then snake alias) or UnknownCommand with suggestions.
        let reg = registry();
        let Some(spec) = reg.find(id).copied() else {
            return Err(EngineError::UnknownCommand { id: id.to_owned(), suggestions: reg.suggestions(id).into_iter().map(str::to_owned).collect() });
        };
        self.run_spec(spec, origin, parent, params)
    }

    /// Pipeline steps 2–9 for one resolved spec.
    pub(crate) fn run_spec(&mut self, spec: crate::command::CommandSpec, origin: Origin, parent: Option<Origin>, params: Value) -> Result<Value, EngineError> {
        let effective = if origin == Origin::Replay { parent.unwrap_or(Origin::Replay) } else { origin };
        if spec.access == Access::Stop {
            return self.run_stop_spec(spec, origin, effective, params);
        }
        // 2. params is an object or null; split the universal keys.
        let (params, coalesce, dry_run) = split_universal(spec.id, params)?;
        // 3. policy, before any decode or side effect.
        policy(&spec, origin, effective, self.cfg.window)?;
        // 4. roots: a non-human Roots command needs filesystem authority.
        if spec.access == Access::Roots && effective != Origin::Human && matches!(self.cfg.files, FsAuthority::Denied) {
            return Err(EngineError::Io(FS_DENIED.to_owned()));
        }
        if dry_run && !matches!(spec.kind, Kind::Edit | Kind::View | Kind::Query) {
            return Err(EngineError::bad(spec.id, "/dryRun", "dryRun works for edit, view and query commands only"));
        }
        let journal_params = (spec.kind.journaled() && !dry_run).then(|| params.clone());
        let snapshot = (dry_run || !matches!(spec.kind, Kind::Query | Kind::Device(_))).then(|| self.snapshot());
        // 5–7 run inside `run` (decode, enabled, busy); 8: catch_unwind.
        let prev = self.ctx.replace(Ctx { origin, effective, coalesce, dry_run });
        let run = spec.run;
        let outcome = catch_unwind(AssertUnwindSafe(|| run(self, params)));
        self.ctx = prev;
        let result = match outcome {
            Ok(r) => {
                if dry_run && let Some(s) = snapshot {
                    self.restore(s, false);
                }
                r
            }
            Err(_) => {
                log::error!("{}: panicked; the project was restored", spec.id);
                if let Some(s) = snapshot {
                    self.restore(s, true);
                }
                Err(EngineError::Internal(spec.id.to_owned()))
            }
        };
        // 9. journal (Edit/Io only; never Device). Revision bumps and events happen in `edit` and friends.
        if result.is_ok()
            && let Some(p) = journal_params
        {
            if self.journal.len() >= MAX_JOURNAL {
                self.journal.pop_front();
            }
            self.journal.push_back(JournalEntry { id: spec.id.to_owned(), params: p });
        }
        result
    }

    /// A Stop command is never refused, never disabled and never journaled (§1.3 rule 5): no policy, no split of the
    /// universal keys, no decode here. Its `run` gets the raw params and trips the latches before it looks at them.
    /// If it panics anyway, every latch is tripped before the error is returned.
    fn run_stop_spec(&mut self, spec: crate::command::CommandSpec, origin: Origin, effective: Origin, params: Value) -> Result<Value, EngineError> {
        let prev = self.ctx.replace(Ctx { origin, effective, coalesce: None, dry_run: false });
        let run = spec.run;
        let outcome = catch_unwind(AssertUnwindSafe(|| run(self, params)));
        self.ctx = prev;
        match outcome {
            Ok(r) => r,
            Err(_) => {
                log::error!("{}: panicked; every machine is being stopped", spec.id);
                let registry = Arc::clone(&self.stop_registry);
                if catch_unwind(AssertUnwindSafe(|| registry.stop_all(crate::devices::STOP_WAIT))).is_err() {
                    log::error!("{}: stop_all panicked too", spec.id);
                }
                self.void_pending_arm("stopped");
                Err(EngineError::Internal(spec.id.to_owned()))
            }
        }
    }

    /// Steps 6 and 7, called by every `command!` `run` after the strict decode: `enabled` (never for Stop commands)
    /// and the device-busy refusal for `Device(Light | Emit | Connect)`.
    pub(crate) fn pre_run(&mut self, id: &str) -> Result<(), EngineError> {
        let Some(spec) = registry().find(id).copied() else { return Ok(()) };
        if spec.access == Access::Stop {
            return Ok(());
        }
        (spec.enabled)(self).map_err(|d| EngineError::Disabled { id: spec.id.to_owned(), reason: d.reason, needs_document: d.needs_document })?;
        if let Kind::Device(class) = spec.kind
            && class != crate::command::DeviceClass::Stop
            && let Some(m) = self.busy_machine()
        {
            return Err(EngineError::refused(openlaser_types::RefusalCode::MachineBusy, format!("{} is framing or running; stop it first", m.0)));
        }
        Ok(())
    }

    /// Non-atomic by default (PhotoCraft); `atomic: Some(label)` = one undo step, rolled back on any error;
    /// device commands are refused inside an atomic batch.
    pub fn batch(&mut self, origin: Origin, steps: Vec<(String, Value)>, stop_on_error: bool, atomic: Option<String>) -> BatchResult {
        if steps.len() > MAX_BATCH_STEPS {
            let e = EngineError::bad("command_batch", "/steps", format!("{} steps is more than the {MAX_BATCH_STEPS} a batch may hold", steps.len()));
            return BatchResult { completed: 0, failed: 1, rolled_back: false, results: vec![step_error(&e)] };
        }
        match atomic {
            None => {
                let mut out = BatchResult { completed: 0, failed: 0, rolled_back: false, results: Vec::with_capacity(steps.len()) };
                let mut steps = steps.into_iter().enumerate();
                while let Some((_, (id, params))) = steps.next() {
                    match self.dispatch(Origin::Replay, Some(origin), &id, params) {
                        Ok(v) => {
                            out.completed += 1;
                            out.results.push(serde_json::json!({ "ok": true, "result": v }));
                        }
                        Err(e) => {
                            out.failed += 1;
                            out.results.push(step_error(&e));
                            if stop_on_error {
                                self.run_skipped_stops(origin, steps, &mut out);
                                break;
                            }
                        }
                    }
                }
                out
            }
            Some(label) => self.atomic_batch(origin, steps, label),
        }
    }

    /// A Stop step is never skipped (§1.3 rule 5: a stop is never queued behind another request): when a batch ends
    /// early (a failed step with `stop_on_error`, a rolled-back or refused atomic batch), the Stop steps it did not
    /// reach still run, in order. Their results are appended with their `step` index.
    fn run_skipped_stops(&mut self, origin: Origin, rest: impl Iterator<Item = (usize, (String, Value))>, out: &mut BatchResult) {
        let reg = registry();
        for (i, (id, params)) in rest {
            if reg.find(&id).is_none_or(|s| s.access != Access::Stop) {
                continue;
            }
            match self.dispatch(Origin::Replay, Some(origin), &id, params) {
                Ok(v) => {
                    out.completed += 1;
                    out.results.push(serde_json::json!({ "ok": true, "step": i, "result": v }));
                }
                Err(e) => {
                    out.failed += 1;
                    out.results.push(serde_json::json!({ "ok": false, "step": i, "error": e.to_json() }));
                }
            }
        }
    }

    fn atomic_batch(&mut self, origin: Origin, steps: Vec<(String, Value)>, label: String) -> BatchResult {
        let reg = registry();
        let mut refusal = None;
        for (id, _) in &steps {
            refusal = match reg.find(id) {
                None => Some(EngineError::UnknownCommand { id: id.clone(), suggestions: reg.suggestions(id).into_iter().map(str::to_owned).collect() }),
                // A stop is never refused: it runs in its place (rolling the batch back never un-stops anything).
                Some(s) if s.access == Access::Stop => None,
                Some(s) if matches!(s.kind, Kind::Device(_) | Kind::Io | Kind::Request) => Some(EngineError::refused(
                    openlaser_types::RefusalCode::UnsupportedMode,
                    format!("`{}` cannot run inside an atomic batch (device, file and request commands are refused there)", s.id),
                )),
                Some(_) => None,
            };
            if refusal.is_some() {
                break;
            }
        }
        if refusal.is_none() && self.group.is_some() {
            refusal = Some(EngineError::disabled("command_batch", "atomic batches do not nest"));
        }
        if let Some(e) = refusal {
            let mut out = BatchResult { completed: 0, failed: 1, rolled_back: false, results: vec![step_error(&e)] };
            self.run_skipped_stops(origin, steps.into_iter().enumerate(), &mut out);
            return out;
        }
        let base = self.snapshot();
        self.group = Some(Group);
        let mut out = BatchResult { completed: 0, failed: 0, rolled_back: false, results: Vec::with_capacity(steps.len()) };
        let mut steps = steps.into_iter().enumerate();
        for (_, (id, params)) in steps.by_ref() {
            match self.dispatch(Origin::Replay, Some(origin), &id, params) {
                Ok(v) => {
                    out.completed += 1;
                    out.results.push(serde_json::json!({ "ok": true, "result": v }));
                }
                Err(e) => {
                    out.failed += 1;
                    out.results.push(step_error(&e));
                    out.rolled_back = true;
                    break;
                }
            }
        }
        self.group = None;
        if out.rolled_back {
            self.restore(base, true);
            self.run_skipped_stops(origin, steps, &mut out);
            return out;
        }
        // One history state per changed document, labelled with the batch label.
        for (i, doc) in self.docs.iter_mut().enumerate() {
            let Some(b) = base.docs.get(i) else { continue };
            if *doc.project != *b.project {
                let state = HistoryState { label: Label::dynamic(&label), project: Arc::clone(&b.project), selection: b.selection.clone() };
                doc.history.record(state, None);
            }
        }
        out
    }

    pub fn command_list(&self, origin: Origin, f: &ListFilter) -> Vec<CommandRow> {
        let needle = f.filter.as_ref().map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
        let mut rows = Vec::new();
        for spec in registry().specs() {
            let class = spec.access.agent_class();
            if f.agent.is_some_and(|a| a != class) {
                continue;
            }
            if let Some(n) = &needle
                && !spec.id.to_lowercase().contains(n.as_str())
                && !spec.label.to_lowercase().contains(n.as_str())
            {
                continue;
            }
            let (enabled, reason, needs_document) = self.row_state(spec, origin);
            if f.enabled_only && !enabled {
                continue;
            }
            let schema = if f.schema { serde_json::to_value((spec.schema)()).ok() } else { None };
            rows.push(CommandRow {
                id: spec.id.to_owned(),
                label: spec.label.to_owned(),
                menu: spec.menu.iter().map(|s| (*s).to_owned()).collect(),
                shortcut: self.prefs.shortcut(spec),
                default_shortcut: spec.shortcut.map(str::to_owned),
                kind: spec.kind.as_str().to_owned(),
                agent: class,
                params: spec.doc.to_owned(),
                enabled,
                disabled_reason: reason,
                needs_document,
                schema,
            });
        }
        rows
    }

    /// (enabled, disabled reason, needs document) of one row for `origin`.
    fn row_state(&self, spec: &crate::command::CommandSpec, origin: Origin) -> (bool, Option<String>, bool) {
        if spec.access == Access::Stop {
            return (true, None, false);
        }
        let effective = if origin == Origin::Replay { Origin::Replay } else { origin };
        match policy(spec, origin, effective, self.cfg.window) {
            Ok(()) => {}
            Err(EngineError::Refused(_)) => return (false, Some(HUMAN_ONLY_ROW.to_owned()), false),
            Err(EngineError::Disabled { reason, needs_document, .. }) => return (false, Some(reason), needs_document),
            Err(e) => return (false, Some(e.to_string()), false),
        }
        match (spec.enabled)(self) {
            Ok(()) => (true, None, false),
            Err(d) => (false, Some(d.reason), d.needs_document),
        }
    }

    /// `{id, schema}` of one command (exact id or snake alias).
    pub fn command_schema(&self, id: &str) -> Result<Value, EngineError> {
        let reg = registry();
        let spec = reg
            .find(id)
            .ok_or_else(|| EngineError::UnknownCommand { id: id.to_owned(), suggestions: reg.suggestions(id).into_iter().map(str::to_owned).collect() })?;
        let schema = serde_json::to_value((spec.schema)()).map_err(|_| EngineError::Internal(spec.id.to_owned()))?;
        Ok(serde_json::json!({ "id": spec.id, "schema": schema }))
    }

    /// Journaled commands (Edit and Io only, never Device), oldest first.
    pub fn journal(&self) -> Vec<JournalEntry> {
        self.journal.iter().cloned().collect()
    }

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    /// The font index (built on first use).
    pub fn fonts(&self) -> &openlaser_text::FontLibrary {
        &self.fonts
    }

    /// Swap in a differently-rooted index (tests inject the committed fixture; OPENLASER_FONT_DIRS is already
    /// honoured by `FontLibrary::new`).
    pub fn set_font_dirs(&mut self, dirs: Vec<std::path::PathBuf>) {
        self.fonts = openlaser_text::FontLibrary::with_dirs(dirs);
    }

    // ---- documents ------------------------------------------------------------------------------------------
    pub fn docs(&self) -> &[DocState] {
        &self.docs
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// Adds a document and makes it active. `machine` None = the project's device hint if that profile exists, else
    /// the session's first machine. Refused inside an atomic batch (document indices must stay stable there).
    pub fn open_project(&mut self, project: Project, path: Option<String>, machine: Option<MachineId>) -> Result<usize, EngineError> {
        if self.group.is_some() {
            return Err(EngineError::disabled("file.open", "documents cannot be opened or closed inside an atomic batch"));
        }
        let machine = match machine {
            Some(m) => {
                if self.profile(&m).is_none() {
                    return Err(EngineError::bad("file.open", "/machine", format!("no machine profile `{}`", m.0)));
                }
                m
            }
            None => project
                .device_hint
                .as_ref()
                .map(|h| h.profile.clone())
                .filter(|m| self.profile(m).is_some())
                .or_else(|| self.default_machine())
                .ok_or_else(|| EngineError::disabled("file.open", "this session has no machine profiles"))?,
        };
        if self.dry_run() {
            return Ok(self.docs.len());
        }
        let max_states = self.prefs.max_states();
        let doc = DocState {
            project: Arc::new(project),
            history: History::with_max(max_states),
            selection: Selection::default(),
            current_layer: LayerKey::Color(0),
            path,
            machine,
            revision: 1,
            saved_revision: 1,
        };
        self.docs.push(doc);
        let index = self.docs.len() - 1;
        self.set_active(Some(index));
        if let Some(d) = self.docs.get(index) {
            self.push_event(Event::DocChanged { index, revision: d.revision });
        }
        Ok(index)
    }

    /// Makes document `index` the active one. Voids a pending ARM (it was built from another document).
    pub fn select_doc(&mut self, index: usize) -> Result<(), EngineError> {
        if index >= self.docs.len() {
            return Err(EngineError::bad("document.select", "/index", format!("no document {index} ({} open)", self.docs.len())));
        }
        if self.active != Some(index) {
            self.set_active(Some(index));
        }
        Ok(())
    }

    /// Closes document `index` (None = the active one). Requests made from it become stale.
    pub fn close_doc(&mut self, index: Option<usize>) -> Result<(), EngineError> {
        if self.group.is_some() {
            return Err(EngineError::disabled("document.close", "documents cannot be opened or closed inside an atomic batch"));
        }
        let index = index.or(self.active).ok_or(EngineError::NoDocument)?;
        if index >= self.docs.len() {
            return Err(EngineError::bad("document.close", "/index", format!("no document {index} ({} open)", self.docs.len())));
        }
        if self.dry_run() {
            return Ok(());
        }
        self.void_pending_arm("a document was closed");
        let staled = self.requests.close_doc(index);
        for (id, state) in staled {
            self.push_event(Event::Request { id, state });
        }
        self.docs.remove(index);
        let active = match self.active {
            Some(a) if a == index => {
                if self.docs.is_empty() {
                    None
                } else {
                    Some(index.min(self.docs.len() - 1))
                }
            }
            Some(a) if a > index => Some(a - 1),
            other => other,
        };
        self.active = active;
        self.live = None;
        self.live_key = None;
        Ok(())
    }

    fn set_active(&mut self, index: Option<usize>) {
        if self.active != index {
            self.void_pending_arm("another document became active");
        }
        self.active = index;
    }

    /// The one way a document changes: runs `f` on a copy; on Ok swaps it in and records one history state
    /// (or merges into the previous one when `coalesce` matches); on Err nothing changes. Bumps the revision,
    /// voids PendingArm, marks submissions/frame requests stale, invalidates the live check.
    pub fn edit<R>(
        &mut self,
        label: Label,
        coalesce: Option<&str>,
        f: impl FnOnce(&mut Project, &mut Selection) -> Result<R, EngineError>,
    ) -> Result<R, EngineError> {
        let index = self.active.ok_or(EngineError::NoDocument)?;
        let (mut project, mut selection) = {
            let doc = self.docs.get(index).ok_or(EngineError::NoDocument)?;
            ((*doc.project).clone(), doc.selection.clone())
        };
        let out = f(&mut project, &mut selection)?;
        project.normalize();
        selection.ids.retain(|id| project.shape(*id).is_some());
        let mut seen = std::collections::HashSet::new();
        selection.ids.retain(|id| seen.insert(*id));
        if selection.key.is_some_and(|k| !selection.ids.contains(&k)) {
            selection.key = selection.ids.last().copied();
        }
        if self.dry_run() {
            return Ok(out);
        }
        let in_group = self.group.is_some();
        let Some(doc) = self.docs.get_mut(index) else { return Err(EngineError::NoDocument) };
        if project == *doc.project {
            self.set_selection_of(index, selection)?;
            return Ok(out);
        }
        if !in_group {
            let before = HistoryState { label, project: Arc::clone(&doc.project), selection: doc.selection.clone() };
            doc.history.record(before, coalesce);
        }
        doc.project = Arc::new(project);
        doc.selection = selection;
        doc.revision = doc.revision.saturating_add(1);
        self.after_change(index, "the document changed");
        Ok(out)
    }

    /// Replaces document `index`'s selection. With Cut Selected Graphics on, the selection decides what `build_job`
    /// produces, so a change of the selected ids is a revision bump (§3.16): PendingArm voided, requests stale, live
    /// check invalidated. Selections are not saved, so the bump never marks a clean document dirty.
    pub(crate) fn set_selection_of(&mut self, index: usize, selection: Selection) -> Result<(), EngineError> {
        let dry = self.dry_run();
        let Some(doc) = self.docs.get_mut(index) else { return Err(EngineError::NoDocument) };
        let job_changes = doc.project.job.cut_selected && doc.selection.ids != selection.ids;
        doc.selection = selection;
        if job_changes && !dry {
            let clean = !doc.dirty();
            doc.revision = doc.revision.saturating_add(1);
            if clean {
                doc.saved_revision = doc.revision;
            }
            self.after_change(index, "the selection changed while Cut Selected Graphics is on");
        }
        Ok(())
    }

    pub fn undo(&mut self) -> Result<(), EngineError> {
        self.step_history(true)
    }

    pub fn redo(&mut self) -> Result<(), EngineError> {
        self.step_history(false)
    }

    fn step_history(&mut self, undo: bool) -> Result<(), EngineError> {
        let id = if undo { "edit.undo" } else { "edit.redo" };
        if self.group.is_some() {
            return Err(EngineError::disabled(id, "undo and redo are not available inside an atomic batch"));
        }
        let index = self.active.ok_or(EngineError::NoDocument)?;
        let dry = self.dry_run();
        let Some(doc) = self.docs.get_mut(index) else { return Err(EngineError::NoDocument) };
        let available = if undo { doc.history.can_undo() } else { doc.history.can_redo() };
        if !available {
            return Err(EngineError::disabled(id, if undo { "nothing to undo" } else { "nothing to redo" }));
        }
        if dry {
            return Ok(());
        }
        let target = if undo { doc.history.undo.pop_back() } else { doc.history.redo.pop() };
        let Some(target) = target else { return Err(EngineError::disabled(id, "nothing to step to")) };
        let current = HistoryState { label: target.label.clone(), project: Arc::clone(&doc.project), selection: doc.selection.clone() };
        if undo {
            doc.history.redo.push(current);
        } else {
            doc.history.undo.push_back(current);
        }
        doc.history.last_coalesce = None;
        doc.project = target.project;
        doc.selection = target.selection;
        doc.revision = doc.revision.saturating_add(1);
        self.after_change(index, if undo { "undo" } else { "redo" });
        Ok(())
    }

    // ---- machines -------------------------------------------------------------------------------------------
    pub fn machines(&self) -> &[MachineProfile] {
        &self.cfg.machines
    }

    pub fn machine_for(&self, index: Option<usize>) -> Option<&MachineProfile> {
        let i = index.or(self.active)?;
        let doc = self.docs.get(i)?;
        self.profile(&doc.machine)
    }

    pub(crate) fn profile(&self, id: &MachineId) -> Option<&MachineProfile> {
        self.cfg.machines.iter().find(|m| &m.id == id)
    }

    pub(crate) fn default_machine(&self) -> Option<MachineId> {
        self.cfg.machines.first().map(|m| m.id.clone())
    }

    // ---- internals shared by the command modules ---------------------------------------------------------------
    pub(crate) fn dry_run(&self) -> bool {
        self.ctx.as_ref().is_some_and(|c| c.dry_run)
    }

    /// The origin the running command was dispatched with (a Replay step reports Replay). A command `run` called
    /// directly, outside dispatch, skipped the policy table: it gets `Headless` (fail closed), so no human-only path
    /// opens for it whatever the session's default origin is.
    pub(crate) fn current_origin(&self) -> Origin {
        self.ctx.as_ref().map_or(Origin::Headless, |c| c.origin)
    }

    /// The origin whose rights apply (a Replay step inherits its caller's); `Headless` outside dispatch.
    pub(crate) fn effective_origin(&self) -> Origin {
        self.ctx.as_ref().map_or(Origin::Headless, |c| c.effective)
    }

    pub(crate) fn coalesce(&self) -> Option<String> {
        self.ctx.as_ref().and_then(|c| c.coalesce.clone())
    }

    pub(crate) fn doc_index(&self) -> Result<usize, EngineError> {
        self.active.filter(|i| *i < self.docs.len()).ok_or(EngineError::NoDocument)
    }

    pub(crate) fn doc(&self) -> Result<&DocState, EngineError> {
        self.active.and_then(|i| self.docs.get(i)).ok_or(EngineError::NoDocument)
    }

    pub(crate) fn doc_mut(&mut self) -> Result<&mut DocState, EngineError> {
        let i = self.active.ok_or(EngineError::NoDocument)?;
        self.docs.get_mut(i).ok_or(EngineError::NoDocument)
    }

    pub(crate) fn push_event(&mut self, e: Event) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(e);
    }

    /// Everything a revision bump of document `index` implies: PendingArm voided, requests stale, live check
    /// invalidated, `DocChanged` raised.
    pub(crate) fn after_change(&mut self, index: usize, why: &str) {
        self.void_pending_arm(why);
        let staled = self.requests.mark_stale(index);
        for (id, state) in staled {
            self.push_event(Event::Request { id, state });
        }
        self.live = None;
        self.live_key = None;
        let revision = self.docs.get(index).map_or(0, |d| d.revision);
        self.push_event(Event::DocChanged { index, revision });
    }

    /// Bumps the revision of every document whose plan reads `machine` (None = every document).
    pub(crate) fn bump_docs(&mut self, machine: Option<&MachineId>, why: &str) {
        let targets: Vec<usize> = self.docs.iter().enumerate().filter(|(_, d)| machine.is_none_or(|m| &d.machine == m)).map(|(i, _)| i).collect();
        for i in targets {
            if let Some(d) = self.docs.get_mut(i) {
                d.revision = d.revision.saturating_add(1);
            }
            self.after_change(i, why);
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot { docs: self.docs.clone(), active: self.active, prefs: self.prefs.clone() }
    }

    /// Puts the documents back. `bump`: also move every revision past anything handed out meanwhile, so caches,
    /// pending ARMs and requests keyed by revision can never match the restored state by accident.
    pub(crate) fn restore(&mut self, snap: Snapshot, bump: bool) {
        let current: Vec<u64> = self.docs.iter().map(|d| d.revision).collect();
        let changed = snap.docs.len() != self.docs.len() || snap.docs.iter().zip(self.docs.iter()).any(|(a, b)| a.revision != b.revision);
        self.docs = snap.docs;
        self.active = snap.active;
        self.prefs = snap.prefs;
        if bump || changed {
            let n = self.docs.len();
            for (i, d) in self.docs.iter_mut().enumerate() {
                let seen = current.get(i).copied().unwrap_or(0);
                d.revision = d.revision.max(seen).saturating_add(1);
            }
            for i in 0..n {
                self.after_change(i, "the document was restored");
            }
        }
    }

    /// A workspace snapshot for a new project on `m` (display and import only).
    pub(crate) fn workspace_for(m: &MachineProfile) -> Workspace {
        match &m.kind {
            Kinematics::Galvo(g) => {
                let w = g.work_area;
                Workspace { kind: DeviceKind::GalvoLmc, size_mm: [w.max[0] - w.min[0], w.max[1] - w.min[1]], origin: WorkspaceOrigin::Center }
            }
            Kinematics::Gantry(g) => Workspace { kind: DeviceKind::Grbl, size_mm: g.bed_mm, origin: g.origin },
        }
    }

    /// A new, empty project for `m` (`file.new`).
    pub(crate) fn new_project(m: &MachineProfile, name: &str) -> Project {
        let mut p = Project::new(Self::workspace_for(m), &m.layer_defaults);
        p.name = name.to_owned();
        let kind = match m.kind {
            Kinematics::Galvo(_) => DeviceKind::GalvoLmc,
            Kinematics::Gantry(_) => DeviceKind::Grbl,
        };
        p.device_hint = Some(DeviceHint { kind, profile: m.id.clone() });
        p
    }
}

/// Splits the universal keys off the params (pipeline step 2).
pub(crate) fn split_universal(id: &str, params: Value) -> Result<(Value, Option<String>, bool), EngineError> {
    match params {
        Value::Null => Ok((Value::Null, None, false)),
        Value::Object(mut m) => {
            let coalesce = match m.remove("coalesce") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s.chars().count() <= 256 => Some(s),
                Some(Value::String(_)) => return Err(EngineError::bad(id, "/coalesce", "coalesce keys are at most 256 characters")),
                Some(_) => return Err(EngineError::bad(id, "/coalesce", "coalesce must be a string")),
            };
            let dry = match (m.remove("dryRun"), m.remove("dry_run")) {
                (Some(_), Some(_)) => return Err(EngineError::bad(id, "/dryRun", "`dry_run` is given twice (as snake_case and camelCase)")),
                (Some(v), None) | (None, Some(v)) => match v {
                    Value::Bool(b) => b,
                    Value::Null => false,
                    _ => return Err(EngineError::bad(id, "/dryRun", "dryRun must be true or false")),
                },
                (None, None) => false,
            };
            Ok((Value::Object(m), coalesce, dry))
        }
        other => Err(EngineError::bad(id, "", format!("params must be a JSON object or null, not {}", crate::params::type_name(&other)))),
    }
}

fn step_error(e: &EngineError) -> Value {
    serde_json::json!({ "ok": false, "error": e.to_json() })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult {
    pub completed: usize,
    pub failed: usize,
    pub rolled_back: bool,
    pub results: Vec<Value>,
}

#[derive(Clone)]
pub struct DocState {
    pub project: Arc<Project>,
    pub history: History,
    pub selection: Selection,
    pub current_layer: LayerKey,
    pub path: Option<String>,
    pub machine: MachineId,
    pub revision: u64,
    pub saved_revision: u64,
}

impl DocState {
    /// Unsaved changes.
    pub fn dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// The name shown for the document: the file name, else the project name, else "Untitled".
    pub fn display_name(&self) -> String {
        if let Some(p) = &self.path
            && let Some(name) = std::path::Path::new(p).file_name()
        {
            return name.to_string_lossy().into_owned();
        }
        if self.project.name.is_empty() { "Untitled".to_owned() } else { self.project.name.clone() }
    }
}

/// undo: VecDeque<HistoryState>, redo: Vec<HistoryState>, max_states (default 100).
#[derive(Clone)]
pub struct History {
    pub(crate) undo: VecDeque<HistoryState>,
    pub(crate) redo: Vec<HistoryState>,
    max_states: usize,
    pub(crate) last_coalesce: Option<String>,
}

impl Default for History {
    fn default() -> Self {
        History::with_max(100)
    }
}

impl History {
    pub fn with_max(max_states: usize) -> Self {
        History { undo: VecDeque::new(), redo: Vec::new(), max_states: max_states.max(1), last_coalesce: None }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undo labels, oldest first (doc_inspect `history`).
    pub fn labels(&self) -> Vec<String> {
        self.undo.iter().map(|s| s.label.to_string()).collect()
    }

    /// Redo labels, next redo first.
    pub fn redo_labels(&self) -> Vec<String> {
        self.redo.iter().rev().map(|s| s.label.to_string()).collect()
    }

    pub fn max_states(&self) -> usize {
        self.max_states
    }

    pub(crate) fn set_max(&mut self, max: usize) {
        self.max_states = max.max(1);
        while self.undo.len() > self.max_states {
            self.undo.pop_front();
        }
    }

    /// Records the state before an edit. A matching `coalesce` key merges into the previous step (the older state
    /// stays the undo target); anything else starts a new step. The redo stack is cleared either way.
    pub(crate) fn record(&mut self, before: HistoryState, coalesce: Option<&str>) {
        self.redo.clear();
        let merge = coalesce.is_some() && self.last_coalesce.as_deref() == coalesce && self.can_undo();
        if !merge {
            self.undo.push_back(before);
            while self.undo.len() > self.max_states {
                self.undo.pop_front();
            }
        }
        self.last_coalesce = coalesce.map(str::to_owned);
    }
}

#[derive(Clone)]
pub struct HistoryState {
    pub label: Label,
    pub project: Arc<Project>,
    pub selection: Selection,
}

/// "Set {name} Frequency".
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub wording: &'static str,
    pub value: Option<String>,
}

impl Label {
    pub fn new(wording: &'static str) -> Self {
        Label { wording, value: None }
    }

    /// `wording` with `{name}` replaced by `value` when shown.
    pub fn with(wording: &'static str, value: impl Into<String>) -> Self {
        Label { wording, value: Some(value.into()) }
    }

    /// A label whose whole text is runtime data (an atomic batch's label).
    pub fn dynamic(text: &str) -> Self {
        Label { wording: "{name}", value: Some(text.to_owned()) }
    }
}

impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.value {
            Some(v) => f.write_str(&self.wording.replace("{name}", v)),
            None => f.write_str(self.wording),
        }
    }
}

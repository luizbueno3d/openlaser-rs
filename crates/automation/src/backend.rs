//! The headless engine backend and the GUI bridge backend.
//!
//! PNG results are typed (`ToolOutput::Png`) from the moment they are rendered; nothing inspects JSON for a magic
//! key. On the control wire (the GUI's methods and `openlaser-cli serve`) a PNG travels as the result object with a
//! `pngBase64` field next to the note's fields, for exactly the methods that render: `doc.render`, `job.preview`,
//! `job.simulate` and `ui.screenshot`.

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use openlaser_control::budgets;
use openlaser_control::workspace::AuthorizedWorkspace;
use openlaser_engine::{AgentClass, EngineError, Origin, Session};
use serde_json::{Map, Value, json};

use crate::bridge::{BridgeClient, Link, StopHandle};
use crate::tools::{ClientInfo, Mode};
use crate::{BridgeError, BridgeTarget, Error};

pub enum ToolOutput {
    Json(Value),
    Png { png: Vec<u8>, note: Value },
    Error { message: String, data: Option<Value> },
}

impl ToolOutput {
    fn error(message: impl Into<String>) -> Self {
        ToolOutput::Error { message: message.into(), data: None }
    }

    fn engine(error: &EngineError) -> Self {
        ToolOutput::Error { message: error.agent_message(), data: Some(error.to_json()) }
    }
}

/// The MCP layer is identical over both backends. Tool calls run one at a time behind a mutex (PhotoCraft), EXCEPT
/// `stop`, which the server routes to `stop_handle()` without taking that mutex, so a long `job_simulate` or a
/// 55 s bridge timeout can never delay a stop.
pub trait ToolBackend: Send {
    fn mode(&self) -> Mode;
    fn call(&mut self, tool: &str, args: Value, client: &ClientInfo) -> ToolOutput;
    /// Bridge: a handle with its own authenticated connection, used only for `machine.stop`. Headless: None (the
    /// `stop` tool then returns the `needs_bridge:` error).
    fn stop_handle(&self) -> Option<StopHandle>;
}

/// The control method a headless `serve` answers for `machine.stop`, on stdio and TCP alike: nothing is connected,
/// so nothing was stopped — said plainly, never `latched:true`.
pub(crate) fn headless_stop_report(machine: Option<&str>) -> Value {
    json!({
        "latched": false,
        "confirmed": false,
        "machine": machine,
        "stopped": [],
        "backend": "headless",
        "elapsedMs": 0,
        "message": "headless session: no machine is connected, so nothing was stopped",
    })
}

const NEEDS_BRIDGE: &str = "needs the running OpenLaser window: turn on Automation › Allow Agent Connections, then use `openlaser-cli mcp --bridge auto`";

/// Where headless file effects go: the automation roots (mcp, serve) or the caller's own paths (the CLI's local
/// subcommands, whose paths the person typed).
enum Files {
    Scoped(Arc<AuthorizedWorkspace>),
    Local,
}

impl Files {
    fn write(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        match self {
            Files::Scoped(ws) => ws.write_atomic(path, bytes).map_err(|e| e.to_string()),
            Files::Local => write_local_atomic(Path::new(path), bytes).map_err(|e| format!("file: {path}: {e}")),
        }
    }
}

fn write_local_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.openlaser-{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

struct EngineWorkspace {
    inner: Arc<AuthorizedWorkspace>,
}

impl openlaser_engine::FileAccess for EngineWorkspace {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.inner.read(path).map_err(|e| e.to_string())
    }
    fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.inner.write_atomic(path, bytes).map_err(|e| e.to_string())
    }
}

/// engine::Session with origin Headless, window false, DeviceAccess::None; files through AuthorizedWorkspace.
pub struct Headless {
    session: Session,
    files: Files,
}

impl Headless {
    /// For `mcp` and `serve`: file access only inside the roots; no root = denied.
    pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, Error> {
        let workspace = Arc::new(AuthorizedWorkspace::open(read_root, write_root)?);
        let authority = openlaser_engine::FsAuthority::Scoped(Box::new(EngineWorkspace { inner: Arc::clone(&workspace) }));
        Ok(Self::with(authority, Files::Scoped(workspace)))
    }

    /// For `openlaser-cli`'s local subcommands only (`info`, `convert`, `run`, `plan`, …): the person typed the paths,
    /// so they are used as given (§3.21 "trusted-local"). Never for `mcp` or `serve`.
    pub fn trusted_local() -> Self {
        Self::with(openlaser_engine::FsAuthority::TrustedLocal, Files::Local)
    }

    fn with(files_authority: openlaser_engine::FsAuthority, files: Files) -> Self {
        let session = Session::new(openlaser_engine::SessionConfig {
            origin: Origin::Headless,
            window: false,
            devices: openlaser_engine::DeviceAccess::None,
            machines: vec![openlaser_machine::jpt_m7_200_sim()],
            profiles: None,
            camera_store: None,
            files: files_authority,
            clock: Arc::new(openlaser_types::SystemClock::default()),
        });
        Headless { session, files }
    }

    /// One control method (`openlaser-cli serve`) through the same tool implementation and argument checks as MCP.
    /// Params use the control protocol's camelCase keys.
    pub fn dispatch_control(&mut self, method: &str, params: Value) -> ToolOutput {
        let tool = match method {
            "machine.stop" => return ToolOutput::Json(headless_stop_report(params.get("machine").and_then(Value::as_str))),
            "methods" => return ToolOutput::Json(json!(openlaser_control::methods::METHODS)),
            "engine.execute" => "command_run",
            "engine.commands" => "command_list",
            "engine.schema" => "command_schema",
            "jobs.list" => "jobs_list",
            "jobs.cancel" => "jobs_cancel",
            "app.open" => "doc_open",
            "app.import" => "doc_import",
            "app.save" => "doc_save",
            "app.export" => "doc_export",
            "doc.inspect" => "doc_inspect",
            "doc.render" => "render_preview",
            "doc.select" => "doc_select",
            "doc.close" => "doc_close",
            "session.list" => "session_list",
            "machine.list" => "machine_list",
            "machine.status" => "machine_status",
            "job.plan" => "job_plan",
            "job.preview" => "job_preview",
            "job.estimate" => "job_estimate",
            "job.simulate" => "job_simulate",
            "job.export" => "job_export",
            "gcode.export" => "gcode_export",
            _ if openlaser_control::methods::METHODS.contains(&method) => return ToolOutput::error(format!("needs_bridge: `{method}` {NEEDS_BRIDGE}")),
            _ => return ToolOutput::error(format!("unknown method `{method}`")),
        };
        let mut args = match params {
            Value::Null => Value::Object(Map::new()),
            Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (control_key_to_arg(method, &k), v)).collect()),
            _ => return ToolOutput::error(format!("`{method}`: params must be an object")),
        };
        if method == "engine.execute"
            && let Some(object) = args.as_object_mut()
            && let Some(command) = object.remove("command")
        {
            object.insert("id".to_owned(), command);
        }
        if let Some(def) = crate::tools::find(tool)
            && let Err(reason) = crate::tools::check(&(def.input_schema)(), &args)
        {
            return ToolOutput::error(format!("`{method}`: {}", reason.replace("argument", "param")));
        }
        self.call(tool, args, &ClientInfo { name: "openlaser-cli serve".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() })
    }

    fn execute(&mut self, id: &str, params: Value) -> ToolOutput {
        match self.session.execute(id, params) {
            Ok(value) => ToolOutput::Json(value),
            Err(e) => ToolOutput::engine(&e),
        }
    }

    /// Runs `f` with document `index` active, then restores the previous active document.
    fn on_document<R>(&mut self, index: Option<usize>, f: impl FnOnce(&mut Self) -> R) -> Result<R, ToolOutput> {
        let previous = self.session.active_index();
        let switch = index.filter(|i| Some(*i) != previous);
        if let Some(i) = switch {
            self.session.select_doc(i).map_err(|e| ToolOutput::engine(&e))?;
        }
        let result = f(self);
        if switch.is_some()
            && let Some(p) = previous
        {
            let _ = self.session.select_doc(p);
        }
        Ok(result)
    }

    fn session_list(&self) -> Value {
        json!({
            "documents": self.session.docs().iter().enumerate().map(|(index, doc)| json!({
                "index": index, "name": doc.display_name(), "path": doc.path, "dirty": doc.dirty(),
                "revision": doc.revision, "machine": doc.machine.0,
            })).collect::<Vec<_>>(),
            "active": self.session.active_index(),
        })
    }

    fn job_plan(&self, args: &Value) -> Value {
        let index = optional_index(args);
        let opts = job_options(args);
        let machine = opts
            .machine
            .clone()
            .map(|m| m.0)
            .or_else(|| index.or(self.session.active_index()).and_then(|i| self.session.docs().get(i)).map(|d| d.machine.0.clone()));
        match self.session.build_job(index, &opts) {
            Ok(built) => {
                let frame = built.validated.frame();
                let paths = frame.paths();
                json!({
                    "fingerprint": built.summary.fingerprint.hex(),
                    "short": built.summary.fingerprint.short(),
                    "machine": built.summary.machine.0,
                    "ok": true,
                    "refusals": [],
                    "notices": built.validated.notices(),
                    "summary": built.summary,
                    "frame": {
                        "style": frame.style(),
                        "source": frame.source(),
                        "bounds": bounds_of(paths.iter().flat_map(|p| p.0.iter())),
                        "paths": paths.len(),
                    },
                })
            }
            Err(refused) => json!({
                "fingerprint": refused.fingerprint.map(|f| f.hex()),
                "short": refused.fingerprint.map(|f| f.short()),
                "machine": machine,
                "ok": false,
                "refusals": refused.refusals,
                "notices": refused.notices,
                "summary": null,
                "frame": null,
                "error": refused.error,
            }),
        }
    }

    fn command_batch(&mut self, args: &Value) -> ToolOutput {
        let steps: Vec<(String, Value)> = args
            .get("steps")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|s| (s.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(), s.get("params").cloned().unwrap_or_else(|| json!({}))))
                    .collect()
            })
            .unwrap_or_default();
        let stop_on_error = args.get("stop_on_error").and_then(Value::as_bool).unwrap_or(true);
        if args.get("atomic").and_then(Value::as_bool).unwrap_or(false) {
            let label = args.get("label").and_then(Value::as_str).unwrap_or("Automation batch").to_owned();
            let result = self.session.batch(Origin::Headless, steps, stop_on_error, Some(label));
            // All or nothing already happened; the budget only bounds what is reported.
            let mut budget = BatchBudget::escaped();
            let mut results = Vec::new();
            for step in result.results {
                if let Err(e) = budget.charge(&step) {
                    results.push(
                        json!({"ok": false, "error": format!("batch response budget exceeded; the remaining steps ran (atomic batch); inspect state: {e}")}),
                    );
                    break;
                }
                results.push(step);
            }
            return ToolOutput::Json(json!({"completed": result.completed, "failed": result.failed, "rolledBack": result.rolled_back, "results": results}));
        }
        // Non-atomic: one step at a time, so the reply budget can stop later steps (PhotoCraft's BatchReplyBudget).
        // A Stop command is never skipped (§1.3 rule 5), whatever ended the batch.
        let mut budget = BatchBudget::escaped();
        let (mut completed, mut failed) = (0_usize, 0_usize);
        let mut results = Vec::new();
        let mut halted = false;
        for (i, (id, params)) in steps.into_iter().enumerate() {
            let is_stop = openlaser_engine::registry().find(&id).is_some_and(|spec| spec.access == openlaser_engine::Access::Stop);
            if halted && !is_stop {
                continue;
            }
            let one = self.session.batch(Origin::Headless, vec![(id, params)], true, None);
            let ok = one.failed == 0;
            let mut step = one.results.into_iter().next().unwrap_or_else(|| json!({"ok": false, "error": "no result"}));
            if halted && let Some(object) = step.as_object_mut() {
                object.insert("step".to_owned(), json!(i));
            }
            if ok {
                completed += 1;
            } else {
                failed += 1;
            }
            if let Err(e) = budget.charge(&step) {
                if !halted {
                    results.push(json!({"ok": false, "error": format!("batch response budget exceeded; current step may have completed: {e}")}));
                }
                halted = true;
                continue;
            }
            results.push(step);
            if !ok && stop_on_error {
                halted = true;
            }
        }
        ToolOutput::Json(json!({"completed": completed, "failed": failed, "rolledBack": false, "results": results}))
    }

    fn render(&self, list: openlaser_render::DisplayList, max_side: u32, note: Value) -> ToolOutput {
        let Some(world) = openlaser_render::fit(&list, 0.05) else { return ToolOutput::error("preview has no renderable geometry") };
        match openlaser_render::render_png(&list, &openlaser_render::RenderView { world, max_side, background: openlaser_render::Rgba([255, 255, 255, 255]) }) {
            Ok(png) => {
                let mut note = note;
                if let Some(object) = note.as_object_mut() {
                    object.insert("bounds".to_owned(), json!(world));
                }
                png_output(png, note)
            }
            Err(e) => ToolOutput::error(e.to_string()),
        }
    }

    fn run(&mut self, tool: &str, args: Value) -> ToolOutput {
        let index = optional_index(&args);
        match tool {
            "session_list" => ToolOutput::Json(self.session_list()),
            "doc_new" => self.execute("file.new", args),
            "doc_open" => self.execute("file.open", args),
            "doc_import" => {
                let mut params = Map::new();
                for (key, value) in args.as_object().into_iter().flatten() {
                    let key = match key.as_str() {
                        "center" => "centerMm",
                        "svg_units" => "svgUnits",
                        other => other,
                    };
                    params.insert(key.to_owned(), value.clone());
                }
                self.execute("file.import", Value::Object(params))
            }
            "doc_save" => match args.get("path") {
                Some(path) => self.execute("file.saveAs", json!({"path": path})),
                None => self.execute("file.save", json!({})),
            },
            "doc_export" => ToolOutput::error("not implemented: doc_export (SVG export is a P0 stretch goal); save the project with doc_save"),
            "doc_inspect" => match self.session.inspect(index, args.get("objects").and_then(Value::as_str) == Some("all")) {
                Ok(value) => ToolOutput::Json(value),
                Err(e) => ToolOutput::engine(&e),
            },
            "doc_select" => match index.map(|i| self.session.select_doc(i)) {
                Some(Ok(())) => ToolOutput::Json(self.session_list()),
                Some(Err(e)) => ToolOutput::engine(&e),
                None => ToolOutput::error("invalid params: index is required"),
            },
            "doc_close" => match self.session.close_doc(index) {
                Ok(()) => ToolOutput::Json(self.session_list()),
                Err(e) => ToolOutput::engine(&e),
            },
            "render_preview" => self.render_preview(&args),
            "command_list" => {
                let filter = openlaser_engine::ListFilter {
                    filter: args.get("filter").and_then(Value::as_str).map(str::to_owned),
                    enabled_only: args.get("enabled_only").and_then(Value::as_bool).unwrap_or(false),
                    agent: args.get("agent").and_then(Value::as_str).and_then(agent_class),
                    schema: args.get("schema").and_then(Value::as_bool).unwrap_or(false),
                };
                ToolOutput::Json(json!(self.session.command_list(Origin::Headless, &filter)))
            }
            "command_schema" => match self.session.command_schema(args.get("id").and_then(Value::as_str).unwrap_or_default()) {
                Ok(value) => ToolOutput::Json(json!({"id": args.get("id"), "schema": value})),
                Err(e) => ToolOutput::engine(&e),
            },
            "command_run" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                self.execute(&id, args.get("params").cloned().unwrap_or_else(|| json!({})))
            }
            "command_batch" => self.command_batch(&args),
            "jobs_list" => ToolOutput::Json(json!([])),
            "jobs_cancel" => ToolOutput::error("a headless session runs no background jobs"),
            "machine_list" => ToolOutput::Json(json!(self.session.machines())),
            "machine_status" => ToolOutput::Json(json!({
                "connected": false, "backend": "headless", "machine": args.get("machine"),
                "note": "headless sessions have no device access; machine_status reads a live machine in bridge mode (openlaser-cli mcp --bridge auto)",
            })),
            "job_plan" => ToolOutput::Json(self.job_plan(&args)),
            "job_estimate" => match self.session.build_job(index, &job_options(&args)) {
                Ok(built) => ToolOutput::Json(json!({
                    "fingerprint": built.summary.fingerprint.hex(),
                    "estimateS": built.summary.estimate.as_secs_f64(),
                    "crashBoundS": built.summary.crash_bound.as_secs_f64(),
                    "perSection": built.summary.rows.iter().map(|row| json!({
                        "layer": row.layer, "pass": row.pass, "of": row.of, "estimateS": row.estimate.as_secs_f64(), "markMm": row.mark_mm, "paths": row.paths,
                    })).collect::<Vec<_>>(),
                })),
                Err(refused) => refused_output(&refused),
            },
            "job_preview" => self.job_preview(&args),
            "job_simulate" => self.job_simulate(&args),
            "job_export" => {
                let path = args.get("path").cloned().unwrap_or(Value::Null);
                self.on_document(index, |me| me.execute("job.dump", json!({"path": path}))).unwrap_or_else(|e| e)
            }
            "gcode_export" => self.gcode_export(&args),
            "stop" => ToolOutput::error(format!("needs_bridge: stop {NEEDS_BRIDGE}; a headless session has no machine to stop")),
            "frame_request" | "job_submit" | "job_status" | "job_withdraw" | "control_call" => {
                ToolOutput::error(format!("needs_bridge: `{tool}` {NEEDS_BRIDGE}"))
            }
            ui if ui.starts_with("ui_") => ToolOutput::error(format!("needs_bridge: `{tool}` {NEEDS_BRIDGE}")),
            _ => ToolOutput::error(format!("unknown tool `{tool}`")),
        }
    }

    fn render_preview(&mut self, args: &Value) -> ToolOutput {
        let max_side = max_side(args);
        let index = optional_index(args);
        let Some(doc) = index.and_then(|i| self.session.docs().get(i)).or_else(|| self.session.active()) else {
            return ToolOutput::error("no document open");
        };
        let name = doc.display_name();
        if args.get("view").and_then(Value::as_str) == Some("toolpath") {
            let built = match self.session.build_job(index, &openlaser_engine::JobOptions::default()) {
                Ok(built) => built,
                Err(refused) => return refused_output(&refused),
            };
            let Some(machine) = self.session.machine_for(index) else { return ToolOutput::error("no machine profile for document") };
            let list = openlaser_render::plan_list(&built.plan, machine, &openlaser_render::PlanShow { burn: true, travel: true, frame: false, order: false });
            return self.render(list, max_side, json!({"document": name, "view": "toolpath", "fingerprint": built.summary.fingerprint.hex()}));
        }
        let profile = self.session.machines().iter().find(|m| m.id == doc.machine);
        let opts = openlaser_render::ViewOpts {
            dark: false,
            selection: Vec::new(),
            refusals: Vec::new(),
            notices: Vec::new(),
            show_field: true,
            show_work_area: true,
        };
        let list = openlaser_render::document_list(&doc.project, profile, &opts);
        self.render(list, max_side, json!({"document": name, "view": "design"}))
    }

    fn job_preview(&mut self, args: &Value) -> ToolOutput {
        let index = optional_index(args);
        let built = match self.session.build_job(index, &job_options(args)) {
            Ok(built) => built,
            Err(refused) => return refused_output(&refused),
        };
        let Some(profile) = self.session.machine_for(index) else { return ToolOutput::error("no machine profile for document") };
        let names: Option<Vec<&str>> = args.get("show").and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).collect());
        let show = openlaser_render::PlanShow {
            burn: names.as_ref().is_none_or(|items| items.contains(&"burn")),
            travel: names.as_ref().is_none_or(|items| items.contains(&"travel")),
            frame: names.as_ref().is_some_and(|items| items.contains(&"frame")),
            order: names.as_ref().is_some_and(|items| items.contains(&"order")),
        };
        let list = openlaser_render::plan_list(&built.plan, profile, &show);
        self.render(list, max_side(args), json!({"fingerprint": built.summary.fingerprint.hex()}))
    }

    fn job_simulate(&mut self, args: &Value) -> ToolOutput {
        let index = optional_index(args);
        let built = match self.session.build_job(index, &job_options(args)) {
            Ok(built) => built,
            Err(refused) => return refused_output(&refused),
        };
        let Some(profile) = self.session.machine_for(index) else { return ToolOutput::error("no machine profile for document") };
        let (job, galvo) = match (&built.encoded, &profile.kind) {
            (openlaser_engine::Encoded::Lmc { job, .. }, openlaser_machine::Kinematics::Galvo(galvo)) => (job, galvo),
            _ => return ToolOutput::error("job_simulate runs galvo (LMC) jobs on the simulator; use gcode_export for gantry profiles"),
        };
        let recording = openlaser_lmc_sim::simulate_job(job, galvo);
        let light = recording.lit.to_vec();
        let list = openlaser_render::emission_list(&recording.burned, &light, profile);
        let mut note = json!({
            "fingerprint": built.summary.fingerprint.hex(), "summary": built.summary,
            "wireEvents": recording.wire.len(), "burnedSegments": recording.burned.len(),
        });
        if args.get("listing").and_then(Value::as_bool).unwrap_or(false) {
            let total = built.listing.lines.len();
            let mut lines = Vec::new();
            let mut bytes = 0_usize;
            for line in built.listing.lines.iter().take(budgets::SIM_LISTING_MAX_ROWS) {
                if bytes.saturating_add(line.text.len()) > budgets::SIM_LISTING_MAX_BYTES {
                    break;
                }
                bytes = bytes.saturating_add(line.text.len());
                lines.push(line.text.clone());
            }
            if let Some(object) = note.as_object_mut() {
                object.insert("listing".to_owned(), json!({"lines": lines, "shown": lines.len(), "total": total, "truncated": lines.len() < total}));
            }
        }
        self.render(list, max_side(args), note)
    }

    fn gcode_export(&mut self, args: &Value) -> ToolOutput {
        let Some(path) = args.get("path").and_then(Value::as_str) else { return ToolOutput::error("invalid params: path is required") };
        let built = match self.session.build_job(optional_index(args), &job_options(args)) {
            Ok(built) => built,
            Err(refused) => return refused_output(&refused),
        };
        let openlaser_engine::Encoded::Gcode { job, .. } = &built.encoded else {
            return ToolOutput::error(
                "gcode_export is for gantry (GRBL) machine profiles; galvo jobs are reviewed with job_export and run from the OpenLaser window",
            );
        };
        match self.files.write(path, job.text.as_bytes()) {
            Ok(()) => ToolOutput::Json(json!({"path": path, "lines": job.text.lines().count(), "fingerprint": built.summary.fingerprint.hex()})),
            Err(e) => ToolOutput::error(e),
        }
    }
}

impl ToolBackend for Headless {
    fn mode(&self) -> Mode {
        Mode::Headless
    }

    fn call(&mut self, tool: &str, args: Value, client: &ClientInfo) -> ToolOutput {
        let _ = client;
        self.run(tool, args)
    }

    fn stop_handle(&self) -> Option<StopHandle> {
        None
    }
}

impl crate::serve::ControlBackend for Headless {
    fn dispatch_control(&mut self, method: &str, params: Value) -> ToolOutput {
        Headless::dispatch_control(self, method, params)
    }
}

/// The GUI bridge: tool calls on one lazily opened connection, `stop` on a handle of its own.
pub struct Bridge {
    client: BridgeClient,
    stop: StopHandle,
}

impl Bridge {
    /// Never fails and never connects: the GUI is found at the first call (AUT B10.2), and a GUI that starts later
    /// is still reached; with none, every call says `OpenLaser is not running …` and nothing falls back to headless.
    pub fn new(target: BridgeTarget, client: &ClientInfo) -> Self {
        let link = Link::new(target, client.clone());
        Bridge { client: BridgeClient::lazy(Arc::clone(&link)), stop: StopHandle::lazy(link) }
    }

    fn forward(&mut self, method: &str, params: Value) -> ToolOutput {
        let read = openlaser_control::methods::is_read_only(method);
        let result = if read { self.client.call_read(method, params) } else { self.client.call(method, params) };
        match result {
            Ok(value) if matches!(method, "doc.render" | "job.preview" | "job.simulate" | "ui.screenshot") => bridged_png(value),
            Ok(value) => ToolOutput::Json(value),
            Err(e) => ToolOutput::error(format_bridge_error(e)),
        }
    }

    fn command_list(&mut self, args: &Value) -> ToolOutput {
        let rows = match self.client.call_read("engine.commands", json!({})) {
            Ok(Value::Array(rows)) => rows,
            Ok(other) => return ToolOutput::error(format!("engine.commands returned {other}, not a list")),
            Err(e) => return ToolOutput::error(format_bridge_error(e)),
        };
        let filter = args.get("filter").and_then(Value::as_str).map(str::to_lowercase);
        let enabled_only = args.get("enabled_only").and_then(Value::as_bool).unwrap_or(false);
        let agent = args.get("agent").and_then(Value::as_str);
        let schema = args.get("schema").and_then(Value::as_bool).unwrap_or(false);
        let rows: Vec<Value> = rows
            .into_iter()
            .filter(|row| {
                let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default().to_lowercase();
                filter.as_ref().is_none_or(|f| text("id").contains(f) || text("label").contains(f))
                    && (!enabled_only || row.get("enabled").and_then(Value::as_bool) == Some(true))
                    && agent.is_none_or(|a| row.get("agent").and_then(Value::as_str) == Some(a))
            })
            .map(|mut row| {
                if !schema && let Some(object) = row.as_object_mut() {
                    object.remove("schema");
                }
                row
            })
            .collect();
        ToolOutput::Json(Value::Array(rows))
    }

    fn command_batch(&mut self, args: &Value) -> ToolOutput {
        if args.get("atomic").and_then(Value::as_bool).unwrap_or(false) {
            return ToolOutput::error("atomic batches run in a headless session; over the bridge run the steps without atomic (each is undoable)");
        }
        let stop_on_error = args.get("stop_on_error").and_then(Value::as_bool).unwrap_or(true);
        let steps = args.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut budget = BatchBudget::escaped();
        let (mut completed, mut failed) = (0_usize, 0_usize);
        let mut results = Vec::new();
        let mut halted = false;
        for (i, step) in steps.iter().enumerate() {
            let id = step.get("id").and_then(Value::as_str).unwrap_or_default();
            let is_stop = openlaser_engine::registry().find(id).is_some_and(|spec| spec.access == openlaser_engine::Access::Stop);
            if halted && !is_stop {
                continue;
            }
            let mut params = json!({"command": id, "params": step.get("params").cloned().unwrap_or_else(|| json!({}))});
            if let (Some(wait), Some(object)) = (step.get("wait"), params.as_object_mut()) {
                object.insert("wait".to_owned(), wait.clone());
            }
            let response = self.client.call("engine.execute", params);
            let ok = response.is_ok();
            let mut result = match response {
                Ok(value) => json!({"ok": true, "result": value}),
                Err(e) => json!({"ok": false, "error": format_bridge_error(e)}),
            };
            if halted && let Some(object) = result.as_object_mut() {
                object.insert("step".to_owned(), json!(i));
            }
            if ok {
                completed += 1;
            } else {
                failed += 1;
            }
            if let Err(e) = budget.charge(&result) {
                if !halted {
                    results.push(json!({"ok": false, "error": format!("batch response budget exceeded; current step may have completed: {e}")}));
                }
                halted = true;
                continue;
            }
            results.push(result);
            if !ok && stop_on_error {
                halted = true;
            }
        }
        ToolOutput::Json(json!({"completed": completed, "failed": failed, "rolledBack": false, "results": results}))
    }
}

impl ToolBackend for Bridge {
    fn mode(&self) -> Mode {
        Mode::Bridge
    }

    fn call(&mut self, tool: &str, args: Value, client: &ClientInfo) -> ToolOutput {
        self.client.link().set_info(client);
        match tool {
            "stop" => return self.stop.stop(args.get("machine").and_then(Value::as_str)),
            "command_list" => return self.command_list(&args),
            "command_batch" => return self.command_batch(&args),
            "doc_new" => return self.forward("engine.execute", json!({"command": "file.new", "params": args})),
            "command_run" => {
                let mut params = json!({"command": args.get("id"), "params": args.get("params").cloned().unwrap_or_else(|| json!({}))});
                if let (Some(wait), Some(object)) = (args.get("wait"), params.as_object_mut()) {
                    object.insert("wait".to_owned(), wait.clone());
                }
                return self.forward("engine.execute", params);
            }
            "control_call" => {
                let method = args.get("method").and_then(Value::as_str).unwrap_or_default().to_owned();
                let params = args.get("params").cloned().unwrap_or_else(|| json!({}));
                if method == "machine.stop" {
                    return self.stop.stop(params.get("machine").and_then(Value::as_str));
                }
                return match self.client.call(&method, params) {
                    Ok(value) => ToolOutput::Json(value),
                    Err(e) => ToolOutput::error(format_bridge_error(e)),
                };
            }
            _ => {}
        }
        let method = match tool {
            "command_schema" => "engine.schema",
            "doc_open" => "app.open",
            "doc_import" => "app.import",
            "doc_save" => "app.save",
            "doc_export" => "app.export",
            "doc_inspect" => "doc.inspect",
            "render_preview" => "doc.render",
            "doc_select" => "doc.select",
            "doc_close" => "doc.close",
            "session_list" => "session.list",
            "jobs_list" => "jobs.list",
            "jobs_cancel" => "jobs.cancel",
            "machine_list" => "machine.list",
            "machine_status" => "machine.status",
            "job_plan" => "job.plan",
            "job_preview" => "job.preview",
            "job_estimate" => "job.estimate",
            "job_simulate" => "job.simulate",
            "job_export" => "job.export",
            "gcode_export" => "gcode.export",
            "frame_request" => "frame.request",
            "job_submit" => "job.submit",
            "job_status" => "job.status",
            "job_withdraw" => "job.withdraw",
            "ui_inspect" => "ui.inspect",
            "ui_elements" => "ui.elements",
            "ui_screenshot" => "ui.screenshot",
            "ui_click" => "ui.click",
            "ui_drag" => "ui.drag",
            "ui_key" => "ui.key",
            "ui_type" => "ui.type",
            "ui_pointer" => "ui.pointer",
            "ui_menu_invoke" => "ui.menu.invoke",
            "ui_set" => "ui.set",
            _ => return ToolOutput::error(format!("unknown tool `{tool}`")),
        };
        self.forward(method, args_to_control(args))
    }

    fn stop_handle(&self) -> Option<StopHandle> {
        Some(self.stop.clone())
    }
}

/// Tool arguments (snake_case) as control params (camelCase); `center` is `file.import`'s `centerMm`; a numeric
/// submission id travels as a number.
fn args_to_control(args: Value) -> Value {
    let Value::Object(map) = args else { return args };
    Value::Object(
        map.into_iter()
            .map(|(key, value)| {
                let value = match (key.as_str(), &value) {
                    ("submission", Value::String(s)) => s.parse::<u64>().map_or(value.clone(), |n| json!(n)),
                    _ => value,
                };
                let key = if key == "center" { "centerMm".to_owned() } else { openlaser_engine::snake_to_camel(&key) };
                (key, value)
            })
            .collect(),
    )
}

/// Control params (camelCase) as tool arguments (snake_case), the inverse of [`args_to_control`].
fn control_key_to_arg(method: &str, key: &str) -> String {
    if method == "app.import" && key == "centerMm" {
        return "center".to_owned();
    }
    let mut out = String::with_capacity(key.len() + 4);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// For `openlaser-cli ui --out`: a control result's PNG (`pngBase64`, checked against the PNG budget and the
/// bridged-image limits) and the rest of the result.
pub fn png_from_result(value: Value) -> Result<(Option<Vec<u8>>, Value), String> {
    if value.get("pngBase64").is_none() {
        return Ok((None, value));
    }
    match bridged_png(value) {
        ToolOutput::Png { png, note } => Ok((Some(png), note)),
        ToolOutput::Json(rest) => Ok((None, rest)),
        ToolOutput::Error { message, .. } => Err(message),
    }
}

/// A rendering control method's result: `{pngBase64, …note}` → typed PNG output, checked against the PNG budget and
/// the bridged-image limits before an agent sees it.
fn bridged_png(value: Value) -> ToolOutput {
    let Value::Object(mut map) = value else { return ToolOutput::error("the app returned an image result that is not an object") };
    let Some(encoded) = map.remove("pngBase64") else { return ToolOutput::Json(Value::Object(map)) };
    let Some(encoded) = encoded.as_str() else { return ToolOutput::error("the app returned a non-text pngBase64") };
    // base64 is 4/3 of the bytes: refuse before decoding anything far over the budget.
    if encoded.len() / 4 * 3 > budgets::MAX_PNG_BYTES + 3 {
        return png_too_large(encoded.len() / 4 * 3);
    }
    match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(png) => png_output(png, Value::Object(map)),
        Err(e) => ToolOutput::error(format!("the app returned an undecodable image: {e}")),
    }
}

fn png_too_large(bytes: usize) -> ToolOutput {
    ToolOutput::error(format!("image is {bytes} bytes, over the {} byte PNG budget; ask for a smaller max_side", budgets::MAX_PNG_BYTES))
}

/// Every PNG an agent gets passes the 5 MiB budget and the decode limits (edge ≤ 8192, ≤ 16 Mi pixels).
pub(crate) fn png_output(png: Vec<u8>, note: Value) -> ToolOutput {
    if png.len() > budgets::MAX_PNG_BYTES {
        return png_too_large(png.len());
    }
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let dims = png.get(..24).filter(|head| head.starts_with(SIGNATURE) && head.get(12..16) == Some(b"IHDR".as_slice())).and_then(|head| {
        let w = u32::from_be_bytes(head.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(head.get(20..24)?.try_into().ok()?);
        Some((w, h))
    });
    let Some((width, height)) = dims else { return ToolOutput::error("the image is not a PNG") };
    if width == 0 || height == 0 || width.max(height) > budgets::SCREENSHOT_MAX_EDGE || u64::from(width) * u64::from(height) > budgets::SCREENSHOT_MAX_PIXELS {
        return ToolOutput::error(format!("image is {width}×{height}, over the bridged image limits; ask for a smaller max_side"));
    }
    ToolOutput::Png { png, note }
}

/// PhotoCraft's `BatchReplyBudget`: each step result is charged at its size inside a JSON string (the MCP text
/// content escapes it again); once spent, later steps are not run.
struct BatchBudget {
    remaining: usize,
}

impl BatchBudget {
    fn escaped() -> Self {
        BatchBudget { remaining: budgets::MAX_RESPONSE_BYTES.saturating_sub(budgets::MAX_REQUEST_BYTES).saturating_sub(4096) }
    }

    fn charge(&mut self, result: &Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(result).map_err(|e| e.to_string())?;
        // Compact JSON has no raw control characters, so only `"` and `\` grow when escaped again.
        let size = bytes.len().saturating_add(bytes.iter().filter(|&&b| b == b'"' || b == b'\\').count());
        if size >= self.remaining {
            return Err(format!("step result is {size} bytes once escaped; {} remain", self.remaining));
        }
        self.remaining = self.remaining.saturating_sub(size.saturating_add(1));
        Ok(())
    }
}

fn refused_output(refused: &openlaser_engine::JobRefused) -> ToolOutput {
    let data = json!({
        "refusals": refused.refusals, "notices": refused.notices,
        "fingerprint": refused.fingerprint.map(|f| f.hex()), "error": refused.error,
    });
    let message = match (&refused.error, refused.refusals.len()) {
        (Some(error), 0) => format!("refused: the job could not be built: {error}"),
        (_, n) => format!("refused: {n} refusal(s); see data.refusals (job_plan lists them with the objects)"),
    };
    ToolOutput::Error { message, data: Some(data) }
}

fn agent_class(text: &str) -> Option<AgentClass> {
    Some(match text {
        "allowed" => AgentClass::Allowed,
        "rootsOnly" => AgentClass::RootsOnly,
        "ui" => AgentClass::Ui,
        "humanOnly" => AgentClass::HumanOnly,
        "stop" => AgentClass::Stop,
        _ => return None,
    })
}

fn optional_index(args: &Value) -> Option<usize> {
    args.get("index").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok())
}

fn max_side(args: &Value) -> u32 {
    args.get("max_side")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .map_or(budgets::PREVIEW_DEFAULT_SIDE, |v| v.clamp(1, budgets::PREVIEW_MAX_SIDE))
}

fn job_options(args: &Value) -> openlaser_engine::JobOptions {
    openlaser_engine::JobOptions {
        machine: args.get("machine").and_then(Value::as_str).map(|id| openlaser_types::MachineId(id.to_owned())),
        selected_only: args.get("selected_only").and_then(Value::as_bool),
    }
}

fn bounds_of<'a>(points: impl Iterator<Item = &'a [f64; 2]>) -> Value {
    let mut bounds: Option<([f64; 2], [f64; 2])> = None;
    for p in points {
        let (min, max) = bounds.get_or_insert((*p, *p));
        *min = [min[0].min(p[0]), min[1].min(p[1])];
        *max = [max[0].max(p[0]), max[1].max(p[1])];
    }
    bounds.map_or(Value::Null, |(min, max)| json!({"min": min, "max": max}))
}

pub(crate) fn format_bridge_error(error: BridgeError) -> String {
    match error {
        BridgeError::NotSent(text) => format!("not_connected: {text}; the request was not sent"),
        BridgeError::Transport(text) => format!("transport: {text}; operation may have completed; inspect state before retrying"),
        BridgeError::TimedOut { read: true } => {
            "timeout: no reply from OpenLaser in 55 s (the window may be asleep or busy); a read may be retried safely".to_owned()
        }
        BridgeError::TimedOut { read: false } => {
            "timeout: no reply from OpenLaser in 55 s; the operation may have completed; inspect state before retrying".to_owned()
        }
        BridgeError::Definite(text) => text,
        BridgeError::NotImplemented(text) => format!("not implemented: {text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> ClientInfo {
        ClientInfo { name: "test".into(), version: "0".into() }
    }

    #[test]
    fn bridge_failure_wording() {
        assert!(format_bridge_error(BridgeError::NotSent("connection refused".to_owned())).contains("the request was not sent"));
        assert!(format_bridge_error(BridgeError::Transport("reset".to_owned())).contains("operation may have completed; inspect state before retrying"));
        assert!(format_bridge_error(BridgeError::TimedOut { read: true }).contains("may be retried safely"));
        assert!(format_bridge_error(BridgeError::TimedOut { read: false }).contains("may have completed"));
    }

    #[test]
    fn headless_control_dispatch_uses_engine_tool_routes() {
        let mut backend = Headless::new(None, None).unwrap();
        let output = backend.dispatch_control("methods", json!({}));
        assert!(matches!(output, ToolOutput::Json(value) if value.as_array().is_some_and(|items| items.iter().any(|item| item == "engine.execute"))));
        let unknown = backend.dispatch_control("made.up", json!({}));
        assert!(matches!(unknown, ToolOutput::Error { message, .. } if message.starts_with("unknown method")));
        let strict = backend.dispatch_control("doc.inspect", json!({"bogus": 1}));
        assert!(matches!(strict, ToolOutput::Error { message, .. } if message.contains("unknown param `bogus`")), "control params are strict too");
        assert!(matches!(backend.dispatch_control("engine.execute", json!({"command":"file.new","params":{}})), ToolOutput::Json(_)));
    }

    #[test]
    fn headless_device_tools_need_bridge() {
        let mut backend = Headless::new(None, None).unwrap();
        for tool in ["frame_request", "job_submit", "job_status", "job_withdraw", "stop", "ui_click", "control_call"] {
            let output = backend.call(tool, json!({}), &client());
            assert!(
                matches!(&output, ToolOutput::Error { message, .. } if message.starts_with("needs_bridge:") && message.contains("--bridge auto")),
                "{tool}"
            );
        }
        assert!(matches!(backend.call("machine_status", json!({}), &client()), ToolOutput::Json(v) if v["connected"] == false && v["backend"] == "headless"));
    }

    #[test]
    fn headless_document_create_and_list() {
        let temp = tempfile::tempdir().unwrap();
        let mut backend = Headless::new(Some(temp.path()), Some(temp.path())).unwrap();
        assert!(matches!(backend.call("doc_new", json!({"name":"WP7"}), &client()), ToolOutput::Json(_)));
        assert!(
            matches!(backend.call("session_list", json!({}), &client()), ToolOutput::Json(value) if value["documents"].as_array().is_some_and(|docs| docs.len() == 1))
        );
    }

    /// The command-run path is the engine's: a human-only command is refused by the engine's policy, with its text.
    #[test]
    fn human_only_commands_are_refused_by_the_engine() {
        let mut backend = Headless::new(None, None).unwrap();
        backend.call("doc_new", json!({}), &client());
        for id in ["laser.start", "device.settings.set", "device.connect", "laser.frame"] {
            match backend.call("command_run", json!({"id": id, "params": {}}), &client()) {
                ToolOutput::Error { message, data } => {
                    assert!(message.starts_with("human-only: "), "{id}: {message}");
                    assert_eq!(data.unwrap()["code"], "human_only", "{id}");
                }
                _ => panic!("{id} ran from a headless origin"),
            }
        }
    }

    /// A job that is refused (here: the planner of this branch is still a stub; with I1 the rectangle outside the
    /// ±100 mm field is an `outside_work_area` refusal) is a successful tool result with ok:false.
    #[test]
    fn job_plan_refusal_is_ok_false() {
        let mut backend = Headless::new(None, None).unwrap();
        backend.call("doc_new", json!({}), &client());
        let made = backend.call("command_run", json!({"id":"tools.rectangle","params":{"x_mm":150,"y_mm":150,"w_mm":20,"h_mm":20}}), &client());
        assert!(matches!(made, ToolOutput::Json(_)));
        match backend.call("job_plan", json!({}), &client()) {
            ToolOutput::Json(value) => {
                assert_eq!(value["ok"], false);
                assert_eq!(value["summary"], Value::Null);
                assert_eq!(value["machine"], "jpt-m7-200-sim");
                assert!(value["refusals"].is_array());
            }
            _ => panic!("a refused plan must be a successful call"),
        }
    }

    #[test]
    fn job_plan_refusal_names_outside_work_area() {
        let mut backend = Headless::new(None, None).unwrap();
        backend.call("doc_new", json!({}), &client());
        backend.call("command_run", json!({"id":"tools.rectangle","params":{"x_mm":150,"y_mm":150,"w_mm":20,"h_mm":20}}), &client());
        let ToolOutput::Json(value) = backend.call("job_plan", json!({}), &client()) else { panic!("job_plan failed") };
        assert_eq!(value["ok"], false);
        let codes: Vec<&str> = value["refusals"].as_array().unwrap().iter().filter_map(|r| r["code"].as_str()).collect();
        assert!(codes.contains(&"outside_work_area"), "{codes:?}");
    }

    #[test]
    fn job_simulate_business_card() {
        let mut backend = Headless::new(None, None).unwrap();
        backend.call("doc_new", json!({}), &client());
        backend.call("command_run", json!({"id":"tools.rectangle","params":{"x_mm":-45,"y_mm":-27.5,"w_mm":90,"h_mm":55}}), &client());
        let output = backend.call("job_simulate", json!({"index":0,"listing":true}), &client());
        assert!(matches!(output, ToolOutput::Png { ref note, .. } if note["summary"].is_object() && note["listing"]["lines"].is_array()));
    }

    /// The document is rendered, not the window. Needs I1: `openlaser_render` is a stub on this branch (WP2).
    #[test]
    fn render_preview_is_document() {
        let mut backend = Headless::new(None, None).unwrap();
        backend.call("doc_new", json!({"name":"card"}), &client());
        backend.call("command_run", json!({"id":"tools.rectangle","params":{"x_mm":-10,"y_mm":-10,"w_mm":20,"h_mm":20}}), &client());
        let output = backend.call("render_preview", json!({"max_side":256}), &client());
        assert!(
            matches!(output, ToolOutput::Png { ref png, ref note } if png.starts_with(b"\x89PNG") && note["document"] == "card" && note["view"] == "design")
        );
    }

    #[test]
    fn png_budget_and_limits_are_enforced() {
        let header = |w: u32, h: u32| {
            let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
            png.extend_from_slice(&w.to_be_bytes());
            png.extend_from_slice(&h.to_be_bytes());
            png
        };
        assert!(matches!(png_output(header(10, 10), json!({})), ToolOutput::Png { .. }));
        assert!(matches!(png_output(header(9000, 10), json!({})), ToolOutput::Error { .. }));
        assert!(matches!(png_output(header(5000, 5000), json!({})), ToolOutput::Error { .. }));
        let mut big = header(10, 10);
        big.resize(budgets::MAX_PNG_BYTES + 1, 0);
        assert!(matches!(png_output(big, json!({})), ToolOutput::Error { message, .. } if message.contains("PNG budget")));
        assert!(matches!(png_output(b"not a png at all, really not".to_vec(), json!({})), ToolOutput::Error { .. }));
    }

    /// Only the rendering methods' `pngBase64` becomes an image: a command result that happens to carry such a key
    /// (or the old `__png`) is returned as JSON.
    #[test]
    fn no_png_sentinel_in_command_results() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        let mut backend = Headless::new(None, None).unwrap();
        let out = backend.call("command_schema", json!({"id": "file.new"}), &client());
        assert!(matches!(out, ToolOutput::Json(_)));
        assert!(matches!(bridged_png(json!({"pngBase64": "!!"})), ToolOutput::Error { .. }));
        assert!(matches!(bridged_png(json!({"note": 1, "__png": encoded})), ToolOutput::Json(_)));
    }

    #[test]
    fn batch_reply_budget_escaped() {
        let mut budget = BatchBudget { remaining: 11 };
        // `"a\"b"` is 6 bytes compact and 10 once escaped inside a string.
        assert!(budget.charge(&json!("a\"b")).is_ok());
        assert_eq!(budget.remaining, 0);
        let mut budget = BatchBudget { remaining: 10 };
        assert!(budget.charge(&json!("a\"b")).is_err());
    }

    #[test]
    fn batch_stops_at_the_reply_budget_but_never_skips_a_stop_step() {
        let mut backend = Headless::new(None, None).unwrap();
        backend.call("doc_new", json!({}), &client());
        let steps: Vec<Value> = (0..3)
            .map(|_| json!({"id":"edit.selectAll"}))
            .chain([json!({"id":"edit.bogus"}), json!({"id":"edit.selectNone"}), json!({"id":"laser.stop","params":{"bogus":true}})])
            .collect();
        let ToolOutput::Json(result) = backend.call("command_batch", json!({"steps": steps}), &client()) else { panic!("batch failed") };
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 5, "stop_on_error skips edit.selectNone but never the stop: {result}");
        assert_eq!(results[4]["step"], 5, "the stop step still ran: {result}");
        let mut budget = BatchBudget { remaining: 64 };
        assert!(budget.charge(&json!({"ok": true, "result": "x".repeat(100)})).is_err());
    }

    #[test]
    fn control_and_tool_keys_round_trip() {
        assert_eq!(args_to_control(json!({"max_side": 5, "center": [1, 2], "submission": "7"})), json!({"maxSide": 5, "centerMm": [1, 2], "submission": 7}));
        assert_eq!(control_key_to_arg("doc.render", "maxSide"), "max_side");
        assert_eq!(control_key_to_arg("app.import", "centerMm"), "center");
        assert_eq!(control_key_to_arg("app.import", "svgUnits"), "svg_units");
    }

    #[test]
    fn headless_stop_reports_nothing_stopped() {
        let mut backend = Headless::new(None, None).unwrap();
        let ToolOutput::Json(report) = backend.dispatch_control("machine.stop", json!({"machine": 5})) else { panic!("no report") };
        assert_eq!(report["latched"], false);
        assert_eq!(report, headless_stop_report(None));
    }
}

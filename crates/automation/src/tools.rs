//! The MCP tool table (§4.1) and its schemas. Tool arguments are snake_case; every object the server defines is
//! strict (`additionalProperties:false`); free-form payloads it only forwards (`command_run.params`, a batch step's
//! `params`, `control_call.params`, `ui_set.fields`, `tool_call.arguments`) are open objects, as in PhotoCraft,
//! because the engine or the GUI checks them strictly against the command's own schema.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Headless,
    Bridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAccess {
    Read,
    Agent,
    Approval,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hints {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// Strict: additionalProperties:false on every object the server defines, $refs inlined.
    pub input_schema: fn() -> Value,
    pub modes: &'static [Mode],
    pub access: ToolAccess,
    pub hints: Hints,
    /// Listed under --compact (`stop` always is).
    pub compact: bool,
}

impl ToolDef {
    /// "Doc open" for `doc_open`: the MCP `title`.
    pub fn title(&self) -> String {
        let words = self.name.replace('_', " ");
        let mut chars = words.chars();
        chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
    }

    /// The `tools/list` row: name, title, description, input schema and AUT A4 annotations.
    pub fn listing(&self) -> Value {
        let title = self.title();
        json!({
            "name": self.name,
            "title": title,
            "description": self.description,
            "inputSchema": (self.input_schema)(),
            "annotations": {
                "title": title,
                "readOnlyHint": self.hints.read_only,
                "destructiveHint": self.hints.destructive,
                "idempotentHint": self.hints.idempotent,
                "openWorldHint": false,
            },
        })
    }
}

const BOTH: &[Mode] = &[Mode::Headless, Mode::Bridge];
const BRIDGE: &[Mode] = &[Mode::Bridge];

/// ro/destr/idem as AUT B5.
const fn h(read_only: bool, destructive: bool, idempotent: bool) -> Hints {
    Hints { read_only, destructive, idempotent }
}
const READ: Hints = h(true, false, true);

// ---- schema builders ------------------------------------------------------------------------------------------

fn obj(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let props: Map<String, Value> = properties.iter().map(|(name, value)| ((*name).to_owned(), value.clone())).collect();
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}
fn open_object(description: &str) -> Value {
    json!({"type":"object","description":description})
}
fn s(description: &str) -> Value {
    json!({"type":"string","description":description})
}
fn one_of(values: &[&str], description: &str) -> Value {
    json!({"type":"string","enum":values,"description":description})
}
fn b(description: &str) -> Value {
    json!({"type":"boolean","description":description})
}
fn n(description: &str) -> Value {
    json!({"type":"number","description":description})
}
fn int(min: u64, max: Option<u64>, description: &str) -> Value {
    match max {
        Some(max) => json!({"type":"integer","minimum":min,"maximum":max,"description":description}),
        None => json!({"type":"integer","minimum":min,"description":description}),
    }
}
fn point(description: &str) -> Value {
    json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":description})
}
fn index() -> Value {
    int(0, None, "Document index from session_list. Default: the active document.")
}
fn machine() -> Value {
    s("Machine profile id from machine_list. Default: the document's machine.")
}
fn max_side() -> Value {
    int(1, Some(2048), "Longest image side in pixels (default 1024, at most 2048).")
}
fn path(what: &str) -> Value {
    s(&format!("{what} Relative to the automation root (or an absolute path inside it)."))
}
fn modifiers() -> Value {
    json!({"type":"array","items":{"type":"string","enum":["shift","ctrl","alt","cmd"]},"description":"Held modifier keys."})
}
fn button() -> Value {
    one_of(&["primary", "secondary", "middle"], "Mouse button (default primary).")
}

fn empty() -> Value {
    obj(&[], &[])
}
fn schema_doc_new() -> Value {
    obj(&[("name", s("Project name (default \"Untitled\").")), ("machine", s("Machine profile id (default: the first profile)."))], &[])
}
fn schema_doc_open() -> Value {
    obj(&[("path", path(".olp project, or a .svg/.dxf/.png…/workspace.json design opened as a new project."))], &["path"])
}
fn schema_doc_import() -> Value {
    obj(
        &[
            ("path", path("SVG, DXF, image or OpenMopa workspace.json to import into the active document.")),
            ("layer", s("Put every imported object on this layer (\"C00\"…). Default: colour match.")),
            ("center", point("Centre the import here, design mm [x, y].")),
            ("scale", json!({"type":"number","exclusiveMinimum":0,"description":"Scale factor (default 1)."})),
            ("svg_units", one_of(&["physical", "px96", "px72"], "How unitless SVG lengths are read (default physical: refuse them).")),
        ],
        &["path"],
    )
}
fn schema_doc_save() -> Value {
    obj(&[("path", path("Save as this .olp file. Default: the document's own path."))], &[])
}
fn schema_doc_export() -> Value {
    obj(&[("path", path("Output file.")), ("format", one_of(&["svg"], "Export format (default svg)."))], &["path"])
}
fn schema_doc_inspect() -> Value {
    obj(&[("index", index()), ("objects", one_of(&["summary", "all"], "Objects: the first 200 (summary, default) or all."))], &[])
}
fn schema_doc_select() -> Value {
    obj(&[("index", int(0, None, "Document index from session_list."))], &["index"])
}
fn schema_doc_close() -> Value {
    obj(&[("index", index())], &[])
}
fn schema_render_preview() -> Value {
    obj(&[("index", index()), ("max_side", max_side()), ("view", one_of(&["design", "toolpath"], "What to draw (default design)."))], &[])
}
fn schema_command_list() -> Value {
    obj(
        &[
            ("filter", s("Case-insensitive text in the id or label.")),
            ("enabled_only", b("Only commands enabled right now.")),
            ("agent", one_of(&["allowed", "rootsOnly", "ui", "humanOnly", "stop"], "Only rows of this agent class.")),
            ("schema", b("Include each command's parameter schema.")),
        ],
        &[],
    )
}
fn schema_command_schema() -> Value {
    obj(&[("id", s("Command id, for example layer.cutSettings.set."))], &["id"])
}
fn schema_command_run() -> Value {
    obj(
        &[
            ("id", s("Command id from command_list (camelCase, or its snake_case alias).")),
            ("params", open_object("The command's parameters, checked strictly against command_schema(id).")),
            ("wait", b("Wait for a background job the command starts (bridge).")),
        ],
        &["id"],
    )
}
fn schema_command_batch() -> Value {
    let step = obj(
        &[
            ("id", s("Command id.")),
            ("params", open_object("The command's parameters, checked strictly by the engine.")),
            ("wait", b("Wait for a background job this step starts (bridge).")),
        ],
        &["id"],
    );
    obj(
        &[
            (
                "steps",
                json!({"type":"array","items":step,"minItems":1,"maxItems":openlaser_control::budgets::BATCH_MAX_STEPS,"description":"Commands in order (at most 256)."}),
            ),
            ("stop_on_error", b("Stop at the first failing step (default true). Stop commands still run.")),
            ("atomic", b("One undo step, rolled back on any error (headless; default false).")),
            ("label", s("Undo label of an atomic batch.")),
        ],
        &["steps"],
    )
}
fn schema_jobs_cancel() -> Value {
    obj(&[("job", s("Background job id from jobs_list. Default: all of this session's jobs."))], &[])
}
fn schema_machine_status() -> Value {
    obj(&[("machine", machine())], &[])
}
fn schema_job_plan() -> Value {
    obj(&[("index", index()), ("machine", machine()), ("selected_only", b("Only the selected objects. Default: the project's Cut Selected setting."))], &[])
}
fn schema_job_preview() -> Value {
    obj(
        &[
            ("index", index()),
            ("max_side", max_side()),
            (
                "show",
                json!({"type":"array","items":{"type":"string","enum":["burn","frame","travel","order"]},"description":"Overlays (default burn and travel)."}),
            ),
        ],
        &[],
    )
}
fn schema_job_estimate() -> Value {
    obj(&[("index", index())], &[])
}
fn schema_job_simulate() -> Value {
    obj(&[("index", index()), ("max_side", max_side()), ("listing", b("Include the decoded command listing (at most 2000 rows / 256 KiB)."))], &[])
}
fn schema_job_export() -> Value {
    obj(&[("path", path("Where to write the job JSON (packets hex + listing + summary).")), ("index", index())], &["path"])
}
fn schema_gcode_export() -> Value {
    obj(
        &[
            ("path", path("Where to write the G-code.")),
            ("index", index()),
            ("machine", s("Gantry (GRBL) machine profile id. Default: the document's machine.")),
        ],
        &["path"],
    )
}
fn schema_frame_request() -> Value {
    obj(&[("mode", one_of(&["once", "loop"], "Trace the frame once (default) or until stopped.")), ("machine", machine()), ("index", index())], &[])
}
fn schema_job_submit() -> Value {
    obj(
        &[
            ("machine", machine()),
            ("index", index()),
            (
                "note",
                json!({"type":"string","maxLength":openlaser_control::budgets::NOTE_MAX_CHARS,"description":"Plain-text note shown to the person (at most 280 characters)."}),
            ),
        ],
        &[],
    )
}
fn submission(description: &str) -> Value {
    json!({"type":["string","integer"],"description":description})
}
fn schema_job_status() -> Value {
    obj(&[("submission", submission("Submission id. Default: the latest of this connection."))], &[])
}
fn schema_job_withdraw() -> Value {
    obj(&[("submission", submission("Submission id from job_submit."))], &["submission"])
}
fn schema_stop() -> Value {
    obj(&[("machine", s("Machine id. Default: every machine."))], &[])
}
fn schema_ui_elements() -> Value {
    obj(
        &[
            ("query", s("Text in the label or id.")),
            ("role", s("AccessKit role, for example button.")),
            ("limit", int(1, Some(1000), "At most this many rows.")),
        ],
        &[],
    )
}
fn schema_ui_screenshot() -> Value {
    obj(&[("max_side", max_side())], &[])
}
fn schema_ui_click() -> Value {
    obj(
        &[
            ("id", json!({"type":["string","integer"],"description":"Widget id from ui_elements (or give x, y)."})),
            ("x", n("Window point x (logical pixels).")),
            ("y", n("Window point y (logical pixels).")),
            ("button", button()),
            ("count", int(1, Some(3), "Clicks (2 = double click).")),
        ],
        &[],
    )
}
fn schema_ui_drag() -> Value {
    obj(
        &[
            ("from", point("Start, window logical pixels [x, y].")),
            ("to", point("End, window logical pixels [x, y].")),
            ("steps", int(1, Some(240), "Intermediate pointer moves.")),
            ("modifiers", modifiers()),
            ("button", button()),
        ],
        &["from", "to"],
    )
}
fn schema_ui_key() -> Value {
    obj(&[("key", s("Key name, for example Escape, Enter, ArrowLeft, A.")), ("modifiers", modifiers())], &["key"])
}
fn schema_ui_type() -> Value {
    obj(&[("text", s("Text typed into the focused field."))], &["text"])
}
fn schema_ui_pointer() -> Value {
    let event = obj(&[("kind", one_of(&["down", "move", "up"], "Pointer event.")), ("at", point("Document mm [x, y]."))], &["kind", "at"]);
    obj(
        &[
            ("events", json!({"type":"array","items":event,"minItems":1,"maxItems":1000,"description":"Canvas tool events in document mm."})),
            ("modifiers", modifiers()),
            ("button", button()),
        ],
        &["events"],
    )
}
fn schema_ui_menu_invoke() -> Value {
    obj(&[("id", s("Menu command id from ui_inspect."))], &["id"])
}
fn schema_ui_set() -> Value {
    obj(&[("fields", open_object("Field ids and values (the window accepts a closed list; human-only fields are refused)."))], &["fields"])
}
fn schema_control_call() -> Value {
    obj(&[("method", s("Control method (methods lists them).")), ("params", open_object("The method's parameters."))], &["method"])
}
fn schema_tool_search() -> Value {
    obj(
        &[
            ("query", s("Text in a tool's name or description.")),
            (
                "category",
                one_of(
                    &["session", "doc", "render", "command", "jobs", "machine", "job", "gcode", "frame", "stop", "ui", "control", "tool"],
                    "Tool name prefix.",
                ),
            ),
            ("name", s("Exact tool name: returns its full input schema.")),
        ],
        &[],
    )
}
fn schema_tool_call() -> Value {
    obj(&[("name", s("Tool name from tool_search.")), ("arguments", open_object("That tool's arguments."))], &["name"])
}

type Row = (&'static str, &'static str, fn() -> Value, &'static [Mode], ToolAccess, Hints, bool);

/// §4.1, in that order. Hints are AUT B5's ro/destr/idem; `compact` is §4.1's `--compact` list exactly.
pub fn tools() -> &'static [ToolDef] {
    static TOOLS: std::sync::OnceLock<Vec<ToolDef>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        use ToolAccess::{Agent, Approval, Read, Stop};
        let rows: &[Row] = &[
            ("session_list", "List open documents: {documents:[{index,name,path,dirty,revision,machine}], active}.", empty, BOTH, Read, READ, false),
            ("doc_new", "Create an empty project for a machine profile.", schema_doc_new, BOTH, Agent, h(false, false, false), false),
            (
                "doc_open",
                "Open a .olp project, or a design file (.svg, .dxf, image, workspace.json) as a new project.",
                schema_doc_open,
                BOTH,
                Agent,
                h(false, false, false),
                true,
            ),
            ("doc_import", "Import a design file into the active document.", schema_doc_import, BOTH, Agent, h(false, false, false), true),
            ("doc_save", "Save the active document as .olp.", schema_doc_save, BOTH, Agent, h(false, true, true), true),
            ("doc_export", "Export the active document (SVG).", schema_doc_export, BOTH, Agent, h(false, true, true), false),
            (
                "doc_inspect",
                "Document state: layers with cut settings, objects, selection, history and the live job check.",
                schema_doc_inspect,
                BOTH,
                Read,
                READ,
                true,
            ),
            ("doc_select", "Make a document active; returns session_list.", schema_doc_select, BOTH, Agent, h(false, false, false), false),
            ("doc_close", "Close a document (default: the active one); returns session_list.", schema_doc_close, BOTH, Agent, h(false, true, false), false),
            ("render_preview", "PNG of the document (never the window): design view or toolpath.", schema_render_preview, BOTH, Read, READ, true),
            ("command_list", "Commands with ids, labels, enabled state and agent class (bare array).", schema_command_list, BOTH, Read, READ, true),
            ("command_schema", "The JSON schema of one command's parameters.", schema_command_schema, BOTH, Read, READ, false),
            (
                "command_run",
                "Run one command by id (undoable with edit.undo). Human-only commands are refused.",
                schema_command_run,
                BOTH,
                Agent,
                h(false, true, false),
                true,
            ),
            (
                "command_batch",
                "Run up to 256 commands in order: {completed, failed, rolledBack, results}.",
                schema_command_batch,
                BOTH,
                Agent,
                h(false, true, false),
                true,
            ),
            ("jobs_list", "Background tasks (planning, hatching).", empty, BOTH, Read, READ, false),
            ("jobs_cancel", "Cancel a background task.", schema_jobs_cancel, BOTH, Agent, h(false, false, false), false),
            ("machine_list", "Machine profiles: id, name, kind, work area, source limits, connection.", empty, BOTH, Read, READ, false),
            (
                "machine_status",
                "Cached machine status (never causes device I/O). Headless: {connected:false, backend:\"headless\"}.",
                schema_machine_status,
                BOTH,
                Read,
                READ,
                true,
            ),
            (
                "job_plan",
                "Plan → validate → encode → summary decoded from the bytes. A refusing plan is a successful call with ok:false.",
                schema_job_plan,
                BOTH,
                Read,
                READ,
                true,
            ),
            (
                "job_preview",
                "PNG of the planned toolpath with burn/frame/travel/order overlays, plus {fingerprint, bounds}.",
                schema_job_preview,
                BOTH,
                Read,
                READ,
                true,
            ),
            ("job_estimate", "Job time estimate: {fingerprint, estimateS, perSection, crashBoundS}.", schema_job_estimate, BOTH, Read, READ, false),
            (
                "job_simulate",
                "PNG of the simulator's emission of the exact bytes, the EmissionSummary and an optional listing.",
                schema_job_simulate,
                BOTH,
                Read,
                READ,
                true,
            ),
            (
                "job_export",
                "Write the encoded job (packets hex + listing + summary JSON) for review.",
                schema_job_export,
                BOTH,
                Agent,
                h(false, true, true),
                false,
            ),
            ("gcode_export", "Write G-code for a gantry (GRBL) profile.", schema_gcode_export, BOTH, Agent, h(false, true, true), false),
            (
                "frame_request",
                "Ask the person at the machine to approve red-light framing.",
                schema_frame_request,
                BRIDGE,
                Approval,
                h(false, false, false),
                false,
            ),
            (
                "job_submit",
                "Hand the job to the person at the machine, who frames and arms it. Refusals: tool error, nothing created.",
                schema_job_submit,
                BRIDGE,
                Approval,
                h(false, false, false),
                true,
            ),
            (
                "job_status",
                "State of a submission: awaiting_human, framing, awaiting_arm, running, finished, stopped…",
                schema_job_status,
                BRIDGE,
                Read,
                READ,
                true,
            ),
            ("job_withdraw", "Withdraw a submission.", schema_job_withdraw, BRIDGE, Agent, h(false, false, true), false),
            (
                "stop",
                "Stop every machine (or one) now. Always works in bridge mode; never queued behind another call.",
                schema_stop,
                BRIDGE,
                Stop,
                h(false, false, true),
                true,
            ),
            ("ui_inspect", "Window state, including read-only humanSurfaces.", empty, BRIDGE, Read, READ, false),
            ("ui_elements", "AccessKit widgets {id, role, label, rect, enabled, humanOnly}.", schema_ui_elements, BRIDGE, Read, READ, false),
            ("ui_screenshot", "PNG of the window.", schema_ui_screenshot, BRIDGE, Read, READ, false),
            ("ui_click", "Click a widget by id or a window point (human-only widgets refuse).", schema_ui_click, BRIDGE, Agent, h(false, true, false), false),
            ("ui_drag", "Drag in the window.", schema_ui_drag, BRIDGE, Agent, h(false, true, false), false),
            ("ui_key", "Press a key.", schema_ui_key, BRIDGE, Agent, h(false, true, false), false),
            ("ui_type", "Type text into the focused field.", schema_ui_type, BRIDGE, Agent, h(false, true, false), false),
            ("ui_pointer", "Canvas tool pointer events in document mm.", schema_ui_pointer, BRIDGE, Agent, h(false, true, false), false),
            ("ui_menu_invoke", "Invoke a menu item.", schema_ui_menu_invoke, BRIDGE, Agent, h(false, true, false), false),
            ("ui_set", "Set window fields from a closed list.", schema_ui_set, BRIDGE, Agent, h(false, false, false), false),
            ("control_call", "Call a raw control method.", schema_control_call, BRIDGE, Agent, h(false, true, false), false),
            ("tool_search", "Find tools (by text, category or exact name for the full schema).", schema_tool_search, BOTH, Read, READ, true),
            ("tool_call", "Call any tool by name.", schema_tool_call, BOTH, Agent, h(false, true, false), true),
        ];
        rows.iter()
            .map(|&(name, description, input_schema, modes, access, hints, compact)| ToolDef { name, description, input_schema, modes, access, hints, compact })
            .collect()
    })
}

pub fn find(name: &str) -> Option<&'static ToolDef> {
    tools().iter().find(|tool| tool.name == name)
}

/// Closes every object schema that declares `properties` (`additionalProperties:false`), leaves free-form objects
/// (no `properties`, such as a `serde_json::Value` field) open, and inlines local `$ref`s from `$defs`/`definitions`.
pub fn strict_schema(s: schemars::Schema) -> Value {
    let root = s.to_value();
    let defs = root.get("$defs").or_else(|| root.get("definitions")).cloned().unwrap_or(Value::Null);
    let mut value = inline_refs(&root, &defs, 0);
    if let Some(map) = value.as_object_mut() {
        map.remove("$defs");
        map.remove("definitions");
    }
    close_objects(&mut value);
    value
}

fn inline_refs(value: &Value, defs: &Value, depth: usize) -> Value {
    match value {
        Value::Object(map) => {
            if let Some(target) = map.get("$ref").and_then(Value::as_str)
                && depth < 32
            {
                let name = target.rsplit('/').next().unwrap_or("");
                if let Some(def) = defs.get(name) {
                    let mut resolved = inline_refs(def, defs, depth + 1);
                    if let Some(out) = resolved.as_object_mut() {
                        for (k, v) in map.iter().filter(|(k, _)| k.as_str() != "$ref") {
                            out.entry(k.clone()).or_insert_with(|| inline_refs(v, defs, depth + 1));
                        }
                    }
                    return resolved;
                }
            }
            Value::Object(map.iter().map(|(k, v)| (k.clone(), inline_refs(v, defs, depth))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| inline_refs(v, defs, depth)).collect()),
        other => other.clone(),
    }
}

fn close_objects(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("properties") {
                map.insert("additionalProperties".to_owned(), Value::Bool(false));
            }
            for child in map.values_mut() {
                close_objects(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(close_objects),
        _ => {}
    }
}

/// Checks `value` against the subset of JSON Schema the tool table uses. The error names the argument (with its
/// path for nested ones), so the MCP layer can answer `-32602` "unknown argument `x`".
pub fn check(schema: &Value, value: &Value) -> Result<(), String> {
    check_at(schema, value, "")
}

fn join(at: &str, key: &str) -> String {
    if at.is_empty() { key.to_owned() } else { format!("{at}/{key}") }
}

fn type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        _ => true,
    }
}

fn check_at(schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let name = if at.is_empty() { "arguments".to_owned() } else { format!("argument `{at}`") };
    match schema.get("type") {
        Some(Value::String(kind)) if !type_matches(kind, value) => return Err(format!("{name} must be {kind}")),
        Some(Value::Array(kinds)) if !kinds.iter().filter_map(Value::as_str).any(|k| type_matches(k, value)) => {
            return Err(format!("{name} has the wrong type"));
        }
        _ => {}
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        let list: Vec<String> = allowed.iter().map(Value::to_string).collect();
        return Err(format!("{name} must be one of {}", list.join(", ")));
    }
    if let Some(number) = value.as_f64() {
        if schema.get("minimum").and_then(Value::as_f64).is_some_and(|min| number < min) {
            return Err(format!("{name} is below its minimum {}", schema["minimum"]));
        }
        if schema.get("maximum").and_then(Value::as_f64).is_some_and(|max| number > max) {
            return Err(format!("{name} is above its maximum {}", schema["maximum"]));
        }
        if schema.get("exclusiveMinimum").and_then(Value::as_f64).is_some_and(|min| number <= min) {
            return Err(format!("{name} must be greater than {}", schema["exclusiveMinimum"]));
        }
    }
    if let Some(text) = value.as_str()
        && let Some(max) = schema.get("maxLength").and_then(Value::as_u64)
        && u64::try_from(text.chars().count()).unwrap_or(u64::MAX) > max
    {
        return Err(format!("{name} is longer than {max} characters"));
    }
    if let Some(items) = value.as_array() {
        let len = u64::try_from(items.len()).unwrap_or(u64::MAX);
        if schema.get("minItems").and_then(Value::as_u64).is_some_and(|min| len < min) {
            return Err(format!("{name} needs at least {} items", schema["minItems"]));
        }
        if schema.get("maxItems").and_then(Value::as_u64).is_some_and(|max| len > max) {
            return Err(format!("{name} has more than {} items", schema["maxItems"]));
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                check_at(item_schema, item, &join(at, &i.to_string()))?;
            }
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if schema.get("additionalProperties") == Some(&Value::Bool(false))
            && let Some(key) = object.keys().find(|key| properties.is_none_or(|p| !p.contains_key(*key)))
        {
            return Err(format!("unknown argument `{}`", join(at, key)));
        }
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("missing required argument `{}`", join(at, key)));
                }
            }
        }
        if let Some(properties) = properties {
            for (key, child) in object {
                if let Some(child_schema) = properties.get(key) {
                    check_at(child_schema, child, &join(at, key))?;
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (name, required, optional, modes, access, compact, (ro, destr, idem)) — §4.1 and AUT B5, pinned.
    type Pin = (&'static str, &'static [&'static str], &'static [&'static str], &'static [Mode], ToolAccess, bool, (bool, bool, bool));

    const H_B: &[Mode] = &[Mode::Headless, Mode::Bridge];
    const B: &[Mode] = &[Mode::Bridge];

    const SPEC: &[Pin] = &[
        ("session_list", &[], &[], H_B, ToolAccess::Read, false, (true, false, true)),
        ("doc_new", &[], &["name", "machine"], H_B, ToolAccess::Agent, false, (false, false, false)),
        ("doc_open", &["path"], &[], H_B, ToolAccess::Agent, true, (false, false, false)),
        ("doc_import", &["path"], &["layer", "center", "scale", "svg_units"], H_B, ToolAccess::Agent, true, (false, false, false)),
        ("doc_save", &[], &["path"], H_B, ToolAccess::Agent, true, (false, true, true)),
        ("doc_export", &["path"], &["format"], H_B, ToolAccess::Agent, false, (false, true, true)),
        ("doc_inspect", &[], &["index", "objects"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("doc_select", &["index"], &[], H_B, ToolAccess::Agent, false, (false, false, false)),
        ("doc_close", &[], &["index"], H_B, ToolAccess::Agent, false, (false, true, false)),
        ("render_preview", &[], &["index", "max_side", "view"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("command_list", &[], &["filter", "enabled_only", "agent", "schema"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("command_schema", &["id"], &[], H_B, ToolAccess::Read, false, (true, false, true)),
        ("command_run", &["id"], &["params", "wait"], H_B, ToolAccess::Agent, true, (false, true, false)),
        ("command_batch", &["steps"], &["stop_on_error", "atomic", "label"], H_B, ToolAccess::Agent, true, (false, true, false)),
        ("jobs_list", &[], &[], H_B, ToolAccess::Read, false, (true, false, true)),
        ("jobs_cancel", &[], &["job"], H_B, ToolAccess::Agent, false, (false, false, false)),
        ("machine_list", &[], &[], H_B, ToolAccess::Read, false, (true, false, true)),
        ("machine_status", &[], &["machine"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("job_plan", &[], &["index", "machine", "selected_only"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("job_preview", &[], &["index", "max_side", "show"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("job_estimate", &[], &["index"], H_B, ToolAccess::Read, false, (true, false, true)),
        ("job_simulate", &[], &["index", "max_side", "listing"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("job_export", &["path"], &["index"], H_B, ToolAccess::Agent, false, (false, true, true)),
        ("gcode_export", &["path"], &["index", "machine"], H_B, ToolAccess::Agent, false, (false, true, true)),
        ("frame_request", &[], &["mode", "machine", "index"], B, ToolAccess::Approval, false, (false, false, false)),
        ("job_submit", &[], &["machine", "index", "note"], B, ToolAccess::Approval, true, (false, false, false)),
        ("job_status", &[], &["submission"], B, ToolAccess::Read, true, (true, false, true)),
        ("job_withdraw", &["submission"], &[], B, ToolAccess::Agent, false, (false, false, true)),
        ("stop", &[], &["machine"], B, ToolAccess::Stop, true, (false, false, true)),
        ("ui_inspect", &[], &[], B, ToolAccess::Read, false, (true, false, true)),
        ("ui_elements", &[], &["query", "role", "limit"], B, ToolAccess::Read, false, (true, false, true)),
        ("ui_screenshot", &[], &["max_side"], B, ToolAccess::Read, false, (true, false, true)),
        ("ui_click", &[], &["id", "x", "y", "button", "count"], B, ToolAccess::Agent, false, (false, true, false)),
        ("ui_drag", &["from", "to"], &["steps", "modifiers", "button"], B, ToolAccess::Agent, false, (false, true, false)),
        ("ui_key", &["key"], &["modifiers"], B, ToolAccess::Agent, false, (false, true, false)),
        ("ui_type", &["text"], &[], B, ToolAccess::Agent, false, (false, true, false)),
        ("ui_pointer", &["events"], &["modifiers", "button"], B, ToolAccess::Agent, false, (false, true, false)),
        ("ui_menu_invoke", &["id"], &[], B, ToolAccess::Agent, false, (false, true, false)),
        ("ui_set", &["fields"], &[], B, ToolAccess::Agent, false, (false, false, false)),
        ("control_call", &["method"], &["params"], B, ToolAccess::Agent, false, (false, true, false)),
        ("tool_search", &[], &["query", "category", "name"], H_B, ToolAccess::Read, true, (true, false, true)),
        ("tool_call", &["name"], &["arguments"], H_B, ToolAccess::Agent, true, (false, true, false)),
    ];

    #[test]
    fn tools_table_matches_spec() {
        let names: Vec<_> = tools().iter().map(|tool| tool.name).collect();
        let pinned: Vec<_> = SPEC.iter().map(|pin| pin.0).collect();
        assert_eq!(names, pinned, "tool order (§4.1)");
        for (tool, &(name, required, optional, modes, access, compact, (ro, destr, idem))) in tools().iter().zip(SPEC) {
            let schema = (tool.input_schema)();
            let mut properties: Vec<&str> = schema["properties"].as_object().unwrap().keys().map(String::as_str).collect();
            properties.sort_unstable();
            let mut expected: Vec<&str> = required.iter().chain(optional).copied().collect();
            expected.sort_unstable();
            assert_eq!(properties, expected, "{name}: arguments");
            let mut got_required: Vec<&str> = schema["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            got_required.sort_unstable();
            let mut want_required = required.to_vec();
            want_required.sort_unstable();
            assert_eq!(got_required, want_required, "{name}: required arguments");
            assert_eq!(tool.modes, modes, "{name}: modes");
            assert_eq!(tool.access, access, "{name}: access");
            assert_eq!(tool.compact, compact, "{name}: --compact");
            assert_eq!(tool.hints, Hints { read_only: ro, destructive: destr, idempotent: idem }, "{name}: hints");
        }
    }

    #[test]
    fn compact_lists_stop() {
        let compact: Vec<_> = tools().iter().filter(|tool| tool.compact).map(|tool| tool.name).collect();
        assert_eq!(
            compact,
            [
                "doc_open",
                "doc_import",
                "doc_save",
                "doc_inspect",
                "render_preview",
                "command_list",
                "command_run",
                "command_batch",
                "machine_status",
                "job_plan",
                "job_preview",
                "job_simulate",
                "job_submit",
                "job_status",
                "stop",
                "tool_search",
                "tool_call",
            ]
        );
    }

    fn walk<'a>(value: &'a Value, out: &mut Vec<&'a serde_json::Map<String, Value>>) {
        if let Some(object) = value.as_object() {
            out.push(object);
            object.values().for_each(|child| walk(child, out));
        } else if let Some(array) = value.as_array() {
            array.iter().for_each(|child| walk(child, out));
        }
    }

    #[test]
    fn schemas_have_no_override_fields() {
        for tool in tools() {
            let schema = (tool.input_schema)();
            let mut objects = Vec::new();
            walk(&schema, &mut objects);
            for object in objects {
                if object.contains_key("properties") {
                    assert_eq!(object.get("additionalProperties"), Some(&Value::Bool(false)), "{}: an object with properties is open", tool.name);
                }
                for name in object.get("properties").and_then(Value::as_object).into_iter().flat_map(|p| p.keys()) {
                    assert!(
                        !matches!(name.as_str(), "force" | "proceed" | "arm" | "confirm" | "skip_frame" | "override" | "token"),
                        "{}: forbidden property `{name}`",
                        tool.name
                    );
                }
            }
        }
    }

    #[test]
    fn free_form_payloads_stay_open() {
        let open = |tool: &str, pointer: &str| {
            let schema = (find(tool).unwrap().input_schema)();
            let node = schema.pointer(pointer).unwrap_or_else(|| panic!("{tool}{pointer}"));
            assert_eq!(node["type"], "object", "{tool}{pointer}");
            assert!(node.get("additionalProperties").is_none() && node.get("properties").is_none(), "{tool}{pointer} is closed");
        };
        open("command_run", "/properties/params");
        open("command_batch", "/properties/steps/items/properties/params");
        open("control_call", "/properties/params");
        open("ui_set", "/properties/fields");
        open("tool_call", "/properties/arguments");
        let run = (find("command_run").unwrap().input_schema)();
        assert!(check(&run, &json!({"id":"tools.rectangle","params":{"x_mm":1,"anything":{"nested":true}}})).is_ok());
        let batch = (find("command_batch").unwrap().input_schema)();
        assert!(check(&batch, &json!({"steps":[{"id":"a","params":{"free":1}}]})).is_ok());
        assert_eq!(check(&batch, &json!({"steps":[{"id":"a","bogus":1}]})), Err("unknown argument `steps/0/bogus`".to_owned()));
    }

    #[test]
    fn arguments_are_checked_by_type_enum_and_range() {
        let preview = (find("render_preview").unwrap().input_schema)();
        assert!(check(&preview, &json!({"max_side":2048,"view":"toolpath"})).is_ok());
        assert!(check(&preview, &json!({"max_side":2049})).is_err());
        assert!(check(&preview, &json!({"view":"window"})).is_err());
        assert!(check(&preview, &json!({"index":"0"})).is_err());
        assert_eq!(check(&preview, &json!({"show":["burn"]})), Err("unknown argument `show`".to_owned()));
        let open = (find("doc_open").unwrap().input_schema)();
        assert_eq!(check(&open, &json!({"path":"a.olp","index":0})), Err("unknown argument `index`".to_owned()));
        assert_eq!(check(&open, &json!({})), Err("missing required argument `path`".to_owned()));
        let submit = (find("job_submit").unwrap().input_schema)();
        assert!(check(&submit, &json!({"note":"x".repeat(281)})).is_err());
        assert!(check(&submit, &json!({"note":"x".repeat(280)})).is_ok());
        assert!(check(&preview, &json!([])).is_err(), "arguments must be an object");
    }

    #[test]
    fn strict_schema_closes_typed_objects_and_inlines_refs() {
        #[derive(schemars::JsonSchema)]
        #[allow(dead_code)]
        struct Inner {
            x: f64,
        }
        #[derive(schemars::JsonSchema)]
        #[allow(dead_code)]
        struct Outer {
            inner: Inner,
            free: serde_json::Value,
            map: std::collections::BTreeMap<String, f64>,
        }
        let schema = strict_schema(schemars::schema_for!(Outer));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["properties"]["inner"]["additionalProperties"], json!(false));
        assert!(schema.get("$defs").is_none());
        assert!(!schema.to_string().contains("$ref"));
        assert!(schema["properties"]["free"].get("additionalProperties").is_none());
    }
}

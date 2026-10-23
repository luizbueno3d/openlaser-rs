//! The MCP tool table (§4.1). Tool arguments are snake_case and schemas reject unknown fields.

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
    /// Strict: additionalProperties:false everywhere, $refs inlined.
    pub input_schema: fn() -> serde_json::Value,
    pub modes: &'static [Mode],
    pub access: ToolAccess,
    pub hints: Hints,
    /// Listed under --compact (`stop` always is).
    pub compact: bool,
}

const BOTH: &[Mode] = &[Mode::Headless, Mode::Bridge];
const BRIDGE: &[Mode] = &[Mode::Bridge];
const READ: Hints = Hints { read_only: true, destructive: false, idempotent: true };
const EDIT: Hints = Hints { read_only: false, destructive: true, idempotent: false };
const STOP_HINTS: Hints = Hints { read_only: false, destructive: false, idempotent: true };
type ToolTuple = (&'static str, &'static str, &'static [Mode], ToolAccess, Hints, bool);

fn object_schema() -> serde_json::Value {
    schema(&[], &[])
}

fn schema(properties: &[(&str, serde_json::Value)], required: &[&str]) -> serde_json::Value {
    let props: serde_json::Map<String, serde_json::Value> = properties.iter().map(|(name, value)| ((*name).to_owned(), value.clone())).collect();
    let mut value = serde_json::json!({"type":"object","properties":props,"required":required,"additionalProperties":false});
    strict_value(&mut value);
    value
}
fn p_string() -> serde_json::Value {
    serde_json::json!({"type":"string"})
}
fn p_bool() -> serde_json::Value {
    serde_json::json!({"type":"boolean"})
}
fn p_index() -> serde_json::Value {
    serde_json::json!({"type":"integer","minimum":0})
}
fn p_number() -> serde_json::Value {
    serde_json::json!({"type":"number"})
}
fn schema_by_name(name: &str) -> fn() -> serde_json::Value {
    match name {
        "doc_new" => schema_doc_new,
        "doc_open" | "doc_export" | "job_export" | "gcode_export" => schema_path,
        "doc_save" => schema_save,
        "doc_import" => schema_import,
        "doc_inspect" => schema_inspect,
        "doc_select" | "doc_close" => schema_index,
        "render_preview" | "job_preview" => schema_preview,
        "command_list" => schema_command_list,
        "command_schema" => schema_id,
        "command_run" => schema_run,
        "command_batch" => schema_batch,
        "jobs_cancel" => schema_job_cancel,
        "machine_status" | "stop" => schema_machine,
        "job_plan" => schema_plan,
        "job_simulate" => schema_simulate,
        "frame_request" => schema_frame_request,
        "job_submit" => schema_submit,
        "job_status" => schema_submission,
        "job_withdraw" => schema_withdraw,
        "ui_elements" => schema_ui_elements,
        "ui_screenshot" => schema_preview,
        "ui_click" => schema_ui_click,
        "ui_drag" => schema_ui_drag,
        "ui_key" | "ui_type" | "ui_menu_invoke" => schema_ui_id,
        "ui_pointer" => schema_ui_pointer,
        "ui_set" => schema_ui_set,
        "control_call" => schema_control_call,
        "tool_search" => schema_search,
        "tool_call" => schema_tool_call,
        _ => object_schema,
    }
}
fn schema_doc_new() -> serde_json::Value {
    schema(&[("name", p_string()), ("machine", p_string())], &[])
}
fn schema_path() -> serde_json::Value {
    schema(&[("path", p_string()), ("index", p_index()), ("format", p_string())], &["path"])
}
fn schema_save() -> serde_json::Value {
    schema(&[("path", p_string())], &[])
}
fn schema_import() -> serde_json::Value {
    schema(
        &[
            ("path", p_string()),
            ("layer", p_string()),
            ("center", serde_json::json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2})),
            ("scale", p_number()),
            ("svg_units", p_string()),
        ],
        &["path"],
    )
}
fn schema_inspect() -> serde_json::Value {
    schema(&[("index", p_index()), ("objects", p_string())], &[])
}
fn schema_index() -> serde_json::Value {
    schema(&[("index", p_index())], &["index"])
}
fn schema_preview() -> serde_json::Value {
    schema(
        &[
            ("index", p_index()),
            ("max_side", serde_json::json!({"type":"integer","minimum":1,"maximum":2048})),
            ("view", p_string()),
            ("show", serde_json::json!({"type":"array","items":{"type":"string"}})),
        ],
        &[],
    )
}
fn schema_command_list() -> serde_json::Value {
    schema(&[("filter", p_string()), ("enabled_only", p_bool()), ("agent", p_string()), ("schema", p_bool())], &[])
}
fn schema_id() -> serde_json::Value {
    schema(&[("id", p_string())], &["id"])
}
fn schema_run() -> serde_json::Value {
    schema(&[("id", p_string()), ("params", serde_json::json!({"type":"object"})), ("wait", p_bool())], &["id"])
}
fn schema_batch() -> serde_json::Value {
    schema(
        &[
            (
                "steps",
                serde_json::json!({"type":"array","maxItems":256,"items":{"type":"object","properties":{"id":p_string(),"params":{"type":"object"},"wait":p_bool()},"required":["id"],"additionalProperties":false}}),
            ),
            ("stop_on_error", p_bool()),
            ("atomic", p_bool()),
            ("label", p_string()),
        ],
        &["steps"],
    )
}
fn schema_job_cancel() -> serde_json::Value {
    schema(&[("job", p_string())], &[])
}
fn schema_machine() -> serde_json::Value {
    schema(&[("machine", p_string())], &[])
}
fn schema_plan() -> serde_json::Value {
    schema(&[("index", p_index()), ("machine", p_string()), ("selected_only", p_bool())], &[])
}
fn schema_simulate() -> serde_json::Value {
    schema(&[("index", p_index()), ("max_side", p_index()), ("listing", p_bool())], &[])
}
fn schema_frame_request() -> serde_json::Value {
    schema(&[("mode", p_string()), ("machine", p_string()), ("index", p_index())], &[])
}
fn schema_submit() -> serde_json::Value {
    schema(&[("machine", p_string()), ("index", p_index()), ("note", serde_json::json!({"type":"string","maxLength":280}))], &[])
}
fn schema_submission() -> serde_json::Value {
    schema(&[("submission", p_string())], &[])
}
fn schema_withdraw() -> serde_json::Value {
    schema(&[("submission", p_string())], &["submission"])
}
fn schema_ui_elements() -> serde_json::Value {
    schema(&[("query", p_string()), ("role", p_string()), ("limit", p_index())], &[])
}
fn schema_ui_click() -> serde_json::Value {
    schema(&[("id", p_string()), ("x", p_number()), ("y", p_number()), ("button", p_string()), ("count", p_index())], &[])
}
fn schema_ui_drag() -> serde_json::Value {
    schema(
        &[
            ("from", serde_json::json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2})),
            ("to", serde_json::json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2})),
        ],
        &[],
    )
}
fn schema_ui_id() -> serde_json::Value {
    schema(&[("id", p_string())], &["id"])
}
fn schema_ui_pointer() -> serde_json::Value {
    schema(&[("x", p_number()), ("y", p_number()), ("action", p_string())], &[])
}
fn schema_ui_set() -> serde_json::Value {
    schema(&[("fields", serde_json::json!({"type":"object"}))], &["fields"])
}
fn schema_control_call() -> serde_json::Value {
    schema(&[("method", p_string()), ("params", serde_json::json!({"type":"object"}))], &["method"])
}
fn schema_search() -> serde_json::Value {
    schema(&[("query", p_string())], &["query"])
}
fn schema_tool_call() -> serde_json::Value {
    schema(&[("name", p_string()), ("arguments", serde_json::json!({"type":"object"}))], &["name"])
}

/// §4.1, in that order.
pub fn tools() -> &'static [ToolDef] {
    static TOOLS: std::sync::OnceLock<Vec<ToolDef>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        let defs: &[ToolTuple] = &[
            ("session_list", "List open documents", BOTH, ToolAccess::Read, READ, true),
            ("doc_new", "Create a document", BOTH, ToolAccess::Agent, EDIT, false),
            ("doc_open", "Open a project or supported design file", BOTH, ToolAccess::Agent, EDIT, true),
            ("doc_import", "Import design objects", BOTH, ToolAccess::Agent, EDIT, false),
            ("doc_save", "Save a project", BOTH, ToolAccess::Agent, EDIT, true),
            ("doc_export", "Export a document", BOTH, ToolAccess::Agent, EDIT, false),
            ("doc_inspect", "Inspect document contents", BOTH, ToolAccess::Read, READ, true),
            ("doc_select", "Select a document", BOTH, ToolAccess::Agent, EDIT, false),
            ("doc_close", "Close a document", BOTH, ToolAccess::Agent, EDIT, false),
            ("render_preview", "Render a document preview", BOTH, ToolAccess::Read, READ, true),
            ("command_list", "List registered commands", BOTH, ToolAccess::Read, READ, true),
            ("command_schema", "Get a command parameter schema", BOTH, ToolAccess::Read, READ, false),
            ("command_run", "Run an agent-allowed command", BOTH, ToolAccess::Agent, EDIT, false),
            ("command_batch", "Run a batch of commands", BOTH, ToolAccess::Agent, EDIT, false),
            ("jobs_list", "List background jobs", BOTH, ToolAccess::Read, READ, false),
            ("jobs_cancel", "Cancel a background job", BOTH, ToolAccess::Agent, EDIT, false),
            ("machine_list", "List machine profiles", BOTH, ToolAccess::Read, READ, true),
            ("machine_status", "Read cached machine status", BOTH, ToolAccess::Read, READ, true),
            ("job_plan", "Plan and validate a job", BOTH, ToolAccess::Read, READ, true),
            ("job_preview", "Render the planned toolpath", BOTH, ToolAccess::Read, READ, false),
            ("job_estimate", "Estimate job duration", BOTH, ToolAccess::Read, READ, false),
            ("job_simulate", "Simulate exact job bytes", BOTH, ToolAccess::Read, READ, false),
            ("job_export", "Export encoded job bytes for review", BOTH, ToolAccess::Agent, EDIT, false),
            ("gcode_export", "Export gantry G-code", BOTH, ToolAccess::Agent, EDIT, false),
            ("frame_request", "Request human-approved framing", BRIDGE, ToolAccess::Approval, EDIT, false),
            ("job_submit", "Submit a job for human review", BRIDGE, ToolAccess::Approval, EDIT, true),
            ("job_status", "Read job submission status", BRIDGE, ToolAccess::Read, READ, true),
            ("job_withdraw", "Withdraw a submitted job", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("stop", "Stop active machine work immediately", BRIDGE, ToolAccess::Stop, STOP_HINTS, true),
            ("ui_inspect", "Inspect the running window", BRIDGE, ToolAccess::Read, READ, false),
            ("ui_elements", "List accessible UI elements", BRIDGE, ToolAccess::Read, READ, false),
            ("ui_screenshot", "Capture the running window", BRIDGE, ToolAccess::Read, READ, false),
            ("ui_click", "Click a non-human-only UI element", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("ui_drag", "Drag a UI element", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("ui_key", "Send a UI key", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("ui_type", "Type into a UI field", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("ui_pointer", "Move the UI pointer", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("ui_menu_invoke", "Invoke an allowed menu command", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("ui_set", "Set allowed UI fields", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("control_call", "Call a raw control method", BRIDGE, ToolAccess::Agent, EDIT, false),
            ("tool_search", "Search the available tools", BOTH, ToolAccess::Read, READ, true),
            ("tool_call", "Call a tool by name", BOTH, ToolAccess::Agent, EDIT, true),
        ];
        defs.iter()
            .map(|(name, description, modes, access, hints, compact)| ToolDef {
                name,
                description,
                input_schema: schema_by_name(name),
                modes,
                access: *access,
                hints: *hints,
                compact: *compact,
            })
            .collect()
    })
}

/// Closes every object schema and inlines local definitions before sending to MCP clients.
pub fn strict_schema(s: schemars::Schema) -> serde_json::Value {
    let mut value = s.to_value();
    strict_value(&mut value);
    value
}

fn strict_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("type").and_then(serde_json::Value::as_str) == Some("object") || map.contains_key("properties") {
                map.insert("additionalProperties".to_owned(), serde_json::Value::Bool(false));
            }
            for child in map.values_mut() {
                strict_value(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                strict_value(child);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_table_matches_spec_and_compact_always_contains_stop() {
        let names: Vec<_> = tools().iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            [
                "session_list",
                "doc_new",
                "doc_open",
                "doc_import",
                "doc_save",
                "doc_export",
                "doc_inspect",
                "doc_select",
                "doc_close",
                "render_preview",
                "command_list",
                "command_schema",
                "command_run",
                "command_batch",
                "jobs_list",
                "jobs_cancel",
                "machine_list",
                "machine_status",
                "job_plan",
                "job_preview",
                "job_estimate",
                "job_simulate",
                "job_export",
                "gcode_export",
                "frame_request",
                "job_submit",
                "job_status",
                "job_withdraw",
                "stop",
                "ui_inspect",
                "ui_elements",
                "ui_screenshot",
                "ui_click",
                "ui_drag",
                "ui_key",
                "ui_type",
                "ui_pointer",
                "ui_menu_invoke",
                "ui_set",
                "control_call",
                "tool_search",
                "tool_call",
            ]
        );
        assert!(tools().iter().find(|tool| tool.name == "stop").is_some_and(|tool| tool.compact && tool.access == ToolAccess::Stop));
    }

    #[test]
    fn schemas_are_strict_and_have_no_override_properties() {
        fn scan(value: &serde_json::Value) {
            if let Some(object) = value.as_object() {
                if object.get("type").and_then(serde_json::Value::as_str) == Some("object") {
                    assert_eq!(object.get("additionalProperties"), Some(&serde_json::Value::Bool(false)));
                }
                if let Some(properties) = object.get("properties").and_then(serde_json::Value::as_object) {
                    for name in properties.keys() {
                        assert!(!matches!(name.as_str(), "force" | "proceed" | "arm" | "confirm" | "skip_frame" | "override" | "token"));
                    }
                }
                for child in object.values() {
                    scan(child);
                }
            } else if let Some(array) = value.as_array() {
                for child in array {
                    scan(child);
                }
            }
        }
        for tool in tools() {
            scan(&(tool.input_schema)());
        }
    }
}

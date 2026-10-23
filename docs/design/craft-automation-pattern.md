# Craft automation pattern, and OpenLaser's agent interface

*2026-10-23. Design note for `openlaser-cli mcp` and bridge mode. Part A reverse-engineers how the
Craft apps (PhotoCraft, PdfCraft, with FilmCraft and VectorCraft as observed from their live MCP
servers) expose themselves to agents. Part B is the OpenLaser design that copies it. Part C is the
emission safety boundary. Nothing here was built or run; every Craft fact cites the source file it
was read from.*

Sources read (read only, nothing built):
- PhotoCraft `crates/automation/src/{lib,server,server/transport,server/conventions,server/progress,rpc,bridge,headless,security,budgets,files,workspace}.rs`, `apps/photocraft-cli/src/{lib,main}.rs`, `apps/photocraft/src/{control_server,assistant_bridge,main}.rs`, `crates/ui-egui/src/{control,lib}.rs` (relevant parts), `crates/engine/src/{commands,lib}.rs` (registry, `Session::edit`), `crates/automation/tests/mcp_wire.rs`, `docs/{mcp,control-protocol,development}.md`.
- PdfCraft `crates/automation/src/{mcp,conventions,tools,lib}.rs` (relevant parts), `apps/pdfcraft-cli/src/main.rs`, `crates/ui-egui/src/control.rs` (header), `docs/mcp.md`.
- `~/AI/PDF_Craft_Edit/craft-assistant-plan.md`; egui 0.36.2 `response.rs`/`context.rs` (click provenance).
- OpenLaser plan: `~/AI/ClaudeCode/openlaser-rust-plan/PLAN.md` §3.1, §3.3–§3.6, §5, §6.4, §7; research 02 §16 (LightBurn automation), research 05 §1.6, §4.4, H11.
- Luiz's `~/.claude.json` MCP entries (command lines only).

Crate and binary names below follow PLAN §3.1 (`ol-*`). If `docs/SPEC.md` renames crates, the
dependency rules carry over unchanged.

---

## 0. Summary

**What the Craft apps do.** Each app has one automation crate that serves the same tool set two
ways: **headless** (an in-process engine session behind an MCP server on stdio, started as
`<app>-cli mcp`) and **bridge** (the same MCP server forwarding every tool to a running GUI over an
authenticated loopback JSON-lines "control protocol"). Everything an agent does is an engine
command found by id in a registry (`command_list` → `command_run` / `command_batch`), plus a few
document tools (`doc_*`), a bounded PNG preview, and, in bridge mode only, `ui_*` tools that inspect,
screenshot and drive the live window. File access fails closed unless launch-time read/write roots
are granted, and is enforced with directory capabilities (`cap-std`), not path string checks.

**What OpenLaser copies.** The two modes, the tool names and shapes (`command_list`,
`command_run`, `command_batch`, `doc_inspect`, `render_preview`, `doc_*`, `jobs_*`, `ui_*`,
`control_call`), the control protocol envelope, token auth, the root model, the budgets and the
error model. It adds laser tools: `machine_list`, `machine_status`, `job_plan`, `job_preview`,
`job_estimate`, `job_simulate`, `job_export`, `gcode_export`, `frame_request`, `job_submit`,
`job_status`, `job_withdraw`, `stop`.

**The boundary, in one sentence.** Agents may do everything up to the burn (design, import, edit
layers and cut settings, plan, preview, estimate, simulate the exact bytes on the emulator, export,
submit a job, watch status, stop), but **no protocol message, tool, command id, preference or
synthetic input can start emission**: arming exists only as a human-only surface in the GUI (typed
`ARM` in M1–M2, the foot switch from M3) and the TTY flow in a separate hardware binary, and both
mint the ARM token in a crate that the automation stack cannot link. Red-light framing requested by
an agent runs only after a human click on an approval card. `stop` always works, from anyone.

This is LightBurn's own stance: its 2.2 REST API "deliberately cannot start jobs; the operator
always starts it" (research 02, AUT-02).

---

## Part A. How the Craft family does it

### A1. Processes and pieces

```
                         stdio (newline-delimited JSON-RPC 2.0, MCP)
 agent (Claude Code) ───────────────────────────────► photocraft-cli mcp ...
                                                         │
                     ┌───────────────────────────────────┴──────────────────────┐
                     │ Backend::Headless                    Backend::Bridge      │
                     │ Arc<Mutex<Headless>>                 BridgeClient          │
                     │  └ photocraft_engine::Session         └ TCP 127.0.0.1:port │
                     │  └ AuthorizedWorkspace (cap-std)         JSON lines + auth │
                     └──────────────────────────────────────────────┬───────────┘
                                                                    │
  photocraft --control <port> --control-token-file <f> --automation-read-root ... (GUI)
     control_server.rs: listener thread, one thread per connection (≤ 16)
        auth frame → ControlRequest{method, params, reply, deadline, dispatch} → mpsc
     ui-egui lib.rs: drain_control() between frames on the UI thread
        control::handle() → Outcome::{Done | AfterInput | AfterJob | Screenshot}
        synthetic egui events queued in app.synthetic, injected by raw_input_hook,
        frame marked automation_input = true
```

- `photocraft-automation` (`crates/automation`): the MCP server (`PhotocraftMcp`, rmcp), the
  headless core (`Headless`), the headless JSON-lines server (`rpc.rs`, used by `photocraft-cli
  serve`), the bridge client (`bridge.rs`), shared security primitives (`security.rs`), output
  ceilings (`budgets.rs`), file I/O (`files.rs`) and the capability filesystem (`workspace.rs`).
  No UI-toolkit dependency (`lib.rs`: "L6, no UI-toolkit dependencies"). `#![forbid(unsafe_code)]`
  and `deny(clippy::unwrap_used, expect_used, panic, ...)`.
- The GUI never runs an MCP server itself. Its transport (`apps/photocraft/src/control_server.rs`)
  is plain `std::net` + threads; handlers live in `crates/ui-egui/src/control.rs`.
- **In-app Assistant** (`apps/photocraft/src/assistant_bridge.rs`, `craft-assistant-plan.md`):
  Window › Assistant starts the control channel on demand (port 0 = free port, a fresh 256-bit token
  written to an owner-only file `assistant-control.token`, reused if the app was launched with
  `--control`) and gives the agent the command line `PhotoCraft --mcp-bridge 127.0.0.1:<port>
  --control-token-file <path>`: the app binary re-executes itself as the bridged MCP server before
  any window opens. The agent is reached over ACP (Agent Client Protocol). Without `--control`
  the assistant's workspace has no roots, so file opens/saves fail closed.

Luiz's registrations (`~/.claude.json`), for reference:

| Server | Command line |
|---|---|
| photocraft | `photocraft-cli mcp --automation-read-root ~/Craft --automation-write-root ~/Craft` |
| vectorcraft | `vectorcraft-cli mcp --automation-read-root ~/Craft --automation-write-root ~/Craft` |
| pdfcraft | `pdfcraft-cli mcp --root ~/Craft --compact` |
| filmcraft | `filmcraft-cli mcp` |

PhotoCraft's `docs/development.md` recommends a second entry for the live app, `photocraft-live`:
`photocraft-cli mcp --bridge 127.0.0.1:7878 --control-token-file <private-path>`.

### A2. MCP transport and protocol

**PhotoCraft** (official Rust SDK `rmcp` 3.5, features `server, macros, transport-io`; tokio):
- **Framing:** newline-delimited JSON-RPC 2.0 on stdin/stdout. `serve_io` (`server/transport.rs`)
  puts its own reader in front of rmcp: `bounded_line` reads a whole line (max
  `MAX_REQUEST_BYTES` = 1 MiB) before forwarding any byte, so a valid JSON prefix can never execute
  after an oversized suffix is rejected. An oversized line **ends the session** with "request
  exceeds 1048576 bytes" (test `oversized_valid_tool_call_never_reaches_dispatch`). A malformed line
  gets `{"jsonrpc":"2.0","id":null,"error":{"code":-32700,...}}` and the next line is still served.
  A single writer task serializes parse errors and SDK replies. (`docs/control-protocol.md` still
  says MCP stdio does not cap request bytes; the code does. Code is the truth.)
- **Lifecycle:** `initialize` (tests send `protocolVersion: "2025-06-18"`), `notifications/initialized`,
  `ping`. `get_info` advertises capabilities `tools` + `resources`, server info
  `{name:"photocraft", version}`, and an `instructions` string (see A3).
- **Newer protocol fields:** requests carrying `_meta` keys `io.modelcontextprotocol/protocolVersion:
  "2026-07-28"` are accepted; `tools/list` and `resources/list` reply with `resultType:"complete"`,
  `ttlMs:600000`, `cacheScope:"private"`; `resources/read` with `ttlMs:0` (`server/conventions.rs`,
  test `malformed_line_recovers_and_modern_lists_are_complete`).
- **Tools:** `tools/list`, `tools/call`. **Resources:** `photocraft://document` (live `doc_inspect`)
  and `photocraft://commands` (live `command_list`), `application/json`. **No prompts, no resource
  templates.**
- **Progress and cancellation:** only headless `command_run` of `file.export.renderVideo`:
  `notifications/progress` when `_meta.progressToken` is present, strictly increasing, at most 10/s
  plus completion, with `total`; `notifications/cancelled` stops at a frame boundary, removes only the
  files that job created, and suppresses the response (`server/progress.rs`, `docs/mcp.md`).
- **Concurrency:** every headless call runs `spawn_blocking` on one `Arc<Mutex<Headless>>`; a
  panic poisons the lock but the session keeps serving (`PoisonError::into_inner`), because edits run
  on a copy of the document.

**PdfCraft** (hand-rolled, no rmcp, `crates/automation/src/mcp.rs`, feature `mcp` default on):
- Same stdio framing, `serve` loops over `lines()` (no byte cap). Versions offered:
  `["2025-06-18", "2025-03-26", "2024-11-05"]`, echoing the client's if supported.
- `initialize` → capabilities `tools{listChanged:false}`, `resources{listChanged:false,
  subscribe:false}`, `serverInfo{name, title, version, websiteUrl}`, `instructions`.
- Methods: `initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`,
  `resources/templates/list`, `resources/read`; notifications get no reply.
- Resources: `pdfcraft://document`, `pdfcraft://commands`, plus per open document
  `pdfcraft://doc/{doc}/info|text|page/{p}/text|page/{p}/image{?dpi}` and their templates.
- **`--compact`:** `tools/list` returns a core set (15 tools) plus two meta tools, `tool_search
  {query?, category?, name?}` (name → full `input_schema`) and `tool_call {name, arguments}`;
  `tools/call` accepts every tool by name either way.
- Results: JSON objects become pretty text content **and** `structuredContent`; PNGs become image
  content. A `command_batch` with failures is marked `isError:true`.

**Observed from the other two live servers** (their sources are not on this Mac; tool names come from
the MCP tool registry in this session): FilmCraft lists `command_list/run/batch`, `doc_inspect`,
`project_inspect`, `sequence_inspect`, `media_import`, `render_preview`, `render_frame`,
`ui_elements`, `ui_click`, `ui_drag`, `ui_key`, `ui_type`, `ui_control`, `ui_inspect`,
`ui_screenshot`. VectorCraft's instructions mention `prompts/list` ("ready-made workflows"),
`completion/complete`, and resource templates `vectorcraft://object/{id}`, `command/{id}`,
`effect/{id}`, `swatch/{name}`, and report "Backend: headless" in the instructions.

### A3. PhotoCraft's tool set

Instructions string (verbatim, `server/conventions.rs`): "PhotoCraft image editor. Start with
doc_new/doc_open, discover ids and parameter docs with command_list, edit with command_run or
command_batch (one history step per command). Inspect with doc_inspect and render_preview; doc_save
writes native .pcraft or exports by extension. File paths are relative to the read/write roots
granted at launch. ui_inspect/ui_screenshot and other ui_* tools require bridge mode. Read
photocraft://document and photocraft://commands for live JSON state."

| Tool | Arguments (all optional unless marked) | Result | Bridge mapping |
|---|---|---|---|
| `session_list` | — | `{documents:[{index,name,size,dirty,revision,…}], active}` | `ui.inspect` (whole UI state: a quirk) |
| `doc_new` | `width, height, mode (rgb\|gray\|cmyk\|lab), depth (8\|16\|32), background, name` | `file.new` result | `engine.execute file.new` |
| `doc_open` | **`path`** | `{index,name,width,height,layers,warnings}` | `app.open {path}` |
| `doc_place` | **`path`**, `index, scale, fit, center` | `{layer,scale,bounds}` | `engine.execute file.placeEmbedded` |
| `doc_export_layers` | **`dir`**, `index, format, prefix, visible_only, quality` | `{files,count}` | `engine.execute file.export.layersToFiles` |
| `doc_export_data_sets` / `doc_import_data_sets` | **`dir`** / **`path`**, … | `{files,count}` / `{imported}` | `engine.execute file.export.dataSetsAsFiles` / `file.import.variableDataSets` |
| `doc_save` | `path, format, quality, tiffLayers, tgaBits, dpxBits, index` | `{path, warnings}` | `app.save {path?, tgaBits?}`; other options refused in bridge |
| `doc_export` | as `doc_save`, `path` required | same | same |
| `doc_inspect` | `index` | layer tree, history, selection, active layer; `{document:null, session}` when empty | `engine.execute document.inspect` |
| `render_preview`, `doc_render_preview` | `index` (headless only), `max_side` (default 1024, max 2048, 0 = full within ceiling) | PNG image content + text note | `ui.screenshot` (window, not document: a quirk) |
| `doc_select` / `doc_close` | **`index`** / `index` | `session_list` | headless only |
| `command_list` | `filter, enabled_only` | **bare array** of `{id,label,menu,shortcut,params,enabled,disabledReason,needsDocument}` | `engine.commands`, filtered by the MCP server |
| `command_run` | **`id`**, `params` (object), `wait` (default true) | the command's JSON, or `{job, pending:true}` | `engine.execute {command, params, wait}` |
| `command_batch` | **`steps:[{id, params?, wait?}]`** (≤ 256), `stop_on_error` (default true) | `{completed, failed, results:[{ok, result\|error}]}`; not atomic, no rollback | one `engine.execute` per step |
| `jobs_list` / `jobs_cancel` | — / `job` | running jobs with progress, recently ended ones | `jobs.list` / `jobs.cancel` |
| `ui_inspect` | — | full UI state (tool, panels, views, dialogs, windows) | `ui.inspect` |
| `ui_screenshot` | `max_side` | PNG of the window | `ui.screenshot` |
| `ui_pointer` | **`events:[{kind,x,y,pressure,tiltX,tiltY,rotation}]`**, `modifiers`, `button` | — | `ui.pointer` |
| `ui_menu_invoke` | **`id`** | — | `ui.menu.invoke` |
| `ui_set` | **`fields`** (closed list; unknown field = error) | — | `ui.set` |
| `control_call` | **`method`**, `params` | raw control result | any control method |

`ui_*` and `control_call` in headless return a tool error explaining how to start bridge mode;
`doc_select`/`doc_close` in bridge return "only available in headless mode".

**PdfCraft** lists the five shared tools with slightly different shapes (`command_list {doc?,
filter?, enabled_only?}` returns `{commands:[{id,label,menu,shortcut,enabled,tool,params}]}` where
`params` is the mapped tool's JSON Schema; `command_run {id, params?}` maps a registry command to a
task tool and drops unknown params with `warnings`; `command_batch` ≤ 1000 steps with every envelope
checked before any edit; `doc_inspect {doc?}`; `render_preview {doc?, page?, max_side? 1–4096}`)
plus about 140 task tools in its static table (`doc_open`, `page_rotate`, `text_find`, …), which is
why it has `--compact`. Its MCP has no bridge; its GUI has a
separate control channel (A8).

### A4. Tool conventions

- **Names** are `snake_case` (dots are invalid in some clients' tool-name grammar,
  `server.rs` header). Control methods and command ids use dots.
- **Schemas** come from Rust structs: `#[derive(Deserialize, JsonSchema)] #[serde(deny_unknown_fields)]`,
  generated by `schemars` 1, then rewritten by `strict_schema` (`server.rs`): `$ref`s into `$defs`
  are inlined (depth ≤ 32, so recursive defs stop), `$defs` dropped, a `true` schema becomes `{}`,
  and `annotated()` sets `additionalProperties:false` on every input schema. Reason in the code:
  some providers reject the whole request over `$ref` or `true` schemas.
- **Unknown arguments** are refused before execution with JSON-RPC `-32602` naming the key
  (`call_tool` checks keys against the schema's `properties`). `command_batch` envelopes are parsed
  strictly too. Nested command `params` keep their command-specific handling.
- **Annotations:** every tool gets a `title` (name with underscores as spaces, capitalised) and all
  four hints: `readOnlyHint` true for `command_list, doc_inspect, session_list, render_preview,
  doc_render_preview, ui_inspect, ui_screenshot`; `destructiveHint = !read && !additive` (additive:
  `doc_new, doc_open, doc_place`); `idempotentHint = read`; `openWorldHint = false`. `docs/mcp.md`:
  "Hints do not grant permission."
- **Results:** success is one text content holding compact JSON (no `structuredContent` in
  PhotoCraft); images are image content (base64 PNG) plus a text note. Failure is
  `CallToolResult::error` (`isError:true`) with one text message.

### A5. Command discovery: registry → list → schema

- The engine registry (`crates/engine/src/commands.rs`) holds `CommandSpec { id, label, menu:
  &[&str], shortcut: Option<&str>, params: &str /* human-readable doc, "JSON-ish" */, enabled: fn ->
  Result<(), reason>, run, journal: bool }`. Menus, shortcuts, the CLI, the control channel and MCP
  all dispatch by id.
- `list_entry` makes the eight-key row; `needsDocument` is true when the disabled reason is "no
  document…". The reply is a **bare array** in registry order (pinned by test
  `command_list_replies_are_documented_array_of_objects`, issue #3684). `filter` matches id or label,
  case-insensitive; `enabled_only` keeps enabled rows.
- **PhotoCraft's `params` is a doc string, not a JSON Schema.** Strict param checking exists only
  for commands listed in `STRICT` (`crates/engine/src/param_check.rs`); others ignore unknown keys
  (`docs/control-protocol.md`). Tool schemas (A4) are separate and only cover the `doc_*`/`ui_*`
  wrappers.
- **PdfCraft** instead keeps a static tool table (`tools.rs`: `ToolDef {name, title, description,
  input_schema, read_only, destructive, command: Option<id>}`) with hand-written JSON Schema;
  `command_list` reports, per registry command, the tool that runs it and that tool's schema.

### A6. Undo semantics

- `Session::edit` (`crates/engine/src/lib.rs`) clones the `Arc<Document>`, runs the edit on the
  copy, and only on success swaps it in and records one history state, or merges into the previous
  state when the call carries the same `coalesce` key (slider drags become one undo step). A failed
  or panicking command leaves the document unchanged.
- Agents undo with `command_run {"id":"edit.undo","params":{}}`, once per history entry added,
  reading `doc_inspect.history` as they go. Not every command adds exactly one entry; batches are
  not transactions; undo never rolls back files already written (`docs/mcp.md`).
- For experiments `docs/mcp.md` recommends `image.duplicate` or `history.newDocument` first, so the
  edits land on a copy. PdfCraft exposes the same idea as an `edit_undo` tool.
- Background jobs (`wait:false`) apply their result when finished; `sync_jobs()` runs before every
  request, so inspect/save/preview see finished jobs without polling (issue #503). A cancelled or
  failed job leaves the document unchanged.

### A7. Headless mode: sessions and documents

`Headless { session, writers (incremental .pcraft writers per doc), filesystem }` with three
filesystem authorities (`headless.rs`):
- `Denied` (`Headless::new()`, MCP with no roots): every open/save fails "automation filesystem access
  is not granted".
- `Workspace(AuthorizedWorkspace)`: MCP and `serve` with roots. `session.authorize =
  authorize_engine_step`, so nested `actions.play` steps are re-checked.
- `TrustedLocal`: only the CLI's own subcommands (`info`, `run`, `convert`, `batch`), whose paths the
  local user typed. Previews there skip the automation budgets.

Loading: `doc_open` reads the bytes through the read capability, decodes by extension
(`files::open_bytes`; `.pcraft` natively, else `photocraft_io::import`), names template files
"Untitled", records the path unless the source is read-only, and returns import warnings. A
directory-form `.pcraft` loads through the directory capability (`open_pcraft`). Saving without
`path` writes back only a layered native file that holds the whole document; a flat export is a copy
and leaves `dirty` set; a native save marks the revision saved and adopts the path (issues #416,
#1547).

`photocraft-cli serve` exposes the same session over the control envelope on stdio (no auth: the pipe
is the authority) or `127.0.0.1:<port>` (auth required, one shared session for all connections),
methods `engine.execute, jobs.list/cancel, engine.commands, session.list, doc.open/place/new/save/
exportLayers/exportDataSets/importDataSets/inspect/render/select/close, batch` (steps may mix
`{command}` and `{method}`; nested `batch` refused) and `methods`.

### A8. Bridge mode: how a CLI reaches the running GUI

**GUI side (PhotoCraft).**
- Launch: `photocraft --control <port> --control-token-file <path> --automation-read-root <dir>
  --automation-write-root <dir>` (env: `PHOTOCRAFT_CONTROL_PORT`, `PHOTOCRAFT_CONTROL_TOKEN[_FILE]`,
  `PHOTOCRAFT_AUTOMATION_READ_ROOT/WRITE_ROOT`). A bad port is a usage error (exit 2), never a
  silent "no server". **The desktop process owns the roots**; the bridging CLI's root flags are
  irrelevant.
- Transport: TCP bound to `127.0.0.1` only. **No Unix socket, no discovery file**: the client must
  be told the port and token. The listener refuses connection 17+ with "connection limit reached".
  Each connection thread applies 30 s read/write timeouts.
- Auth: the first frame must be `{"id":…, "method":"auth", "params":{"token":"<64 hex>"}}`; anything
  else gets `"authentication required"` and the connection closes. A repeat `auth` gets `"already
  authenticated"` and the connection keeps serving.
- Each request becomes a `ControlRequest {method, params, reply: Sender, deadline: now+60 s,
  dispatch: Dispatch}` sent over an mpsc channel, followed by `ctx.request_repaint()`. The UI thread
  drains the channel **between frames** (`drain_control`). `Dispatch` is an atomic settled once
  between QUEUED → DISPATCHED (UI thread starts it) and QUEUED → ABANDONED (transport gave up), so a
  timeout says either `"timeout: not run …"` (safe to resend) or `"timeout: no reply in 60s — the
  command may still be running and may still apply …"` (issue #3662). Requests need the window to
  draw frames: an asleep display or fully occluded window makes everything time out.
- Handlers return `Outcome::Done`, `AfterInput` (reply once queued synthetic input has been
  processed), `AfterJob` (reply when the background job is applied) or `Screenshot` (after
  animations settle and ≥ 3 frames).
- **Synthetic input:** `ui.click/move/key/type/pointer` push `egui::Event`s into `app.synthetic`;
  `raw_input_hook` appends them to the frame's raw input one press/release step per frame
  (`take_synthetic_step`) and sets `app.automation_input = true` for that frame. Commands triggered
  by shortcuts or dialogs in such a frame pass the same policy as direct control calls (the
  `automation_authorize` hook). This **taint flag** is the seed of OpenLaser's human-only widgets
  (C3.2).
- `handle()` swaps in `services.automation_authorize` as the session's authorize hook for the call;
  `engine.execute`/`ui.menu.invoke` are checked by `services.automation_command`
  (`authorize_desktop_engine_command`, which also refuses `image.mode.*` because the desktop may
  hold ambient profile paths). `engine.execute` never opens a dialog; `ui.menu.invoke` behaves like
  a menu click (may open one) and respects the modal gate.
- Method list (`docs/control-protocol.md`): `engine.execute, engine.commands, jobs.list/cancel,
  ui.inspect, ui.set, ui.menu.invoke/list, ui.context.choose, ui.dialog.open/set/confirm/apply/
  cancel, ui.window.open/close, ui.pointer, ui.click, ui.move, ui.key, ui.type, ui.resize,
  ui.gpu.simulateLoss, ui.screenshot {path?, focus?}, ui.focus, app.open, app.save, app.quit`.
  Every input method validates all params before queuing anything (#2449).

**Client side (`BridgeClient`, `bridge.rs`).**
- Refuses non-loopback addresses; validates the token format; keeps **one** authenticated
  connection behind a tokio mutex; reconnects before a call once idle ≥ 25 s (app drops idle
  connections at 30 s).
- `call` (may mutate) never resends after a transport failure (issue #1007); `call_read` (vouched
  read) retries once on a fresh connection. Read-only MCP tools use `call_read`.
- Call timeout 55 s, strictly below the app's 60 s deadline, so the bridge's error is the one seen.
- Failure taxonomy → wording: `NotSent` ("the request was not sent"), `Transport` ("operation may
  have completed; inspect state before retrying"), `TimedOut` (read: "may be retried safely";
  write: "may have completed"), `Definite` (the app's own `{"ok":false,"error"}`). Stale replies
  with another `id` are skipped. A reply over 8 MiB drops the connection without retry.

**PdfCraft's GUI channel** differs: `pdfcraft --control <file>` listens on a random loopback port and
writes `{port, token}` to `<file>` (a discovery file); an egui plugin keeps the **AccessKit tree**
each frame, so `ui.inspect {query?, role?}` returns widgets with ids, roles, labels, rects and states,
and `ui.click {id}|{label}|{x,y}` clicks a widget **by its AccessKit action** or a point. The client
is `pdfcraft-cli ui --control FILE <method> key=value…`. PdfCraft's MCP server does not bridge.

### A9. Security model

| Concern | Mechanism (PhotoCraft unless noted) |
|---|---|
| Credential | 256-bit token from the OS CSPRNG (`getrandom`), 64 lowercase hex; `validate_token` exact format; constant-time `token_matches` |
| Token storage | `--control-token-file`: created `create_new` with mode `0600`, reused if present, raced safely; token on the command line discouraged; generated token printed to stderr only when neither is given |
| Exposure | Loopback only on both ends; unencrypted; "do not tunnel or proxy" |
| Authorization | Bearer token = whole non-filesystem control surface ("not client identity or per-method authorization") |
| Filesystem | `AuthorizedWorkspace`: separate optional read and write roots opened as `cap_std::fs::Dir` handles; omitted authority fails closed |
| Path rules | Request paths: non-empty, `/`-separated relative; no `\`, no leading `/`, no `:`, no empty / `.` / `..` component, no component ending in `.` or space, no Windows device names. An absolute path is accepted only if its text lies beneath the root (component-wise, drive letter case-insensitive) and is then re-checked as relative. Nothing is resolved on the filesystem; the held `Dir` does all opening, so symlinks cannot escape |
| Reads | Regular files only; bytes read up to the observed length (growth refused), no upfront reservation from metadata |
| Writes | Parent must exist; temp file `create_new` beside the target, `sync`, rename (retry on Windows), directory fsync; failure leaves the old file |
| Multi-file exports | `ExportSequence`: `create_new` per file (refuses collisions, including symlinks), records each file before writing, `cleanup()` removes only files it created on error, cancel or panic (`Drop`) |
| Ambient-path commands | `authorize_engine_command` refuses every `file.*` except a path-free allowlist, smart-object source commands, preset/plugin/actions import-export by path, `prefs.set` on file-backed sections or whole-prefs updates, and named path params (`path`, `file`, `mapPath`, `profile`, …). A registry test fails if a new command documents a path param without a policy. Use `doc_*`/`app.open`/`app.save` instead |
| Nested commands | `Session.authorize` hook re-checks every `actions.play` step |
| Script events | Automation runs commands with script events off |
| PdfCraft | One `--root` confines reads and writes by lexical join + `canonicalize` + `starts_with` (checks then opens: TOCTOU-prone); **without `--root` paths are unconfined**. Root must be given at most once |

### A10. Budgets and limits

| Limit | Value | Where |
|---|---|---|
| Request line | 1 MiB (`MAX_REQUEST_BYTES`) | MCP stdio (ends session), `serve` stdio (error, skip line, keep session), TCP (error, close) |
| Reply | 8 MiB encoded incl. newline (`MAX_RESPONSE_BYTES`); oversize replaced by a small error with the same id, "operation may have completed" | all |
| PNG | 5 MiB before base64 (`MAX_PNG_BYTES`) | previews, screenshots |
| Preview side | default 1024, max 2048; `0` = full size but still ≤ 2048 | headless preview |
| Preview source | ≤ 64 Mi pixels, checked before compositing | headless preview |
| Bridged screenshot decode | ≤ 8192 px edge, ≤ 16 Mi px, ≤ 64 MiB alloc | MCP server |
| Batch | ≤ 256 steps (PdfCraft 1000); aggregate reply budget charged per step **at its JSON-escaped size** for MCP; exhausting it stops later steps even with `stop_on_error:false` | `BatchReplyBudget` |
| Connections | ≤ 16 per listener | TCP |
| Socket idle / write | 30 s | TCP |
| App reply deadline / bridge timeout / idle reconnect | 60 s / 55 s / 25 s | bridge |
| `ui.click` count | ≤ 256 | GUI |
| Not bounded | JSON depth, total memory, command duration | stated in docs |

### A11. Error model

| Layer | Shape | Cases |
|---|---|---|
| JSON-RPC (MCP) | `{"jsonrpc":"2.0","id":…,"error":{"code","message"}}` | `-32700` parse error (`id:null`, next line served); `-32602` unknown tool argument (named) or bad batch envelope; `-32601` unknown method; oversized line ends the session |
| MCP tool | `{"content":[{"type":"text","text":msg}],"isError":true}` | engine errors, refusals, path rejections, budget errors ("…; operation may have completed"), bridge failures (A8 wording), mode errors ("needs bridge mode: start the app with …") |
| Panics | caught: MCP `spawn_blocking` → tool error, poisoned lock reused; `serve` → `catch_unwind` → `"internal error: \`method\` panicked"`; PdfCraft `guarded_tool` | |
| Control protocol | `{"id","ok":false,"error":string}`; `"authentication required"`, `"already authenticated"`, `"connection limit reached"`, `"bad JSON: …"` (`id:null`), `"unknown method \`x\`"`, `"timeout: not run …"`, `"timeout: no reply in 60s …"` | |
| Engine params | invalid filter params report command id + key and leave history unchanged (tests in `server.rs`, `rpc.rs`) | |

### A12. CLI subcommands besides `mcp`

| PhotoCraft (`photocraft-cli`) | PdfCraft (`pdfcraft-cli`) |
|---|---|
| `convert <in> <out> [--format --quality --tiff-layers --tga-bits --dpx-bits]` | `info`, `render`, `text`, `edit`, `combine`, `extract`, `split` |
| `info <file> [--compact]` (document JSON) | `check` (each file in a child process with timeout) |
| `run (<file> \| --new <json>) --cmd <id> [--params <json>] … [--out]` (prints each result as JSON) | `audit` (faux redactions) |
| `batch --actions <json\|.atn> [--action] --in --out [--in-place]` | `batch <files…> --steps convert,ocr,optimize --out-dir` |
| `droplet <file.pcdroplet> <inputs…>` | `tools` (tool table + schemas) |
| `commands [--json] [--filter]` | `run <tool> key=value… \| --script steps.json [--root] [--out]` |
| `serve [--port] [token flags] [root flags]` | `ui --control FILE <method> key=value… [--out]` (GUI client) |
| `--help`, `-h`, `<sub> --help`, `--version` | `--help` |

Both reject unknown flags (PhotoCraft: per-subcommand flag tables, issue #423; non-Unicode args are a
usage error, exit 2). Exit codes 0 ok, 1 failure, 2 usage.

### A13. Quirks OpenLaser should not copy

1. Bridge `session_list` returns the whole `ui.inspect`; bridge `render_preview` returns a window
   screenshot, not the document. OpenLaser renders the document in both modes and keeps
   `ui_screenshot` for the window.
2. `command_list` shapes differ (bare array vs `{commands}`); `params` is a prose string in
   PhotoCraft. OpenLaser keeps the bare array (PhotoCraft parity) and adds a real schema per row.
3. Strict param checking for only a few commands. OpenLaser is strict for all from day one.
4. PdfCraft is unconfined without `--root`. OpenLaser fails closed, like PhotoCraft.
5. Requests time out when the window is not drawing frames. OpenLaser's `stop` must not depend on
   the frame loop (B10.4).
6. Docs that drifted from code (MCP stdio cap). OpenLaser pins the documented shapes with tests.

---

## Part B. OpenLaser design

### B1. Goals and non-goals

Goals:
- An agent configured like the Craft apps can do every design and preparation task Luiz does in
  LightBurn, headless or on the live window, with the same tool vocabulary.
- An agent can watch a machine and stop it, and can hand a job to the human at the machine.
- The emission boundary (Part C) holds by construction (crate graph, types, GUI-side checks), not by
  the MCP server's good behaviour.

Non-goals:
- Any remote, scripted or API path to emission, including "trusted" opt-ins. PLAN §3.6 layer 5:
  "**Never:** an API, script or LAN path". (Research 02 line 694 floated a gated API start for
  production lines; the final plan rejects it, and so does this design.)
- Machine connection, jogging, homing, G-code streaming or calibration by agents.

### B2. Binaries, processes and crates

```
 agent ──stdio MCP──► openlaser-cli mcp            (crate ol-automation: rmcp + tokio)
                         ├─ headless: ol-app Session, FakeLmc (ol-lmc-emu) for job_simulate
                         └─ bridge:   BridgeClient ──TCP 127.0.0.1 + token──┐
                                                                             ▼
 human ──────────────────────────────────────────► openlaser (GUI, ol-ui + ol-human + ol-lmc-usb)
                                                     control server thread (ol-control-proto, std::net)
                                                     ├─ machine.stop fast path → DriverHandle::stop()
                                                     └─ everything else → UI thread, between frames
                                                     driver thread (one per machine) owns the USB device

 human at a TTY ──────────────────────────────────► openlaser-hw  (PLAN's `ol`: detect, status, frame,
                                                     run --job, bench, hw; ol-arm + ol-lmc-usb)
```

**Two CLIs, on purpose.** PLAN §3.1 requires that `ol-arm` (the only code that can mint an
`ArmToken`) has reverse dependencies exactly {`ol-ui`, `ol-cli`} and that neither pulls in a server
crate. An MCP server (rmcp, tokio, a JSON-RPC dispatcher) inside the binary that can TTY-ARM breaks
that rule. So:
- **`openlaser-cli`** is the Craft-style automation CLI: `mcp`, `serve`, `ui`, `commands`, `info`,
  `convert`, `run`, `plan`, `preview`, `simulate`, `estimate`, `gcode`, `job-export`. It cannot
  link `ol-arm` or `ol-lmc-usb`, so it cannot open the laser's USB device or arm anything, whatever
  flags it is given.
- **`openlaser-hw`** is PLAN's `ol` (renamed only for discoverability): `detect`, `status`, `frame`,
  `run --job`, `bench start`, `hw <item>`. Human at a TTY, bench-session gated, blocked for agents by
  the PreToolUse hook. `openlaser-cli frame|run|mark` prints "hardware commands live in
  `openlaser-hw` and are run by a human at a TTY" and exits 2; it never forwards or execs.

New crates (all `publish = false`, `#![forbid(unsafe_code)]`, no panics):

| Crate | Responsibility | May depend on | Must not depend on |
|---|---|---|---|
| `ol-control-proto` | Leaf, std-only: token generate/validate/compare, token files, bounded line reader, `write_reply`, budgets, `AuthorizedWorkspace` (cap-std), control envelope types, method-name constants, discovery-file format | `ol-types`, cap-std, serde_json, getrandom | tokio, egui, any `ol-*` above `ol-types` |
| `ol-automation` | MCP server (rmcp), headless backend over `ol-app`, `BridgeClient`, `serve` | `ol-app` (built without the USB transport, B2.1), `ol-lmc-emu`, `ol-control-proto`, rmcp, tokio | `ol-arm`, `ol-lmc-usb`, `nusb`, egui |
| `ol-human` | Human-only GUI surfaces and the input-provenance gate (Part C): ARM dialog, frame approval, connect, machine-profile save, automation settings; the `HumanGesture` witness | egui, `ol-types`, `ol-arm` | `ol-control-proto`, `ol-automation`, any server crate |
| `ol-ui` (existing) | Panels, canvas, control dispatcher (handlers), STOP | `ol-app`, `ol-control-proto`, egui | `ol-arm` (it receives `ArmToken`s from `ol-human` through a trait object injected by the app binary) |
| `apps/openlaser` | Composes `ol-ui` + `ol-human` + `ol-lmc-usb` | | |
| `apps/openlaser-cli` | Arg parsing over `ol-automation` | | `ol-arm`, `ol-lmc-usb` |
| `apps/openlaser-hw` | PLAN's `ol-cli` | `ol-app`, `ol-arm`, `ol-lmc-usb` | `ol-automation`, `ol-control-proto` server parts, rmcp, tokio |

**B2.1 Dependency rules added to `xtask deps` / cargo-deny** (extending PLAN §3.1):
1. Reverse dependencies of `ol-arm` are exactly {`ol-human`, `apps/openlaser-hw`}.
   (This moves ARM out of `ol-ui` into a smaller crate; `ol-ui` hosts the control dispatcher and
   must not be able to mint.)
2. The dependency closure of `ol-automation` and of `apps/openlaser-cli` contains neither `ol-arm`
   nor `ol-lmc-usb` nor `nusb`. This needs `ol-app` to take the device transport by injection
   (`Box<dyn TransportFactory>`) instead of depending on `ol-lmc-usb`; PLAN §3.1 currently says
   `ol-app` depends on "everything above except `ol-arm`", which would put `nusb` into the
   automation CLI. The GUI and `openlaser-hw` inject `ol-lmc-usb`; `openlaser-cli` injects nothing
   (or FakeLmc for `simulate`).
3. `ol-human` and `apps/openlaser-hw` closures contain no rmcp, tokio, hyper, axum, tungstenite,
   rhai, extism, pyo3 (PLAN's list plus rmcp/tokio).
4. `ArmToken` and `HumanGesture` are `!Clone`, `!Serialize`, `!Deserialize`, with private fields;
   their constructors are clippy `disallowed-methods` everywhere except `ol-arm/src/mint.rs` and
   `ol-human/src/gesture.rs`; xtask greps for any other `#[allow]`.
5. No crate in the automation closure uses `std::process::Command` (no exec of `openlaser-hw`).

### B3. `openlaser-cli`

```
openlaser-cli mcp   [--bridge (auto | <127.0.0.1:port>) | --bridge-file <control.json>]
                    [--control-token <64-hex> | --control-token-file <path>]
                    [--automation-read-root <dir>] [--automation-write-root <dir>] [--root <dir>]
                    [--compact]
openlaser-cli serve [--port <port>] [token flags] [root flags]      JSON lines on stdio or loopback TCP
openlaser-cli ui    (--bridge-file <f> | --bridge <addr> --control-token-file <f>) <method> [key=value…] [--out shot.png]
openlaser-cli commands [--json] [--filter <text>]
openlaser-cli info  <project.olp|file.svg|file.dxf> [--compact]
openlaser-cli convert <in> <out>                                     .olp ⇄ .svg/.dxf (and .lbrn2 in from M5)
openlaser-cli run   (<project> | --new <json>) --cmd <id> [--params <json>] … [--out <file>]
openlaser-cli plan  <project> [--machine <id>] [--json]              refusals + byte-decoded summary; exit 1 on refusal
openlaser-cli preview <project> --out <png> [--show burn,frame,travel] [--max-side N]
openlaser-cli simulate <project> --out <png> [--listing <txt>]       FakeLmc run of the exact bytes
openlaser-cli estimate <project>
openlaser-cli gcode <project> --out <file.gcode>                     gantry profiles only (M2)
openlaser-cli job-export <project> --out <job.oljob>                 for `openlaser-hw run --job` at a TTY
openlaser-cli --help | -h | <sub> --help | --version
```

- `mcp` and `serve` follow PhotoCraft's flags exactly. `--root <dir>` is PdfCraft's spelling and sets
  both roots. **Without roots, file access fails closed** (PhotoCraft semantics, not PdfCraft's).
  `serve` stdio needs no auth; `serve --port` needs the auth frame.
- `info`, `convert`, `run`, `plan`, `preview`, `simulate`, `estimate`, `gcode`, `job-export` are
  `trusted_local` (the caller typed the paths) and can never touch hardware (B2.1).
- Unknown flags are usage errors (exit 2); arguments are taken with `args_os` so non-Unicode is a
  usage error, never a panic.
- Env vars: `OPENLASER_CONTROL_TOKEN`, `OPENLASER_CONTROL_TOKEN_FILE` (client side). Root env vars
  are honoured by the GUI only, as in PhotoCraft.

Recommended `~/.claude.json` entries (same shape as photocraft / photocraft-live):

```json
"openlaser":      {"type":"stdio","command":"/Users/luizbueno/.local/bin/openlaser-cli",
                   "args":["mcp","--automation-read-root","/Users/luizbueno/Craft","--automation-write-root","/Users/luizbueno/Craft"]},
"openlaser-live": {"type":"stdio","command":"/Users/luizbueno/.local/bin/openlaser-cli",
                   "args":["mcp","--bridge","auto"]}
```

### B4. The MCP server

- **Transport:** rmcp (same major as PhotoCraft) behind PhotoCraft's `serve_io` pre-reader:
  1 MiB line cap (ends the session), `-32700` recovery, single writer. Protocol versions as rmcp
  negotiates; the `2026-07-28` `_meta` request style and `resultType/ttlMs/cacheScope` list fields
  as PhotoCraft does. Server info `{name:"openlaser", version}`.
- **Capabilities:** tools, resources (with templates), and from A3 below, prompts.
- **Instructions** (the agent's first orientation; draft):
  > OpenLaser designs and runs laser jobs, LightBurn-style. Units are millimetres, Y up; on a galvo
  > the origin is the field centre. Start with doc_new, doc_open or doc_import; discover command ids
  > and their parameter schemas with command_list; edit with command_run or command_batch (each step
  > undoable with edit.undo). Check a job with job_plan (refusals plus the summary decoded from the
  > bytes that would be sent), job_preview and job_simulate. You cannot fire the laser or move the
  > machine: job_submit hands the job to the person at the machine, who frames and arms it in the
  > OpenLaser window; poll job_status. stop always works in bridge mode. File paths are relative to
  > the read/write roots granted at launch. machine_status, frame_request, job_submit, stop and the
  > ui_* tools need bridge mode (openlaser-cli mcp --bridge auto). Backend: headless|bridge.
- **Resources:** `openlaser://document` (live `doc_inspect`), `openlaser://commands` (live
  `command_list`), `openlaser://machines` (live `machine_list`), `openlaser://job` (the last
  `job_plan` of the active document, recomputed when stale). Templates: `openlaser://layer/{id}`,
  `openlaser://object/{id}`, `openlaser://command/{id}` (one row with its schema),
  `openlaser://submission/{id}`. Resource reads are uncached (`ttlMs:0`); catalogs private, 10 min.
- **Prompts** (A3, optional, VectorCraft-style): `prepare-job` (import → assign layers → job_plan →
  fix refusals → job_simulate → job_submit), `mopa-color-test` (build a frequency × pulse grid as
  M2's test-grid command, then plan), `fix-refusals`.
- **Progress:** `job_plan`, `job_simulate` and long `command_run`s (hatching a big fill) send
  `notifications/progress` when `_meta.progressToken` is given (PhotoCraft rules: increasing,
  ≤ 10/s, with total) and honour `notifications/cancelled` (planning is pure; cancel drops the
  result).
- **`--compact`:** lists `command_list, command_run, command_batch, doc_inspect, render_preview,
  doc_open, doc_import, doc_save, job_plan, job_preview, job_submit, job_status, machine_status,
  stop` plus `tool_search`/`tool_call`. **`stop` is always listed** so no agent has to search for it.

### B5. Tool catalogue

Modes: **H** headless, **B** bridge. Access: **agent** (runs), **approval** (creates a request a
human must act on in the GUI), **read**. Every input schema is strict (`additionalProperties:false`,
unknown key → `-32602` naming it). No schema anywhere has a `force`, `proceed`, `arm`, `confirm`,
`skip_frame` or `override` property; a CI test greps the generated schemas for those names.

| Tool | Args | Result | Modes | Access | Hints (ro/destr/idem) |
|---|---|---|---|---|---|
| `session_list` | — | `{documents:[{index,name,path,dirty,revision,machine}], active}` | H B | read | T/F/T |
| `doc_new` | `name?, machine?` | `{index,name,machine}` | H B | agent | F/F/F |
| `doc_open` | **`path`** (`.olp`; `.svg/.dxf` open as a new project; `.lbrn2` from M5) | `{index,name,layers,objects,warnings}` | H B | agent | F/F/F |
| `doc_import` | **`path`**, `layer?, center?:[x,y] mm, scale?, fit?, svg_units?:"mm"\|"px96"\|"px72"` | `{objects:[ids], bounds, warnings}` | H B | agent | F/F/F |
| `doc_save` | `path?` (`.olp`) | `{path, warnings}` | H B | agent | F/T/T |
| `doc_export` | **`path`**, `format?` (svg, dxf, png) | `{path, warnings}` | H B | agent | F/T/T |
| `doc_inspect` | `index?, objects?:"summary"\|"all"` | B7 | H B | read | T/F/T |
| `render_preview` | `index?, max_side?, view?:"design"\|"toolpath"` | PNG + note | H B | read | T/F/T |
| `doc_select` / `doc_close` | **`index`** / `index?` | `session_list` | H B | agent | F/F/F, F/T/F |
| `command_list` | `filter?, enabled_only?, access?` | bare array, B6 | H B | read | T/F/T |
| `command_run` | **`id`**, `params?`, `wait?` | command JSON or `{job,pending}` | H B | agent | F/T/F |
| `command_batch` | **`steps:[{id,params?,wait?}]`** ≤ 256, `stop_on_error?` | `{completed,failed,results}` | H B | agent | F/T/F |
| `jobs_list` / `jobs_cancel` | — / `job?` | background tasks (planning, hatching, trace) | H B | read / agent | T/F/T, F/F/F |
| `machine_list` | — | `[{id,name,kind,work_area,source:{max_power_pct,freq_khz:[lo,hi],pulses_ns},connected?}]` | H B | read | T/F/T |
| `machine_status` | `machine?` | B8.1 | H (stub) B | read | T/F/T |
| `job_plan` | `index?, machine?, selected_only?` | B8.2 | H B | read | T/F/T |
| `job_preview` | `index?, max_side?, show?:["burn","frame","travel","order"]` | PNG + `{fingerprint, bounds}` | H B | read | T/F/T |
| `job_estimate` | `index?` | `{fingerprint, estimate_s, per_section:[…], crash_bound_s}` | H B | read | T/F/T |
| `job_simulate` | `index?, max_side?, listing?:bool` | PNG of the FakeLmc emission image + `EmissionSummary` + `listing?` (bounded, `truncated` flagged) | H B | read | T/F/T |
| `job_export` | **`path`**, `index?` | `{path, fingerprint}` (`.oljob` for `openlaser-hw run --job`) | H B | agent | F/T/T |
| `gcode_export` | **`path`**, `index?` | `{path, lines, fingerprint, warnings}`; gantry profiles only (M2) | H B | agent | F/T/T |
| `frame_request` | `mode?:"once"\|"loop", machine?, index?` | `{request, state, fingerprint}` (C3.4) | B | approval | F/F/F |
| `job_submit` | `machine?, index?, note?` (≤ 280 chars, plain text) | `{submission, state:"awaiting_human", fingerprint, summary, expires_at}` (C3.5) | B | approval | F/F/F |
| `job_status` | `submission?` (default: latest of this connection) | `{submission, state, fingerprint, outcome?, stop_report?, fault?}` | B | read | T/F/T |
| `job_withdraw` | **`submission`** | `{submission, state:"withdrawn"}` | B | agent | F/F/T |
| `stop` | `machine?` | `{latched:true, confirmed, report?, elapsed_ms}` (waits ≤ 500 ms for the `StopReport`) | B (H: error) | **always** | F/F/T |
| `ui_inspect` | — | UI state, incl. `humanSurfaces` (read-only, C3.2) | B | read | T/F/T |
| `ui_elements` | `query?, role?, limit?` | AccessKit widgets `{id,role,label,rect,enabled,humanOnly}` (PdfCraft/FilmCraft style) | B | read | T/F/T |
| `ui_screenshot` | `max_side?` | PNG of the window | B | read | T/F/T |
| `ui_click` | `id` \| `x,y` + `button?, count?` | — | B | agent | F/T/F |
| `ui_drag` | **`from`,`to`**, `steps?, modifiers?, button?` | — | B | agent | F/T/F |
| `ui_key` / `ui_type` | **`key`**, modifiers / **`text`** | — | B | agent | F/T/F |
| `ui_pointer` | **`events`** (canvas tool events, document mm), `modifiers?, button?` | — | B | agent | F/T/F |
| `ui_menu_invoke` | **`id`** | — | B | agent | F/T/F |
| `ui_set` | **`fields`** (closed list) | — | B | agent | F/F/F |
| `control_call` | **`method`**, `params?` | raw result | B | agent | F/T/F |
| `tool_search` / `tool_call` | as PdfCraft | | H B (`--compact`) | | T/F/T, F/T/F |

Notes:
- `stop` is not `destructiveHint` on purpose: some clients prompt before destructive tools, and a
  stop must never wait for a prompt. Luiz should also allowlist `mcp__openlaser-live__stop` (and
  the read tools) in Claude Code permissions.
- `render_preview` renders the **document** in both modes (A13.1). `view:"toolpath"` is
  `job_preview` without the frame overlay.
- `machine_status` in headless returns `{connected:false, backend:"headless", note}` rather than an
  error, so agents can branch on it. `frame_request`, `job_submit`, `job_status`, `job_withdraw`,
  `stop`, `ui_*` and `control_call` in headless return a tool error naming the bridge command line.

### B6. Commands: registry → list → schema

Every user-visible action is an engine command (AGENTS.md "Everything is a command"):

```rust
pub struct CommandSpec {
    pub id: &'static str,                 // "layer.setCut", "object.align.left", "laser.frameOnce"
    pub label: &'static str,
    pub menu: &'static [&'static str],
    pub shortcut: Option<&'static str>,
    pub params_doc: &'static str,         // prose, for command_list "params" (PhotoCraft parity)
    pub schema: fn() -> schemars::Schema, // from a #[derive(Deserialize, JsonSchema)] #[serde(deny_unknown_fields)] struct
    pub access: Access,                   // Agent | Approval | HumanOnly
    pub enabled: fn(&Session) -> Result<(), String>,
    pub run: Run,
    pub journal: bool,
}
```

- `command_list` rows keep PhotoCraft's eight keys and add two: `schema` (strict JSON Schema,
  `$ref`s inlined as in `strict_schema`) and `access` (`"agent"`, `"approval"`, `"human"`). The
  reply stays a bare array. `access` filters (`access:"agent"`).
- Params are parsed into the typed struct for **every** command (unknown key → error naming it and
  the accepted keys; nothing runs), closing PhotoCraft's `STRICT`-list gap.
- **Human-only commands** exist in the registry so menus and shortcuts work (`laser.connect`,
  `laser.frameOnce`, `laser.frameLoop`, `laser.start` (opens the ARM dialog), `laser.testDot`,
  `machine.profile.save`, `machine.calibration.import`, `automation.*`). Through `engine.execute`,
  `ui.menu.invoke`, `command_run` or `command_batch` they return
  `human-only: \`laser.start\` is started by a person in the OpenLaser window` and nothing runs.
  The check sits in the GUI's command authorizer (as PhotoCraft's `automation_command`) **and** in
  `Session::dispatch` when the session is marked untrusted, so nested calls (a future
  `actions.play`) are covered.
- `laser.stop` is `access: agent` everywhere and is never disabled.

Illustrative command ids (LightBurn-shaped, final list belongs to SPEC.md): `file.new`,
`edit.undo`, `edit.redo`, `edit.selectAll`, `object.move {ids?, dx, dy}`, `object.scale`,
`object.rotate`, `object.mirror`, `object.delete`, `object.align.*`, `arrange.group`,
`layer.assign {ids, layer}`, `layer.setCut {layer, galvo:{speed_mm_s, power_pct, frequency_khz,
q_pulse_ns}, passes, mode, fill:{interval_mm, angle_deg, …}}`, `layer.setOutput`, `layer.setShow`,
`machine.select {id}` (refused while that GUI machine is connected or busy), `job.repeat {count}`.

### B7. `doc_inspect` result

```json
{
  "index": 0, "name": "card.olp", "path": "jobs/card.olp", "dirty": true, "revision": 14,
  "units": "mm", "axes": "y-up; galvo origin at field centre",
  "machine": {"id": "jpt-m7-200", "kind": "galvo", "workArea": [-100, -100, 100, 100]},
  "layers": [{"id": 1, "name": "C00", "color": "#000000", "mode": "line", "output": true, "show": true,
              "passes": 1, "galvo": {"speedMmS": 1000, "powerPct": 30, "frequencyKhz": 30, "qPulseNs": 200}}],
  "objects": {"count": 214, "shown": 200, "truncated": 14,
              "items": [{"id": 12, "name": "rect", "layer": 1, "bounds": [-20, -10, 20, 10], "paths": 1, "closed": true}]},
  "selection": [12], "history": ["Import card.svg", "Move", "Set cut C00"], "canUndo": true, "canRedo": false,
  "live": {"fingerprint": "7f2a91…", "refusals": 0, "notices": 1}
}
```

`objects:"all"` lifts the 200-item cap (the 8 MiB reply budget still applies; overflow is an error,
not a silent cut). `live` comes from the same `ol_safety::validate` the GUI runs continuously.

### B8. Laser tool results

**B8.1 `machine_status`** (bridge; read from the driver thread's last events, **never triggers USB
I/O**):

```json
{"machine": "jpt-m7-200", "connected": true, "board": {"vid": "0x9588", "pid": "0x9899"},
 "state": "idle|framing|running|stopping|stopped|fault|disconnected",
 "statusBits": "0x0220", "ready": true, "busy": false,
 "fault": null, "progress": null, "lastStop": {"elapsedMs": 141, "confirmed": true},
 "frameReceipt": {"fingerprint": "7f2a91…", "fullPasses": 3, "ageS": 42, "origin": "human"},
 "pending": {"submission": "s-3", "frameRequest": null},
 "automation": {"connectedAgents": 1, "agentFraming": "ask"}}
```

**B8.2 `job_plan`** runs `plan → ol_safety::validate → encode → disassemble → emission_summary`, the
same pipeline the ARM dialog runs, and returns:

```json
{"fingerprint": "7f2a91…(64 hex)", "short": "7f2a91", "machine": "jpt-m7-200", "ok": true,
 "refusals": [],
 "notices": [{"n": 1, "code": "open_shape_unhatched", "object": 7, "message": "…"}],
 "summary": {"rows": [{"layer": 1, "name": "C00", "pass": 1, "of": 1, "powerPct": 30, "frequencyKhz": 30,
             "pulseNs": 200, "speedEncodedMmS": 999.6, "speedUnits": 4224, "bounds": [-20, -10, 20, 10],
             "estimateS": 3.1}],
             "repeats": 1, "estimateS": 41.2, "crashBoundS": 2.0},
 "frame": {"bounds": [-20, -10, 20, 10], "paths": 3}}
```

With refusals, `ok:false`, `summary:null`, and each refusal is `{code, object?, layer?, at?,
message}` using `ol-types` refusal codes (`outside_work_area`, `speed_zero_units`,
`power_above_max`, `hidden_layer_with_output`, `orphan_object`, `fill_without_hatch`,
`nothing_to_mark`, …). `job_plan` itself succeeds (`isError:false`) with a refusing plan, so agents
read structured refusals; `job_submit` with refusals is `isError:true` and creates nothing.

### B9. Undo

- `ol-app`'s `Session::edit` follows PhotoCraft: copy-on-write `Arc<Document>`, the edit runs on the
  copy, success swaps and records one history state (or coalesces by key, for slider drags),
  failure changes nothing. PLAN puts snapshot undo
  in M2; the bridge (which needs undo for agents to recover) ships no earlier (D below).
- `edit.undo` / `edit.redo` via `command_run`; `doc_inspect.history` and `canUndo/canRedo`.
- **Every edit, undo and redo changes the fingerprint.** A pending submission or frame request for
  the old fingerprint becomes `stale`; an ARM in progress is voided (PLAN §3.6 layer 5).
- Undo never touches the machine: it cannot un-burn, and it cannot reach a `FrameReceipt` or token.
- Experiments: `command_run {"id":"doc.duplicate"}` (PhotoCraft's `image.duplicate` advice).

### B10. Bridge mode

**B10.1 Starting it (human).**
- Launch flags with PhotoCraft spelling: `openlaser --control <port> --control-token-file <path>
  --automation-read-root <dir> --automation-write-root <dir>` (env `OPENLASER_CONTROL_PORT`, …).
- Or, without flags, **Automation › Allow Agent Connections** (a human-only toggle, C3.2), like the
  PhotoCraft Assistant's on-demand start: free loopback port, fresh token, roots from Preferences ›
  Automation (default: none, so file access fails closed until Luiz picks a folder, suggested
  `~/Craft/OpenLaser`). Turning it off stops the listener, drops connections and deletes the
  discovery file.

**B10.2 Discovery.** When the listener starts, the GUI writes
`~/Library/Application Support/OpenLaser/automation/control.json` (directory `0700`, file
`0600`, `create_new` then rename): `{"version":1, "pid":…, "port":…, "token":"<64 hex>",
"started":"…", "roots":{"read":"…","write":"…"}}` (PdfCraft's discovery file, with PhotoCraft's
token rules). `--bridge auto` reads it at the first tool call and stays bound to that GUI for the
session; if the file is absent or stale (pid gone), each call fails with "OpenLaser is not running
with Automation › Allow Agent Connections on". **It never falls back to headless**, so edits never
land in an invisible session. `--bridge <addr> --control-token-file <f>` and `--bridge-file <f>` work
as explicit alternatives.

**B10.3 Auth and identity.** First frame `{"method":"auth","params":{"token","client":{"name",
"version"}}}`; the bridge forwards the MCP `clientInfo` (for example "claude-code"). The GUI shows a
persistent **"Agent connected: claude-code"** indicator in the status bar with a Disconnect button
(human-only), logs every mutating request with its connection id, and records the client on every
submission and frame request. Loopback only, 16 connections, 30 s idle, constant-time compare,
repeat-auth reply: all as PhotoCraft.

**B10.4 Stop fast path.** The control server thread holds a clone of each machine's `Arc<StopLatch>`
and answers `machine.stop` on the transport thread: it calls `DriverHandle::stop()` (an atomic store
plus wake, never blocking, PLAN §3.4) before anything touches the UI queue, then waits ≤ 500 ms for
the driver's `StopReport` on its own channel. So `stop` works while the window is occluded, the
display is asleep or the UI thread is stalled (A13.5). `machine.stop` needs auth like every method
(an unauthenticated local process could otherwise only stop jobs, but uniform auth keeps the
protocol simple); it is never refused for being "busy", "human-only" or "paused".

**B10.5 Busy and read-only states.** While a machine is framing or running, or any human-only
surface is open (C3.2), the GUI refuses every mutating control method (document edits, `ui.*`
input, `app.open/save`, `machine.select`, `app.quit`) with `machine_busy` or
`human_confirmation_open`, and keeps serving reads, `ui.screenshot` and `machine.stop`.
`app.quit` while busy is refused ("stop first"); stop is the agent's lever.

**B10.6 Control methods.** Envelope, auth, deadlines, `Dispatch` and `Outcome` exactly as A8.

| Group | Methods |
|---|---|
| Engine | `engine.execute {command, params?, wait?}`, `engine.commands`, `jobs.list`, `jobs.cancel` |
| Document I/O | `app.open {path}`, `app.import {path, …}`, `app.save {path?}`, `app.export {path, format?}`, `doc.inspect {index?}`, `doc.render {maxSide?, view?}` |
| Laser (read/plan) | `machine.list`, `machine.status`, `job.plan`, `job.preview`, `job.estimate`, `job.simulate`, `job.export {path}`, `gcode.export {path}` |
| Laser (requests) | `frame.request {mode, machine?}`, `frame.requests`, `job.submit {note?}`, `job.status {submission?}`, `job.withdraw {submission}` |
| Stop | `machine.stop {machine?}` (transport-thread fast path) |
| UI | `ui.inspect`, `ui.elements`, `ui.screenshot {path?, focus?}`, `ui.focus`, `ui.resize`, `ui.set`, `ui.menu.list`, `ui.menu.invoke`, `ui.dialog.open/set/confirm/cancel` (ordinary dialogs only), `ui.pointer`, `ui.click`, `ui.move`, `ui.drag`, `ui.key`, `ui.type` |
| App | `app.quit` (refused while busy), `methods` |

**Deliberately absent** (no handler, so `"unknown method"`): connect/disconnect, frame start
without approval, arm, run, mark, test dot, jog, home, G-code send, machine profile or calibration
writes, automation access changes, and `prefs.set` on `automation.*`, `machine.*`, `safety.*` or
`humanOnly.*` keys (refused even through `engine.execute`). A test asserts the `methods` list
equals this table.

**MCP → control mapping.** One-to-one by name (`job_plan` → `job.plan`, `stop` → `machine.stop`,
`doc_open` → `app.open`, `render_preview` → `doc.render`, `doc_inspect` → `doc.inspect`,
`ui_click` → `ui.click`, …); `command_list` filters `engine.commands` client-side as PhotoCraft does.
`doc_select`/`doc_close` map to `ui.menu.invoke window.document.<i>` / `file.close` (PhotoCraft
refuses them in bridge; OpenLaser supports them).

**Enforcement lives in the GUI.** The MCP server is a convenience client: anyone holding the token
can speak the control protocol with `nc`. So `control_call` passes any method through unchanged, and
every rule in Part C is checked in the GUI process (the device owner), never only in `ol-automation`.

### B11. Security model

Identical to A9 for tokens, loopback, connection limits, timeouts, `AuthorizedWorkspace` (cap-std,
separate read/write roots, fail closed, the same path grammar, atomic writes, `ExportSequence`), and
the ambient-path command denylist with a registry test (any command whose schema has a path-like
property must have a policy). OpenLaser additions:
- **Machine configuration is outside every root and never writable by automation.** Profiles,
  calibration (`scale_correction`, `.cor`), work area, red-dot calibration and source limits live in
  the app config directory, are read by `machine_list`, and change only through human-only surfaces.
  Opening a project never changes a machine profile; a project naming an unknown machine is refused.
- **Imported files are data.** SVG/DXF/`.lbrn2` text (layer names, descriptions) is shown to the
  agent as data and never interpreted as commands; an SVG `<script>` is ignored by the importer.
- **Submission notes are untrusted text.** Shown in the GUI as plain text, labelled "Note from
  agent", truncated to 280 chars, outside the confirmation block (C3.5).

### B12. Budgets

PhotoCraft's constants unchanged (1 MiB request, 8 MiB reply, 5 MiB PNG, preview side 1024 / 2048,
batch 256 with escaped reply budget, 16 connections, 30 s idle, 60 s app deadline, 55 s bridge
timeout, 25 s idle reconnect, screenshot decode limits). Additions:
- `job_simulate` listing ≤ 2,000 rows or 256 KiB, flagged `truncated:true` with `shown/total`.
- `doc_inspect` objects ≤ 200 by default (flagged), `objects:"all"` bounded by the reply budget.
- One pending submission and one pending frame request per machine; a new one supersedes the old.
- Submissions expire after 15 min, frame requests after 2 min (both refreshed by nothing an agent can do).
- PLAN's job limits (paths/points per job) are **safety refusals** in `ol-safety`, not transport budgets.

### B13. Error model

Exactly A11 (JSON-RPC codes, `isError` tool errors, panic containment, control `{ok:false,error}`,
the bridge failure wording and the two timeout wordings). Laser-specific error strings start with a
stable machine-readable prefix:

| Prefix | Meaning |
|---|---|
| `refused:` | `ol_safety::validate` refused; the tool result also carries `{refusals:[…]}` |
| `human-only:` | a human-only command or surface was addressed through automation |
| `human_confirmation_open:` | a human-only surface is open; mutating automation paused |
| `machine_busy:` | framing or running; only reads and stop are served |
| `needs_bridge:` | tool needs the live app; message gives the command line |
| `stale:` | the document changed since the fingerprint the request names |
| `not_connected:` | the GUI has no connected machine for this request |

---

## Part C. The emission safety boundary

### C1. The rule

| Capability | Agent (MCP/control) | Human in the GUI | Human at a TTY (`openlaser-hw`) |
|---|---|---|---|
| Design, import, edit, layers, cut settings, undo | yes | yes | — |
| Plan, refusals, preview, estimate, simulate on FakeLmc, export SVG/DXF/G-code/`.oljob` | yes | yes | — |
| Read machine list and cached status | yes | yes | yes |
| **Stop** | **yes, always** | yes (Esc, STOP, E-stop) | yes (Esc, Space, Ctrl-C, SIGHUP) |
| Submit a job for approval; withdraw it | yes | — | — |
| Request red-light framing | yes (creates a request) | approves it with a click | — |
| Connect / disconnect the board | no | yes | yes |
| Start framing (red light) | no | yes | yes |
| ARM / start emission / test dot | **no, never** | yes (typed `ARM`; foot switch from M3) | yes (`ARM <nonce>` on a TTY) |
| Jog, home, stream G-code (M4) | no | yes | yes |
| Edit machine profiles, calibration, work area, source limits | no | yes | — |
| Change automation access, roots, agent framing | no | yes | — |

### C2. Threat model: every way input reaches OpenLaser

| # | Channel | Can it forge a human gesture? | Answer in this design |
|---|---|---|---|
| 1 | MCP tools / control protocol methods | No method exists for it | B10.6 "deliberately absent"; crate graph (B2.1) |
| 2 | Control-protocol synthetic input (`ui.click/key/type/drag/pointer`) | Yes in PhotoCraft (it is egui input) | `ol-human` taint gate (C3.2) |
| 3 | Control-protocol direct actions (`ui.menu.invoke`, `ui.dialog.*`, `ui.set`, `engine.execute`) | Could open or confirm dialogs | Human-only commands refused; human-only dialogs are not addressable (C3.2, B6) |
| 4 | OS accessibility actions (AXPress via VoiceOver or any AX client, arriving as AccessKit action requests) | `Response::clicked()` counts them | Human-only widgets use `clicked_by(PointerButton::Primary)`, which in egui 0.36.2 excludes AccessKit and keyboard activations (verified in `response.rs:184-199`); AccessKit cannot set TextEdit text in 0.36.2 (only `SetTextSelection`) |
| 5 | OS-level synthetic events (CGEventPost: computer-use agents, AppleScript, Keyboard Maestro, cliclick) | **Yes: indistinguishable from a real mouse/keyboard to winit/egui** | Not solvable in software alone. M1–M2: procedural layers (C3.9). M3: the foot switch (C3.8) is the boundary that holds |
| 6 | A terminal an agent can type into (`openlaser-hw` TTY ARM) | Yes, if the agent has a terminal-typing tool | PLAN §3.6 bench layers; hook also covers `mcp__terminal__run_in_terminal` (C3.9) |
| 7 | Hostile code inside the GUI process | Yes | Out of scope (PLAN §3.1 says the same) |

### C3. Mechanisms

**C3.1 No path exists (structural).** There is no MCP tool, control method, command id, preference
key or file format field that arms or starts emission. `ol-automation` cannot link `ol-arm` or
`ol-lmc-usb`; `ol-ui` (which hosts the control dispatcher) cannot link `ol-arm`;
`DriverHandle::run(job, ArmToken)` needs a token only `ol-human` (GUI) or `openlaser-hw` (TTY) can
mint (B2.1). Any `arm`/`confirm` field in an imported `.oljob` or OM payload is ignored, as PLAN
M1.2 already requires.

**C3.2 Human-only surfaces and the provenance gate (`ol-human`).**
- **Which surfaces:** Connect/Disconnect; Frame Once/Loop; frame-request Approve/Decline; Review &
  ARM; the ARM dialog (checklist ticks, the `ARM` text field, Confirm); Test Dot; submission
  Frame/Reject; machine profile, calibration and work-area Save; Automation menu (Allow Agent
  Connections, Agent Framing, roots, Disconnect agents); later Jog/Home/Console (M4) and any
  Assistant permission buttons. **STOP is not human-only**: it accepts every input source.
- **Injection goes through `ol-human`.** The control dispatcher cannot push events into egui
  itself; it calls `ol_human::Injector::inject(&mut raw_input, events)`, which marks the frame (and
  the next two, because a click's press and release land in separate frames, as in PhotoCraft's
  `take_synthetic_step`) as tainted in a record keyed by a **private type** in egui's context data,
  so no other crate can read or clear it.
- **`ol_human::gesture(ctx, &response) -> Option<HumanGesture>`** returns `Some` only if
  `response.clicked_by(PointerButton::Primary)`, the frame is untainted, no AccessKit action request
  is pending this frame, the window is focused, and the surface has been visible for ≥ 300 ms (no
  pre-positioned instant click). `HumanGesture` is the witness `ol_arm::mint` requires (PLAN's
  `HumanConfirm`).
- **Typed `ARM`:** the field accepts `Event::Text`/`Event::Key` only in untainted frames. Any tainted
  frame while the dialog is open **clears the field and the checklist** and shows "Automation input
  reached this dialog and was ignored."
- **Freeze:** while any human-only surface is open, the injector drops queued synthetic events and
  the dispatcher refuses input methods with `human_confirmation_open` (defence in depth: the gate
  above would already ignore them).
- **Not addressable:** human-only dialogs have no `ui.dialog.*` id; `ui.inspect.humanSurfaces`
  reports them read-only (`{armDialog:{open, fingerprint}, frameApprovals:[…], submissions:[…]}`),
  and `ui.elements` flags their widgets `humanOnly:true`. `ui.click {id}` on one returns
  `human-only:` up front; `ui.click {x,y}` over one is refused after an AccessKit hit-test, and would
  be ignored by the gate anyway.
- **Accessibility trade-off:** VoiceOver cannot ARM (it acts through AccessKit). STOP and everything
  else stay accessible (PLAN S6). The M3 foot switch gives an ARM path that works for everyone.

**C3.3 Confirmation stays from bytes, and from the human's own pipeline.** The ARM dialog
recomputes `plan → validate → encode → emission_summary` itself (PLAN §3.3); it never renders the
agent's `job_submit.summary`. The agent's note is shown outside the confirmation block, plain text,
labelled. So an agent can neither alter nor narrate the numbers the human confirms.

**C3.4 Frame requests (red light).**
```
frame_request ─► awaiting_human ─(human Approve, gesture)─► framing ─► done (once) │ loop until stop
                   ├─(human Decline)─► declined     ├─(document edit)─► stale
                   └─(2 min)─► expired              └─(stop from anyone)─► stopped
```
- The GUI shows a card: "claude-code asks to frame `card.olp` (fingerprint 7f2a91) with the red
  light on JPT M7 · Approve · Decline". Approve is a human-only button; the frame then runs exactly
  as the human's own Frame button (`FramePlan`, `LightingList`, which cannot express `MarkTo`,
  `Fiber_SetMo(1)` or `LaserOnPoint`).
- The `FrameReceipt` records `origin: Human` (approval gesture) or `origin: Agent`. `ol_arm::mint`
  accepts only `Human` receipts (an `ol-types` change to PLAN's `FrameReceipt`).
- Default **Agent Framing: Ask**. An optional **"Allow for this session"** mode (frames start
  without a click, receipts `origin: Agent` so they never count toward ARM) is off by default, not
  persisted, not settable by automation, and **not shipped before M3** (open question Q2). Reasons:
  AGENTS.md rule 1 forbids agents causing hardware I/O; PLAN D19 bench rules; the purpose of
  frame-before-mark is a human looking at where the burn lands.

**C3.5 Job submission state machine.**
```
job_submit ─(refusals)─► isError, nothing created
   │
   ▼
awaiting_human ─(human Frame, gesture)─► framing ─(≥1 full acked pass)─► awaiting_arm
   │                                                                     │
   ├─(human Reject)─► rejected                                           ├─(human ARM: gesture + typed ARM, or foot switch M3)─► running ─► finished │ stopped │ fault
   ├─(job_withdraw)─► withdrawn                                          ├─(10 min receipt age)─► awaiting_human
   ├─(newer job_submit)─► superseded                                     │
   ├─(any edit/undo: fingerprint changes)─► stale ◄──────────────────────┘
   └─(15 min)─► expired
```
- `job_submit` plans and validates in the GUI; it never queues bytes or touches the driver.
- The submission is a card in the Laser panel (fingerprint, decoded summary, agent note, client
  name), not a modal: it never steals focus from STOP and never blocks the human's own work.
- `job_status` lets the agent follow every transition, including `outcome` (passes marked, duration)
  and `stop_report`. It can stop the running job at any time with `stop`.
- The run itself goes through the unchanged PLAN path: token check, the driver's own receipt check,
  single-use nonce, run-ahead cap, watchdog.

**C3.6 Configuration that bounds safety is human-only.** Machine profiles (work area, source max
power, frequency range, pulse table), calibration and red-dot calibration feed `ol_safety::validate`.
An agent that could raise `max_power` or grow the work area could make the validator accept what it
should refuse, so those writes are human-only surfaces (B11), and `prefs.set` refuses their keys.

**C3.7 Stop is always available** (B10.4), from MCP, the control protocol, the GUI, the CLI and the
E-stop. No mode, lock, modal, budget or human-only state refuses it.

**C3.8 The physical input (M3).** After H11 confirms the LMC input port map (GP names `foot_pin =
15`; inputs via `ReadPort 0x0025`, research 05 §1.6 and H11), the ARM dialog offers "Press the foot
switch to ARM": the driver thread watches for a released → pressed → released edge after the dialog
opened (untainted), within 30 s. The foot switch then **replaces** the typed `ARM` (PLAN §3.6 layer
5). This is the only boundary that also holds against channel 5 (OS-level synthetic events),
because no software on the Mac can press it.

**C3.9 Procedural layers until M3** (PLAN §3.6 "Agents and the bench", extended):
1. AGENTS.md rule 1 (already in the repo) and the `CLAUDE.md` import.
2. The PreToolUse hook (hookify) blocks, besides PLAN's patterns (`ol (detect|status|frame|run|hw|
   bench)`, `ol-hw-raw`, `--features hw`, `OPENLASER_HW_PRESENT`): any Bash or
   `mcp__terminal__run_in_terminal` command matching `openlaser-hw`; and
   `mcp__computer-use__request_access` / `open_application` naming OpenLaser, so a computer-use
   agent cannot be granted the window (computer-use's own per-app approval is a second prompt Luiz
   would see). Luiz decides whether to install these hooks; this design only specifies them.
3. The bench-session file gate on `openlaser-hw`.
4. USB unplugged whenever executors (coding agents) work (D19).
5. Optional, UNVERIFIED heuristic: on macOS, read the posting process of the ARM click event
   (`CGEventGetIntegerValueField(…, kCGEventSourceUnixProcessID)` via the current `NSEvent`) and
   refuse non-HID sources. It is a speed bump, needs a safe objc2 binding, and is not a boundary.

### C4. Justification against PLAN §3.6

| PLAN §3.6 layer | How the automation design keeps it |
|---|---|
| 0 Physical | Untouched. Automation never reads or overrides interlocks; `stop` adds a software path, never replaces the E-stop |
| 1 `ol_safety::validate` | Agents see refusals as structured results (`job_plan`) and cannot pass a refusal: no override field exists in any schema (CI grep), machine limits are human-only (C3.6), and the GUI re-validates at submit and at ARM |
| 2 Types | Unchanged; agent framing reaches only `FramePlan`/`LightingList`, and only after a human gesture (C3.4) |
| 3 Frame ⊇ burn | Same `JobPlan` for frame and burn; only human-originated receipts count toward ARM (C3.4) |
| 4 Confirm from bytes | The ARM dialog decodes its own bytes; the agent's summary and note never enter the confirmation (C3.3) |
| 5 ARM | GUI: typed `ARM` (M1–M2) or foot switch (M3) behind the provenance gate (C3.2); CLI: TTY in `openlaser-hw`, a binary with no server code. "Never: an API, script or LAN path": no such method exists (C3.1), enforced by the crate graph (B2.1) |
| 6 Driver invariants | Unchanged; `machine_status` reads cached events and adds no USB I/O; `stop` uses the existing latch |
| 7 Interlocks | Unchanged |
| 8 Cold build at ARM | The submission's fingerprint must equal the cold rebuild at ARM, or it is `stale` |
| Agents and the bench (5 layers) | Kept and extended to MCP and terminal tools (C3.9); the foot switch (layer 5) becomes the decisive layer once it exists |

**Against Luiz's wish ("agents can operate the app").** Agents get the whole LightBurn-equivalent
workflow up to the burn, plus the byte-exact view of the burn through `job_simulate` (FakeLmc runs
the encoded packets and renders the emission), plus status and stop. The human's part shrinks to
looking at the red frame and typing `ARM` (or pressing the pedal). That is the smallest human step
that still means "a person saw where this lands and chose to fire", and it matches LightBurn's REST
API, whose documented intent is that the operator always starts the job.

### C5. What this does not protect against

- A program on the Mac that can post OS-level mouse and keyboard events to the OpenLaser window
  before M3's foot switch (C2 #5). Mitigated procedurally only.
- An agent with a tool that types into a real terminal running `openlaser-hw` (C2 #6), beyond the
  hook and bench-session speed bumps.
- Hostile code linked into the GUI process.
- A human who arms without looking. The frame and the decoded confirmation inform; they do not
  compel.

### C6. Tests that prove it

All in CI, on FakeLmc, no hardware:
- `xtask_arm_reverse_deps_exact` ({`ol-human`, `openlaser-hw`}); `xtask_automation_closure_has_no_arm_or_usb`
  (`ol-automation`, `apps/openlaser-cli`); `xtask_no_server_crates_in_arm_closure`.
- `control_methods_table_is_exact` (the `methods` reply equals B10.6; no arm/run/connect/jog).
- `schemas_have_no_override_fields` (no `force|proceed|arm|confirm|skip_frame|override` property).
- kittest, each mutation-checked:
  `synthetic_click_on_arm_confirm_mints_nothing`, `synthetic_type_arm_clears_field_and_checklist`,
  `accesskit_click_on_arm_confirm_mints_nothing`, `synthetic_click_on_frame_button_does_not_frame`,
  `synthetic_click_on_connect_does_nothing`, `human_click_after_300ms_mints_token` (harness input
  counts as human), `stop_accepts_synthetic_click`.
- Protocol tests: `frame_request_without_approval_sends_no_lighting_packets` (FakeLmc records zero
  packets), `frame_request_approved_frames_and_receipt_is_human`,
  `agent_origin_receipt_is_refused_by_mint`, `job_submit_never_reaches_driver_run`,
  `edit_after_submit_marks_stale`, `submit_with_refusals_creates_nothing`,
  `human_only_command_via_engine_execute_refused`, `prefs_set_refuses_machine_and_automation_keys`,
  `machine_profile_write_via_control_refused`.
- Stop: `mcp_stop_latches_within_budget_while_ui_thread_blocked` (the UI loop is parked; the latch
  still trips and FakeLmc reports the stop in ≤ 100 ms p99, PLAN's emulator budget),
  `stop_served_while_human_surface_open`, `stop_served_while_machine_busy`.
- Wire tests copied from PhotoCraft: `-32700` recovery, unknown argument `-32602`, oversized line
  ends the session, batch reply budget escaped, modern list fields, panic containment, bridge
  failure wording.
- Fuzz the control-protocol line parser and the discovery-file reader (cargo-fuzz, as PLAN §6.3).

The red zone (PLAN §6.4) grows to include `ol-human`, the control dispatcher's refusal paths,
the frame-request and submission state machines, and `machine.stop`'s fast path: Claude designs
them, a second model reviews, Luiz reviews behaviour.

---

## Part D. Rollout

| Step | When (PLAN milestone) | Scope | Effort (focused weeks, rough) |
|---|---|---|---|
| A0 Headless | After M1.2 (needs `ol-plan`, `ol-safety`, encoders, FakeLmc), parallel with M1.3 | `ol-control-proto`, `ol-automation` headless, `openlaser-cli` (`mcp`, `serve`, `commands`, `info`, `convert`, `run`, `plan`, `preview`, `simulate`, `estimate`, `job-export`), all H tools, resources, wire tests, B2.1 xtask rules | 2 |
| A1 Bridge | M2 (needs the GUI, snapshot undo, `ol-human`) | GUI control server, discovery file, agent indicator, `ol-human` gate, read/design/UI tools, `stop` fast path, `job_submit` + `frame_request` with approval, C6 tests, `gcode_export` | 3 (red zone: + review time) |
| A2 Physical ARM | M3 (after H11) | Foot-switch ARM; decide Q2 (agent framing session mode) | 1 |
| A3 LightBurn shims | T2, by demand | LightBurn-REST-compatible subset (port 19520 style pairing with 30 s on-screen approval) and UDP 19840/19841 verbs mapped onto the same control surface; `START` maps to `job.submit`, never to emission | 1–2 |

A0 is executor-friendly (tool table + PhotoCraft wire tests as the spec). A1's `ol-human`, the
dispatcher refusals and the stop path are red zone.

---

## Part E. Open questions

- **Q1. One CLI or two?** AGENTS.md mentions `openlaser-cli` frame/mark/run "against a real
  device". This design moves those to `openlaser-hw` so the MCP binary cannot link `ol-arm` or
  `nusb` (B2). Luiz/SPEC.md must confirm the split (or accept a weaker single-binary variant where
  only crate boundaries, not linkage, separate them).
- **Q2. Agent framing without a click?** Default here: every agent-requested frame needs a human
  Approve click. Should M3 add a per-session "Allow for this session" mode? Depends on the JPT M7
  red pointer's laser class (UNVERIFIED) and H26.
- **Q3. `ol-app` transport injection.** PLAN §3.1 lets `ol-app` depend on `ol-lmc-usb`; B2.1 needs
  injection instead. Confirm before M1.1 crates are laid out.
- **Q4. `ol-ui` → `ol-human` split** changes PLAN's xtask rule from {`ol-ui`, `ol-cli`} to
  {`ol-human`, `openlaser-hw`}. Red-zone decision.
- **Q5. Hooks on computer-use and terminal tools** (C3.9 #2): Luiz must agree to install them; they
  also block legitimate computer-use of OpenLaser for screenshots (agents should use
  `ui_screenshot` instead).
- **Q6. Discovery file contents.** Token inline (PdfCraft) vs a separate token file (PhotoCraft).
  This design inlines it in a `0600` file inside a `0700` directory; confirm.
- **Q7. Should the agent indicator require a one-time on-screen "Allow this agent?" pairing**
  (LightBurn REST style) in addition to the token? Not needed for the boundary (nothing an agent
  can do emits), but it tells Luiz when a new client attaches.
- **Q8. egui 0.37 verification.** `clicked_by` excluding AccessKit/keyboard and AccessKit's lack of
  TextEdit `SetValue` were verified in egui 0.36.2 only; re-check in the M1.3 spike on 0.37.
- **Q9. Undo before M2?** If agents should edit in the GUI before snapshot undo lands, A1 would need
  a minimal history in M1.3; PLAN defers undo to M2.

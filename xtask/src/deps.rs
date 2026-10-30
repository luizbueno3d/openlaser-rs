//! `cargo xtask deps`: SPEC §2.2 (the exact edge lists) and §2.3 rules 1–17, from `cargo metadata` plus source
//! greps for rules 9, 11 and 12–17. Every violation is reported, then the task fails.
//!
//! Closures are computed on cargo's resolved graph (normal edges only) once per shipping platform (macOS, Windows
//! and Linux; §2.6), with the apps' `hw` feature off and on (rule 12: `cargo metadata --features openlaser/hw,
//! openlaser-hw/hw` resolves and never compiles).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::cargo;
use crate::files::{has_ext, rel, walk};

const TRANSPORT: &str = "openlaser-transport";
const ARM: &str = "openlaser-arm";
const HUMAN: &str = "openlaser-human";
const TESTKIT: &str = "openlaser-testkit";
const LMC_SIM: &str = "openlaser-lmc-sim";
const APP_GUI: &str = "openlaser";
const APP_CLI: &str = "openlaser-cli";
const APP_HW: &str = "openlaser-hw";

/// Shipping platforms (SPEC §2.6): every closure rule must hold on each.
const PLATFORMS: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc", "x86_64-unknown-linux-gnu"];

/// SPEC §2.2 (+ docs/SPEC-CHANGES.md): (crate, allowed internal normal deps, allowed external normal deps).
/// Names without the `openlaser-` prefix are workspace crates.
const TABLE: &[(&str, &[&str], &[&str])] = &[
    ("types", &[], &["serde", "thiserror"]),
    ("geom", &["types"], &["kurbo", "i_overlay", "rstar", "serde", "thiserror"]),
    ("doc", &["types", "geom"], &["kurbo", "i_overlay", "serde", "serde_json", "thiserror"]),
    ("machine", &["types", "geom", "doc"], &["serde", "thiserror"]),
    (
        "format",
        &["types", "geom", "doc", "machine", "camera"],
        &["zip", "flate2", "serde", "serde_json", "blake3", "craft-atomic", "craft-settings", "thiserror"],
    ),
    ("import", &["types", "geom", "doc"], &["usvg", "roxmltree", "base64", "image", "serde", "serde_json", "blake3", "thiserror"]),
    ("plan", &["types", "geom", "doc", "machine"], &["blake3", "serde", "thiserror"]),
    ("text", &["types", "geom"], &["kurbo", "skrifa", "harfrust", "serde", "thiserror"]),
    ("safety", &["types", "geom", "doc", "machine", "plan"], &["serde", "thiserror"]),
    ("arm", &["types"], &["getrandom"]),
    ("render", &["types", "geom", "doc", "machine", "plan"], &["tiny-skia", "thiserror"]),
    ("gcode", &["types", "geom", "machine", "plan", "safety"], &["serde", "blake3", "thiserror"]),
    // sha2: Packet's manual Debug shows a sha256 prefix (docs/SPEC-CHANGES.md).
    ("lmc", &["types", "geom", "machine", "plan", "safety"], &["serde", "blake3", "sha2", "thiserror"]),
    ("driver", &["types", "machine", "lmc", "gcode"], &["crossbeam-channel", "serde", "thiserror", "log"]),
    ("lmc-sim", &["types", "machine", "lmc", "driver"], &["crossbeam-channel", "thiserror"]),
    ("transport", &["types", "machine", "lmc", "driver"], &["nusb", "serialport", "keepawake", "craft-settings", "serde", "serde_json", "thiserror"]),
    ("camera", &[], &["nokhwa", "thiserror", "log", "serde"]),
    (
        "engine",
        &["types", "geom", "doc", "machine", "format", "import", "plan", "safety", "lmc", "gcode", "driver", "text", "camera"],
        &["serde", "serde_json", "schemars", "thiserror", "log"],
    ),
    ("control", &["types"], &["serde", "serde_json", "cap-std", "getrandom", "craft-atomic", "thiserror"]),
    (
        "automation",
        &["types", "doc", "machine", "plan", "engine", "control", "render", "lmc", "lmc-sim"],
        &["rmcp", "tokio", "schemars", "serde", "serde_json", "base64", "thiserror"],
    ),
    (
        "ui-egui",
        &["types", "geom", "doc", "machine", "plan", "engine", "control", "render", "text"],
        &["egui", "eframe", "egui_extras", "craft-ui", "craft-settings", "craft-atomic", "serde", "serde_json", "log"],
    ),
    ("human", &["types", "ui-egui", "arm"], &["egui"]),
    ("testkit", &["types"], &["serde", "serde_json", "sha2"]),
    (
        APP_GUI,
        &["types", "engine", "control", "ui-egui", "human", "lmc-sim", "format", "transport"],
        &["eframe", "egui", "muda", "rfd", "craft-quit", "craft-settings", "signal-hook", "ctrlc", "log"],
    ),
    (APP_CLI, &["types", "engine", "control", "automation", "render", "lmc-sim"], &["serde_json"]),
    (APP_HW, &["types", "machine", "engine", "driver", "arm", "lmc-sim", "format", "transport"], &["crossterm", "signal-hook", "ctrlc", "serde_json"]),
    ("xtask", &[], &["serde_json", "sha2"]),
];

/// Rule 2: GUI crates.
const GUI: &[&str] = &["egui", "eframe", "egui_extras", "egui_kittest", "wgpu", "craft-ui", "muda", "rfd"];
const GUI_OWNERS: &[&str] = &["openlaser-ui-egui", HUMAN, APP_GUI];
/// Rule 3: server crates.
const SERVER: &[&str] = &["tokio", "rmcp", "hyper", "axum", "tungstenite"];
const SERVER_OWNERS: &[&str] = &["openlaser-automation", APP_CLI];
/// Rule 6.
const NO_SERVER_OR_SCRIPTING: &[&str] = &["rmcp", "tokio", "hyper", "axum", "tungstenite", "rhai", "extism", "pyo3"];
/// Rule 10: test-only features, and where they live.
const TEST_FEATURES: &[(&str, &str)] = &[("openlaser-safety", "unchecked-for-tests"), ("openlaser-lmc", "py-compat"), (ARM, "test-witness")];
/// Rule 10: dev-dependency edges that would otherwise be cycles, used only from integration tests in `tests/`:
/// driver's T6 runs on FakeLmc (§6.2, §6.3); `types/tests/arm_*.rs` get tokens from `arm::testing` (§3.9, §7.12).
const DEV_CYCLE_EXCEPTIONS: &[(&str, &str)] = &[("openlaser-driver", LMC_SIM), ("openlaser-types", ARM)];
/// Rule 11: the only files that may allow clippy::disallowed_methods.
const DISALLOWED_ALLOWED_IN: &[&str] = &["crates/arm/src/mint.rs", "crates/human/src/gesture.rs", "apps/openlaser-hw/src/tty_arm.rs"];
const DISALLOWED_PATHS: &[&str] = &[
    "openlaser_types::arm::ArmToken::issue",
    "openlaser_arm::HumanConfirm::from_gui_gesture",
    "openlaser_arm::GuiGestureWitness::new",
    "openlaser_arm::HumanConfirm::from_tty",
    "openlaser_arm::TtyWitness::new",
];

#[derive(Debug, Clone)]
struct Dep {
    name: String,
    kind: Kind,
    optional: bool,
    features: Vec<String>,
    workspace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
struct Pkg {
    deps: Vec<Dep>,
    features: BTreeMap<String, Vec<String>>,
}

struct Graph {
    /// "<target triple>" or "<target triple>, hw".
    platform: String,
    /// Workspace packages by name.
    ws: BTreeMap<String, Pkg>,
    /// Resolved normal edges, by package name (all packages; names are enough for these rules).
    normal: BTreeMap<String, BTreeSet<String>>,
}

pub fn run(root: &Path) -> Result<(), String> {
    let mut v = Vec::new();
    let mut crates = 0;
    for platform in PLATFORMS {
        for hw in [false, true] {
            let meta = metadata(root, platform, hw)?;
            let g = Graph::from_metadata(&meta, if hw { format!("{platform}, hw") } else { (*platform).to_owned() })?;
            check_graph(&g, &mut v);
            crates = g.ws.len();
        }
    }
    check_sources(root, &mut v);
    if v.is_empty() {
        println!("deps: {crates} workspace crates on {} platforms, hw off and on; SPEC §2.2 edges and §2.3 rules 1–17 hold", PLATFORMS.len());
        Ok(())
    } else {
        v.sort();
        v.dedup();
        Err(v.join("\n"))
    }
}

/// `hw`: resolve with the apps' hardware feature on (rule 12: metadata resolves, never compiles).
fn metadata(root: &Path, platform: &str, hw: bool) -> Result<Value, String> {
    let mut args = vec!["metadata", "--format-version", "1", "--filter-platform", platform];
    if hw {
        args.extend(["--features", "openlaser/hw,openlaser-hw/hw"]);
    }
    let out = Command::new(cargo()).args(&args).current_dir(root).env("OPENLASER_NO_HW", "1").output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata output: {e}"))
}

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

impl Graph {
    fn from_metadata(meta: &Value, platform: String) -> Result<Self, String> {
        let packages = meta.get("packages").and_then(Value::as_array).ok_or("metadata: no packages")?;
        let members: BTreeSet<String> = strs(meta.get("workspace_members")).into_iter().collect();
        let mut id_to_name = BTreeMap::new();
        let mut ws = BTreeMap::new();
        for p in packages {
            let id = p.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
            let name = p.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
            id_to_name.insert(id.clone(), name.clone());
            if !members.contains(&id) {
                continue;
            }
            let deps = p
                .get("dependencies")
                .and_then(Value::as_array)
                .map(|ds| {
                    ds.iter()
                        .map(|d| Dep {
                            name: d.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                            kind: match d.get("kind").and_then(Value::as_str) {
                                Some("dev") => Kind::Dev,
                                Some("build") => Kind::Build,
                                _ => Kind::Normal,
                            },
                            optional: d.get("optional").and_then(Value::as_bool).unwrap_or(false),
                            features: strs(d.get("features")),
                            workspace: d.get("path").is_some_and(|p| !p.is_null()),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let features =
                p.get("features").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), strs(Some(v)))).collect()).unwrap_or_default();
            ws.insert(name, Pkg { deps, features });
        }
        let nodes = meta.get("resolve").and_then(|r| r.get("nodes")).and_then(Value::as_array).ok_or("metadata: no resolve graph")?;
        let mut normal: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for n in nodes {
            let from = n.get("id").and_then(Value::as_str).and_then(|id| id_to_name.get(id)).cloned().unwrap_or_default();
            let entry = normal.entry(from).or_default();
            for d in n.get("deps").and_then(Value::as_array).into_iter().flatten() {
                let is_normal = d.get("dep_kinds").and_then(Value::as_array).is_some_and(|ks| ks.iter().any(|k| k.get("kind").is_none_or(Value::is_null)));
                if !is_normal {
                    continue;
                }
                if let Some(to) = d.get("pkg").and_then(Value::as_str).and_then(|id| id_to_name.get(id)) {
                    entry.insert(to.clone());
                }
            }
        }
        Ok(Graph { platform, ws, normal })
    }

    /// Normal (resolved) closure of `name`, excluding `name` itself.
    fn closure(&self, name: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![name.to_owned()];
        while let Some(n) = stack.pop() {
            for d in self.normal.get(&n).into_iter().flatten() {
                if seen.insert(d.clone()) {
                    stack.push(d.clone());
                }
            }
        }
        seen
    }
}

fn full(short: &str) -> String {
    if short.starts_with("openlaser") || short == "xtask" { short.to_owned() } else { format!("openlaser-{short}") }
}

fn check_graph(g: &Graph, v: &mut Vec<String>) {
    let table: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> =
        TABLE.iter().map(|(c, int, ext)| (full(c), (int.iter().map(|s| full(s)).collect(), ext.iter().map(|s| (*s).to_owned()).collect()))).collect();

    // §2.2: every workspace crate is registered, and its normal/build edges are listed.
    for (name, pkg) in &g.ws {
        let Some((int, ext)) = table.get(name) else {
            v.push(format!("{name}: not in xtask's SPEC §2.2 table (xtask/src/deps.rs); new crates go through the integrator"));
            continue;
        };
        for d in pkg.deps.iter().filter(|d| d.kind != Kind::Dev) {
            let ok = if d.workspace { int.contains(&d.name) } else { ext.contains(&d.name) };
            if !ok {
                v.push(format!("{name} -> {} ({:?}): not an allowed edge in SPEC §2.2", d.name, d.kind));
            }
        }
    }

    // Rule 1: only transport touches nusb/serialport; transport is never a dev-dependency and is a normal
    // dependency only of the two apps, optional, behind their `hw` feature.
    for (from, tos) in &g.normal {
        for usb in ["nusb", "serialport"] {
            if tos.contains(usb) && from != TRANSPORT {
                v.push(format!("rule 1 [{}]: {from} depends on {usb}; only {TRANSPORT} may", g.platform));
            }
        }
    }
    for (name, pkg) in &g.ws {
        for d in pkg.deps.iter().filter(|d| d.name == TRANSPORT) {
            if d.kind == Kind::Dev {
                v.push(format!("rule 1: {name} has {TRANSPORT} as a dev-dependency"));
            } else if name != APP_GUI && name != APP_HW {
                v.push(format!("rule 1: {name} depends on {TRANSPORT}; only {APP_GUI} and {APP_HW} may (feature hw)"));
            } else if !d.optional {
                v.push(format!("rule 1: {name} -> {TRANSPORT} must be optional (behind feature hw)"));
            }
        }
        if name == APP_GUI || name == APP_HW {
            let hw = pkg.features.get("hw").cloned().unwrap_or_default();
            if !hw.iter().any(|f| f == "dep:openlaser-transport") {
                v.push(format!("rule 1: {name} feature `hw` must enable dep:openlaser-transport"));
            }
            for (feat, list) in &pkg.features {
                if feat != "hw" && list.iter().any(|f| f.contains(TRANSPORT) || f == "hw") {
                    v.push(format!("rule 1: {name} feature `{feat}` reaches {TRANSPORT}; only `hw` may"));
                }
            }
        }
    }

    // Rule 2 and 3: direct dependencies (any kind) on GUI and server crates; GUI never in other closures.
    for (name, pkg) in &g.ws {
        for d in &pkg.deps {
            if GUI.contains(&d.name.as_str()) && !GUI_OWNERS.contains(&name.as_str()) {
                v.push(format!("rule 2: {name} depends on {}; only ui-egui, human and openlaser may", d.name));
            }
            if SERVER.contains(&d.name.as_str()) && !SERVER_OWNERS.contains(&name.as_str()) {
                v.push(format!("rule 3: {name} depends on {}; only automation and openlaser-cli may", d.name));
            }
        }
        if !GUI_OWNERS.contains(&name.as_str()) {
            let closure = g.closure(name);
            for gui in GUI {
                if closure.contains(*gui) {
                    v.push(format!("rule 2 [{}]: {gui} is in {name}'s dependency closure", g.platform));
                }
            }
        }
    }

    // Rule 4: normal reverse dependencies of arm are exactly {human, openlaser-hw}.
    let arm_users: BTreeSet<&str> =
        g.ws.iter().filter(|(_, p)| p.deps.iter().any(|d| d.name == ARM && d.kind == Kind::Normal)).map(|(n, _)| n.as_str()).collect();
    let want: BTreeSet<&str> = [HUMAN, APP_HW].into_iter().collect();
    if arm_users != want {
        v.push(format!("rule 4: normal reverse dependencies of {ARM} are {arm_users:?}; must be exactly {want:?}"));
    }

    // Rule 5: agent closures.
    for agent in ["openlaser-automation", APP_CLI] {
        let c = g.closure(agent);
        for bad in [ARM, HUMAN, TRANSPORT, "nusb", "serialport"].iter().chain(GUI) {
            if c.contains(*bad) {
                v.push(format!("rule 5 [{}]: {bad} is in {agent}'s dependency closure", g.platform));
            }
        }
    }

    // Rule 6: human surfaces link no server or scripting crate (checked with hw off and on).
    for h in [HUMAN, APP_HW] {
        let c = g.closure(h);
        for bad in NO_SERVER_OR_SCRIPTING {
            if c.contains(*bad) {
                v.push(format!("rule 6 [{}]: {bad} is in {h}'s dependency closure", g.platform));
            }
        }
    }

    // Rule 7: the dispatcher cannot reach arm or human.
    let ui = g.closure("openlaser-ui-egui");
    for bad in [ARM, HUMAN] {
        if ui.contains(bad) {
            v.push(format!("rule 7 [{}]: {bad} is in openlaser-ui-egui's dependency closure", g.platform));
        }
    }

    // Rule 8: safety depends on no encoder or driver crate.
    let safety = g.closure("openlaser-safety");
    for bad in ["openlaser-lmc", "openlaser-gcode", "openlaser-driver", LMC_SIM, TRANSPORT] {
        if safety.contains(bad) {
            v.push(format!("rule 8 [{}]: {bad} is in openlaser-safety's dependency closure", g.platform));
        }
    }

    check_dev_edges(g, &table, v);
}

/// Rule 10 (and the dev-dependency half of rules 1, 5 and 7).
fn check_dev_edges(g: &Graph, table: &BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>, v: &mut Vec<String>) {
    for (name, pkg) in &g.ws {
        // Test-only features are enabled only on dev edges and never forwarded by a feature.
        for d in &pkg.deps {
            for (krate, feat) in TEST_FEATURES {
                if d.name == *krate && d.features.iter().any(|f| f == feat) && d.kind != Kind::Dev {
                    v.push(format!("rule 10: {name} enables {krate}/{feat} on a {:?} edge; only dev-dependencies may", d.kind));
                }
            }
        }
        for (feat, list) in &pkg.features {
            for (krate, tf) in TEST_FEATURES {
                let own = name == krate && feat == tf;
                let forwards = list.iter().any(|f| f == &format!("{krate}/{tf}") || f == &format!("{krate}?/{tf}"));
                let default_on = name == krate && feat == "default" && list.iter().any(|f| f == tf);
                if (forwards && !own) || default_on {
                    v.push(format!("rule 10: {name} feature `{feat}` turns on the test-only {krate}/{tf}"));
                }
            }
        }

        // Dev-dependency edges on workspace crates.
        for d in pkg.deps.iter().filter(|d| d.kind == Kind::Dev && d.workspace) {
            let to = d.name.as_str();
            let enables_test_feature = TEST_FEATURES.iter().any(|(k, f)| *k == to && d.features.iter().any(|x| x == f));
            if to == TRANSPORT {
                continue; // reported by rule 1
            }
            if matches!(name.as_str(), "openlaser-automation" | APP_CLI | "openlaser-ui-egui") && (to == ARM || to == HUMAN) {
                v.push(format!("rule 10: {name} has {to} as a dev-dependency (rules 5 and 7 keep it out entirely)"));
                continue;
            }
            if to == ARM && name != ARM && !enables_test_feature {
                v.push(format!("rule 10: {name} has {ARM} as a dev-dependency without test-witness"));
                continue;
            }
            let listed = table.get(name).is_some_and(|(int, _)| int.contains(to));
            let self_edge = to == name;
            let exception = DEV_CYCLE_EXCEPTIONS.iter().any(|(f, t)| f == name && *t == to);
            let acyclic = !g.closure(to).contains(name.as_str());
            if !(to == TESTKIT || self_edge || listed || exception || acyclic) {
                v.push(format!("rule 10: {name} -> {to} (dev) would be a dependency cycle"));
            }
        }
    }
}

fn check_sources(root: &Path, v: &mut Vec<String>) {
    let rust_files = walk(root, &|p| has_ext(p, "rs"));

    // Rule 9: std::process::Command only in xtask and apps/*.
    for f in &rust_files {
        let r = rel(root, f);
        if r.starts_with("xtask/") || r.starts_with("apps/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        if uses_process_command(&text) {
            v.push(format!("rule 9: {r} uses std::process::Command; only xtask and apps/* may"));
        }
    }

    // Rule 11: allow(clippy::disallowed_methods) only in the three minting files; clippy.toml lists the paths.
    let allow_pat = ["allow(clippy::", "disallowed_methods)"].concat();
    let expect_pat = ["expect(clippy::", "disallowed_methods)"].concat();
    for f in &rust_files {
        let r = rel(root, f);
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        // Code only: `//` comments may mention the attribute.
        let compact: String = text.lines().map(|l| l.split("//").next().unwrap_or_default()).flat_map(str::chars).filter(|c| !c.is_whitespace()).collect();
        if (compact.contains(&allow_pat) || compact.contains(&expect_pat)) && !DISALLOWED_ALLOWED_IN.contains(&r.as_str()) {
            v.push(format!("rule 11: {r} allows clippy::disallowed_methods; only {DISALLOWED_ALLOWED_IN:?} may"));
        }
    }
    match std::fs::read_to_string(root.join("clippy.toml")) {
        Ok(cfg) => {
            for p in DISALLOWED_PATHS {
                if !cfg.contains(&format!("\"{p}\"")) {
                    v.push(format!("rule 11: clippy.toml disallowed-methods is missing {p}"));
                }
            }
        }
        Err(e) => v.push(format!("rule 11: clippy.toml: {e}")),
    }

    check_source_rules(root, &rust_files, v);

    // Rule 12: nobody runs --all-features or builds the hw feature from CI, scripts or cargo config.
    let all_features = ["--all", "-features"].concat();
    let hw_feature = ["features", " hw"].concat();
    let hw_feature_eq = ["features", "=hw"].concat();
    let scripts = walk(root, &|p| {
        let r = rel(root, p);
        r.starts_with(".github/") || r.starts_with(".cargo/") || has_ext(p, "sh") || r == "Makefile" || r == "justfile"
    });
    for f in scripts {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for (pat, what) in [(&all_features, "--all-features"), (&hw_feature, "the hw feature"), (&hw_feature_eq, "the hw feature")] {
            if text.contains(pat.as_str()) {
                v.push(format!("rule 12: {} uses {what}", rel(root, &f)));
            }
        }
    }
}

/// Rules 13–17: source greps over code (never `//` comments). Patterns are assembled at run time so this file does
/// not match itself.
fn check_source_rules(root: &Path, rust_files: &[std::path::PathBuf], v: &mut Vec<String>) {
    let origin_human = ["Origin::", "Human"].concat();
    // (path, the crate that defines it: its own messages may name the path).
    let test_only = [
        (["openlaser_safety::", "testing"].concat(), "crates/safety/"),
        (["openlaser_arm::", "testing"].concat(), "crates/arm/"),
        (["openlaser_lmc::", "py_compat"].concat(), "crates/lmc/"),
    ];
    let usb_forbidden = [".transfer_blocking(", ".reset(", "::reset(", ".clear_halt(", ".detach_and_claim_interface("];
    let event_kinds = ["PointerButton", "PointerMoved", "Key", "Text", "Paste", "AccessKitActionRequest"];
    let dir_store = ["DirProfile", "Store"].concat();

    for f in rust_files {
        let r = rel(root, f);
        if r.starts_with("xtask/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let in_tests_dir = r.split('/').any(|c| c == "tests");
        let mut after_cfg_test = false;
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                after_cfg_test = true;
            }
            let code = line.split("//").next().unwrap_or_default();
            let at = format!("{r}:{}", i + 1);

            // Rule 13: Origin::Human only where a human is (the engine defines and matches it).
            let human_ok = ["crates/ui-egui/", "crates/human/", "apps/openlaser/", "apps/openlaser-hw/", "crates/engine/"].iter().any(|p| r.starts_with(p));
            if !human_ok && names_path(code, &origin_human) {
                v.push(format!("rule 13: {at} names {origin_human}; only ui-egui, human, the GUI, openlaser-hw (and the engine) may"));
            }
            // Rule 14: test-only modules only from tests/ and #[cfg(test)].
            if !in_tests_dir && !after_cfg_test {
                for (pat, home) in &test_only {
                    if code.contains(pat.as_str()) && !r.starts_with(home) {
                        v.push(format!("rule 14: {at} uses {pat} outside tests"));
                    }
                }
            }
            // Rule 15: no USB reset or unbounded blocking transfer in transport.
            if r.starts_with("crates/transport/") {
                for pat in usb_forbidden {
                    if code.contains(pat) {
                        v.push(format!("rule 15: {at} calls {pat}…) in openlaser-transport"));
                    }
                }
            }
            // Rule 16: synthetic egui events are built only in ui-egui's src/control/input.rs (and tests).
            if r.starts_with("crates/ui-egui/") && r != "crates/ui-egui/src/control/input.rs" && !in_tests_dir && !after_cfg_test {
                let pattern_context = code.contains("matches!(") || code.contains("if let") || code.contains("=>") || code.contains("..");
                for kind in event_kinds {
                    if code.contains(&format!("Event::{kind}")) && !pattern_context {
                        v.push(format!("rule 16: {at} builds egui::Event::{kind}; only src/control/input.rs may"));
                    }
                }
            }
            // Rule 17: DirProfileStore only in the two human apps (and where format defines it).
            let store_ok = ["apps/openlaser/", "apps/openlaser-hw/", "crates/format/"].iter().any(|p| r.starts_with(p));
            if !store_ok && code.contains(&dir_store) {
                v.push(format!("rule 17: {at} names {dir_store}; only apps/openlaser and apps/openlaser-hw may"));
            }
        }
    }
}

/// `process::Command` in code (not in `//` comments), or a `use std::process::{…Command…}` group.
fn uses_process_command(text: &str) -> bool {
    let mut in_group = false;
    for line in text.lines() {
        let code = line.split("//").next().unwrap_or_default();
        if code.contains("process::Command") {
            return true;
        }
        if code.contains("process::{") {
            in_group = true;
        }
        if in_group {
            if code.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "Command") {
                return true;
            }
            if code.contains('}') {
                in_group = false;
            }
        }
    }
    false
}

/// `code` names `path` (e.g. `Origin::Human`) as a path of its own: an occurrence preceded by an identifier character
/// (`FrameOrigin::Human`, a different enum) does not count.
fn names_path(code: &str, path: &str) -> bool {
    code.match_indices(path).any(|(i, _)| !code.get(..i).and_then(|before| before.chars().next_back()).is_some_and(|c| c.is_alphanumeric() || c == '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_13_matches_the_engine_origin_not_frame_origin() {
        let p = ["Origin::", "Human"].concat();
        assert!(names_path("let o = Origin::Human;", &p));
        assert!(names_path("x(openlaser_engine::Origin::Human)", &p));
        assert!(!names_path("use openlaser_types::FrameOrigin::Human;", &p));
        assert!(!names_path("let o = MyOrigin::Human;", &p));
        assert!(names_path("FrameOrigin::Human; Origin::Human", &p));
    }

    #[test]
    fn process_command_grep() {
        assert!(uses_process_command("use std::process::Command;"));
        assert!(uses_process_command("use std::process::{ExitCode,\n    Command};"));
        assert!(uses_process_command("let c = std::process::Command::new(\"x\");"));
        assert!(!uses_process_command("// std::process::Command is not used here"));
        assert!(!uses_process_command("use std::process::{ExitCode, exit};\nfn command() {}"));
    }

    #[test]
    fn table_names_are_unique() {
        let names: BTreeSet<&str> = TABLE.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(names.len(), TABLE.len());
    }
}

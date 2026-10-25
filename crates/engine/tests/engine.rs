//! WP6 acceptance tests over the public engine API (SPEC §7.7). One binary, grouped by module:
//!
//! - `registry`: ids, aliases, schemas, the policy table, the `command_list` snapshot;
//! - `undo`: an undo round trip for every Edit command and `prop_undo_redo_roundtrip`;
//! - `emission`: the human emission path end to end on a FakeLmc (WP1–WP5, merged at I1);
//! - `gantry`: a gantry job lands where it was designed (plan → safety → G-code coordinates, pinned at I1);
//! - `panic_hunt`: every command × `testkit::adversarial::params()` (`#[ignore]`, run at I2).
//!
//! Nothing here opens a real device: every provider is a test double or a FakeLmc (AGENTS.md §1).

// Test helpers outside `#[test]` fns: clippy's in-test allowances do not reach them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use openlaser_doc::{ColorLayer, CutMode, CutOp, ImageCut, ImageMode, Project};
use openlaser_engine::{Access, AgentClass, DeviceAccess, EngineError, FsAuthority, Kind, ListFilter, Origin, Session, SessionConfig, registry};
use openlaser_machine::{MachineProfile, jpt_m7_200, jpt_m7_200_sim};
use openlaser_types::{Clock, VirtualClock};
use serde_json::{Value, json};

// ---- fixtures ----------------------------------------------------------------------------------------------------

/// A colour layer from the profile's defaults (WP1's `Project::new` builds these; the WP0 stub does not).
fn color_layer(i: u8, m: &MachineProfile) -> ColorLayer {
    let d = &m.layer_defaults;
    ColorLayer {
        index: i,
        name: format!("C{i:02}"),
        output: true,
        show: true,
        ops: vec![CutOp {
            name: String::new(),
            enabled: true,
            mode: CutMode::Line,
            passes: 1,
            galvo: d.galvo.clone(),
            gantry: d.gantry.clone(),
            fill: d.fill.clone(),
        }],
        global_passes: 1,
        image: ImageCut {
            mode: ImageMode::Threshold,
            interval: d.fill.interval,
            angle: d.fill.angle,
            bidirectional: true,
            negative: false,
            threshold: 128,
            passes: 1,
            galvo: d.galvo.clone(),
            gantry: d.gantry.clone(),
        },
        air_assist: false,
    }
}

/// `Project::new` for the machine, with all 30 colour layers whatever the model crate provides yet.
fn full_project(m: &MachineProfile) -> Project {
    let ws = openlaser_doc::Workspace { kind: openlaser_types::DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: openlaser_doc::WorkspaceOrigin::Center };
    let mut p = Project::new(ws, &m.layer_defaults);
    p.name = "Test".into();
    if p.layers.colors.len() != 30 {
        p.layers.colors = (0..30u8).map(|i| color_layer(i, m)).collect();
    }
    p
}

fn config(origin: Origin, window: bool, devices: DeviceAccess, clock: Arc<dyn Clock>) -> SessionConfig {
    SessionConfig { origin, window, devices, machines: vec![jpt_m7_200_sim(), jpt_m7_200()], profiles: None, files: FsAuthority::TrustedLocal, clock }
}

fn headless() -> Session {
    Session::new(config(Origin::Headless, false, DeviceAccess::None, Arc::new(VirtualClock::new())))
}

fn with_doc(s: &mut Session) {
    let m = jpt_m7_200_sim();
    s.open_project(full_project(&m), None, Some(m.id.clone())).unwrap();
}

fn ids_of(v: &Value) -> Vec<u64> {
    v["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
}

/// A device provider that counts opens and hands out transports that refuse everything.
#[derive(Default)]
struct CountingProvider {
    opens: AtomicU32,
}

struct DeadTransport;

impl openlaser_lmc::LmcTransport for DeadTransport {
    fn submit_single(&mut self, _cmd: openlaser_lmc::Single) -> Result<(), openlaser_lmc::XferError> {
        Err(openlaser_lmc::XferError::HardwareDisabled)
    }
    fn poll_single(&mut self, _slice: Duration) -> Result<Option<Option<[u16; 4]>>, openlaser_lmc::XferError> {
        Err(openlaser_lmc::XferError::HardwareDisabled)
    }
    fn submit_packet(&mut self, _pkt: &openlaser_lmc::Packet) -> Result<(), openlaser_lmc::XferError> {
        Err(openlaser_lmc::XferError::HardwareDisabled)
    }
    fn poll_packet(&mut self, _slice: Duration) -> Result<Option<()>, openlaser_lmc::XferError> {
        Err(openlaser_lmc::XferError::HardwareDisabled)
    }
    fn cancel_all(&mut self) {}
    fn drain_replies(&mut self) {}
}

impl openlaser_driver::DeviceProvider for CountingProvider {
    fn name(&self) -> &'static str {
        "counting"
    }
    fn detect(&self) -> Result<Vec<openlaser_driver::DetectedDevice>, openlaser_driver::DriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
    fn open_lmc(&self, _m: &openlaser_machine::GalvoMachine) -> Result<Box<dyn openlaser_lmc::LmcTransport>, openlaser_driver::DriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(DeadTransport))
    }
    fn open_link(&self, _m: &openlaser_machine::GantryMachine) -> Result<Box<dyn openlaser_driver::ByteLink>, openlaser_driver::DriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Err(openlaser_driver::DriverError::NotConnected)
    }
    fn emission_cap_pct(&self) -> Result<Option<f64>, openlaser_driver::DriverError> {
        Ok(None)
    }
    fn record_emission(&self) -> Result<(), openlaser_driver::DriverError> {
        Ok(())
    }
}

// ---- registry and policy -------------------------------------------------------------------------------------------

mod registry_tests {
    use super::*;

    /// `^[a-z][a-zA-Z0-9]*(\.[a-z][a-zA-Z0-9]*)+$`
    fn id_grammar(id: &str) -> bool {
        let segs: Vec<&str> = id.split('.').collect();
        segs.len() >= 2
            && segs.iter().all(|s| {
                let mut c = s.chars();
                c.next().is_some_and(|f| f.is_ascii_lowercase()) && c.all(|x| x.is_ascii_alphanumeric())
            })
    }

    fn camel_to_snake(seg: &str) -> String {
        let mut out = String::new();
        for c in seg.chars() {
            if c.is_ascii_uppercase() {
                out.push('_');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn registry_ids_unique() {
        let r = registry();
        assert!(r.duplicates().is_empty(), "duplicate ids: {:?}", r.duplicates());
        let mut ids: Vec<&str> = r.specs().iter().map(|s| s.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert!(n >= 70, "the Appendix A commands are registered ({n})");
    }

    #[test]
    fn registry_id_grammar() {
        for s in registry().specs() {
            assert!(id_grammar(s.id), "`{}` breaks the id grammar", s.id);
        }
        assert!(!id_grammar("Edit.undo") && !id_grammar("edit") && !id_grammar("edit.undo_x") && !id_grammar("edit..x"));
    }

    #[test]
    fn snake_aliases_resolve() {
        let r = registry();
        for s in r.specs() {
            let snake: Vec<String> = s.id.split('.').map(camel_to_snake).collect();
            let alias = snake.join(".");
            let found = r.find(&alias).unwrap_or_else(|| panic!("alias `{alias}` of `{}` does not resolve", s.id));
            assert_eq!(found.id, s.id);
        }
        assert_eq!(r.find("arrange.flip_h").map(|s| s.id), Some("arrange.flipH"));
        assert_eq!(r.find("layer.cut_settings.set").map(|s| s.id), Some("layer.cutSettings.set"));
        assert!(r.find("arrange.flip_x").is_none());
        let e = headless().execute("arrange.flipp_h", json!({})).unwrap_err();
        match e {
            EngineError::UnknownCommand { suggestions, .. } => assert!(suggestions.contains(&"arrange.flipH".to_owned()), "{suggestions:?}"),
            other => panic!("{other:?}"),
        }
    }

    fn walk(v: &Value, path: &str, f: &mut dyn FnMut(&str, &serde_json::Map<String, Value>)) {
        match v {
            Value::Object(m) => {
                f(path, m);
                for (k, x) in m {
                    walk(x, &format!("{path}/{k}"), f);
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    walk(x, &format!("{path}/{i}"), f);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn every_schema_generates() {
        let s = headless();
        for spec in registry().specs() {
            let v = serde_json::to_value((spec.schema)()).unwrap();
            assert_eq!(v["type"], "object", "{}: params are an object", spec.id);
            assert_eq!(v["additionalProperties"], false, "{}: schemas are strict", spec.id);
            let via_session = s.command_schema(spec.id).unwrap();
            assert_eq!(via_session["schema"], v);
            // Strict all the way down: every nested object schema with properties rejects unknown keys.
            walk(&v, spec.id, &mut |path, m| {
                if m.contains_key("properties") {
                    assert_eq!(m.get("additionalProperties"), Some(&Value::Bool(false)), "{path} is not strict");
                }
            });
        }
    }

    #[test]
    fn schemas_have_no_override_fields() {
        const FORBIDDEN: [&str; 8] = ["force", "proceed", "arm", "confirm", "skip_frame", "skipFrame", "override", "token"];
        for spec in registry().specs() {
            let v = serde_json::to_value((spec.schema)()).unwrap();
            walk(&v, spec.id, &mut |path, m| {
                if let Some(Value::Object(props)) = m.get("properties") {
                    for k in props.keys() {
                        assert!(!FORBIDDEN.contains(&k.as_str()), "{path} has a forbidden property `{k}`");
                    }
                }
            });
        }
    }

    #[test]
    fn human_only_commands_have_no_menu_or_shortcut() {
        let human: Vec<_> = registry().specs().iter().filter(|s| s.access == Access::Human).collect();
        let ids: Vec<&str> = human.iter().map(|s| s.id).collect();
        for want in [
            "laser.frame",
            "laser.start",
            "device.connect",
            "device.disconnect",
            "device.detect",
            "device.settings.set",
            "device.create",
            "device.importEzcad",
            "device.calibrateAxis",
        ] {
            assert!(ids.contains(&want), "{want} is human-only");
        }
        for s in human {
            assert!(s.menu.is_empty(), "{} has a menu path", s.id);
            assert!(s.shortcut.is_none(), "{} has a shortcut", s.id);
        }
        // Emission and framing are never reachable through any other access class.
        for s in registry().specs() {
            if matches!(
                s.kind,
                Kind::Device(openlaser_engine::DeviceClass::Emit | openlaser_engine::DeviceClass::Light | openlaser_engine::DeviceClass::Connect)
            ) {
                assert_eq!(s.access, Access::Human, "{}", s.id);
            }
        }
    }

    #[test]
    fn policy_human_only_refused_for_all_non_human_origins() {
        let provider = Arc::new(CountingProvider::default());
        let dyn_p: Arc<dyn openlaser_driver::DeviceProvider> = provider.clone();
        let mut s = Session::new(config(Origin::Human, true, DeviceAccess::Provider(dyn_p), Arc::new(VirtualClock::new())));
        with_doc(&mut s);
        let human: Vec<&str> = registry().specs().iter().filter(|s| s.access == Access::Human).map(|s| s.id).collect();
        for id in &human {
            for origin in [Origin::Synthetic, Origin::Headless, Origin::Replay] {
                // Refused before the params are decoded: valid, empty, unknown-key and override-shaped params alike.
                for params in [Value::Null, json!({}), json!({ "bogus": 1 }), json!({ "force": true, "token": "x" })] {
                    let e = s.execute_as(origin, id, params.clone()).unwrap_err();
                    assert_eq!(e.code(), "human_only", "{id} from {origin:?} with {params}: {e}");
                    assert!(e.agent_message().starts_with("human-only: "), "{}", e.agent_message());
                }
                let row = s.command_list(origin, &ListFilter::default()).into_iter().find(|r| r.id == *id).unwrap();
                assert!(!row.enabled);
                assert_eq!(row.agent, AgentClass::HumanOnly);
            }
            // A batch from a human is Replay: still refused.
            let r = s.batch(Origin::Human, vec![((*id).to_owned(), json!({}))], true, None);
            assert_eq!(r.results[0]["error"]["code"], "human_only", "{id} via batch");
        }
        assert_eq!(provider.opens.load(Ordering::SeqCst), 0, "no non-human call opened or enumerated a device");
        assert!(s.pending_arm().is_none());
    }

    #[test]
    fn ui_commands_need_a_window() {
        let mut s = headless();
        for spec in registry().specs().iter().filter(|s| s.access == Access::Ui) {
            let e = s.execute(spec.id, json!({})).unwrap_err();
            assert_eq!(e.code(), "disabled", "{}: {e}", spec.id);
        }
    }

    #[test]
    fn commands_snapshot() {
        let s = headless();
        let rows = s.command_list(Origin::Headless, &ListFilter { schema: true, ..ListFilter::default() });
        insta::assert_json_snapshot!("commands", rows);
    }
}

// ---- undo ----------------------------------------------------------------------------------------------------------

mod undo {
    use proptest::prelude::*;

    use super::*;

    /// Two objects, both selected: an open, nearly closed path on C00 and a rectangle on C01.
    fn scene() -> (Session, u64, u64) {
        let mut s = headless();
        with_doc(&mut s);
        let a = ids_of(&s.execute("tools.drawLines", json!({ "pointsMm": [[0, 0], [10, 0], [10, 10], [0, 0.2]] })).unwrap())[0];
        let b = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 20, "yMm": 20, "wMm": 10, "hMm": 5, "layer": 1 })).unwrap())[0];
        s.execute("edit.selectAll", json!({})).unwrap();
        // The session clipboard holds the selection so paste cases have something to land.
        s.execute("edit.copy", json!({})).unwrap();
        (s, a, b)
    }

    fn assert_undo_roundtrip(s: &mut Session, id: &str, params: Value) {
        let before = Arc::clone(&s.active().unwrap().project);
        let sel_before = s.active().unwrap().selection.clone();
        let depth = s.active().unwrap().history.labels().len();
        s.execute(id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        let after = Arc::clone(&s.active().unwrap().project);
        assert_ne!(*before, *after, "{id} changed nothing");
        assert_eq!(s.active().unwrap().history.labels().len(), depth + 1, "{id} records exactly one step");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().project, *before, "{id}: undo restores the project");
        assert_eq!(s.active().unwrap().selection, sel_before, "{id}: undo restores the selection");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().project, *after, "{id}: redo restores the edit");
    }

    /// Every Edit command with params that change the scene. Commands that need other packages' behaviour are in
    /// `needs_i1_cases`; `file.new` creates a document (nothing to undo) and undo/redo are the mechanism itself.
    fn cases(a: u64) -> Vec<(&'static str, Value)> {
        vec![
            ("edit.delete", json!({})),
            ("edit.duplicate", json!({})),
            ("edit.cut", json!({})),
            ("edit.paste", json!({})),
            ("edit.pasteInPlace", json!({})),
            ("edit.closePath", json!({ "ids": [a] })),
            ("tools.rectangle", json!({ "xMm": 0, "yMm": 0, "wMm": 5, "hMm": 5 })),
            ("tools.ellipse", json!({ "cxMm": 0, "cyMm": 0, "wMm": 4, "hMm": 2 })),
            ("tools.drawLines", json!({ "pointsMm": [[1, 1], [2, 3]] })),
            ("tools.polygon", json!({ "cxMm": 5, "cyMm": 5, "radiusMm": 5, "sides": 5 })),
            ("arrange.group", json!({})),
            ("arrange.flipH", json!({})),
            ("arrange.flipV", json!({})),
            ("arrange.rotateCw90", json!({})),
            ("arrange.rotateCcw90", json!({})),
            ("arrange.transform", json!({ "wMm": 50, "lockRatio": true, "anchor": "bottomLeft" })),
            ("arrange.nudge", json!({ "dxMm": 1, "dyMm": -1 })),
            ("arrange.rotateBy", json!({ "deg": 15 })),
            ("arrange.moveSelection.pageCenter", json!({})),
            ("layer.assign", json!({ "layer": "C07" })),
            ("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": 55 } })),
            ("layer.setMode", json!({ "layer": 0, "mode": "fill" })),
            ("layer.setOutput", json!({ "layer": 0, "on": false })),
            ("layer.setShow", json!({ "layer": 1, "on": false })),
            ("layer.reorder", json!({ "layer": "C01", "toIndex": 0 })),
            ("laser.cutSelected", json!({ "on": true })),
            ("laser.repeat", json!({ "count": 3 })),
        ]
    }

    /// `arrange.ungroup` needs a grouped scene (covered in `undo_roundtrip_edit_commands_needing_geometry_and_import`).
    const NEEDS_I1: [&str; 14] = [
        "file.import",
        "edit.autoJoin",
        "edit.deleteDuplicates",
        "arrange.ungroup",
        "tools.weld",
        "tools.booleanUnion",
        "tools.booleanSubtract",
        "tools.booleanIntersect",
        "tools.offsetShapes",
        "tools.text",
        "text.edit",
        "edit.convertToPath",
        "path.set",
        "tools.traceImage",
    ];

    /// `edit.copy` only writes the session clipboard: it changes no document and records no history step.
    const NOT_UNDOABLE: [&str; 4] = ["file.new", "edit.undo", "edit.redo", "edit.copy"];

    #[test]
    fn undo_roundtrip_every_edit_command() {
        let covered: Vec<&str> = cases(0).iter().map(|(id, _)| *id).collect();
        for spec in registry().specs().iter().filter(|s| s.kind == Kind::Edit) {
            assert!(
                covered.contains(&spec.id) || NEEDS_I1.contains(&spec.id) || NOT_UNDOABLE.contains(&spec.id),
                "{} is an Edit command without an undo round-trip case",
                spec.id
            );
        }
        let (_, a, _) = scene();
        for (id, params) in cases(a) {
            let (mut s, _, _) = scene();
            assert_undo_roundtrip(&mut s, id, params);
        }
    }

    #[test]
    fn undo_roundtrip_edit_commands_needing_geometry_and_import() {
        let (mut s, _, _) = scene();
        let svg = openlaser_testkit::fixture("square10mm.svg").to_string_lossy().into_owned();
        assert_undo_roundtrip(&mut s, "file.import", json!({ "path": svg }));
        let (mut s, _, _) = scene();
        let l1 = ids_of(&s.execute("tools.drawLines", json!({ "pointsMm": [[50, 0], [60, 0]] })).unwrap())[0];
        let l2 = ids_of(&s.execute("tools.drawLines", json!({ "pointsMm": [[60, 0], [70, 5]] })).unwrap())[0];
        fn b_id(s: &Session) -> u64 {
            s.active().unwrap().selection.ids.first().map(|i| i.0).unwrap_or(0)
        }
        assert_undo_roundtrip(&mut s, "edit.autoJoin", json!({ "ids": [l1, l2] }));
        let (mut s, _, _) = scene();
        let d1 = ids_of(&s.execute("tools.drawLines", json!({ "pointsMm": [[50, 0], [60, 0]] })).unwrap())[0];
        let d2 = ids_of(&s.execute("tools.drawLines", json!({ "pointsMm": [[50, 0], [60, 0]] })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "edit.deleteDuplicates", json!({ "ids": [d1, d2] }));
        // A grouped scene for ungroup: group the selection first (outside the round-trip), then undo through it.
        let (mut s, _, _) = scene();
        let g = ids_of(&s.execute("arrange.group", json!({})).unwrap())[0];
        assert_undo_roundtrip(&mut s, "arrange.ungroup", json!({ "ids": [g] }));
        // Closed-shape cases (the shared scene's open path would refuse them).
        let (mut s, _, _) = scene();
        let r1 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 100, "yMm": 100, "wMm": 10, "hMm": 10 })).unwrap())[0];
        let r2 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 105, "yMm": 105, "wMm": 10, "hMm": 10 })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "tools.weld", json!({ "ids": [r1, r2] }));
        let (mut s, _, _) = scene();
        let r1 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 100, "yMm": 100, "wMm": 10, "hMm": 10 })).unwrap())[0];
        let r2 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 105, "yMm": 105, "wMm": 10, "hMm": 10 })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "tools.booleanUnion", json!({ "ids": [r1, r2] }));
        let (mut s, _, _) = scene();
        let r1 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 100, "yMm": 100, "wMm": 10, "hMm": 10 })).unwrap())[0];
        let r2 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 105, "yMm": 105, "wMm": 10, "hMm": 10 })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "tools.booleanSubtract", json!({ "ids": [r1, r2] }));
        let (mut s, _, _) = scene();
        let r1 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 100, "yMm": 100, "wMm": 10, "hMm": 10 })).unwrap())[0];
        let r2 = ids_of(&s.execute("tools.rectangle", json!({ "xMm": 105, "yMm": 105, "wMm": 10, "hMm": 10 })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "tools.booleanIntersect", json!({ "ids": [r1, r2] }));
        let (mut s, _, _) = scene();
        let sel = b_id(&s);
        assert_undo_roundtrip(&mut s, "tools.offsetShapes", json!({ "ids": [sel], "distanceMm": 1.0 }));
        // Text commands need the committed Hack fixture (fonts are host-dependent).
        let hack = || std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../text/tests/fonts");
        let (mut s, _, _) = scene();
        s.set_font_dirs(vec![hack()]);
        assert_undo_roundtrip(&mut s, "tools.text", json!({ "xMm": 0, "yMm": 0, "text": "HI", "family": "Hack" }));
        let (mut s, _, _) = scene();
        s.set_font_dirs(vec![hack()]);
        let t = ids_of(&s.execute("tools.text", json!({ "xMm": 0, "yMm": 0, "text": "A", "family": "Hack" })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "text.edit", json!({ "id": t, "text": "WW" }));
        let (mut s, _, _) = scene();
        s.set_font_dirs(vec![hack()]);
        let t = ids_of(&s.execute("tools.text", json!({ "xMm": 0, "yMm": 0, "text": "A", "family": "Hack" })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "edit.convertToPath", json!({ "ids": [t] }));
        // path.set needs an existing Path shape.
        let (mut s, _, _) = scene();
        let p = ids_of(&s.execute("tools.drawLines", json!({ "pointsMm": [[0, 0], [10, 0], [10, 10]] })).unwrap())[0];
        assert_undo_roundtrip(&mut s, "path.set", json!({ "id": p, "d": "M0 0L20 0L20 5Z" }));
        // tools.traceImage needs a bitmap target: inject one, then trace it.
        let (mut s, _, _) = scene();
        let bid = {
            let mut g = vec![255u8; 20 * 20];
            for r in 5..15 {
                for c in 5..15 {
                    g[r * 20 + c] = 0;
                }
            }
            use std::sync::Arc;
            s.edit(openlaser_engine::Label::new("bitmap"), None, move |project, _| {
                let id = project.alloc_id().expect("fresh ids available");
                project.shapes.push(Arc::new(openlaser_doc::Shape {
                    id,
                    name: None,
                    layer: openlaser_types::LayerKey::Color(0),
                    xform: openlaser_doc::Affine::IDENTITY,
                    geom: openlaser_doc::Geom::Bitmap(Arc::new(openlaser_doc::BitmapData {
                        asset: openlaser_doc::AssetId([7; 32]),
                        px: [20, 20],
                        size_mm: [40.0, 40.0],
                        gray: Arc::from(g.as_slice()),
                        source_ext: "png".to_owned(),
                    })),
                    locked: false,
                    power_scale: openlaser_types::Percent::MAX_VALID,
                    priority: 0,
                }));
                Ok::<_, openlaser_engine::EngineError>(())
            })
            .map(|_| s.active().unwrap().project.shapes.last().unwrap().id.0)
            .unwrap()
        };
        s.execute("edit.select", json!({ "ids": [bid] })).unwrap();
        assert_undo_roundtrip(&mut s, "tools.traceImage", json!({ "id": bid, "smoothness": 0 }));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Draw(i8, i8, i8, i8),
        Rect(i8, i8),
        Nudge(i8, i8),
        Flip,
        Rotate(i16),
        Power(u8),
        Delete,
        Duplicate,
        Assign(u8),
        Repeat(u16),
        CutSelected(bool),
        SelectAll,
        SelectNone,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (any::<i8>(), any::<i8>(), any::<i8>(), any::<i8>()).prop_map(|(a, b, c, d)| Op::Draw(a, b, c, d)),
            (any::<i8>(), any::<i8>()).prop_map(|(x, y)| Op::Rect(x, y)),
            (any::<i8>(), any::<i8>()).prop_map(|(x, y)| Op::Nudge(x, y)),
            Just(Op::Flip),
            (-360i16..360).prop_map(Op::Rotate),
            (0u8..=100).prop_map(Op::Power),
            Just(Op::Delete),
            Just(Op::Duplicate),
            (0u8..30).prop_map(Op::Assign),
            (1u16..=999).prop_map(Op::Repeat),
            any::<bool>().prop_map(Op::CutSelected),
            Just(Op::SelectAll),
            Just(Op::SelectNone),
        ]
    }

    fn apply(s: &mut Session, op: &Op) {
        let (id, p) = match op {
            Op::Draw(a, b, c, d) => ("tools.drawLines", json!({ "pointsMm": [[a, b], [c, d]] })),
            Op::Rect(x, y) => ("tools.rectangle", json!({ "xMm": x, "yMm": y, "wMm": 3, "hMm": 2 })),
            Op::Nudge(x, y) => ("arrange.nudge", json!({ "dxMm": x, "dyMm": y })),
            Op::Flip => ("arrange.flipH", json!({})),
            Op::Rotate(d) => ("arrange.rotateBy", json!({ "deg": d })),
            Op::Power(p) => ("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": p } })),
            Op::Delete => ("edit.delete", json!({})),
            Op::Duplicate => ("edit.duplicate", json!({})),
            Op::Assign(l) => ("layer.assign", json!({ "layer": l })),
            Op::Repeat(n) => ("laser.repeat", json!({ "count": n })),
            Op::CutSelected(on) => ("laser.cutSelected", json!({ "on": on })),
            Op::SelectAll => ("edit.selectAll", json!({})),
            Op::SelectNone => ("edit.selectNone", json!({})),
        };
        // Many ops are refused in some states (nothing selected, nothing to undo); that is part of the property.
        let _ = s.execute(id, p);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn prop_undo_redo_roundtrip(ops in proptest::collection::vec(op(), 1..14)) {
            let mut s = headless();
            with_doc(&mut s);
            let initial = Arc::clone(&s.active().unwrap().project);
            for o in &ops {
                apply(&mut s, o);
            }
            let fin = Arc::clone(&s.active().unwrap().project);
            let mut guard = 0;
            while s.active().unwrap().history.can_undo() {
                s.execute("edit.undo", json!({})).unwrap();
                guard += 1;
                prop_assert!(guard < 200);
            }
            prop_assert_eq!(&*s.active().unwrap().project, &*initial);
            while s.active().unwrap().history.can_redo() {
                s.execute("edit.redo", json!({})).unwrap();
                guard += 1;
                prop_assert!(guard < 400);
            }
            prop_assert_eq!(&*s.active().unwrap().project, &*fin);
        }
    }
}

// ---- the human emission path on a FakeLmc --------------------------------------------------------------------------

mod emission {
    use std::sync::Mutex;
    use std::time::Instant;

    use openlaser_engine::{DriverEvent, Encoded, Event, FrameMode, FrameSource, FrameStyle, JobOptions, MachineState};
    use openlaser_lmc_sim::{FakeLmc, Recorder, SimConfig, SimWire, WireHook};
    use openlaser_types::RefusalCode;

    use super::*;

    /// FakeLmc on the session's virtual clock (the driver's slices advance it), so long jobs run fast.
    struct FakeProvider {
        clock: Arc<VirtualClock>,
        recorder: Mutex<Option<Arc<Recorder>>>,
        hook: Mutex<Option<WireHook>>,
        opens: AtomicU32,
    }

    impl openlaser_driver::DeviceProvider for FakeProvider {
        fn name(&self) -> &'static str {
            "sim"
        }
        fn detect(&self) -> Result<Vec<openlaser_driver::DetectedDevice>, openlaser_driver::DriverError> {
            Ok(Vec::new())
        }
        fn open_lmc(&self, _m: &openlaser_machine::GalvoMachine) -> Result<Box<dyn openlaser_lmc::LmcTransport>, openlaser_driver::DriverError> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            let cfg =
                SimConfig { buffer_depth: 3, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid: 0x9899, faults: Vec::new(), emission_cap_pct: None };
            let fake = FakeLmc::new(cfg, self.clock.clone());
            if let Some(h) = self.hook.lock().unwrap().take() {
                fake.set_hook(h);
            }
            *self.recorder.lock().unwrap() = Some(fake.recorder());
            Ok(Box::new(fake))
        }
        fn open_link(&self, _m: &openlaser_machine::GantryMachine) -> Result<Box<dyn openlaser_driver::ByteLink>, openlaser_driver::DriverError> {
            Err(openlaser_driver::DriverError::NotConnected)
        }
        fn emission_cap_pct(&self) -> Result<Option<f64>, openlaser_driver::DriverError> {
            Ok(None)
        }
        fn record_emission(&self) -> Result<(), openlaser_driver::DriverError> {
            Ok(())
        }
    }

    struct Bench {
        s: Session,
        provider: Arc<FakeProvider>,
        machine: openlaser_engine::MachineId,
    }

    fn bench(window: bool, hook: Option<WireHook>) -> Bench {
        let clock = Arc::new(VirtualClock::new());
        let provider = Arc::new(FakeProvider { clock: clock.clone(), recorder: Mutex::new(None), hook: Mutex::new(hook), opens: AtomicU32::new(0) });
        let dyn_p: Arc<dyn openlaser_driver::DeviceProvider> = provider.clone();
        let mut s = Session::new(config(Origin::Human, window, DeviceAccess::Provider(dyn_p), clock));
        with_doc(&mut s);
        let machine = s.active().unwrap().machine.clone();
        Bench { s, provider, machine }
    }

    /// Polls until `done` holds for an event or the status (≤ 60 s of real time; the driver runs on virtual time).
    fn wait_for(b: &mut Bench, what: &str, mut done: impl FnMut(&Session, &[Event]) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut seen = Vec::new();
        loop {
            let evs = b.s.poll();
            seen.extend(evs);
            if done(&b.s, &seen) {
                return seen;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}; events: {seen:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn connect(b: &mut Bench) {
        b.s.execute("device.connect", json!({})).unwrap();
        let m = b.machine.clone();
        wait_for(b, "connected", |s, _| s.status(&m).is_some_and(|st| st.connected && matches!(st.state, MachineState::Idle)));
    }

    fn frame(b: &mut Bench, style: Option<&str>, source: Option<&str>) {
        let mut p = serde_json::Map::new();
        if let Some(st) = style {
            p.insert("style".into(), json!(st));
        }
        if let Some(src) = source {
            p.insert("source".into(), json!(src));
        }
        b.s.execute("laser.frame", Value::Object(p)).unwrap();
        let m = b.machine.clone();
        wait_for(b, "a frame receipt", |s, evs| {
            evs.iter().any(|e| matches!(e, Event::Driver { event: DriverEvent::FrameCompleted(_), .. })) && s.status(&m).is_some_and(|st| !st.busy())
        });
    }

    /// The token a human ARM produces: `mint` from the pending summary and receipt, ticks and typed "ARM".
    fn token(b: &Bench) -> openlaser_types::ArmToken {
        let p = b.s.pending_arm().unwrap();
        let now = b.provider.clock.now();
        let confirm = openlaser_arm::testing::confirm_for_tests("ARM", vec![true; p.checklist.len()], now);
        openlaser_arm::mint(&p.summary, &p.receipt, confirm, p.checklist.len(), b.s.id(), now).unwrap()
    }

    fn import_business_card(b: &mut Bench) {
        let ws = openlaser_testkit::golden_dir().join("inputs").join("workspace.json");
        let v = b.s.execute("file.import", json!({ "path": ws.to_string_lossy() })).unwrap();
        assert_eq!(v["objects"].as_array().unwrap().len(), 403);
    }

    fn small_square(b: &mut Bench) {
        b.s.execute("tools.drawLines", json!({ "pointsMm": [[-5, -5], [5, -5], [5, 5], [-5, 5]], "closed": true })).unwrap();
    }

    #[test]
    fn e2e_mopa_business_card_sim() {
        let stop_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let registry_slot: Arc<Mutex<Option<Arc<openlaser_engine::StopRegistry>>>> = Arc::new(Mutex::new(None));
        let (flag, reg) = (stop_now.clone(), registry_slot.clone());
        let hook: WireHook = Box::new(move |w: &SimWire, _at| {
            // Second run only: trip the stop on its first packet write (deterministic, on the driver thread).
            if matches!(w, SimWire::Packet { .. })
                && flag.swap(false, Ordering::SeqCst)
                && let Some(r) = reg.lock().unwrap().as_ref()
            {
                for m in r.machines() {
                    r.stop_one(&m, Duration::ZERO);
                }
            }
        });
        let mut b = bench(true, Some(hook));
        *registry_slot.lock().unwrap() = Some(b.s.stop_registry());
        import_business_card(&mut b);
        let built = b.s.build_job(None, &JobOptions::default()).unwrap_or_else(|r| panic!("{r:?}"));
        let open = built.validated.notices().iter().filter(|n| n.code == openlaser_types::NoticeCode::OpenShapeUnhatched).count();
        assert_eq!(open, 67, "the business card's open raster shapes are numbered notices");
        connect(&mut b);
        frame(&mut b, None, None);
        let receipt = b.s.status(&b.machine).unwrap().receipt.clone().unwrap();
        assert_eq!(receipt.origin, openlaser_types::FrameOrigin::Human);
        assert!(receipt.full_passes >= 1);
        b.s.execute("laser.start", json!({})).unwrap();
        let tok = token(&b);
        let plan = Arc::clone(&built.plan);
        b.s.run_armed(tok).unwrap();
        let m = b.machine.clone();
        wait_for(&mut b, "the run to finish", |_, evs| {
            evs.iter().any(|e| matches!(e, Event::Driver { event: DriverEvent::Finished(openlaser_engine::Outcome::Finished { .. }), .. }))
        });
        // FakeLmc's independent decoder burned exactly the plan's segments (within one galvo unit).
        let rec = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot();
        let unit = 200.0 * 1.1834 / 65535.0;
        let expected: Vec<[[f64; 2]; 2]> =
            plan.sections.iter().flat_map(|sec| sec.paths.iter().flat_map(|p| p.0.windows(2).map(|w| [w[0], w[1]]).collect::<Vec<_>>())).collect();
        assert_eq!(rec.burned.len(), expected.len());
        for (got, want) in rec.burned.iter().zip(&expected) {
            for k in 0..2 {
                assert!((got.a[k] - want[0][k]).abs() <= unit && (got.b[k] - want[1][k]).abs() <= unit, "{got:?} vs {want:?}");
            }
        }
        assert_eq!(rec.writes_while_not_ready, 0);
        // A stop mid-run gives a confirmed StopReport.
        frame(&mut b, None, None);
        b.s.execute("laser.start", json!({})).unwrap();
        let tok = token(&b);
        stop_now.store(true, Ordering::SeqCst);
        b.s.run_armed(tok).unwrap();
        let evs = wait_for(&mut b, "the stop report", |_, evs| {
            evs.iter().any(|e| matches!(e, Event::Driver { event: DriverEvent::Stopped(_) | DriverEvent::Finished(openlaser_engine::Outcome::Stopped(_)), .. }))
        });
        let report = evs
            .iter()
            .find_map(|e| match e {
                Event::Driver { event: DriverEvent::Stopped(r), .. }
                | Event::Driver { event: DriverEvent::Finished(openlaser_engine::Outcome::Stopped(r)), .. } => Some(r.clone()),
                _ => None,
            })
            .unwrap();
        assert!(report.confirmed && report.mo_closed && report.ports_zero, "{report:?}");
        assert!(b.s.status(&m).unwrap().last_stop.is_some());
    }

    #[test]
    fn run_armed_requires_receipt() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        let e = b.s.execute("laser.start", json!({})).unwrap_err();
        assert!(matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::FrameRequired), "{e}");
        assert!(e.to_string().contains("Frame this job first"));
        assert!(b.s.pending_arm().is_none());
    }

    #[test]
    fn tool_layer_receipt_does_not_arm() {
        let mut b = bench(true, None);
        small_square(&mut b);
        b.s.execute("tools.rectangle", json!({ "xMm": -20, "yMm": -20, "wMm": 40, "hMm": 40, "layer": "T1" })).unwrap();
        connect(&mut b);
        frame(&mut b, None, Some("toolLayersOnly"));
        let e = b.s.execute("laser.start", json!({})).unwrap_err();
        assert!(matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::FrameDoesNotCoverJob), "{e}");
    }

    #[test]
    fn frame_with_bounds_style_satisfies_start_for_contour_job() {
        let mut b = bench(true, None);
        small_square(&mut b);
        assert_eq!(b.s.active().unwrap().project.job.frame_style, FrameStyle::Contour);
        connect(&mut b);
        frame(&mut b, Some("bounds"), Some("job"));
        let r = b.s.status(&b.machine).unwrap().receipt.clone().unwrap();
        assert_eq!((r.style, r.source), (FrameStyle::Bounds, FrameSource::Job));
        b.s.execute("laser.start", json!({})).unwrap();
        assert_eq!(b.s.pending_arm().unwrap().fingerprint, r.fingerprint, "S21: style is not in the fingerprint");
    }

    #[test]
    fn pending_arm_voided_by_edit_on_the_real_path() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        frame(&mut b, None, None);
        b.s.execute("laser.start", json!({})).unwrap();
        assert!(b.s.pending_arm().is_some());
        b.s.execute("arrange.nudge", json!({ "ids": [1], "dxMm": 1, "dyMm": 0 })).unwrap();
        assert!(b.s.pending_arm().is_none());
    }

    #[test]
    fn arm_token_invalid_after_edit_on_the_real_path() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        frame(&mut b, None, None);
        b.s.execute("laser.start", json!({})).unwrap();
        let old = token(&b);
        b.s.execute("laser.repeat", json!({ "count": 2 })).unwrap();
        assert_eq!(b.s.run_armed(old).unwrap_err().code(), "stale");
        // Framed and started again: the old token is for the old fingerprint and bytes.
        frame(&mut b, None, None);
        b.s.execute("laser.start", json!({})).unwrap();
        let fresh_fp = b.s.pending_arm().unwrap().fingerprint;
        let stale_token = openlaser_arm::testing::token_for_tests(
            openlaser_types::Fingerprint([9; 32]),
            b.s.pending_arm().unwrap().summary.bytes_hash,
            b.machine.clone(),
            b.s.id(),
            b.s.token_expiry(),
            1,
        );
        assert_ne!(fresh_fp, openlaser_types::Fingerprint([9; 32]));
        assert!(b.s.run_armed(stale_token).is_err());
        let rec = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot();
        assert!(rec.burned.is_empty(), "nothing was burned");
    }

    #[test]
    fn run_armed_sends_the_pending_job_object() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        frame(&mut b, None, None);
        b.s.execute("laser.start", json!({})).unwrap();
        let p = b.s.pending_arm().unwrap();
        let Encoded::Lmc { job, .. } = &p.job else { panic!("galvo job") };
        assert_eq!(job.bytes_hash(), p.summary.bytes_hash, "the summary is of these bytes");
        let expected: Vec<String> = job.lists.iter().flat_map(|l| l.packets.iter().map(|pk| openlaser_testkit::sha256_hex(pk.bytes()))).collect();
        let before = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot().wire.len();
        let tok = token(&b);
        b.s.run_armed(tok).unwrap();
        wait_for(&mut b, "the run to finish", |_, evs| evs.iter().any(|e| matches!(e, Event::Driver { event: DriverEvent::Finished(_), .. })));
        let rec = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot();
        let sent: Vec<String> = rec.wire[before..]
            .iter()
            .filter_map(|w| match w {
                SimWire::Packet { sha256, .. } => Some(sha256.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(sent, expected, "the driver sent exactly the packets whose summary the human saw");
    }

    #[test]
    fn job_submit_never_reaches_driver_run() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        let wire_before = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot().wire.len();
        let v = b.s.execute_as(Origin::Synthetic, "job.submit", json!({ "note": "card for the shop" })).unwrap();
        assert_eq!(v["state"], "awaiting_human");
        let fr = b.s.execute_as(Origin::Synthetic, "job.frameRequest", json!({ "mode": "once" })).unwrap();
        assert_eq!(fr["state"], "awaiting_human");
        for _ in 0..50 {
            b.s.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        let rec = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot();
        assert_eq!(rec.wire.len(), wire_before, "no request reached the wire");
        assert!(rec.burned.is_empty() && rec.lit.is_empty());
        assert!(b.s.pending_arm().is_none());
        // The agent can withdraw but never advance it.
        let id = v["submission"].as_u64().unwrap();
        assert_eq!(b.s.execute_as(Origin::Synthetic, "job.withdraw", json!({ "submission": id })).unwrap()["state"], "withdrawn");
    }

    #[test]
    fn edit_after_submit_marks_stale() {
        let mut b = bench(true, None);
        small_square(&mut b);
        let v = b.s.execute_as(Origin::Synthetic, "job.submit", json!({})).unwrap();
        let id = v["submission"].as_u64().unwrap();
        b.s.execute_as(Origin::Synthetic, "laser.repeat", json!({ "count": 4 })).unwrap();
        let st = b.s.execute_as(Origin::Synthetic, "job.status", json!({ "submission": id })).unwrap();
        assert_eq!(st["state"], "stale");
    }

    #[test]
    fn submission_flow_human_frames_then_arms() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        let v = b.s.execute_as(Origin::Synthetic, "job.submit", json!({})).unwrap();
        let id = openlaser_engine::RequestId(v["submission"].as_u64().unwrap());
        b.s.approve_request(Origin::Human, id).unwrap();
        let m = b.machine.clone();
        wait_for(&mut b, "awaiting_arm", |s, _| {
            s.requests().submission(id).is_some_and(|x| x.state == openlaser_engine::SubmissionState::AwaitingArm) && s.status(&m).is_some_and(|st| !st.busy())
        });
        b.s.approve_request(Origin::Human, id).unwrap();
        assert!(b.s.pending_arm().is_some(), "approving an awaiting_arm submission opens the human's own ARM dialog");
        let tok = token(&b);
        b.s.run_armed(tok).unwrap();
        wait_for(&mut b, "finished", |s, _| s.requests().submission(id).is_some_and(|x| x.state == openlaser_engine::SubmissionState::Finished));
    }

    #[test]
    fn stop_from_an_agent_stops_a_frame_loop() {
        let mut b = bench(true, None);
        small_square(&mut b);
        connect(&mut b);
        b.s.execute("laser.frame", json!({ "loop": true })).unwrap();
        let m = b.machine.clone();
        wait_for(&mut b, "a loop receipt", |_, evs| evs.iter().any(|e| matches!(e, Event::Driver { event: DriverEvent::FrameCompleted(_), .. })));
        let v = b.s.execute_as(Origin::Synthetic, "laser.stop", json!({})).unwrap();
        assert_eq!(v["latched"], true);
        wait_for(&mut b, "idle after the stop", |s, _| s.status(&m).is_some_and(|st| !st.busy()));
        let rec = b.provider.recorder.lock().unwrap().as_ref().unwrap().snapshot();
        assert!(rec.burned.is_empty(), "framing never burns");
        let _ = FrameMode::Loop;
    }
}

// ---- gantry coordinates (plan → safety → G-code) -------------------------------------------------------------------

mod gantry {
    use openlaser_engine::{Encoded, JobOptions};
    use openlaser_types::Bounds;

    use super::*;

    /// The (x, y) of every G1 line of a program.
    fn g1_points(text: &str) -> Vec<[f64; 2]> {
        let mut at = [f64::NAN, f64::NAN];
        let mut out = Vec::new();
        for line in text.lines().filter(|l| l.starts_with("G1") || l.starts_with("G0")) {
            for w in line.split_whitespace() {
                if let Some(v) = w.strip_prefix('X').and_then(|v| v.parse().ok()) {
                    at[0] = v;
                }
                if let Some(v) = w.strip_prefix('Y').and_then(|v| v.parse().ok()) {
                    at[1] = v;
                }
            }
            if line.starts_with("G1") {
                out.push(at);
            }
        }
        out
    }

    /// I1: `Section.paths` of a gantry plan are centre-origin bed mm (the convention `openlaser_gcode` maps to the
    /// controller). WP3 had planned controller coordinates, so the encoder moved them by half the bed again: a 10 mm
    /// square at the centre of a 600 × 400 mm bottom-left bed was refused as off the bed.
    #[test]
    fn gantry_job_lands_where_it_was_designed() {
        let mut machines = vec![jpt_m7_200_sim()];
        machines.extend(openlaser_machine::om_gantries());
        let g = machines.iter().find(|m| m.id.0 == "co2-grbl-600x400").cloned().expect("OM's CO2 gantry profile");
        let openlaser_machine::Kinematics::Gantry(gm) = &g.kind else { panic!("a gantry") };
        assert_eq!((gm.bed_mm, gm.origin), ([600.0, 400.0], openlaser_doc::WorkspaceOrigin::BottomLeft));
        for (design, square, want) in [
            // A centre-origin design: its (0, 0) is the bed's centre, (300, 200) on the controller.
            (openlaser_doc::WorkspaceOrigin::Center, [[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]], [[295.0, 195.0], [305.0, 205.0]]),
            // A design whose origin is the controller's: design mm are G-code mm.
            (openlaser_doc::WorkspaceOrigin::BottomLeft, [[50.0, 50.0], [60.0, 50.0], [60.0, 60.0], [50.0, 60.0]], [[50.0, 50.0], [60.0, 60.0]]),
        ] {
            let cfg = SessionConfig {
                origin: Origin::Human,
                window: true,
                devices: DeviceAccess::None,
                machines: machines.clone(),
                profiles: None,
                files: FsAuthority::TrustedLocal,
                clock: Arc::new(VirtualClock::new()),
            };
            let mut s = Session::new(cfg);
            let ws = openlaser_doc::Workspace { kind: openlaser_types::DeviceKind::Grbl, size_mm: [600.0, 400.0], origin: design };
            s.open_project(Project::new(ws, &g.layer_defaults), None, Some(g.id.clone())).unwrap();
            s.execute("tools.drawLines", json!({ "pointsMm": square, "closed": true })).unwrap();
            let built = s
                .build_job(None, &JobOptions { machine: Some(g.id.clone()), selected_only: None })
                .unwrap_or_else(|r| panic!("{design:?}: {:?} {:?}", r.refusals, r.error));
            let Encoded::Gcode { job, .. } = &built.encoded else { panic!("a G-code job") };
            let points = g1_points(&job.text);
            assert!(!points.is_empty(), "{design:?}: the program burns");
            for p in &points {
                assert!(
                    p[0] >= want[0][0] - 1e-6 && p[0] <= want[1][0] + 1e-6 && p[1] >= want[0][1] - 1e-6 && p[1] <= want[1][1] + 1e-6,
                    "{design:?}: G1 at {p:?}, want inside {want:?}"
                );
            }
            // The confirmation and the decoded burn share the plan's frame: centre-origin bed mm.
            assert_eq!(built.summary.work_area, Bounds { min: [-300.0, -200.0], max: [300.0, 200.0] });
            let row = &built.summary.rows[0].bounds;
            for seg in &built.burned {
                for q in [seg.a, seg.b] {
                    assert!(
                        q[0] >= row.min[0] - 1e-6 && q[0] <= row.max[0] + 1e-6 && q[1] >= row.min[1] - 1e-6 && q[1] <= row.max[1] + 1e-6,
                        "{design:?}: burn {q:?} outside the row {row:?}"
                    );
                }
            }
        }
    }
}

// ---- panic hunt (run at I2) ------------------------------------------------------------------------------------------

#[test]
#[ignore = "panic hunt: every command × adversarial params; run at I2 (`cargo test -p openlaser-engine -- --ignored panic_hunt`)"]
fn panic_hunt() {
    let params = openlaser_testkit::adversarial::params();
    let ids: Vec<&'static str> = registry().specs().iter().map(|s| s.id).collect();
    let handle = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut internal = Vec::new();
            for origin in [Origin::Human, Origin::Synthetic, Origin::Headless] {
                let provider: Arc<dyn openlaser_driver::DeviceProvider> = Arc::new(CountingProvider::default());
                let devices = if origin == Origin::Headless { DeviceAccess::None } else { DeviceAccess::Provider(provider) };
                let mut s = Session::new(SessionConfig {
                    origin,
                    window: origin != Origin::Headless,
                    devices,
                    machines: vec![jpt_m7_200_sim(), jpt_m7_200()],
                    profiles: None,
                    files: FsAuthority::Denied,
                    clock: Arc::new(VirtualClock::new()),
                });
                with_doc(&mut s);
                s.execute("tools.drawLines", json!({ "pointsMm": [[0, 0], [5, 5], [5, 0]] })).ok();
                s.execute("edit.selectAll", json!({})).ok();
                for id in &ids {
                    // Device-opening commands only from the human session; quitting would end nothing here.
                    for p in &params {
                        let out = s.execute(id, p.clone());
                        if let Err(EngineError::Internal(x)) = &out {
                            internal.push(format!("{origin:?} {x} {p}"));
                        }
                        if s.active().is_none() {
                            with_doc(&mut s);
                        }
                    }
                }
            }
            internal
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    while !handle.is_finished() {
        assert!(std::time::Instant::now() < deadline, "panic hunt hung");
        std::thread::sleep(Duration::from_millis(50));
    }
    let internal = handle.join().unwrap();
    assert!(internal.is_empty(), "contained panics: {internal:#?}");
}

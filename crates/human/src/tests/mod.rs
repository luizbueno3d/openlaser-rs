//! Kittest suite for the provenance gate and the ARM dialog (SPEC §6.4: harness input counts as human; synthetic
//! input goes through `Human::inject`; AccessKit actions arrive as `Event::AccessKitActionRequest`).
//!
//! Most tests install a recording mint through `Human::test_mint` (compiled only under `cfg(test)`) to see exactly
//! what the dialog passes; the test at the end runs WP3's real `openlaser_arm::mint`.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use openlaser_arm::HumanConfirm;
use openlaser_arm::testing::token_for_tests;
use openlaser_types::{
    ARM_TOKEN_TTL, ArmToken, Bounds, EmissionSummary, Fingerprint, FrameOrigin, FrameReceipt, FrameSource, FrameStyle, GalvoWords, LayerKey, MachineId,
    MonoTime, Notice, NoticeCode, PassRow, Refusal, RefusalCode, SectionMode, SessionId, VirtualClock,
};
use openlaser_ui_egui::{ArmModel, ArmOutcome, DisplayList, HumanSurfaces};

use crate::{AUTOMATION_IGNORED, ApprovalAction, ApprovalCard, CANCEL_LABEL, CONFIRM_LABEL, Human, MintFn};

const CHECKLIST: [&str; 2] = ["Key switch and E-stop within reach", "Laser safety glasses on"];
const SESSION: SessionId = SessionId(3);

thread_local! {
    static MINTS: Cell<u32> = const { Cell::new(0) };
}

fn mints() -> u32 {
    MINTS.with(Cell::get)
}

/// Records the call and hands out a test token (the real checks are WP3's, exercised by the last test).
fn recording_mint(s: &EmissionSummary, r: &FrameReceipt, c: HumanConfirm, n: usize, session: SessionId, now: MonoTime) -> Result<ArmToken, Refusal> {
    drop(c);
    let _ = (r, n);
    MINTS.with(|m| m.set(m.get() + 1));
    Ok(token_for_tests(s.fingerprint, s.bytes_hash, s.machine.clone(), session, now.plus(ARM_TOKEN_TTL), 11))
}

fn refusing_mint(_: &EmissionSummary, _: &FrameReceipt, c: HumanConfirm, _: usize, _: SessionId, _: MonoTime) -> Result<ArmToken, Refusal> {
    drop(c);
    MINTS.with(|m| m.set(m.get() + 1));
    Err(Refusal::new(RefusalCode::FrameReceiptExpired, "the frame receipt is older than 10 minutes; frame the job again"))
}

/// One pass row as the encoder's summary reports it for C00 at 30 %, 30 kHz, 200 ns, 1000 mm/s on F = 236.68 mm:
/// mark current 1228 → 29.99 %, period 667 → 29.99 kHz, 276 speed units → 996.8 mm/s.
fn row() -> PassRow {
    PassRow {
        layer: LayerKey::Color(0),
        layer_name: "C00".into(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        power_pct: 1228.0 * 100.0 / 4095.0,
        frequency_khz: Some(20_000.0 / 667.0),
        pulse_ns: Some(200),
        speed_mm_s: 276.0 * 236.68 * 1000.0 / 65_535.0,
        galvo: Some(GalvoWords { mark_current: 1228, q_period: 667, pulse_ns: 200, speed_units: 276, jump_speed_units: 830 }),
        paths: 3,
        mark_mm: 120.0,
        bounds: Bounds { min: [-20.0, -10.0], max: [20.0, 10.0] },
        estimate: Duration::from_millis(3_100),
    }
}

fn summary(fp: u8) -> EmissionSummary {
    EmissionSummary {
        rotary: None,
        fingerprint: Fingerprint([fp; 32]),
        bytes_hash: Fingerprint([fp.wrapping_add(100); 32]),
        machine: MachineId("jpt-m7-200-sim".into()),
        rows: vec![row()],
        notices: vec![Notice { n: 1, code: NoticeCode::OpenShapeUnhatched, message: "an open shape on C02 is not hatched".into(), object: None, layer: None }],
        repeats: 1,
        estimate: Duration::from_secs(41),
        crash_bound: Duration::from_secs(2),
        packets: 4,
        work_area: Bounds { min: [-100.0, -100.0], max: [100.0, 100.0] },
        negate: [false, false],
        swap_xy: false,
    }
}

fn receipt(s: &EmissionSummary, at: MonoTime) -> FrameReceipt {
    FrameReceipt {
        fingerprint: s.fingerprint,
        machine: s.machine.clone(),
        session: SESSION,
        full_passes: 2,
        at,
        style: FrameStyle::Bounds,
        source: FrameSource::Job,
        origin: FrameOrigin::Human,
    }
}

fn model(fp: u8, cap: Option<f64>) -> ArmModel {
    let s = summary(fp);
    let r = receipt(&s, MonoTime::default());
    ArmModel {
        summary: s,
        receipt: r,
        checklist: CHECKLIST.iter().map(|s| (*s).to_owned()).collect(),
        session: SESSION,
        now: MonoTime::default(),
        thumbnail: DisplayList::default(),
        emission_cap_pct: cap,
    }
}

struct Rig {
    human: Human,
    model: ArmModel,
    show: bool,
    armed: Vec<ArmToken>,
    cancelled: u32,
}

fn harness_with(mint: Option<MintFn>, m: ArmModel) -> (Harness<'static, Rig>, Arc<VirtualClock>) {
    MINTS.with(|c| c.set(0));
    let clock = Arc::new(VirtualClock::new());
    let mut human = Human::new(clock.clone());
    human.test_mint = mint;
    let rig = Rig { human, model: m, show: true, armed: Vec::new(), cancelled: 0 };
    let h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, rig: &mut Rig| {
            if !rig.show {
                return;
            }
            // As the app does: Armed and Cancelled end this PendingArm's dialog.
            match rig.human.arm_dialog(ui.ctx(), &rig.model) {
                ArmOutcome::Armed(t) => {
                    rig.armed.push(t);
                    rig.show = false;
                }
                ArmOutcome::Cancelled => {
                    rig.cancelled += 1;
                    rig.show = false;
                }
                ArmOutcome::Open => {}
            }
        },
        rig,
    );
    (h, clock)
}

fn harness() -> (Harness<'static, Rig>, Arc<VirtualClock>) {
    harness_with(Some(recording_mint), model(7, None))
}

/// Types ARM and ticks every checklist item with harness (human) input.
fn human_fills(h: &mut Harness<'_, Rig>) {
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("ARM");
    h.run();
    for item in CHECKLIST {
        h.get_by_label(item).click();
        h.run();
    }
}

fn field_value(h: &Harness<'_, Rig>) -> String {
    h.get_by_role(Role::TextInput).value().unwrap_or_default()
}

fn ticks(h: &Harness<'_, Rig>) -> Vec<bool> {
    h.state().human.arm.ticked_for_tests()
}

fn click_confirm(h: &mut Harness<'_, Rig>) {
    h.get_by_label(CONFIRM_LABEL).click();
    h.run();
}

/// Hands `events` to `Human::inject` the way ui-egui's raw_input_hook does, then runs the frame.
fn inject(h: &mut Harness<'_, Rig>, events: Vec<egui::Event>) {
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    h.state_mut().human.inject(&ctx, &mut raw, events);
    *h.input_mut() = raw;
    h.step();
}

fn synthetic_click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    vec![egui::Event::PointerMoved(pos), button(true), button(false)]
}

fn shows(h: &Harness<'_, Rig>, text: &str) -> bool {
    h.query_by_label(text).is_some()
}

// ---- the acceptance list (SPEC §7.10) -------------------------------------------------------------------------

#[test]
fn human_click_after_300ms_mints_token() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    assert_eq!(field_value(&h), "ARM");
    assert_eq!(ticks(&h), vec![true, true]);
    clock.advance(Duration::from_millis(300));
    click_confirm(&mut h);
    assert_eq!(mints(), 1);
    assert_eq!(h.state().armed.len(), 1, "Armed(token) is returned once");
    assert!(!h.state().human.surface_open(), "the dialog closes once armed");
    // Drawn again in the same session (ARM once per session), it opens filled in with what the person confirmed,
    // but nothing arms without a new real click: still one token.
    h.state_mut().show = true;
    h.run();
    assert_eq!(h.state().armed.len(), 1);
    assert_eq!(field_value(&h), "ARM");
    assert_eq!(ticks(&h), vec![true, true]);
}

#[test]
fn instant_click_before_300ms_ignored() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_millis(299));
    click_confirm(&mut h);
    assert_eq!(mints(), 0, "visible for 299 ms only");
    assert!(h.state().armed.is_empty());
    clock.advance(Duration::from_millis(1));
    click_confirm(&mut h);
    assert_eq!(mints(), 1, "the same click at 300 ms counts");
}

#[test]
fn unfocused_window_click_ignored() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    h.input_mut().focused = false;
    click_confirm(&mut h);
    assert_eq!(mints(), 0);
    h.input_mut().focused = true;
    click_confirm(&mut h);
    assert_eq!(mints(), 1);
}

#[test]
fn synthetic_click_on_arm_confirm_mints_nothing() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    let pos = h.get_by_label(CONFIRM_LABEL).rect().center();
    inject(&mut h, synthetic_click_at(pos));
    h.run();
    assert_eq!(mints(), 0);
    assert!(h.state().armed.is_empty());
    assert_eq!(h.state().human.dropped_events(), 3, "nothing injected reaches an open human-only surface");
    assert_eq!(field_value(&h), "", "a tainted frame clears the field");
    assert_eq!(ticks(&h), vec![false, false], "and every tick");
    assert!(shows(&h, AUTOMATION_IGNORED));
}

#[test]
fn synthetic_type_arm_clears_field_and_checklist() {
    let (mut h, _clock) = harness();
    for item in CHECKLIST {
        h.get_by_label(item).click();
        h.run();
    }
    assert_eq!(ticks(&h), vec![true, true]);
    inject(&mut h, vec![egui::Event::Text("ARM".into())]);
    h.run();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
    assert!(shows(&h, AUTOMATION_IGNORED));
    assert_eq!(mints(), 0);
}

#[test]
fn accesskit_click_on_arm_confirm_mints_nothing() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    h.get_by_label(CONFIRM_LABEL).click_accesskit();
    h.run();
    assert_eq!(mints(), 0);
    assert!(h.state().armed.is_empty());
    assert!(shows(&h, AUTOMATION_IGNORED), "an AccessKit action is treated as automation input");
    assert_eq!(field_value(&h), "");
}

#[test]
fn accesskit_ticks_are_undone_in_their_own_pass() {
    // AccessKit Click toggles egui checkboxes in the pass that carries it. That pass is tainted (an AccessKit action
    // request), but its taint is not recorded in the 3-pass/1 s window, so whatever it ticked must be undone after the
    // dialog was drawn: otherwise a person who only types ARM and clicks Confirm would arm items nobody ticked.
    let (mut h, clock) = harness();
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("ARM");
    h.run();
    for item in CHECKLIST {
        h.get_by_label(item).click_accesskit();
    }
    h.step();
    assert_eq!(ticks(&h), vec![false, false], "nothing ticked by AccessKit survives its pass");
    h.run();
    assert_eq!(ticks(&h), vec![false, false]);
    assert!(shows(&h, AUTOMATION_IGNORED));
    // The person types ARM again and clicks Confirm, never ticking anything: nothing arms.
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("ARM");
    h.run();
    assert_eq!(field_value(&h), "ARM");
    clock.advance(Duration::from_secs(1));
    click_confirm(&mut h);
    assert_eq!(mints(), 0);
    assert!(h.state().armed.is_empty());
}

#[test]
fn arm_dialog_shows_document_text_as_plain_text() {
    // Layer names, notices, checklist items and the machine id are shown without control, bidi or zero-width
    // characters (egui does not interpret ANSI, but newlines and bidi overrides still mislead).
    let mut m = model(7, None);
    if let Some(r) = m.summary.rows.first_mut() {
        r.layer_name = "C00 5.00 %\u{1b}[8m\nfake\u{202E}".into();
    }
    if let Some(n) = m.summary.notices.first_mut() {
        n.message = "an open shape\non C02\u{200B} is not hatched\u{2066}".into();
    }
    m.checklist = vec!["Key switch\nand E-stop\u{202E} within reach".into(), "Laser safety glasses on".into()];
    let (mut h, _clock) = harness_with(Some(recording_mint), m);
    h.run();
    for shown in ["C00 · C00 5.00 %[8mfake", "Notice 1: an open shapeon C02 is not hatched", "Key switchand E-stop within reach"] {
        assert!(shows(&h, shown), "the dialog shows {shown:?}");
    }
}

#[test]
fn kittest_confirm_shows_decoded_layer_params() {
    // B4: the prototype's confirmation showed the Frame/requested values. The layer here was requested at 30 %,
    // 30 kHz, 1000 mm/s and then changed to 80 % / 60 kHz after the plan; the dialog renders only what the bytes
    // decode to.
    let (mut h, _clock) = harness();
    let requested_after_edit = ["80.00 %", "60.00 kHz", "30.00 %", "30.00 kHz", "1000.0 mm/s"];
    h.run();
    for decoded in ["C00", "Line", "1/1", "29.99 %", "29.99 kHz", "200 ns", "996.8 mm/s", "3", "(-20.00, -10.00) to (20.00, 10.00) mm", "3.1 s"] {
        assert!(shows(&h, decoded), "the dialog shows the decoded {decoded}");
    }
    for stale in requested_after_edit {
        assert!(!shows(&h, stale), "the dialog never shows {stale}");
    }
    for line in [
        "Repeats: 1",
        "Notice 1: an open shape on C02 is not hatched",
        "Total time: 41.0 s",
        "Emission still possible after a host crash: up to 2.0 s",
        "Machine: jpt-m7-200-sim",
    ] {
        assert!(shows(&h, line), "the dialog shows {line}");
    }
    assert!(shows(&h, &format!("Job {}", Fingerprint([7; 32]).short())));
}

#[test]
fn arm_dialog_text_does_not_persist() {
    let (mut h, _clock) = harness();
    human_fills(&mut h);
    h.get_by_label(CANCEL_LABEL).click();
    h.run();
    assert_eq!(h.state().cancelled, 1);
    assert!(!h.state().human.surface_open());
    // Opened again for the same PendingArm: empty.
    h.state_mut().show = true;
    h.run();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
    // A frame without the dialog closes it too.
    human_fills(&mut h);
    h.state_mut().show = false;
    h.run();
    h.state_mut().show = true;
    h.run();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
}

#[test]
fn arm_dialog_closed_by_revision_bump() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    // A revision bump voids PendingArm: the app stops drawing it, and the next Start builds a new model.
    h.state_mut().show = false;
    h.run();
    assert!(!h.state().human.frame_is_synthetic(&h.ctx));
    assert!(!h.state().human.surface_open(), "not drawn → closed");
    h.state_mut().model = model(8, None);
    h.state_mut().show = true;
    h.run();
    assert_eq!(field_value(&h), "", "nothing typed carries over to the new confirmation");
    assert_eq!(ticks(&h), vec![false, false]);
    // Even when the app swaps the model without a gap, the typed text and ticks are forgotten.
    human_fills(&mut h);
    h.state_mut().model = model(9, None);
    h.run();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
    click_confirm(&mut h);
    assert_eq!(mints(), 0, "Confirm is disabled until ARM is typed and ticked again");
}

#[test]
fn arm_dialog_forgets_input_when_what_is_confirmed_changes() {
    // Same job bytes, but a different checklist (same length), frame receipt or bench cap is a different PendingArm:
    // what was ticked and typed for one never confirms the other.
    let changes: [fn(&mut ArmModel); 3] =
        [|m| m.checklist = vec!["Shield closed".into(), "Fume extraction on".into()], |m| m.receipt.full_passes += 1, |m| m.emission_cap_pct = Some(10.0)];
    for change in changes {
        let (mut h, clock) = harness();
        human_fills(&mut h);
        assert_eq!(ticks(&h), vec![true, true]);
        change(&mut h.state_mut().model);
        h.run();
        assert_eq!(field_value(&h), "");
        assert_eq!(ticks(&h), vec![false, false]);
        clock.advance(Duration::from_secs(1));
        click_confirm(&mut h);
        assert_eq!(mints(), 0);
    }
}

#[test]
fn close_arm_dialog_forgets_input_even_for_an_identical_model() {
    // The app voids the PendingArm (an undo then redo bumps the revision twice and rebuilds the same job) and calls
    // close_arm_dialog; the dialog drawn again with an identical model is a new opening.
    let (mut h, clock) = harness();
    human_fills(&mut h);
    h.state_mut().human.close_arm_dialog();
    h.run();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
    clock.advance(Duration::from_secs(1));
    click_confirm(&mut h);
    assert_eq!(mints(), 0);
}

#[test]
fn one_undrawn_pass_closes_the_arm_dialog() {
    // Exactly one pass without the dialog (a stop voided the PendingArm), then drawn again: a new opening.
    let (mut h, _clock) = harness();
    human_fills(&mut h);
    h.state_mut().show = false;
    h.step();
    h.state_mut().show = true;
    h.step();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
}

#[test]
fn arm_dialog_closed_by_stop() {
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    assert!(h.state().human.surface_open());
    // Esc / STOP: the app calls stop_all, PendingArm is voided and the dialog is not drawn in that frame, so a
    // Confirm click in the same frame has nothing to land on.
    let pos = h.get_by_label(CONFIRM_LABEL).rect().center();
    h.state_mut().show = false;
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run();
    assert_eq!(mints(), 0);
    // The app's logic() asks frame_is_synthetic every frame (shortcuts); that is enough to notice the close.
    assert!(!h.state().human.frame_is_synthetic(&h.ctx));
    assert!(!h.state().human.surface_open());
    h.state_mut().show = true;
    h.run();
    assert_eq!(field_value(&h), "");
    assert_eq!(ticks(&h), vec![false, false]);
}

// ---- beyond the list ---------------------------------------------------------------------------------------------

#[test]
fn taint_window_blocks_a_click_that_bypassed_inject() {
    // Mechanism 3 in depth: even if a click reached the dialog without going through inject (an integration bug),
    // the taint left by any injected event within 1 s makes it worthless.
    let (mut h, clock) = harness();
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    inject(&mut h, vec![egui::Event::PointerMoved(egui::pos2(1.0, 1.0))]);
    human_fills(&mut h);
    clock.advance(Duration::from_millis(500));
    click_confirm(&mut h);
    assert_eq!(mints(), 0);
}

#[test]
fn arm_dialog_refuses_rows_above_bench_cap() {
    let (mut h, clock) = harness_with(Some(recording_mint), model(7, Some(10.0)));
    h.run();
    assert!(shows(&h, "First emission of this bench session: power is capped at 10 %."));
    assert!(h.query_by_label_contains("is above the 10 % cap").is_some());
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    click_confirm(&mut h);
    assert_eq!(mints(), 0, "rows above the cap are refused before minting");
}

#[test]
fn empty_checklist_can_never_arm() {
    let mut m = model(7, None);
    m.checklist.clear();
    let (mut h, clock) = harness_with(Some(recording_mint), m);
    assert!(shows(&h, crate::arm_dialog::NO_CHECKLIST));
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("ARM");
    h.run();
    clock.advance(Duration::from_secs(1));
    click_confirm(&mut h);
    assert_eq!(mints(), 0);
}

#[test]
fn mint_refusal_is_shown_and_nothing_arms() {
    let (mut h, clock) = harness_with(Some(refusing_mint), model(7, None));
    human_fills(&mut h);
    clock.advance(Duration::from_secs(1));
    click_confirm(&mut h);
    assert_eq!(mints(), 1);
    assert!(h.state().armed.is_empty());
    assert!(h.query_by_label_contains("frame_receipt_expired").is_some());
    assert!(h.state().human.surface_open(), "the dialog stays open after a refusal");
}

#[test]
fn approval_card_needs_a_human_click() {
    MINTS.with(|c| c.set(0));
    let clock = Arc::new(VirtualClock::new());
    let s = summary(5);
    struct CardRig {
        human: Human,
        summary: EmissionSummary,
        actions: Vec<ApprovalAction>,
    }
    let rig = CardRig { human: Human::new(clock.clone()), summary: s, actions: Vec::new() };
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, rig: &mut CardRig| {
            let card = ApprovalCard {
                id: 1,
                title: "Job from an agent",
                client: "claude-code\u{202E}\n\u{1b}[8m",
                fingerprint: &rig.summary.fingerprint,
                summary: Some(&rig.summary),
                note: "Business card, C00 vector engrave.\u{7}",
                approve_label: "Frame",
                decline_label: "Reject",
            };
            let a = rig.human.approval_card(ui, &card);
            if a != ApprovalAction::Pending {
                rig.actions.push(a);
            }
        },
        rig,
    );
    assert!(h.query_by_label("Note from agent").is_some());
    let from = format!("From claude-code[8m · job {}", Fingerprint([5; 32]).short());
    assert!(h.query_by_label(&from).is_some(), "the client is shown as plain text");
    assert!(h.query_by_label("Business card, C00 vector engrave.").is_some(), "control characters are dropped");
    assert!(h.state().human.surface_open());
    // Synthetic click: dropped while the card is open, and tainted anyway.
    let pos = h.get_by_label("Frame").rect().center();
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    h.state_mut().human.inject(&ctx, &mut raw, synthetic_click_at(pos));
    *h.input_mut() = raw;
    h.run();
    assert!(h.state().actions.is_empty());
    // Human click before 300 ms (the taint is also still live): ignored. After: counts.
    h.get_by_label("Frame").click();
    h.run();
    assert!(h.state().actions.is_empty());
    clock.advance(Duration::from_secs(2));
    h.get_by_label("Frame").click();
    h.run();
    assert_eq!(h.state().actions, vec![ApprovalAction::Approve]);
}

// ---- the gate on an ordinary human-only button (Connect, Frame, Start, Save, Allow agent connections) ------------

struct ButtonRig {
    human: Human,
    clicks: u32,
    human_clicks: u32,
}

fn button_harness() -> (Harness<'static, ButtonRig>, Arc<VirtualClock>) {
    let clock = Arc::new(VirtualClock::new());
    let rig = ButtonRig { human: Human::new(clock.clone()), clicks: 0, human_clicks: 0 };
    let h = Harness::builder().with_size(egui::vec2(400.0, 200.0)).build_ui_state(
        |ui, rig: &mut ButtonRig| {
            let r = ui.button("Frame");
            if r.clicked() {
                rig.clicks += 1;
            }
            if rig.human.is_human_click(ui.ctx(), &r) {
                rig.human_clicks += 1;
            }
        },
        rig,
    );
    (h, clock)
}

fn inject_into(h: &mut Harness<'_, ButtonRig>, events: Vec<egui::Event>) {
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    h.state_mut().human.inject(&ctx, &mut raw, events);
    *h.input_mut() = raw;
    h.step();
}

#[test]
fn synthetic_click_on_human_only_button_refused() {
    let (mut h, clock) = button_harness();
    clock.advance(Duration::from_secs(1));
    let pos = h.get_by_label("Frame").rect().center();
    inject_into(&mut h, synthetic_click_at(pos));
    h.run();
    assert!(h.state().clicks >= 1, "no surface is open, so the synthetic click reaches the button");
    assert_eq!(h.state().human_clicks, 0, "but it is not a human click");
    assert_eq!(h.state().human.dropped_events(), 0);
    // While tainted (< 1 s since the injection) a harness click does not count either.
    h.get_by_label("Frame").click();
    h.run();
    assert_eq!(h.state().human_clicks, 0);
    clock.advance(Duration::from_secs(1));
    h.get_by_label("Frame").click();
    h.run();
    assert_eq!(h.state().human_clicks, 1);
}

#[test]
fn press_synthetic_release_human_ignored() {
    let (mut h, clock) = button_harness();
    // Remove egui's own click-duration limit, as any code holding the Context could, so only the gate decides.
    h.ctx.options_mut(|o| o.input_options.max_click_duration = f64::INFINITY);
    clock.advance(Duration::from_secs(1));
    let pos = h.get_by_label("Frame").rect().center();
    inject_into(
        &mut h,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ],
    );
    // The taint window (3 passes, 1 s) runs out with the button still held.
    clock.advance(Duration::from_secs(2));
    h.run_steps(5);
    assert!(!h.state().human.frame_is_synthetic(&h.ctx), "the taint window has passed");
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run();
    assert_eq!(h.state().clicks, 1, "egui saw a click");
    assert_eq!(h.state().human_clicks, 0, "whose press was synthetic");
    // Control: a human press and release counts.
    h.get_by_label("Frame").click();
    h.run();
    assert_eq!(h.state().human_clicks, 1);
}

#[test]
fn release_in_tainted_frame_ignored() {
    // Human press, then a human release in a frame that also carries automation input: the frame is tainted.
    let (mut h, clock) = button_harness();
    h.ctx.options_mut(|o| o.input_options.max_click_duration = f64::INFINITY);
    clock.advance(Duration::from_secs(1));
    let pos = h.get_by_label("Frame").rect().center();
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.step();
    // The release and an injected event land in the same frame.
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    inject_into(&mut h, vec![egui::Event::PointerMoved(pos)]);
    assert_eq!(h.state().clicks, 1, "egui saw a click");
    assert_eq!(h.state().human_clicks, 0, "released in a tainted frame");
}

#[test]
fn keyboard_activation_is_not_a_human_click() {
    let (mut h, clock) = button_harness();
    clock.advance(Duration::from_secs(1));
    h.get_by_label("Frame").click();
    h.run();
    assert_eq!(h.state().human_clicks, 1);
    // Focused by keyboard, Enter and Space "click" it for egui, never for the gate.
    let before = h.state().clicks;
    h.key_press(egui::Key::Tab);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.key_press(egui::Key::Space);
    h.run();
    assert!(h.state().clicks > before, "egui activated the button from the keyboard");
    assert_eq!(h.state().human_clicks, 1);
}

#[test]
fn frame_is_synthetic_tracks_the_taint_window() {
    let (mut h, clock) = button_harness();
    assert!(!h.state().human.frame_is_synthetic(&h.ctx));
    inject_into(&mut h, vec![egui::Event::PointerMoved(egui::pos2(5.0, 5.0))]);
    assert!(h.state().human.frame_is_synthetic(&h.ctx));
    clock.advance(Duration::from_secs(1));
    h.step();
    assert!(h.state().human.frame_is_synthetic(&h.ctx), "pass + 2 is still tainted");
    h.step();
    assert!(!h.state().human.frame_is_synthetic(&h.ctx), "3 passes and 1 s later the taint is gone");
    assert_eq!(h.state().human.injected_events(), 1);
}

// ---- WP3's real mint -----------------------------------------------------------------------------------------------

#[test]
fn human_click_after_300ms_mints_token_with_real_mint() {
    let (mut h, clock) = harness_with(None, model(7, None));
    human_fills(&mut h);
    clock.advance(Duration::from_millis(300));
    click_confirm(&mut h);
    assert_eq!(h.state().armed.len(), 1);
    let token = h.state().armed.first().map(|t| (t.fingerprint().to_owned(), t.bytes().to_owned()));
    assert_eq!(token, Some((Fingerprint([7; 32]), Fingerprint([107; 32]))));
}

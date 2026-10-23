//! The taint record, visibility timers and open surfaces (SPEC §3.20, UI §7.3). All of it lives in `Human`, never in
//! egui context data: any code holding the `Context` can rewrite `ctx.data_mut(..)`. The one context marker written
//! here is a secondary signal for the free `gesture()` function; nothing relies on it alone.

use std::collections::HashMap;
use std::time::Duration;

use openlaser_types::MonoTime;

/// Passes tainted by one injection: the pass that carries it and the next two.
pub const TAINT_PASSES: u64 = 3;
/// Every pass until this long after the last injected event is tainted too.
pub const TAINT_HOLD: Duration = Duration::from_secs(1);

/// Visibility entries are pruned once the map grows past this.
const VISIBILITY_PRUNE_AT: usize = 512;

fn marker_id() -> egui::Id {
    egui::Id::new("openlaser-human::taint-through-pass")
}

/// Secondary marker: true while the context says the current pass is tainted.
pub(crate) fn context_marker_live(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(marker_id())).is_some_and(|through| pass <= through)
}

#[derive(Debug, Default)]
pub(crate) struct Taint {
    /// Last tainted pass (inclusive).
    through_pass: Option<u64>,
    /// Clock time of the last injected event.
    last_injected: Option<MonoTime>,
    /// The pointer press now in progress (or the last one) started with automation input.
    press_tainted: bool,
    injected: u64,
    dropped: u64,
}

impl Taint {
    /// Records one non-empty injection in pass `pass` at `now`.
    pub(crate) fn record(&mut self, ctx: &egui::Context, pass: u64, now: MonoTime, events: &[egui::Event]) {
        let through = pass.saturating_add(TAINT_PASSES - 1);
        let through = self.through_pass.map_or(through, |p| p.max(through));
        self.through_pass = Some(through);
        self.last_injected = Some(self.last_injected.map_or(now, |t| t.max(now)));
        if events.iter().any(starts_a_press) {
            self.press_tainted = true;
        }
        self.injected = self.injected.saturating_add(u64::try_from(events.len()).unwrap_or(u64::MAX));
        ctx.data_mut(|d| d.insert_temp(marker_id(), through));
    }

    pub(crate) fn note_dropped(&mut self, n: usize) {
        self.dropped = self.dropped.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    }

    /// The pass `pass` at `now` is tainted.
    pub(crate) fn live(&self, pass: u64, now: MonoTime) -> bool {
        self.through_pass.is_some_and(|p| pass <= p) || self.last_injected.is_some_and(|t| now.saturating_since(t) < TAINT_HOLD)
    }

    /// Press provenance, evaluated on every gate call: a press seen in a tainted pass (or injected) taints the whole
    /// press-release sequence; only a fresh press in an untainted pass clears it.
    pub(crate) fn observe_pointer(&mut self, tainted: bool, pressed: bool, down: bool) {
        if tainted {
            if pressed || down {
                self.press_tainted = true;
            }
        } else if pressed {
            self.press_tainted = false;
        }
    }

    pub(crate) fn press_tainted(&self) -> bool {
        self.press_tainted
    }

    pub(crate) fn injected(&self) -> u64 {
        self.injected
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

fn starts_a_press(e: &egui::Event) -> bool {
    matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Touch { phase: egui::TouchPhase::Start, .. })
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    since: MonoTime,
    last_pass: u64,
}

/// "Widget visible ≥ 300 ms": a widget counts as continuously visible while its gate is evaluated in consecutive
/// passes. A pass without it restarts the timer.
#[derive(Debug, Default)]
pub(crate) struct Visibility {
    seen: HashMap<egui::Id, Seen>,
}

impl Visibility {
    pub(crate) fn visible_since(&mut self, id: egui::Id, pass: u64, now: MonoTime) -> MonoTime {
        if self.seen.len() > VISIBILITY_PRUNE_AT {
            self.seen.retain(|_, s| s.last_pass.saturating_add(1) >= pass);
        }
        let entry = self.seen.entry(id).or_insert(Seen { since: now, last_pass: pass });
        if entry.last_pass.saturating_add(1) < pass {
            *entry = Seen { since: now, last_pass: pass };
        }
        entry.last_pass = entry.last_pass.max(pass);
        entry.since
    }
}

/// Human-only surfaces (the ARM dialog, approval cards, anything the app marks with `Human::surface_shown`). A
/// surface is open from the pass it is drawn until a pass passes without it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Surfaces {
    last_pass: Option<u64>,
    open: bool,
}

impl Surfaces {
    pub(crate) fn shown(&mut self, pass: u64) {
        self.last_pass = Some(self.last_pass.map_or(pass, |p| p.max(pass)));
        self.open = true;
    }

    /// Returns true when the surfaces just closed (no surface drawn in the previous pass).
    pub(crate) fn refresh(&mut self, pass: u64) -> bool {
        if self.open && self.last_pass.is_none_or(|p| p.saturating_add(1) < pass) {
            self.open = false;
            return true;
        }
        false
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.last_pass = None;
    }

    pub(crate) fn open(&self) -> bool {
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> MonoTime {
        MonoTime(Duration::from_millis(ms))
    }

    #[test]
    fn taint_covers_three_passes_and_one_second() {
        let ctx = egui::Context::default();
        let mut taint = Taint::default();
        assert!(!taint.live(10, t(0)));
        taint.record(&ctx, 10, t(1_000), &[egui::Event::Text("x".into())]);
        // Within the second: tainted whatever the pass.
        assert!(taint.live(10, t(1_000)));
        assert!(taint.live(50, t(1_999)));
        // After the second: only the three passes.
        assert!(taint.live(12, t(5_000)));
        assert!(!taint.live(13, t(5_000)));
        assert_eq!(taint.injected(), 1);
    }

    #[test]
    fn a_later_injection_extends_never_shortens() {
        let ctx = egui::Context::default();
        let mut taint = Taint::default();
        taint.record(&ctx, 20, t(2_000), &[egui::Event::Text("a".into())]);
        taint.record(&ctx, 5, t(1_000), &[egui::Event::Text("b".into())]);
        assert!(taint.live(22, t(10_000)));
        assert!(taint.live(0, t(2_999)));
    }

    #[test]
    fn press_provenance_needs_a_fresh_untainted_press() {
        let ctx = egui::Context::default();
        let mut taint = Taint::default();
        let press =
            egui::Event::PointerButton { pos: egui::pos2(1.0, 1.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        taint.record(&ctx, 1, t(0), &[press]);
        assert!(taint.press_tainted());
        // Untainted pass, button still down or released without a new press: still tainted.
        taint.observe_pointer(false, false, true);
        taint.observe_pointer(false, false, false);
        assert!(taint.press_tainted());
        // A fresh press in an untainted pass clears it.
        taint.observe_pointer(false, true, true);
        assert!(!taint.press_tainted());
        // A press seen while tainted taints again.
        taint.observe_pointer(true, false, true);
        assert!(taint.press_tainted());
    }

    #[test]
    fn visibility_restarts_after_a_missed_pass() {
        let mut v = Visibility::default();
        let id = egui::Id::new("w");
        assert_eq!(v.visible_since(id, 1, t(0)), t(0));
        assert_eq!(v.visible_since(id, 2, t(100)), t(0));
        assert_eq!(v.visible_since(id, 2, t(150)), t(0));
        assert_eq!(v.visible_since(id, 3, t(400)), t(0));
        // Pass 4 missed.
        assert_eq!(v.visible_since(id, 5, t(500)), t(500));
    }

    #[test]
    fn surfaces_close_one_pass_after_the_last_draw() {
        let mut s = Surfaces::default();
        assert!(!s.open());
        s.shown(7);
        assert!(!s.refresh(7));
        assert!(!s.refresh(8));
        assert!(s.open());
        assert!(s.refresh(9));
        assert!(!s.open());
    }

    #[test]
    fn context_marker_follows_the_record() {
        let ctx = egui::Context::default();
        let mut taint = Taint::default();
        assert!(!context_marker_live(&ctx));
        taint.record(&ctx, ctx.cumulative_pass_nr(), t(0), &[egui::Event::Text("x".into())]);
        assert!(context_marker_live(&ctx));
    }
}

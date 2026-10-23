//! Human-only surfaces and the input-provenance gate (SPEC §3.20; owned by WP9, red zone).
//!
//! WP0 stubs fail closed (§1.3 rule 7): nothing is ever a human click, every frame counts as synthetic, a surface
//! counts as open (so injected input is dropped), and the ARM dialog cancels.
#![forbid(unsafe_code)]

mod gesture;

use std::sync::Arc;

pub use gesture::{HumanGesture, gesture};
use openlaser_types::Clock;
use openlaser_ui_egui::{ArmModel, ArmOutcome, HumanSurfaces};

/// The taint record lives in this struct (keyed by `ctx.cumulative_pass_nr()` plus the time of the last injected
/// event), NOT in egui context data: any code holding the `Context` can clear `ctx.data_mut(..)`, while `Human` is
/// reachable only through `&mut dyn HumanSurfaces`.
pub struct Human {
    clock: Arc<dyn Clock>,
}

impl Human {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Human { clock }
    }
}

impl HumanSurfaces for Human {
    fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>) {
        // WP0 stub: synthetic input is dropped (never reaches a widget) until WP9 implements the taint record.
        let _ = (ctx, raw, events);
    }

    fn frame_is_synthetic(&self, ctx: &egui::Context) -> bool {
        let _ = ctx;
        true
    }

    fn is_human_click(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        let now = self.clock.now();
        gesture(ctx, response, now, now).is_some()
    }

    fn arm_dialog(&mut self, ctx: &egui::Context, model: &ArmModel) -> ArmOutcome {
        let _ = (ctx, model);
        ArmOutcome::Cancelled
    }

    fn surface_open(&self) -> bool {
        true
    }
}

//! The control dispatcher (§4.5). Human-only commands and surfaces are refused here (AND in engine policy).
//! WP0 stub: every method is refused.

pub mod input;

use std::sync::mpsc::Sender;

use openlaser_control::server::ControlRequest;
use openlaser_control::wire::Reply;

use crate::app::OpenLaserApp;

pub enum Outcome {
    Done(Reply),
    AfterInput(Reply),
    Screenshot { max_side: u32, reply: Sender<Reply> },
}

/// Methods §4.5.
pub fn handle(app: &mut OpenLaserApp, ctx: &egui::Context, req: ControlRequest) -> Outcome {
    let _ = (app, ctx);
    let method = req.request.method.clone();
    Outcome::Done(Reply { id: req.request.id, ok: false, result: None, error: Some(format!("not implemented: control method `{method}`")) })
}

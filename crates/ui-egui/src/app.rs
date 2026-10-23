//! `OpenLaserApp`. WP0 stub: a placeholder window; Esc latches every machine's stop.

use std::sync::mpsc::Receiver;

use openlaser_control::server::ControlRequest;
use openlaser_engine::Session;

use crate::services::Services;

/// UI state (panels, canvas view, dialogs). WP8 fills it.
#[derive(Debug, Default)]
pub struct UiState {
    _private: (),
}

pub struct OpenLaserApp {
    session: Session,
    ui: UiState,
    services: Services,
    control_rx: Option<Receiver<ControlRequest>>,
}

impl OpenLaserApp {
    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, services: Services) -> Self {
        let _ = cc;
        OpenLaserApp { session, ui: UiState::default(), services, control_rx: None }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn ui_state(&self) -> &UiState {
        &self.ui
    }
}

impl eframe::App for OpenLaserApp {
    /// (1) Esc while a machine is framing, armed or marking, or while the ARM dialog is open → session.stop_all()
    /// (latch first, then PendingArm voided) and remove the event before any widget sees it; (2) native-menu key
    /// equivalents; (3) queued synthetic events go in only through services.human.inject() (§2.3 rule 16).
    /// WP0 stub: every Esc press stops every registered machine.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let _ = ctx;
        let esc = raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
        if esc {
            let _ = self.session.stop_all();
        }
    }

    /// Every frame, and while hidden whenever a repaint was requested (eframe 0.36 calls `logic` for a hidden window
    /// only then: the control server's `wake` and a 250 ms `request_repaint_after` while a machine is active keep it
    /// running): session.poll(); drain the control channel (control::handle); native menu clicks; shortcuts
    /// (Origin::Synthetic if services.human.frame_is_synthetic()); file drops; close guard (machine active →
    /// CancelClose + stop first; craft_quit last). WP0 stub: polls the session only.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _ = (ctx, frame, &self.control_rx, &self.services.quit);
        let _ = self.session.poll();
    }

    /// The LightBurn layout (UI §4.1), STOP overlay last at Order::Tooltip. WP0 stub: a placeholder.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _ = frame;
        ui.label("OpenLaser: WP0 scaffold. The LightBurn layout arrives with WP8.");
    }
}

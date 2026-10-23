//! The canvas (UI §5.3). WP0 stubs paint nothing.

use openlaser_render::DisplayList;

pub struct ViewXform {
    /// pt per mm.
    pub zoom: f64,
    pub center: [f64; 2],
    pub rect: egui::Rect,
}

pub trait CanvasRenderer {
    fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform);
}

/// Own hairline mesher + mesh cache (UI §5.3).
#[derive(Debug, Default)]
pub struct PainterRenderer {
    _private: (),
}

impl CanvasRenderer for PainterRenderer {
    fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform) {
        let _ = (painter, list, view);
    }
}

/// Paints a display list fitted into `rect` (used by human's ARM-dialog thumbnail and the Preview window).
pub fn paint_fit(painter: &egui::Painter, list: &DisplayList, rect: egui::Rect) {
    let _ = (painter, list, rect);
}

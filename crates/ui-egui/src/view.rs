//! The design canvas (UI §5): rulers outside the canvas rect, an adaptive grid in the work area, the encodable field
//! and the work area, the centre cross, the document display list, selection (click, Shift, marquee), and zoom/pan
//! as LightBurn does it: wheel or pinch zooms about the cursor, trackpad scroll and middle-drag (or the Pan tool, or
//! Space-drag) pan, Zoom to Page frames the field.

use crate::app::OpenLaserApp;
use crate::canvas::{CanvasRenderer, PainterRenderer, ViewXform};

/// Ruler strip thickness (UI §5.2 item 9).
pub(crate) const RULER: f32 = 16.0;
/// Points per millimetre.
pub(crate) const ZOOM_MIN: f64 = 0.2;
pub(crate) const ZOOM_MAX: f64 = 5000.0;
/// Fit leaves a 5 % margin around the field (UI §5.1).
const FIT_MARGIN: f64 = 1.05;

#[derive(Default)]
pub(crate) struct CanvasView {
    /// None = a fit is pending (first frame, Zoom to Page).
    pub zoom: Option<f64>,
    pub center: [f64; 2],
    pub renderer: PainterRenderer,
    /// The zoom a fit gives for the current canvas, for zoom buttons pressed before the first paint.
    pub last_fit: f64,
    pub marquee_from: Option<egui::Pos2>,
}

impl CanvasView {
    pub(crate) fn fit(&mut self) {
        self.zoom = None;
    }

    /// Zoom In / Out buttons and shortcuts: about the canvas centre.
    pub(crate) fn zoom_by(&mut self, factor: f64) {
        let current = self.zoom.unwrap_or(if self.last_fit > 0.0 { self.last_fit } else { 1.0 });
        let next = current * factor;
        if next.is_finite() {
            self.zoom = Some(next.clamp(ZOOM_MIN, ZOOM_MAX));
        }
    }
}

/// What the canvas draws for a machine, in world mm (Y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MachineGeometry {
    /// Galvo: the encodable field ±F/2, F = encoding_field × cal.scale. Gantry: none.
    pub field: Option<[[f64; 2]; 2]>,
    /// Galvo: the work area (rated lens field). Gantry: the bed.
    pub work_area: [[f64; 2]; 2],
}

impl MachineGeometry {
    pub(crate) fn of(machine: Option<&openlaser_engine::MachineProfile>) -> Self {
        match machine.map(|m| &m.kind) {
            Some(openlaser_machine::Kinematics::Galvo(g)) => {
                let half = [g.encoding_field.get() * g.cal.scale[0] / 2.0, g.encoding_field.get() * g.cal.scale[1] / 2.0];
                Self { field: Some([[-half[0], -half[1]], half]), work_area: [g.work_area.min, g.work_area.max] }
            }
            Some(openlaser_machine::Kinematics::Gantry(g)) => Self { field: None, work_area: [[0.0, 0.0], g.bed_mm] },
            None => Self { field: None, work_area: [[-100.0, -100.0], [100.0, 100.0]] },
        }
    }

    /// The rectangle Zoom to Page frames.
    pub(crate) fn extent(&self) -> [[f64; 2]; 2] {
        self.field.unwrap_or(self.work_area)
    }
}

pub(crate) fn fit_zoom(canvas: egui::Rect, extent: [[f64; 2]; 2]) -> f64 {
    let w = (extent[1][0] - extent[0][0]).abs().max(1.0) * FIT_MARGIN;
    let h = (extent[1][1] - extent[0][1]).abs().max(1.0) * FIT_MARGIN;
    let zoom = (f64::from(canvas.width().max(1.0)) / w).min(f64::from(canvas.height().max(1.0)) / h);
    if zoom.is_finite() { zoom.clamp(ZOOM_MIN, ZOOM_MAX) } else { 1.0 }
}

/// The smallest step of the 1-2-5 series that is at least `min` (mm).
pub(crate) fn nice_step(min: f64) -> f64 {
    if !min.is_finite() || min <= 0.0 {
        return 1.0;
    }
    let base = 10f64.powf(min.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|k| k * base).find(|s| *s >= min).unwrap_or(10.0 * base)
}

fn label_for(value: f64, step: f64) -> String {
    let decimals = if step >= 1.0 { 0 } else { (-step.log10()).ceil().clamp(0.0, 4.0) as usize };
    let value = if value.abs() < step * 1e-6 { 0.0 } else { value };
    format!("{value:.decimals$}")
}

/// Wheel/pinch zoom about `anchor` (screen): the world point under the cursor stays put.
pub(crate) fn zoom_about(zoom: f64, center: [f64; 2], rect: egui::Rect, anchor: egui::Pos2, factor: f64) -> (f64, [f64; 2]) {
    let next = (zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    if !next.is_finite() {
        return (zoom, center);
    }
    let before = ViewXform { zoom, center, rect }.screen_to_world(anchor);
    let after = ViewXform { zoom: next, center, rect }.screen_to_world(anchor);
    (next, [center[0] + before[0] - after[0], center[1] + before[1] - after[1]])
}

pub(crate) struct Palette {
    pub(crate) background: egui::Color32,
    pub(crate) pasteboard: egui::Color32,
    pub(crate) grid_minor: egui::Color32,
    pub(crate) grid_major: egui::Color32,
    pub(crate) field: egui::Color32,
    pub(crate) work_area: egui::Color32,
    pub(crate) cross: egui::Color32,
    pub(crate) ruler: egui::Color32,
    pub(crate) ruler_text: egui::Color32,
}

pub(crate) fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            background: egui::Color32::from_gray(44),
            pasteboard: egui::Color32::from_gray(34),
            grid_minor: egui::Color32::from_gray(54),
            grid_major: egui::Color32::from_gray(70),
            field: egui::Color32::from_rgb(90, 140, 110),
            work_area: egui::Color32::from_gray(190),
            cross: egui::Color32::from_gray(150),
            ruler: egui::Color32::from_gray(28),
            ruler_text: egui::Color32::from_gray(170),
        }
    } else {
        Palette {
            background: egui::Color32::from_gray(252),
            pasteboard: egui::Color32::from_gray(228),
            grid_minor: egui::Color32::from_gray(236),
            grid_major: egui::Color32::from_gray(214),
            field: egui::Color32::from_rgb(60, 130, 90),
            work_area: egui::Color32::from_gray(70),
            cross: egui::Color32::from_gray(110),
            ruler: egui::Color32::from_gray(238),
            ruler_text: egui::Color32::from_gray(70),
        }
    }
}

impl OpenLaserApp {
    pub(crate) fn paint_canvas(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        if full.width() < RULER * 2.0 || full.height() < RULER * 2.0 {
            return;
        }
        let response = ui.allocate_rect(full, egui::Sense::click_and_drag());
        let canvas = egui::Rect::from_min_max(full.min + egui::vec2(RULER, RULER), full.max);
        let geometry = MachineGeometry::of(self.active_machine());
        let fit = fit_zoom(canvas, geometry.extent());
        self.view.last_fit = fit;
        if self.view.zoom.is_none() {
            let e = geometry.extent();
            self.view.zoom = Some(fit);
            self.view.center = [(e[0][0] + e[1][0]) / 2.0, (e[0][1] + e[1][1]) / 2.0];
        }
        let mut zoom = self.view.zoom.unwrap_or(fit);
        let mut center = self.view.center;

        // Zoom and pan input, only while the pointer is over the canvas (a docked panel's scrolling never zooms it).
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| canvas.contains(*p));
        if let Some(anchor) = hover {
            let (pinch, wheel_lines, trackpad) = ui.input(|i| {
                let mut lines = 0.0f32;
                let mut points = egui::Vec2::ZERO;
                for event in &i.raw.events {
                    if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
                        if modifiers.command || modifiers.ctrl {
                            continue; // egui folds these into zoom_delta
                        }
                        match unit {
                            egui::MouseWheelUnit::Point => points += *delta,
                            egui::MouseWheelUnit::Line => lines += delta.y,
                            egui::MouseWheelUnit::Page => lines += delta.y * 10.0,
                        }
                    }
                }
                (i.zoom_delta(), lines, points)
            });
            let factor = f64::from(pinch) * (f64::from(wheel_lines) * 0.15).exp();
            if factor.is_finite() && (factor - 1.0).abs() > 1e-9 {
                (zoom, center) = zoom_about(zoom, center, canvas, anchor, factor);
            }
            if trackpad != egui::Vec2::ZERO {
                center[0] -= f64::from(trackpad.x) / zoom;
                center[1] += f64::from(trackpad.y) / zoom;
            }
        }
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let panning = response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && (self.ui.selected_tool == "Pan" || space));
        if panning {
            let delta = response.drag_delta();
            center[0] -= f64::from(delta.x) / zoom;
            center[1] += f64::from(delta.y) / zoom;
        }
        self.view.zoom = Some(zoom);
        self.view.center = center;
        let view = ViewXform { zoom, center, rect: canvas };
        self.ui.pointer_mm = hover.map(|p| view.screen_to_world(p));

        let colors = palette(ui.visuals().dark_mode);
        let painter = ui.painter().with_clip_rect(canvas);
        painter.rect_filled(canvas, 0.0, colors.pasteboard);
        let work = egui::Rect::from_two_pos(view.world_to_screen(geometry.work_area[0]), view.world_to_screen(geometry.work_area[1]));
        painter.rect_filled(work, 0.0, colors.background);

        // Grid in the work area: minor lines ≥ 8 pt apart, major lines ≥ 80 pt apart (1-2-5 series).
        let grid = painter.with_clip_rect(work.intersect(canvas));
        let low = view.screen_to_world(canvas.left_bottom());
        let high = view.screen_to_world(canvas.right_top());
        for (min_pt, color) in [(8.0, colors.grid_minor), (80.0, colors.grid_major)] {
            let step = nice_step(min_pt / zoom);
            for axis in 0..2 {
                let (from, to) = if axis == 0 { (low[0], high[0]) } else { (low[1], high[1]) };
                let first = (from / step).ceil();
                let count = ((to - from) / step).floor();
                if !first.is_finite() || !count.is_finite() || count > 4000.0 {
                    continue;
                }
                for n in 0..=(count as i64) {
                    let value = (first + n as f64) * step;
                    let (a, b) = if axis == 0 {
                        let x = view.world_to_screen([value, 0.0]).x;
                        (egui::pos2(x, canvas.top()), egui::pos2(x, canvas.bottom()))
                    } else {
                        let y = view.world_to_screen([0.0, value]).y;
                        (egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y))
                    };
                    grid.line_segment([a, b], egui::Stroke::new(1.0, color));
                }
            }
        }

        // The encodable field: faint dashed, labelled; the work area: solid; the centre cross at world (0, 0).
        if let Some(field) = geometry.field {
            let r = egui::Rect::from_two_pos(view.world_to_screen(field[0]), view.world_to_screen(field[1]));
            let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            for shape in egui::Shape::dashed_line(&corners, egui::Stroke::new(1.0, colors.field), 6.0, 4.0) {
                painter.add(shape);
            }
            painter.text(r.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "encodable", egui::FontId::proportional(10.0), colors.field);
        }
        painter.rect_stroke(work, 0.0, egui::Stroke::new(1.0, colors.work_area), egui::StrokeKind::Middle);
        let origin = view.world_to_screen([0.0, 0.0]);
        let cross = egui::Stroke::new(1.0, colors.cross);
        painter.line_segment([origin - egui::vec2(8.0, 0.0), origin + egui::vec2(8.0, 0.0)], cross);
        painter.line_segment([origin - egui::vec2(0.0, 8.0), origin + egui::vec2(0.0, 8.0)], cross);

        // The document, from the engine's display list (layer colours, refusal and notice markers).
        let opts = openlaser_render::ViewOpts {
            dark: ui.visuals().dark_mode,
            selection: self.selection(),
            refusals: self.plan().map(|p| p.refusals().to_vec()).unwrap_or_default(),
            notices: self.plan().map(|p| p.notices().to_vec()).unwrap_or_default(),
            show_field: false,
            show_work_area: false,
        };
        let list = self.project().map(|project| openlaser_render::document_list(project, self.active_machine(), &opts));
        if let Some(list) = list {
            self.view.renderer.paint(&painter, &list, &view);
        }

        self.canvas_selection(ui, &response, &view);
        self.paint_rulers(ui, full, canvas, &view, hover, &colors);

        // Zoom to fit, at the canvas corner.
        let button = egui::Rect::from_min_size(canvas.right_bottom() - egui::vec2(34.0, 30.0), egui::vec2(28.0, 24.0));
        let fit_response = ui.put(button, egui::Button::image(crate::panels::icon_image("maximize")));
        self.labeled(&fit_response, egui::WidgetType::Button, "Zoom to fit", false);
        if fit_response.on_hover_text("Zoom to fit (frame the field)").clicked() {
            self.view.fit();
        }
    }

    fn canvas_selection(&mut self, ui: &egui::Ui, response: &egui::Response, view: &ViewXform) {
        if self.ui.selected_tool != "Select" || ui.input(|i| i.key_down(egui::Key::Space)) {
            self.view.marquee_from = None;
            return;
        }
        let shift = ui.input(|i| i.modifiers.shift);
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.view.marquee_from = response.interact_pointer_pos();
        }
        if let (Some(from), Some(to)) = (self.view.marquee_from, response.interact_pointer_pos()) {
            let crossing = to.x < from.x;
            let rect = egui::Rect::from_two_pos(from, to);
            let color = if crossing { egui::Color32::from_rgb(80, 200, 120) } else { egui::Color32::from_rgb(90, 150, 255) };
            let painter = ui.painter().with_clip_rect(view.rect);
            if crossing {
                let c = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
                painter.extend(egui::Shape::dashed_line(&c, egui::Stroke::new(1.0, color), 4.0, 3.0));
            } else {
                painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Middle);
            }
            if response.drag_stopped() {
                self.view.marquee_from = None;
                let (a, b) = (view.screen_to_world(from), view.screen_to_world(to));
                self.execute_widget(
                    "edit.select",
                    serde_json::json!({ "rectMm": [a, b], "crossing": crossing, "mode": if shift { "add" } else { "replace" } }),
                );
            }
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(position) = response.interact_pointer_pos()
        {
            let world = view.screen_to_world(position);
            let hit = self.project().and_then(|project| {
                project.shapes.iter().rev().find_map(|shape| {
                    // A shape the kernel cannot flatten is not hit (there is nothing the user could have clicked).
                    let paths: Vec<Vec<[f64; 2]>> = project.world_polylines(shape.id, 0.02).unwrap_or_default().into_iter().map(|line| line.0).collect();
                    crate::canvas::hit_test(&paths, world, 4.0 / view.zoom).map(|_| shape.id)
                })
            });
            match (hit, shift) {
                (None, false) => self.invoke_ui_action("edit.selectNone"),
                (None, true) => {}
                (Some(id), _) => self.execute_widget("edit.select", serde_json::json!({ "ids": [id], "mode": if shift { "toggle" } else { "replace" } })),
            }
        }
    }

    fn paint_rulers(&self, ui: &egui::Ui, full: egui::Rect, canvas: egui::Rect, view: &ViewXform, hover: Option<egui::Pos2>, colors: &Palette) {
        let painter = ui.painter().with_clip_rect(full);
        let top = egui::Rect::from_min_max(egui::pos2(canvas.left(), full.top()), egui::pos2(full.right(), canvas.top()));
        let left = egui::Rect::from_min_max(egui::pos2(full.left(), canvas.top()), egui::pos2(canvas.left(), full.bottom()));
        painter.rect_filled(top, 0.0, colors.ruler);
        painter.rect_filled(left, 0.0, colors.ruler);
        painter.rect_filled(egui::Rect::from_min_max(full.min, canvas.min), 0.0, colors.ruler);
        let label_step = nice_step(60.0 / view.zoom);
        let tick_step = nice_step(8.0 / view.zoom).max(label_step / 10.0);
        let low = view.screen_to_world(canvas.left_bottom());
        let high = view.screen_to_world(canvas.right_top());
        let tick = egui::Stroke::new(1.0, colors.ruler_text);
        let font = egui::FontId::monospace(9.0);
        for axis in 0..2 {
            let (from, to) = if axis == 0 { (low[0], high[0]) } else { (low[1], high[1]) };
            for (step, length, labelled) in [(tick_step, 4.0, false), (label_step, RULER - 2.0, true)] {
                let first = (from / step).ceil();
                let count = ((to - from) / step).floor();
                if !first.is_finite() || !count.is_finite() || count > 4000.0 {
                    continue;
                }
                for n in 0..=(count as i64) {
                    let value = (first + n as f64) * step;
                    if axis == 0 {
                        let x = view.world_to_screen([value, 0.0]).x;
                        painter.line_segment([egui::pos2(x, top.bottom() - length), egui::pos2(x, top.bottom())], tick);
                        if labelled {
                            painter.text(egui::pos2(x + 2.0, top.top()), egui::Align2::LEFT_TOP, label_for(value, step), font.clone(), colors.ruler_text);
                        }
                    } else {
                        let y = view.world_to_screen([0.0, value]).y;
                        painter.line_segment([egui::pos2(left.right() - length, y), egui::pos2(left.right(), y)], tick);
                        if labelled {
                            painter.text(
                                egui::pos2(left.left() + 1.0, y + 1.0),
                                egui::Align2::LEFT_TOP,
                                label_for(value, step),
                                font.clone(),
                                colors.ruler_text,
                            );
                        }
                    }
                }
            }
        }
        if let Some(p) = hover {
            let marker = egui::Stroke::new(1.0, egui::Color32::from_rgb(230, 80, 60));
            painter.line_segment([egui::pos2(p.x, top.top()), egui::pos2(p.x, top.bottom())], marker);
            painter.line_segment([egui::pos2(left.left(), p.y), egui::pos2(left.right(), p.y)], marker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_step_is_1_2_5() {
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(1.2), 2.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(6.0), 10.0);
        assert_eq!(nice_step(0.013), 0.02);
        assert_eq!(nice_step(f64::NAN), 1.0);
    }

    #[test]
    fn zoom_about_keeps_the_world_point_under_the_cursor() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0));
        let anchor = egui::pos2(123.0, 456.0);
        let before = ViewXform { zoom: 2.0, center: [5.0, -3.0], rect }.screen_to_world(anchor);
        let (zoom, center) = zoom_about(2.0, [5.0, -3.0], rect, anchor, 1.7);
        assert!((zoom - 3.4).abs() < 1e-9);
        let after = ViewXform { zoom, center, rect }.screen_to_world(anchor);
        assert!((before[0] - after[0]).abs() < 1e-4 && (before[1] - after[1]).abs() < 1e-4, "{before:?} vs {after:?}");
        assert_eq!(zoom_about(ZOOM_MAX, [0.0; 2], rect, anchor, 10.0).0, ZOOM_MAX);
    }

    #[test]
    fn galvo_geometry_is_the_encodable_field_and_the_work_area() {
        let g = MachineGeometry::of(Some(&openlaser_machine::jpt_m7_200_sim()));
        let field = g.field.unwrap();
        assert!((field[1][0] - 118.34).abs() < 1e-9, "F/2 = 200 × 1.1834 / 2: {field:?}");
        assert_eq!(g.work_area, [[-100.0, -100.0], [100.0, 100.0]]);
        assert_eq!(g.extent(), field);
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 500.0));
        assert!((fit_zoom(canvas, g.extent()) - 500.0 / (236.68 * 1.05)).abs() < 1e-9);
    }
}

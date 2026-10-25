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

/// The draw tools: (command id, tool label). Clicking the toolbar button or menu row only ARMS the tool; the
/// canvas drag or clicks produce the command's parameters, like LightBurn.
/// The armed tools: (command id, tool label). Clicking the toolbar button or menu row only ARMS the tool; the
/// canvas drag or clicks produce the command's parameters, like LightBurn. `tools.measure` is armed the same
/// way but never dispatches — the measuring line is purely visual.
pub(crate) const DRAW_TOOLS: [(&str, &str); 5] = [
    ("tools.rectangle", "Rectangle"),
    ("tools.ellipse", "Ellipse"),
    ("tools.drawLines", "Draw Lines"),
    ("tools.polygon", "Polygon"),
    ("tools.measure", "Measure"),
];

/// Selection handle size on screen (points).
const HANDLE_SIZE: f32 = 7.0;
/// How close a press must land to a handle to grab it (points).
const HANDLE_GRAB: f32 = 6.0;

/// The resize cursor of scale handle `i` in screen space (grid position maps to the diagonal/axis arrows).
fn resize_cursor(i: usize) -> egui::CursorIcon {
    match i {
        0 | 4 => egui::CursorIcon::ResizeNwSe,
        2 | 6 => egui::CursorIcon::ResizeNeSw,
        1 | 5 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeHorizontal,
    }
}

#[derive(Default)]
pub(crate) struct CanvasView {
    /// None = a fit is pending (first frame, Zoom to Page).
    pub zoom: Option<f64>,
    pub center: [f64; 2],
    pub renderer: PainterRenderer,
    /// The zoom a fit gives for the current canvas, for zoom buttons pressed before the first paint.
    pub last_fit: f64,
    pub marquee_from: Option<egui::Pos2>,
    /// A Rectangle/Ellipse drag in progress (screen space, like the marquee).
    pub draw_from: Option<egui::Pos2>,
    /// Draw Lines points placed so far (world mm, so a mid-draw pan keeps them anchored).
    pub draw_points: Vec<[f64; 2]>,
    /// The canvas rect of the last paint (screen points); tests map world to screen through it.
    pub last_canvas: Option<egui::Rect>,
    /// A move / scale / rotate drag on the selection (live ghost; one command on release).
    pub sel_drag: Option<SelectionDrag>,
    /// The last measured segment (world mm): kept on screen until the next press, Esc or a tool change.
    pub measure: Option<([f64; 2], [f64; 2])>,
    /// LightBurn's handle toggle: clicking an already-selected shape switches between scale and rotate
    /// handles. Any selection change resets this to scale handles.
    pub rotate_handles: bool,
}

/// What a selection drag does on release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SelDragKind {
    Move,
    /// Scale about the opposite handle (the handle index, `canvas_xform` order).
    Scale(usize),
    Rotate,
}

/// An in-progress selection transform drag. The document is untouched until release; `ghost` is the
/// selection's polylines at press, drawn under the same affine the commit dispatches.
pub(crate) struct SelectionDrag {
    pub kind: SelDragKind,
    /// The world-space press point (stable across a mid-drag zoom).
    pub press_world: [f64; 2],
    /// Latest pointer position (screen).
    pub pos: egui::Pos2,
    /// The selection bounds at press (world mm).
    pub bounds: openlaser_types::Bounds,
    /// World polylines of the selected shapes at press.
    pub ghost: Vec<Vec<[f64; 2]>>,
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
        self.view.last_canvas = Some(canvas);
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

        // Live ghost of the Offset Shapes dialog's result, computed with the same kernel the command commits.
        if let Some(o) = self.ui.offset {
            use openlaser_geom::boolean::{Cap, Join};
            let join = match o.corner {
                1 => Join::Miter { limit: 4.0 },
                2 => Join::Bevel,
                _ => Join::Round,
            };
            let ghost = egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 255));
            for oid in self.selection() {
                if let Some(polys) = self.project().and_then(|p| p.world_polylines(oid, 0.01).ok()) {
                    let closed: Vec<openlaser_geom::Polyline> = polys.iter().filter(|pl| pl.is_closed(openlaser_geom::CLOSURE_TOL_MM)).cloned().collect();
                    let mut rings = Vec::new();
                    if closed.is_empty() {
                        rings.extend(openlaser_geom::boolean::stroke_outline(&polys, o.distance / 2.0, join, Cap::Round, 0.01).unwrap_or_default());
                    } else {
                        for sign in match o.direction {
                            2 => vec![o.distance, -o.distance],
                            1 => vec![-o.distance],
                            _ => vec![o.distance],
                        } {
                            rings.extend(openlaser_geom::boolean::offset(&closed, sign, join, 0.01).unwrap_or_default());
                        }
                    }
                    for r in rings {
                        let pts: Vec<egui::Pos2> = r.0.iter().map(|p| view.world_to_screen(*p)).collect();
                        painter.add(egui::Shape::line(pts, ghost));
                    }
                }
            }
        }

        self.canvas_selection(ui, &response, &view);
        self.canvas_nodes(ui, &response, &view);
        self.canvas_draw(ui, &response, &view, hover);
        self.paint_nodes(ui, &view);

        // ---- the inline text editor: an Area at the anchor, live outline ghost, commit on Esc/outside/Enter.
        if let Some(mut edit) = self.ui.text.take() {
            // The click that opened the editor must not also commit it on this same frame.
            let just_opened = edit.request_focus;
            let d = &self.ui.text_defaults;
            // Ghost: the same layout the command commits, so WYSIWYG.
            let face = self
                .session
                .fonts()
                .face(&d.family, Some(&d.style))
                .or_else(|| ["Arial", "Helvetica", "Hack"].iter().find_map(|f| self.session.fonts().face(f, None)));
            if let Some(face) = face {
                let align = match d.align {
                    1 => openlaser_types::TextAlign::Center,
                    2 => openlaser_types::TextAlign::Right,
                    _ => openlaser_types::TextAlign::Left,
                };
                if let Ok(path) = openlaser_text::layout(&face, &edit.buffer, d.height, d.letter_spacing, d.line_spacing, align) {
                    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 255));
                    let mut world = path;
                    world.apply_affine(openlaser_doc::Affine::translate((edit.anchor[0], edit.anchor[1])));
                    for p in openlaser_geom::flatten(&world, 0.01).unwrap_or_default() {
                        let pts: Vec<egui::Pos2> = p.0.iter().map(|pt| view.world_to_screen(*pt)).collect();
                        painter.add(egui::Shape::line(pts, stroke));
                    }
                }
            }
            let screen = view.world_to_screen(edit.anchor);
            let mut commit = false;
            let area = egui::Area::new(egui::Id::new("text-inline-edit")).order(egui::Order::Foreground).fixed_pos(screen).show(ui.ctx(), |ui| {
                egui::Frame::NONE.show(ui, |ui| {
                    let widget = egui::TextEdit::multiline(&mut edit.buffer).desired_rows(4).desired_width(200.0);
                    let r = ui.add(widget);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text".to_owned()));
                    if edit.request_focus {
                        edit.request_focus = false;
                        r.request_focus();
                    }
                    // Commit triggers: Esc, click outside the editor, Cmd+Enter.
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) || ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) {
                        commit = true;
                    }
                    if ui.input(|i| i.pointer.any_click())
                        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
                        && !r.rect.contains(pos)
                    {
                        commit = true;
                    }
                });
            });
            // Clicking the canvas itself (outside the Area's own rect) also commits — but not the opening click.
            if !just_opened && response.clicked_by(egui::PointerButton::Primary) {
                commit = true;
            }
            if commit {
                ui.input_mut(|i| {
                    i.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
                });
                if !edit.buffer.trim().is_empty() {
                    let d = &self.ui.text_defaults;
                    if let Some(id) = edit.editing {
                        let mut p = serde_json::json!({ "id": id, "text": edit.buffer, "align": d.align_str(), "heightMm": d.height, "letterSpacingMm": d.letter_spacing, "lineSpacing": d.line_spacing });
                        if let Some(map) = p.as_object_mut() {
                            if !d.family.is_empty() {
                                map.insert("family".to_owned(), d.family.clone().into());
                            }
                            if !d.style.is_empty() {
                                map.insert("style".to_owned(), d.style.clone().into());
                            }
                        }
                        self.execute_widget("text.edit", p);
                    } else {
                        self.execute_widget("tools.text", d.create_params(edit.anchor, &edit.buffer));
                    }
                }
                self.ui.selected_tool = "Select";
            } else if self.ui.selected_tool != "Edit Text" && edit.editing.is_none() {
                // A tool change commits too.
                if !edit.buffer.trim().is_empty() {
                    self.execute_widget("tools.text", self.ui.text_defaults.create_params(edit.anchor, &edit.buffer));
                }
            } else {
                self.ui.text = Some(edit);
            }
            let _ = area;
        }
        self.paint_rulers(ui, full, canvas, &view, hover, &colors);

        // Zoom to fit, at the canvas corner.
        let button = egui::Rect::from_min_size(canvas.right_bottom() - egui::vec2(34.0, 30.0), egui::vec2(28.0, 24.0));
        let fit_response = ui.put(button, egui::Button::image(crate::panels::icon_image("maximize", ui)));
        self.labeled(&fit_response, egui::WidgetType::Button, "Zoom to fit", false);
        if fit_response.on_hover_text("Zoom to fit (frame the field)").clicked() {
            self.view.fit();
        }
    }

    /// The draw tools (UI §5): Rectangle and Ellipse drag out a shape, Draw Lines clicks points and finishes on
    /// double-click, Enter or right-click (clicking the first point closes the path). Esc cancels a draw in
    /// progress and returns to Select; a placed shape does too, as LightBurn does. The command runs only once the
    /// drag or clicks produced real millimetre parameters — a bare tool click never fires it.
    fn canvas_draw(&mut self, ui: &egui::Ui, response: &egui::Response, view: &ViewXform, hover: Option<egui::Pos2>) {
        let armed = ["Edit Text", "Edit Nodes"].contains(&self.ui.selected_tool) || DRAW_TOOLS.iter().any(|(_, label)| *label == self.ui.selected_tool);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && armed && self.ui.nodes.is_none() {
            self.ui.selected_tool = "Select";
            self.view.draw_from = None;
            self.view.draw_points.clear();
            self.view.measure = None;
            return;
        }
        if !armed {
            self.view.draw_from = None;
            self.view.draw_points.clear();
            self.view.measure = None;
            return;
        }
        if hover.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let painter = ui.painter().with_clip_rect(view.rect);
        let ghost = egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 255));
        let finish = |app: &mut Self| {
            app.ui.selected_tool = "Select";
            app.view.draw_from = None;
            app.view.draw_points.clear();
        };
        match self.ui.selected_tool {
            "Rectangle" | "Ellipse" => {
                if response.drag_started_by(egui::PointerButton::Primary) {
                    // The press position, not the pointer's spot when the drag crossed its threshold.
                    self.view.draw_from = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
                }
                let Some(from) = self.view.draw_from else { return };
                // latest_pos stays set on the release frame (interact_pointer_pos does not).
                let Some(to) = ui.input(|i| i.pointer.latest_pos()) else {
                    self.view.draw_from = None;
                    return;
                };
                let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
                // Shift keeps the drag square/a circle; Alt draws from the centre instead of corner to corner.
                let rect = if alt {
                    let d = to - from;
                    let half = if shift { egui::vec2(d.x.abs().max(d.y.abs()), d.x.abs().max(d.y.abs())) } else { egui::vec2(d.x.abs(), d.y.abs()) };
                    egui::Rect::from_center_size(from, half * 2.0)
                } else if shift {
                    let d = to - from;
                    let side = d.x.abs().max(d.y.abs());
                    egui::Rect::from_two_pos(from, from + egui::vec2(d.x.signum() * side, d.y.signum() * side))
                } else {
                    egui::Rect::from_two_pos(from, to)
                };
                if self.ui.selected_tool == "Rectangle" {
                    painter.add(egui::Shape::dashed_line(
                        &[rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()],
                        ghost,
                        4.0,
                        3.0,
                    ));
                } else {
                    painter.add(egui::Shape::ellipse_stroke(rect.center(), rect.size() / 2.0, ghost));
                }
                if !response.drag_stopped() {
                    return;
                }
                // The committed rect is the constrained one (Shift/Alt), not the raw drag.
                let (a, b) = (view.screen_to_world(rect.min), view.screen_to_world(rect.max));
                let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
                let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
                let (w, h) = (x1 - x0, y1 - y0);
                if w >= 0.01 && h >= 0.01 {
                    if self.ui.selected_tool == "Rectangle" {
                        self.execute_widget("tools.rectangle", serde_json::json!({ "xMm": x0, "yMm": y0, "wMm": w, "hMm": h }));
                    } else {
                        self.execute_widget("tools.ellipse", serde_json::json!({ "cxMm": (x0 + x1) / 2.0, "cyMm": (y0 + y1) / 2.0, "wMm": w, "hMm": h }));
                    }
                }
                finish(self);
            }
            "Draw Lines" => {
                // The ghost: placed points plus a rubber segment to the pointer.
                let mut pts: Vec<egui::Pos2> = self.view.draw_points.iter().map(|p| view.world_to_screen(*p)).collect();
                if pts.last().is_some()
                    && let Some(now) = response.hover_pos()
                {
                    pts.push(now);
                }
                if pts.len() >= 2 {
                    painter.extend(egui::Shape::dashed_line(&pts, ghost, 4.0, 3.0));
                }
                let close = self.view.draw_points.len() >= 3
                    && response.clicked_by(egui::PointerButton::Primary)
                    && response
                        .interact_pointer_pos()
                        .zip(self.view.draw_points.first().map(|p| view.world_to_screen(*p)))
                        .is_some_and(|(pos, first)| pos.distance(first) < 6.0);
                if close {
                    let points = std::mem::take(&mut self.view.draw_points);
                    self.execute_widget("tools.drawLines", serde_json::json!({ "pointsMm": points, "closed": true }));
                    finish(self);
                } else if response.double_clicked_by(egui::PointerButton::Primary)
                    || response.secondary_clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    if self.view.draw_points.len() >= 2 {
                        let points = std::mem::take(&mut self.view.draw_points);
                        self.execute_widget("tools.drawLines", serde_json::json!({ "pointsMm": points, "closed": false }));
                    } else {
                        self.view.draw_points.clear();
                    }
                    finish(self);
                } else if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    // A double-click's own click must not add a duplicate point on top of the last one.
                    let duplicate = self.view.draw_points.last().is_some_and(|p| view.world_to_screen(*p).distance(pos) < 1.0);
                    if !duplicate {
                        self.view.draw_points.push(view.screen_to_world(pos));
                    }
                }
            }
            "Polygon" => {
                // Centre-out drag: radius = distance, the drag angle points the first vertex (the model's
                // polygon has its first vertex straight up, so rotateDeg = angle - 90°). Shift snaps to 15°.
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.view.draw_from = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
                }
                let Some(from) = self.view.draw_from else { return };
                let Some(to) = ui.input(|i| i.pointer.latest_pos()) else {
                    self.view.draw_from = None;
                    return;
                };
                let shift = ui.input(|i| i.modifiers.shift);
                let (c, p) = (view.screen_to_world(from), view.screen_to_world(to));
                let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                let radius = dx.hypot(dy);
                let mut rotate = dy.atan2(dx).to_degrees() - 90.0;
                if shift {
                    rotate = (rotate / 15.0).round() * 15.0;
                }
                let sides = self.ui.polygon_sides;
                let outline: Vec<egui::Pos2> = (0..=sides)
                    .map(|i| {
                        let a = (90.0 + rotate).to_radians() + std::f64::consts::TAU * f64::from(i) / f64::from(sides);
                        view.world_to_screen([c[0] + radius * a.cos(), c[1] + radius * a.sin()])
                    })
                    .collect();
                painter.extend(egui::Shape::dashed_line(&outline, ghost, 4.0, 3.0));
                self.ui.status_message = format!("Polygon: radius {radius:.2} mm, {sides} sides, rotate {rotate:+.1}°");
                if response.drag_stopped() {
                    if radius >= 0.01 {
                        self.execute_widget(
                            "tools.polygon",
                            serde_json::json!({ "cxMm": c[0], "cyMm": c[1], "radiusMm": radius, "sides": sides, "rotateDeg": rotate }),
                        );
                    }
                    finish(self);
                }
            }
            "Edit Text" => {
                // A canvas click opens the inline editor at that point (LightBurn's click-to-type).
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let world = view.screen_to_world(pos);
                    self.ui.text = Some(crate::app::TextUi { anchor: world, buffer: String::new(), editing: None, request_focus: true });
                }
            }
            "Measure" => {
                // A press starts a new measurement (the previous one clears); release keeps it on screen.
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.view.draw_from = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
                    self.view.measure = None;
                }
                let shift = ui.input(|i| i.modifiers.shift);
                // Shift snaps the segment to 15° directions.
                let snap = |a: [f64; 2], b: [f64; 2]| -> [f64; 2] {
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let deg = (dy.atan2(dx).to_degrees() / 15.0).round() * 15.0;
                    [a[0] + dx.hypot(dy) * deg.to_radians().cos(), a[1] + dx.hypot(dy) * deg.to_radians().sin()]
                };
                let live = match (self.view.draw_from, ui.input(|i| i.pointer.latest_pos())) {
                    (Some(from), Some(to)) => {
                        let a = view.screen_to_world(from);
                        let b = view.screen_to_world(to);
                        Some((a, if shift { snap(a, b) } else { b }))
                    }
                    _ => None,
                };
                if response.drag_stopped()
                    && let Some(from) = self.view.draw_from.take()
                {
                    let a = view.screen_to_world(from);
                    let b = view.screen_to_world(ui.input(|i| i.pointer.latest_pos()).unwrap_or(from));
                    self.view.measure = Some((a, if shift { snap(a, b) } else { b }));
                }
                let seg = live.or(self.view.measure);
                if let Some((a, b)) = seg {
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let (d, deg) = (dx.hypot(dy), dy.atan2(dx).to_degrees());
                    painter.line_segment([view.world_to_screen(a), view.world_to_screen(b)], egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 255)));
                    let text = format!("{d:.2} mm  dx {dx:+.2}  dy {dy:+.2}  {deg:+.1}°");
                    painter.text(
                        view.world_to_screen(b) + egui::vec2(8.0, -8.0),
                        egui::Align2::LEFT_BOTTOM,
                        &text,
                        egui::FontId::proportional(11.0),
                        egui::Color32::from_rgb(200, 210, 230),
                    );
                    self.ui.status_message = text;
                }
            }
            _ => {}
        }
    }

    /// The selection bounds in world mm, or None when nothing measurable is selected.
    fn selection_bounds(&self) -> Option<openlaser_types::Bounds> {
        let selection = self.selection();
        if selection.is_empty() {
            return None;
        }
        self.project().and_then(|p| p.bounds(&selection).ok().flatten())
    }

    /// The world polylines of every selected shape (the live-drag ghost).
    fn selection_ghost(&self) -> Vec<Vec<[f64; 2]>> {
        let selection = self.selection();
        self.project().map(|p| selection.iter().filter_map(|id| p.world_polylines(*id, 0.02).ok()).flatten().map(|line| line.0).collect()).unwrap_or_default()
    }

    /// Topmost shape under `world` (stroke hit), as the click path uses.
    fn hit_shape(&self, view: &ViewXform, world: [f64; 2]) -> Option<openlaser_types::ObjectId> {
        self.project().and_then(|project| {
            project.shapes.iter().rev().find_map(|shape| {
                // A shape the kernel cannot flatten is not hit (there is nothing the user could have clicked).
                let paths: Vec<Vec<[f64; 2]>> = project.world_polylines(shape.id, 0.02).unwrap_or_default().into_iter().map(|line| line.0).collect();
                crate::canvas::hit_test(&paths, world, 4.0 / view.zoom).map(|_| shape.id)
            })
        })
    }

    /// Which handle (canvas_xform index) `press` (screen) is on, if any. Rotate mode has the four corners only.
    fn handle_hit(&self, press: egui::Pos2, b: &openlaser_types::Bounds, view: &ViewXform) -> Option<usize> {
        let indices: &[usize] = if self.view.rotate_handles { &crate::canvas_xform::CORNERS } else { &[0, 1, 2, 3, 4, 5, 6, 7] };
        indices.iter().copied().find(|i| view.world_to_screen(crate::canvas_xform::handle_point(b, *i)).distance(press) <= HANDLE_GRAB)
    }

    // ---- Edit Nodes -----------------------------------------------------------------------------------------------

    /// The shape the node editor holds, in object space, mirroring `Geom::Path`. Selection or shape changes
    /// (undo, conversions) re-sync it; a missing/converted shape closes the editor.
    fn nodes_sync(&mut self) {
        if self.ui.selected_tool != "Edit Nodes" {
            self.ui.nodes = None;
            return;
        }
        let sel = self.selection();
        if sel.len() != 1 {
            self.ui.nodes = None;
            if sel.is_empty() {
                self.ui.status_message = "Edit Nodes: select one shape".to_owned();
            } else {
                self.ui.status_message = "Edit Nodes edits one shape at a time".to_owned();
            }
            return;
        }
        let Some(&id) = sel.first() else {
            self.ui.nodes = None;
            return;
        };
        let Some(project) = self.project() else { return };
        let Some(shape) = project.shape(id) else {
            self.ui.nodes = None;
            return;
        };
        match &shape.geom {
            openlaser_doc::Geom::Path(path) => {
                let keep = self.ui.nodes.as_ref().is_some_and(|n| n.id == id.0);
                if !keep {
                    self.ui.nodes =
                        Some(crate::app::NodeEditUi { id: id.0, subs: crate::node_edit::to_nodes(path), sel: Vec::new(), drag: None, marquee: None });
                }
            }
            openlaser_doc::Geom::Rect { .. }
            | openlaser_doc::Geom::Ellipse { .. }
            | openlaser_doc::Geom::Polygon { .. }
            | openlaser_doc::Geom::Text(_)
            | openlaser_doc::Geom::Bitmap(_) => {
                self.ui.nodes = None;
                self.ui.status_message = "Convert to path to edit its nodes".to_owned();
            }
            openlaser_doc::Geom::Group(_) => {
                self.ui.nodes = None;
                self.ui.status_message = "Edit Nodes works on a single path; ungroup first".to_owned();
            }
        }
    }

    /// The shape's object → world transform while node editing.
    fn node_world(&self) -> openlaser_doc::Affine {
        let (p, id) = (self.project(), self.ui.nodes.as_ref().map(|n| n.id));
        match (p, id) {
            (Some(p), Some(id)) => p.world_transform(openlaser_types::ObjectId(id)).ok().flatten().unwrap_or(openlaser_doc::Affine::IDENTITY),
            _ => openlaser_doc::Affine::IDENTITY,
        }
    }

    /// Serialize the edited subpaths and dispatch one `path.set` (one undo step; `coalesce` merges runs).
    pub(crate) fn nodes_commit(&mut self, coalesce: Option<&str>) {
        let Some(n) = self.ui.nodes.take() else { return };
        let (id, d) = (n.id, crate::node_edit::to_path(&n.subs).to_svg());
        self.ui.nodes = Some(n);
        let mut params = serde_json::json!({ "id": id, "d": d });
        if let Some(c) = coalesce
            && let Some(m) = params.as_object_mut()
        {
            m.insert("coalesce".to_owned(), c.into());
        }
        self.execute_widget("path.set", params);
    }

    /// Paint the ghost + anchors + handles; called from `paint_canvas` while `ui.nodes` is `Some`.
    fn paint_nodes(&mut self, ui: &egui::Ui, view: &ViewXform) {
        let Some(n) = &self.ui.nodes else { return };
        let xform = self.node_world();
        let painter = ui.painter().with_clip_rect(view.rect);
        // Ghost: the live path in world space.
        let world_path = {
            let mut p = crate::node_edit::to_path(&n.subs);
            p.apply_affine(xform);
            p
        };
        if let Ok(polys) = openlaser_geom::flatten(&world_path, 0.01) {
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(60, 200, 120));
            for line in &polys {
                let pts: Vec<egui::Pos2> = line.0.iter().map(|pt| view.world_to_screen(*pt)).collect();
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
        // Anchors (squares) and the handles of curves adjacent to selected nodes (circles + thin lines).
        let anchor_col = egui::Color32::from_rgb(30, 90, 200);
        let sel_fill = egui::Color32::from_rgb(255, 160, 40);
        let handle_col = egui::Color32::from_rgb(150, 150, 150);
        let sel = n.sel.clone();
        for (si, sp) in n.subs.iter().enumerate() {
            for (i, node) in sp.nodes.iter().enumerate() {
                let w = xform * node.p;
                let c = view.world_to_screen([w.x, w.y]);
                let chosen = sel.contains(&(si, i));
                let size = egui::vec2(6.0, 6.0);
                let rect = egui::Rect::from_center_size(c, size);
                painter.rect_filled(rect, 1.0, if chosen { sel_fill } else { egui::Color32::WHITE });
                painter.rect_stroke(rect, 1.0, egui::Stroke::new(1.0, anchor_col), egui::StrokeKind::Outside);
                // Handles of the node's own curve ends, drawn when selected.
                if chosen {
                    for h in [node.out, node.inn].into_iter().flatten() {
                        let hw = xform * h;
                        let hc = view.world_to_screen([hw.x, hw.y]);
                        painter.line_segment([c, hc], egui::Stroke::new(0.8, handle_col));
                        painter.circle(hc, 3.0, egui::Color32::WHITE, egui::Stroke::new(1.0, handle_col));
                    }
                }
            }
        }
        // The marquee.
        if let Some(from) = n.marquee
            && let Some(to) = ui.input(|i| i.pointer.latest_pos())
        {
            let a = view.world_to_screen(from);
            let rect = egui::Rect::from_two_pos(a, to);
            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 255)), egui::StrokeKind::Outside);
        }
    }

    /// Edit Nodes interaction: click selects nodes (Shift toggles), a drag moves them, a handle drag reshapes,
    /// a marquee selects inside it, a double-click on a segment inserts a node, keys do the model ops.
    fn canvas_nodes(&mut self, ui: &egui::Ui, response: &egui::Response, view: &ViewXform) {
        self.nodes_sync();
        if self.ui.nodes.is_none() {
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Esc: a drag/marquee cancels; otherwise the tool leaves to Select.
            let had = self.ui.nodes.as_ref().is_some_and(|n| n.drag.is_some() || n.marquee.is_some());
            if had {
                self.nodes_sync_forget();
            } else {
                self.ui.nodes = None;
                self.ui.selected_tool = "Select";
            }
            ui.ctx().input_mut(|i| {
                i.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
            });
            return;
        }
        let xform = self.node_world();
        let inv = xform.inverse();
        let painter = ui.painter().with_clip_rect(view.rect);
        let _ = painter;

        // ---- keys while the tool is active ---------------------------------------------------------------------
        if ui.ctx().memory(|m| m.focused().is_none()) {
            let (shift, cmd) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
            if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
                && self.ui.nodes.as_ref().is_some_and(|n| !n.sel.is_empty())
            {
                if let Some(n) = &mut self.ui.nodes {
                    crate::node_edit::delete_nodes(&mut n.subs, &n.sel.clone());
                    n.sel.clear();
                }
                self.nodes_commit(None);
            } else if ui.input(|i| i.key_pressed(egui::Key::L)) {
                let sel = self.ui.nodes.as_ref().map(|n| n.sel.clone()).unwrap_or_default();
                if let Some(n) = &mut self.ui.nodes {
                    for &(s, i) in &sel {
                        crate::node_edit::convert_segment(&mut n.subs, s, i, true);
                    }
                }
                if !sel.is_empty() {
                    self.nodes_commit(None);
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::C)) {
                let sel = self.ui.nodes.as_ref().map(|n| n.sel.clone()).unwrap_or_default();
                if let Some(n) = &mut self.ui.nodes {
                    for &(s, i) in &sel {
                        crate::node_edit::convert_segment(&mut n.subs, s, i, false);
                    }
                }
                if !sel.is_empty() {
                    self.nodes_commit(None);
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::B)) {
                let sel = self.ui.nodes.as_ref().map(|n| n.sel.clone()).unwrap_or_default();
                if let Some(n) = &mut self.ui.nodes {
                    for &(s, i) in &sel {
                        crate::node_edit::break_subpath(&mut n.subs, s, i);
                    }
                    n.sel.clear();
                }
                if !sel.is_empty() {
                    self.nodes_commit(None);
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::J)) {
                let sel = self.ui.nodes.as_ref().map(|n| n.sel.clone()).unwrap_or_default();
                if sel.len() == 2
                    && let Some(n) = &mut self.ui.nodes
                {
                    crate::node_edit::join_endpoints(&mut n.subs, sel.first().copied().unwrap_or((0, 0)), sel.get(1).copied().unwrap_or((0, 0)), 0.01);
                    n.sel.clear();
                }
                if sel.len() == 2 {
                    self.nodes_commit(None);
                }
            } else {
                // Arrow nudge: 0.1 mm (Shift: 1 mm), coalesced per run.
                let step = if shift { 1.0 } else { 0.1 };
                let delta = if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                    Some([-step, 0.0])
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                    Some([step, 0.0])
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                    Some([0.0, step])
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                    Some([0.0, -step])
                } else {
                    None
                };
                if let Some(d) = delta
                    && !self.ui.nodes.as_ref().is_none_or(|n| n.sel.is_empty())
                {
                    let v = lin(&inv, d);
                    if let Some(n) = &mut self.ui.nodes {
                        crate::node_edit::move_nodes(&mut n.subs, &n.sel.clone(), v);
                    }
                    self.nodes_commit(Some("nodes-nudge"));
                }
                let _ = cmd;
            }
        }

        // ---- double-click on a segment inserts a node -------------------------------------------------------
        if response.double_clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = view.screen_to_world(pos);
            let tol_px = 6.0 / view.zoom;
            if let Some((s, i, t)) = self.nearest_segment(&xform, world, tol_px) {
                if let Some(n) = &mut self.ui.nodes {
                    crate::node_edit::insert_node(&mut n.subs, s, i, t);
                    n.sel = vec![(s, i)];
                }
                self.nodes_commit(None);
                return;
            }
        }

        // ---- press / drag / release --------------------------------------------------------------------------
        let Some(n_ref) = self.ui.nodes.as_ref() else { return };
        if response.drag_started_by(egui::PointerButton::Primary) {
            let press = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
            if let Some(press) = press {
                let world = view.screen_to_world(press);
                // Handle hit first (6 px screen).
                let handle = n_ref.subs.iter().enumerate().find_map(|(si, sp)| {
                    sp.nodes.iter().enumerate().find_map(|(i, node)| {
                        for (inn, h) in [(true, node.inn), (false, node.out)] {
                            if let Some(h) = h {
                                let hw = xform * h;
                                let c = view.world_to_screen([hw.x, hw.y]);
                                if press.distance(c) <= 6.0 {
                                    return Some((si, i, inn));
                                }
                            }
                        }
                        None
                    })
                });
                if let Some((si, i, inn)) = handle {
                    if let Some(n) = self.ui.nodes.as_mut() {
                        n.drag = Some(crate::app::NodeDrag::Handle { sub: si, idx: i, inn });
                    }
                } else {
                    let hit = self.node_hit(&xform, view, press);
                    if let Some((si, i)) = hit {
                        if let Some(n) = self.ui.nodes.as_mut() {
                            let was = n.sel.contains(&(si, i));
                            if ui.input(|i2| i2.modifiers.shift) {
                                if !was {
                                    n.sel.push((si, i));
                                }
                            } else if !was {
                                n.sel = vec![(si, i)];
                            }
                            n.drag = Some(crate::app::NodeDrag::Move { last: world });
                        }
                    } else if let Some(n) = self.ui.nodes.as_mut() {
                        n.marquee = Some(world);
                    }
                }
            }
        }
        if let Some(n) = &mut self.ui.nodes {
            match n.drag {
                Some(crate::app::NodeDrag::Move { last }) => {
                    if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                        let world = view.screen_to_world(pos);
                        let dw = [world[0] - last[0], world[1] - last[1]];
                        if dw[0] != 0.0 || dw[1] != 0.0 {
                            let dv = lin(&inv, dw);
                            let sel = n.sel.clone();
                            crate::node_edit::move_nodes(&mut n.subs, &sel, dv);
                            n.drag = Some(crate::app::NodeDrag::Move { last: world });
                            self.ui.status_message = format!("Node move: {:.2}, {:.2} mm", dv.x, dv.y);
                        }
                    }
                }
                Some(crate::app::NodeDrag::Handle { sub, idx, inn }) => {
                    if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                        let world = view.screen_to_world(pos);
                        let op = inv * openlaser_geom::Point::new(world[0], world[1]);
                        crate::node_edit::move_handle(&mut n.subs, sub, idx, inn, op);
                    }
                }
                None => {}
            }
            // Marquee updates.
            if let Some(from) = n.marquee
                && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
            {
                let w = view.screen_to_world(pos);
                let rect = egui::Rect::from_two_pos(view.world_to_screen(from), pos);
                let shift = ui.input(|i| i.modifiers.shift);
                if !shift {
                    n.sel.clear();
                }
                for (si, sp) in n.subs.iter().enumerate() {
                    for (i, node) in sp.nodes.iter().enumerate() {
                        let wp = xform * node.p;
                        if rect.contains(view.world_to_screen([wp.x, wp.y])) && !n.sel.contains(&(si, i)) {
                            n.sel.push((si, i));
                        }
                    }
                }
                let _ = w;
            }
        }
        if response.drag_stopped() {
            let was_drag = if let Some(n) = self.ui.nodes.as_mut() {
                let was_drag = n.drag.is_some();
                n.drag = None;
                n.marquee = None;
                was_drag
            } else {
                false
            };
            if was_drag {
                self.nodes_commit(None);
            }
        }
        // A plain click (no drag) on a node toggles selection only; an empty click clears it.
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let press = pos;
            if let Some((si, i)) = self.node_hit(&xform, view, press) {
                if let Some(n) = self.ui.nodes.as_mut() {
                    if ui.input(|i2| i2.modifiers.shift) {
                        if let Some(at) = n.sel.iter().position(|x| *x == (si, i)) {
                            n.sel.remove(at);
                        } else {
                            n.sel.push((si, i));
                        }
                    } else {
                        n.sel = vec![(si, i)];
                    }
                }
            } else if !ui.input(|i| i.modifiers.shift)
                && let Some(n) = self.ui.nodes.as_mut()
            {
                n.sel.clear();
            }
        }
    }

    /// Discard the live edits (Esc mid-drag): re-sync drops `subs` back to the committed path.
    fn nodes_sync_forget(&mut self) {
        let Some(n) = &mut self.ui.nodes else { return };
        n.drag = None;
        n.marquee = None;
        // Reload from the document so a cancelled drag leaves no residue.
        let id = n.id;
        if let Some(project) = self.project()
            && let Some(openlaser_doc::Geom::Path(path)) = project.shape(openlaser_types::ObjectId(id)).map(|s| &s.geom)
        {
            let subs = crate::node_edit::to_nodes(path);
            if let Some(n) = self.ui.nodes.as_mut() {
                n.subs = subs;
            }
        }
    }

    /// The node nearest `press` (screen space), within 6 px.
    fn node_hit(&self, xform: &openlaser_doc::Affine, view: &ViewXform, press: egui::Pos2) -> Option<(usize, usize)> {
        let n = self.ui.nodes.as_ref()?;
        n.subs.iter().enumerate().find_map(|(si, sp)| {
            sp.nodes.iter().enumerate().find_map(|(i, node)| {
                let w = *xform * node.p;
                if press.distance(view.world_to_screen([w.x, w.y])) <= 6.0 { Some((si, i)) } else { None }
            })
        })
    }

    /// Nearest (subpath, node-index of the segment's end, t) to a world point, within a screen tolerance.
    /// `i == nodes.len()` addresses a closed subpath's closing segment.
    fn nearest_segment(&self, xform: &openlaser_doc::Affine, world: [f64; 2], tol_world: f64) -> Option<(usize, usize, f64)> {
        let n = self.ui.nodes.as_ref()?;
        let mut best: Option<(usize, usize, f64, f64)> = None;
        for (si, sp) in n.subs.iter().enumerate() {
            let count = sp.nodes.len();
            let ends = if sp.closed { count + 1 } else { count };
            for i in 1..ends {
                let Some((a, b)) = sp.nodes.get(i - 1).zip(sp.nodes.get(i % count)) else { continue };
                let (a, b) = (a.clone(), b.clone());
                let seg = |t: f64| -> [f64; 2] {
                    if b.line_in {
                        let p = a.p + (b.p - a.p) * t;
                        let w = *xform * p;
                        [w.x, w.y]
                    } else {
                        let h0 = a.out.unwrap_or(a.p);
                        let h1 = b.inn.unwrap_or(b.p);
                        let q = a.p + (h0 - a.p) * t;
                        let r = h0 + (h1 - h0) * t;
                        let s2 = h1 + (b.p - h1) * t;
                        let q2 = q + (r - q) * t;
                        let r2 = r + (s2 - r) * t;
                        let m = q2 + (r2 - q2) * t;
                        let w = *xform * m;
                        [w.x, w.y]
                    }
                };
                // Coarse sample then refine around the best t.
                let (mut bt, mut bd) = (0.0, f64::MAX);
                for k in 0..=24 {
                    let t = k as f64 / 24.0;
                    let p = seg(t);
                    let d = (p[0] - world[0]).hypot(p[1] - world[1]);
                    if d < bd {
                        bd = d;
                        bt = t;
                    }
                }
                let (mut lo, mut hi) = ((bt - 1.0 / 24.0).max(0.0), (bt + 1.0 / 24.0).min(1.0));
                for _ in 0..16 {
                    let m1 = lo + (hi - lo) / 3.0;
                    let m2 = hi - (hi - lo) / 3.0;
                    let d1 = {
                        let p = seg(m1);
                        (p[0] - world[0]).hypot(p[1] - world[1])
                    };
                    let d2 = {
                        let p = seg(m2);
                        (p[0] - world[0]).hypot(p[1] - world[1])
                    };
                    if d1 < d2 { hi = m2 } else { lo = m1 }
                }
                let p = seg((lo + hi) / 2.0);
                let d = (p[0] - world[0]).hypot(p[1] - world[1]);
                if d < bd {
                    bd = d;
                    bt = (lo + hi) / 2.0;
                }
                if bd <= tol_world && best.is_none_or(|(_, _, _, dd)| bd < dd) {
                    best = Some((si, i, bt, bd));
                }
            }
        }
        best.map(|(s, i, t, _)| (s, i, t))
    }

    /// Selection interaction on the Select tool: move drags from a shape/box press, handle drags scale and
    /// rotate, the marquee from an empty press, clicks select/deselect — and a click on an already-selected
    /// shape toggles LightBurn's scale↔rotate handles. A transform drag only previews; release dispatches
    /// exactly one engine command (one undo step).
    fn canvas_selection(&mut self, ui: &egui::Ui, response: &egui::Response, view: &ViewXform) {
        if self.ui.selected_tool != "Select" || ui.input(|i| i.key_down(egui::Key::Space)) {
            self.view.marquee_from = None;
            self.view.sel_drag = None;
            return;
        }
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        // Double-click a text shape: reopen the inline editor (commit becomes text.edit).
        if response.double_clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = view.screen_to_world(pos);
            if let Some(id) = self.hit_shape(view, world)
                && let Some(openlaser_doc::Geom::Text(t)) = self.project().and_then(|p| p.shape(id).map(|sh| sh.geom.clone()))
            {
                let d = &mut self.ui.text_defaults;
                d.family = t.family.clone();
                d.style = t.style.clone();
                d.height = t.height_mm;
                d.letter_spacing = t.letter_spacing_mm;
                d.line_spacing = t.line_spacing;
                d.align = match t.align {
                    openlaser_types::TextAlign::Left => 0,
                    openlaser_types::TextAlign::Center => 1,
                    openlaser_types::TextAlign::Right => 2,
                };
                self.ui.text = Some(crate::app::TextUi { anchor: world, buffer: t.text.clone(), editing: Some(id.0), request_focus: true });
                return;
            }
        }
        // Esc cancels a drag or marquee in flight: no dispatch, the selection is kept (logic skips
        // edit.selectNone while a drag exists).
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && (self.view.sel_drag.is_some() || self.view.marquee_from.is_some()) {
            self.view.sel_drag = None;
            self.view.marquee_from = None;
            self.ui.status_message = "Cancelled".to_owned();
            return;
        }
        let painter = ui.painter().with_clip_rect(view.rect);

        // ---- an in-flight transform drag: ghost + status, then one command on release -----------------------
        if self.view.sel_drag.is_some() {
            let pos = ui.input(|i| i.pointer.latest_pos());
            if let (Some(drag), Some(pos)) = (self.view.sel_drag.as_mut(), pos) {
                drag.pos = pos;
            }
            let Some(drag) = self.view.sel_drag.as_ref() else { return };
            let to_world = view.screen_to_world(drag.pos);
            let delta = [to_world[0] - drag.press_world[0], to_world[1] - drag.press_world[1]];
            let lock = shift || self.ui.lock_ratio;
            let (affine, preview_bounds, status) = match drag.kind {
                SelDragKind::Move => {
                    let a = crate::canvas_xform::move_affine(delta);
                    (a, crate::canvas_xform::bounds_of(&drag.bounds, a), format!("Move {:+.3}, {:+.3} mm", delta[0], delta[1]))
                }
                SelDragKind::Scale(i) => {
                    let new_b = crate::canvas_xform::scale_bounds(&drag.bounds, i, delta, lock, alt);
                    let a = crate::canvas_xform::scale_affine(&drag.bounds, &new_b, i, alt);
                    (a, new_b, format!("{:.3} × {:.3} mm", new_b.max[0] - new_b.min[0], new_b.max[1] - new_b.min[1]))
                }
                SelDragKind::Rotate => {
                    let c = [(drag.bounds.min[0] + drag.bounds.max[0]) / 2.0, (drag.bounds.min[1] + drag.bounds.max[1]) / 2.0];
                    let deg = crate::canvas_xform::rotate_deg(c, drag.press_world, to_world, if shift { 15.0 } else { 0.0 });
                    let a = crate::canvas_xform::rotate_affine(&drag.bounds, deg);
                    (a, crate::canvas_xform::bounds_of(&drag.bounds, a), format!("Rotate {deg:+.2}°"))
                }
            };
            let ghost_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 255));
            for line in &drag.ghost {
                let pts: Vec<egui::Pos2> = line
                    .iter()
                    .map(|p| {
                        let q = affine * openlaser_geom::Point::new(p[0], p[1]);
                        view.world_to_screen([q.x, q.y])
                    })
                    .collect();
                if pts.len() >= 2 {
                    painter.extend(egui::Shape::dashed_line(&pts, ghost_stroke, 4.0, 3.0));
                }
            }
            self.paint_box(&painter, view, &preview_bounds);
            self.ui.status_message = status;
            if response.drag_stopped() {
                let Some(drag) = self.view.sel_drag.take() else { return };
                match drag.kind {
                    SelDragKind::Move => {
                        if delta.iter().any(|d| d.abs() >= 0.001) {
                            let c = [(drag.bounds.min[0] + drag.bounds.max[0]) / 2.0 + delta[0], (drag.bounds.min[1] + drag.bounds.max[1]) / 2.0 + delta[1]];
                            self.execute_widget("arrange.transform", serde_json::json!({ "anchor": "center", "xMm": c[0], "yMm": c[1] }));
                        }
                    }
                    SelDragKind::Scale(i) => {
                        let new_b = crate::canvas_xform::scale_bounds(&drag.bounds, i, delta, lock, alt);
                        if (new_b.max[0] - new_b.min[0] - (drag.bounds.max[0] - drag.bounds.min[0])).abs() >= 0.001
                            || (new_b.max[1] - new_b.min[1] - (drag.bounds.max[1] - drag.bounds.min[1])).abs() >= 0.001
                        {
                            let anchor = if alt { "center" } else { crate::canvas_xform::anchor_of(crate::canvas_xform::opposite(i)) };
                            // An edge handle moves one axis only; corners send both.
                            let mut params = serde_json::json!({ "anchor": anchor });
                            if let Some(map) = params.as_object_mut() {
                                match crate::canvas_xform::edge_axis(i) {
                                    Some(axis) => {
                                        map.insert(
                                            if axis == 0 { "wMm".to_owned() } else { "hMm".to_owned() },
                                            serde_json::json!(new_b.max.get(axis).unwrap_or(&0.0) - new_b.min.get(axis).unwrap_or(&0.0)),
                                        );
                                    }
                                    None => {
                                        map.insert("wMm".to_owned(), serde_json::json!(new_b.max[0] - new_b.min[0]));
                                        map.insert("hMm".to_owned(), serde_json::json!(new_b.max[1] - new_b.min[1]));
                                    }
                                }
                            }
                            self.execute_widget("arrange.transform", params);
                        }
                    }
                    SelDragKind::Rotate => {
                        let c = [(drag.bounds.min[0] + drag.bounds.max[0]) / 2.0, (drag.bounds.min[1] + drag.bounds.max[1]) / 2.0];
                        let deg = crate::canvas_xform::rotate_deg(c, drag.press_world, to_world, if shift { 15.0 } else { 0.0 });
                        if deg.abs() >= 0.01 {
                            self.execute_widget("arrange.rotateBy", serde_json::json!({ "deg": deg, "anchor": "center" }));
                        }
                    }
                }
            }
            return;
        }

        // ---- box, handles, cursors on the static selection ----------------------------------------------------
        let bounds = self.selection_bounds();
        if let Some(b) = &bounds {
            self.paint_box(&painter, view, b);
            let indices: &[usize] = if self.view.rotate_handles { &crate::canvas_xform::CORNERS } else { &[0, 1, 2, 3, 4, 5, 6, 7] };
            for i in indices {
                let p = view.world_to_screen(crate::canvas_xform::handle_point(b, *i));
                let rect = egui::Rect::from_center_size(p, egui::vec2(HANDLE_SIZE, HANDLE_SIZE));
                if self.view.rotate_handles {
                    painter.circle_stroke(p, HANDLE_SIZE / 2.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(70, 160, 230)));
                } else {
                    painter.rect_filled(rect, 1.0, egui::Color32::WHITE);
                    painter.rect_stroke(rect, 1.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 130, 220)), egui::StrokeKind::Middle);
                }
            }
            if let Some(hover) = ui.input(|i| i.pointer.hover_pos()).filter(|p| view.rect.contains(*p)) {
                if let Some(i) = self.handle_hit(hover, b, view) {
                    ui.ctx().set_cursor_icon(if self.view.rotate_handles { egui::CursorIcon::Grab } else { resize_cursor(i) });
                } else {
                    let w = view.screen_to_world(hover);
                    let inside = w[0] >= b.min[0] && w[0] <= b.max[0] && w[1] >= b.min[1] && w[1] <= b.max[1];
                    if inside || self.hit_shape(view, w).is_some() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                    }
                }
            }
        }

        // ---- drag start: handle → shape/box → marquee ------------------------------------------------------------
        if response.drag_started_by(egui::PointerButton::Primary) {
            let press = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
            let handle = press.zip(bounds).and_then(|(p, b)| self.handle_hit(p, &b, view));
            match press {
                Some(press) if handle.is_some() => {
                    let i = handle.unwrap_or(0);
                    let kind = if self.view.rotate_handles { SelDragKind::Rotate } else { SelDragKind::Scale(i) };
                    if let Some(b) = bounds {
                        self.view.sel_drag =
                            Some(SelectionDrag { kind, press_world: view.screen_to_world(press), pos: press, bounds: b, ghost: self.selection_ghost() });
                    }
                }
                Some(press) => {
                    let world = view.screen_to_world(press);
                    let inside = bounds.is_some_and(|b| world[0] >= b.min[0] && world[0] <= b.max[0] && world[1] >= b.min[1] && world[1] <= b.max[1]);
                    let hit = if inside { None } else { self.hit_shape(view, world) };
                    if inside || hit.is_some() {
                        if let Some(id) = hit.filter(|id| !self.selection().contains(id)) {
                            // Pressing an unselected shape selects it first (Shift adds), then drags it.
                            self.execute_widget("edit.select", serde_json::json!({ "ids": [id], "mode": if shift { "add" } else { "replace" } }));
                            self.view.rotate_handles = false;
                        }
                        if let Some(b) = self.selection_bounds() {
                            self.view.sel_drag =
                                Some(SelectionDrag { kind: SelDragKind::Move, press_world: world, pos: press, bounds: b, ghost: self.selection_ghost() });
                        }
                    } else {
                        self.view.marquee_from = Some(press);
                    }
                }
                None => {}
            }
        }

        // ---- marquee --------------------------------------------------------------------------------------------
        if let (Some(from), Some(to)) = (self.view.marquee_from, ui.input(|i| i.pointer.latest_pos())) {
            let crossing = to.x < from.x;
            let rect = egui::Rect::from_two_pos(from, to);
            let color = if crossing { egui::Color32::from_rgb(80, 200, 120) } else { egui::Color32::from_rgb(90, 150, 255) };
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
                self.view.rotate_handles = false;
            }
        }

        // ---- click: select, deselect, or toggle scale↔rotate handles ---------------------------------------------
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(position) = response.interact_pointer_pos()
        {
            let world = view.screen_to_world(position);
            let hit = self.hit_shape(view, world);
            // A stroke hit decides: an unselected shape inside the box is selected, not a handle toggle. Only an
            // empty spot inside the box counts as "on the selection".
            let on_selection = match hit {
                Some(id) => self.selection().contains(&id),
                None => bounds.is_some_and(|b| world[0] >= b.min[0] && world[0] <= b.max[0] && world[1] >= b.min[1] && world[1] <= b.max[1]),
            };
            match (hit, shift) {
                // A click on an already-selected object (stroke or inside its box) toggles the handles.
                _ if !shift && on_selection => self.view.rotate_handles = !self.view.rotate_handles,
                (None, false) => {
                    self.invoke_ui_action("edit.selectNone");
                    self.view.rotate_handles = false;
                }
                (None, true) => {}
                (Some(id), _) => {
                    self.execute_widget("edit.select", serde_json::json!({ "ids": [id], "mode": if shift { "toggle" } else { "replace" } }));
                    self.view.rotate_handles = false;
                }
            }
        }
    }

    /// The selection's bounding box above the shapes.
    fn paint_box(&self, painter: &egui::Painter, view: &ViewXform, b: &openlaser_types::Bounds) {
        let rect = egui::Rect::from_two_pos(view.world_to_screen(b.min), view.world_to_screen(b.max));
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 130, 220)), egui::StrokeKind::Middle);
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

/// The linear part of an affine applied to a delta (translation ignored).
fn lin(inv: &openlaser_doc::Affine, d: [f64; 2]) -> openlaser_geom::Vec2 {
    let c = inv.as_coeffs();
    openlaser_geom::Vec2::new(c[0] * d[0] + c[2] * d[1], c[1] * d[0] + c[3] * d[1])
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

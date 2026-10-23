//! Canvas coordinate transforms and native painter renderer (UI §5.3).

use openlaser_render::{DisplayList, Item, Rgba, Stroke};

pub struct ViewXform {
    /// Points per millimetre.
    pub zoom: f64,
    /// World-space centre, in millimetres; Y points up.
    pub center: [f64; 2],
    pub rect: egui::Rect,
}

impl ViewXform {
    pub fn world_to_screen(&self, point: [f64; 2]) -> egui::Pos2 {
        let scale = finite_positive(self.zoom).unwrap_or(1.0) as f32;
        egui::pos2(self.rect.center().x + ((point[0] - self.center[0]) as f32 * scale), self.rect.center().y - ((point[1] - self.center[1]) as f32 * scale))
    }

    pub fn screen_to_world(&self, point: egui::Pos2) -> [f64; 2] {
        let scale = finite_positive(self.zoom).unwrap_or(1.0);
        [self.center[0] + f64::from(point.x - self.rect.center().x) / scale, self.center[1] - f64::from(point.y - self.rect.center().y) / scale]
    }
}

/// Returns the first path whose nearest segment is within `tolerance` millimetres.
/// Invalid points and tolerances are ignored; geometry from a malformed display list cannot crash hit testing.
pub fn hit_test(paths: &[Vec<[f64; 2]>], point: [f64; 2], tolerance: f64) -> Option<usize> {
    if !point.iter().all(|v| v.is_finite()) || !tolerance.is_finite() || tolerance < 0.0 {
        return None;
    }
    paths.iter().position(|path| {
        path.windows(2).any(|pair| {
            let Some((a, b)) = pair.first().zip(pair.get(1)) else { return false };
            let (a, b) = (*a, *b);
            if !a.iter().chain(b.iter()).all(|v| v.is_finite()) {
                return false;
            }
            distance_to_segment(point, a, b) <= tolerance
        })
    })
}

/// Marquee selection in design coordinates. Left-to-right (`crossing == false`) encloses every point;
/// right-to-left (`crossing == true`) selects any path that intersects the rectangle.
pub fn marquee_hits(paths: &[Vec<[f64; 2]>], rect: [[f64; 2]; 2], crossing: bool) -> Vec<usize> {
    if !rect.iter().flatten().all(|v| v.is_finite()) {
        return Vec::new();
    }
    let min = [rect[0][0].min(rect[1][0]), rect[0][1].min(rect[1][1])];
    let max = [rect[0][0].max(rect[1][0]), rect[0][1].max(rect[1][1])];
    paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let points: Vec<_> = path.iter().copied().filter(|p| p.iter().all(|v| v.is_finite())).collect();
            let hit = if crossing {
                points.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| segment_intersects_rect(*a, *b, min, max)))
                    || points.iter().any(|p| in_rect(*p, min, max))
            } else {
                !points.is_empty() && points.iter().all(|p| in_rect(*p, min, max))
            };
            hit.then_some(index)
        })
        .collect()
}

fn in_rect(p: [f64; 2], min: [f64; 2], max: [f64; 2]) -> bool {
    p[0] >= min[0] && p[0] <= max[0] && p[1] >= min[1] && p[1] <= max[1]
}

fn segment_intersects_rect(a: [f64; 2], b: [f64; 2], min: [f64; 2], max: [f64; 2]) -> bool {
    if !a.iter().chain(b.iter()).all(|v| v.is_finite()) {
        return false;
    }
    if in_rect(a, min, max) || in_rect(b, min, max) {
        return true;
    }
    let bottom_left = [min[0], min[1]];
    let bottom_right = [max[0], min[1]];
    let top_right = [max[0], max[1]];
    let top_left = [min[0], max[1]];
    [(bottom_left, bottom_right), (bottom_right, top_right), (top_right, top_left), (top_left, bottom_left)]
        .iter()
        .any(|(c, d)| segments_intersect(a, b, *c, *d))
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (ab_c, ab_d, cd_a, cd_b) = (cross(a, b, c), cross(a, b, d), cross(c, d, a), cross(c, d, b));
    let opposite = |left: f64, right: f64| (left < 0.0 && right > 0.0) || (left > 0.0 && right < 0.0);
    if opposite(ab_c, ab_d) && opposite(cd_a, cd_b) {
        return true;
    }
    let on_segment =
        |p: [f64; 2], q: [f64; 2], r: [f64; 2]| q[0] >= p[0].min(r[0]) && q[0] <= p[0].max(r[0]) && q[1] >= p[1].min(r[1]) && q[1] <= p[1].max(r[1]);
    (ab_c == 0.0 && on_segment(a, c, b)) || (ab_d == 0.0 && on_segment(a, d, b)) || (cd_a == 0.0 && on_segment(c, a, d)) || (cd_b == 0.0 && on_segment(c, b, d))
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let length2 = delta[0] * delta[0] + delta[1] * delta[1];
    let t = if length2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * delta[0] + (p[1] - a[1]) * delta[1]) / length2).clamp(0.0, 1.0) };
    ((p[0] - (a[0] + t * delta[0])).powi(2) + (p[1] - (a[1] + t * delta[1])).powi(2)).sqrt()
}

pub trait CanvasRenderer {
    fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform);
}

/// Stateless renderer: hairline geometry is painted in screen points, preserving a one-pixel outline as zoom changes.
#[derive(Debug, Default)]
pub struct PainterRenderer;

impl CanvasRenderer for PainterRenderer {
    fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform) {
        let clip = painter.with_clip_rect(view.rect);
        for item in &list.items {
            match item {
                Item::Polylines { stroke, color, lines, .. } => {
                    let stroke = stroke_for(*stroke, *color);
                    for line in lines.iter() {
                        let points: Vec<egui::Pos2> = line.0.iter().map(|p| view.world_to_screen(*p)).collect();
                        if points.len() >= 2 {
                            clip.add(egui::Shape::line(points, stroke));
                        }
                    }
                }
                Item::Segments { color, segs } => {
                    let stroke = egui::Stroke::new(1.0, rgba(*color));
                    for segment in segs.iter() {
                        clip.line_segment([view.world_to_screen(segment[0]), view.world_to_screen(segment[1])], stroke);
                    }
                }
                Item::Rect { stroke, color, rect } => {
                    let min = view.world_to_screen(rect.min);
                    let max = view.world_to_screen(rect.max);
                    let screen = egui::Rect::from_two_pos(min, max);
                    clip.rect_stroke(screen, 0.0, stroke_for(*stroke, *color), egui::StrokeKind::Inside);
                }
                Item::Marker { kind, at, color } => {
                    let p = view.world_to_screen(*at);
                    let radius = match kind {
                        openlaser_render::MarkerKind::Refusal => 5.0,
                        _ => 3.5,
                    };
                    clip.circle_filled(p, radius, rgba(*color));
                    clip.circle_stroke(p, radius + 1.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
                }
                Item::Image { rect, gray, px } => {
                    let Some(count) = usize::try_from(px[0]).ok().and_then(|w| usize::try_from(px[1]).ok().and_then(|h| w.checked_mul(h))) else { continue };
                    if count != gray.len() || px[0] == 0 || px[1] == 0 {
                        continue;
                    }
                    let pixels: Vec<egui::Color32> = gray.iter().map(|g| egui::Color32::from_gray(*g)).collect();
                    let image = egui::ColorImage::new([px[0] as usize, px[1] as usize], pixels);
                    let texture = painter.ctx().load_texture("openlaser-canvas-image", image, egui::TextureOptions::LINEAR);
                    let screen = egui::Rect::from_two_pos(view.world_to_screen(rect.min), view.world_to_screen(rect.max));
                    clip.image(texture.id(), screen, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
            }
        }
    }
}

/// Paint a list centered and fitted into a thumbnail rectangle.
pub fn paint_fit(painter: &egui::Painter, list: &DisplayList, rect: egui::Rect) {
    let Some(bounds) = bounds(list) else { return };
    let width = (bounds.max[0] - bounds.min[0]).max(0.001);
    let height = (bounds.max[1] - bounds.min[1]).max(0.001);
    let zoom = (f64::from(rect.width().max(1.0)) / width).min(f64::from(rect.height().max(1.0)) / height) * 0.9;
    let view = ViewXform { zoom, center: [(bounds.min[0] + bounds.max[0]) * 0.5, (bounds.min[1] + bounds.max[1]) * 0.5], rect };
    PainterRenderer.paint(painter, list, &view);
}

fn bounds(list: &DisplayList) -> Option<openlaser_types::Bounds> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut has_points = false;
    let mut add = |p: [f64; 2]| {
        if p.iter().all(|v| v.is_finite()) {
            min[0] = min[0].min(p[0]);
            min[1] = min[1].min(p[1]);
            max[0] = max[0].max(p[0]);
            max[1] = max[1].max(p[1]);
            has_points = true;
        }
    };
    for item in &list.items {
        match item {
            Item::Polylines { lines, .. } => {
                for line in lines.iter() {
                    for point in &line.0 {
                        add(*point);
                    }
                }
            }
            Item::Segments { segs, .. } => {
                for segment in segs.iter() {
                    add(segment[0]);
                    add(segment[1]);
                }
            }
            Item::Rect { rect, .. } | Item::Image { rect, .. } => {
                add(rect.min);
                add(rect.max);
            }
            Item::Marker { at, .. } => add(*at),
        }
    }
    has_points.then_some(openlaser_types::Bounds { min, max })
}

fn rgba(c: Rgba) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

fn stroke_for(stroke: Stroke, color: Rgba) -> egui::Stroke {
    let width = match stroke {
        Stroke::Hairline | Stroke::Dashed => 1.0,
        Stroke::Width(w) if w.is_finite() => w.max(0.1),
        Stroke::Width(_) => 1.0,
    };
    egui::Stroke::new(width, rgba(color))
}

fn finite_positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_view_xform_roundtrip() {
        let view = ViewXform { zoom: 2.5, center: [12.0, -7.0], rect: egui::Rect::from_min_size(egui::pos2(11.0, 29.0), egui::vec2(600.0, 420.0)) };
        let world = [-13.25, 4.5];
        let restored = view.screen_to_world(view.world_to_screen(world));
        assert!((restored[0] - world[0]).abs() < 1e-5);
        assert!((restored[1] - world[1]).abs() < 1e-5);
    }

    #[test]
    fn hit_test_and_marquee() {
        let paths = vec![vec![[-3.0, 0.0], [3.0, 0.0]], vec![[2.0, 2.0], [3.0, 3.0]]];
        assert_eq!(hit_test(&paths, [0.0, 0.2], 0.25), Some(0));
        assert_eq!(hit_test(&paths, [0.0, 0.3], 0.25), None);
        assert_eq!(marquee_hits(&paths, [[-1.0, -1.0], [1.0, 1.0]], false), Vec::<usize>::new());
        assert_eq!(marquee_hits(&paths, [[-1.0, -1.0], [1.0, 1.0]], true), vec![0]);
        assert_eq!(marquee_hits(&paths, [[5.0, 5.0], [6.0, 6.0]], true), Vec::<usize>::new());
        assert_eq!(marquee_hits(&[vec![[2.0, 0.0], [3.0, 0.0]]], [[-1.0, -1.0], [1.0, 1.0]], true), Vec::<usize>::new());
    }
}

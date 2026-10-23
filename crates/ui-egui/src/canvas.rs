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

/// Bounded texture cache and zoom-dependent mesh cache. Panning translates cached vertices.
#[derive(Default)]
pub struct PainterRenderer {
    textures: Vec<(std::sync::Arc<[u8]>, [u32; 2], egui::TextureHandle)>,
    mesh_key: Option<(DisplayList, f64)>,
    mesh: egui::epaint::Mesh,
}

fn add_segment(mesh: &mut egui::epaint::Mesh, a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke) {
    let vector = b - a;
    if !a.is_finite() || !b.is_finite() || vector.length_sq() <= f32::EPSILON {
        return;
    }
    let n = egui::vec2(-vector.y, vector.x).normalized() * stroke.width * 0.5;
    let Ok(base) = u32::try_from(mesh.vertices.len()) else { return };
    if base > u32::MAX - 4 {
        return;
    }
    for p in [a + n, b + n, b - n, a - n] {
        mesh.colored_vertex(p, stroke.color);
    }
    mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn add_line(mesh: &mut egui::epaint::Mesh, points: &[egui::Pos2], stroke: egui::Stroke, dashed: bool) {
    for pair in points.windows(2) {
        let Some((&a, &b)) = pair.first().zip(pair.get(1)) else { continue };
        if dashed {
            let length = a.distance(b);
            // A pathological display list must not generate unbounded dash geometry.
            if !length.is_finite() || length > 100_000.0 {
                continue;
            }
            let direction = (b - a).normalized();
            let mut distance = 0.0;
            while distance < length {
                add_segment(mesh, a + direction * distance, a + direction * (distance + 6.0).min(length), stroke);
                distance += 10.0;
            }
        } else {
            add_segment(mesh, a, b, stroke);
        }
    }
}

impl CanvasRenderer for PainterRenderer {
    fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform) {
        let clip = painter.with_clip_rect(view.rect);
        if self.mesh_key.as_ref().is_none_or(|(cached, zoom)| cached != list || *zoom != view.zoom) {
            self.mesh.clear();
            let base = ViewXform { zoom: view.zoom, center: [0.0; 2], rect: egui::Rect::from_center_size(egui::Pos2::ZERO, egui::Vec2::ZERO) };
            for item in &list.items {
                match item {
                    Item::Polylines { stroke, color, lines, .. } => {
                        for line in lines.iter() {
                            let points: Vec<_> = line.0.iter().map(|p| base.world_to_screen(*p)).collect();
                            add_line(&mut self.mesh, &points, stroke_for(*stroke, *color), matches!(stroke, Stroke::Dashed));
                        }
                    }
                    Item::Segments { color, segs } => {
                        for segment in segs.iter() {
                            add_segment(
                                &mut self.mesh,
                                base.world_to_screen(segment[0]),
                                base.world_to_screen(segment[1]),
                                egui::Stroke::new(1.0, rgba(*color)),
                            );
                        }
                    }
                    Item::Rect { stroke, color, rect } => {
                        let a = base.world_to_screen(rect.min);
                        let b = base.world_to_screen(rect.max);
                        add_line(
                            &mut self.mesh,
                            &[a, egui::pos2(b.x, a.y), b, egui::pos2(a.x, b.y), a],
                            stroke_for(*stroke, *color),
                            matches!(stroke, Stroke::Dashed),
                        );
                    }
                    _ => {}
                }
            }
            self.mesh_key = Some((list.clone(), view.zoom));
        }
        if !self.mesh.is_empty() {
            let mut mesh = self.mesh.clone();
            mesh.translate(view.world_to_screen([0.0; 2]).to_vec2());
            clip.add(egui::Shape::mesh(mesh));
        }
        for item in &list.items {
            match item {
                Item::Marker { kind, at, color } => {
                    let p = view.world_to_screen(*at);
                    let radius = if matches!(kind, openlaser_render::MarkerKind::Refusal) { 5.0 } else { 3.5 };
                    clip.circle_filled(p, radius, rgba(*color));
                }
                Item::Image { rect, gray, px } => {
                    let limit = painter.ctx().input(|i| i.max_texture_side).min(2048);
                    let Some(count) = usize::try_from(px[0]).ok().and_then(|w| usize::try_from(px[1]).ok().and_then(|h| w.checked_mul(h))) else { continue };
                    if count != gray.len() || px.contains(&0) || px.iter().any(|side| *side as usize > limit) {
                        continue;
                    }
                    let index = self.textures.iter().position(|(data, size, _)| std::sync::Arc::ptr_eq(data, gray) && size == px);
                    let index = match index {
                        Some(index) => index,
                        None => {
                            if self.textures.len() >= 8 {
                                let _ = self.textures.remove(0);
                            }
                            let pixels = gray.iter().map(|g| egui::Color32::from_gray(*g)).collect();
                            let texture = painter.ctx().load_texture(
                                "openlaser-canvas-image",
                                egui::ColorImage::new([px[0] as usize, px[1] as usize], pixels),
                                egui::TextureOptions::LINEAR,
                            );
                            self.textures.push((std::sync::Arc::clone(gray), *px, texture));
                            self.textures.len() - 1
                        }
                    };
                    if let Some((_, _, texture)) = self.textures.get(index) {
                        clip.image(
                            texture.id(),
                            egui::Rect::from_two_pos(view.world_to_screen(rect.min), view.world_to_screen(rect.max)),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                }
                _ => {}
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
    PainterRenderer::default().paint(painter, list, &view);
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
    fn renderer_caches_textures_meshes_and_bounds_images() {
        let image = std::sync::Arc::<[u8]>::from(vec![80_u8; 4]);
        let list = DisplayList {
            items: vec![
                Item::Image { rect: openlaser_types::Bounds { min: [0.0; 2], max: [2.0; 2] }, gray: image, px: [2, 2] },
                Item::Rect { stroke: Stroke::Dashed, color: Rgba([255; 4]), rect: openlaser_types::Bounds { min: [0.0; 2], max: [20.0; 2] } },
            ],
        };
        let mut renderer = PainterRenderer::default();
        let ctx = egui::Context::default();
        let view = ViewXform { zoom: 2.0, center: [0.0; 2], rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 500.0)) };
        let mut first = ctx.run_ui(egui::RawInput::default(), |ui| renderer.paint(ui.painter(), &list, &view));
        first.textures_delta.clear();
        assert_eq!(renderer.textures.len(), 1);
        let texture = renderer.textures[0].2.id();
        let mesh = renderer.mesh.clone();
        assert!(mesh.vertices.len() > 16, "dashed rectangle must contain separate dash quads");
        let moved = ViewXform { center: [10.0, 20.0], ..view };
        let mut next = ctx.run_ui(egui::RawInput::default(), |ui| renderer.paint(ui.painter(), &list, &moved));
        let no_upload = next.textures_delta.set.is_empty();
        next.textures_delta.clear();
        assert!(no_upload, "texture was uploaded again");
        assert_eq!(renderer.textures[0].2.id(), texture);
        assert_eq!(renderer.mesh, mesh, "pan rebuilt cached geometry");
        let invalid = DisplayList {
            items: vec![Item::Image { rect: openlaser_types::Bounds { min: [0.0; 2], max: [2.0; 2] }, gray: vec![0; 4096].into(), px: [4096, 1] }],
        };
        let mut next = ctx.run_ui(egui::RawInput::default(), |ui| renderer.paint(ui.painter(), &invalid, &moved));
        next.textures_delta.clear();
        assert_eq!(renderer.textures.len(), 1);
    }

    #[test]
    #[ignore = "S1 perf: Luiz runs in release on the benchmark machine"]
    fn s1_large_display_list() {
        let list = DisplayList {
            items: vec![Item::Segments {
                color: Rgba([255; 4]),
                segs: (0..200_000)
                    .map(|i| {
                        let x = f64::from(i % 1000);
                        [[x, 0.0], [x, 1.0]]
                    })
                    .collect::<Vec<_>>()
                    .into(),
            }],
        };
        let ctx = egui::Context::default();
        let mut renderer = PainterRenderer::default();
        let view = ViewXform { zoom: 1.0, center: [0.0; 2], rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0)) };
        let started = std::time::Instant::now();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| renderer.paint(ui.painter(), &list, &view));
        output.textures_delta.clear();
        assert_eq!(renderer.mesh.indices.len(), 200_000 * 6);
        eprintln!("S1: {:?}", started.elapsed());
    }

    #[test]
    #[ignore = "S2 perf: Luiz runs in release on the benchmark machine"]
    fn s2_cached_pan() {
        let ctx = egui::Context::default();
        let mut renderer = PainterRenderer::default();
        let list = DisplayList {
            items: vec![Item::Rect { stroke: Stroke::Hairline, color: Rgba([255; 4]), rect: openlaser_types::Bounds { min: [0.0; 2], max: [200.0; 2] } }],
        };
        let started = std::time::Instant::now();
        for i in 0..120 {
            let view = ViewXform { zoom: 2.0, center: [f64::from(i), 0.0], rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0)) };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| renderer.paint(ui.painter(), &list, &view));
            output.textures_delta.clear();
        }
        eprintln!("S2: {:?}/120 frames", started.elapsed());
    }

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

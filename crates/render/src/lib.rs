//! Display lists and deterministic CPU PNG rendering (SPEC §3.10).
#![forbid(unsafe_code)]

use std::sync::Arc;

pub use openlaser_doc::Project;
pub use openlaser_geom::{P2, Polyline};
pub use openlaser_machine::MachineProfile;
pub use openlaser_plan::JobPlan;
pub use openlaser_types::{Bounds, EmittedSegment, Notice, ObjectId, Refusal};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("render too large: {0}")]
    TooLarge(String),
    #[error("invalid render request: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub [u8; 4]);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stroke {
    Hairline,
    Dashed,
    Width(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Refusal,
    Notice(u32),
    Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Polylines {
        stroke: Stroke,
        color: Rgba,
        lines: Arc<[Polyline]>,
        object: Option<ObjectId>,
    },
    /// Hatch, emission, travel.
    Segments {
        color: Rgba,
        segs: Arc<[[P2; 2]]>,
    },
    /// Field, work area, selection box.
    Rect {
        stroke: Stroke,
        color: Rgba,
        rect: Bounds,
    },
    Marker {
        kind: MarkerKind,
        at: P2,
        color: Rgba,
    },
    Image {
        rect: Bounds,
        gray: Arc<[u8]>,
        px: [u32; 2],
    },
}

/// World mm, Y up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewOpts {
    pub dark: bool,
    pub selection: Vec<ObjectId>,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
    pub show_field: bool,
    pub show_work_area: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanShow {
    pub burn: bool,
    pub travel: bool,
    pub frame: bool,
    pub order: bool,
}

pub fn document_list(p: &Project, m: Option<&MachineProfile>, o: &ViewOpts) -> DisplayList {
    let mut list = DisplayList::default();
    for shape in &p.shapes {
        let color = layer_color(shape.layer);
        match &shape.geom {
            openlaser_doc::Geom::Path(path) => {
                let lines = openlaser_geom::flatten(path, openlaser_geom::DEFAULT_FLATTEN_TOL_MM)
                    .into_iter()
                    .map(|line| line.transformed(&shape.xform))
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: lines.into(), object: Some(shape.id) });
                }
            }
            openlaser_doc::Geom::Rect { w, h, .. } => {
                let points = [[0.0, 0.0], [w.get(), 0.0], [w.get(), h.get()], [0.0, h.get()], [0.0, 0.0]];
                let line = Polyline(
                    points
                        .into_iter()
                        .map(|p| {
                            let q = shape.xform * openlaser_geom::Point::new(p[0], p[1]);
                            [q.x, q.y]
                        })
                        .collect(),
                );
                list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: vec![line].into(), object: Some(shape.id) });
            }
            openlaser_doc::Geom::Ellipse { rx, ry } => {
                let points = (0..=72)
                    .map(|i| {
                        let angle = f64::from(i) * std::f64::consts::TAU / 72.0;
                        let q = shape.xform * openlaser_geom::Point::new(rx.get() * angle.cos(), ry.get() * angle.sin());
                        [q.x, q.y]
                    })
                    .collect();
                list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: vec![Polyline(points)].into(), object: Some(shape.id) });
            }
            openlaser_doc::Geom::Polygon { sides, radius } => {
                if *sides >= 3 {
                    let points = (0..=*sides)
                        .map(|i| {
                            let angle = f64::from(i) * std::f64::consts::TAU / f64::from(*sides);
                            let q = shape.xform * openlaser_geom::Point::new(radius.get() * angle.cos(), radius.get() * angle.sin());
                            [q.x, q.y]
                        })
                        .collect();
                    list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: vec![Polyline(points)].into(), object: Some(shape.id) });
                }
            }
            openlaser_doc::Geom::Bitmap(bitmap) => {
                let rect = Bounds { min: [0.0, 0.0], max: bitmap.size_mm };
                list.items.push(Item::Image { rect: transform_bounds(rect, &shape.xform), gray: bitmap.gray.clone(), px: bitmap.px });
            }
            openlaser_doc::Geom::Group(children) => {
                for child in children {
                    if let openlaser_doc::Geom::Path(path) = &child.geom {
                        let transform = shape.xform * child.xform;
                        let lines = openlaser_geom::flatten(path, openlaser_geom::DEFAULT_FLATTEN_TOL_MM)
                            .into_iter()
                            .map(|line| line.transformed(&transform))
                            .collect::<Vec<_>>();
                        if !lines.is_empty() {
                            list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: lines.into(), object: Some(shape.id) });
                        }
                    }
                }
            }
        }
    }
    if let Some(profile) = m {
        if o.show_field
            && let openlaser_machine::Kinematics::Galvo(g) = &profile.kind
        {
            let r = g.effective_work_area();
            list.items.push(Item::Rect { stroke: Stroke::Dashed, color: Rgba([120, 120, 120, 255]), rect: Bounds { min: r.min, max: r.max } });
        }
        if o.show_work_area
            && let openlaser_machine::Kinematics::Gantry(g) = &profile.kind
        {
            list.items.push(Item::Rect { stroke: Stroke::Dashed, color: Rgba([120, 120, 120, 255]), rect: Bounds { min: [0.0, 0.0], max: g.bed_mm } });
        }
    }
    for refusal in &o.refusals {
        let _ = refusal;
        list.items.push(Item::Marker { kind: MarkerKind::Refusal, at: [0.0, 0.0], color: Rgba([230, 40, 40, 255]) });
    }
    for notice in &o.notices {
        list.items.push(Item::Marker { kind: MarkerKind::Notice(notice.n), at: [0.0, 0.0], color: Rgba([230, 170, 30, 255]) });
    }
    list.items.push(Item::Marker { kind: MarkerKind::Origin, at: [0.0, 0.0], color: Rgba([220, 220, 220, 255]) });
    list
}

pub fn plan_list(plan: &JobPlan, m: &MachineProfile, show: &PlanShow) -> DisplayList {
    let mut list = DisplayList::default();
    for section in &plan.sections {
        if !show.burn {
            continue;
        }
        let color = layer_color(section.layer);
        if section.mode == openlaser_types::SectionMode::Line {
            list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: section.paths.clone().into(), object: section.objects.first().copied() });
        } else {
            let segs: Vec<[P2; 2]> = section.paths.iter().filter_map(|p| Some([*p.0.first()?, *p.0.get(1)?])).collect();
            list.items.push(Item::Segments { color, segs: segs.into() });
        }
    }
    if show.frame {
        list.items.push(Item::Polylines { stroke: Stroke::Dashed, color: Rgba([50, 160, 255, 255]), lines: plan.frame.clone().into(), object: None });
    }
    if show.travel {
        let mut segs = Vec::new();
        for section in &plan.sections {
            for path in &section.paths {
                if let (Some(a), Some(b)) = (path.0.first(), path.0.get(1)) {
                    segs.push([*a, *b]);
                }
            }
        }
        list.items.push(Item::Segments { color: Rgba([130, 130, 130, 180]), segs: segs.into() });
    }
    let _ = (m, show.order);
    list
}

pub fn emission_list(burned: &[EmittedSegment], light: &[[P2; 2]], m: &MachineProfile) -> DisplayList {
    let mut list = DisplayList::default();
    if !burned.is_empty() {
        let segs: Vec<[P2; 2]> = burned.iter().map(|e| [e.a, e.b]).collect();
        list.items.push(Item::Segments { color: Rgba([255, 50, 50, 255]), segs: segs.into() });
    }
    if !light.is_empty() {
        list.items.push(Item::Segments { color: Rgba([255, 40, 40, 120]), segs: light.to_vec().into() });
    }
    let _ = m;
    list
}

pub fn fit(list: &DisplayList, margin_frac: f64) -> Option<Bounds> {
    if !margin_frac.is_finite() || margin_frac < 0.0 {
        return None;
    }
    let mut bounds: Option<Bounds> = None;
    let mut add = |p: P2| {
        if !p[0].is_finite() || !p[1].is_finite() {
            return;
        }
        bounds = Some(match bounds {
            None => Bounds { min: p, max: p },
            Some(b) => Bounds { min: [b.min[0].min(p[0]), b.min[1].min(p[1])], max: [b.max[0].max(p[0]), b.max[1].max(p[1])] },
        });
    };
    for item in &list.items {
        match item {
            Item::Polylines { lines, .. } => {
                for line in lines.iter() {
                    for p in &line.0 {
                        add(*p);
                    }
                }
            }
            Item::Segments { segs, .. } => {
                for seg in segs.iter() {
                    add(seg[0]);
                    add(seg[1]);
                }
            }
            Item::Rect { rect, .. } | Item::Image { rect, .. } => {
                add(rect.min);
                add(rect.max);
            }
            Item::Marker { at, .. } => add(*at),
        }
    }
    bounds.map(|b| {
        let cx = (b.min[0] + b.max[0]) / 2.0;
        let cy = (b.min[1] + b.max[1]) / 2.0;
        let w = (b.max[0] - b.min[0]).max(1.0) * (1.0 + 2.0 * margin_frac);
        let h = (b.max[1] - b.min[1]).max(1.0) * (1.0 + 2.0 * margin_frac);
        Bounds { min: [cx - w / 2.0, cy - h / 2.0], max: [cx + w / 2.0, cy + h / 2.0] }
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderView {
    pub world: Bounds,
    pub max_side: u32,
    pub background: Rgba,
}

/// Deterministic CPU raster (same input → same bytes). max_side <= 2048; output <= 5 MiB or Err.
pub fn render_png(list: &DisplayList, view: &RenderView) -> Result<Vec<u8>, Error> {
    if view.max_side == 0 || view.max_side > 2048 {
        return Err(Error::TooLarge("max_side must be in 1..=2048".to_owned()));
    }
    let width = view.world.max[0] - view.world.min[0];
    let height = view.world.max[1] - view.world.min[1];
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(Error::Invalid("world bounds must be finite and non-empty".to_owned()));
    }
    let (pxw, pxh) = if width >= height {
        (view.max_side, ((f64::from(view.max_side) * height / width).round()).max(1.0) as u32)
    } else {
        (((f64::from(view.max_side) * width / height).round()).max(1.0) as u32, view.max_side)
    };
    let mut pixmap = tiny_skia::Pixmap::new(pxw, pxh).ok_or_else(|| Error::TooLarge("pixel dimensions overflow".to_owned()))?;
    let bg = view.background.0;
    pixmap.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
    let sx = f64::from(pxw) / width;
    let sy = f64::from(pxh) / height;
    for item in &list.items {
        match item {
            Item::Polylines { stroke, color, lines, .. } => {
                for line in lines.iter() {
                    draw_line(&mut pixmap, &line.0, *stroke, *color, view.world, sx, sy)?;
                }
            }
            Item::Segments { color, segs } => {
                for seg in segs.iter() {
                    draw_line(&mut pixmap, &seg[..], Stroke::Hairline, *color, view.world, sx, sy)?;
                }
            }
            Item::Rect { stroke, color, rect } => {
                let ps = [
                    [rect.min[0], rect.min[1]],
                    [rect.max[0], rect.min[1]],
                    [rect.max[0], rect.max[1]],
                    [rect.min[0], rect.max[1]],
                    [rect.min[0], rect.min[1]],
                ];
                draw_line(&mut pixmap, &ps, *stroke, *color, view.world, sx, sy)?;
            }
            Item::Marker { at, color, .. } => {
                let (x, y) = screen(*at, view.world, sx, sy);
                let mut b = tiny_skia::PathBuilder::new();
                b.push_circle(x, y, 3.0);
                if let Some(path) = b.finish() {
                    let mut paint = tiny_skia::Paint::default();
                    paint.set_color(to_color(*color));
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
                }
            }
            Item::Image { rect, gray, px } => draw_image(&mut pixmap, *rect, gray, *px, view.world, sx, sy)?,
        }
    }
    let png = pixmap.encode_png().map_err(|e| Error::Invalid(e.to_string()))?;
    if png.len() > 5 * 1024 * 1024 {
        return Err(Error::TooLarge("encoded PNG exceeds 5 MiB".to_owned()));
    }
    Ok(png)
}

fn layer_color(layer: openlaser_types::LayerKey) -> Rgba {
    match layer {
        openlaser_types::LayerKey::Color(i) => {
            let h = u32::from(i) * 137;
            Rgba([
                u8::try_from(80 + (h % 160)).unwrap_or(200),
                u8::try_from(80 + ((h / 7) % 160)).unwrap_or(200),
                u8::try_from(80 + ((h / 13) % 160)).unwrap_or(200),
                255,
            ])
        }
        openlaser_types::LayerKey::Tool(1) => Rgba([255, 60, 60, 255]),
        _ => Rgba([70, 170, 255, 255]),
    }
}
fn transform_bounds(rect: Bounds, a: &openlaser_geom::Affine) -> Bounds {
    let ps = [[rect.min[0], rect.min[1]], [rect.min[0], rect.max[1]], [rect.max[0], rect.min[1]], [rect.max[0], rect.max[1]]].map(|p| {
        let q = *a * openlaser_geom::Point::new(p[0], p[1]);
        [q.x, q.y]
    });
    Bounds {
        min: [ps.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min), ps.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min)],
        max: [ps.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max), ps.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max)],
    }
}
fn to_color(c: Rgba) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.0[0], c.0[1], c.0[2], c.0[3])
}
fn screen(p: P2, w: Bounds, sx: f64, sy: f64) -> (f32, f32) {
    (((p[0] - w.min[0]) * sx) as f32, ((w.max[1] - p[1]) * sy) as f32)
}
fn draw_line(pix: &mut tiny_skia::Pixmap, points: &[P2], stroke: Stroke, color: Rgba, w: Bounds, sx: f64, sy: f64) -> Result<(), Error> {
    if points.len() < 2 {
        return Ok(());
    }
    let mut b = tiny_skia::PathBuilder::new();
    for (i, p) in points.iter().enumerate() {
        let (x, y) = screen(*p, w, sx, sy);
        if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) }
    }
    let Some(path) = b.finish() else { return Ok(()) };
    let width = match stroke {
        Stroke::Width(v) if v.is_finite() && v > 0.0 => v,
        Stroke::Width(_) => return Err(Error::Invalid("stroke width must be finite and positive".to_owned())),
        _ => 1.0,
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(to_color(color));
    paint.anti_alias = false;
    pix.stroke_path(&path, &paint, &tiny_skia::Stroke { width, ..tiny_skia::Stroke::default() }, tiny_skia::Transform::identity(), None);
    Ok(())
}
fn draw_image(pix: &mut tiny_skia::Pixmap, rect: Bounds, gray: &[u8], px: [u32; 2], w: Bounds, sx: f64, sy: f64) -> Result<(), Error> {
    let expected = u64::from(px[0]) * u64::from(px[1]);
    if expected != u64::try_from(gray.len()).unwrap_or(u64::MAX) {
        return Err(Error::Invalid("bitmap byte count does not match dimensions".to_owned()));
    }
    let (x0, y1) = screen([rect.min[0], rect.min[1]], w, sx, sy);
    let (x1, y0) = screen([rect.max[0], rect.max[1]], w, sx, sy);
    let left = x0.max(0.0).floor() as u32;
    let right = x1.min(pix.width() as f32).ceil() as u32;
    let top = y0.max(0.0).floor() as u32;
    let bottom = y1.min(pix.height() as f32).ceil() as u32;
    let pix_width = pix.width() as usize;
    let data = pix.data_mut();
    for y in top..bottom {
        for x in left..right {
            let u = ((x as f32 - x0) / (x1 - x0).max(1.0)).clamp(0.0, 0.999999);
            let v = ((y as f32 - y0) / (y1 - y0).max(1.0)).clamp(0.0, 0.999999);
            let ix = (u * px[0] as f32) as usize;
            let iy = (v * px[1] as f32) as usize;
            let Some(g) = gray.get(iy.saturating_mul(px[0] as usize).saturating_add(ix)).copied() else { continue };
            let start = ((y as usize).saturating_mul(pix_width).saturating_add(x as usize)).saturating_mul(4);
            if let Some(dst) = data.get_mut(start..start + 4) {
                dst.copy_from_slice(&[g, g, g, 255]);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (DisplayList, RenderView) {
        let list = DisplayList {
            items: vec![Item::Polylines {
                stroke: Stroke::Hairline,
                color: Rgba([220, 30, 40, 255]),
                lines: vec![Polyline(vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]])].into(),
                object: None,
            }],
        };
        let view = RenderView { world: Bounds { min: [-10.0, -10.0], max: [10.0, 10.0] }, max_side: 128, background: Rgba([255, 255, 255, 255]) };
        (list, view)
    }

    #[test]
    fn render_png_is_byte_deterministic() {
        let (list, view) = sample();
        assert_eq!(render_png(&list, &view).expect("render"), render_png(&list, &view).expect("render again"));
    }

    #[test]
    fn render_png_rejects_dimensions_over_cap() {
        let (list, mut view) = sample();
        view.max_side = 2049;
        assert!(matches!(render_png(&list, &view), Err(Error::TooLarge(_))));
    }

    #[test]
    fn render_business_card_snapshot() {
        let case = openlaser_testkit::t4::load()
            .expect("T4 fixtures")
            .into_iter()
            .find(|case| case.name == "businessCard.dxf.import_geometry")
            .expect("business card DXF case");
        let bytes = openlaser_testkit::t4::source_bytes(&case).expect("business card DXF bytes");
        let imported = openlaser_import::import(&case.filename, &bytes, &openlaser_import::ImportOptions::default()).expect("business card imports");
        assert_eq!(imported.shapes.len(), 403);
        let bounds = imported.bounds.expect("business card bounds");
        let mut list = DisplayList::default();
        for shape in imported.shapes {
            if let openlaser_doc::Geom::Path(path) = shape.geom {
                let mut lines = Vec::new();
                let mut points = Vec::new();
                for element in path.elements() {
                    match element {
                        openlaser_geom::PathEl::MoveTo(point) => {
                            if points.len() >= 2 {
                                lines.push(Polyline(std::mem::take(&mut points)));
                            }
                            let point = shape.xform * *point;
                            points.push([point.x, point.y]);
                        }
                        openlaser_geom::PathEl::LineTo(point) => {
                            let point = shape.xform * *point;
                            points.push([point.x, point.y]);
                        }
                        openlaser_geom::PathEl::ClosePath => {
                            if points.len() >= 2 {
                                lines.push(Polyline(std::mem::take(&mut points)));
                            }
                        }
                        openlaser_geom::PathEl::QuadTo(_, _) | openlaser_geom::PathEl::CurveTo(_, _, _) => {}
                    }
                }
                if points.len() >= 2 {
                    lines.push(Polyline(points));
                }
                if !lines.is_empty() {
                    list.items.push(Item::Polylines { stroke: Stroke::Hairline, color: layer_color(shape.layer), lines: lines.into(), object: Some(shape.id) });
                }
            }
        }
        assert!(!list.items.is_empty(), "business card paths must be present");
        let width = bounds.max[0] - bounds.min[0];
        let height = bounds.max[1] - bounds.min[1];
        let margin = width.max(height) * 0.06;
        let view = RenderView {
            world: Bounds { min: [bounds.min[0] - margin, bounds.min[1] - margin], max: [bounds.max[0] + margin, bounds.max[1] + margin] },
            max_side: 768,
            background: Rgba([250, 250, 248, 255]),
        };
        let png = render_png(&list, &view).expect("business card renders");
        insta::assert_binary_snapshot!("business_card.png", png);
    }
}

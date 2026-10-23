//! Display lists and deterministic CPU PNG rendering (SPEC §3.10).
//!
//! Lists are in world millimetres, Y up: design mm for [`document_list`], machine mm for [`plan_list`] and
//! [`emission_list`] (for a gantry these are the same centre-origin frame; for a galvo the planner's
//! `machine_transform` maps one to the other).
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use openlaser_doc::{Geom, PALETTE, TOOL_COLORS};
use openlaser_geom::Affine;

pub use openlaser_doc::Project;
pub use openlaser_geom::{P2, Polyline};
pub use openlaser_machine::MachineProfile;
pub use openlaser_plan::JobPlan;
pub use openlaser_types::{Bounds, EmittedSegment, Notice, ObjectId, Refusal};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Largest PNG side, in pixels.
pub const MAX_SIDE: u32 = 2048;
/// Largest encoded PNG, in bytes.
pub const MAX_PNG_BYTES: usize = 5 * 1024 * 1024;

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

pub const REFUSAL_COLOR: Rgba = Rgba([230, 40, 40, 255]);
pub const NOTICE_COLOR: Rgba = Rgba([230, 170, 30, 255]);
pub const TRAVEL_COLOR: Rgba = Rgba([130, 130, 130, 180]);
pub const FRAME_COLOR: Rgba = Rgba([50, 160, 255, 255]);
pub const SELECTION_COLOR: Rgba = Rgba([40, 120, 255, 255]);
const AREA_COLOR: Rgba = Rgba([120, 120, 120, 255]);

/// The canvas: every visible shape in its layer's palette colour (selected shapes dashed, with a dashed selection
/// box), shapes on hidden layers left out, the field or bed outline, refusal and notice markers where they apply
/// (a refusal's `at`, else the centre of its object; a notice at its object), and the origin.
pub fn document_list(p: &Project, m: Option<&MachineProfile>, o: &ViewOpts) -> DisplayList {
    let mut list = DisplayList::default();
    let mut object_bounds: HashMap<ObjectId, Bounds> = HashMap::new();
    let mut selection_bounds: Option<Bounds> = None;
    // An explicit stack (groups may nest deeply in a foreign file): (shape, parent transform, top-level id).
    let mut stack: Vec<(&openlaser_doc::Shape, Affine, ObjectId)> = p.shapes.iter().rev().map(|s| (s.as_ref(), Affine::IDENTITY, s.id)).collect();
    while let Some((shape, parent, top)) = stack.pop() {
        let xform = parent * shape.xform;
        if let Geom::Group(children) = &shape.geom {
            stack.extend(children.iter().rev().map(|child| (child.as_ref(), xform, top)));
            continue;
        }
        if !layer_shown(p, shape.layer) {
            continue;
        }
        let selected = o.selection.contains(&top);
        let stroke = if selected { Stroke::Dashed } else { Stroke::Hairline };
        let color = layer_rgba(shape.layer, o.dark);
        let mut add_bounds = |b: Option<Bounds>| {
            if let Some(b) = b {
                object_bounds.entry(top).and_modify(|old| *old = union(*old, b)).or_insert(b);
                if selected {
                    selection_bounds = Some(selection_bounds.map_or(b, |old| union(old, b)));
                }
            }
        };
        let lines: Vec<Polyline> = match &shape.geom {
            Geom::Path(path) => {
                openlaser_geom::flatten(path, openlaser_geom::DEFAULT_FLATTEN_TOL_MM).into_iter().map(|line| line.transformed(&xform)).collect()
            }
            Geom::Rect { w, h, .. } => {
                vec![Polyline(vec![[0.0, 0.0], [w.get(), 0.0], [w.get(), h.get()], [0.0, h.get()], [0.0, 0.0]]).transformed(&xform)]
            }
            Geom::Ellipse { rx, ry } => vec![
                Polyline(
                    (0..=72)
                        .map(|i| {
                            let angle = f64::from(i) * std::f64::consts::TAU / 72.0;
                            [rx.get() * angle.cos(), ry.get() * angle.sin()]
                        })
                        .collect(),
                )
                .transformed(&xform),
            ],
            Geom::Polygon { sides, radius } if *sides >= 3 => vec![
                Polyline(
                    (0..=*sides)
                        .map(|i| {
                            let angle = f64::from(i) * std::f64::consts::TAU / f64::from(*sides);
                            [radius.get() * angle.cos(), radius.get() * angle.sin()]
                        })
                        .collect(),
                )
                .transformed(&xform),
            ],
            Geom::Polygon { .. } | Geom::Group(_) => Vec::new(),
            Geom::Bitmap(bitmap) => {
                let rect = transform_bounds(Bounds { min: [0.0, 0.0], max: bitmap.size_mm }, &xform);
                add_bounds(Some(rect));
                list.items.push(Item::Image { rect, gray: bitmap.gray.clone(), px: bitmap.px });
                if selected {
                    list.items.push(Item::Rect { stroke: Stroke::Dashed, color: SELECTION_COLOR, rect });
                }
                continue;
            }
        };
        if lines.is_empty() {
            continue;
        }
        add_bounds(openlaser_geom::bounds_of(&lines));
        list.items.push(Item::Polylines { stroke, color, lines: lines.into(), object: Some(top) });
    }
    if let Some(rect) = selection_bounds {
        list.items.push(Item::Rect { stroke: Stroke::Dashed, color: SELECTION_COLOR, rect });
    }
    let mut machine_to_design = Affine::IDENTITY;
    if let Some(profile) = m {
        match &profile.kind {
            openlaser_machine::Kinematics::Galvo(g) => {
                machine_to_design = openlaser_plan::machine_transform(g).inverse();
                if o.show_field {
                    let r = g.effective_work_area();
                    list.items.push(Item::Rect { stroke: Stroke::Dashed, color: AREA_COLOR, rect: Bounds { min: r.min, max: r.max } });
                }
            }
            openlaser_machine::Kinematics::Gantry(g) => {
                if o.show_work_area {
                    // Design coordinates are centre-origin: the bed is centred on (0, 0).
                    let [w, h] = g.bed_mm;
                    list.items.push(Item::Rect {
                        stroke: Stroke::Dashed,
                        color: AREA_COLOR,
                        rect: Bounds { min: [-w / 2.0, -h / 2.0], max: [w / 2.0, h / 2.0] },
                    });
                }
            }
        }
    }
    let centre = |b: &Bounds| [b.min[0] / 2.0 + b.max[0] / 2.0, b.min[1] / 2.0 + b.max[1] / 2.0];
    for refusal in &o.refusals {
        let at = match (refusal.at, refusal.object) {
            (Some([x, y]), _) => {
                let q = machine_to_design * openlaser_geom::Point::new(x, y);
                Some([q.x, q.y])
            }
            (None, Some(id)) => object_bounds.get(&id).map(centre),
            (None, None) => None,
        };
        if let Some(at) = at.filter(|p| p[0].is_finite() && p[1].is_finite()) {
            list.items.push(Item::Marker { kind: MarkerKind::Refusal, at, color: REFUSAL_COLOR });
        }
    }
    for notice in &o.notices {
        if let Some(at) = notice.object.and_then(|id| object_bounds.get(&id)).map(centre) {
            list.items.push(Item::Marker { kind: MarkerKind::Notice(notice.n), at, color: NOTICE_COLOR });
        }
    }
    let origin = if o.dark { Rgba([220, 220, 220, 255]) } else { Rgba([60, 60, 60, 255]) };
    list.items.push(Item::Marker { kind: MarkerKind::Origin, at: [0.0, 0.0], color: origin });
    list
}

/// The planned job in burn order: burns in layer colours (or, with `show.order`, on a blue → red ramp along the
/// burn order), travel moves from the end of each path to the start of the next, and the dashed frame.
pub fn plan_list(plan: &JobPlan, m: &MachineProfile, show: &PlanShow) -> DisplayList {
    let _ = m;
    let mut list = DisplayList::default();
    let total: usize = plan.sections.iter().map(|s| s.paths.len()).sum();
    let mut index = 0usize;
    let mut previous_end: Option<P2> = None;
    let mut travel: Vec<[P2; 2]> = Vec::new();
    for section in &plan.sections {
        let first_index = index;
        for path in &section.paths {
            if let (Some(first), Some(last)) = (path.0.first(), path.0.last()) {
                if let Some(previous) = previous_end
                    && previous != *first
                {
                    travel.push([previous, *first]);
                }
                previous_end = Some(*last);
            }
            index += 1;
        }
        if !show.burn || section.paths.is_empty() {
            continue;
        }
        let object = section.objects.first().copied();
        match section.mode {
            openlaser_types::SectionMode::Line if show.order => {
                for (k, path) in section.paths.iter().enumerate() {
                    let color = order_rgba(first_index + k, total);
                    list.items.push(Item::Polylines { stroke: Stroke::Hairline, color, lines: vec![path.clone()].into(), object });
                }
            }
            openlaser_types::SectionMode::Line => {
                list.items.push(Item::Polylines {
                    stroke: Stroke::Hairline,
                    color: layer_rgba(section.layer, false),
                    lines: section.paths.clone().into(),
                    object,
                });
            }
            openlaser_types::SectionMode::Fill | openlaser_types::SectionMode::Image => {
                let color = if show.order { order_rgba(first_index + section.paths.len() / 2, total) } else { layer_rgba(section.layer, false) };
                let segs: Vec<[P2; 2]> = section.paths.iter().filter_map(|p| Some([*p.0.first()?, *p.0.last()?])).collect();
                list.items.push(Item::Segments { color, segs: segs.into() });
            }
        }
    }
    if show.travel && !travel.is_empty() {
        list.items.push(Item::Segments { color: TRAVEL_COLOR, segs: travel.into() });
    }
    if show.frame && !plan.frame.is_empty() {
        list.items.push(Item::Polylines { stroke: Stroke::Dashed, color: FRAME_COLOR, lines: plan.frame.clone().into(), object: None });
    }
    list
}

pub fn emission_list(burned: &[EmittedSegment], light: &[[P2; 2]], m: &MachineProfile) -> DisplayList {
    let _ = m;
    let mut list = DisplayList::default();
    if !burned.is_empty() {
        let segs: Vec<[P2; 2]> = burned.iter().map(|e| [e.a, e.b]).collect();
        list.items.push(Item::Segments { color: Rgba([255, 50, 50, 255]), segs: segs.into() });
    }
    if !light.is_empty() {
        list.items.push(Item::Segments { color: Rgba([255, 40, 40, 120]), segs: light.to_vec().into() });
    }
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
    if view.max_side == 0 || view.max_side > MAX_SIDE {
        return Err(Error::TooLarge(format!("max_side must be in 1..={MAX_SIDE} (got {})", view.max_side)));
    }
    let width = view.world.max[0] - view.world.min[0];
    let height = view.world.max[1] - view.world.min[1];
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(Error::Invalid("world bounds must be finite and non-empty".to_owned()));
    }
    // Both sides are in 1..=max_side (<= 2048), so the casts are exact.
    let (pxw, pxh) = if width >= height {
        (view.max_side, (f64::from(view.max_side) * height / width).round().clamp(1.0, f64::from(view.max_side)) as u32)
    } else {
        ((f64::from(view.max_side) * width / height).round().clamp(1.0, f64::from(view.max_side)) as u32, view.max_side)
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
            Item::Marker { kind, at, color } => draw_marker(&mut pixmap, *kind, *at, *color, view.world, sx, sy),
            Item::Image { rect, gray, px } => draw_image(&mut pixmap, *rect, gray, *px, view.world, sx, sy)?,
        }
    }
    let png = pixmap.encode_png().map_err(|e| Error::Invalid(e.to_string()))?;
    if png.len() > MAX_PNG_BYTES {
        return Err(Error::TooLarge(format!("encoded PNG is {} bytes; the limit is {MAX_PNG_BYTES}", png.len())));
    }
    Ok(png)
}

fn layer_shown(p: &Project, layer: openlaser_types::LayerKey) -> bool {
    match layer {
        openlaser_types::LayerKey::Color(i) => p.color_layer(i).is_none_or(|l| l.show),
        openlaser_types::LayerKey::Tool(w) => p.layers.tools.iter().find(|t| t.which == w).is_none_or(|t| t.show),
    }
}

/// The layer's palette colour, kept readable on the background: near-black lightens on a dark canvas and
/// near-white darkens on a light one.
fn layer_rgba(layer: openlaser_types::LayerKey, dark: bool) -> Rgba {
    let rgb = match layer {
        openlaser_types::LayerKey::Color(i) => PALETTE.get(usize::from(i)).map_or([0, 0, 0], |c| c.0),
        openlaser_types::LayerKey::Tool(w) => TOOL_COLORS.get(usize::from(w).saturating_sub(1)).map_or([0, 0, 0], |c| c.0),
    };
    let luma = (u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114) / 1000;
    match (dark, luma) {
        (true, 0..=79) => Rgba([225, 225, 225, 255]),
        (false, 231..) => Rgba([45, 45, 45, 255]),
        _ => Rgba([rgb[0], rgb[1], rgb[2], 255]),
    }
}

/// Burn-order ramp: blue (first) → red (last).
fn order_rgba(index: usize, total: usize) -> Rgba {
    let t = if total <= 1 { 0.0 } else { index as f64 / (total - 1) as f64 };
    // t in 0..=1, so both channels are in 0..=255.
    let red = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba([red, 40, 255 - red, 255])
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds { min: [a.min[0].min(b.min[0]), a.min[1].min(b.min[1])], max: [a.max[0].max(b.max[0]), a.max[1].max(b.max[1])] }
}

fn transform_bounds(rect: Bounds, a: &Affine) -> Bounds {
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

fn paint(color: Rgba) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(to_color(color));
    paint.anti_alias = false;
    paint
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
    let (width, dash) = match stroke {
        Stroke::Width(v) if v.is_finite() && v > 0.0 => (v, None),
        Stroke::Width(_) => return Err(Error::Invalid("stroke width must be finite and positive".to_owned())),
        Stroke::Hairline => (1.0, None),
        Stroke::Dashed => (1.0, tiny_skia::StrokeDash::new(vec![6.0, 4.0], 0.0)),
    };
    pix.stroke_path(&path, &paint(color), &tiny_skia::Stroke { width, dash, ..tiny_skia::Stroke::default() }, tiny_skia::Transform::identity(), None);
    Ok(())
}

fn draw_marker(pix: &mut tiny_skia::Pixmap, kind: MarkerKind, at: P2, color: Rgba, w: Bounds, sx: f64, sy: f64) {
    let (x, y) = screen(at, w, sx, sy);
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let mut b = tiny_skia::PathBuilder::new();
    match kind {
        // A cross for the origin, a dot for a notice, a larger dot with a ring gap for a refusal.
        MarkerKind::Origin => {
            b.move_to(x - 5.0, y);
            b.line_to(x + 5.0, y);
            b.move_to(x, y - 5.0);
            b.line_to(x, y + 5.0);
            if let Some(path) = b.finish() {
                pix.stroke_path(&path, &paint(color), &tiny_skia::Stroke::default(), tiny_skia::Transform::identity(), None);
            }
        }
        MarkerKind::Notice(_) | MarkerKind::Refusal => {
            b.push_circle(x, y, if matches!(kind, MarkerKind::Refusal) { 5.0 } else { 3.5 });
            if let Some(path) = b.finish() {
                pix.fill_path(&path, &paint(color), tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            }
        }
    }
}

fn draw_image(pix: &mut tiny_skia::Pixmap, rect: Bounds, gray: &[u8], px: [u32; 2], w: Bounds, sx: f64, sy: f64) -> Result<(), Error> {
    let expected = u64::from(px[0]) * u64::from(px[1]);
    if expected != u64::try_from(gray.len()).unwrap_or(u64::MAX) {
        return Err(Error::Invalid("bitmap byte count does not match dimensions".to_owned()));
    }
    if ![rect.min[0], rect.min[1], rect.max[0], rect.max[1]].iter().all(|v| v.is_finite()) {
        return Err(Error::Invalid("image rectangle is not finite".to_owned()));
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
            let u = ((x as f32 - x0) / (x1 - x0).max(1.0)).clamp(0.0, 0.999_999);
            let v = ((y as f32 - y0) / (y1 - y0).max(1.0)).clamp(0.0, 0.999_999);
            let ix = (u * px[0] as f32) as usize;
            let iy = (v * px[1] as f32) as usize;
            let Some(g) = gray.get(iy.saturating_mul(px[0] as usize).saturating_add(ix)).copied() else { continue };
            let start = ((y as usize).saturating_mul(pix_width).saturating_add(x as usize)).saturating_mul(4);
            if let Some(dst) = data.get_mut(start..start.saturating_add(4)) {
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
    fn render_png_deterministic() {
        let (mut list, view) = sample();
        list.items.push(Item::Rect { stroke: Stroke::Dashed, color: SELECTION_COLOR, rect: Bounds { min: [-8.0, -8.0], max: [8.0, 8.0] } });
        list.items.push(Item::Marker { kind: MarkerKind::Refusal, at: [1.0, 1.0], color: REFUSAL_COLOR });
        let first = render_png(&list, &view).expect("render");
        for _ in 0..3 {
            assert_eq!(render_png(&list, &view).expect("render again"), first);
        }
        assert_eq!(first, render_png(&list.clone(), &view.clone()).expect("a clone renders the same bytes"));
    }

    #[test]
    fn render_png_size_capped() {
        let (list, mut view) = sample();
        for side in [0, MAX_SIDE + 1, u32::MAX] {
            view.max_side = side;
            assert!(matches!(render_png(&list, &view), Err(Error::TooLarge(_))), "max_side {side}");
        }
        // Incompressible content at the largest side exceeds the 5 MiB output cap and is refused, not truncated.
        let px = [MAX_SIDE, MAX_SIDE];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let gray: Arc<[u8]> = (0..u64::from(px[0]) * u64::from(px[1]))
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        let noise = DisplayList { items: vec![Item::Image { rect: Bounds { min: [0.0, 0.0], max: [10.0, 10.0] }, gray, px }] };
        let view = RenderView { world: Bounds { min: [0.0, 0.0], max: [10.0, 10.0] }, max_side: MAX_SIDE, background: Rgba([0, 0, 0, 255]) };
        assert!(matches!(render_png(&noise, &view), Err(Error::TooLarge(_))));
        // Within the caps: the long side is max_side.
        let (list, mut view) = sample();
        view.world = Bounds { min: [0.0, 0.0], max: [40.0, 10.0] };
        view.max_side = 64;
        let png = render_png(&list, &view).expect("render");
        assert_eq!(&png[16..24], &[0, 0, 0, 64, 0, 0, 0, 16], "IHDR width 64, height 16");
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
        // `geom::flatten` is WP1's; until it lands the DXF's line-only paths are walked here (no curves to flatten).
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
                            points.clear();
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
                        openlaser_geom::PathEl::QuadTo(_, _) | openlaser_geom::PathEl::CurveTo(_, _, _) => panic!("DXF import gives line paths only"),
                    }
                }
                if points.len() >= 2 {
                    lines.push(Polyline(points));
                }
                assert!(!lines.is_empty());
                list.items.push(Item::Polylines {
                    stroke: Stroke::Hairline,
                    color: layer_rgba(shape.layer, false),
                    lines: lines.into(),
                    object: Some(shape.id),
                });
            }
        }
        assert_eq!(list.items.len(), 403, "every business card path is drawn");
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

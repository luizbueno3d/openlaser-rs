//! Clean-room LightBurn XML import. Format observations and limitations: docs/LBRN2.md.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use base64::Engine;
use openlaser_doc::{CutMode, DeviceKind, ImageMode, Project, Workspace, WorkspaceOrigin};
use openlaser_geom::{PathEl, Point};
use openlaser_types::{Deg, Hz, Mm, MmPerS, Ns};
use roxmltree::{Document, Node, ParsingOptions};

use crate::*;

pub const MAX_SHAPES: usize = 100_000;
pub const MAX_DEPTH: usize = 64;

fn malformed(message: impl ToString) -> Error {
    Error::Malformed { format: "LBRN2", message: message.to_string() }
}
fn child<'a, 'i>(n: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(tag))
}
fn number(s: &str) -> Result<f64> {
    let v: f64 = s.parse().map_err(malformed)?;
    if !v.is_finite() {
        return Err(malformed("number must be finite"));
    }
    Ok(v)
}
fn attr(n: Node<'_, '_>, key: &str) -> Result<f64> {
    number(n.attribute(key).ok_or_else(|| malformed(format!("{} missing {key}", n.tag_name().name())))?)
}
fn dimension(n: Node<'_, '_>, key: &str) -> Result<Mm> {
    let v = attr(n, key)?;
    if v <= 0.0 {
        return Err(malformed(format!("{key} must be positive")));
    }
    Mm::new(v).map_err(malformed)
}
fn transform(n: Node<'_, '_>) -> Result<Affine> {
    let Some(x) = child(n, "XForm") else { return Ok(Affine::IDENTITY) };
    let values = x.text().unwrap_or_default().split_whitespace().map(number).collect::<Result<Vec<_>>>()?;
    let values: [f64; 6] = values.try_into().map_err(|_| malformed("XForm needs six numbers"))?;
    Ok(Affine::new(values))
}

struct Parser<'a> {
    verts: BTreeMap<&'a str, Arc<Vec<Vertex>>>,
    prims: BTreeMap<&'a str, &'a str>,
    skipped: BTreeMap<String, usize>,
    budget: Budget,
    bitmap_bytes: u64,
}
impl Parser<'_> {
    fn notice(&mut self, message: impl Into<String>) {
        *self.skipped.entry(message.into()).or_default() += 1;
    }
    fn path(&mut self, n: Node<'_, '_>) -> Result<Geom> {
        let vertices = if let Some(list) = child(n, "VertList") {
            if let Some(shared) = n.attribute("VertID").and_then(|id| self.verts.get(id)) {
                Arc::clone(shared)
            } else {
                Arc::new(vertices(list.text().unwrap_or_default())?)
            }
        } else {
            self.verts
                .get(n.attribute("VertID").ok_or_else(|| malformed("Path missing VertList/VertID"))?)
                .cloned()
                .ok_or_else(|| malformed("unknown VertID"))?
        };
        let prims = child(n, "PrimList")
            .and_then(|v| v.text())
            .or_else(|| n.attribute("PrimID").and_then(|id| self.prims.get(id).copied()))
            .ok_or_else(|| malformed("Path missing PrimList or known PrimID"))?;
        let path = primitives(prims, &vertices)?;
        self.budget.entity(path.elements().len().saturating_mul(3))?;
        Ok(Geom::Path(Arc::new(path)))
    }
    fn shape(&mut self, n: Node<'_, '_>, opts: &ImportOptions) -> Result<Option<Shape>> {
        let kind = n.attribute("Type").unwrap_or("(missing Type)");
        let index: u8 = n.attribute("CutIndex").unwrap_or("0").parse().map_err(malformed)?;
        if index >= 30 {
            return Err(malformed(format!("CutIndex {index} is not a colour layer")));
        }
        for c in n.children().filter(|c| c.is_element()) {
            if !matches!(c.tag_name().name(), "XForm" | "VertList" | "PrimList" | "Children" | "BackupPath" | "SubLayer") {
                self.notice(format!("Skipped {kind} element {}", c.tag_name().name()));
            }
        }
        let layer = opts.layer.unwrap_or(LayerKey::Color(index));
        let xform = transform(n)?;
        let geom = match kind {
            "Rect" => {
                let w = dimension(n, "W")?;
                let h = dimension(n, "H")?;
                let cr = number(n.attribute("Cr").unwrap_or("0"))?;
                if cr < 0.0 || cr > w.get().min(h.get()) / 2.0 {
                    return Err(malformed("Cr is outside the rectangle"));
                }
                self.budget.entity(16)?;
                Geom::Rect { w, h, corner_radius: Mm::new(cr).map_err(malformed)? }
            }
            "Ellipse" => {
                self.budget.entity(16)?;
                Geom::Ellipse { rx: dimension(n, "Rx")?, ry: dimension(n, "Ry")? }
            }
            "Path" => self.path(n)?,
            "Group" => {
                let mut children = Vec::new();
                if let Some(container) = child(n, "Children") {
                    for c in container.children().filter(|c| c.is_element()) {
                        if c.has_tag_name("Shape") {
                            if let Some(s) = self.shape(c, opts)? {
                                children.push(Arc::new(s));
                            }
                        } else {
                            self.notice(format!("Skipped group element {}", c.tag_name().name()));
                        }
                    }
                }
                Geom::Group(children)
            }
            "Text" => {
                let name = n.attribute("Str").unwrap_or("unnamed");
                let Some(backup) = child(n, "BackupPath") else {
                    self.notice(format!("Skipped Text {name:?}: no BackupPath"));
                    return Ok(None);
                };
                self.notice("Text imported as BackupPath outlines; fonts and editable text are not reproduced");
                // BackupPath's XForm is a cached copy and can be stale after text is moved.
                // Its vertices are in the text's object space: use the live Text XForm once.
                self.path(child(backup, "Shape").unwrap_or(backup))?
            }
            "Bitmap" => {
                let data = n.attribute("Data").ok_or_else(|| malformed("Bitmap missing Data (shared bitmap references are unsupported)"))?;
                let png = decode_png(data)?;
                let imported = import_raster("embedded.png", &png, &ImportOptions::default())?;
                let bitmap = imported
                    .shapes
                    .first()
                    .and_then(|s| match &s.geom {
                        Geom::Bitmap(b) => Some(b),
                        _ => None,
                    })
                    .ok_or_else(|| malformed("PNG decoder produced no bitmap"))?;
                self.bitmap_bytes = self.bitmap_bytes.checked_add(bitmap.gray.len() as u64).ok_or_else(|| Error::TooLarge("bitmap bytes overflow".into()))?;
                if self.bitmap_bytes > MAX_RASTER_ALLOC {
                    return Err(Error::TooLarge("LBRN2 total decoded bitmap bytes exceed raster allocation limit".into()));
                }
                let mut bitmap = bitmap.as_ref().clone();
                bitmap.size_mm = [dimension(n, "W")?.get(), dimension(n, "H")?.get()];
                for key in ["Gamma", "Contrast", "Brightness", "EnhanceAmount", "EnhanceRadius", "EnhanceDenoise"] {
                    if let Some(v) = n.attribute(key) {
                        let expected = if key == "Gamma" { 1.0 } else { 0.0 };
                        if number(v)? != expected {
                            self.notice(format!("Bitmap adjustment {key} ignored"));
                        }
                    }
                }
                Geom::Bitmap(Arc::new(bitmap))
            }
            other => {
                self.notice(format!("Skipped shape Type={other}"));
                return Ok(None);
            }
        };
        // Geometry is already Y-up, including the text outlines. No root reflection.
        let mut s = shape(layer, geom, xform)?;
        s.name = n.attribute("Str").or_else(|| n.attribute("Name")).map(str::to_owned);
        // Check every control point after all ancestor transforms below as well.
        for v in xform.as_coeffs() {
            check_placed(v, 0.0, "XForm")?;
        }
        Ok(Some(s))
    }
}

/// Decodes only embedded PNG data; never resolves paths or URLs.
pub fn decode_png(data: &str) -> Result<Vec<u8>> {
    if data.len() > MAX_RASTER_FILE_BYTES.saturating_mul(4) / 3 + 4 {
        return Err(Error::TooLarge("embedded PNG exceeds raster byte limit".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(malformed)?;
    if bytes.len() > MAX_RASTER_FILE_BYTES {
        return Err(Error::TooLarge("embedded PNG exceeds raster byte limit".into()));
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(malformed("Bitmap Data is not PNG"));
    }
    Ok(bytes)
}

#[derive(Clone, Debug)]
struct Vertex {
    p: Point,
    c0: Point,
    c1: Point,
}

// A bounded scanner, including exponent syntax. `c`/`V`/`L`/`B` delimit compact tokens.
struct Scan<'a>(&'a str);
impl Scan<'_> {
    fn space(&mut self) {
        self.0 = self.0.trim_start();
    }
    fn take(&mut self, prefix: &str) -> bool {
        self.space();
        if let Some(rest) = self.0.strip_prefix(prefix) {
            self.0 = rest;
            true
        } else {
            false
        }
    }
    fn token(&mut self) -> Result<&str> {
        self.space();
        let len = self.0.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(self.0.len());
        if len == 0 {
            return Err(malformed("expected a number in path"));
        }
        let (token, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(token)
    }
    fn num(&mut self) -> Result<f64> {
        number(self.token()?)
    }
    fn index(&mut self) -> Result<usize> {
        self.token()?.parse().map_err(malformed)
    }
    fn empty(&mut self) -> bool {
        self.space();
        self.0.is_empty()
    }
}
fn vertices(text: &str) -> Result<Vec<Vertex>> {
    let mut scan = Scan(text);
    let mut out = Vec::new();
    while !scan.empty() {
        if out.len() >= MAX_ENTITY_POINTS {
            return Err(Error::TooLarge("LBRN2 vertices per path".into()));
        }
        if !scan.take("V") {
            return Err(malformed("expected V in VertList"));
        }
        let p = Point::new(scan.num()?, scan.num()?);
        let mut controls = [None; 4];
        while scan.take("c") {
            let slot = if scan.take("0x") {
                0
            } else if scan.take("0y") {
                1
            } else if scan.take("1x") {
                2
            } else if scan.take("1y") {
                3
            } else {
                return Err(malformed("unknown control component"));
            };
            let value = scan.num()?;
            let target = controls.get_mut(slot).ok_or_else(|| malformed("control component"))?;
            if target.replace(value).is_some() {
                return Err(malformed("duplicate control component"));
            }
        }
        fn control(p: Point, x: Option<f64>, y: Option<f64>) -> Result<Point> {
            match (x, y) {
                (Some(x), Some(y)) => Ok(Point::new(x, y)),
                (None, None) | (Some(1.0), None) => Ok(p),
                _ => Err(malformed("incomplete control point (only cNx1 may omit cNy)")),
            }
        }
        let [x0, y0, x1, y1] = controls;
        out.push(Vertex { p, c0: control(p, x0, y0)?, c1: control(p, x1, y1)? });
    }
    Ok(out)
}
fn primitives(text: &str, vertices: &[Vertex]) -> Result<BezPath> {
    let mut path = BezPath::new();
    let mut scan = Scan(text);
    if text.trim() == "LineClosed" || text.trim() == "LineOpen" {
        if let Some(v) = vertices.first() {
            path.move_to(v.p);
        }
        for v in vertices.iter().skip(1) {
            path.line_to(v.p);
        }
        if text.trim() == "LineClosed" {
            path.close_path();
        }
        return Ok(path);
    }
    let mut current = None;
    let mut start = None;
    let mut count = 0usize;
    while !scan.empty() {
        count += 1;
        check_entity_points(count.saturating_mul(3))?;
        let bezier = if scan.take("L") {
            false
        } else if scan.take("B") {
            true
        } else {
            return Err(malformed("unsupported PrimList command"));
        };
        let (i, j) = (scan.index()?, scan.index()?);
        let a = vertices.get(i).ok_or_else(|| malformed("primitive vertex index out of range"))?;
        let b = vertices.get(j).ok_or_else(|| malformed("primitive vertex index out of range"))?;
        if current != Some(i) {
            path.move_to(a.p);
            start = Some(i);
        }
        if bezier {
            path.curve_to(a.c0, b.c1, b.p);
        } else {
            path.line_to(b.p);
        }
        current = Some(j);
        if start == Some(j) {
            path.close_path();
            current = None;
            start = None;
        }
    }
    Ok(path)
}

/// Profile defaults are mandatory: hidden LightBurn defaults are never guessed.
pub fn import(bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    validate_options(opts)?;
    let bytes = check_vector_size("LBRN2", bytes)?;
    let text = std::str::from_utf8(bytes).map_err(malformed)?;
    if text.contains("<!DOCTYPE") {
        return Err(malformed("DTD is disabled"));
    }
    // Share the existing raw XML guard: bound attribute/namespace quadratic work and
    // parser nesting before roxmltree allocates a DOM. Its checks are XML-generic.
    let _ = svg_guard::prepare(text).map_err(|e| match e {
        Error::Malformed { message, .. } => malformed(message),
        other => other,
    })?;
    let xml = Document::parse_with_options(text, ParsingOptions { allow_dtd: false, nodes_limit: MAX_SVG_XML_NODES }).map_err(malformed)?;
    let root = xml.root_element();
    if !root.has_tag_name("LightBurnProject") {
        return Err(malformed("expected LightBurnProject root"));
    }
    let mut count = 0;
    for n in root.descendants().filter(|n| n.is_element()) {
        if n.ancestors().take(MAX_DEPTH + 2).count() > MAX_DEPTH {
            return Err(Error::TooLarge("LBRN2 XML nesting depth".into()));
        }
        if n.has_tag_name("Shape") || n.has_tag_name("BackupPath") {
            count += 1;
        }
        if count > MAX_SHAPES {
            return Err(Error::TooLarge("LBRN2 shape count".into()));
        }
    }
    let defaults = opts.layer_defaults.as_ref().ok_or_else(|| malformed("target machine layer defaults are required"))?;
    let mut project = Project::new(Workspace { kind: DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: WorkspaceOrigin::Center }, defaults);
    let mut parser = Parser { verts: BTreeMap::new(), prims: BTreeMap::new(), skipped: BTreeMap::new(), budget: Budget::new(), bitmap_bytes: 0 };
    // Collect definitions once; output reuse is separately charged to the output budget.
    let mut definitions = 0usize;
    for n in root.descendants().filter(|n| n.is_element()) {
        if let (Some(id), Some(list)) = (n.attribute("VertID"), child(n, "VertList")) {
            let verts = vertices(list.text().unwrap_or_default())?;
            definitions = definitions.checked_add(verts.len()).ok_or_else(|| Error::TooLarge("vertex definitions overflow".into()))?;
            if definitions > MAX_IMPORT_POINTS {
                return Err(Error::TooLarge("LBRN2 total vertex definitions".into()));
            }
            if parser.verts.insert(id, Arc::new(verts)).is_some() {
                return Err(malformed("duplicate VertID definition"));
            }
        }
        if let (Some(id), Some(list)) = (n.attribute("PrimID"), child(n, "PrimList"))
            && parser.prims.insert(id, list.text().unwrap_or_default()).is_some()
        {
            return Err(malformed("duplicate PrimID definition"));
        }
        if n.has_tag_name("SubLayer") {
            parser.notice("Skipped SubLayer");
        }
    }
    let mut notices = Notices::default();
    let layers = layers(root, &mut project, &mut notices)?;
    for key in ["MirrorX", "MirrorY"] {
        if let Some(value) = root.attribute(key)
            && !matches!(value, "False" | "false" | "0")
        {
            parser.notice(format!("Unapplied project {key}={value}; root mirror semantics are unverified"));
        }
    }
    let mut shapes = Vec::new();
    for n in root.children().filter(|n| n.is_element()) {
        if n.has_tag_name("Shape") {
            if let Some(s) = parser.shape(n, opts)? {
                shapes.push(s);
            }
        } else if !matches!(n.tag_name().name(), "CutSetting" | "CutSetting_Img" | "Thumbnail" | "SubLayer") {
            parser.notice(format!("Skipped project element {}", n.tag_name().name()));
        }
    }
    for (message, count) in parser.skipped {
        notices.push(NoticeCode::ImportEntitySkipped, format!("{message} ({count})"), None);
    }
    let initial_bounds = bounds(&shapes)?;
    recenter_shapes(&mut shapes, initial_bounds, opts.center_at, opts.scale);
    let bounds = bounds(&shapes)?;
    Ok(Imported { shapes, layers, notices: notices.0, bounds })
}
fn bounds(shapes: &[Shape]) -> Result<Option<Bounds>> {
    let mut result: Option<Bounds> = None;
    let mut stack: Vec<_> = shapes.iter().map(|s| (s, Affine::IDENTITY)).collect();
    while let Some((s, parent)) = stack.pop() {
        let xf = parent * s.xform;
        if let Geom::Group(children) = &s.geom {
            stack.extend(children.iter().map(|s| (s.as_ref(), xf)));
            continue;
        }
        for el in s.geom.to_bez().elements() {
            let pts: &[Point] = match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => std::slice::from_ref(p),
                PathEl::QuadTo(a, b) => &[*a, *b],
                PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
                PathEl::ClosePath => &[],
            };
            for p in pts {
                let p = xf * *p;
                check_placed(p.x, p.y, "LBRN2 transformed geometry")?;
                result = Some(result.map_or(Bounds { min: [p.x, p.y], max: [p.x, p.y] }, |b| Bounds {
                    min: [b.min[0].min(p.x), b.min[1].min(p.y)],
                    max: [b.max[0].max(p.x), b.max[1].max(p.y)],
                }));
            }
        }
    }
    Ok(result)
}

fn layers(root: Node<'_, '_>, project: &mut Project, notices: &mut Notices) -> Result<Vec<(LayerKey, ColorLayer)>> {
    let mut used = BTreeSet::new();
    let mut priorities = BTreeMap::<u8, i32>::new();
    let mut repeats = BTreeMap::new();
    let mut missing = BTreeMap::<u8, BTreeSet<String>>::new();
    let mut ignored = BTreeMap::<u8, BTreeSet<String>>::new();
    let mut defined = BTreeSet::new();
    for n in root.descendants().filter(|n| n.has_tag_name("Shape")) {
        if let Some(i) = n.attribute("CutIndex") {
            let i: u8 = i.parse().map_err(malformed)?;
            if i >= 30 {
                return Err(malformed("CutIndex outside C00..C29"));
            }
            used.insert(i);
        }
    }
    for n in root.children().filter(|n| n.has_tag_name("CutSetting") || n.has_tag_name("CutSetting_Img")) {
        let image = n.has_tag_name("CutSetting_Img");
        let mut values = BTreeMap::new();
        for c in n.children().filter(|c| c.is_element() && !c.has_tag_name("SubLayer")) {
            let name = c.tag_name().name();
            let v = c.attribute("Value").ok_or_else(|| malformed(format!("{name} missing Value")))?;
            if values.insert(name, v).is_some() {
                return Err(malformed(format!("duplicate cut setting {name}")));
            }
        }
        let i: u8 = values.get("index").ok_or_else(|| malformed("CutSetting missing index"))?.parse().map_err(malformed)?;
        if i >= 30 {
            return Err(malformed("CutSetting index outside C00..C29"));
        }
        if !defined.insert((i, image)) {
            return Err(malformed("duplicate CutSetting index/type"));
        }
        used.insert(i);
        repeats.insert((i, image), 1u16);
        let key = LayerKey::Color(i);
        let layer = project.color_layer_mut(i).ok_or_else(|| malformed("missing target layer"))?;
        let prefix = if image { "image." } else { "" };
        let required = if image {
            &["speed", "maxPower", "minPower", "frequency", "QPulseWidth", "interval", "angle", "bidir", "numPasses", "globalRepeat", "ditherMode"][..]
        } else {
            &["speed", "maxPower", "minPower", "frequency", "QPulseWidth", "interval", "angle", "crossHatch", "bidir", "numPasses", "globalRepeat"][..]
        };
        if !values.contains_key("name") && !values.contains_key("subname") {
            missing.entry(i).or_default().insert(format!("{prefix}name"));
        }
        for field in required {
            if !values.contains_key(field) {
                missing.entry(i).or_default().insert(format!("{prefix}{field}"));
            }
        }
        let kind = n.attribute("type").unwrap_or("(missing)");
        let mode = match (image, kind) {
            (true, "Image") | (false, "Cut") => Some(CutMode::Line),
            (false, "Scan" | "Scan+Cut") => Some(CutMode::Fill),
            (false, "Offset") => Some(CutMode::OffsetFill),
            _ => None,
        };
        if mode.is_none() {
            layer.output = false;
            notices.push(NoticeCode::ImportEntitySkipped, format!("{key}: unknown cut type {kind}; Output OFF"), Some(key));
        }
        let mut op = layer.ops.first().cloned().ok_or_else(|| malformed("profile default layer has no operation"))?;
        if let Some(mode) = mode {
            op.mode = mode;
        }
        if image {
            op.galvo = layer.image.galvo.clone();
            op.gantry = layer.image.gantry.clone();
            op.fill.interval = layer.image.interval;
            op.fill.angle = layer.image.angle;
            op.fill.bidirectional = layer.image.bidirectional;
            op.passes = layer.image.passes;
        }
        for (field, value) in &values {
            let apply = || -> Result<()> {
                match *field {
                    "index" => {}
                    "name" | "subname" => {
                        let text = value.trim();
                        if text.chars().count() > 64 || text.chars().any(char::is_control) {
                            return Err(malformed("layer name must be at most 64 characters without controls"));
                        }
                        if (!image || !defined.contains(&(i, false))) && (*field == "subname" || !values.contains_key("subname")) {
                            layer.name = text.into();
                        }
                        op.name = text.into();
                    }
                    "speed" => {
                        let v = MmPerS::new(number(value)?).map_err(malformed)?;
                        op.galvo.speed = v;
                        op.gantry.speed = v;
                    }
                    "maxPower" => {
                        let v = Percent::new(number(value)?).map_err(malformed)?;
                        op.galvo.power = v;
                        op.gantry.power_max = v;
                    }
                    "minPower" => {
                        let v = Percent::new(number(value)?).map_err(malformed)?;
                        op.galvo.min_power = v;
                        op.gantry.power_min = v;
                    }
                    "frequency" => {
                        op.galvo.frequency = Hz::new(value.parse().map_err(malformed)?).map_err(malformed)?;
                    }
                    "QPulseWidth" => {
                        op.galvo.q_pulse = Some(Ns::new(number(value)?).map_err(malformed)?);
                    }
                    "interval" => {
                        let v = number(value)?;
                        if v <= 0.0 || v > 100.0 {
                            return Err(malformed("interval outside 0 (exclusive)..=100 mm"));
                        }
                        op.fill.interval = Mm::new(v).map_err(malformed)?;
                    }
                    "angle" => {
                        op.fill.angle = Deg::new(number(value)?).map_err(malformed)?;
                    }
                    "crossHatch" if image => {
                        ignored.entry(i).or_default().insert("image.crossHatch".into());
                        if boolean(value)? {
                            return Err(malformed("image crossHatch is unsupported"));
                        }
                    }
                    "crossHatch" => {
                        op.fill.cross_hatch = boolean(value)?;
                    }
                    "bidir" => {
                        op.fill.bidirectional = boolean(value)?;
                    }
                    "numPasses" => {
                        op.passes = pass_count(value)?;
                    }
                    "globalRepeat" => {
                        let passes = pass_count(value)?;
                        repeats.insert((i, image), passes);
                        layer.global_passes = passes;
                    }
                    "priority" => {
                        let p: i32 = value.parse().map_err(malformed)?;
                        if let Some(previous) = priorities.get(&i)
                            && *previous != p
                        {
                            // Priority only orders the layers; it never changes what burns, so the layer stays on
                            // and takes the earlier of the two (the notice says so).
                            notices.push(
                                NoticeCode::ImportEntitySkipped,
                                format!(
                                    "{key}: vector/image priorities differ ({previous}, {p}); one colour layer has one place in the order, so it uses the earlier one"
                                ),
                                Some(key),
                            );
                        }
                        priorities.entry(i).and_modify(|v| *v = (*v).min(p)).or_insert(p);
                    }
                    "ditherMode" if image => {
                        layer.image.mode = match value.to_ascii_lowercase().as_str() {
                            "threshold" => ImageMode::Threshold,
                            "jarvis" => ImageMode::Jarvis,
                            "stucki" => ImageMode::Stucki,
                            "floyd" | "dither" | "floydsteinberg" | "floyd_steinberg" => ImageMode::FloydSteinberg,
                            "atkinson" => ImageMode::Atkinson,
                            "ordered" => ImageMode::Ordered,
                            _ => return Err(malformed(format!("unsupported ditherMode {value}"))),
                        };
                    }
                    other => {
                        ignored.entry(i).or_default().insert(format!("{prefix}{other}"));
                    }
                }
                Ok(())
            };
            let mut apply = apply;
            if let Err(e) = apply() {
                layer.output = false;
                notices.push(
                    NoticeCode::ImportEntitySkipped,
                    format!("{key}: {prefix}{field}={value}: {e}; Output OFF (invalid value retained as target default)"),
                    Some(key),
                );
            }
        }
        if image {
            layer.image.galvo = op.galvo;
            layer.image.gantry = op.gantry;
            layer.image.interval = op.fill.interval;
            layer.image.angle = op.fill.angle;
            layer.image.bidirectional = op.fill.bidirectional;
            layer.image.passes = op.passes;
        } else {
            layer.ops = vec![op.clone()];
            if kind == "Scan+Cut" {
                op.mode = CutMode::Line;
                layer.ops.push(op);
            }
        }
    }
    for i in &used {
        let key = LayerKey::Color(*i);
        if let (Some(vector), Some(image)) = (repeats.get(&(*i, false)), repeats.get(&(*i, true)))
            && vector != image
        {
            if let Some(layer) = project.color_layer_mut(*i) {
                layer.output = false;
                layer.global_passes = *vector;
            }
            notices.push(
                NoticeCode::ImportEntitySkipped,
                format!("{key}: independent vector/image global passes ({vector}, {image}) cannot be represented by one colour layer; Output OFF"),
                Some(key),
            );
        }
        if !defined.contains(&(*i, false)) && !defined.contains(&(*i, true)) {
            missing.entry(*i).or_default().insert("all cut settings (no CutSetting)".into());
        }
        if let Some(fields) = missing.get(i) {
            notices.push(
                NoticeCode::ImportEntitySkipped,
                format!("{key}: OpenLaser target defaults used for missing values: {}", fields.iter().cloned().collect::<Vec<_>>().join(", ")),
                Some(key),
            );
        }
        if let Some(fields) = ignored.get(i) {
            notices.push(
                NoticeCode::ImportEntitySkipped,
                format!("{key}: ignored cut-setting keys: {}", fields.iter().cloned().collect::<Vec<_>>().join(", ")),
                Some(key),
            );
        }
    }
    let mut used: Vec<_> = used.into_iter().collect();
    used.sort_by_key(|i| (priorities.get(i).copied().unwrap_or(i32::MAX), *i));
    Ok(used.into_iter().filter_map(|i| project.color_layer(i).cloned().map(|l| (LayerKey::Color(i), l))).collect())
}
fn boolean(s: &str) -> Result<bool> {
    match s {
        "1" | "True" | "true" => Ok(true),
        "0" | "False" | "false" => Ok(false),
        _ => Err(malformed("expected boolean 0 or 1")),
    }
}
fn pass_count(s: &str) -> Result<u16> {
    let n: u16 = s.parse().map_err(malformed)?;
    if !(1..=999).contains(&n) {
        return Err(malformed("passes outside 1..=999"));
    }
    Ok(n)
}

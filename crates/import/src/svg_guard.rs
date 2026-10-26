//! The SVG guard (SPEC §1.3 rule 6, never crash). roxmltree, usvg, svgtypes and simplecss recurse per nesting level,
//! expand `<use>`, copy paint servers and styles per element, split arcs into curves and match CSS selectors, none
//! of it with a bound of its own. Everything they would do is measured here, before usvg builds anything, and a file
//! over a bound is refused:
//!
//! - [`prepare`] tokenizes the raw text once (comments, processing instructions, CDATA and quoted attribute values
//!   are skipped exactly as an XML parser skips them) and, before roxmltree runs, refuses element nesting deeper than
//!   [`MAX_SVG_DEPTH`] and what makes roxmltree itself quadratic: more than [`MAX_SVG_ATTRIBUTES`] attributes in a
//!   start tag or [`MAX_SVG_ATTRIBUTE_PAIRS`] attribute pairs in all (its duplicate check), more than
//!   [`MAX_SVG_NAMESPACES`] `xmlns` declarations (its namespace scans). A DOCTYPE internal subset is refused
//!   outright, so no entity is ever defined or expanded; a plain DOCTYPE is blanked out so that roxmltree parses
//!   with `allow_dtd: false`.
//! - [`check`] refuses, on the parsed tree, every reference usvg would follow without a bound and everything that
//!   hides geometry usvg would still draw: `clip-path` and `mask` of any value but `none`, `filter` and markers
//!   given as `url(…)`, every `<pattern>`, `<tref>`, element `<feImage>`s, nested `<svg>` elements without both
//!   sizes (browsers clip them, usvg does not), gradient `href` chains that loop or run longer than
//!   [`MAX_SVG_HREF_CHAIN`], CSS beyond element / class / id selectors, stroke widths over [`MAX_SVG_STROKE_WIDTH`]
//!   (usvg outlines every stroke) or in relative units, a viewBox beyond f32. It then computes, with every `<use>`
//!   expanded, the elements, the nesting depth, the vertices (path commands, an arc counted as the curves it becomes),
//!   the bytes usvg would read or copy, the XML nodes it visits, the attribute scans its tree makes, the text
//!   simplecss rescans, the selector work and the gradient copies, and refuses a file over [`MAX_SVG_ELEMENTS`],
//!   [`MAX_SVG_DEPTH`], [`MAX_IMPORT_POINTS`] / [`MAX_ENTITY_POINTS`], [`MAX_SVG_EXPANDED_BYTES`],
//!   [`MAX_SVG_EXPANDED_NODES`], [`MAX_SVG_ATTRIBUTE_WORK`], [`MAX_SVG_STYLE_SCAN`], [`MAX_SVG_CSS_MATCHES`] or
//!   [`MAX_SVG_GRADIENT_COPIES`]. What usvg drops without a word (`<foreignObject>` content, failed conditional
//!   attributes outside `<switch>`, degenerate transforms, coordinates beyond f32) is counted for the notices.
//!
//! The path-data and arc arithmetic mirrors svgtypes 0.15 and kurbo 0.11 (the versions usvg 0.45.1 uses) so that the
//! vertex count is exact; where mirroring is not needed for safety the guard over-counts instead.

use std::borrow::Cow;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::ops::Range;

use usvg::roxmltree::{self, Node, NodeId};

use crate::{
    Error, MAX_ENTITY_POINTS, MAX_IMPORT_POINTS, MAX_SVG_ARC_CURVES, MAX_SVG_ATTRIBUTE_PAIRS, MAX_SVG_ATTRIBUTE_WORK, MAX_SVG_ATTRIBUTES, MAX_SVG_CSS_MATCHES,
    MAX_SVG_DEPTH, MAX_SVG_ELEMENTS, MAX_SVG_EMPTY_SEGMENTS, MAX_SVG_EXPANDED_BYTES, MAX_SVG_EXPANDED_NODES, MAX_SVG_EXPANDED_SOURCE_BYTES,
    MAX_SVG_GRADIENT_COPIES, MAX_SVG_HREF_CHAIN, MAX_SVG_NAME_BYTES, MAX_SVG_NAMESPACES, MAX_SVG_PATH_DATA_BYTES, MAX_SVG_STROKE_WIDTH, MAX_SVG_STYLE_BYTES,
    MAX_SVG_STYLE_SCAN, Result,
};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed { format: "SVG", message: message.into() }
}

fn too_deep(context: &str) -> Error {
    Error::TooLarge(format!("SVG elements are nested more than {MAX_SVG_DEPTH} deep{context}"))
}

// ---- raw markup ---------------------------------------------------------------------------------------------------

/// Tokenizes the raw text, refuses what roxmltree must never see, and returns the text to parse: the source itself,
/// or a copy with its (subset-free) DOCTYPE replaced by spaces so roxmltree can run with `allow_dtd: false`.
pub(crate) fn prepare(source: &str) -> Result<Cow<'_, str>> {
    let Some(doctype) = scan_markup(source.as_bytes())? else { return Ok(Cow::Borrowed(source)) };
    let mut bytes = source.as_bytes().to_vec();
    if let Some(span) = bytes.get_mut(doctype) {
        span.fill(b' ');
    }
    String::from_utf8(bytes).map(Cow::Owned).map_err(|e| malformed(e.to_string()))
}

/// One pass over the markup, as an XML parser reads it: a comment ends at the first `-->`, a processing instruction
/// at the first `?>` (quotes mean nothing inside either), CDATA at the first `]]>`, a start tag at the first `>` that
/// is not inside a quoted attribute value. On any well-formed prefix this is the tokenization roxmltree makes, so
/// roxmltree can never nest deeper than this count before it stops. Every element counts as a level, a self-closing
/// leaf included. Start tags are also measured for roxmltree's own quadratic checks: attributes per tag (each is
/// compared with all before it) and namespace declarations in all (each element resolves its namespaces by linear
/// scans). Returns the DOCTYPE's byte range, if there is one.
fn scan_markup(bytes: &[u8]) -> Result<Option<Range<usize>>> {
    let mut depth = 0usize;
    let mut seen_element = false;
    let mut doctype = None;
    let mut i = 0usize;
    let (mut attribute_pairs, mut namespaces) = (0u64, 0usize);
    while let Some(offset) = bytes.get(i..).and_then(|rest| rest.iter().position(|&b| b == b'<')) {
        let start = i + offset;
        let rest = bytes.get(start..).unwrap_or_default();
        i = if rest.starts_with(b"<?") {
            after(bytes, start + 2, b"?>").ok_or_else(|| malformed("a processing instruction is not closed"))?
        } else if rest.starts_with(b"<!--") {
            after(bytes, start + 4, b"-->").ok_or_else(|| malformed("a comment is not closed"))?
        } else if rest.starts_with(b"<![CDATA[") {
            after(bytes, start + 9, b"]]>").ok_or_else(|| malformed("a CDATA section is not closed"))?
        } else if rest.starts_with(b"<!DOCTYPE") {
            if seen_element || doctype.is_some() {
                return Err(malformed("a DOCTYPE may appear only once, before the root element"));
            }
            let end = doctype_end(bytes, start + 9)?;
            doctype = Some(start..end);
            end
        } else if rest.starts_with(b"<!") {
            return Err(malformed("unexpected <! markup (only comments, CDATA sections and one DOCTYPE are allowed)"));
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            after(bytes, start + 2, b">").ok_or_else(|| malformed("an end tag is not closed"))?
        } else {
            let tag = tag_end(bytes, start + 1).ok_or_else(|| malformed("a tag is not closed"))?;
            if depth >= MAX_SVG_DEPTH {
                return Err(too_deep(""));
            }
            if tag.attributes > MAX_SVG_ATTRIBUTES {
                return Err(Error::TooLarge(format!("an SVG element has {} attributes; the limit is {MAX_SVG_ATTRIBUTES}", tag.attributes)));
            }
            let n = tag.attributes as u64;
            attribute_pairs = attribute_pairs.saturating_add(n * n.saturating_sub(1) / 2);
            if attribute_pairs > MAX_SVG_ATTRIBUTE_PAIRS {
                return Err(Error::TooLarge(format!(
                    "the SVG's elements carry too many attributes each (checking them for duplicates takes more than {MAX_SVG_ATTRIBUTE_PAIRS} comparisons)"
                )));
            }
            namespaces = namespaces.saturating_add(tag.namespaces);
            if namespaces > MAX_SVG_NAMESPACES {
                return Err(Error::TooLarge(format!("the SVG declares more than {MAX_SVG_NAMESPACES} XML namespaces (xmlns attributes)")));
            }
            seen_element = true;
            if !tag.self_closing {
                depth += 1;
            }
            tag.end
        };
    }
    Ok(doctype)
}

/// A start tag as [`tag_end`] reads it.
struct Tag {
    /// Index after its closing `>`.
    end: usize,
    self_closing: bool,
    /// Its `=` signs outside quotes: one per attribute (never fewer).
    attributes: usize,
    /// Its `xmlns` / `xmlns:*` attributes.
    namespaces: usize,
}

/// Index just after the first `needle` at or after `from`.
fn after(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let rest = bytes.get(from..)?;
    rest.windows(needle.len()).position(|window| window == needle).map(|p| from + p + needle.len())
}

/// The start tag whose name begins at `from`, up to the first `>` outside a quoted attribute value.
fn tag_end(bytes: &[u8], from: usize) -> Option<Tag> {
    let mut quote: Option<u8> = None;
    let mut previous = 0u8;
    let (mut attributes, mut namespaces) = (0usize, 0usize);
    for (offset, &b) in bytes.get(from..)?.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(Tag { end: from + offset + 1, self_closing: previous == b'/', attributes, namespaces }),
            None if b == b'=' => {
                attributes += 1;
                namespaces += usize::from(declares_namespace(bytes, from + offset));
            }
            None => {}
        }
        previous = b;
    }
    None
}

/// Whether the attribute name before the `=` at `eq` is `xmlns` or `xmlns:…`. The walk back stops at whitespace, a
/// quote, `=` or `<`, so the walks of one tag never overlap.
fn declares_namespace(bytes: &[u8], eq: usize) -> bool {
    let mut end = eq;
    while end > 0 && bytes.get(end - 1).is_some_and(u8::is_ascii_whitespace) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes.get(start - 1).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b'=' | b'<')) {
        start -= 1;
    }
    let name = bytes.get(start..end).unwrap_or_default();
    name == b"xmlns" || name.starts_with(b"xmlns:")
}

/// Index after the `>` closing a DOCTYPE whose body starts at `from`. Only a name and quoted public / system
/// literals are accepted; an internal subset (`[`) is refused.
fn doctype_end(bytes: &[u8], from: usize) -> Result<usize> {
    let mut quote: Option<u8> = None;
    for (offset, &b) in bytes.get(from..).unwrap_or_default().iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' => return Err(internal_subset(bytes, from + offset + 1)),
                b'<' => return Err(malformed("the DOCTYPE is malformed")),
                b'>' => return Ok(from + offset + 1),
                _ => {}
            },
        }
    }
    Err(malformed("the DOCTYPE is not closed"))
}

/// The refusal of a DOCTYPE internal subset (no entity is ever defined, so none is ever expanded). When markup in
/// one of its quoted values nests deeper than [`MAX_SVG_DEPTH`] (counting the root element the entity would be
/// expanded into), the refusal says so; otherwise it names the subset. Either way the file is refused, so this scan
/// only chooses the message.
fn internal_subset(bytes: &[u8], from: usize) -> Error {
    let mut i = from;
    while let Some(&b) = bytes.get(i) {
        let rest = bytes.get(i..).unwrap_or_default();
        let skip = if rest.starts_with(b"<!--") {
            after(bytes, i + 4, b"-->")
        } else if rest.starts_with(b"<?") {
            after(bytes, i + 2, b"?>")
        } else if b == b'"' || b == b'\'' {
            let close = bytes.get(i + 1..).and_then(|r| r.iter().position(|&c| c == b)).map_or(bytes.len(), |p| i + 1 + p);
            if markup_nesting(bytes.get(i + 1..close).unwrap_or_default()) + 1 > MAX_SVG_DEPTH {
                return too_deep(" (in a DOCTYPE entity)");
            }
            Some(close + 1)
        } else if b == b']' {
            break;
        } else {
            Some(i + 1)
        };
        let Some(next) = skip else { break };
        i = next;
    }
    // Older Adobe Illustrator files rely on the subset: deleting it leaves their `&ns_…;` references undefined, so the
    // file has to be exported again instead.
    Error::NotImplemented(
        "SVG files whose DOCTYPE has an internal subset (entity declarations in [ … ], as in SVGs saved by older Adobe Illustrator versions); \
         export the file again without entities (Illustrator: Save As SVG 1.1 with \"Preserve Illustrator Editing Capabilities\" off, or File > \
         Export > Export As SVG)",
    )
}

/// The deepest element nesting in a markup fragment (the same tokenization as [`scan_markup`], never refusing).
fn markup_nesting(fragment: &[u8]) -> usize {
    let (mut depth, mut deepest, mut i) = (0usize, 0usize, 0usize);
    while let Some(offset) = fragment.get(i..).and_then(|rest| rest.iter().position(|&b| b == b'<')) {
        let start = i + offset;
        let rest = fragment.get(start..).unwrap_or_default();
        let next = if rest.starts_with(b"<?") {
            after(fragment, start + 2, b"?>")
        } else if rest.starts_with(b"<!--") {
            after(fragment, start + 4, b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            after(fragment, start + 9, b"]]>")
        } else if rest.starts_with(b"<!") || rest.starts_with(b"</") {
            if rest.starts_with(b"</") {
                depth = depth.saturating_sub(1);
            }
            after(fragment, start + 2, b">")
        } else {
            tag_end(fragment, start + 1).map(|tag| {
                deepest = deepest.max(depth + 1);
                if !tag.self_closing {
                    depth += 1;
                }
                tag.end
            })
        };
        let Some(next) = next else { break };
        i = next;
    }
    deepest
}

// ---- the parsed tree ----------------------------------------------------------------------------------------------

/// What the parsed SVG holds that the import reports in its notices.
#[derive(Debug, Default)]
pub(crate) struct SvgFacts {
    pub images: usize,
    pub texts: usize,
    /// Elements hidden with `display: none` (attribute or `style`); usvg drops them with their contents.
    pub display_none: usize,
    /// A `<style>` sheet sets `display: none` (usvg drops what it matches).
    pub sheet_hides: bool,
    /// `<foreignObject>` elements: usvg skips them and their (HTML or other non-SVG) content.
    pub foreign_objects: usize,
    /// Elements outside `<switch>` whose `requiredFeatures`, `requiredExtensions` or `systemLanguage` condition fails:
    /// usvg drops them with their contents.
    pub failed_conditions: usize,
    /// Elements with a geometry number beyond f32 (usvg converts to f32 and drops what becomes infinite).
    pub f32_overflows: usize,
    /// Elements whose `transform` attribute scales to zero or is not finite in f32: usvg drops them.
    pub degenerate_transforms: usize,
}

/// Elements by id: `all` holds every element carrying an id usvg resolves `url(#…)` and gradient `href`s against
/// (`id`, `xml:id`, `svg:id`; conservative: usvg picks one of them); `first_plain` is usvg's own map for `<use>`,
/// the first element in document order with a plain `id`.
struct Ids<'a, 'input> {
    all: HashMap<&'a str, Vec<Node<'a, 'input>>>,
    first_plain: HashMap<&'a str, Node<'a, 'input>>,
}

/// Attributes usvg reads (no namespace, SVG, XLink or XML); it ignores every other namespace.
fn read_by_usvg(attribute: &roxmltree::Attribute<'_, '_>) -> bool {
    matches!(attribute.namespace(), None | Some(SVG_NS | XLINK_NS | XML_NS))
}

/// Elements usvg converts (no namespace or SVG).
fn svg_element(node: Node<'_, '_>) -> bool {
    node.is_element() && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
}

fn is_gradient(node: Node<'_, '_>) -> bool {
    svg_element(node) && matches!(node.tag_name().name(), "linearGradient" | "radialGradient")
}

/// Values of an element's attributes with this local name that usvg reads.
fn attribute_values<'a>(node: Node<'a, '_>, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    node.attributes().filter(move |a| a.name() == name && read_by_usvg(a)).map(|a| a.value())
}

/// Elements usvg turns into renderable content, containers or paint servers. Anything else in the SVG namespace
/// (and any element in another namespace) makes usvg drop the whole subtree; browsers keep rendering SVG children
/// below it, so such an element is only acceptable when it contains no SVG elements at all.
const ALLOWED_ELEMENTS: [&str; 21] = [
    "svg",
    "g",
    "a",
    "defs",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "use",
    "text",
    "tspan",
    "textPath",
    "image",
    "foreignObject",
    "linearGradient",
    "radialGradient",
    "stop",
    "style",
];

/// Whether an element that usvg skips entirely (an unknown SVG element like `<font>`, `<title>` / `<desc>` /
/// `<metadata>`, or anything in another namespace) hides SVG elements a browser would render.
fn inert_check(node: Node<'_, '_>) -> Result<()> {
    if node.descendants().any(|d| d.id() != node.id() && svg_element(d)) {
        return Err(malformed(format!(
            "an SVG <{}> element (in an unsupported namespace or an unsupported element) contains SVG content that is not imported; ungroup or delete it",
            node.tag_name().name()
        )));
    }
    Ok(())
}

/// Checks the parsed tree before usvg converts it (see the module docs).
pub(crate) fn check(doc: &roxmltree::Document<'_>) -> Result<SvgFacts> {
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(malformed(format!("the root element is <{}>, not <svg>", root.tag_name().name())));
    }
    for node in doc.descendants() {
        // A browser loads the external sheet; usvg would apply only the internal ones — different geometry.
        if node.is_pi() && node.pi().is_some_and(|pi| pi.target.eq_ignore_ascii_case("xml-stylesheet")) {
            return Err(Error::NotImplemented(
                "an <?xml-stylesheet?> processing instruction (an external style sheet); inline it as a <style> element or use presentation attributes",
            ));
        }
    }
    let mut ids = Ids { all: HashMap::new(), first_plain: HashMap::new() };
    for node in doc.descendants().filter(Node::is_element) {
        for id in attribute_values(node, "id") {
            ids.all.entry(id).or_default().push(node);
        }
        if let Some(id) = node.attribute("id") {
            ids.first_plain.entry(id).or_insert(node);
        }
    }
    let mut facts = SvgFacts::default();
    let mut styles = Styles::default();
    for node in doc.descendants().filter(Node::is_element) {
        let svg = svg_element(node);
        let name = node.tag_name().name();
        match name {
            "image" => facts.images += 1,
            "text" => facts.texts += 1,
            "tref" => return Err(Error::NotImplemented("SVG <tref> elements (they copy text from elsewhere in the file); delete them")),
            "feImage" if attribute_values(node, "href").any(|href| href.trim_start().starts_with('#')) => {
                return Err(Error::NotImplemented("SVG <feImage> elements that render another element; delete the filter"));
            }
            "foreignObject" if svg => facts.foreign_objects += 1,
            // A nested <svg> without both sizes is clipped by browsers to its enclosing viewport but not by usvg (one
            // with both is clipped by usvg too, and refused after conversion).
            "svg"
                if svg && node.id() != root.id() && (attribute_values(node, "width").next().is_none() || attribute_values(node, "height").next().is_none()) =>
            {
                return Err(Error::NotImplemented(
                    "nested SVG <svg> elements without both width and height (other viewers clip them to the enclosing viewport); give them a size or ungroup them before importing",
                ));
            }
            "style" => {
                // usvg reads any element named `style` (whatever its namespace), but honours `type` exactly and
                // ignores `media` and `disabled`, which a browser applies: they are refused where they diverge.
                let mut parses_as_css = true;
                for attribute in node.attributes().filter(|a| read_by_usvg(a)) {
                    match attribute.name() {
                        "media" => {
                            let matches_screen =
                                attribute.value().split(',').any(|query| matches!(query.trim().to_ascii_lowercase().as_str(), "" | "all" | "screen"));
                            if !matches_screen {
                                return Err(Error::NotImplemented(
                                    "an SVG style sheet with a media condition that does not match on screen (media=\"…\"); a browser would not apply it but the parser would",
                                ));
                            }
                        }
                        "type" => {
                            let value = attribute.value().trim();
                            if value != "text/css" && value.eq_ignore_ascii_case("text/css") {
                                return Err(Error::NotImplemented("a <style> type that is not exactly \"text/css\"; the parser is case-sensitive here"));
                            }
                            parses_as_css &= value == "text/css";
                        }
                        "disabled" => {
                            return Err(Error::NotImplemented("a disabled SVG style sheet (a browser would not apply it but the parser would)"));
                        }
                        // Titled style sheets are alternate sets in a browser (only one set applies); the parser
                        // would apply them all.
                        "title" if !attribute.value().is_empty() => {
                            return Err(Error::NotImplemented("an SVG style sheet with a title (a browser treats it as an alternate set)"));
                        }
                        _ => {}
                    }
                }
                if parses_as_css {
                    for text in node.descendants().filter(|t| t.is_text()).filter_map(|t| t.text()) {
                        styles.sheet_bytes = styles.sheet_bytes.saturating_add(text.len());
                        if styles.sheet_bytes > MAX_SVG_STYLE_BYTES {
                            return Err(Error::TooLarge(format!("SVG style sheets hold more than {MAX_SVG_STYLE_BYTES} bytes")));
                        }
                        styles.sheet_scan = styles.sheet_scan.saturating_add(css_rescans(text));
                        styles.check_sheet(text)?;
                    }
                }
            }
            _ => {}
        }
        if svg {
            match name {
                // Rendered features the importer cannot reproduce (kept refused, named for the user).
                "clipPath" | "mask" => {
                    return Err(Error::NotImplemented("SVG <clipPath> and <mask> elements; release them in the editor before importing"));
                }
                "marker" => {
                    return Err(Error::NotImplemented("SVG <marker> elements; convert them to paths before importing"));
                }
                "pattern" => {
                    return Err(Error::NotImplemented("SVG <pattern> elements (pattern fills and strokes); convert them to plain colours before importing"));
                }
                "symbol" => {
                    return Err(Error::NotImplemented("SVG <symbol> elements (viewport-scaled clones); ungroup them before importing"));
                }
                // Browsers evaluate <switch> conditions with the user's own language; a wrong branch could burn.
                "switch" => {
                    return Err(Error::NotImplemented("SVG <switch> conditional content; delete the branches that do not apply"));
                }
                "animate" | "animateColor" | "animateMotion" | "animateTransform" | "set" | "mpath" => {
                    return Err(Error::NotImplemented("SVG animation elements (<animate>, <set>, …); export the static result"));
                }
                "script" | "handler" | "listener" | "discard" => {
                    return Err(Error::NotImplemented("SVG scripting and event elements (<script>, <handler>, <listener>, <discard>); remove them"));
                }
                "link" => {
                    return Err(Error::NotImplemented("SVG <link> elements (external style sheets); use a <style> element or presentation attributes"));
                }
                "filter" => {
                    return Err(Error::NotImplemented("SVG <filter> elements (<feGaussianBlur>, <feImage>, …); delete the filter"));
                }
                _ if name.len() > 2 && name.starts_with("fe") && name.as_bytes().get(2).is_some_and(u8::is_ascii_uppercase) => {
                    return Err(Error::NotImplemented("SVG filter elements (<feGaussianBlur>, <feImage>, …); delete the filter"));
                }
                _ if ALLOWED_ELEMENTS.contains(&name) => {}
                _ => inert_check(node)?,
            }
        } else {
            inert_check(node)?;
        }
        facts.failed_conditions +=
            usize::from(svg && condition_fails(node) && !node.parent_element().is_some_and(|p| svg_element(p) && p.tag_name().name() == "switch"));
        let (mut hidden, mut degenerate, mut overflow) = (false, false, false);
        for attribute in node.attributes().filter(read_by_usvg) {
            let (name, value) = (attribute.name(), attribute.value());
            match name {
                "style" => {
                    if value.len() > MAX_SVG_STYLE_BYTES {
                        return Err(Error::TooLarge(format!("an SVG style attribute holds {} bytes; the limit is {MAX_SVG_STYLE_BYTES}", value.len())));
                    }
                    let declared = styles.check_declarations(value)?;
                    hidden |= declared.hides;
                }
                "display" => hidden |= value.trim() == "none",
                "d" | "points" if value.len() > MAX_SVG_PATH_DATA_BYTES => {
                    return Err(Error::TooLarge(format!("an SVG {name} attribute holds {} bytes; the limit is {MAX_SVG_PATH_DATA_BYTES}", value.len())));
                }
                "points" if !points_complete(value) => {
                    return Err(malformed(format!(
                        "an SVG points list stops parsing before its end (\"{}\"); a browser drops the whole element but the parser would keep the prefix — fix the list",
                        value.get(..40).unwrap_or(value)
                    )));
                }
                "transform" => degenerate |= degenerate_transform(value),
                "transform-origin" | "transform-box" => {
                    return Err(malformed(format!("the SVG {name} attribute (transform origins and boxes); use a plain transform")));
                }
                "viewBox" => {
                    // usvg ignores a viewBox it cannot hold in f32 and would draw the content at the wrong scale.
                    if overflows_f32(value) {
                        return Err(Error::TooLarge("an SVG viewBox holds a number beyond ±3.4e38 (single precision)".to_owned()));
                    }
                    check_viewbox(value)?;
                }
                _ => styles.check_property(name, value)?,
            }
            if LENGTH_ATTRIBUTES.contains(&name) {
                check_unit_suffixes(name, value)?;
                if SINGLE_LENGTH_ATTRIBUTES.contains(&name) && value != value.trim_end() {
                    return Err(malformed(format!(
                        "an SVG {name} value of \"{value}\" ends in whitespace; a browser trims it but the parser drops the whole value — remove the whitespace"
                    )));
                }
            }
            overflow |= GEOMETRY_ATTRIBUTES.contains(&name) && overflows_f32(value);
        }
        facts.display_none += usize::from(hidden);
        facts.degenerate_transforms += usize::from(svg && degenerate);
        facts.f32_overflows += usize::from(svg && overflow);
    }
    facts.sheet_hides = styles.sheet_hides;
    if styles.selector_parts > 0 {
        // simplecss compares every selector with every element's `class` and `id`: keep each comparison short.
        for node in doc.descendants().filter(Node::is_element) {
            if let Some(value) = node.attributes().filter(|a| matches!(a.name(), "class" | "id")).map(|a| a.value()).find(|v| v.len() > MAX_SVG_NAME_BYTES) {
                return Err(Error::TooLarge(format!(
                    "an SVG class or id of {} bytes in a file with a style sheet; the limit is {MAX_SVG_NAME_BYTES}",
                    value.len()
                )));
            }
        }
    }
    let gradients = paint_servers(&styles.paints, &ids)?;
    let sheet = Sheet { declarations: styles.sheet_declarations, paints: styles.sheet_paints };
    let expanded = expand(root, &ids, &sheet)?;
    // simplecss tests every selector part against every element, reading the element's attributes for each part.
    let matches = expanded.attribute_slots.saturating_mul(styles.selector_parts);
    if matches > MAX_SVG_CSS_MATCHES {
        return Err(Error::TooLarge(format!(
            "matching the SVG style sheet's {} selector parts against {} elements takes more than {MAX_SVG_CSS_MATCHES} comparisons; export with presentation attributes",
            styles.selector_parts, expanded.elements
        )));
    }
    let rescans = expanded.style_scan.saturating_add(styles.sheet_scan);
    if rescans > MAX_SVG_STYLE_SCAN {
        return Err(Error::TooLarge(format!(
            "the SVG's style attributes and sheets are too long for their number of declarations (parsing them would rescan about {rescans} bytes; the limit is {MAX_SVG_STYLE_SCAN}); export with presentation attributes"
        )));
    }
    // usvg copies an object-bounding-box gradient (and any gradient reached through a context paint) for every path it
    // paints, else shares one per gradient, then deduplicates them with a linear scan each.
    if !styles.paints.is_empty() {
        let copies = if gradients.object_units || styles.context_paint { expanded.gradient_painted } else { gradients.distinct };
        if copies > MAX_SVG_GRADIENT_COPIES {
            return Err(Error::TooLarge(format!(
                "the SVG paints {copies} elements with gradients that usvg copies per element (objectBoundingBox units or context paints), or uses that many gradients; the limit is {MAX_SVG_GRADIENT_COPIES}; export gradients in userSpaceOnUse units or as plain colours"
            )));
        }
    }
    // What usvg copies into every element: the style sheet's declarations (the bound assumes every rule matches), a
    // dash array (`stroke-dasharray` is inherited and re-parsed per path, f32 values, odd lists doubled) and, for each
    // of fill and stroke, a gradient with its stops (copied per path in object-bounding-box units).
    let per_element = (styles.declaration_bytes as u64)
        .saturating_add(styles.dash_values.saturating_mul(8).saturating_add(32))
        .saturating_add(if styles.paints.is_empty() { 0 } else { gradients.stops.saturating_mul(16).saturating_add(256).saturating_mul(2) });
    let copied = expanded.elements.saturating_mul(per_element).saturating_add(expanded.bytes);
    if copied > MAX_SVG_EXPANDED_BYTES {
        return Err(Error::TooLarge(format!(
            "the SVG would make usvg read or copy about {copied} bytes once <use> references, styles and gradients are expanded; the limit is {MAX_SVG_EXPANDED_BYTES}"
        )));
    }
    Ok(facts)
}

// ---- styles and references ----------------------------------------------------------------------------------------

/// Presentation properties that take a `url(…)` usvg follows into content (clip paths, masks, filters, markers).
fn reference_refusal(property: &str) -> Error {
    match property {
        "clip-path" | "mask" => Error::NotImplemented("SVG clipping paths and masks (clip-path, mask); release them in the editor before importing"),
        "filter" => Error::NotImplemented("SVG filter effects (filter: url(…)); remove them before importing"),
        p if p.starts_with("marker") => Error::NotImplemented("SVG markers (marker-start, marker-mid, marker-end); convert them to paths before importing"),
        _ => Error::NotImplemented("SVG url(…) references other than gradient fills and strokes"),
    }
}

/// How many `url(` a value holds, ignoring ASCII case and whitespace (more than usvg reads, never less).
fn url_count(value: &str) -> usize {
    let compact: Vec<u8> = value.bytes().filter(|b| !b.is_ascii_whitespace()).map(|b| b.to_ascii_lowercase()).collect();
    compact.windows(4).filter(|window| window == b"url(").count()
}

/// The id a value starting with `url(#id)` names: svgtypes 0.15 `Stream::parse_func_iri`, which `Paint::from_str`
/// and `FuncIRI::from_str` use (anything after the closing `)` is a paint fallback and does not change the id).
fn func_iri(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    let spaces = |mut i: usize| {
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        i
    };
    let mut i = spaces(0);
    if !bytes.get(i..)?.starts_with(b"url(") {
        return None;
    }
    i = spaces(i + 4);
    let quote = match bytes.get(i) {
        Some(&q @ (b'\'' | b'"')) => {
            i = spaces(i + 1);
            Some(q)
        }
        _ => None,
    };
    if bytes.get(i) != Some(&b'#') {
        return None;
    }
    i += 1;
    let start = i;
    while let Some(&b) = bytes.get(i) {
        if quote.map_or(b == b' ' || b == b')', |q| b == q) {
            break;
        }
        i += 1;
    }
    let link = value.get(start..i)?;
    let link = if quote.is_some() { link.trim_end() } else { link };
    if link.is_empty() || link.contains(['\'', '"']) {
        return None;
    }
    i = spaces(i);
    if let Some(q) = quote {
        if bytes.get(i) != Some(&q) {
            return None;
        }
        i = spaces(i + 1);
    }
    (bytes.get(i) == Some(&b')')).then_some(link)
}

/// The ids an `href` may name: svgtypes `IRI` (optional spaces, `#`, then up to the first space) and, to stay
/// conservative, everything after `#` with the surrounding whitespace trimmed.
fn iri_keys(value: &str) -> [Option<&str>; 2] {
    let mirror = value.trim_start_matches([' ', '\t', '\n', '\r']).strip_prefix('#').map(|rest| rest.split(' ').next().unwrap_or(rest));
    [mirror, value.trim().strip_prefix('#')]
}

/// The elements a `<use>` may instantiate: usvg reads `xlink:href`, else `href`, and takes the first element with
/// that plain `id`; every `href` (any namespace) and both id spellings are taken here, so at most four.
fn use_targets<'a, 'input>(node: Node<'a, 'input>, ids: &Ids<'a, 'input>) -> Vec<Node<'a, 'input>> {
    let mut out: Vec<Node<'a, 'input>> = Vec::new();
    for attribute in node.attributes().filter(|a| a.name() == "href") {
        for key in iri_keys(attribute.value()).into_iter().flatten() {
            if let Some(&target) = ids.first_plain.get(key)
                && !out.iter().any(|n| n.id() == target.id())
            {
                out.push(target);
            }
        }
    }
    out
}

/// The gradients a gradient's `href` names; an id several elements share is refused (the chain must be one chain).
fn gradient_targets<'a, 'input>(node: Node<'a, 'input>, ids: &Ids<'a, 'input>) -> Result<Vec<Node<'a, 'input>>> {
    let mut out: Vec<Node<'a, 'input>> = Vec::new();
    for attribute in node.attributes().filter(|a| a.name() == "href" && read_by_usvg(a)) {
        for key in iri_keys(attribute.value()).into_iter().flatten() {
            match ids.all.get(key).map(Vec::as_slice).unwrap_or_default() {
                [] => {}
                [target] => {
                    if !out.iter().any(|n| n.id() == target.id()) {
                        out.push(*target);
                    }
                }
                _ => return Err(Error::NotImplemented("SVG gradients whose href names an id that several elements share")),
            }
        }
    }
    Ok(out)
}

/// Styles and paint references gathered from attributes, `style` attributes and `<style>` sheets.
#[derive(Default)]
struct Styles<'a> {
    /// Ids named by `fill` / `stroke` `url(…)`.
    paints: Vec<&'a str>,
    /// A `fill` / `stroke` is `context-fill` or `context-stroke` (usvg copies the context's paint per path).
    context_paint: bool,
    sheet_bytes: usize,
    /// Simple-selector parts of all sheet selectors (`path.a` and `.a.b` are two each).
    selector_parts: u64,
    /// Declarations in all sheet blocks (counted at every `;`).
    sheet_declarations: u64,
    declaration_bytes: usize,
    /// Bytes simplecss rescans while parsing the sheets (see [`css_rescans`]).
    sheet_scan: u64,
    /// A sheet block paints with a gradient.
    sheet_paints: bool,
    /// The most numbers any `stroke-dasharray` holds.
    dash_values: u64,
    sheet_hides: bool,
}

/// What a `style` attribute or sheet block declares that the import reports.
#[derive(Default)]
struct Declared {
    hides: bool,
}

/// Properties that clip or mask what an element shows (usvg reads `clip-path` and `mask`; browsers also honour the
/// others in a `style`). Only `none` is accepted: usvg ignores the CSS shape and image forms, which would burn the
/// clipped-away geometry.
const CLIPPING_PROPERTIES: [&str; 6] = ["clip-path", "mask", "mask-image", "-webkit-clip-path", "-webkit-mask", "-webkit-mask-image"];

impl<'a> Styles<'a> {
    /// One property, from an attribute or a declaration (`property` lower-case for declarations).
    fn check_property(&mut self, property: &str, value: &'a str) -> Result<()> {
        match property {
            "fill" | "stroke" => {
                let urls = url_count(value);
                if urls > 0 {
                    // Exactly one `url(` and it must start the value (svgtypes `Paint`); anything else is refused
                    // rather than guessed at.
                    let id = func_iri(value.trim()).filter(|_| urls == 1).ok_or_else(|| reference_refusal(property))?;
                    self.paints.push(id);
                }
                self.context_paint |= matches!(value.trim(), "context-fill" | "context-stroke");
            }
            "stroke-dasharray" => {
                // Every digit counted as a value: never fewer than svgtypes parses.
                let numbers = value.bytes().filter(u8::is_ascii_digit).count() as u64;
                self.dash_values = self.dash_values.max(numbers);
            }
            "stroke-width" => {
                // The largest width anywhere bounds every stroke usvg outlines, inherited ones included.
                let width = absolute_length(value).ok_or(Error::NotImplemented("SVG stroke widths in %, em or ex; use absolute units (px, mm, …)"))?;
                if width > MAX_SVG_STROKE_WIDTH {
                    return Err(Error::TooLarge(format!("an SVG stroke-width of {width} user units; the limit is {MAX_SVG_STROKE_WIDTH}")));
                }
            }
            p if CLIPPING_PROPERTIES.contains(&p) && !(value.trim().is_empty() || value.trim().eq_ignore_ascii_case("none")) => {
                return Err(reference_refusal("clip-path"));
            }
            // A filter or marker with any value but none changes what a browser draws (CSS filter functions need no
            // url(…) reference).
            "filter" | "marker" | "marker-start" | "marker-mid" | "marker-end" => {
                let value = value.trim();
                if !(value.is_empty() || value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("inherit")) {
                    return Err(reference_refusal(property));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Declarations of a `style` attribute or a sheet block, split at every `;` and the first `:` (simplecss splits
    /// at the same `;` unless one sits in a string or a function, and then this split sees a declaration that a
    /// `url(` cannot hide in: every `url(` must be the whole start of a `fill` / `stroke` value, or the import is
    /// refused).
    fn check_declarations(&mut self, text: &'a str) -> Result<Declared> {
        if text.contains('\\') {
            return Err(Error::NotImplemented("backslash escapes in SVG style attributes and style sheets"));
        }
        if url_count(text) > 0 && text.contains("/*") {
            return Err(Error::NotImplemented("comments in SVG styles that hold url(…) references"));
        }
        let mut declared = Declared::default();
        for declaration in text.split(';') {
            let (name, value) = declaration.split_once(':').unwrap_or((declaration, ""));
            if url_count(name) > 0 {
                return Err(reference_refusal(""));
            }
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            let value = strip_important(value);
            match name.as_str() {
                "display" => declared.hides |= value == "none",
                // A CSS transform is a different grammar than the transform attribute (units, translateX, 3-D); the
                // parser reads only the SVG form, so anything a browser could interpret differently is refused.
                "transform" | "translate" | "rotate" | "scale" | "transform-origin" | "transform-box" | "perspective" | "rotate3d" => {
                    if !matches!(value, "" | "none" | "inherit" | "initial" | "unset" | "revert") {
                        return Err(malformed(format!("a CSS transform ({name}:) in a style attribute or style sheet; move it to a transform attribute")));
                    }
                }
                // SVG2 geometry properties: browsers apply them, the parser only reads the attributes.
                _ if GEOMETRY_CSS_PROPERTIES.contains(&name.as_str()) => {
                    return Err(malformed(format!("the CSS geometry property {name}: in a style attribute or style sheet; set it as an attribute")));
                }
                // Animation, transitions and motion move or hide geometry in a browser but not here.
                _ if name.starts_with("animation") || name.starts_with("transition") || name.starts_with("offset") || name.starts_with("motion") => {
                    return Err(malformed(format!("the CSS property {name}: (animation, transition or motion); export the static result")));
                }
                _ if url_count(value) > 0 && !matches!(name.as_str(), "fill" | "stroke") => return Err(reference_refusal(&name)),
                _ => {
                    check_unit_suffixes(&name, value)?;
                    self.check_property(&name, value)?;
                }
            }
        }
        Ok(declared)
    }

    /// A `<style>` sheet: rules `selectors { declarations }` only, every selector a single element, class or id
    /// selector (no combinators, attribute selectors or pseudo-classes: simplecss matches those by recursion and
    /// backtracking), no `@` rules, escapes or nested blocks.
    fn check_sheet(&mut self, css: &'a str) -> Result<()> {
        if css.contains('\\') || css.contains('@') {
            return Err(Error::NotImplemented("SVG style sheets with @-rules or backslash escapes; export with presentation attributes"));
        }
        let bytes = css.as_bytes();
        let mut i = 0usize;
        loop {
            i = skip_css_space(bytes, i)?;
            if i >= bytes.len() {
                return Ok(());
            }
            let open = bytes
                .get(i..)
                .and_then(|rest| rest.iter().position(|&b| b == b'{'))
                .map(|p| i + p)
                .ok_or_else(|| malformed("a style sheet rule has no { … } block"))?;
            for selector in css.get(i..open).unwrap_or_default().split(',') {
                let selector = selector.trim();
                if selector.is_empty() || !selector.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'#' | b'*')) {
                    return Err(Error::NotImplemented(
                        "SVG style sheet selectors other than element, class and id selectors (no combinators, attribute selectors or pseudo-classes)",
                    ));
                }
                // simplecss tests each part (`path`, `.a`, `#b`, `*`) on its own.
                self.selector_parts = self.selector_parts.saturating_add(1 + selector.bytes().filter(|b| matches!(b, b'.' | b'#')).count() as u64);
            }
            let close = css_block_end(bytes, open + 1)?;
            let block = css.get(open + 1..close).unwrap_or_default();
            self.declaration_bytes = self.declaration_bytes.saturating_add(block.len());
            self.sheet_declarations = self.sheet_declarations.saturating_add(1 + block.bytes().filter(|&b| b == b';').count() as u64);
            let paints = self.paints.len();
            let declared = self.check_declarations(block)?;
            self.sheet_paints |= self.paints.len() > paints;
            self.sheet_hides |= declared.hides;
            i = close + 1;
        }
    }
}

/// An upper bound of the bytes simplecss rescans while it tokenizes this style text: at every token it rejects (each
/// declaration's end, a number without a unit, a `#` before a digit) it computes a line and a column by scanning the
/// text from its start, so at most `len` bytes per rejection. Rejections are counted at every `;`, `#`, `+`, `-`, `.`,
/// `,` and digit run (never fewer than simplecss makes).
fn css_rescans(text: &str) -> u64 {
    let mut rejections = 2u64;
    let mut in_digits = false;
    for b in text.bytes() {
        let digit = b.is_ascii_digit();
        rejections += u64::from(digit && !in_digits) + u64::from(matches!(b, b';' | b'#' | b'+' | b'-' | b'.' | b','));
        in_digits = digit;
    }
    rejections.saturating_mul(text.len() as u64)
}

/// Index of the first byte after whitespace and `/* … */` comments.
fn skip_css_space(bytes: &[u8], mut i: usize) -> Result<usize> {
    loop {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if bytes.get(i..).is_some_and(|rest| rest.starts_with(b"/*")) {
            i = after(bytes, i + 2, b"*/").ok_or_else(|| malformed("a style sheet comment is not closed"))?;
        } else {
            return Ok(i);
        }
    }
}

/// Index of the `}` closing a block whose body starts at `from` (strings and comments honoured).
fn css_block_end(bytes: &[u8], from: usize) -> Result<usize> {
    let mut quote: Option<u8> = None;
    let mut i = from;
    while let Some(&b) = bytes.get(i) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    i = after(bytes, i + 2, b"*/").ok_or_else(|| malformed("a style sheet comment is not closed"))?;
                    continue;
                }
                b'{' => return Err(Error::NotImplemented("nested blocks in SVG style sheets")),
                b'}' => return Ok(i),
                _ => {}
            },
        }
        i += 1;
    }
    Err(malformed("a style sheet block is not closed"))
}

#[derive(Clone, Copy)]
enum Visit<T> {
    Active,
    Done(T),
}

/// The paint references must name gradients only (a pattern renders arbitrary content per painted path); each
/// gradient's `href` chain must name gradients, never loop (usvg would follow a loop forever) and stay within
/// [`MAX_SVG_HREF_CHAIN`].
fn paint_servers<'a, 'input>(paints: &[&str], ids: &Ids<'a, 'input>) -> Result<Gradients> {
    let mut memo: HashMap<NodeId, Visit<(usize, u64)>> = HashMap::new();
    let mut gradients = Gradients::default();
    let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    // Each id once: a file may name the same gradient from every element.
    let keys: std::collections::BTreeSet<&str> = paints.iter().flat_map(|id| [*id, id.trim()]).collect();
    for key in keys {
        for &target in ids.all.get(key).map(Vec::as_slice).unwrap_or_default() {
            if !is_gradient(target) {
                return Err(Error::NotImplemented("SVG pattern fills and strokes (paint servers other than gradients); convert them to plain colours"));
            }
            gradients.stops = gradients.stops.max(gradient_chain(target, ids, &mut memo)?);
            if seen.insert(target.id()) {
                gradients.distinct += 1;
                gradients.object_units |= !user_space_units(target, ids)?;
            }
        }
    }
    Ok(gradients)
}

/// The gradients the paints reference.
#[derive(Default)]
struct Gradients {
    /// The most `<stop>`s a referenced chain holds (usvg copies them per painted path).
    stops: u64,
    /// Distinct gradient elements referenced.
    distinct: u64,
    /// One of them may be in objectBoundingBox units (usvg then copies it for every path it paints).
    object_units: bool,
}

/// Whether usvg resolves this gradient's `gradientUnits` to `userSpaceOnUse`: the first gradient along its `href`
/// chain that has the attribute decides (usvg's default is objectBoundingBox). The chain is already known to be
/// short and loop-free; an ambiguous link (two targets) counts as objectBoundingBox.
fn user_space_units<'a, 'input>(start: Node<'a, 'input>, ids: &Ids<'a, 'input>) -> Result<bool> {
    let mut node = start;
    for _ in 0..=MAX_SVG_HREF_CHAIN {
        let mut units = attribute_values(node, "gradientUnits").peekable();
        if units.peek().is_some() {
            return Ok(units.all(|u| u == "userSpaceOnUse"));
        }
        match gradient_targets(node, ids)?.as_slice() {
            [next] => node = *next,
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// The stops along the longest `href` chain from `start` (iterative; refuses loops, long chains and non-gradients).
fn gradient_chain<'a, 'input>(start: Node<'a, 'input>, ids: &Ids<'a, 'input>, memo: &mut HashMap<NodeId, Visit<(usize, u64)>>) -> Result<u64> {
    let looped = || Error::Malformed { format: "SVG", message: "gradients reference each other in a loop (href)".to_owned() };
    let mut stack = vec![(start, false)];
    while let Some((node, expanded)) = stack.pop() {
        let targets = gradient_targets(node, ids)?;
        if !expanded {
            match memo.get(&node.id()) {
                Some(Visit::Done(_)) => continue,
                Some(Visit::Active) => return Err(looped()),
                None => {}
            }
            memo.insert(node.id(), Visit::Active);
            stack.push((node, true));
            for target in targets {
                if !is_gradient(target) {
                    return Err(Error::NotImplemented("SVG gradients whose href names an element that is not a gradient"));
                }
                match memo.get(&target.id()) {
                    Some(Visit::Active) => return Err(looped()),
                    Some(Visit::Done(_)) => {}
                    None => stack.push((target, false)),
                }
            }
            continue;
        }
        let own_stops = node.children().filter(|c| svg_element(*c) && c.tag_name().name() == "stop").count() as u64;
        let (mut chain, mut stops) = (1usize, own_stops);
        for target in targets {
            let Some(Visit::Done((c, s))) = memo.get(&target.id()).copied() else { return Err(looped()) };
            chain = chain.max(c + 1);
            stops = stops.max(own_stops.saturating_add(s));
        }
        if chain > MAX_SVG_HREF_CHAIN {
            return Err(Error::TooLarge(format!("an SVG gradient href chain is longer than {MAX_SVG_HREF_CHAIN}")));
        }
        memo.insert(node.id(), Visit::Done((chain, stops)));
    }
    match memo.get(&start.id()) {
        Some(Visit::Done((_, stops))) => Ok(*stops),
        _ => Err(looped()),
    }
}

// ---- expansion ----------------------------------------------------------------------------------------------------

/// What one element costs once its children and `<use>` targets are expanded.
#[derive(Clone, Copy, Default)]
struct Cost {
    elements: u64,
    depth: usize,
    vertices: u64,
    /// Attribute and text bytes usvg reads (and partly copies) per expanded element, and a fixed cost per attribute.
    bytes: u64,
    /// XML nodes usvg's tree builder visits (every child of every element: text, comments and processing
    /// instructions too).
    nodes: u64,
    /// Bytes simplecss rescans while it tokenizes `style` attributes (see [`css_rescans`]).
    style_scan: u64,
    /// Attribute scans in usvg's tree: each style declaration it applies scans the element's attributes, and each
    /// inherited-property lookup scans every ancestor's.
    attribute_work: u64,
    /// What selector matching reads: one per element plus one per attribute.
    attribute_slots: u64,
    /// Elements painted (directly or by inheritance) with a gradient.
    gradient_painted: u64,
}

/// What the style sheets add to every element.
struct Sheet {
    /// Declarations in all sheet blocks (the bound assumes every rule matches every element).
    declarations: u64,
    /// A sheet block paints with a gradient (the bound assumes it matches every element).
    paints: bool,
}

/// Distinct presentation attributes style declarations can add to one element (usvg's presentation attributes and
/// the shorthands' expansions, rounded up): an element's attributes in usvg's tree are its own plus at most these.
const CSS_ATTRIBUTES: u64 = 96;

/// Attributes per element that the style-declaration scan is not charged for: that part of the scan is bounded by
/// the declaration bytes copied per element ([`MAX_SVG_EXPANDED_BYTES`]).
const FREE_ATTRIBUTES: u64 = 16;

/// The fixed cost of one attribute usvg reads, in bytes (it looks up the name and copies a reference to the value).
const ATTRIBUTE_BYTES: u64 = 32;

fn recursive_use() -> Error {
    malformed("a <use> element references itself or one of its ancestors")
}

/// The cost of `root` with every `<use>` expanded (iterative and memoized: a hostile file cannot exhaust the stack or
/// make this pass itself expand anything). A `<use>` costs two levels (usvg adds a group for it).
fn expand<'a, 'input>(root: Node<'a, 'input>, ids: &Ids<'a, 'input>, sheet: &Sheet) -> Result<Cost> {
    let mut memo: HashMap<NodeId, Visit<Cost>> = HashMap::new();
    let mut stack = vec![(root, false)];
    while let Some((node, expanded)) = stack.pop() {
        let targets = if node.tag_name().name() == "use" { use_targets(node, ids) } else { Vec::new() };
        let children = || node.children().filter(Node::is_element).map(|n| (n, false)).chain(targets.iter().map(|n| (*n, true)));
        if !expanded {
            match memo.get(&node.id()) {
                Some(Visit::Done(_)) => continue,
                Some(Visit::Active) => return Err(recursive_use()),
                None => {}
            }
            memo.insert(node.id(), Visit::Active);
            stack.push((node, true));
            for (next, _) in children() {
                match memo.get(&next.id()) {
                    Some(Visit::Active) => return Err(recursive_use()),
                    Some(Visit::Done(_)) => {}
                    None => stack.push((next, false)),
                }
            }
            continue;
        }
        let own = own_cost(node, sheet)?;
        let mut cost = own.cost;
        for (next, via_use) in children() {
            let Some(Visit::Done(c)) = memo.get(&next.id()).copied() else { return Err(recursive_use()) };
            cost.elements = cost.elements.saturating_add(c.elements);
            cost.depth = cost.depth.max(c.depth.saturating_add(if via_use { 2 } else { 1 }));
            cost.vertices = cost.vertices.saturating_add(c.vertices);
            cost.bytes = cost.bytes.saturating_add(c.bytes);
            cost.nodes = cost.nodes.saturating_add(c.nodes);
            cost.style_scan = cost.style_scan.saturating_add(c.style_scan);
            cost.attribute_work = cost.attribute_work.saturating_add(c.attribute_work);
            cost.attribute_slots = cost.attribute_slots.saturating_add(c.attribute_slots);
            cost.gradient_painted = cost.gradient_painted.saturating_add(c.gradient_painted);
        }
        // Every inherited-property lookup from an element of this subtree scans this element's attributes.
        cost.attribute_work = cost.attribute_work.saturating_add(own.tree_attributes.saturating_mul(cost.elements));
        if own.paints || sheet.paints {
            cost.gradient_painted = cost.elements;
        }
        if cost.depth > MAX_SVG_DEPTH {
            return Err(too_deep(" once <use> references are expanded"));
        }
        if cost.elements > MAX_SVG_ELEMENTS {
            return Err(Error::TooLarge(format!("SVG expands to more than {MAX_SVG_ELEMENTS} elements once <use> references are expanded")));
        }
        if cost.vertices > MAX_IMPORT_POINTS as u64 {
            return Err(Error::TooLarge(format!("the SVG produces more than {MAX_IMPORT_POINTS} points once <use> references are expanded")));
        }
        if cost.bytes > MAX_SVG_EXPANDED_BYTES {
            return Err(Error::TooLarge(format!(
                "the SVG makes usvg read more than {MAX_SVG_EXPANDED_BYTES} bytes of attributes and text once <use> references are expanded"
            )));
        }
        if cost.nodes > MAX_SVG_EXPANDED_NODES {
            return Err(Error::TooLarge(format!(
                "the SVG makes usvg visit more than {MAX_SVG_EXPANDED_NODES} XML nodes (elements, text, comments) once <use> references are expanded"
            )));
        }
        if cost.attribute_work > MAX_SVG_ATTRIBUTE_WORK {
            return Err(Error::TooLarge(format!(
                "the SVG's elements carry too many attributes and style declarations for their nesting and <use> expansion (usvg would make more than {MAX_SVG_ATTRIBUTE_WORK} attribute comparisons)"
            )));
        }
        if cost.style_scan > MAX_SVG_STYLE_SCAN {
            return Err(Error::TooLarge(format!(
                "the SVG's style attributes are too long for their number of declarations once <use> references are expanded (parsing them would rescan more than {MAX_SVG_STYLE_SCAN} bytes); export with presentation attributes"
            )));
        }
        memo.insert(node.id(), Visit::Done(cost));
    }
    match memo.get(&root.id()) {
        Some(Visit::Done(cost)) => Ok(*cost),
        _ => Err(recursive_use()),
    }
}

/// One element on its own, and what its ancestors' subtree totals need from it.
struct Own {
    cost: Cost,
    /// An upper bound of its attributes in usvg's tree (its own plus those style declarations add).
    tree_attributes: u64,
    /// It paints with a gradient itself (`fill`, `stroke` or `style`).
    paints: bool,
}

/// One element on its own: one element, one level, its vertices, the bytes of its attributes and text, its child
/// nodes, and the attribute and style work usvg spends on it.
fn own_cost(node: Node<'_, '_>, sheet: &Sheet) -> Result<Own> {
    let (mut attributes, mut bytes, mut style_scan, mut style_declarations, mut paints) = (0u64, 0u64, 0u64, 0u64, false);
    for attribute in node.attributes() {
        let (name, value) = (attribute.name(), attribute.value());
        attributes += 1;
        bytes = bytes.saturating_add(ATTRIBUTE_BYTES + name.len() as u64 + value.len() as u64);
        if name == "style" {
            style_scan = style_scan.saturating_add(css_rescans(value));
            style_declarations = style_declarations.saturating_add(1 + value.bytes().filter(|&b| b == b';').count() as u64);
        }
        paints |= matches!(name, "fill" | "stroke" | "style") && read_by_usvg(&attribute) && url_count(value) > 0;
    }
    let mut nodes = 1u64;
    for child in node.children().filter(|c| !c.is_element()) {
        nodes += 1;
        if child.is_text() {
            bytes = bytes.saturating_add(child.text().map_or(0, str::len) as u64);
        }
    }
    let vertices = own_vertices(node)?;
    if vertices > MAX_ENTITY_POINTS as u64 {
        return Err(Error::TooLarge(format!("an SVG <{}> makes {vertices} points; the limit is {MAX_ENTITY_POINTS}", node.tag_name().name())));
    }
    // usvg applies each matching declaration by scanning the element's attributes for one of the same name.
    let declarations = sheet.declarations.saturating_add(style_declarations);
    let insertion = declarations.saturating_mul(attributes.saturating_sub(FREE_ATTRIBUTES));
    Ok(Own {
        cost: Cost {
            elements: 1,
            depth: 1,
            vertices,
            bytes,
            nodes,
            style_scan,
            attribute_work: insertion,
            attribute_slots: attributes + 1,
            gradient_painted: 0,
        },
        tree_attributes: attributes.saturating_add(declarations.min(CSS_ATTRIBUTES)),
        paints,
    })
}

/// An upper bound of the points usvg builds for one element (see [`path_points`] and [`shape_arc_curves`]).
fn own_vertices(node: Node<'_, '_>) -> Result<u64> {
    if !svg_element(node) {
        return Ok(0);
    }
    let radius = |names: &[&str]| -> Result<f64> {
        let mut largest = 0.0f64;
        for name in names {
            for value in attribute_values(node, name) {
                largest = largest.max(radius_px(value)?);
            }
        }
        Ok(largest)
    };
    let most = |name: &str, count: fn(&str) -> Result<u64>| -> Result<u64> {
        let mut most = 0u64;
        for value in attribute_values(node, name) {
            most = most.max(count(value)?);
        }
        Ok(most)
    };
    Ok(match node.tag_name().name() {
        "path" => most("d", path_points)?,
        "polyline" | "polygon" => most("points", |points| Ok(point_pairs(points)))?,
        "line" => 2,
        // usvg's rectangle is a move, four lines and a close; rounded corners add four quarter arcs.
        "rect" => {
            let r = radius(&["rx", "ry"])?;
            if r > 0.0 { 5u64.saturating_add(shape_arc_curves(r)?.saturating_mul(12)) } else { 5 }
        }
        "circle" => 1u64.saturating_add(shape_arc_curves(radius(&["r"])?)?.saturating_mul(12)),
        "ellipse" => 1u64.saturating_add(shape_arc_curves(radius(&["rx", "ry"])?)?.saturating_mul(12)),
        _ => 0,
    })
}

// ---- <use> expansion (usvg applies a <use> transform twice; refs are expanded here instead) -------------------------

/// Elements a `<use>` may clone (shapes, groups and the ignorable `<text>` / `<image>` / `<foreignObject>`). A nested
/// `<svg>`, a `<symbol>` or a paint server / style element cannot be cloned: `<svg>` and `<symbol>` get a viewport
/// clip a browser computes differently than the importer, and the others are not renderable.
const USE_TARGETS: [&str; 14] = ["g", "a", "defs", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "text", "image", "use", "foreignObject"];

fn is_use_element(node: Node<'_, '_>) -> bool {
    svg_element(node) && node.tag_name().name() == "use"
}

/// `<use>` elements inside `node`, excluding children of other `<use>` elements (a `<use>`'s own children are
/// fallback content: never rendered, so they vanish with their parent instead of being expanded).
fn top_uses<'a, 'input>(node: Node<'a, 'input>, out: &mut Vec<Node<'a, 'input>>) {
    for child in node.children().filter(|c| c.is_element()) {
        if is_use_element(child) {
            out.push(child);
        } else {
            top_uses(child, out);
        }
    }
}

/// Replaces every `<use>` element in the document with the `<g>` a browser instantiates (`transform` plus the `x`,`y`
/// translation, then the referenced subtree cloned verbatim) and returns the rewritten markup; `None` when there is
/// no `<use>`. usvg must never see a `<use>`: it composes the use element's `transform` into the absolute transform
/// of the cloned content a second time (a translate lands twice as far as a browser puts it). The reference is
/// resolved the way the browser does (first element with the id in document order), `x` and `y` become a `translate`
/// appended to the use transform, every other attribute moves onto the group so the checks of [`check`] apply to it
/// unchanged, and the clone keeps its markup byte for byte (with the namespace declarations of its original scope).
pub(crate) fn expand_uses(doc: &roxmltree::Document<'_>, source: &str, dpi: f64) -> Result<Option<String>> {
    let root = doc.root_element();
    let mut uses = Vec::new();
    top_uses(root, &mut uses);
    if uses.is_empty() {
        return Ok(None);
    }
    let mut first_plain: HashMap<&str, Node<'_, '_>> = HashMap::new();
    for node in doc.descendants().filter(Node::is_element) {
        if let Some(id) = node.attribute("id") {
            first_plain.entry(id).or_insert(node);
        }
    }
    let mut out = String::with_capacity(source.len().min(MAX_SVG_EXPANDED_SOURCE_BYTES).saturating_add(256));
    let mut active = std::collections::HashSet::new();
    emit_chunk(&mut out, source, 0, root.range().start)?;
    emit_subtree(root, 1, source, &mut out, &first_plain, &mut active, dpi)?;
    emit_chunk(&mut out, source, root.range().end, source.len())?;
    Ok(Some(out))
}

/// Appends `source[from..to]` to `out`, bounded by [`MAX_SVG_EXPANDED_SOURCE_BYTES`].
fn emit_chunk(out: &mut String, source: &str, from: usize, to: usize) -> Result<()> {
    if out.len().saturating_add(to.saturating_sub(from)) > MAX_SVG_EXPANDED_SOURCE_BYTES {
        return Err(Error::TooLarge(format!("the SVG's <use> elements expand to more than {} bytes of markup", MAX_SVG_EXPANDED_SOURCE_BYTES)));
    }
    out.push_str(source.get(from..to).unwrap_or_default());
    Ok(())
}

/// Emits `source[node.range()]` with every `<use>` inside it replaced; `out_depth` is the depth `node` sits at in the
/// emitted document (the root is 1, a `<use>`'s generated `<g>` puts the clone one level below it). `active` holds
/// the ids of every element whose markup is currently being emitted: a `<use>` targeting one of them can never
/// terminate (`<g id="a"><use href="#a"/></g>` or the mutual version).
fn emit_subtree<'a, 'input>(
    node: Node<'a, 'input>,
    out_depth: usize,
    source: &'a str,
    out: &mut String,
    first_plain: &HashMap<&str, Node<'a, 'input>>,
    active: &mut std::collections::HashSet<NodeId>,
    dpi: f64,
) -> Result<()> {
    if out_depth >= MAX_SVG_DEPTH {
        return Err(too_deep(" once <use> references are expanded"));
    }
    active.insert(node.id());
    let result = (|| {
        if is_use_element(node) {
            return emit_use(node, out_depth, source, out, first_plain, active, dpi);
        }
        let mut uses = Vec::new();
        top_uses(node, &mut uses);
        let mut pos = node.range().start;
        for u in uses {
            // `ancestors()` yields the node itself: u at out_depth + (elements between them, u counted).
            let below = u.ancestors().take_while(|a| a.id() != node.id()).count();
            emit_chunk(out, source, pos, u.range().start)?;
            emit_use(u, out_depth.saturating_add(below), source, out, first_plain, active, dpi)?;
            pos = u.range().end;
        }
        emit_chunk(out, source, pos, node.range().end)
    })();
    active.remove(&node.id());
    result
}

/// Emits `<g …>clone</g>` for one `<use>`, or nothing where the reference resolves to nothing (the browser renders
/// nothing there either).
fn emit_use<'a, 'input>(
    u: Node<'a, 'input>,
    out_depth: usize,
    source: &'a str,
    out: &mut String,
    first_plain: &HashMap<&str, Node<'a, 'input>>,
    active: &mut std::collections::HashSet<NodeId>,
    dpi: f64,
) -> Result<()> {
    // Collect the use's own values; everything else is carried onto the generated group verbatim so that the guard's
    // attribute checks (and style inheritance) see exactly the same document a browser renders.
    let mut ids: Vec<&str> = Vec::new();
    let mut transform: Option<&str> = None;
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for attribute in u.attributes() {
        let (name, value) = (attribute.name(), attribute.value());
        if !read_by_usvg(&attribute) {
            continue;
        }
        match name {
            "href" => {
                if !value.trim_start().starts_with('#') {
                    return Err(Error::NotImplemented(
                        "an SVG <use> with an external reference (an href that does not start with '#'); place the referenced element inside the file",
                    ));
                }
                for key in iri_keys(value).into_iter().flatten() {
                    if !ids.contains(&key) {
                        ids.push(key);
                    }
                }
            }
            "x" | "y" => {
                let user_units = absolute_length(value).ok_or(Error::NotImplemented("SVG <use> x or y in %, em or ex; use absolute units"))? * dpi / 96.0;
                if name == "x" {
                    x = user_units;
                } else {
                    y = user_units;
                }
            }
            "transform" => {
                if transform.is_none() {
                    transform = Some(value);
                }
            }
            // Only matter for <svg>/<symbol> targets, which are refused below: dropped like the browser drops them.
            "width" | "height" => {}
            _ => {}
        }
    }
    if ids.len() > 1 {
        return Err(Error::NotImplemented("an SVG <use> with conflicting href and xlink:href attributes"));
    }
    let Some(target) = ids.first().and_then(|id| first_plain.get(id)) else {
        return Ok(());
    };
    if !svg_element(*target) {
        return Err(Error::NotImplemented("an SVG <use> cloning an element that is not in the SVG namespace"));
    }
    let name = target.tag_name().name();
    if matches!(name, "svg" | "symbol") {
        return Err(Error::NotImplemented(
            "SVG <use> elements cloning <svg> or <symbol> (the clone gets a viewport clip computed differently); ungroup or link to plain shapes",
        ));
    }
    if !USE_TARGETS.contains(&name) {
        return Err(malformed(format!("an SVG <use> element cloning <{name}>; only shapes and groups can be cloned")));
    }
    if active.contains(&target.id()) {
        return Err(recursive_use());
    }
    // The clone's markup is copied byte for byte, so it keeps the prefixes its scope gives it: every namespace
    // declared on the target or an ancestor goes onto the generated group (innermost first, so shadowing wins).
    let mut decls: Vec<(Option<String>, String)> = Vec::new();
    for ancestor in target.ancestors() {
        for ns in ancestor.namespaces() {
            if !decls.iter().any(|(n, _)| n.as_deref() == ns.name()) {
                decls.push((ns.name().map(str::to_owned), ns.uri().to_owned()));
            }
        }
    }
    // Attributes the generated group carries (everything but the consumed ones), with qualified names: a namespaced
    // attribute needs a declared prefix — reuse the clone's scope prefix for the URI, else mint one.
    let mut attrs: Vec<(String, &str)> = Vec::new();
    for attribute in u.attributes().filter(|a| read_by_usvg(a)) {
        let name = attribute.name();
        if matches!(name, "href" | "x" | "y" | "width" | "height" | "transform") {
            continue;
        }
        let qualified = match attribute.namespace() {
            None => name.to_owned(),
            Some(XML_NS) => format!("xml:{name}"),
            Some(uri) => {
                let existing = decls.iter().find(|(n, u)| u == uri && n.is_some()).and_then(|(n, _)| n.clone());
                let prefix = existing.unwrap_or_else(|| {
                    let p = mint_prefix(&decls);
                    decls.push((Some(p.clone()), uri.to_owned()));
                    p
                });
                format!("{prefix}:{name}")
            }
        };
        attrs.push((qualified, attribute.value()));
    }
    // The generated group must land in the SVG namespace; when the clone's default namespace is another one the
    // clone keeps it and the group gets a generated prefix instead.
    let default_ns = decls.iter().find(|(n, _)| n.is_none()).map(|(_, u)| u.clone());
    let svg_prefix = match default_ns.as_deref() {
        Some(SVG_NS) => None,
        Some(_) => Some(mint_prefix(&decls)),
        // No default declaration in the clone's scope: declare the SVG namespace anyway — elements copied from a
        // no-namespace scope are treated like SVG elements by usvg either way.
        None => None,
    };
    let mut start = String::with_capacity(64);
    start.push_str(&format!(" xmlns=\"{}\"", escape(default_ns.as_deref().unwrap_or(SVG_NS))));
    if let Some(p) = &svg_prefix {
        start.push_str(&format!(" xmlns:{p}=\"{SVG_NS}\""));
    }
    for (n, uri) in decls.iter().filter(|(n, _)| n.is_some()) {
        start.push_str(&format!(" xmlns:{}=\"{}\"", n.as_deref().unwrap_or_default(), escape(uri)));
    }
    for (qualified, value) in &attrs {
        start.push_str(&format!(" {qualified}=\"{}\"", escape(value)));
    }
    // translate(x, y) is appended to the use transform list (applied first), as browsers and the SVG spec compose it.
    let mut merged = transform.map(String::from).unwrap_or_default();
    if x != 0.0 || y != 0.0 {
        if !merged.is_empty() {
            merged.push(' ');
        }
        merged.push_str(&format!("translate({x} {y})"));
    }
    if !merged.is_empty() {
        start.push_str(&format!(" transform=\"{}\"", escape(&merged)));
    }
    out.push('<');
    if let Some(p) = &svg_prefix {
        out.push_str(p);
        out.push(':');
    }
    out.push('g');
    out.push_str(&start);
    out.push('>');
    emit_subtree(*target, out_depth + 1, source, out, first_plain, active, dpi)?;
    out.push_str("</");
    if let Some(p) = &svg_prefix {
        out.push_str(p);
        out.push(':');
    }
    out.push_str("g>");
    Ok(())
}

/// A fresh prefix not colliding with the clone's scope declarations.
fn mint_prefix(decls: &[(Option<String>, String)]) -> String {
    for i in 0..1000usize {
        let candidate = format!("ons{i}");
        if !decls.iter().any(|(n, _)| n.as_deref() == Some(candidate.as_str())) {
            return candidate;
        }
    }
    "onsX".to_owned()
}

/// `&`, `<`, `>` and `"` for an emitted attribute value or namespace URI.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

// ---- path data (svgtypes 0.15 and kurbo 0.11 mirrors) -------------------------------------------------------------

/// svgtypes' byte stream, as far as path data, point lists and lengths need it.
struct Stream<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Stream<'a> {
    fn new(text: &'a str) -> Self {
        Stream { bytes: text.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.pos..).unwrap_or_default()
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn skip_digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    /// `Stream::parse_number`: sign, digits, fraction, exponent (not `em` / `ex`), then Rust's f64 parser; finite.
    fn number(&mut self) -> Option<f64> {
        self.skip_spaces();
        let start = self.pos;
        let mut c = self.peek()?;
        if c == b'+' || c == b'-' {
            self.pos += 1;
            c = self.peek()?;
        }
        match c {
            b'0'..=b'9' => self.skip_digits(),
            b'.' => {}
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.skip_digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let next = *self.bytes.get(self.pos + 1)?;
            if next != b'm' && next != b'x' {
                self.pos += 1;
                match self.peek()? {
                    b'+' | b'-' => {
                        self.pos += 1;
                        self.skip_digits();
                    }
                    b'0'..=b'9' => self.skip_digits(),
                    _ => return None,
                }
            }
        }
        let text = std::str::from_utf8(self.bytes.get(start..self.pos)?).ok()?;
        text.parse::<f64>().ok().filter(|n| n.is_finite())
    }

    /// `Stream::parse_list_number`: a number, spaces and one optional comma.
    fn list_number(&mut self) -> Option<f64> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let n = self.number()?;
        self.skip_spaces();
        if self.peek() == Some(b',') {
            self.pos += 1;
        }
        Some(n)
    }

    fn list<const N: usize>(&mut self) -> Option<[f64; N]> {
        let mut out = [0.0; N];
        for slot in &mut out {
            *slot = self.list_number()?;
        }
        Some(out)
    }

    /// svgtypes `parse_flag`: one `0` or `1`, an optional comma, spaces.
    fn flag(&mut self) -> Option<bool> {
        self.skip_spaces();
        let c = self.peek()?;
        if c != b'0' && c != b'1' {
            return None;
        }
        self.pos += 1;
        if self.peek() == Some(b',') {
            self.pos += 1;
        }
        self.skip_spaces();
        Some(c == b'1')
    }
}

/// `PointsParser`: the number of (x, y) pairs a `points` list yields.
fn point_pairs(points: &str) -> u64 {
    let mut s = Stream::new(points);
    let mut pairs = 0u64;
    while s.list::<2>().is_some() {
        pairs += 1;
    }
    pairs
}

/// A length as usvg resolves it, in user units at the larger of its two DPIs (96), or `None` for `%`, `em` and `ex`
/// (their size depends on the viewport or the font). A value that is not a number is 0 (usvg then uses its default).
fn absolute_length(value: &str) -> Option<f64> {
    let mut s = Stream::new(value);
    let Some(n) = s.number() else { return Some(0.0) };
    let unit = s.rest();
    let factor = if unit.starts_with(b"%") || unit.starts_with(b"em") || unit.starts_with(b"ex") {
        return None;
    } else if unit.starts_with(b"in") {
        96.0
    } else if unit.starts_with(b"cm") {
        96.0 / 2.54
    } else if unit.starts_with(b"mm") {
        96.0 / 25.4
    } else if unit.starts_with(b"pt") {
        96.0 / 72.0
    } else if unit.starts_with(b"pc") {
        16.0
    } else {
        1.0
    };
    Some(n.abs() * factor)
}

/// A radius in user units; `%`, `em` and `ex` are refused (the curve count depends on the radius).
fn radius_px(value: &str) -> Result<f64> {
    absolute_length(value).ok_or(Error::NotImplemented("SVG circle, ellipse and rounded-rectangle radii in %, em or ex; use absolute units"))
}

// ---- what usvg drops without a word -------------------------------------------------------------------------------

/// Attributes whose numbers become geometry (`transform` is checked as a transform instead, `viewBox` refused).
const GEOMETRY_ATTRIBUTES: [&str; 15] = ["d", "points", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height"];

/// The SVG2 geometry properties a `style` declaration could carry: browsers apply them, usvg reads only the
/// attributes, so `style="cx:50"` would draw a circle at its attribute position while a browser moves it.
const GEOMETRY_CSS_PROPERTIES: [&str; 14] = ["d", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height"];

/// Attribute values parsed as one length each by svgtypes (`!at_end()` after the unit fails the whole value):
/// a trailing whitespace survives there but a browser trims it.
const SINGLE_LENGTH_ATTRIBUTES: [&str; 28] = [
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "fr",
    "width",
    "height",
    "refX",
    "refY",
    "z",
    "radius",
    "azimuth",
    "elevation",
    "stroke-width",
    "stroke-dashoffset",
    "offset",
    "startOffset",
    "textLength",
    "pathLength",
];

/// Attributes parsed as lengths or length/number lists: an ASCII letter right after a number must be a unit
/// svgtypes reads (lower-case only), or the value is mis-scaled or dropped where a browser accepts it.
const LENGTH_ATTRIBUTES: [&str; 38] = [
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "fr",
    "width",
    "height",
    "refX",
    "refY",
    "dx",
    "dy",
    "rotate",
    "z",
    "radius",
    "azimuth",
    "elevation",
    "order",
    "scale",
    "kernelUnitLength",
    "points",
    "viewBox",
    "stroke-width",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-miterlimit",
    "offset",
    "startOffset",
    "textLength",
    "pathLength",
];

/// Units svgtypes parses after a number (the suffix must be exactly lower-case).
const SVG_UNITS: [&str; 9] = ["em", "ex", "px", "in", "cm", "mm", "pt", "pc", "%"];

/// Units a browser accepts but svgtypes does not (a value using one is mis-parsed, not just refused by both).
const BROWSER_UNITS: [&str; 35] = [
    "q", "rem", "ch", "cap", "ic", "lh", "rlh", "vw", "vh", "vi", "vb", "vmin", "vmax", "svw", "svh", "svi", "svb", "lvw", "lvh", "lvi", "lvb", "dvw", "dvh",
    "dvi", "dvb", "deg", "grad", "rad", "turn", "s", "ms", "fr", "dpi", "dpcm", "dppx",
];

/// Scans `value` for `number + ASCII letters` where the letters spell a unit browsers know but in a form svgtypes
/// does not accept (`100MM`, `1Q`, `2Deg`): usvg then drops or mis-scales what a browser honours.
fn check_unit_suffixes(name: &str, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii_digit() || b == b'.' {
            let mut s = Stream::new(value);
            s.pos = i;
            if s.number().is_some() {
                let run_start = s.pos;
                let mut j = run_start;
                while bytes.get(j).is_some_and(u8::is_ascii_alphabetic) {
                    j += 1;
                }
                if j > run_start {
                    let unit = &value[run_start..j];
                    if !SVG_UNITS.contains(&unit) && SVG_UNITS.iter().chain(BROWSER_UNITS.iter()).any(|u| unit.eq_ignore_ascii_case(u)) {
                        return Err(malformed(format!(
                            "an SVG {name} value with the unit \"{unit}\"; browsers accept unit spellings the parser does not — use lowercase px, mm, cm, in, pt or pc"
                        )));
                    }
                }
                i = s.pos.max(i + 1);
                continue;
            }
        }
        i += 1;
    }
    Ok(())
}

/// A `viewBox` must be exactly four numbers with a positive size: trailing junk is ignored by svgtypes but invalidates
/// the attribute for a browser, and a non-positive size draws nothing in a browser while usvg would draw it.
fn check_viewbox(value: &str) -> Result<()> {
    let mut s = Stream::new(value);
    let mut numbers = [0.0f64; 4];
    for slot in &mut numbers {
        let Some(n) = s.list_number() else { return Ok(()) };
        *slot = n;
    }
    s.skip_spaces();
    if s.pos < s.bytes.len() {
        return Err(Error::NotImplemented("an SVG viewBox with content after its four numbers; a browser ignores the whole attribute"));
    }
    if numbers[2] <= 0.0 || numbers[3] <= 0.0 {
        return Err(malformed("an SVG viewBox with a non-positive width or height (it disables rendering in browsers)"));
    }
    Ok(())
}

/// Whether a `points` list parses to its end: svgtypes stops at the first bad pair and usvg draws the prefix, while a
/// browser drops the whole element.
fn points_complete(points: &str) -> bool {
    let mut s = Stream::new(points);
    loop {
        s.skip_spaces();
        if s.pos >= s.bytes.len() {
            return true;
        }
        if s.list::<2>().is_none() {
            return false;
        }
    }
}

/// A declaration value with a trailing `!important` (any case) stripped.
fn strip_important(value: &str) -> &str {
    let value = value.trim_end();
    if value.len() >= 10 && value[value.len() - 10..].eq_ignore_ascii_case("!important") { value[..value.len() - 10].trim_end() } else { value }
}

/// Whether a value holds a number that is finite in f64 but infinite in f32 (usvg parses f64, then converts; a path
/// or shape with such a number is dropped).
fn overflows_f32(value: &str) -> bool {
    let bytes = value.as_bytes();
    // Without an exponent a number needs 39 integer digits to pass f32::MAX.
    let mut run = 0usize;
    let long_run = bytes.iter().any(|b| {
        run = if b.is_ascii_digit() { run + 1 } else { 0 };
        run >= 39
    });
    if !long_run && !bytes.iter().any(|b| matches!(b, b'e' | b'E')) {
        return false;
    }
    let mut s = Stream::new(value);
    while s.pos < bytes.len() {
        let start = s.pos;
        if matches!(s.peek(), Some(b'0'..=b'9' | b'.' | b'+' | b'-'))
            && let Some(n) = s.number()
            && (n as f32).is_infinite()
        {
            return true;
        }
        if s.pos == start {
            s.pos += 1;
        }
    }
    false
}

/// Whether usvg drops an element with this `transform`: svgtypes 0.15 `Transform::from_str` (an unparsable list is
/// the identity), converted to f32 as usvg does, must be finite with both scale factors above f32::EPSILON
/// (tiny-skia `Transform::is_valid`).
fn degenerate_transform(value: &str) -> bool {
    let Some([a, b, c, d, e, f]) = svg_transform(value) else { return false };
    let [a, b, c, d, e, f] = [a as f32, b as f32, c as f32, d as f32, e as f32, f as f32];
    if ![a, b, c, d, e, f].iter().all(|v| v.is_finite()) {
        return true;
    }
    (a * a + c * c).sqrt() <= f32::EPSILON || (b * b + d * d).sqrt() <= f32::EPSILON
}

/// A transform list as svgtypes 0.15 parses it, as the matrix `[a, b, c, d, e, f]`; `None` where it fails.
fn svg_transform(value: &str) -> Option<[f64; 6]> {
    let multiply = |t: [f64; 6], u: [f64; 6]| {
        [
            t[0] * u[0] + t[2] * u[1],
            t[1] * u[0] + t[3] * u[1],
            t[0] * u[2] + t[2] * u[3],
            t[1] * u[2] + t[3] * u[3],
            t[0] * u[4] + t[2] * u[5] + t[4],
            t[1] * u[4] + t[3] * u[5] + t[5],
        ]
    };
    let mut s = Stream::new(value);
    let mut ts = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    loop {
        s.skip_spaces();
        if s.peek().is_none() {
            return Some(ts);
        }
        let start = s.pos;
        while s.peek().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            s.pos += 1;
        }
        let name = s.bytes.get(start..s.pos)?;
        s.skip_spaces();
        if s.peek() != Some(b'(') {
            return None;
        }
        s.pos += 1;
        // The optional second number is read only when the list does not close first.
        let closes = |s: &mut Stream<'_>| {
            s.skip_spaces();
            s.peek() == Some(b')')
        };
        match name {
            b"matrix" => ts = multiply(ts, s.list::<6>()?),
            b"translate" => {
                let x = s.list_number()?;
                let y = if closes(&mut s) { 0.0 } else { s.list_number()? };
                ts = multiply(ts, [1.0, 0.0, 0.0, 1.0, x, y]);
            }
            b"scale" => {
                let x = s.list_number()?;
                let y = if closes(&mut s) { x } else { s.list_number()? };
                ts = multiply(ts, [x, 0.0, 0.0, y, 0.0, 0.0]);
            }
            b"rotate" => {
                let angle = s.list_number()?;
                let center = if closes(&mut s) { None } else { Some(s.list::<2>()?) };
                let (sin, cos) = angle.to_radians().sin_cos();
                let rotation = [cos, sin, -sin, cos, 0.0, 0.0];
                ts = match center {
                    None => multiply(ts, rotation),
                    Some([x, y]) => multiply(multiply(multiply(ts, [1.0, 0.0, 0.0, 1.0, x, y]), rotation), [1.0, 0.0, 0.0, 1.0, -x, -y]),
                };
            }
            b"skewX" => ts = multiply(ts, [1.0, 0.0, s.list_number()?.to_radians().tan(), 1.0, 0.0, 0.0]),
            b"skewY" => ts = multiply(ts, [1.0, s.list_number()?.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => return None,
        }
        if !closes(&mut s) {
            return None;
        }
        s.pos += 1;
        s.skip_spaces();
        if s.peek() == Some(b',') {
            s.pos += 1;
        }
    }
}

/// SVG 1.1 feature strings usvg 0.45 supports in `requiredFeatures` (`switch.rs`).
const FEATURES: [&str; 26] = [
    "http://www.w3.org/TR/SVG11/feature#SVGDOM-static",
    "http://www.w3.org/TR/SVG11/feature#SVG-static",
    "http://www.w3.org/TR/SVG11/feature#CoreAttribute",
    "http://www.w3.org/TR/SVG11/feature#Structure",
    "http://www.w3.org/TR/SVG11/feature#BasicStructure",
    "http://www.w3.org/TR/SVG11/feature#ContainerAttribute",
    "http://www.w3.org/TR/SVG11/feature#ConditionalProcessing",
    "http://www.w3.org/TR/SVG11/feature#Image",
    "http://www.w3.org/TR/SVG11/feature#Style",
    "http://www.w3.org/TR/SVG11/feature#Shape",
    "http://www.w3.org/TR/SVG11/feature#Text",
    "http://www.w3.org/TR/SVG11/feature#BasicText",
    "http://www.w3.org/TR/SVG11/feature#PaintAttribute",
    "http://www.w3.org/TR/SVG11/feature#BasicPaintAttribute",
    "http://www.w3.org/TR/SVG11/feature#OpacityAttribute",
    "http://www.w3.org/TR/SVG11/feature#GraphicsAttribute",
    "http://www.w3.org/TR/SVG11/feature#BasicGraphicsAttribute",
    "http://www.w3.org/TR/SVG11/feature#Marker",
    "http://www.w3.org/TR/SVG11/feature#Gradient",
    "http://www.w3.org/TR/SVG11/feature#Pattern",
    "http://www.w3.org/TR/SVG11/feature#Clip",
    "http://www.w3.org/TR/SVG11/feature#BasicClip",
    "http://www.w3.org/TR/SVG11/feature#Mask",
    "http://www.w3.org/TR/SVG11/feature#Filter",
    "http://www.w3.org/TR/SVG11/feature#BasicFilter",
    "http://www.w3.org/TR/SVG11/feature#XlinkAttribute",
];

/// Whether usvg's conditional processing drops this element (usvg 0.45 `switch::is_condition_passed` with its
/// default language list, `en`): any `requiredExtensions`, a `requiredFeatures` naming a feature it lacks, or a
/// `systemLanguage` without `en`.
fn condition_fails(node: Node<'_, '_>) -> bool {
    attribute_values(node, "requiredExtensions").next().is_some()
        || attribute_values(node, "requiredFeatures").any(|features| features.split(' ').any(|f| !FEATURES.contains(&f)))
        || attribute_values(node, "systemLanguage").any(|languages| {
            !languages.split(',').map(str::trim).any(|language| language == "en" || language.split_once('-').is_some_and(|(prefix, _)| prefix == "en"))
        })
}

fn arc_too_large(radius: f64) -> Error {
    Error::TooLarge(format!("an SVG arc of radius {radius} would become more than {MAX_SVG_ARC_CURVES} curves"))
}

/// Curves per quarter arc of a circle, ellipse or rounded rectangle of this radius (user units): kurbo's count for a
/// full turn (an upper bound for a quarter) at twice the radius (f32 rounding of the end points can at most double
/// the chord, which bounds kurbo's radius scaling), plus one.
fn shape_arc_curves(radius: f64) -> Result<u64> {
    let n = (1.1163 * (2.0 * radius) / 0.1).powf(1.0 / 6.0).max(3.999_999).ceil();
    // NaN or infinite (a radius beyond f64) is refused too.
    if n.is_nan() || n > f64::from(MAX_SVG_ARC_CURVES) {
        return Err(arc_too_large(radius));
    }
    Ok(n as u64 + 1)
}

/// The points usvg builds from path data: an exact mirror of svgtypes 0.15 `PathParser` and `SimplifyingPathParser`
/// (relative to absolute, implicit MoveTo after ClosePath, ClosePath runs collapsed, arcs to cubics through kurbo),
/// stopping where they stop (the first error). Refuses an arc that would become more than [`MAX_SVG_ARC_CURVES`]
/// curves (svgtypes buffers them and drains the buffer with `Vec::remove(0)`, quadratic in the count) and a run of
/// more than [`MAX_SVG_EMPTY_SEGMENTS`] segments that add nothing (svgtypes recurses once per such segment).
fn path_points(d: &str) -> Result<u64> {
    let mut s = Stream::new(d);
    let mut previous_command: Option<u8> = None;
    let mut current = (0.0f64, 0.0f64);
    let mut subpath_start = (0.0f64, 0.0f64);
    let mut after_close = false;
    let mut points = 0u64;
    let mut empty_run = 0usize;
    loop {
        s.skip_spaces();
        let Some(first) = s.peek() else { break };
        let (command, implicit_move) = if b"MmZzLlHhVvCcSsQqTtAa".contains(&first) {
            if previous_command.is_none() && first != b'M' && first != b'm' {
                break;
            }
            s.pos += 1;
            (first, false)
        } else if let Some(previous) = previous_command.filter(|_| matches!(first, b'0'..=b'9' | b'.' | b'-' | b'+')) {
            match previous {
                b'Z' | b'z' => break,
                b'M' => (b'L', true),
                b'm' => (b'l', true),
                other => (other, false),
            }
        } else {
            break;
        };
        let absolute = command.is_ascii_uppercase();
        let segment = match command.to_ascii_lowercase() {
            b'm' => s.list::<2>().map(|[x, y]| Segment::Move(x, y)),
            b'l' => s.list::<2>().map(|[x, y]| Segment::Line(x, y)),
            b'h' => s.list::<1>().map(|[x]| Segment::Horizontal(x)),
            b'v' => s.list::<1>().map(|[y]| Segment::Vertical(y)),
            b'c' => s.list::<6>().map(|[_, _, _, _, x, y]| Segment::Curve(3, x, y)),
            b's' | b'q' => s.list::<4>().map(|[_, _, x, y]| Segment::Curve(if command.eq_ignore_ascii_case(&b's') { 3 } else { 2 }, x, y)),
            b't' => s.list::<2>().map(|[x, y]| Segment::Curve(2, x, y)),
            b'a' => (|| {
                let [rx, ry, rotation] = s.list::<3>()?;
                let (large, sweep) = (s.flag()?, s.flag()?);
                let [x, y] = s.list::<2>()?;
                Some(Segment::Arc { rx, ry, rotation, large, sweep, x, y })
            })(),
            _ => Some(Segment::Close),
        };
        let Some(segment) = segment else { break };
        previous_command = Some(match (implicit_move, absolute) {
            (true, true) => b'M',
            (true, false) => b'm',
            (false, _) => command,
        });
        let to = |x: f64, y: f64, current: (f64, f64)| if absolute { (x, y) } else { (x + current.0, y + current.1) };
        // After a ClosePath, any segment but MoveTo / ClosePath starts with an implicit MoveTo to the subpath start
        // (where the ClosePath left the current point).
        let implicit = after_close && !matches!(segment, Segment::Move(..) | Segment::Close);
        if implicit {
            points += 1;
            after_close = false;
        }
        // Whether svgtypes emits anything for this segment; when it does not, it recurses for the next one.
        let mut emitted = true;
        match segment {
            Segment::Move(x, y) => {
                current = to(x, y, current);
                subpath_start = current;
                points += 1;
                after_close = false;
            }
            Segment::Line(x, y) => {
                current = to(x, y, current);
                points += 1;
            }
            Segment::Horizontal(x) => {
                current.0 = if absolute { x } else { x + current.0 };
                points += 1;
            }
            Segment::Vertical(y) => {
                current.1 = if absolute { y } else { y + current.1 };
                points += 1;
            }
            Segment::Curve(n, x, y) => {
                current = to(x, y, current);
                points += n;
            }
            Segment::Arc { rx, ry, rotation, large, sweep, x, y } => {
                let end = to(x, y, current);
                match kurbo_arc(current, end, rx, ry, rotation, large, sweep)? {
                    ArcCurves::Line => {
                        current = end;
                        points += 1;
                    }
                    // No curve: the current point stays (the implicit MoveTo's, if any, is the same point).
                    ArcCurves::Cubics(0, _) => emitted = implicit,
                    ArcCurves::Cubics(n, last) => {
                        current = last;
                        points = points.saturating_add(n.saturating_mul(3));
                    }
                }
            }
            // A ClosePath right after a ClosePath is dropped.
            Segment::Close if after_close => emitted = false,
            Segment::Close => {
                current = subpath_start;
                after_close = true;
            }
        }
        if points > MAX_ENTITY_POINTS as u64 {
            // Over the per-entity limit already: the caller refuses; parsing the rest would only cost time.
            return Ok(points);
        }
        if emitted {
            empty_run = 0;
        } else {
            empty_run += 1;
            if empty_run > MAX_SVG_EMPTY_SEGMENTS {
                return Err(Error::TooLarge(format!("SVG path data has more than {MAX_SVG_EMPTY_SEGMENTS} segments in a row that draw nothing")));
            }
        }
    }
    Ok(points)
}

#[derive(Clone, Copy)]
enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    Horizontal(f64),
    Vertical(f64),
    /// Points added (3 for cubic, 2 for quadratic) and the end point.
    Curve(u64, f64, f64),
    Arc {
        rx: f64,
        ry: f64,
        rotation: f64,
        large: bool,
        sweep: bool,
        x: f64,
        y: f64,
    },
    Close,
}

enum ArcCurves {
    /// kurbo treats it as a straight line (svgtypes emits a LineTo).
    Line,
    /// This many cubics, the last ending here (svgtypes' next current point).
    Cubics(u64, (f64, f64)),
}

/// kurbo 0.11 `Arc::from_svg_arc` then `Arc::append_iter(0.1)`, as svgtypes calls them, with the same arithmetic in
/// the same order (so the curve count and the end point are bit-identical): the count is
/// `ceil(max((1.1163 · r / 0.1)^(1/6), 3.999999) · |sweep| / 2π)` for the arc's largest (scaled) radius r.
fn kurbo_arc(from: (f64, f64), to: (f64, f64), rx: f64, ry: f64, rotation_deg: f64, large: bool, sweep: bool) -> Result<ArcCurves> {
    let x_rotation = rotation_deg.to_radians();
    if rx.abs() <= 1e-5 || ry.abs() <= 1e-5 || from == to {
        return Ok(ArcCurves::Line);
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    let xr = x_rotation % (2.0 * PI);
    let (sin_phi, cos_phi) = xr.sin_cos();
    let hd_x = (from.0 - to.0) * 0.5;
    let hd_y = (from.1 - to.1) * 0.5;
    let hs_x = (from.0 + to.0) * 0.5;
    let hs_y = (from.1 + to.1) * 0.5;
    let p = (cos_phi * hd_x + sin_phi * hd_y, -sin_phi * hd_x + cos_phi * hd_y);
    let rf = p.0 * p.0 / (rx * rx) + p.1 * p.1 / (ry * ry);
    if rf > 1.0 {
        let scale = rf.sqrt();
        rx *= scale;
        ry *= scale;
    }
    let rxry = rx * ry;
    let rxpy = rx * p.1;
    let rypx = ry * p.0;
    let sum_of_sq = rxpy * rxpy + rypx * rypx;
    let sign_coe = if large == sweep { -1.0 } else { 1.0 };
    let coe = sign_coe * ((rxry * rxry - sum_of_sq) / sum_of_sq).abs().sqrt();
    let transformed_cx = coe * rxpy / ry;
    let transformed_cy = -coe * rypx / rx;
    let center = (cos_phi * transformed_cx - sin_phi * transformed_cy + hs_x, sin_phi * transformed_cx + cos_phi * transformed_cy + hs_y);
    let start_v = ((p.0 - transformed_cx) / rx, (p.1 - transformed_cy) / ry);
    let end_v = ((-p.0 - transformed_cx) / rx, (-p.1 - transformed_cy) / ry);
    let start_angle = start_v.1.atan2(start_v.0);
    let mut sweep_angle = (end_v.1.atan2(end_v.0) - start_angle) % (2.0 * PI);
    if sweep && sweep_angle < 0.0 {
        sweep_angle += 2.0 * PI;
    } else if !sweep && sweep_angle > 0.0 {
        sweep_angle -= 2.0 * PI;
    }
    let scaled_err = rx.max(ry) / 0.1;
    let n_err = (1.1163 * scaled_err).powf(1.0 / 6.0).max(3.999_999);
    let n = (n_err * sweep_angle.abs() * (1.0 / (2.0 * PI))).ceil();
    if n.is_nan() || n <= 0.0 {
        // `n as usize` is 0: kurbo emits nothing and svgtypes keeps its current point.
        return Ok(ArcCurves::Cubics(0, from));
    }
    if n > f64::from(MAX_SVG_ARC_CURVES) {
        return Err(arc_too_large(rx.max(ry)));
    }
    let angle_step = sweep_angle / n;
    let count = n as u64;
    let mut angle0 = start_angle;
    let mut last = from;
    for _ in 0..count {
        let angle1 = angle0 + angle_step;
        let (sin, cos) = angle1.sin_cos();
        let (u, v) = (rx * cos, ry * sin);
        let (rot_sin, rot_cos) = x_rotation.sin_cos();
        last = (center.0 + (u * rot_cos - v * rot_sin), center.1 + (u * rot_sin + v * rot_cos));
        angle0 = angle1;
    }
    Ok(ArcCurves::Cubics(count, last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_scan_skips_comments_quotes_and_processing_instructions() {
        let ok = r#"<?xml version="1.0"?><!-- it's <g> --><svg a="<g>" b='>'><!-- <g><g> --><![CDATA[<g>]]><?pi "<g>?><g/><g></g></svg>"#;
        assert_eq!(scan_markup(ok.as_bytes()).ok(), Some(None));
        let deep = format!("<svg>{}{}</svg>", "<g>".repeat(MAX_SVG_DEPTH), "</g>".repeat(MAX_SVG_DEPTH));
        assert!(matches!(scan_markup(deep.as_bytes()), Err(Error::TooLarge(_))));
        let at_limit = format!("<svg>{}<path/>{}</svg>", "<g>".repeat(MAX_SVG_DEPTH - 2), "</g>".repeat(MAX_SVG_DEPTH - 2));
        assert!(scan_markup(at_limit.as_bytes()).is_ok());
        let leaf_over = format!("<svg>{}<path/>{}</svg>", "<g>".repeat(MAX_SVG_DEPTH - 1), "</g>".repeat(MAX_SVG_DEPTH - 1));
        assert!(scan_markup(leaf_over.as_bytes()).is_err());
        // A quote inside a processing instruction does not open a string (the review's desync).
        let pi_quote = format!(r#"<?pi "?><svg>{}{}</svg>"#, "<g>".repeat(MAX_SVG_DEPTH), "</g>".repeat(MAX_SVG_DEPTH));
        assert!(matches!(scan_markup(pi_quote.as_bytes()), Err(Error::TooLarge(_))));
    }

    #[test]
    fn doctype_is_blanked_or_refused() {
        let plain = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg/>"#;
        let prepared = prepare(plain).expect("plain DOCTYPE");
        assert!(prepared.starts_with(' ') && prepared.ends_with("<svg/>") && prepared.len() == plain.len());
        for subset in [r#"<!DOCTYPE svg [<!ENTITY a "x">]><svg/>"#, r#"<!DOCTYPE svg [<?x [?>]><svg/>"#, r#"<!DOCTYPE svg "a[" [ ]><svg/>"#] {
            assert!(matches!(prepare(subset), Err(Error::NotImplemented(_))), "{subset}");
        }
        assert!(prepare("<svg/><!DOCTYPE svg>").is_err());
        assert!(prepare(r#"<!ENTITY a "x"><svg/>"#).is_err());
    }

    #[test]
    fn path_points_mirror_svgtypes() {
        assert_eq!(path_points("M10 10 L20 10 l0 10 H5 v-5 Z m30 0 l5 5 h3 V30 z").ok(), Some(9));
        assert_eq!(path_points("M0 0 1 1 2 2").ok(), Some(3), "implicit LineTo after MoveTo");
        assert_eq!(path_points("M0 0L1 1ZL2 2").ok(), Some(4), "implicit MoveTo after ClosePath");
        assert_eq!(path_points("M0 0C1 1 2 2 3 3S4 4 5 5Q6 6 7 7T8 8").ok(), Some(11));
        assert_eq!(path_points("M0 0L1").ok(), Some(1), "stops at the first error");
        assert_eq!(path_points("L0 0").ok(), Some(0), "must start with MoveTo");
        assert_eq!(path_points("M0 0A5 5 0 0 1 10 0").ok(), Some(1 + 3 * 2), "a half circle of radius 5 is two cubics");
        assert_eq!(path_points("M0 0A0 5 0 0 1 10 0").ok(), Some(2), "a zero radius is a line");
        assert_eq!(path_points("M0 0A5 5 0 1110 0").ok(), Some(7), "flags without separators");
        assert!(matches!(path_points("M0 0A1e60 1e60 0 1 1 1e60 0"), Err(Error::TooLarge(_))));
        // The first ClosePath is drawn; each one after it is dropped (and svgtypes recurses for it).
        let closes = format!("M0 0L1 1{}", "Z".repeat(MAX_SVG_EMPTY_SEGMENTS + 2));
        assert!(matches!(path_points(&closes), Err(Error::TooLarge(_))));
        assert!(path_points(&format!("M0 0L1 1{}", "Z".repeat(MAX_SVG_EMPTY_SEGMENTS + 1))).is_ok());
        let flat_arcs = format!("M0 0L1 1Z{}", "A1 1 0 0 0 0 0".repeat(MAX_SVG_EMPTY_SEGMENTS + 1));
        assert_eq!(path_points(&flat_arcs).ok(), Some(usvg_points(&flat_arcs) as u64), "an arc back to its own start is a line: never empty");
    }

    /// The points usvg itself builds for path data (tiny-skia may merge a MoveTo run, so it can be fewer).
    fn usvg_points(d: &str) -> usize {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="{d}" stroke="black"/></svg>"#);
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("parses");
        let mut count = 0;
        let mut stack = vec![tree.root()];
        while let Some(group) = stack.pop() {
            for node in group.children() {
                match node {
                    usvg::Node::Group(g) => stack.push(g),
                    usvg::Node::Path(p) => count += p.data().points().len(),
                    _ => {}
                }
            }
        }
        count
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { cases: 512, ..proptest::prelude::ProptestConfig::default() })]

        /// The mirror never under-counts what usvg builds, and matches it exactly when no MoveTo run is merged.
        #[test]
        fn path_points_bound_usvg(parts in proptest::collection::vec(proptest::sample::select(vec![
            "M", "m", "L", "l", "H", "h", "V", "v", "C", "c", "S", "s", "Q", "q", "T", "t", "A", "a", "Z", "z",
            " ", ",", "0", "1", "-2.5", "3e1", ".5", "1e", "7", "100", "-0.25", "5 5 0 1 1", "1e30", "0 0 0 0 1", "x",
        ]), 0..40)) {
            let d = format!("M1 2{}", parts.concat());
            let Ok(estimate) = path_points(&d) else { return Ok(()) };
            let actual = usvg_points(&d) as u64;
            proptest::prop_assert!(actual <= estimate, "{d}: usvg {actual} > estimate {estimate}");
        }
    }

    #[test]
    fn path_points_match_usvg() {
        for d in [
            "M10 10 L20 10 l0 10 H5 v-5 Z m30 0 l5 5 h3 V30 z",
            "M0 0L1 1ZL2 2",
            "M0 0C1 1 2 2 3 3S4 4 5 5Q6 6 7 7T8 8",
            "M0 0A5 5 0 0 1 10 0a20 30 45 1 0 7 -3A1000 1000 0 1 1 3000 0",
            "M3.5 8a.5.5 0 01.5-.5h8a.5.5 0 010 1H4a.5.5 0 01-.5-.5z",
            "M0 0A1e9 1e9 0 1 1 1e9 0",
            "M0 0L1 1ZZZZ l1 1zm2 2a3 3 0 1 1 1 1",
        ] {
            assert_eq!(path_points(d).ok(), Some(usvg_points(d) as u64), "{d}");
        }
    }

    #[test]
    fn start_tags_count_attributes_and_namespace_declarations() {
        let tag = tag_end(br#"svg xmlns="a" xmlns:x = 'b' a="xmlns:y=" b='>'/>"#, 0).expect("closed");
        assert_eq!((tag.attributes, tag.namespaces, tag.self_closing), (4, 2, true));
        let tag = tag_end(b"g a=b=c xmlnsx=1>", 0).expect("closed");
        assert_eq!((tag.attributes, tag.namespaces), (3, 0));
    }

    #[test]
    fn transforms_mirror_svgtypes_and_tiny_skia() {
        for degenerate in ["scale(0)", "scale(0, 1)", " scale( 1e-30 ) ", "translate(1e39)", "matrix(0 0 0 0 1 1)", "rotate(30) scale(0 1)x"] {
            // The last one is unparsable: svgtypes fails, usvg keeps the element (identity).
            assert_eq!(degenerate_transform(degenerate), !degenerate.ends_with('x'), "{degenerate}");
        }
        for kept in ["", "scale(1)", "rotate(45 10 10) scale(2)", "skewX(89)", "rotate(30) scale(0, 1)", "translate(1)scale(2),rotate(3)", "scale(1) junk"] {
            assert!(!degenerate_transform(kept), "{kept}");
        }
        // usvg itself keeps exactly the elements the mirror calls valid.
        for transform in [
            "scale(0)",
            "scale(0, 1)",
            "scale(1e-30)",
            "scale(1e-19)",
            "scale(1e-18)",
            "translate(1e39)",
            "skewX(90)",
            "rotate(30) scale(0, 1)",
            "scale(1) junk",
            "matrix(1e20 0 0 1e20 0 0)",
        ] {
            let svg =
                format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path transform="{transform}" d="M0 0L1 1" stroke="black"/></svg>"#);
            let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("parses");
            assert_eq!(tree.root().has_children(), !degenerate_transform(transform), "{transform}");
        }
        assert_eq!(svg_transform("translate(1 2) scale(3)"), Some([3.0, 0.0, 0.0, 3.0, 1.0, 2.0]));
        assert_eq!(svg_transform("rotate(90 1 1)").map(|m| m.map(|v| v.round())), Some([0.0, 1.0, -1.0, 0.0, 2.0, 0.0]));
    }

    #[test]
    fn f32_overflow_is_found_in_any_number() {
        assert!(overflows_f32("M0 0L1e39 0") && overflows_f32("-4e38") && overflows_f32(&"1".repeat(40)));
        assert!(!overflows_f32("3.4e38") && !overflows_f32("1e400") && !overflows_f32("M0 0 L1e-39 5e+3") && !overflows_f32(&"1".repeat(38)));
    }

    #[test]
    fn func_iri_mirrors_svgtypes() {
        assert_eq!(func_iri("url(#a)"), Some("a"));
        assert_eq!(func_iri(" url( '#a b ' ) red"), Some("a b"));
        assert_eq!(func_iri("url(\"#a\")"), Some("a"));
        assert_eq!(func_iri("url(#a b)"), None);
        assert_eq!(func_iri("url(a)"), None);
        assert_eq!(func_iri("red url(#a)"), None);
        assert_eq!(func_iri("url(#a"), None);
    }
}

//! The SVG guard (SPEC §1.3 rule 6, never crash). roxmltree, usvg, svgtypes and simplecss recurse per nesting level,
//! expand `<use>`, copy paint servers and styles per element, split arcs into curves and match CSS selectors, none
//! of it with a bound of its own. Everything they would do is measured here, before usvg builds anything, and a file
//! over a bound is refused:
//!
//! - [`prepare`] tokenizes the raw text once (comments, processing instructions, CDATA and quoted attribute values
//!   are skipped exactly as an XML parser skips them) and refuses element nesting deeper than [`MAX_SVG_DEPTH`]
//!   before roxmltree runs. A DOCTYPE internal subset is refused outright, so no entity is ever defined or expanded;
//!   a plain DOCTYPE is blanked out so that roxmltree parses with `allow_dtd: false`.
//! - [`check`] refuses, on the parsed tree, every reference usvg would follow without a bound or that hides
//!   geometry: `clip-path`, `mask`, `filter` and markers given as `url(…)`, paint servers other than gradients
//!   (patterns), `<tref>`, element `<feImage>`s, gradient `href` chains that loop or run longer than
//!   [`MAX_SVG_HREF_CHAIN`], CSS beyond element / class / id selectors. It then computes, with every `<use>`
//!   expanded, the elements, the nesting depth, the vertices (path commands, an arc counted as the curves it becomes)
//!   and the bytes usvg would read or copy, and refuses a file over [`MAX_SVG_ELEMENTS`], [`MAX_SVG_DEPTH`],
//!   [`MAX_IMPORT_POINTS`] / [`MAX_ENTITY_POINTS`] or [`MAX_SVG_EXPANDED_BYTES`].
//!
//! The path-data and arc arithmetic mirrors svgtypes 0.15 and kurbo 0.11 (the versions usvg 0.45.1 uses) so that the
//! vertex count is exact; where mirroring is not needed for safety the guard over-counts instead.

use std::borrow::Cow;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::ops::Range;

use usvg::roxmltree::{self, Node, NodeId};

use crate::{
    Error, MAX_ENTITY_POINTS, MAX_IMPORT_POINTS, MAX_SVG_ARC_CURVES, MAX_SVG_CSS_MATCHES, MAX_SVG_DEPTH, MAX_SVG_ELEMENTS, MAX_SVG_EMPTY_SEGMENTS,
    MAX_SVG_EXPANDED_BYTES, MAX_SVG_HREF_CHAIN, MAX_SVG_NAME_BYTES, MAX_SVG_PATH_DATA_BYTES, MAX_SVG_STYLE_BYTES, Result,
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
/// leaf included. Returns the DOCTYPE's byte range, if there is one.
fn scan_markup(bytes: &[u8]) -> Result<Option<Range<usize>>> {
    let mut depth = 0usize;
    let mut seen_element = false;
    let mut doctype = None;
    let mut i = 0usize;
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
            let (end, self_closing) = tag_end(bytes, start + 1).ok_or_else(|| malformed("a tag is not closed"))?;
            if depth >= MAX_SVG_DEPTH {
                return Err(too_deep(""));
            }
            seen_element = true;
            if !self_closing {
                depth += 1;
            }
            end
        };
    }
    Ok(doctype)
}

/// Index just after the first `needle` at or after `from`.
fn after(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let rest = bytes.get(from..)?;
    rest.windows(needle.len()).position(|window| window == needle).map(|p| from + p + needle.len())
}

/// Index after the `>` closing a start tag whose name begins at `from` (a quoted attribute value may hold `>`), and
/// whether the tag ended in `/>`.
fn tag_end(bytes: &[u8], from: usize) -> Option<(usize, bool)> {
    let mut quote: Option<u8> = None;
    let mut previous = 0u8;
    for (offset, &b) in bytes.get(from..)?.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some((from + offset + 1, previous == b'/')),
            None => {}
        }
        previous = b;
    }
    None
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
    Error::NotImplemented("SVG files whose DOCTYPE has an internal subset (entity declarations in [ … ]); delete the bracketed part and import again")
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
            tag_end(fragment, start + 1).map(|(end, self_closing)| {
                deepest = deepest.max(depth + 1);
                if !self_closing {
                    depth += 1;
                }
                end
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

/// Checks the parsed tree before usvg converts it (see the module docs).
pub(crate) fn check(doc: &roxmltree::Document<'_>) -> Result<SvgFacts> {
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(malformed(format!("the root element is <{}>, not <svg>", root.tag_name().name())));
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
        match node.tag_name().name() {
            "image" => facts.images += 1,
            "text" => facts.texts += 1,
            "tref" => return Err(Error::NotImplemented("SVG <tref> elements (they copy text from elsewhere in the file); delete them")),
            "feImage" if attribute_values(node, "href").any(|href| href.trim_start().starts_with('#')) => {
                return Err(Error::NotImplemented("SVG <feImage> elements that render another element; delete the filter"));
            }
            "style" => {
                // usvg reads any element named `style` (whatever its namespace or `type`); each text is checked.
                for text in node.descendants().filter(|t| t.is_text()).filter_map(|t| t.text()) {
                    styles.sheet_bytes = styles.sheet_bytes.saturating_add(text.len());
                    if styles.sheet_bytes > MAX_SVG_STYLE_BYTES {
                        return Err(Error::TooLarge(format!("SVG style sheets hold more than {MAX_SVG_STYLE_BYTES} bytes")));
                    }
                    styles.check_sheet(text)?;
                }
            }
            _ => {}
        }
        let mut hidden = false;
        for attribute in node.attributes().filter(read_by_usvg) {
            let (name, value) = (attribute.name(), attribute.value());
            match name {
                "style" => {
                    if value.len() > MAX_SVG_STYLE_BYTES {
                        return Err(Error::TooLarge(format!("an SVG style attribute holds {} bytes; the limit is {MAX_SVG_STYLE_BYTES}", value.len())));
                    }
                    hidden |= styles.check_declarations(value)?;
                }
                "display" => hidden |= value.trim() == "none",
                "d" | "points" if value.len() > MAX_SVG_PATH_DATA_BYTES => {
                    return Err(Error::TooLarge(format!("an SVG {name} attribute holds {} bytes; the limit is {MAX_SVG_PATH_DATA_BYTES}", value.len())));
                }
                _ => styles.check_property(name, value)?,
            }
        }
        facts.display_none += usize::from(hidden);
    }
    facts.sheet_hides = styles.sheet_hides;
    if styles.selectors > 0 {
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
    let stops = paint_stops(&styles.paints, &ids)?;
    let expanded = expand(root, &ids)?;
    let matches = expanded.elements.saturating_mul(styles.selectors as u64);
    if matches > MAX_SVG_CSS_MATCHES {
        return Err(Error::TooLarge(format!(
            "matching the SVG style sheet's {} selectors against {} elements takes more than {MAX_SVG_CSS_MATCHES} comparisons; export with presentation attributes",
            styles.selectors, expanded.elements
        )));
    }
    // What usvg copies into every element: the style sheet's declarations (the bound assumes every rule matches), a
    // dash array (`stroke-dasharray` is inherited and re-parsed per path, f32 values, odd lists doubled) and, for each
    // of fill and stroke, a gradient with its stops (copied per path in object-bounding-box units).
    let per_element = (styles.declaration_bytes as u64)
        .saturating_add(styles.dash_values.saturating_mul(8).saturating_add(32))
        .saturating_add(if styles.paints.is_empty() { 0 } else { stops.saturating_mul(16).saturating_add(256).saturating_mul(2) });
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
    sheet_bytes: usize,
    selectors: usize,
    declaration_bytes: usize,
    /// The most numbers any `stroke-dasharray` holds.
    dash_values: u64,
    sheet_hides: bool,
}

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
            }
            "stroke-dasharray" => {
                // Every digit counted as a value: never fewer than svgtypes parses.
                let numbers = value.bytes().filter(u8::is_ascii_digit).count() as u64;
                self.dash_values = self.dash_values.max(numbers);
            }
            "clip-path" | "mask" | "filter" | "marker" | "marker-start" | "marker-mid" | "marker-end" if url_count(value) > 0 => {
                return Err(reference_refusal(property));
            }
            _ => {}
        }
        Ok(())
    }

    /// Declarations of a `style` attribute or a sheet block, split at every `;` and the first `:` (simplecss splits
    /// at the same `;` unless one sits in a string or a function, and then this split sees a declaration that a
    /// `url(` cannot hide in: every `url(` must be the whole start of a `fill` / `stroke` value, or the import is
    /// refused). Returns whether one sets `display: none`.
    fn check_declarations(&mut self, text: &'a str) -> Result<bool> {
        if text.contains('\\') {
            return Err(Error::NotImplemented("backslash escapes in SVG style attributes and style sheets"));
        }
        if url_count(text) > 0 && text.contains("/*") {
            return Err(Error::NotImplemented("comments in SVG styles that hold url(…) references"));
        }
        let mut hides = false;
        for declaration in text.split(';') {
            let (name, value) = declaration.split_once(':').unwrap_or((declaration, ""));
            if url_count(name) > 0 {
                return Err(reference_refusal(""));
            }
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            let value = value.strip_suffix("!important").map_or(value, str::trim_end);
            match name.as_str() {
                "display" => hides |= value == "none",
                _ if url_count(value) > 0 && !matches!(name.as_str(), "fill" | "stroke") => return Err(reference_refusal(&name)),
                _ => self.check_property(&name, value)?,
            }
        }
        Ok(hides)
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
                self.selectors += 1;
            }
            let close = css_block_end(bytes, open + 1)?;
            let block = css.get(open + 1..close).unwrap_or_default();
            self.declaration_bytes = self.declaration_bytes.saturating_add(block.len());
            self.sheet_hides |= self.check_declarations(block)?;
            i = close + 1;
        }
    }
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
/// [`MAX_SVG_HREF_CHAIN`]. Returns the most `<stop>`s a referenced chain holds (usvg copies them per painted path).
fn paint_stops<'a, 'input>(paints: &[&str], ids: &Ids<'a, 'input>) -> Result<u64> {
    let mut memo: HashMap<NodeId, Visit<(usize, u64)>> = HashMap::new();
    let mut most = 0u64;
    // Each id once: a file may name the same gradient from every element.
    let keys: std::collections::BTreeSet<&str> = paints.iter().flat_map(|id| [*id, id.trim()]).collect();
    for key in keys {
        for &target in ids.all.get(key).map(Vec::as_slice).unwrap_or_default() {
            if !is_gradient(target) {
                return Err(Error::NotImplemented("SVG pattern fills and strokes (paint servers other than gradients); convert them to plain colours"));
            }
            most = most.max(gradient_chain(target, ids, &mut memo)?);
        }
    }
    Ok(most)
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
    /// Attribute and text bytes usvg reads (and partly copies) per expanded element.
    bytes: u64,
}

fn recursive_use() -> Error {
    malformed("a <use> element references itself or one of its ancestors")
}

/// The cost of `root` with every `<use>` expanded (iterative and memoized: a hostile file cannot exhaust the stack or
/// make this pass itself expand anything). A `<use>` costs two levels (usvg adds a group for it).
fn expand<'a, 'input>(root: Node<'a, 'input>, ids: &Ids<'a, 'input>) -> Result<Cost> {
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
        let mut cost = own_cost(node)?;
        for (next, via_use) in children() {
            let Some(Visit::Done(c)) = memo.get(&next.id()).copied() else { return Err(recursive_use()) };
            cost.elements = cost.elements.saturating_add(c.elements);
            cost.depth = cost.depth.max(c.depth.saturating_add(if via_use { 2 } else { 1 }));
            cost.vertices = cost.vertices.saturating_add(c.vertices);
            cost.bytes = cost.bytes.saturating_add(c.bytes);
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
        memo.insert(node.id(), Visit::Done(cost));
    }
    match memo.get(&root.id()) {
        Some(Visit::Done(cost)) => Ok(*cost),
        _ => Err(recursive_use()),
    }
}

/// One element on its own: one element, one level, its vertices and the bytes of its attributes and text.
fn own_cost(node: Node<'_, '_>) -> Result<Cost> {
    let attribute_bytes: usize = node.attributes().map(|a| a.value().len()).sum();
    let text_bytes: usize = node.children().filter(|c| c.is_text()).filter_map(|c| c.text()).map(str::len).sum();
    let vertices = own_vertices(node)?;
    if vertices > MAX_ENTITY_POINTS as u64 {
        return Err(Error::TooLarge(format!("an SVG <{}> makes {vertices} points; the limit is {MAX_ENTITY_POINTS}", node.tag_name().name())));
    }
    Ok(Cost { elements: 1, depth: 1, vertices, bytes: attribute_bytes.saturating_add(text_bytes) as u64 })
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

/// A length as usvg resolves it, in user units at the larger of its two DPIs (96); `%`, `em` and `ex` are refused
/// where an arc's radius is read (their size depends on the viewport or the font, and the curve count on the radius).
fn radius_px(value: &str) -> Result<f64> {
    let mut s = Stream::new(value);
    let Some(n) = s.number() else { return Ok(0.0) };
    let unit = s.rest();
    let factor = if unit.starts_with(b"%") || unit.starts_with(b"em") || unit.starts_with(b"ex") {
        return Err(Error::NotImplemented("SVG circle, ellipse and rounded-rectangle radii in %, em or ex; use absolute units"));
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
    Ok(n.abs() * factor)
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

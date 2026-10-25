//! System-font discovery and LightBurn-style text layout into kurbo paths.
//!
//! [`FontLibrary`] scans the platform font directories (plus `OPENLASER_FONT_DIRS`) once, lazily, and caches an
//! index of (family, style, weight, italic, path, collection index) read from each face's name table via skrifa.
//! [`layout`] shapes each line with harfrust (kerning, ligatures) and draws glyph outlines into one Y-up
//! [`BezPath`]: `height_mm` is the cap height, the origin is the first line's baseline anchor, and a character
//! with no glyph is a [`TextError::MissingGlyph`], never a silent drop or tofu.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use kurbo::BezPath;
use openlaser_types::TextAlign;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::string::StringId;

/// The font handle `FontLibrary::face` hands out (skrifa FontRef, borrowed from the cached bytes).
pub use skrifa::FontRef;

/// A font file can hold at most this many faces (a sane cap for hostile `.ttc`s).
const MAX_FACE_INDEX: u32 = 128;
/// Scanning stops at this many candidates (bounded work on huge trees).
const MAX_FILES: usize = 20_000;
/// A font file over this size is skipped rather than loaded.
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
/// `layout` rejects text longer than this and heights outside 0.1..=1000 mm.
pub const MAX_TEXT_CHARS: usize = 10_000;
pub const MIN_HEIGHT_MM: f64 = 0.1;
pub const MAX_HEIGHT_MM: f64 = 1000.0;
/// The outline is capped at the shared geom point budget.
const MAX_PATH_POINTS: usize = openlaser_geom::MAX_FLATTEN_POINTS;

#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("the font '{family}' has no glyph for '{chars}'")]
    MissingGlyph { family: String, chars: String },
    #[error("no fonts were found on this system")]
    NoFonts,
    #[error("the font '{0}' could not be read")]
    Unreadable(String),
    #[error("the text is longer than {MAX_TEXT_CHARS} characters")]
    TooLong,
    #[error("height must be within {MIN_HEIGHT_MM}..={MAX_HEIGHT_MM} mm (got {0})")]
    BadHeight(f64),
    #[error("the laid-out outline exceeds the {0} point limit")]
    TooManyPoints(usize),
    #[error("a layout parameter was not finite")]
    NonFinite,
}

/// One discovered face.
#[derive(Debug)]
pub struct FontEntry {
    pub family: String,
    pub style: String,
    /// OS/2 usWeightClass (400 = Regular).
    pub weight: u16,
    pub italic: bool,
    pub path: PathBuf,
    /// The face index inside a `.ttc`/`.otc` (0 otherwise).
    pub index: u32,
    bytes: std::sync::OnceLock<Vec<u8>>,
}

/// The cached font index. `new` does no work; the first query scans, `rescan` rebuilds.
#[derive(Debug)]
pub struct FontLibrary {
    dirs: Vec<PathBuf>,
    index: std::sync::OnceLock<Vec<FontEntry>>,
}

impl FontLibrary {
    /// The platform directories plus `OPENLASER_FONT_DIRS`.
    pub fn new() -> Self {
        FontLibrary { dirs: default_dirs(), index: std::sync::OnceLock::new() }
    }

    /// An index over exactly these directories (tests point it at the fixture).
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        FontLibrary { dirs, index: std::sync::OnceLock::new() }
    }

    /// Discard the index and face caches; the next query rescans.
    pub fn rescan(&mut self) {
        self.index = std::sync::OnceLock::new();
    }

    /// The discovered faces, sorted by family, weight, italic.
    pub fn entries(&self) -> &[FontEntry] {
        self.index.get_or_init(|| scan(&self.dirs))
    }

    /// Families with their styles, sorted by family name.
    pub fn families(&self) -> Vec<(String, Vec<String>)> {
        let mut map: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for e in self.entries() {
            let styles = map.entry(e.family.clone()).or_default();
            if !styles.contains(&e.style) {
                styles.push(e.style.clone());
            }
        }
        map.into_iter().collect()
    }

    /// A face for (family, style), or the family's closest face when `style` is absent. `None` when the family is
    /// unknown or the file can no longer be read.
    pub fn face(&self, family: &str, style: Option<&str>) -> Option<FontRef<'_>> {
        let entry = self
            .entries()
            .iter()
            .find(|e| e.family.eq_ignore_ascii_case(family) && style.is_none_or(|s| e.style.eq_ignore_ascii_case(s)))
            .or_else(|| self.entries().iter().find(|e| e.family.eq_ignore_ascii_case(family) && e.weight == 400 && !e.italic))
            .or_else(|| self.entries().iter().find(|e| e.family.eq_ignore_ascii_case(family)))?;
        let bytes = entry.bytes.get_or_init(|| std::fs::read(&entry.path).unwrap_or_default());
        if bytes.is_empty() {
            return None;
        }
        FontRef::from_index(bytes, entry.index).ok()
    }
}

impl Default for FontLibrary {
    fn default() -> Self {
        Self::new()
    }
}

/// The platform's font directories (env var wins are merged in).
pub fn default_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(Path::new(&home).join("Library").join("Fonts"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(windir) = std::env::var_os("WINDIR") {
            dirs.push(Path::new(&windir).join("Fonts"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(Path::new(&local).join("Microsoft").join("Windows").join("Fonts"));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(Path::new(&home).join(".local").join("share").join("fonts"));
            dirs.push(Path::new(&home).join(".fonts"));
        }
    }
    if let Some(extra) = std::env::var_os("OPENLASER_FONT_DIRS") {
        dirs.extend(std::env::split_paths(&extra));
    }
    dirs
}

fn font_ext(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()), Some(e) if matches!(e.as_str(), "ttf" | "otf" | "ttc" | "otc"))
}

/// Walk `dirs` bounded (depth ≤ 6, ≤ MAX_FILES candidates, no symlink cycles), parse each face's names.
fn scan(dirs: &[PathBuf]) -> Vec<FontEntry> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for root in dirs {
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if files.len() >= MAX_FILES {
                break;
            }
            if depth > 6 {
                continue;
            }
            if let Ok(canon) = dir.canonicalize()
                && !seen.insert(canon)
            {
                continue;
            }
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for item in read.flatten() {
                let path = item.path();
                if path.is_dir() {
                    stack.push((path, depth + 1));
                } else if font_ext(&path) {
                    files.push(path);
                    if files.len() >= MAX_FILES {
                        break;
                    }
                }
            }
        }
    }
    files.sort();
    for path in files {
        index_file(&path, &mut entries);
    }
    entries.sort_by(|a, b| a.family.cmp(&b.family).then(a.weight.cmp(&b.weight)).then(a.italic.cmp(&b.italic)).then(a.style.cmp(&b.style)));
    entries
}

/// The names of one face (typographic family/subfamily, falling back to family/subfamily).
fn face_names(font: &FontRef) -> Option<(String, String)> {
    let pick = |id| -> Option<String> { font.localized_strings(id).map(|s| s.to_string()).find(|s| !s.trim().is_empty()) };
    let family = pick(StringId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| pick(StringId::FAMILY_NAME))?;
    let style = pick(StringId::TYPOGRAPHIC_SUBFAMILY_NAME).or_else(|| pick(StringId::SUBFAMILY_NAME)).unwrap_or_else(|| "Regular".to_owned());
    Some((family, style))
}

/// Index every face of one file, bounded; a face that fails to parse is skipped, a panic is contained.
fn index_file(path: &Path, entries: &mut Vec<FontEntry>) {
    let Ok(meta) = path.metadata() else { return };
    if meta.len() > MAX_FONT_BYTES {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else { return };
    for index in 0..MAX_FACE_INDEX {
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let font = FontRef::from_index(&bytes, index).ok()?;
            let (family, style) = face_names(&font)?;
            let attrs = font.attributes();
            let weight = attrs.weight.value().clamp(1.0, 1000.0) as u16;
            let italic = attrs.style != skrifa::attribute::Style::Normal;
            Some((family, style, weight, italic))
        }));
        let Ok(inner) = read else {
            continue;
        };
        let Some((family, style, weight, italic)) = inner else {
            break;
        };
        entries.push(FontEntry { family, style, weight, italic, path: path.to_path_buf(), index, bytes: std::sync::OnceLock::new() });
    }
}

/// The family name a font advertises (for error messages).
fn family_of(font: &FontRef) -> String {
    face_names(font).map(|(f, _)| f).unwrap_or_else(|| "unknown".to_owned())
}

/// A pen that accumulates glyph outlines into a BezPath at `offset`, scaled font-units→mm.
struct BezPen {
    path: BezPath,
    scale: f64,
    origin: [f64; 2],
    points: usize,
}

impl BezPen {
    fn pt(&self, x: f32, y: f32) -> (f64, f64) {
        (self.origin[0] + x as f64 * self.scale, self.origin[1] + y as f64 * self.scale)
    }
}

impl OutlinePen for BezPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.points += 1;
        self.path.move_to(self.pt(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.points += 1;
        self.path.line_to(self.pt(x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.points += 2;
        let (c, p) = (self.pt(cx0, cy0), self.pt(x, y));
        self.path.quad_to(c, p);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.points += 3;
        let (c0, c1, p) = (self.pt(cx0, cy0), self.pt(cx1, cy1), self.pt(x, y));
        self.path.curve_to(c0, c1, p);
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Shape `text` with `font` into one Y-up path. `height_mm` = the cap height; the origin is the first line's
/// baseline anchor (left/centre/right per `align`). `letter_spacing_mm` is added to every advance;
/// `line_spacing` multiplies the font's line height. A character the font cannot show is a MissingGlyph error.
pub fn layout(font: &FontRef, text: &str, height_mm: f64, letter_spacing_mm: f64, line_spacing: f64, align: TextAlign) -> Result<BezPath, TextError> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(TextError::TooLong);
    }
    if !height_mm.is_finite() || !(MIN_HEIGHT_MM..=MAX_HEIGHT_MM).contains(&height_mm) {
        return Err(TextError::BadHeight(height_mm));
    }
    if !letter_spacing_mm.is_finite() || !line_spacing.is_finite() {
        return Err(TextError::NonFinite);
    }
    // Every character must have a real glyph; refuse them all at once with the family name.
    let missing: HashSet<char> = text.chars().filter(|c| !matches!(c, '\n' | '\r')).filter(|c| font.charmap().map(*c).is_none()).collect();
    if !missing.is_empty() {
        let mut chars: Vec<char> = missing.into_iter().collect();
        chars.sort();
        return Err(TextError::MissingGlyph { family: family_of(font), chars: chars.into_iter().collect() });
    }
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let cap = metrics.cap_height.unwrap_or(0.7 * metrics.units_per_em as f32);
    if cap <= 0.0 {
        return Err(TextError::Unreadable(family_of(font)));
    }
    let scale = height_mm / cap as f64;
    let line_step = line_spacing.max(0.0) * ((metrics.ascent - metrics.descent + metrics.leading).max(0.0) as f64) * scale;
    let data = harfrust::ShaperData::new(font);
    let shaper = data.shaper(font).build();
    let outlines = font.outline_glyphs();
    let mut pen = BezPen { path: BezPath::new(), scale, origin: [0.0, 0.0], points: 0 };
    for (line_no, line) in text.split('\n').enumerate() {
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(line);
        buffer.set_direction(harfrust::Direction::LeftToRight);
        buffer.guess_segment_properties();
        let glyphs = shaper.shape(buffer, harfrust::ShapeOptions::new());
        let infos = glyphs.glyph_infos();
        let positions = glyphs.glyph_positions();
        let mut advance: f64 = 0.0;
        for (i, pos) in positions.iter().enumerate() {
            advance += pos.x_advance as f64 * scale + if i + 1 < positions.len() { letter_spacing_mm } else { 0.0 };
        }
        let align_shift = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => -advance / 2.0,
            TextAlign::Right => -advance,
        };
        let mut x = align_shift;
        let y = -(line_no as f64) * line_step;
        for (info, pos) in infos.iter().zip(positions.iter()) {
            if pen.points > MAX_PATH_POINTS {
                return Err(TextError::TooManyPoints(pen.points));
            }
            pen.origin = [x + pos.x_offset as f64 * scale, y + pos.y_offset as f64 * scale];
            if let Some(glyph) = outlines.get(skrifa::GlyphId::new(info.glyph_id)) {
                let draw = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = glyph.draw(draw, &mut pen);
                }))
                .map_err(|_| TextError::Unreadable(family_of(font)))?;
            }
            x += pos.x_advance as f64 * scale + letter_spacing_mm;
        }
    }
    if pen.points > MAX_PATH_POINTS {
        return Err(TextError::TooManyPoints(pen.points));
    }
    Ok(pen.path)
}

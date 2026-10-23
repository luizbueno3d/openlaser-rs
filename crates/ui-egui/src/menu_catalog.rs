//! Every LightBurn menu item from CAT §3, in LightBurn order, "---" separators, ids converted to camelCase.
//!
//! Pattern ported from PhotoCraft `crates/ui-egui/src/menu_catalog.rs` (MIT OR Apache-2.0); the rows are OpenLaser's
//! own, written from `docs/design/lightburn-command-catalog.md` (CAT §3). LightBurn was studied for its menu
//! layout only: no LightBurn text is copied, and LightBurn's name never appears in a label.
//!
//! Rules:
//! - A row is live when the engine registers its id or the shell handles it (`crate::shell`); every other row
//!   renders disabled ("Not in OpenLaser yet").
//! - **Human-only commands never get a menu item or a shortcut (S18).** CAT rows whose command would drive the
//!   hardware (`hw`/`arm` in CAT's agent column) keep their LightBurn place for parity, but their id carries the
//!   [`HW_PREFIX`] (`hw:tools.positionLaser`): no registry id contains a colon, so such a row can never resolve to
//!   a command, never counts as live and never has a shortcut. Hardware actions live in the Laser panel only.
//! - Shortcuts use the strict grammar of `crate::shortcuts` (`Cmd` = ⌘ on macOS and Ctrl elsewhere, `Ctrl` = the
//!   Control key, `Alt` = ⌥/Alt), LightBurn's defaults with CAT §10's macOS decisions applied. Esc is never in
//!   this table: it is STOP while a machine is active (S18).
//! - Panel rows dispatch `window.show {panel, visible}` as specified in Appendix A.

/// One catalogue row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub path: &'static [&'static str],
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub id: &'static str,
    pub panel: Option<&'static str>,
}

/// The id of a separator row (its label is "---" too).
pub const SEPARATOR: &str = "---";

/// Prefix of rows that stand for a hardware action: never dispatched from a menu (S18).
pub const HW_PREFIX: &str = "hw:";

const fn r(path: &'static [&'static str], label: &'static str, shortcut: Option<&'static str>, id: &'static str) -> Row {
    Row { path, label, shortcut, id, panel: None }
}

const fn sep(path: &'static [&'static str]) -> Row {
    Row { path, label: SEPARATOR, shortcut: None, id: SEPARATOR, panel: None }
}

const fn panel(path: &'static [&'static str], label: &'static str, panel: &'static str) -> Row {
    Row { path, label, shortcut: None, id: "window.show", panel: Some(panel) }
}

/// The top-level menus in LightBurn's order (macOS menu bar; the app menu is built by `crate::native_menu`).
pub const MENUS: &[&str] = &["File", "Edit", "Tools", "Arrange", "Laser Tools", "Window", "Language", "Help"];

const FILE: &[&str] = &["File"];
const FILE_PREFS: &[&str] = &["File", "Preferences"];
const FILE_BUNDLES: &[&str] = &["File", "Bundles"];
const EDIT: &[&str] = &["Edit"];
const EDIT_IMAGE: &[&str] = &["Edit", "Image Options"];
const TOOLS: &[&str] = &["Tools"];
const TOOLS_SHAPE: &[&str] = &["Tools", "Draw Shape"];
const ARRANGE: &[&str] = &["Arrange"];
const ARRANGE_ALIGN: &[&str] = &["Arrange", "Align"];
const ARRANGE_DISTRIBUTE: &[&str] = &["Arrange", "Distribute"];
const ARRANGE_DOCK: &[&str] = &["Arrange", "Dock"];
const ARRANGE_MOVE_SEL: &[&str] = &["Arrange", "Move Selected Objects"];
const ARRANGE_MOVE_LASER: &[&str] = &["Arrange", "Move Laser to Selection"];
const ARRANGE_VARRAY: &[&str] = &["Arrange", "Virtual Array"];
const ARRANGE_ORDER: &[&str] = &["Arrange", "Draw Order"];
const LASER_TOOLS: &[&str] = &["Laser Tools"];
const WINDOW: &[&str] = &["Window"];
const WINDOW_ZOOM: &[&str] = &["Window", "Zoom"];
const WINDOW_STYLE: &[&str] = &["Window", "View Style"];
const LANGUAGE: &[&str] = &["Language"];
const HELP: &[&str] = &["Help"];

/// Every LightBurn menu item from CAT §3, in LightBurn order, "---" separators, ids converted to camelCase.
pub static CATALOG: &[Row] = &[
    // ---- File (CAT §3.1) ----
    r(FILE, "New", Some("Cmd+N"), "file.new"),
    r(FILE, "New Window", None, "file.newWindow"),
    r(FILE, "Recent Projects", None, "file.openRecent"),
    r(FILE, "Recent Templates", None, "file.openRecentTemplate"),
    r(FILE, "Open…", Some("Cmd+O"), "file.open"),
    r(FILE, "Import…", Some("Cmd+I"), "file.import"),
    sep(FILE),
    r(FILE, "Show Notes", Some("Cmd+Alt+N"), "file.notes"),
    r(FILE, "Save", Some("Cmd+S"), "file.save"),
    r(FILE, "Save As…", Some("Cmd+Shift+S"), "file.saveAs"),
    r(FILE, "Save As Template…", None, "file.saveAsTemplate"),
    r(FILE, "Save GCode…", Some("Alt+Shift+L"), "file.saveMachineFile"),
    r(FILE, "Export…", Some("Alt+X"), "file.export"),
    r(FILE, "Print (black only)", Some("Cmd+P"), "file.printBlack"),
    sep(FILE),
    r(FILE, "Print (keep colors)", Some("Cmd+Shift+P"), "file.printColors"),
    sep(FILE),
    r(FILE_PREFS, "Import Prefs…", None, "file.prefs.import"),
    r(FILE_PREFS, "Export Prefs…", None, "file.prefs.export"),
    r(FILE_PREFS, "Open Prefs Folder", None, "file.prefs.openFolder"),
    r(FILE_PREFS, "Load Prefs Backup…", None, "file.prefs.loadBackup"),
    r(FILE_PREFS, "Edit Hotkeys…", None, "file.prefs.editHotkeys"),
    r(FILE_BUNDLES, "Import Bundles…", None, "file.bundle.import"),
    r(FILE_BUNDLES, "Export Bundle…", None, "file.bundle.export"),
    sep(FILE),
    r(FILE, "Save Processed Bitmap…", None, "file.saveProcessedBitmap"),
    r(FILE, "Save Background Capture…", None, "file.saveBackgroundCapture"),
    sep(FILE),
    r(FILE, "Exit", Some("Cmd+Q"), "file.exit"),
    // ---- Edit (CAT §3.2) ----
    r(EDIT, "Undo", Some("Cmd+Z"), "edit.undo"),
    r(EDIT, "Redo", Some("Cmd+Shift+Z"), "edit.redo"),
    r(EDIT, "Undo History", None, "edit.undoHistory"),
    sep(EDIT),
    r(EDIT, "Select All", Some("Cmd+A"), "edit.selectAll"),
    r(EDIT, "Invert Selection", Some("Cmd+Shift+I"), "edit.selectInvert"),
    r(EDIT, "Cut", Some("Cmd+X"), "edit.cut"),
    r(EDIT, "Copy", Some("Cmd+C"), "edit.copy"),
    r(EDIT, "Duplicate", Some("Cmd+D"), "edit.duplicate"),
    r(EDIT, "Paste", Some("Cmd+V"), "edit.paste"),
    r(EDIT, "Paste in Place", Some("Alt+V"), "edit.pasteInPlace"),
    r(EDIT, "Delete", Some("Delete"), "edit.delete"),
    sep(EDIT),
    r(EDIT, "Convert to Path", Some("Cmd+Shift+C"), "edit.convertToPath"),
    r(EDIT, "Convert to Bitmap…", None, "edit.convertToBitmap"),
    r(EDIT, "Generate Path From Cut", None, "edit.generatePathFromCut"),
    r(EDIT, "Close Path", None, "edit.closePath"),
    r(EDIT, "Close Selected Paths with Tolerance…", None, "edit.closePathsTolerance"),
    r(EDIT, "Auto-Join Selected Shapes", Some("Alt+J"), "edit.autoJoin"),
    sep(EDIT),
    r(EDIT, "Optimize Selected Shapes…", Some("Alt+Shift+O"), "edit.optimizeShapes"),
    r(EDIT, "Delete Duplicates", Some("Alt+D"), "edit.deleteDuplicates"),
    r(EDIT, "Reverse Shape Direction", None, "edit.reverseDirection"),
    r(EDIT, "Select Open Shapes", None, "edit.selectOpenShapes"),
    r(EDIT, "Select Open Shapes Set to Fill", None, "edit.selectOpenFillShapes"),
    r(EDIT, "Select All Shapes in Current Layer", None, "edit.selectLayerShapes"),
    r(EDIT, "Select Contained Shapes", None, "edit.selectContained"),
    r(EDIT, "Select Shapes Smaller Than Selected", None, "edit.selectSmaller"),
    r(EDIT, "Select Circles…", None, "edit.selectCircles"),
    sep(EDIT),
    r(EDIT_IMAGE, "Refresh Image", None, "edit.image.refresh"),
    r(EDIT_IMAGE, "Replace Image…", None, "edit.image.replace"),
    r(EDIT_IMAGE, "Replace Image to Fit…", None, "edit.image.replaceFit"),
    sep(EDIT),
    r(EDIT, "Settings…", Some("Cmd+Comma"), "edit.settings"),
    r(EDIT, "Device Settings…", None, "edit.deviceSettings"),
    // ---- Tools (CAT §3.3) ----
    r(TOOLS, "Select", None, "tools.select"),
    r(TOOLS, "Draw Lines", Some("Cmd+L"), "tools.drawLines"),
    r(TOOLS_SHAPE, "Rectangle", Some("Cmd+R"), "tools.rectangle"),
    r(TOOLS_SHAPE, "Ellipse", Some("Cmd+E"), "tools.ellipse"),
    r(TOOLS_SHAPE, "Triangle", None, "tools.triangle"),
    r(TOOLS_SHAPE, "Pentagon", None, "tools.pentagon"),
    r(TOOLS_SHAPE, "Polygon", None, "tools.polygon"),
    r(TOOLS_SHAPE, "Octagon", None, "tools.octagon"),
    r(TOOLS_SHAPE, "Star", None, "tools.star"),
    r(TOOLS_SHAPE, "Dual Star", None, "tools.dualStar"),
    r(TOOLS, "Edit Nodes", Some("Ctrl+Backtick"), "tools.editNodes"),
    r(TOOLS, "Trim Shapes", Some("Cmd+K"), "tools.trimShapes"),
    r(TOOLS, "Add Tabs", Some("Ctrl+Tab"), "tools.addTabs"),
    r(TOOLS, "Edit Text", Some("Cmd+T"), "tools.text"),
    r(TOOLS, "Position Laser", None, "hw:tools.positionLaser"),
    r(TOOLS, "Measure", Some("Alt+M"), "tools.measure"),
    r(TOOLS, "Create Bar Code…", None, "tools.barcode"),
    sep(TOOLS),
    r(TOOLS, "Offset Shapes…", Some("Alt+O"), "tools.offsetShapes"),
    r(TOOLS, "Weld Shapes", Some("Cmd+W"), "tools.weld"),
    r(TOOLS, "Boolean Union", Some("Alt+Plus"), "tools.booleanUnion"),
    r(TOOLS, "Boolean Subtract", Some("Alt+Minus"), "tools.booleanSubtract"),
    r(TOOLS, "Boolean Intersection", Some("Alt+Shift+8"), "tools.booleanIntersect"),
    r(TOOLS, "Boolean Assistant…", Some("Cmd+B"), "tools.booleanAssistant"),
    r(TOOLS, "Cut Shapes", Some("Alt+Shift+C"), "tools.cutShapes"),
    sep(TOOLS),
    r(TOOLS, "Adjust Image…", Some("Alt+I"), "tools.adjustImage"),
    r(TOOLS, "Trace Image…", Some("Alt+T"), "tools.traceImage"),
    r(TOOLS, "Multi-File Trace Image (Labs)…", None, "tools.traceMulti"),
    r(TOOLS, "Apply Path to Text", None, "tools.applyPathToText"),
    r(TOOLS, "Apply Mask to Image", None, "tools.applyMask"),
    r(TOOLS, "Crop Image", None, "tools.cropImage"),
    sep(TOOLS),
    r(TOOLS, "Resize Slots in Selection…", None, "tools.resizeSlots"),
    r(TOOLS, "Warp Selection (4 Points)", None, "tools.warp"),
    r(TOOLS, "Deform Selection (16 Points)", None, "tools.deform"),
    r(TOOLS, "Generate Tangent Circle", None, "tools.tangentCircle"),
    r(TOOLS, "Set Start Point", None, "tools.setStartPoint"),
    r(TOOLS, "Radius / Fillet", None, "tools.radius"),
    // ---- Arrange (CAT §3.4) ----
    r(ARRANGE, "Group", Some("Cmd+G"), "arrange.group"),
    r(ARRANGE, "Ungroup", Some("Cmd+U"), "arrange.ungroup"),
    r(ARRANGE, "Auto-Group", None, "arrange.autoGroup"),
    sep(ARRANGE),
    r(ARRANGE, "Flip Horizontal", Some("Cmd+Shift+H"), "arrange.flipH"),
    r(ARRANGE, "Flip Vertical", Some("Cmd+Shift+V"), "arrange.flipV"),
    r(ARRANGE, "Mirror Across Line", Some("Cmd+Shift+M"), "arrange.mirrorAcrossLine"),
    r(ARRANGE, "Rotate 90° Clockwise", Some("Period"), "arrange.rotateCw90"),
    r(ARRANGE, "Rotate 90° Counter-Clockwise", Some("Comma"), "arrange.rotateCcw90"),
    r(ARRANGE, "Two-Point Rotate / Scale", Some("Cmd+2"), "arrange.twoPoint"),
    sep(ARRANGE),
    r(ARRANGE_ALIGN, "Align Left", Some("Alt+Left"), "arrange.align.left"),
    r(ARRANGE_ALIGN, "Align Right", Some("Alt+Right"), "arrange.align.right"),
    r(ARRANGE_ALIGN, "Align Top", Some("Alt+Up"), "arrange.align.top"),
    r(ARRANGE_ALIGN, "Align Bottom", Some("Alt+Down"), "arrange.align.bottom"),
    r(ARRANGE_ALIGN, "Align Vertical Centers", Some("Alt+PageUp"), "arrange.align.vCenters"),
    r(ARRANGE_ALIGN, "Align Horizontal Centers", Some("Alt+PageDown"), "arrange.align.hCenters"),
    r(ARRANGE_ALIGN, "Align Centers", Some("Alt+Home"), "arrange.align.centers"),
    r(ARRANGE, "Make Same Width", None, "arrange.sameWidth"),
    r(ARRANGE, "Make Same Height", None, "arrange.sameHeight"),
    r(ARRANGE_DISTRIBUTE, "Distribute H-Spaced", None, "arrange.distribute.hSpaced"),
    r(ARRANGE_DISTRIBUTE, "Distribute H-Centered", None, "arrange.distribute.hCentered"),
    r(ARRANGE_DISTRIBUTE, "Distribute V-Spaced", None, "arrange.distribute.vSpaced"),
    r(ARRANGE_DISTRIBUTE, "Distribute V-Centered", None, "arrange.distribute.vCentered"),
    sep(ARRANGE_DISTRIBUTE),
    r(ARRANGE_DISTRIBUTE, "Move H Together", Some("Alt+Shift+H"), "arrange.moveTogether.h"),
    r(ARRANGE_DISTRIBUTE, "Move V Together", Some("Alt+Shift+V"), "arrange.moveTogether.v"),
    r(ARRANGE_DOCK, "Dock Left", None, "arrange.dock.left"),
    r(ARRANGE_DOCK, "Dock Right", None, "arrange.dock.right"),
    r(ARRANGE_DOCK, "Dock Up", None, "arrange.dock.up"),
    r(ARRANGE_DOCK, "Dock Down", None, "arrange.dock.down"),
    sep(ARRANGE),
    r(ARRANGE_MOVE_SEL, "Page Center", Some("P"), "arrange.moveSelection.pageCenter"),
    r(ARRANGE_MOVE_SEL, "Upper Left", None, "arrange.moveSelection.upperLeft"),
    r(ARRANGE_MOVE_SEL, "Upper Right", None, "arrange.moveSelection.upperRight"),
    r(ARRANGE_MOVE_SEL, "Lower Left", None, "arrange.moveSelection.lowerLeft"),
    r(ARRANGE_MOVE_SEL, "Lower Right", None, "arrange.moveSelection.lowerRight"),
    r(ARRANGE_MOVE_SEL, "Left", None, "arrange.moveSelection.left"),
    r(ARRANGE_MOVE_SEL, "Right", None, "arrange.moveSelection.right"),
    r(ARRANGE_MOVE_SEL, "Top", None, "arrange.moveSelection.top"),
    r(ARRANGE_MOVE_SEL, "Bottom", None, "arrange.moveSelection.bottom"),
    sep(ARRANGE_MOVE_SEL),
    r(ARRANGE_MOVE_SEL, "Move to Laser Position", None, "hw:arrange.moveSelection.laserPosition"),
    r(ARRANGE_MOVE_LASER, "Center", None, "hw:arrange.moveLaser.center"),
    r(ARRANGE_MOVE_LASER, "Upper Left", None, "hw:arrange.moveLaser.upperLeft"),
    r(ARRANGE_MOVE_LASER, "Upper Right", None, "hw:arrange.moveLaser.upperRight"),
    r(ARRANGE_MOVE_LASER, "Lower Left", None, "hw:arrange.moveLaser.lowerLeft"),
    r(ARRANGE_MOVE_LASER, "Lower Right", None, "hw:arrange.moveLaser.lowerRight"),
    r(ARRANGE_MOVE_LASER, "Left", None, "hw:arrange.moveLaser.left"),
    r(ARRANGE_MOVE_LASER, "Right", None, "hw:arrange.moveLaser.right"),
    r(ARRANGE_MOVE_LASER, "Top", None, "hw:arrange.moveLaser.top"),
    r(ARRANGE_MOVE_LASER, "Bottom", None, "hw:arrange.moveLaser.bottom"),
    sep(ARRANGE_MOVE_LASER),
    r(ARRANGE_MOVE_LASER, "Jog Laser", None, "hw:arrange.jog"),
    sep(ARRANGE),
    r(ARRANGE, "Nesting…", None, "arrange.nest"),
    r(ARRANGE, "SVGNest (External Tool)", None, "arrange.svgnest"),
    sep(ARRANGE),
    r(ARRANGE, "Grid Array…", None, "arrange.gridArray"),
    r(ARRANGE_VARRAY, "Create Virtual Array", None, "arrange.virtualArray.create"),
    r(ARRANGE_VARRAY, "Edit Virtual Array", None, "arrange.virtualArray.edit"),
    r(ARRANGE_VARRAY, "Duplicate Virtual Array", None, "arrange.virtualArray.duplicate"),
    r(ARRANGE_VARRAY, "Flatten Virtual Array", None, "arrange.virtualArray.flatten"),
    r(ARRANGE_VARRAY, "Remove from Virtual Array", None, "arrange.virtualArray.remove"),
    r(ARRANGE_VARRAY, "Add to Virtual Array", None, "arrange.virtualArray.add"),
    r(ARRANGE_VARRAY, "Exit Virtual Array", None, "arrange.virtualArray.exit"),
    r(ARRANGE, "Circular Array…", None, "arrange.circularArray"),
    r(ARRANGE, "Copy Along Path…", None, "arrange.copyAlongPath"),
    r(ARRANGE, "Create Rubber-Band Outline from Selection", None, "arrange.rubberBandOutline"),
    r(ARRANGE, "Break Apart", Some("Alt+B"), "arrange.breakApart"),
    sep(ARRANGE),
    r(ARRANGE_ORDER, "Push Forward", Some("PageDown"), "arrange.order.forward"),
    r(ARRANGE_ORDER, "Push Backward", Some("PageUp"), "arrange.order.backward"),
    r(ARRANGE_ORDER, "Push to Front", Some("Cmd+PageUp"), "arrange.order.front"),
    r(ARRANGE_ORDER, "Push to Back", Some("Cmd+PageDown"), "arrange.order.back"),
    sep(ARRANGE),
    r(ARRANGE, "Lock Selected Shapes", None, "arrange.lock"),
    r(ARRANGE, "Unlock Selected Shapes", None, "arrange.unlock"),
    // ---- Laser Tools (CAT §3.5) ----
    r(LASER_TOOLS, "Print and Cut", None, "hw:laserTools.printAndCut"),
    r(LASER_TOOLS, "Manage Cameras…", None, "laserTools.cameras"),
    r(LASER_TOOLS, "Rotary Setup…", Some("Cmd+Shift+R"), "laserTools.rotarySetup"),
    r(LASER_TOOLS, "Cylinder Correction Setup…", None, "laserTools.cylinderCorrection"),
    sep(LASER_TOOLS),
    r(LASER_TOOLS, "Split Marking…", None, "hw:laserTools.splitMarking"),
    r(LASER_TOOLS, "Repeat Marking…", None, "hw:laserTools.repeatMarking"),
    r(LASER_TOOLS, "Focus Test…", None, "laserTools.focusTest"),
    r(LASER_TOOLS, "Interval Test…", None, "laserTools.intervalTest"),
    r(LASER_TOOLS, "Material Test…", None, "laserTools.materialTest"),
    r(LASER_TOOLS, "Center Finder…", None, "hw:laserTools.centerFinder"),
    sep(LASER_TOOLS),
    r(LASER_TOOLS, "Taper Warp…", None, "laserTools.taperWarp"),
    r(LASER_TOOLS, "Calibrate Galvo Lens…", None, "laserTools.calibrateGalvoLens"),
    sep(LASER_TOOLS),
    r(LASER_TOOLS, "Manage Devices…", None, "device.manage"),
    r(LASER_TOOLS, "Device Settings…", None, "device.settings"),
    r(LASER_TOOLS, "Machine Settings…", None, "hw:device.machineSettings"),
    // ---- Window (CAT §3.7, §5) ----
    panel(WINDOW, "Cuts / Layers", "cuts"),
    panel(WINDOW, "Laser", "laser"),
    panel(WINDOW, "Library", "library"),
    panel(WINDOW, "Move", "move"),
    panel(WINDOW, "Console", "console"),
    panel(WINDOW, "Shape Properties", "shapeProperties"),
    panel(WINDOW, "Variable Text", "variableText"),
    panel(WINDOW, "Camera Control", "camera"),
    panel(WINDOW, "Art Library", "artLibrary"),
    panel(WINDOW, "Undo History", "undoHistory"),
    panel(WINDOW, "File List", "fileList"),
    r(WINDOW, "Live Framing", Some("F1"), "laser.liveFraming"),
    sep(WINDOW),
    r(WINDOW, "Reset to Default Layout", None, "window.resetLayout"),
    sep(WINDOW),
    r(WINDOW_ZOOM, "Zoom In", Some("Cmd+Equals"), "view.zoomIn"),
    r(WINDOW_ZOOM, "Zoom Out", Some("Cmd+Minus"), "view.zoomOut"),
    r(WINDOW_ZOOM, "Zoom to Page", Some("Cmd+0"), "view.zoomPage"),
    r(WINDOW_ZOOM, "Frame Selection", Some("Cmd+Shift+A"), "view.zoomSelection"),
    r(WINDOW, "Preview", Some("Alt+P"), "preview.open"),
    r(WINDOW_STYLE, "Wireframe", None, "view.style.wireframe"),
    r(WINDOW_STYLE, "Filled", None, "view.style.filled"),
    r(WINDOW_STYLE, "Smooth Edges", None, "view.edgeQuality"),
    sep(WINDOW),
    r(WINDOW, "Pan / Drag View", None, "view.pan"),
    r(WINDOW, "Toggle Side Panels", Some("F12"), "window.toggleSidePanels"),
    r(WINDOW, "Lock Window Layout", None, "window.lockLayout"),
    // ---- Language (CAT §3.8) ----
    r(LANGUAGE, "English", None, "prefs.language.en"),
    r(LANGUAGE, "Português (Brasil)", None, "prefs.language.ptBr"),
    // ---- Help (CAT §3.9) ----
    r(HELP, "About OpenLaser", None, "help.about"),
    r(HELP, "License", None, "help.license"),
    r(HELP, "Support Forum", None, "help.forum"),
    r(HELP, "Submit Feature Request", None, "help.featureRequest"),
    sep(HELP),
    r(HELP, "Quick Help and Notes", Some("Cmd+Shift+Slash"), "help.quickHelp"),
    r(HELP, "Online Documentation", None, "help.docs"),
    r(HELP, "PDF Documentation Link", None, "help.docsPdf"),
    r(HELP, "Online Video Tutorials", None, "help.videos"),
    sep(HELP),
    r(HELP, "Report a Bug", None, "help.reportBug"),
    r(HELP, "Generate Support Data", None, "help.supportData"),
    r(HELP, "Camera Selection Help", None, "help.cameraSelection"),
    r(HELP, "Check for Updates", None, "help.checkUpdates"),
    sep(HELP),
    r(HELP, "Enable Debug Log", None, "help.debugLog"),
    r(HELP, "Debug Drawing", None, "help.debugDrawing"),
    r(HELP, "Real-Time Statistics", None, "help.networkCameraStats"),
];

/// True for a row that stands for a hardware action (never dispatched, never live; S18).
pub fn is_hw_row(row: &Row) -> bool {
    row.id.starts_with(HW_PREFIX)
}

/// True for a separator row.
pub fn is_separator(row: &Row) -> bool {
    row.id == SEPARATOR
}

/// The catalogue row of a command id (the first, if it appears twice), separators and hardware rows excluded.
pub fn find(id: &str) -> Option<&'static Row> {
    CATALOG.iter().find(|r| !is_separator(r) && !is_hw_row(r) && r.id == id)
}

/// The ids a menu can dispatch: every row except separators and hardware rows, each once.
pub fn dispatchable_ids() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for row in CATALOG {
        if is_separator(row) || is_hw_row(row) || out.contains(&row.id) {
            continue;
        }
        out.push(row.id);
    }
    out
}

/// `layer.cut_settings.set` → `layer.cutSettings.set`, `flip_h` → `flipH` (S2: segment by segment).
pub fn camel_id(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    for (i, seg) in snake.split('.').enumerate() {
        if i > 0 {
            out.push('.');
        }
        let mut upper = false;
        for c in seg.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                out.extend(c.to_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        }
    }
    out
}

/// Live engine commands and implemented shell actions share the same native and egui menu policy.
pub fn is_live(row: &Row) -> bool {
    !is_hw_row(row) && (openlaser_engine::registry().find(row.id).is_some() || is_shell_row(row))
}

pub fn is_shell_row(row: &Row) -> bool {
    match row.id {
        "window.show" => row.panel.is_some_and(|p| crate::dock::PANELS.contains(&p)),
        "file.open" | "file.import" | "file.exit" | "preview.open" | "edit.settings" | "edit.deviceSettings" | "device.settings" | "laser.liveFraming" => true,
        _ => false,
    }
}

/// Stable native-menu identity; panel commands share a command id but have distinct parameters.
pub fn native_id(row: &Row) -> String {
    row.panel.map(|p| format!("window.show:{p}")).unwrap_or_else(|| row.id.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_conversion_matches_s2() {
        assert_eq!(camel_id("layer.cut_settings.set"), "layer.cutSettings.set");
        assert_eq!(camel_id("arrange.flip_h"), "arrange.flipH");
        assert_eq!(camel_id("arrange.rotate_cw90"), "arrange.rotateCw90");
        assert_eq!(camel_id("laser_tools.print_and_cut"), "laserTools.printAndCut");
        assert_eq!(camel_id("file.new"), "file.new");
    }

    #[test]
    fn every_path_starts_with_a_known_menu_and_labels_never_name_lightburn() {
        for row in CATALOG {
            let top = row.path.first().copied().unwrap_or_default();
            assert!(MENUS.contains(&top), "{row:?}");
            assert!(!row.label.contains("LightBurn"), "LightBurn's name never appears in the UI: {row:?}");
        }
    }

    #[test]
    fn hardware_rows_have_no_shortcut() {
        for row in CATALOG.iter().filter(|r| is_hw_row(r)) {
            assert!(row.shortcut.is_none(), "{row:?}");
        }
    }

    #[test]
    fn ids_follow_the_registry_grammar() {
        // `^[a-z][a-zA-Z0-9]*(\.[a-z][a-zA-Z0-9]*)+$` (WP6 `registry_id_grammar`), after the hw: marker.
        for row in CATALOG.iter().filter(|r| !is_separator(r)) {
            let id = row.id.strip_prefix(HW_PREFIX).unwrap_or(row.id);
            let segs: Vec<&str> = id.split('.').collect();
            assert!(segs.len() >= 2, "{id}");
            for s in segs {
                let mut chars = s.chars();
                assert!(chars.next().is_some_and(|c| c.is_ascii_lowercase()), "{id}");
                assert!(chars.all(|c| c.is_ascii_alphanumeric()), "{id}");
            }
        }
    }

    #[test]
    fn menu_catalog_matches_cat() {
        assert_eq!(MENUS, &["File", "Edit", "Tools", "Arrange", "Laser Tools", "Window", "Language", "Help"]);
        // CAT §3 is the source of truth. Keep the complete menu table tied to the
        // reference document, including row and separator counts, then verify every
        // directly named CAT command is represented in the UI catalogue.
        let cat = include_str!("../../../docs/design/lightburn-command-catalog.md");
        let menus = cat.split("## 3. Menus").nth(1).and_then(|s| s.split("\n## 4.").next()).unwrap_or_default();
        let cat_rows: Vec<&str> = menus
            .lines()
            .filter(|line| line.starts_with("| P0 | `") || line.starts_with("| P1 | `") || line.starts_with("| P2 | `") || line.starts_with("| P3 | `"))
            .collect();
        assert_eq!(cat_rows.len(), 196, "CAT §3 changed: review the menu catalogue against the new source rows");
        assert_eq!(CATALOG.iter().filter(|row| is_separator(row)).count(), 32, "menu separators changed; review their positions against CAT §3");
        assert_eq!(CATALOG.iter().filter(|row| !is_separator(row)).count(), 215, "menu item count changed; review every item against CAT §3");

        let mut documented = std::collections::BTreeSet::new();
        let mut order = std::collections::BTreeMap::new();
        for row in &cat_rows {
            let id_cell = row.split('|').nth(2).unwrap_or_default();
            let mut base = String::new();
            for id in id_cell.replace('`', "").split(" / ").map(str::trim) {
                let id = if id.starts_with('.') && !base.is_empty() {
                    format!("{}{id}", base.rsplit_once('.').map(|(prefix, _)| prefix).unwrap_or(""))
                } else {
                    base = id.to_owned();
                    id.to_owned()
                };
                if let Some((prefix, variants)) = id.split_once(".{") {
                    if let Some(variants) = variants.strip_suffix('}') {
                        for variant in variants.split(',').map(str::trim) {
                            let id = camel_id(&format!("{prefix}.{variant}"));
                            let next = order.len();
                            order.entry(id.clone()).or_insert(next);
                            documented.insert(id);
                        }
                    }
                } else if id.contains('.') && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_') {
                    let id = camel_id(&id);
                    let next = order.len();
                    order.entry(id.clone()).or_insert(next);
                    documented.insert(id);
                }
            }
        }
        let mut previous = std::collections::BTreeMap::new();
        for row in CATALOG.iter().filter(|row| !is_separator(row)) {
            if let Some(rank) = order.get(row.id.strip_prefix(HW_PREFIX).unwrap_or(row.id))
                && let Some((last, label)) = previous.insert(row.path, (*rank, row.label))
            {
                assert!(*rank >= last, "CAT order changed: {} precedes {} in {:?}", row.label, label, row.path);
            }
        }
        for row in CATALOG.iter().filter(|row| !is_separator(row)) {
            let id = row.id.strip_prefix(HW_PREFIX).unwrap_or(row.id);
            let documented_directly = documented.contains(id);
            let expanded_catalog_id = id.contains('.') && documented.iter().any(|entry| entry.starts_with(id) || id.starts_with(entry));
            let shell_panel = id == "window.show" || id == "window.lockLayout";
            let language_choice = id.starts_with("prefs.language.");
            let live_framing = id == "laser.liveFraming"; // CAT lists this in the Laser panel inventory, not §3.
            assert!(
                documented_directly || expanded_catalog_id || shell_panel || language_choice || live_framing,
                "catalogue id `{id}` is not backed by a CAT §3 command"
            );
        }

        for row in CATALOG {
            assert!(!row.path.is_empty(), "menu rows need a path: {row:?}");
            if is_separator(row) {
                assert!(row.shortcut.is_none(), "separators cannot have shortcuts: {row:?}");
            } else {
                assert!(!row.label.is_empty(), "menu rows need a visible label: {row:?}");
            }
        }
    }

    #[test]
    #[ignore = "needs I2"]
    fn catalog_shortcuts_match_registry() {
        for row in CATALOG.iter().filter(|row| !is_separator(row) && !is_hw_row(row)) {
            if let Some(spec) = openlaser_engine::registry().find(row.id) {
                assert_eq!(row.shortcut, spec.shortcut, "shortcut mismatch for {}", row.id);
                assert_ne!(spec.access, openlaser_engine::Access::Human, "human-only command has a menu row: {}", row.id);
            }
        }
        for spec in openlaser_engine::registry().specs().iter().filter(|spec| !spec.menu.is_empty()) {
            let row = CATALOG.iter().find(|row| !is_separator(row) && !is_hw_row(row) && row.id == spec.id);
            assert!(row.is_some(), "registered menu command missing from catalogue: {}", spec.id);
        }
    }
}

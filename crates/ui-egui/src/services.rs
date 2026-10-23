//! Injected services and the human-surface seam.

use openlaser_engine::EmissionSummary;
use openlaser_render::DisplayList;
use openlaser_types::{ArmToken, FrameReceipt, MonoTime, SessionId};

/// Everything platform- or safety-specific is injected; ui-egui does no file or device I/O itself.
pub struct Services {
    pub human: Box<dyn HumanSurfaces>,
    /// Async (rfd) in the app; scripted in tests.
    pub files: Box<dyn FileDialogs>,
    /// craft-settings in the app; in memory in tests.
    pub settings: Box<dyn SettingsStore>,
    pub clipboard: Box<dyn Clipboard>,
    /// Settings › Automation › Allow agent connections (a human-only switch).
    pub automation: Box<dyn AutomationToggle>,
    pub quit: Box<dyn Fn() + Send>,
}

/// What a file dialog is for (WP0 fill-in; WP8 may extend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialogKind {
    OpenProject,
    Import,
    SaveProject,
    ExportGcode,
    ExportJob,
    ExportSvg,
}

/// A pending dialog (WP0 fill-in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DialogTicket(pub u64);

pub trait FileDialogs: Send {
    fn open(&mut self, kind: DialogKind) -> DialogTicket;
    fn poll(&mut self, t: DialogTicket) -> Option<Option<(String, Vec<u8>)>>;
    fn save(&mut self, kind: DialogKind, bytes: Vec<u8>) -> DialogTicket;
}

/// Persisted UI settings (WP0 fill-in; WP8 adds the fields).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct UiSettings {
    pub in_window_menus: bool,
    /// Stable serialized docking tree. Invalid values are ignored and replaced with the default layout.
    #[serde(default)]
    pub dock_layout: String,
    #[serde(default)]
    pub show_cuts: bool,
    #[serde(default)]
    pub show_laser: bool,
}

impl UiSettings {
    pub fn with_defaults() -> Self {
        Self { in_window_menus: false, dock_layout: "cuts|laser,library,move,console".to_owned(), show_cuts: true, show_laser: true }
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl craft_settings::Settings for UiSettings {
    const APP_ID: &'static str = "openlaser";
    const SCHEMA_VERSION: u32 = 1;
}

/// Validates the compact panel layout representation and falls back atomically on malformed data.
pub fn normalize_dock_layout(value: &str) -> String {
    let valid_names = ["cuts", "laser", "library", "move", "console"];
    let mut seen = std::collections::BTreeSet::new();
    let mut sides = value.split('|');
    let left = sides.next().unwrap_or_default();
    let right = sides.next().unwrap_or_default();
    if sides.next().is_some() {
        return UiSettings::default().dock_layout;
    }
    for name in left.split(',').chain(right.split(',')) {
        if !valid_names.contains(&name) || !seen.insert(name) {
            return UiSettings::default().dock_layout;
        }
    }
    if seen.len() != valid_names.len() || !left.split(',').any(|name| name == "cuts") || !right.split(',').any(|name| name == "laser") {
        return UiSettings::default().dock_layout;
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_layout_roundtrip_and_fallback() {
        let settings = UiSettings::default();
        let encoded = serde_json::to_string(&settings).unwrap_or_default();
        let decoded: UiSettings = serde_json::from_str(&encoded).unwrap_or_default();
        assert_eq!(decoded, settings);
        assert_eq!(normalize_dock_layout("cuts|laser,library,move,console"), settings.dock_layout);
        assert_eq!(normalize_dock_layout("cuts|laser,laser"), settings.dock_layout);
        assert_eq!(normalize_dock_layout("bad"), settings.dock_layout);
    }
}

pub trait SettingsStore: Send {
    fn load(&self) -> UiSettings;
    fn save(&mut self, s: &UiSettings);
}

pub trait Clipboard: Send {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, text: &str);
}

pub trait AutomationToggle: Send {
    fn set(&mut self, on: bool) -> Result<Option<ControlInfo>, String>;
    fn info(&self) -> Option<ControlInfo>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlInfo {
    pub port: u16,
    pub clients: Vec<String>,
}

/// Implemented by openlaser-human (the app injects it). ui-egui never links openlaser-arm.
pub trait HumanSurfaces: Send {
    /// The only way control-channel input enters egui. Taints this frame, the next two, and everything until 1 s
    /// after the last injected event (UI §7.3), so a click whose press and release span frames, or whose press was
    /// synthetic, never counts. While a human-only surface is open, queued synthetic events are dropped.
    fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>);
    /// True if this frame carries automation input.
    fn frame_is_synthetic(&self, ctx: &egui::Context) -> bool;
    /// Gate for every human-only widget (Connect, Frame, Start, Approve, Device-settings Save, Allow Agent
    /// Connections): primary-button click, untainted frame, no AccessKit action this frame, focused window,
    /// widget visible ≥ 300 ms.
    fn is_human_click(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool;
    /// Draws the ARM confirmation as a Modal (STOP stays above). Mints inside; returns the token once.
    fn arm_dialog(&mut self, ctx: &egui::Context, model: &ArmModel) -> ArmOutcome;
    /// Any human-only surface open: the dispatcher refuses mutating control methods (human_confirmation_open).
    fn surface_open(&self) -> bool;
}

pub struct ArmModel {
    pub summary: EmissionSummary,
    pub receipt: FrameReceipt,
    pub checklist: Vec<String>,
    pub session: SessionId,
    pub now: MonoTime,
    pub thumbnail: DisplayList,
    /// S23, from the provider.
    pub emission_cap_pct: Option<f64>,
}

pub enum ArmOutcome {
    Open,
    Cancelled,
    Armed(ArmToken),
}

/// Test double: nothing is ever human, nothing ever arms. ui-egui's own kittests use it. Fail closed (§1.3 rule 7):
/// every frame counts as synthetic and a human-only surface counts as open, so injected input is dropped.
#[derive(Debug, Default)]
pub struct NoHuman;

impl HumanSurfaces for NoHuman {
    fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>) {
        let _ = (ctx, raw, events);
    }

    fn frame_is_synthetic(&self, ctx: &egui::Context) -> bool {
        let _ = ctx;
        true
    }

    fn is_human_click(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        let _ = (ctx, response);
        false
    }

    fn arm_dialog(&mut self, ctx: &egui::Context, model: &ArmModel) -> ArmOutcome {
        let _ = (ctx, model);
        ArmOutcome::Cancelled
    }

    fn surface_open(&self) -> bool {
        true
    }
}

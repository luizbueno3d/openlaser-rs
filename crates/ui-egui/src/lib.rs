//! The LightBurn-layout shell (SPEC §3.19; owned by WP8).
//!
//! Everything platform- or safety-specific is injected through [`Services`]; ui-egui does no file or device I/O
//! itself and can never reach `openlaser-arm` (§2.3 rule 7): ARM tokens come only through [`HumanSurfaces`].
//!
//! Layout (UI §4.1): `app` (state, STOP, Esc, close guard, provenance, eframe hooks), `panels` (menu bar, toolbar,
//! numeric edits, tools, palette, dock, Cuts / Layers, Laser panel, status bar, STOP overlay), `dialogs`, `view`
//! (the canvas), `control` (the §4.5 dispatcher), `menu_catalog`, `shortcuts`, `dock`, `parity`.
#![forbid(unsafe_code)]

mod app;
pub mod canvas;
pub mod canvas_xform;
pub mod control;
mod cut_editor;
mod dialogs;
mod dock;
pub mod i18n;
mod icon_data;
mod material_test;
pub mod menu_catalog;
pub mod node_edit;
mod panels;
pub mod parity;
mod screenshot;
mod services;
pub mod shortcuts;
mod view;

pub use app::{OpenLaserApp, UiState};
pub use services::{
    ArmModel, ArmOutcome, AutomationToggle, Clipboard, ControlInfo, DialogKind, DialogTicket, FileDialogs, HumanSurfaces, NoHuman, Services, SettingsStore,
    UiSettings,
};

// Re-exports so callers can name these without a direct dependency (SPEC §3.0).
pub use openlaser_control::server::ControlRequest;
pub use openlaser_control::wire::Reply;
pub use openlaser_engine::{PedalPress, Registry, Session};
pub use openlaser_render::DisplayList;
pub use openlaser_types::{ArmToken, EmissionSummary, FrameReceipt, MonoTime, SessionId};
pub use {eframe, egui, serde_json};

/// The always-available simulator profile used by the desktop bootstrap.
pub fn simulator_profile() -> openlaser_engine::MachineProfile {
    openlaser_machine::jpt_m7_200_sim()
}

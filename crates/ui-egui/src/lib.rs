//! The LightBurn-layout shell (SPEC §3.19; owned by WP8).
//!
//! Everything platform- or safety-specific is injected through [`Services`]; ui-egui does no file or device I/O
//! itself and can never reach `openlaser-arm` (§2.3 rule 7): ARM tokens come only through [`HumanSurfaces`].
//!
//! WP0 stubs: the traits and types are complete; the app draws a placeholder, Esc latches every stop, and the
//! control dispatcher refuses every method.
#![forbid(unsafe_code)]

mod app;
pub mod canvas;
pub mod control;
pub mod menu_catalog;
pub mod parity;
mod services;
pub mod shortcuts;

pub use app::{OpenLaserApp, UiState};
pub use services::{
    ArmModel, ArmOutcome, AutomationToggle, Clipboard, ControlInfo, DialogKind, DialogTicket, FileDialogs, HumanSurfaces, NoHuman, Services, SettingsStore,
    UiSettings,
};

// Re-exports so callers can name these without a direct dependency (SPEC §3.0).
pub use openlaser_control::server::ControlRequest;
pub use openlaser_control::wire::Reply;
pub use openlaser_engine::{Registry, Session};
pub use openlaser_render::DisplayList;
pub use openlaser_types::{ArmToken, EmissionSummary, FrameReceipt, MonoTime, SessionId};
pub use {eframe, egui};

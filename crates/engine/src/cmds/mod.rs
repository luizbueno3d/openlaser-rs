//! The command table (SPEC Appendix A): one module per LightBurn area. Each module's `specs()` lists its commands;
//! `all()` assembles them in menu order for the registry.
//!
//! Stretch rows of Appendix A (†: `file.export`, `laser.redLight`, `laserTools.materialTest`) are not registered in
//! this build; `menu_catalog.rs` (WP8) renders them disabled.

use openlaser_types::ObjectId;

use crate::command::{CommandSpec, Disabled, EngineError};
use crate::session::Session;

mod app;
mod arrange;
mod camera;
mod device;
mod edit;
mod file;
mod job;
mod laser;
mod layer;
mod path;
mod text;
mod tools;

pub(crate) fn all() -> Vec<CommandSpec> {
    let mut v = Vec::new();
    v.extend(file::specs());
    v.extend(edit::specs());
    v.extend(path::specs());
    v.extend(text::specs());
    v.extend(tools::specs());
    v.extend(arrange::specs());
    v.extend(layer::specs());
    v.extend(laser::specs());
    v.extend(device::specs());
    v.extend(job::specs());
    v.extend(camera::specs());
    v.extend(app::specs());
    v
}

// ---- enabled() helpers ------------------------------------------------------------------------------------------

pub(crate) fn always(_: &Session) -> Result<(), Disabled> {
    Ok(())
}

pub(crate) fn needs_doc(s: &Session) -> Result<(), Disabled> {
    if s.active().is_some() { Ok(()) } else { Err(Disabled::needs_document()) }
}

pub(crate) fn needs_window(s: &Session) -> Result<(), Disabled> {
    if s.has_window() { Ok(()) } else { Err(Disabled::new("needs bridge mode (the OpenLaser window)")) }
}

pub(crate) fn needs_doc_and_window(s: &Session) -> Result<(), Disabled> {
    needs_window(s)?;
    needs_doc(s)
}

// ---- shared helpers ---------------------------------------------------------------------------------------------

/// Defence in depth for human-only commands whose handler does not go through a typed API that checks the origin
/// itself (profile writes, detect): dispatch refused every other origin already; a `run` called directly, outside
/// dispatch, is `Headless` and refused here.
pub(crate) fn human_here(s: &Session, id: &str) -> Result<(), EngineError> {
    if s.current_origin() == crate::command::Origin::Human { Ok(()) } else { Err(EngineError::human_only(id)) }
}

/// The objects a command acts on: `ids` (each must be a top-level object) or the selection.
pub(crate) fn target_ids(s: &Session, id: &str, ids: Option<Vec<u64>>) -> Result<Vec<ObjectId>, EngineError> {
    let doc = s.doc()?;
    let out: Vec<ObjectId> = match ids {
        Some(list) => {
            let mut out = Vec::with_capacity(list.len());
            for raw in list {
                let oid = ObjectId(raw);
                if doc.project.shape(oid).is_none() {
                    return Err(EngineError::NoShape(oid));
                }
                if !out.contains(&oid) {
                    out.push(oid);
                }
            }
            out
        }
        None => doc.selection.ids.clone(),
    };
    if out.is_empty() {
        return Err(EngineError::disabled(id, "nothing is selected; select objects or pass `ids`"));
    }
    Ok(out)
}

/// `{"changed": n}`-style results.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Changed {
    pub changed: usize,
}

/// `{"ids": [...]}` for creators.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Created {
    pub ids: Vec<ObjectId>,
}

//! Cuts / Layers: the colour layers, tool layers T1/T2 and their cut settings.

use openlaser_types::{Deg, Hz, LayerKey, Mm, MmPerS, Ns, Percent, Us};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutTable {
    /// Exactly 30, index = position.
    pub colors: Vec<ColorLayer>,
    pub tools: [ToolLayer; 2],
    /// Cuts / Layers order = output order; a layer appears when first used.
    pub order: Vec<LayerKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorLayer {
    pub index: u8,
    pub name: String,
    pub output: bool,
    pub show: bool,
    /// 1..=11 sub-layers run left to right; ops[0] is the layer. UI for >1 is P1.
    pub ops: Vec<CutOp>,
    /// galvo: repeats the whole stack.
    pub global_passes: u16,
    pub image: ImageCut,
    /// gantry only.
    pub air_assist: bool,
}

impl CutTable {
    /// Every pass count is at least 1. A `passes` of 0 would plan no sections for the layer's shapes — the
    /// planner must never drop them silently — so `format` refuses it on load and save and the planner refuses
    /// it through the same check ([`crate::Error::ZeroPasses`]).
    pub fn check_passes(&self) -> Result<(), crate::Error> {
        for layer in &self.colors {
            let key = LayerKey::Color(layer.index);
            for (op, sub) in layer.ops.iter().zip(0usize..) {
                if op.passes == 0 {
                    return Err(crate::Error::ZeroPasses { layer: key, field: format!("ops[{sub}].passes") });
                }
            }
            if layer.global_passes == 0 {
                return Err(crate::Error::ZeroPasses { layer: key, field: "globalPasses".to_owned() });
            }
            if layer.image.passes == 0 {
                return Err(crate::Error::ZeroPasses { layer: key, field: "image.passes".to_owned() });
            }
        }
        Ok(())
    }
}

/// Never outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLayer {
    pub which: u8,
    pub name: String,
    pub show: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutOp {
    pub name: String,
    pub enabled: bool,
    pub mode: CutMode,
    pub passes: u16,
    pub galvo: GalvoParams,
    pub gantry: GantryParams,
    pub fill: FillParams,
}

/// OffsetFill is P1: the planner refuses it (UnsupportedMode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutMode {
    Line,
    Fill,
    OffsetFill,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalvoParams {
    pub speed: MmPerS,
    pub power: Percent,
    pub min_power: Percent,
    pub frequency: Hz,
    pub q_pulse: Option<Ns>,
    /// P1 behaviour; refused if set in this build (UnsupportedMode).
    pub timings: Option<GalvoTimings>,
    /// P1 behaviour; refused if set in this build.
    pub wobble: Option<Wobble>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalvoTimings {
    pub laser_on_tc: Us,
    pub laser_off_tc: Us,
    pub end_tc: Us,
    pub polygon_tc: Us,
    pub jump_speed: MmPerS,
    pub min_jump_delay: Us,
    pub max_jump_delay: Us,
    pub jump_distance_limit: Mm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wobble {
    pub step: Mm,
    pub size: Mm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GantryParams {
    pub speed: MmPerS,
    pub power_max: Percent,
    pub power_min: Percent,
    pub constant_power: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillParams {
    pub interval: Mm,
    pub angle: Deg,
    pub angle_increment: Deg,
    pub bidirectional: bool,
    pub cross_hatch: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCut {
    pub mode: ImageMode,
    pub interval: Mm,
    pub angle: Deg,
    pub bidirectional: bool,
    pub negative: bool,
    pub threshold: u8,
    pub passes: u16,
    pub galvo: GalvoParams,
    pub gantry: GantryParams,
}

/// On/off image dithering (S16); every run uses the same cut power.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    Threshold,
    Jarvis,
    FloydSteinberg,
    Stucki,
    Atkinson,
    Ordered,
}

/// Defaults for a new colour layer, per device kind (machine profiles hold one per kind; B11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDefaults {
    pub galvo: GalvoParams,
    pub gantry: GantryParams,
    pub fill: FillParams,
}

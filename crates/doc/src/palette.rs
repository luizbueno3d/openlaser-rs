//! LightBurn's layer palette. Values from LightBurn's public palette documentation (interoperability: SVG colours
//! land on the same layers); provenance recorded in crates/doc/PROVENANCE.md by WP1.
//!
//! WP0 stub: every colour is black until WP1 records the documented values and their provenance.

use openlaser_types::Rgb;

pub const COLOR_LAYERS: usize = 30;

/// Display colours of palette 00..=29.
pub const PALETTE: [Rgb; 30] = [Rgb([0, 0, 0]); 30];
/// Display colours of T1, T2.
pub const TOOL_COLORS: [Rgb; 2] = [Rgb([0, 0, 0]); 2];

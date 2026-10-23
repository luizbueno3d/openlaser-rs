//! R-tree over polyline segments for canvas picking (world mm). WP0 stub: an empty index.

use openlaser_types::ids::ObjectId;

use crate::{Bounds, P2, Polyline};

/// R-tree over polyline segments for canvas picking (world mm).
#[derive(Debug, Default)]
pub struct HitIndex {
    /// (object, segment); WP1 replaces this with an `rstar::RTree`.
    segments: Vec<(ObjectId, [P2; 2])>,
}

impl HitIndex {
    pub fn build<'a>(items: impl IntoIterator<Item = (ObjectId, &'a [Polyline])>) -> Self {
        let _ = items.into_iter().count();
        HitIndex { segments: Vec::new() }
    }

    /// Nearest segment within tol.
    pub fn pick(&self, at: P2, tol_mm: f64) -> Option<ObjectId> {
        let _ = (at, tol_mm, self.segments.len());
        None
    }

    /// crossing=false: fully enclosed.
    pub fn in_rect(&self, r: Bounds, crossing: bool) -> Vec<ObjectId> {
        let _ = (r, crossing);
        Vec::new()
    }
}

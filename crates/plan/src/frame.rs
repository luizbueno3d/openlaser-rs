//! Frame paths (SPEC §3.7, PLAN §3.6 layer 3: frame ⊇ burn).
//!
//! Contour traces every outline of every shape on an emitting layer (Fill layers trace their outlines, never the
//! hatch); Bounds traces one closed rectangle around the same outlines. Hull is P1 and refused.

use openlaser_geom::{Bounds, Polyline, bounds_of};
use openlaser_types::FrameStyle;

/// The `facts.unsupported` reason the planner records for a Hull frame; `safety::validate_frame` refuses it too.
pub const HULL_FRAME_UNSUPPORTED: &str = "Hull framing is not in this build; choose Bounds or Contour";

/// The closed rectangle `min → (max.x, min.y) → max → (min.x, max.y) → min`.
pub fn bounds_rect(b: &Bounds) -> Polyline {
    Polyline(vec![b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]], b.min])
}

/// Applies a frame style to outlines (machine mm). Hull yields nothing: the planner records it as unsupported.
pub fn styled(outlines: Vec<Polyline>, style: FrameStyle) -> Vec<Polyline> {
    match style {
        FrameStyle::Contour => outlines,
        FrameStyle::Bounds => bounds_of(&outlines).map(|b| vec![bounds_rect(&b)]).unwrap_or_default(),
        FrameStyle::Hull => Vec::new(),
    }
}

/// The style a planned frame was built with: Bounds when it is exactly one rectangle as [`bounds_rect`] draws it,
/// else Contour. A Contour frame of a single rectangle drawn the same way traces the same path, so either answer
/// describes what the light does. (`JobPlan` does not carry the style; see the hand-off's contract request.)
pub fn frame_style_of(frame: &[Polyline]) -> FrameStyle {
    match frame {
        [only] => match only.bounds() {
            Some(b) if *only == bounds_rect(&b) => FrameStyle::Bounds,
            _ => FrameStyle::Contour,
        },
        _ => FrameStyle::Contour,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Polyline {
        Polyline(vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0]])
    }

    #[test]
    fn bounds_style_is_one_closed_rectangle() {
        let tri = Polyline(vec![[0.0, 0.0], [4.0, 1.0], [2.0, 3.0], [0.0, 0.0]]);
        let f = styled(vec![tri.clone(), Polyline(vec![[-2.0, -1.0], [-1.0, -1.0]])], FrameStyle::Bounds);
        assert_eq!(f, vec![Polyline(vec![[-2.0, -1.0], [4.0, -1.0], [4.0, 3.0], [-2.0, 3.0], [-2.0, -1.0]])]);
        assert_eq!(frame_style_of(&f), FrameStyle::Bounds);
        assert_eq!(frame_style_of(std::slice::from_ref(&tri)), FrameStyle::Contour);
        assert_eq!(styled(vec![tri], FrameStyle::Hull), Vec::<Polyline>::new());
        assert!(styled(Vec::new(), FrameStyle::Bounds).is_empty());
    }

    #[test]
    fn contour_keeps_outlines_and_style_is_recognised() {
        let f = styled(vec![square(), square()], FrameStyle::Contour);
        assert_eq!(f.len(), 2);
        assert_eq!(frame_style_of(&f), FrameStyle::Contour);
        assert_eq!(frame_style_of(&[]), FrameStyle::Contour);
    }
}

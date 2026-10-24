//! The pathfinder's only use of polygon clipping (`PZClipper`), kept behind
//! one small module. Backed by `pz-clipper6`, a port of the Clipper 6 /
//! `PZClipper` code statically linked into the native library.

use pz_clipper6::pz::from_fixed;
use pz_clipper6::{JoinType, PZClipper};

/// A clipper output polygon in world units: outer ring and holes, points
/// in Clipper's output order.
pub(crate) struct ClipPolygon {
    pub outer: Vec<(f32, f32)>,
    pub holes: Vec<Vec<(f32, f32)>>,
}

/// `PZClipper` as used by `VisibilityGraph::addWorldObstaclesClipper`.
pub(crate) struct Clip {
    inner: PZClipper,
}

impl Default for Clip {
    fn default() -> Self {
        Clip {
            inner: PZClipper::new(),
        }
    }
}

impl Clip {
    /// `PZClipper::clear`.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// `PZClipper::addAABB`.
    pub fn add_aabb(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        self.inner.add_aabb(x1, y1, x2, y2);
    }

    /// `PZClipper::generatePolygons(0.0, jtMiter)` followed by reading the
    /// polygon list and `clearPolys`.
    pub fn generate_polygons(&mut self) -> Vec<ClipPolygon> {
        self.inner.generate_polygons(0.0, JoinType::Miter);
        let ring = |p: &pz_clipper6::Path| -> Vec<(f32, f32)> {
            p.iter()
                .map(|pt| (from_fixed(pt.x), from_fixed(pt.y)))
                .collect()
        };
        let out = self
            .inner
            .polys()
            .iter()
            .map(|p| ClipPolygon {
                outer: ring(&p.outer),
                holes: p.holes.iter().map(ring).collect(),
            })
            .collect();
        self.inner.clear_polys();
        out
    }
}

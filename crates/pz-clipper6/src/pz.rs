//! `PZClipper`: the game's own wrapper around ClipperLib inside the native
//! pathfinding library (obstacle / walkable polygon generation for the
//! visibility graph). Original code, reconstructed from the binary's
//! behaviour.
//!
//! Coordinates enter as `f32` world units and are stored as fixed point
//! with three decimals: `(int32_t)floorf(fmaf(v, 1000.0f, 0.5f))`. Results
//! stay in fixed point ([`PZPoly`]); only [`PZClipper::triangulate`]
//! converts back, as `(float)x / 1000.0f`.

use crate::clipper::{
    ClipType, Clipper, ClipperException, ClipperOffset, EndType, IntPoint, JoinType, Path, Paths,
    PolyFillType, PolyNodeId, PolyTree, PolyType,
};
use crate::earcut;

/// The exception `generateObstaclePolygons` / `generateWalkablePolygons`
/// throw when `Clipper::Execute` fails.
pub const EXECUTE_FAILED: ClipperException = ClipperException("Clipper.Execute()");

/// World coordinate to fixed point, as compiled (`fmadd` + `fcvtms`):
/// floor of `v * 1000 + 0.5` with a single rounding, saturated to `i32`.
#[inline]
pub fn to_fixed(v: f32) -> i64 {
    v.mul_add(1000.0, 0.5).floor() as i32 as i64
}

/// Fixed point back to world units, as compiled (`scvtf` + `fdiv`).
#[inline]
pub fn from_fixed(v: i64) -> f32 {
    v as f32 / 1000.0
}

/// `PZPoly`: one output polygon, an outer ring and its holes, in fixed
/// point (world units * 1000). Rings keep ClipperLib's output orientation
/// and point order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PZPoly {
    /// The outer contour.
    pub outer: Path,
    /// Hole contours (direct children of the outer node in the tree).
    pub holes: Paths,
}

/// `Triangle2f`: three vertices in world units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Triangle2f {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// `PZClipper` (derives from `ClipperLib::Clipper` in the original).
#[derive(Clone, Debug)]
pub struct PZClipper {
    clipper: Clipper,
    path: Path,
    tree: PolyTree,
    polys: Vec<PZPoly>,
    points: Path,
    count: i32,
}

impl Default for PZClipper {
    fn default() -> Self {
        Self::new()
    }
}

impl PZClipper {
    /// `PZClipper::PZClipper()`.
    pub fn new() -> Self {
        Self {
            clipper: Clipper::new(0),
            path: Path::new(),
            tree: PolyTree::new(),
            polys: Vec::new(),
            points: Path::new(),
            count: 0,
        }
    }

    /// `PZClipper::clear()`: drops all added paths (`ClipperBase::Clear`).
    pub fn clear(&mut self) {
        self.clipper.clear();
        self.count = 0;
    }

    /// `PZClipper::addAABB(float, float, float, float)`: adds the rectangle
    /// (x1,y1)-(x2,y2) as a closed subject path.
    pub fn add_aabb(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let (ix1, iy1) = (to_fixed(x1), to_fixed(y1));
        let (ix2, iy2) = (to_fixed(x2), to_fixed(y2));
        self.path.clear();
        self.path.push(IntPoint::new(ix1, iy1));
        self.path.push(IntPoint::new(ix2, iy1));
        self.path.push(IntPoint::new(ix2, iy2));
        self.path.push(IntPoint::new(ix1, iy2));
        // i32-range coordinates can never fail the range test.
        let _ = self.clipper.add_path(&self.path, PolyType::Subject, true);
        self.count += 1;
    }

    /// `PZClipper::addAABBBevel(float, float, float, float, float)`: the
    /// native body is empty, so this does nothing (not even count a path).
    pub fn add_aabb_bevel(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _bevel: f32) {}

    /// `PZClipper::addPolygon(EdgeRing2f const&)`: adds the ring's points
    /// (x, y in world units) as a closed subject path.
    pub fn add_polygon<I>(&mut self, ring: I)
    where
        I: IntoIterator<Item = (f32, f32)>,
    {
        self.path.clear();
        for (x, y) in ring {
            self.path.push(IntPoint::new(to_fixed(x), to_fixed(y)));
        }
        let _ = self.clipper.add_path(&self.path, PolyType::Subject, true);
        self.count += 1;
    }

    /// `PZClipper::clearPolys()`: clears the result tree and polygon list.
    pub fn clear_polys(&mut self) {
        self.tree.clear();
        self.polys.clear();
    }

    /// `PZClipper::generatePolygons(double, ClipperLib::JoinType)`: union
    /// (positive fill) of all added paths, then [`Self::get_polygons_offset`].
    /// A failed `Execute` is ignored, as in the original.
    pub fn generate_polygons(&mut self, delta: f64, join_type: JoinType) {
        self.clear_polys();
        self.clipper.execute_tree(
            ClipType::Union,
            &mut self.tree,
            PolyFillType::Positive,
            PolyFillType::Positive,
        );
        self.get_polygons_offset(delta, join_type);
    }

    /// `PZClipper::getPolygons(double, ClipperLib::JoinType)`: when
    /// `delta != 0`, offsets every node of the result tree by `delta`
    /// world units (`ClipperOffset(2.0, 0.25)`, closed polygons, open
    /// paths butt-ended) and replaces the tree with the offset result;
    /// then [`Self::get_polygons`].
    pub fn get_polygons_offset(&mut self, delta: f64, join_type: JoinType) {
        if delta != 0.0 {
            let mut co = ClipperOffset::new(2.0, 0.25);
            let mut node = self.tree.get_first();
            while let Some(n) = node {
                let end_type = if self.tree.is_open(n) {
                    EndType::OpenButt
                } else {
                    EndType::ClosedPolygon
                };
                co.add_path(self.tree.contour(n), join_type, end_type);
                node = self.tree.get_next(n);
            }
            self.tree.clear();
            // Only unreachable coordinate magnitudes can make this throw.
            let _ = co.execute_tree(&mut self.tree, delta * 1000.0);
        }
        self.get_polygons();
    }

    /// `PZClipper::generateObstaclePolygons()`: union (positive fill) of all
    /// added paths into the polygon list; does nothing if no path was
    /// added. `Err(EXECUTE_FAILED)` where the original throws.
    pub fn generate_obstacle_polygons(&mut self) -> Result<(), ClipperException> {
        self.clear_polys();
        if self.count != 0 {
            if !self.clipper.execute_tree(
                ClipType::Union,
                &mut self.tree,
                PolyFillType::Positive,
                PolyFillType::Positive,
            ) {
                return Err(EXECUTE_FAILED);
            }
            self.get_polygons();
        }
        Ok(())
    }

    /// `PZClipper::getPolygons()`: appends the result tree to the polygon
    /// list, one [`PZPoly`] per outer node (depth-first order) with its
    /// direct hole children.
    pub fn get_polygons(&mut self) {
        // std::map<PolyNode*, PZPoly*>
        let mut map: Vec<(PolyNodeId, usize)> = Vec::new();
        let mut node = self.tree.get_first();
        while let Some(n) = node {
            if self.tree.is_hole(n) {
                let parent = self.tree.parent(n).expect("hole without parent");
                let pi = map
                    .iter()
                    .find(|(k, _)| *k == parent)
                    .map(|&(_, v)| v)
                    .expect("hole whose outer polygon was not visited");
                let contour = self.tree.contour(n).clone();
                self.polys[pi].holes.push(contour);
            } else {
                self.polys.push(PZPoly {
                    outer: self.tree.contour(n).clone(),
                    holes: Paths::new(),
                });
                map.push((n, self.polys.len() - 1));
            }
            node = self.tree.get_next(n);
        }
    }

    /// `PZClipper::generateWalkablePolygons(int, int)`: the 8x8 chunk at
    /// (`chunk_x`, `chunk_y`) minus (even-odd) the current result tree,
    /// into the polygon list. Clears all added paths first; the tree is
    /// normally the output of [`Self::generate_obstacle_polygons`].
    /// `Err` where the original throws (failed `Execute`, or an open path
    /// in the tree).
    pub fn generate_walkable_polygons(
        &mut self,
        chunk_x: i32,
        chunk_y: i32,
    ) -> Result<(), ClipperException> {
        self.clipper.clear();
        self.count = 0;
        self.add_aabb(
            chunk_x.wrapping_shl(3) as f32,
            chunk_y.wrapping_shl(3) as f32,
            (chunk_x as f32 + 1.0) * 8.0,
            (chunk_y as f32 + 1.0) * 8.0,
        );
        let mut node = self.tree.get_first();
        while let Some(n) = node {
            let closed = !self.tree.is_open(n);
            self.clipper
                .add_path(self.tree.contour(n), PolyType::Clip, closed)?;
            node = self.tree.get_next(n);
        }
        self.clear_polys();
        if !self.clipper.execute_tree(
            ClipType::Difference,
            &mut self.tree,
            PolyFillType::EvenOdd,
            PolyFillType::EvenOdd,
        ) {
            return Err(EXECUTE_FAILED);
        }
        self.get_polygons();
        Ok(())
    }

    /// `PZClipper::triangulate(int, std::vector<Triangle2f>&)`: earcut of
    /// polygon `index` (outer ring + holes), appending triangles in world
    /// units to `out` (which is not cleared).
    ///
    /// # Panics
    /// If `index` is out of range (undefined behaviour in the original).
    pub fn triangulate(&mut self, index: i32, out: &mut Vec<Triangle2f>) {
        let poly = &self.polys[index as usize];
        let mut rings: Vec<Path> = Vec::with_capacity(1 + poly.holes.len());
        rings.push(poly.outer.clone());
        rings.extend(poly.holes.iter().cloned());
        self.points.clear();
        for r in &rings {
            self.points.extend_from_slice(r);
        }
        let indices = earcut::earcut(&rings);
        let p = &self.points;
        let mut i = 0;
        while i < indices.len() {
            let (a, b, c) = (
                p[indices[i] as usize],
                p[indices[i + 1] as usize],
                p[indices[i + 2] as usize],
            );
            out.push(Triangle2f {
                x0: from_fixed(a.x),
                y0: from_fixed(a.y),
                x1: from_fixed(b.x),
                y1: from_fixed(b.y),
                x2: from_fixed(c.x),
                y2: from_fixed(c.y),
            });
            i += 3;
        }
    }

    /// The polygon list (`PZClipper::polys`, a `std::vector<PZPoly*>`) that
    /// `VisibilityGraph::addWorldObstaclesClipper` reads after
    /// `generatePolygons`.
    pub fn polys(&self) -> &[PZPoly] {
        &self.polys
    }

    /// Mutable access to the polygon list.
    pub fn polys_mut(&mut self) -> &mut Vec<PZPoly> {
        &mut self.polys
    }

    /// The result tree (`PZClipper`'s `ClipperLib::PolyTree` member).
    pub fn tree(&self) -> &PolyTree {
        &self.tree
    }

    /// Number of paths added since the last clear (`addAABB` /
    /// `addPolygon` calls).
    pub fn count(&self) -> i32 {
        self.count
    }

    /// The underlying `ClipperLib::Clipper`.
    pub fn clipper(&self) -> &Clipper {
        &self.clipper
    }

    /// The underlying `ClipperLib::Clipper`, mutably.
    pub fn clipper_mut(&mut self) -> &mut Clipper {
        &mut self.clipper
    }
}

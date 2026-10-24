//! The game's own JNI layer (`libPZClipper`), rebuilt from the behaviour of
//! the shipped binary: the native halves of `zombie.vehicles.Clipper` and
//! `zombie.vehicles.ClipperOffset`.
//!
//! Coordinates cross the boundary as `f32` world units. On the way in each
//! value becomes `(int32_t)floorf(fmaf(v, 1000.0f, 0.5f))` (fused on arm64);
//! on the way out each `int64` coordinate becomes `(float)v / 1000.0f`.
//!
//! The Java side passes `java.nio.ByteBuffer`s. The native code always reads
//! and writes from byte 0 (ignoring and never moving the Java position),
//! big-endian, bounded by the buffer's capacity. The `*_into` methods
//! reproduce that byte layout exactly; the other methods return the same data
//! as plain Rust values.

use crate::core::{Path64, Paths64, Point64};
use crate::earcut::earcut;
use crate::engine::{ClipType, Clipper64, PathType, PolyPath64};
use crate::offset::{ClipperOffset as Offset64, EndType, JoinType};
use crate::FillRule;

/// What the native code turns into a Java exception (after which the Java
/// caller never sees the native return value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipperError {
    /// `java.nio.BufferUnderflowException`: `addPath` read past the buffer.
    BufferUnderflow,
    /// `java.nio.BufferOverflowException`: output did not fit the buffer
    /// (bytes written before the overflow stay in the buffer).
    BufferOverflow,
    /// `std::out_of_range` from `std::vector::at` (bad polygon index),
    /// rethrown as `java.lang.Error("vector")`.
    IndexOutOfRange,
    /// `std::length_error` (negative point count), rethrown as
    /// `java.lang.Error("vector")`.
    LengthError,
}

impl std::fmt::Display for ClipperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipperError::BufferUnderflow => f.write_str("buffer underflow"),
            ClipperError::BufferOverflow => f.write_str("buffer overflow"),
            ClipperError::IndexOutOfRange => f.write_str("polygon index out of range"),
            ClipperError::LengthError => f.write_str("negative point count"),
        }
    }
}

impl std::error::Error for ClipperError {}

/// Converts a world coordinate to the library's integer space.
#[inline]
pub fn to_fixed(v: f32) -> i64 {
    (v.mul_add(1000.0, 0.5).floor() as i32) as i64
}

/// Converts an integer coordinate back to world units.
#[inline]
pub fn to_float(v: i64) -> f32 {
    (v as f32) / 1000.0
}

/// One output polygon (`JNIPolygon`): an outer ring and its direct holes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Polygon {
    pub points: Path64,
    pub holes: Paths64,
}

impl Polygon {
    /// Outer ring as flat `x, y` world floats.
    pub fn points_f32(&self) -> Vec<f32> {
        flatten(&self.points)
    }

    /// Holes as flat `x, y` world floats.
    pub fn holes_f32(&self) -> Vec<Vec<f32>> {
        self.holes.iter().map(|h| flatten(h)).collect()
    }
}

fn flatten(path: &[Point64]) -> Vec<f32> {
    let mut out = Vec::with_capacity(path.len() * 2);
    for p in path {
        out.push(to_float(p.x));
        out.push(to_float(p.y));
    }
    out
}

/// Reads `num_points` float pairs from the start of `buf` (the buffer's
/// whole capacity, as the native code sees it).
fn read_points(num_points: i32, buf: &[f32]) -> Result<Path64, ClipperError> {
    let mut path = Path64::new();
    if num_points == 0 {
        return Ok(path);
    }
    if num_points < 0 {
        return Err(ClipperError::LengthError);
    }
    let mut pos = 0usize;
    for _ in 0..num_points {
        if pos + 2 > buf.len() {
            return Err(ClipperError::BufferUnderflow);
        }
        let x = buf[pos];
        let y = buf[pos + 1];
        pos += 2;
        path.push(Point64::new(to_fixed(x), to_fixed(y)));
    }
    Ok(path)
}

/// Big-endian writer over a byte slice, failing like the native
/// `ByteBuffer::put*` helpers once the capacity is exceeded.
struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Writer<'_> {
    fn put_short(&mut self, v: u16) -> Result<(), ClipperError> {
        if self.pos + 2 > self.buf.len() {
            return Err(ClipperError::BufferOverflow);
        }
        self.buf[self.pos..self.pos + 2].copy_from_slice(&v.to_be_bytes());
        self.pos += 2;
        Ok(())
    }
    fn put_float(&mut self, v: f32) -> Result<(), ClipperError> {
        if self.pos + 4 > self.buf.len() {
            return Err(ClipperError::BufferOverflow);
        }
        self.buf[self.pos..self.pos + 4].copy_from_slice(&v.to_bits().to_be_bytes());
        self.pos += 4;
        Ok(())
    }
    fn put_path(&mut self, path: &[Point64]) -> Result<(), ClipperError> {
        self.put_short(path.len() as u16)?;
        for p in path {
            self.put_float(to_float(p.x))?;
            self.put_float(to_float(p.y))?;
        }
        Ok(())
    }
}

/// `JNIClipper::getPolygons(const PolyPath64&, JNIPolygon*)`.
fn collect_polygon(node: &PolyPath64, polys: &mut Vec<Polygon>, idx: usize) {
    polys[idx].points = node.polygon.clone();
    for hole in &node.childs {
        polys[idx].holes.push(hole.polygon.clone());
        for island in &hole.childs {
            let ni = polys.len();
            polys.push(Polygon::default());
            collect_polygon(island, polys, ni);
        }
    }
}

fn collect_tree(tree: &PolyPath64, polys: &mut Vec<Polygon>) {
    for child in &tree.childs {
        let ni = polys.len();
        polys.push(Polygon::default());
        collect_polygon(child, polys, ni);
    }
}

fn add_tree_to_offset(off: &mut Offset64, jt: JoinType, node: &PolyPath64) {
    off.add_path(&node.polygon, jt, EndType::Polygon);
    for c in &node.childs {
        add_tree_to_offset(off, jt, c);
    }
}

/// Triangulates `poly` (outer ring then holes) with earcut; returns the
/// concatenated vertices and the triangle indices into them.
fn triangulate_polygon(poly: &Polygon) -> (Path64, Vec<u32>) {
    let mut rings: Vec<Path64> = Vec::with_capacity(1 + poly.holes.len());
    rings.push(poly.points.clone());
    rings.extend(poly.holes.iter().cloned());
    let vertices: Path64 = rings.iter().flatten().copied().collect();
    let indices = earcut(&rings);
    (vertices, indices)
}

/// `zombie.vehicles.Clipper`.
pub struct Clipper {
    clipper: Clipper64,
    tree: PolyPath64,
    open_paths: Paths64,
    polygons: Vec<Polygon>,
}

impl Default for Clipper {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipper {
    pub const CT_NO_CLIP: i32 = 0;
    pub const CT_INTERSECTION: i32 = 1;
    pub const CT_UNION: i32 = 2;
    pub const CT_DIFFERENCE: i32 = 3;
    pub const CT_XOR: i32 = 4;
    pub const JT_SQUARE: i32 = 0;
    pub const JT_BEVEL: i32 = 1;
    pub const JT_ROUND: i32 = 2;
    pub const JT_MITER: i32 = 3;

    /// `new Clipper()` (`newInstance`).
    pub fn new() -> Self {
        Clipper {
            clipper: Clipper64::new(),
            tree: PolyPath64::default(),
            open_paths: Paths64::new(),
            polygons: Vec::new(),
        }
    }

    /// `clear()`: drops all added paths (results stay until regenerated).
    pub fn clear(&mut self) {
        self.clipper.clear();
    }

    /// `addPath(numPoints, points, bClip, bClosed)`. `points` is the buffer
    /// content from index 0 as floats (`x0, y0, x1, y1, ...`). A clip path
    /// is always added closed; a subject path is open when `closed` is
    /// false. `addPath(n, bb, bClip)` is `add_path(n, bb, bClip, true)`.
    pub fn add_path(
        &mut self,
        num_points: i32,
        points: &[f32],
        clip: bool,
        closed: bool,
    ) -> Result<(), ClipperError> {
        let path = read_points(num_points, points)?;
        if clip {
            self.clipper.add_paths(&[path], PathType::Clip, false);
        } else if closed {
            self.clipper.add_paths(&[path], PathType::Subject, false);
        } else {
            self.clipper.add_paths(&[path], PathType::Subject, true);
        }
        Ok(())
    }

    /// `addLine(x1, y1, x2, y2)`: an open subject path from `(x2, y2)` to
    /// `(x1, y1)`.
    pub fn add_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let path = vec![
            Point64::new(to_fixed(x2), to_fixed(y2)),
            Point64::new(to_fixed(x1), to_fixed(y1)),
        ];
        self.clipper.add_paths(&[path], PathType::Subject, true);
    }

    fn aabb(x1: f32, y1: f32, x2: f32, y2: f32) -> Path64 {
        let (fx1, fy1, fx2, fy2) = (to_fixed(x1), to_fixed(y1), to_fixed(x2), to_fixed(y2));
        vec![
            Point64::new(fx1, fy1),
            Point64::new(fx2, fy1),
            Point64::new(fx2, fy2),
            Point64::new(fx1, fy2),
        ]
    }

    /// `addAABB(x1, y1, x2, y2)`: a closed subject rectangle.
    pub fn add_aabb(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let path = Self::aabb(x1, y1, x2, y2);
        self.clipper.add_paths(&[path], PathType::Subject, false);
    }

    /// `addAABBBevel(x1, y1, x2, y2, radius)`: an octagon (closed subject).
    pub fn add_aabb_bevel(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) {
        let pts = [
            x1 + radius,
            y1,
            x2 - radius,
            y1,
            x2,
            y1 + radius,
            x2,
            y2 - radius,
            x2 - radius,
            y2,
            x1 + radius,
            y2,
            x1,
            y2 - radius,
            x1,
            y1 + radius,
        ];
        // Java: addPath(8, bb, false) -> closed subject.
        let _ = self.add_path(8, &pts, false, true);
    }

    /// `addPolygon(x1, y1, ..., x4, y4)`: a closed subject quad.
    #[allow(clippy::too_many_arguments)]
    pub fn add_polygon(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x3: f32,
        y3: f32,
        x4: f32,
        y4: f32,
    ) {
        let path = vec![
            Point64::new(to_fixed(x1), to_fixed(y1)),
            Point64::new(to_fixed(x2), to_fixed(y2)),
            Point64::new(to_fixed(x3), to_fixed(y3)),
            Point64::new(to_fixed(x4), to_fixed(y4)),
        ];
        self.clipper.add_paths(&[path], PathType::Subject, false);
    }

    /// `clipAABB(x1, y1, x2, y2)`: a closed clip rectangle.
    pub fn clip_aabb(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let path = Self::aabb(x1, y1, x2, y2);
        self.clipper.add_paths(&[path], PathType::Clip, false);
    }

    fn reset_results(&mut self) {
        self.tree.childs.clear();
        self.polygons.clear();
    }

    /// `generatePolygons()` = `generatePolygons(ctDifference, 0.0, jtSquare)`.
    pub fn generate_polygons(&mut self) -> i32 {
        self.generate_polygons_full(Self::CT_DIFFERENCE, 0.0, Self::JT_SQUARE)
    }

    /// `generatePolygons(delta)`.
    pub fn generate_polygons_delta(&mut self, delta: f64) -> i32 {
        self.generate_polygons_full(Self::CT_DIFFERENCE, delta, Self::JT_SQUARE)
    }

    /// `generatePolygons(delta, joinType)`.
    pub fn generate_polygons_delta_join(&mut self, delta: f64, join_type: i32) -> i32 {
        self.generate_polygons_full(Self::CT_DIFFERENCE, delta, join_type)
    }

    /// `generatePolygons(clipType, delta, joinType)`: runs the boolean
    /// operation (fill rule `Positive`), optionally offsets the result by
    /// `delta` world units, and returns the number of output polygons, or
    /// -1 when the sweep failed.
    pub fn generate_polygons_full(&mut self, clip_type: i32, delta: f64, join_type: i32) -> i32 {
        self.reset_results();
        match self
            .clipper
            .execute_tree(ClipType::from_i32(clip_type), FillRule::Positive)
        {
            Some((tree, open)) => {
                self.tree = tree;
                self.open_paths = open;
            }
            None => return -1,
        }
        self.build_polygons(delta, JoinType::from_i32(join_type));
        self.polygons.len() as i32
    }

    /// `JNIClipper::getPolygons(double, JoinType)`.
    fn build_polygons(&mut self, delta: f64, jt: JoinType) {
        if delta != 0.0 {
            let mut off = Offset64::default();
            for child in &self.tree.childs {
                add_tree_to_offset(&mut off, jt, child);
            }
            for path in &self.open_paths {
                off.add_path(path, jt, EndType::Butt);
            }
            self.tree = off.execute_tree(delta * 1000.0);
        }
        collect_tree(&self.tree, &mut self.polygons);
        // Open paths are always appended (they are not cleared by the offset).
        for path in &self.open_paths {
            self.polygons.push(Polygon {
                points: path.clone(),
                holes: Paths64::new(),
            });
        }
    }

    /// The polygons produced by the last `generate*` call.
    pub fn polygons(&self) -> &[Polygon] {
        &self.polygons
    }

    fn polygon_at(&self, index: i32) -> Result<&Polygon, ClipperError> {
        if index < 0 {
            return Err(ClipperError::IndexOutOfRange);
        }
        self.polygons
            .get(index as usize)
            .ok_or(ClipperError::IndexOutOfRange)
    }

    /// `getPolygon(index)` as data.
    pub fn get_polygon(&self, index: i32) -> Result<&Polygon, ClipperError> {
        self.polygon_at(index)
    }

    /// `getPolygon(index, bb)`: writes `short n, n * (float x, float y),
    /// short holeCount, holeCount * (short m, m * (x, y))`; returns 0.
    pub fn get_polygon_into(&self, index: i32, buf: &mut [u8]) -> Result<i32, ClipperError> {
        let poly = self.polygon_at(index)?;
        let mut w = Writer { buf, pos: 0 };
        w.put_path(&poly.points)?;
        w.put_short(poly.holes.len() as u16)?;
        for h in &poly.holes {
            w.put_path(h)?;
        }
        Ok(0)
    }

    /// `generateTriangulatePolygons(cellX, cellY)`: subtracts the outer
    /// rings of the current result from the 8x8 square at
    /// `(cellX * 8, cellY * 8)` (fill rule `EvenOdd`, all paths are cleared
    /// first). Returns the polygon count, or -1 on failure.
    pub fn generate_triangulate_polygons(&mut self, cx: i32, cy: i32) -> i32 {
        self.clipper.clear();
        let x = cx.wrapping_mul(8000);
        let y = cy.wrapping_mul(8000);
        let rect = vec![
            Point64::new(x as i64, y as i64),
            Point64::new(x.wrapping_add(8000) as i64, y as i64),
            Point64::new(x.wrapping_add(8000) as i64, y.wrapping_add(8000) as i64),
            Point64::new(x as i64, y.wrapping_add(8000) as i64),
        ];
        self.clipper.add_paths(&[rect], PathType::Subject, false);
        let outers: Vec<Path64> = self.tree.childs.iter().map(|c| c.polygon.clone()).collect();
        for p in outers {
            self.clipper.add_paths(&[p], PathType::Clip, false);
        }
        self.reset_results();
        match self
            .clipper
            .execute_tree(ClipType::Difference, FillRule::EvenOdd)
        {
            Some((tree, open)) => {
                self.tree = tree;
                self.open_paths = open;
            }
            None => return -1,
        }
        self.build_polygons(0.0, JoinType::Square);
        self.polygons.len() as i32
    }

    /// `triangulate(index, bb)` as data: three `x, y` float pairs per
    /// triangle.
    pub fn triangulate(&self, index: i32) -> Result<Vec<f32>, ClipperError> {
        let poly = self.polygon_at(index)?;
        let (verts, indices) = triangulate_polygon(poly);
        let mut out = Vec::with_capacity(indices.len() * 2);
        for &i in &indices {
            let p = verts[i as usize];
            out.push(to_float(p.x));
            out.push(to_float(p.y));
        }
        Ok(out)
    }

    /// `triangulate(index, bb)`: writes one `float x, float y` per triangle
    /// corner and returns the number of corners.
    pub fn triangulate_into(&self, index: i32, buf: &mut [u8]) -> Result<i32, ClipperError> {
        let poly = self.polygon_at(index)?;
        let (verts, indices) = triangulate_polygon(poly);
        let mut w = Writer { buf, pos: 0 };
        for &i in &indices {
            let p = verts[i as usize];
            w.put_float(to_float(p.x))?;
            w.put_float(to_float(p.y))?;
        }
        Ok(indices.len() as i32)
    }

    /// `triangulate2(index, bb)` as data: vertices (`x, y` floats) and
    /// triangle indices (as the 16-bit values the game reads).
    pub fn triangulate2(&self, index: i32) -> Result<(Vec<f32>, Vec<u16>), ClipperError> {
        let poly = self.polygon_at(index)?;
        let (verts, indices) = triangulate_polygon(poly);
        Ok((flatten(&verts), indices.iter().map(|&i| i as u16).collect()))
    }

    /// `triangulate2(index, bb)`: writes `short vertexCount`, the vertices,
    /// then one `short` per triangle corner; returns the corner count.
    pub fn triangulate2_into(&self, index: i32, buf: &mut [u8]) -> Result<i32, ClipperError> {
        let poly = self.polygon_at(index)?;
        let (verts, indices) = triangulate_polygon(poly);
        let mut w = Writer { buf, pos: 0 };
        w.put_short(verts.len() as u16)?;
        for p in &verts {
            w.put_float(to_float(p.x))?;
            w.put_float(to_float(p.y))?;
        }
        for &i in &indices {
            w.put_short(i as u16)?;
        }
        Ok(indices.len() as i32)
    }
}

/// `zombie.vehicles.ClipperOffset`.
pub struct ClipperOffset {
    offset: Offset64,
    polygons: Vec<Polygon>,
}

impl Default for ClipperOffset {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipperOffset {
    /// `ClipperOffset.JoinType` ordinals.
    pub const JOIN_SQUARE: i32 = 0;
    pub const JOIN_BEVEL: i32 = 1;
    pub const JOIN_ROUND: i32 = 2;
    pub const JOIN_MITER: i32 = 3;
    /// `ClipperOffset.EndType` ordinals.
    pub const END_POLYGON: i32 = 0;
    pub const END_JOINED: i32 = 1;
    pub const END_BUTT: i32 = 2;
    pub const END_SQUARE: i32 = 3;
    pub const END_ROUND: i32 = 4;

    /// `new ClipperOffset()` (miter limit 2, default arc tolerance).
    pub fn new() -> Self {
        ClipperOffset {
            offset: Offset64::default(),
            polygons: Vec::new(),
        }
    }

    /// `clear()`: drops the added paths (results stay).
    pub fn clear(&mut self) {
        self.offset.clear();
    }

    /// `addPath(numPoints, points, joinType, endType)`.
    pub fn add_path(
        &mut self,
        num_points: i32,
        points: &[f32],
        join_type: i32,
        end_type: i32,
    ) -> Result<(), ClipperError> {
        let path = read_points(num_points, points)?;
        self.offset.add_path(
            &path,
            JoinType::from_i32(join_type),
            EndType::from_i32(end_type),
        );
        Ok(())
    }

    /// `execute(delta)`: offsets every added path by `delta` world units.
    pub fn execute(&mut self, delta: f64) {
        self.polygons.clear();
        let tree = self.offset.execute_tree(delta * 1000.0);
        collect_tree(&tree, &mut self.polygons);
    }

    /// `getPolygonCount()`.
    pub fn get_polygon_count(&self) -> i32 {
        self.polygons.len() as i32
    }

    /// The polygons produced by the last `execute`.
    pub fn polygons(&self) -> &[Polygon] {
        &self.polygons
    }

    fn polygon_at(&self, index: i32) -> Result<&Polygon, ClipperError> {
        if index < 0 {
            return Err(ClipperError::IndexOutOfRange);
        }
        self.polygons
            .get(index as usize)
            .ok_or(ClipperError::IndexOutOfRange)
    }

    /// `getPolygon(index)` as data.
    pub fn get_polygon(&self, index: i32) -> Result<&Polygon, ClipperError> {
        self.polygon_at(index)
    }

    /// `getPolygon(index, bb)`: same layout as [`Clipper::get_polygon_into`].
    pub fn get_polygon_into(&self, index: i32, buf: &mut [u8]) -> Result<i32, ClipperError> {
        let poly = self.polygon_at(index)?;
        let mut w = Writer { buf, pos: 0 };
        w.put_path(&poly.points)?;
        w.put_short(poly.holes.len() as u16)?;
        for h in &poly.holes {
            w.put_path(h)?;
        }
        Ok(0)
    }
}

/// The two call sequences `zombie.iso.zones.Zone` / `IsoMetaGrid` run
/// against the shared `Clipper` / `ClipperOffset` instances, packaged for
/// callers that only need their results (e.g. a `ClipperOps`
/// implementation in the world crate).
#[derive(Default)]
pub struct ZoneClipper {
    pub clipper: Clipper,
    pub offset: ClipperOffset,
}

impl ZoneClipper {
    pub fn new() -> Self {
        Self::default()
    }

    /// `clear(); addPath(n, points, Miter, Butt); execute(delta)`, then the
    /// outer ring of polygon 0 as flat `x, y` floats, or `None` when the
    /// offset produced no polygon.
    pub fn offset_polyline(&mut self, points: &[f32], delta: f64) -> Option<Vec<f32>> {
        self.offset.clear();
        self.offset
            .add_path(
                (points.len() / 2) as i32,
                points,
                ClipperOffset::JOIN_MITER,
                ClipperOffset::END_BUTT,
            )
            .ok()?;
        self.offset.execute(delta);
        if self.offset.get_polygon_count() < 1 {
            return None;
        }
        self.offset.get_polygon(0).ok().map(Polygon::points_f32)
    }

    /// `clear(); addPath(n, points, false); clipAABB(x1, y1, x2, y2);
    /// generatePolygons()`, then the outer ring of every resulting polygon
    /// as flat `x, y` floats (holes are not read by the game).
    pub fn clip_polygon_aabb(
        &mut self,
        points: &[f32],
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> Vec<Vec<f32>> {
        self.clipper.clear();
        if self
            .clipper
            .add_path((points.len() / 2) as i32, points, false, true)
            .is_err()
        {
            return Vec::new();
        }
        self.clipper.clip_aabb(x1, y1, x2, y2);
        let n = self.clipper.generate_polygons();
        (0..n.max(0))
            .filter_map(|i| self.clipper.get_polygon(i).ok().map(Polygon::points_f32))
            .collect()
    }
}

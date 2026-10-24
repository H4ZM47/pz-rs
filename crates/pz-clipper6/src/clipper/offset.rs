//! Port of Clipper 6.4.2 `ClipperOffset`.
//!
//! Copyright (c) Angus Johnson 2010-2017. Distributed under the Boost
//! Software License, Version 1.0 (see `LICENSE-BOOST` in this crate).

use super::{
    fma, orientation, round, ClipType, Clipper, ClipperException, DoublePoint, EndType, IntPoint,
    JoinType, Path, Paths, PolyFillType, PolyTree, PolyType,
};

const PI: f64 = std::f64::consts::PI;
const TWO_PI: f64 = PI * 2.0;
const DEF_ARC_TOLERANCE: f64 = 0.25;
const TOLERANCE: f64 = 1.0e-20;

/// `sin(x)` and `cos(x)`. clang merges the adjacent `std::sin` and
/// `std::cos` calls of `DoOffset` into Apple's `__sincos_stret`, so the
/// same entry point is used here on Apple targets.
#[cfg(target_vendor = "apple")]
fn sin_cos(x: f64) -> (f64, f64) {
    #[repr(C)]
    struct SinCos {
        sin: f64,
        cos: f64,
    }
    extern "C" {
        fn __sincos_stret(x: f64) -> SinCos;
    }
    // SAFETY: a pure libm function taking and returning plain doubles.
    let r = unsafe { __sincos_stret(x) };
    (r.sin, r.cos)
}

#[cfg(not(target_vendor = "apple"))]
fn sin_cos(x: f64) -> (f64, f64) {
    (x.sin(), x.cos())
}

/// `ClipperLib::GetUnitNormal()`.
fn get_unit_normal(pt1: IntPoint, pt2: IntPoint) -> DoublePoint {
    if pt2.x == pt1.x && pt2.y == pt1.y {
        return DoublePoint::new(0.0, 0.0);
    }
    let mut dx = pt2.x.wrapping_sub(pt1.x) as f64;
    let mut dy = pt2.y.wrapping_sub(pt1.y) as f64;
    let f = 1.0 / fma(dx, dx, dy * dy).sqrt();
    dx *= f;
    dy *= f;
    DoublePoint::new(dy, -dx)
}

#[derive(Clone, Debug)]
struct OffsetNode {
    contour: Path,
    join_type: JoinType,
    end_type: EndType,
}

/// `ClipperLib::ClipperOffset`.
#[derive(Clone, Debug)]
pub struct ClipperOffset {
    /// `MiterLimit`.
    pub miter_limit: f64,
    /// `ArcTolerance`.
    pub arc_tolerance: f64,
    dest_polys: Paths,
    src_poly: Path,
    dest_poly: Path,
    normals: Vec<DoublePoint>,
    delta: f64,
    sin_a: f64,
    sin: f64,
    cos: f64,
    miter_lim: f64,
    steps_per_rad: f64,
    lowest: IntPoint,
    poly_nodes: Vec<OffsetNode>,
}

impl Default for ClipperOffset {
    fn default() -> Self {
        Self::new(2.0, DEF_ARC_TOLERANCE)
    }
}

impl ClipperOffset {
    /// `ClipperOffset(double miterLimit = 2.0, double roundPrecision = 0.25)`.
    pub fn new(miter_limit: f64, arc_tolerance: f64) -> Self {
        Self {
            miter_limit,
            arc_tolerance,
            dest_polys: Paths::new(),
            src_poly: Path::new(),
            dest_poly: Path::new(),
            normals: Vec::new(),
            delta: 0.0,
            sin_a: 0.0,
            sin: 0.0,
            cos: 0.0,
            miter_lim: 0.0,
            steps_per_rad: 0.0,
            lowest: IntPoint::new(-1, 0),
            poly_nodes: Vec::new(),
        }
    }

    /// `ClipperOffset::Clear()`.
    pub fn clear(&mut self) {
        self.poly_nodes.clear();
        self.lowest.x = -1;
    }

    /// `ClipperOffset::AddPath()`.
    pub fn add_path(&mut self, path: &[IntPoint], join_type: JoinType, end_type: EndType) {
        let mut high_i = path.len() as i32 - 1;
        if high_i < 0 {
            return;
        }
        // strip duplicate points from path and also get index to the lowest point ...
        if end_type == EndType::ClosedLine || end_type == EndType::ClosedPolygon {
            while high_i > 0 && path[0] == path[high_i as usize] {
                high_i -= 1;
            }
        }
        let mut contour = Path::with_capacity(high_i as usize + 1);
        contour.push(path[0]);
        let mut j = 0usize;
        let mut k = 0usize;
        for &p in &path[1..=high_i as usize] {
            if contour[j] != p {
                j += 1;
                contour.push(p);
                if p.y > contour[k].y || (p.y == contour[k].y && p.x < contour[k].x) {
                    k = j;
                }
            }
        }
        if end_type == EndType::ClosedPolygon && j < 2 {
            return;
        }
        self.poly_nodes.push(OffsetNode {
            contour,
            join_type,
            end_type,
        });

        // if this path's lowest pt is lower than all the others then update m_lowest
        if end_type != EndType::ClosedPolygon {
            return;
        }
        let last = self.poly_nodes.len() as i64 - 1;
        if self.lowest.x < 0 {
            self.lowest = IntPoint::new(last, k as i64);
        } else {
            let ip = self.poly_nodes[self.lowest.x as i32 as usize].contour
                [self.lowest.y as i32 as usize];
            let nk = self.poly_nodes[last as usize].contour[k];
            if nk.y > ip.y || (nk.y == ip.y && nk.x < ip.x) {
                self.lowest = IntPoint::new(last, k as i64);
            }
        }
    }

    /// `ClipperOffset::AddPaths()`.
    pub fn add_paths(&mut self, paths: &[Path], join_type: JoinType, end_type: EndType) {
        for p in paths {
            self.add_path(p, join_type, end_type);
        }
    }

    fn fix_orientations(&mut self) {
        // fixup orientations of all closed paths if the orientation of the
        // closed path with the lowermost vertex is wrong ...
        if self.lowest.x >= 0
            && !orientation(&self.poly_nodes[self.lowest.x as i32 as usize].contour)
        {
            for node in &mut self.poly_nodes {
                if node.end_type == EndType::ClosedPolygon
                    || (node.end_type == EndType::ClosedLine && orientation(&node.contour))
                {
                    node.contour.reverse();
                }
            }
        } else {
            for node in &mut self.poly_nodes {
                if node.end_type == EndType::ClosedLine && !orientation(&node.contour) {
                    node.contour.reverse();
                }
            }
        }
    }

    /// `ClipperOffset::Execute(Paths&, double)`. `Err` is an exception
    /// thrown by the inner `Clipper` (coordinates out of range).
    pub fn execute(&mut self, solution: &mut Paths, delta: f64) -> Result<(), ClipperException> {
        solution.clear();
        self.fix_orientations();
        self.do_offset(delta);

        // now clean up 'corners' ...
        let mut clpr = Clipper::new(0);
        clpr.add_paths(&self.dest_polys, PolyType::Subject, true)?;
        if delta > 0.0 {
            clpr.execute(
                ClipType::Union,
                solution,
                PolyFillType::Positive,
                PolyFillType::Positive,
            )?;
        } else {
            let r = clpr.get_bounds();
            let outer = [
                IntPoint::new(r.left.wrapping_sub(10), r.bottom.wrapping_add(10)),
                IntPoint::new(r.right.wrapping_add(10), r.bottom.wrapping_add(10)),
                IntPoint::new(r.right.wrapping_add(10), r.top.wrapping_sub(10)),
                IntPoint::new(r.left.wrapping_sub(10), r.top.wrapping_sub(10)),
            ];
            clpr.add_path(&outer, PolyType::Subject, true)?;
            clpr.set_reverse_solution(true);
            clpr.execute(
                ClipType::Union,
                solution,
                PolyFillType::Negative,
                PolyFillType::Negative,
            )?;
            if !solution.is_empty() {
                solution.remove(0);
            }
        }
        Ok(())
    }

    /// `ClipperOffset::Execute(PolyTree&, double)`. `Err` is an exception
    /// thrown by the inner `Clipper` (coordinates out of range).
    pub fn execute_tree(
        &mut self,
        solution: &mut PolyTree,
        delta: f64,
    ) -> Result<(), ClipperException> {
        solution.clear();
        self.fix_orientations();
        self.do_offset(delta);

        // now clean up 'corners' ...
        let mut clpr = Clipper::new(0);
        clpr.add_paths(&self.dest_polys, PolyType::Subject, true)?;
        if delta > 0.0 {
            clpr.execute_tree(
                ClipType::Union,
                solution,
                PolyFillType::Positive,
                PolyFillType::Positive,
            );
        } else {
            let r = clpr.get_bounds();
            let outer = [
                IntPoint::new(r.left.wrapping_sub(10), r.bottom.wrapping_add(10)),
                IntPoint::new(r.right.wrapping_add(10), r.bottom.wrapping_add(10)),
                IntPoint::new(r.right.wrapping_add(10), r.top.wrapping_sub(10)),
                IntPoint::new(r.left.wrapping_sub(10), r.top.wrapping_sub(10)),
            ];
            clpr.add_path(&outer, PolyType::Subject, true)?;
            clpr.set_reverse_solution(true);
            clpr.execute_tree(
                ClipType::Union,
                solution,
                PolyFillType::Negative,
                PolyFillType::Negative,
            );
            // remove the outer PolyNode rectangle ...
            let root = PolyTree::ROOT;
            if solution.child_count(root) == 1 && solution.child_count(solution.childs(root)[0]) > 0
            {
                let outer_node = solution.childs(root)[0];
                let outer_childs = solution.childs(outer_node).to_vec();
                let outer_parent = solution.parent(outer_node);
                solution.node_mut(root).childs[0] = outer_childs[0];
                solution.node_mut(outer_childs[0]).parent = outer_parent;
                for &c in &outer_childs[1..] {
                    solution.add_child(root, c);
                }
            } else {
                solution.clear();
            }
        }
        Ok(())
    }

    fn do_offset(&mut self, delta: f64) {
        self.dest_polys.clear();
        self.delta = delta;

        // if Zero offset, just copy any CLOSED polygons to m_p and return ...
        if delta > -TOLERANCE && delta < TOLERANCE {
            for node in &self.poly_nodes {
                if node.end_type == EndType::ClosedPolygon {
                    self.dest_polys.push(node.contour.clone());
                }
            }
            return;
        }

        // see offset_triginometry3.svg in the documentation folder ...
        if self.miter_limit > 2.0 {
            self.miter_lim = 2.0 / (self.miter_limit * self.miter_limit);
        } else {
            self.miter_lim = 0.5;
        }

        let y = if self.arc_tolerance <= 0.0 {
            DEF_ARC_TOLERANCE
        } else if self.arc_tolerance > delta.abs() * DEF_ARC_TOLERANCE {
            delta.abs() * DEF_ARC_TOLERANCE
        } else {
            self.arc_tolerance
        };

        // see offset_triginometry2.svg in the documentation folder ...
        let mut steps = PI / (1.0 - y / delta.abs()).acos();
        if steps > delta.abs() * PI {
            steps = delta.abs() * PI; // ie excessive precision check
        }
        let (s, c) = sin_cos(TWO_PI / steps);
        self.sin = s;
        self.cos = c;
        self.steps_per_rad = steps / TWO_PI;
        if delta < 0.0 {
            self.sin = -self.sin;
        }

        for ni in 0..self.poly_nodes.len() {
            let (join_type, end_type) = {
                let node = &self.poly_nodes[ni];
                (node.join_type, node.end_type)
            };
            self.src_poly.clone_from(&self.poly_nodes[ni].contour);

            let len = self.src_poly.len();
            if len == 0 || (delta <= 0.0 && (len < 3 || end_type != EndType::ClosedPolygon)) {
                continue;
            }

            self.dest_poly.clear();
            if len == 1 {
                let s0 = self.src_poly[0];
                if join_type == JoinType::Round {
                    let mut x = 1.0f64;
                    let mut y = 0.0f64;
                    let mut j: i64 = 1;
                    while j as f64 <= steps {
                        self.dest_poly.push(IntPoint::new(
                            round(fma(x, delta, s0.x as f64)),
                            round(fma(y, delta, s0.y as f64)),
                        ));
                        let x2 = x;
                        x = fma(x, self.cos, -(self.sin * y));
                        y = fma(x2, self.sin, y * self.cos);
                        j += 1;
                    }
                } else {
                    let mut x = -1.0f64;
                    let mut y = -1.0f64;
                    for _ in 0..4 {
                        self.dest_poly.push(IntPoint::new(
                            round(fma(x, delta, s0.x as f64)),
                            round(fma(y, delta, s0.y as f64)),
                        ));
                        if x < 0.0 {
                            x = 1.0;
                        } else if y < 0.0 {
                            y = 1.0;
                        } else {
                            x = -1.0;
                        }
                    }
                }
                self.dest_polys.push(self.dest_poly.clone());
                continue;
            }

            // build m_normals ...
            self.normals.clear();
            for j in 0..len - 1 {
                self.normals
                    .push(get_unit_normal(self.src_poly[j], self.src_poly[j + 1]));
            }
            if end_type == EndType::ClosedLine || end_type == EndType::ClosedPolygon {
                self.normals
                    .push(get_unit_normal(self.src_poly[len - 1], self.src_poly[0]));
            } else {
                self.normals.push(self.normals[len - 2]);
            }

            if end_type == EndType::ClosedPolygon {
                let mut k = len - 1;
                for j in 0..len {
                    self.offset_point(j, &mut k, join_type);
                }
                self.dest_polys.push(self.dest_poly.clone());
            } else if end_type == EndType::ClosedLine {
                let mut k = len - 1;
                for j in 0..len {
                    self.offset_point(j, &mut k, join_type);
                }
                self.dest_polys.push(self.dest_poly.clone());
                self.dest_poly.clear();
                // re-build m_normals ...
                let n = self.normals[len - 1];
                for j in (1..len).rev() {
                    let p = self.normals[j - 1];
                    self.normals[j] = DoublePoint::new(-p.x, -p.y);
                }
                self.normals[0] = DoublePoint::new(-n.x, -n.y);
                k = 0;
                for j in (0..len).rev() {
                    self.offset_point(j, &mut k, join_type);
                }
                self.dest_polys.push(self.dest_poly.clone());
            } else {
                let mut k = 0usize;
                for j in 1..len - 1 {
                    self.offset_point(j, &mut k, join_type);
                }

                if end_type == EndType::OpenButt {
                    let j = len - 1;
                    let (s, n) = (self.src_poly[j], self.normals[j]);
                    self.dest_poly.push(IntPoint::new(
                        round(fma(n.x, delta, s.x as f64)),
                        round(fma(n.y, delta, s.y as f64)),
                    ));
                    self.dest_poly.push(IntPoint::new(
                        round(fma(-n.x, delta, s.x as f64)),
                        round(fma(-n.y, delta, s.y as f64)),
                    ));
                } else {
                    let j = len - 1;
                    k = len - 2;
                    self.sin_a = 0.0;
                    let n = self.normals[j];
                    self.normals[j] = DoublePoint::new(-n.x, -n.y);
                    if end_type == EndType::OpenSquare {
                        self.do_square(j, k);
                    } else {
                        self.do_round(j, k);
                    }
                }

                // re-build m_normals ...
                for j in (1..len).rev() {
                    let p = self.normals[j - 1];
                    self.normals[j] = DoublePoint::new(-p.x, -p.y);
                }
                let n1 = self.normals[1];
                self.normals[0] = DoublePoint::new(-n1.x, -n1.y);

                k = len - 1;
                for j in (1..k).rev() {
                    self.offset_point(j, &mut k, join_type);
                }

                if end_type == EndType::OpenButt {
                    let (s, n) = (self.src_poly[0], self.normals[0]);
                    self.dest_poly.push(IntPoint::new(
                        round(fma(-n.x, delta, s.x as f64)),
                        round(fma(-n.y, delta, s.y as f64)),
                    ));
                    self.dest_poly.push(IntPoint::new(
                        round(fma(n.x, delta, s.x as f64)),
                        round(fma(n.y, delta, s.y as f64)),
                    ));
                } else {
                    self.sin_a = 0.0;
                    if end_type == EndType::OpenSquare {
                        self.do_square(0, 1);
                    } else {
                        self.do_round(0, 1);
                    }
                }
                self.dest_polys.push(self.dest_poly.clone());
            }
        }
    }

    fn offset_point(&mut self, j: usize, k: &mut usize, join_type: JoinType) {
        let (nj, nk) = (self.normals[j], self.normals[*k]);
        let sj = self.src_poly[j];
        // cross product ...
        self.sin_a = fma(nk.x, nj.y, -(nj.x * nk.y));
        if (self.sin_a * self.delta).abs() < 1.0 {
            // dot product ...
            let cos_a = fma(nk.x, nj.x, nj.y * nk.y);
            if cos_a > 0.0 {
                // angle => 0 degrees
                self.dest_poly.push(IntPoint::new(
                    round(fma(nk.x, self.delta, sj.x as f64)),
                    round(fma(nk.y, self.delta, sj.y as f64)),
                ));
                return;
            }
            // else angle => 180 degrees
        } else {
            self.sin_a = self.sin_a.clamp(-1.0, 1.0);
        }

        if self.sin_a * self.delta < 0.0 {
            self.dest_poly.push(IntPoint::new(
                round(fma(nk.x, self.delta, sj.x as f64)),
                round(fma(nk.y, self.delta, sj.y as f64)),
            ));
            self.dest_poly.push(sj);
            self.dest_poly.push(IntPoint::new(
                round(fma(nj.x, self.delta, sj.x as f64)),
                round(fma(nj.y, self.delta, sj.y as f64)),
            ));
        } else {
            match join_type {
                JoinType::Miter => {
                    let r = 1.0 + fma(nj.x, nk.x, nj.y * nk.y);
                    if r >= self.miter_lim {
                        self.do_miter(j, *k, r);
                    } else {
                        self.do_square(j, *k);
                    }
                }
                JoinType::Square => self.do_square(j, *k),
                JoinType::Round => self.do_round(j, *k),
            }
        }
        *k = j;
    }

    fn do_square(&mut self, j: usize, k: usize) {
        let (nj, nk) = (self.normals[j], self.normals[k]);
        let sj = self.src_poly[j];
        let dx = (self.sin_a.atan2(fma(nk.x, nj.x, nk.y * nj.y)) / 4.0).tan();
        self.dest_poly.push(IntPoint::new(
            round(fma(self.delta, fma(-nk.y, dx, nk.x), sj.x as f64)),
            round(fma(self.delta, fma(nk.x, dx, nk.y), sj.y as f64)),
        ));
        self.dest_poly.push(IntPoint::new(
            round(fma(self.delta, fma(nj.y, dx, nj.x), sj.x as f64)),
            round(fma(self.delta, fma(-nj.x, dx, nj.y), sj.y as f64)),
        ));
    }

    fn do_miter(&mut self, j: usize, k: usize, r: f64) {
        let (nj, nk) = (self.normals[j], self.normals[k]);
        let sj = self.src_poly[j];
        let q = self.delta / r;
        self.dest_poly.push(IntPoint::new(
            round(fma(nk.x + nj.x, q, sj.x as f64)),
            round(fma(nk.y + nj.y, q, sj.y as f64)),
        ));
    }

    fn do_round(&mut self, j: usize, k: usize) {
        let (nj, nk) = (self.normals[j], self.normals[k]);
        let sj = self.src_poly[j];
        let a = self.sin_a.atan2(fma(nk.x, nj.x, nk.y * nj.y));
        let steps = (round(self.steps_per_rad * a.abs()) as i32).max(1);

        let mut x = nk.x;
        let mut y = nk.y;
        for _ in 0..steps {
            self.dest_poly.push(IntPoint::new(
                round(fma(x, self.delta, sj.x as f64)),
                round(fma(y, self.delta, sj.y as f64)),
            ));
            let x2 = x;
            x = fma(x, self.cos, -(self.sin * y));
            y = fma(x2, self.sin, y * self.cos);
        }
        self.dest_poly.push(IntPoint::new(
            round(fma(nj.x, self.delta, sj.x as f64)),
            round(fma(nj.y, self.delta, sj.y as f64)),
        ));
    }
}

//! Port of Clipper2 1.5.4 `clipper.offset.cpp` (`ClipperOffset`), without
//! the delta callback (never set by the game).
//!
//! Copyright (c) Angus Johnson 2010-2025. Distributed under the Boost
//! Software License, Version 1.0 (see `LICENSE-BOOST` in this crate).

use crate::core::*;
use crate::engine::{ClipType, Clipper64, PathType, PolyPath64};

const FLOATING_POINT_TOLERANCE: f64 = 1e-12;
const ARC_CONST: f64 = 0.002;

/// `JoinType`. Values outside the enum take the C++ fall-through branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinType {
    Square,
    Bevel,
    Round,
    Miter,
    Other(i32),
}

impl JoinType {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => JoinType::Square,
            1 => JoinType::Bevel,
            2 => JoinType::Round,
            3 => JoinType::Miter,
            o => JoinType::Other(o),
        }
    }
}

/// `EndType`. Values outside the enum take the C++ fall-through branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndType {
    Polygon,
    Joined,
    Butt,
    Square,
    Round,
    Other(i32),
}

impl EndType {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => EndType::Polygon,
            1 => EndType::Joined,
            2 => EndType::Butt,
            3 => EndType::Square,
            4 => EndType::Round,
            o => EndType::Other(o),
        }
    }
}

#[derive(Clone, Debug)]
struct Group {
    paths_in: Paths64,
    lowest_path_idx: Option<usize>,
    is_reversed: bool,
    join_type: JoinType,
    end_type: EndType,
}

fn get_lowest_closed_path_info(paths: &[Path64], idx: &mut Option<usize>, is_neg_area: &mut bool) {
    *idx = None;
    let mut bot_pt = Point64::new(i64::MAX, i64::MIN);
    for (i, path) in paths.iter().enumerate() {
        let mut a = MAX_DBL;
        for pt in path {
            if pt.y < bot_pt.y || (pt.y == bot_pt.y && pt.x >= bot_pt.x) {
                continue;
            }
            if a == MAX_DBL {
                a = area_path(path);
                if a == 0.0 {
                    break;
                }
                *is_neg_area = a < 0.0;
            }
            *idx = Some(i);
            bot_pt.x = pt.x;
            bot_pt.y = pt.y;
        }
    }
}

impl Group {
    fn new(paths: Paths64, join_type: JoinType, end_type: EndType) -> Self {
        let mut paths_in = paths;
        let is_joined = end_type == EndType::Polygon || end_type == EndType::Joined;
        for p in paths_in.iter_mut() {
            strip_duplicates(p, is_joined);
        }
        let (lowest_path_idx, is_reversed) = if end_type == EndType::Polygon {
            let mut is_neg_area = false;
            let mut idx = None;
            get_lowest_closed_path_info(&paths_in, &mut idx, &mut is_neg_area);
            (idx, idx.is_some() && is_neg_area)
        } else {
            (None, false)
        };
        Group {
            paths_in,
            lowest_path_idx,
            is_reversed,
            join_type,
            end_type,
        }
    }
}

#[inline]
fn hypot(x: f64, y: f64) -> f64 {
    fma(x, x, y * y).sqrt()
}

fn get_unit_normal(pt1: Point64, pt2: Point64) -> PointD {
    if pt1 == pt2 {
        return PointD::new(0.0, 0.0);
    }
    let mut dx = pt2.x.wrapping_sub(pt1.x) as f64;
    let mut dy = pt2.y.wrapping_sub(pt1.y) as f64;
    let inverse_hypot = 1.0 / hypot(dx, dy);
    dx *= inverse_hypot;
    dy *= inverse_hypot;
    PointD::new(dy, -dx)
}

#[inline]
fn almost_zero(value: f64) -> bool {
    value.abs() < 0.001
}

fn normalize_vector(vec: PointD) -> PointD {
    let h = hypot(vec.x, vec.y);
    if almost_zero(h) {
        return PointD::new(0.0, 0.0);
    }
    let inverse_hypot = 1.0 / h;
    PointD::new(vec.x * inverse_hypot, vec.y * inverse_hypot)
}

#[inline]
fn get_avg_unit_vector(vec1: PointD, vec2: PointD) -> PointD {
    normalize_vector(PointD::new(vec1.x + vec2.x, vec1.y + vec2.y))
}

#[inline]
fn get_perpendic(pt: Point64, norm: PointD, delta: f64) -> Point64 {
    Point64::from_f64(
        fma(norm.x, delta, pt.x as f64),
        fma(norm.y, delta, pt.y as f64),
    )
}

#[inline]
fn get_perpendic_d(pt: Point64, norm: PointD, delta: f64) -> PointD {
    PointD::new(
        fma(norm.x, delta, pt.x as f64),
        fma(norm.y, delta, pt.y as f64),
    )
}

#[inline]
fn translate_point(pt: PointD, dx: f64, dy: f64) -> PointD {
    PointD::new(pt.x + dx, pt.y + dy)
}

#[inline]
fn reflect_point(pt: PointD, pivot: PointD) -> PointD {
    PointD::new(pivot.x + (pivot.x - pt.x), pivot.y + (pivot.y - pt.y))
}

/// `ClipperOffset`.
#[derive(Clone, Debug)]
pub struct ClipperOffset {
    error_code: i32,
    delta: f64,
    group_delta: f64,
    temp_lim: f64,
    steps_per_rad: f64,
    step_sin: f64,
    step_cos: f64,
    norms: PathD,
    path_out: Path64,
    groups: Vec<Group>,
    join_type: JoinType,
    end_type: EndType,
    pub miter_limit: f64,
    pub arc_tolerance: f64,
    pub preserve_collinear: bool,
    pub reverse_solution: bool,
}

impl Default for ClipperOffset {
    fn default() -> Self {
        Self::new(2.0, 0.0, false, false)
    }
}

impl ClipperOffset {
    pub fn new(
        miter_limit: f64,
        arc_tolerance: f64,
        preserve_collinear: bool,
        reverse_solution: bool,
    ) -> Self {
        ClipperOffset {
            error_code: 0,
            delta: 0.0,
            group_delta: 0.0,
            temp_lim: 0.0,
            steps_per_rad: 0.0,
            step_sin: 0.0,
            step_cos: 0.0,
            norms: PathD::new(),
            path_out: Path64::new(),
            groups: Vec::new(),
            join_type: JoinType::Bevel,
            end_type: EndType::Polygon,
            miter_limit,
            arc_tolerance,
            preserve_collinear,
            reverse_solution,
        }
    }

    /// `Clear()`.
    pub fn clear(&mut self) {
        self.groups.clear();
        self.norms.clear();
    }

    /// `AddPath(path, jt, et)`.
    pub fn add_path(&mut self, path: &[Point64], jt: JoinType, et: EndType) {
        self.groups.push(Group::new(vec![path.to_vec()], jt, et));
    }

    /// `AddPaths(paths, jt, et)`.
    pub fn add_paths(&mut self, paths: &[Path64], jt: JoinType, et: EndType) {
        if paths.is_empty() {
            return;
        }
        self.groups.push(Group::new(paths.to_vec(), jt, et));
    }

    fn build_normals(&mut self, path: &[Point64]) {
        self.norms.clear();
        self.norms.reserve(path.len());
        if path.is_empty() {
            return;
        }
        for w in path.windows(2) {
            self.norms.push(get_unit_normal(w[0], w[1]));
        }
        self.norms
            .push(get_unit_normal(path[path.len() - 1], path[0]));
    }

    fn do_bevel(&mut self, path: &[Point64], j: usize, k: usize) {
        let pj = path[j];
        let (pt1, pt2) = if j == k {
            let abs_delta = self.group_delta.abs();
            let n = self.norms[j];
            (
                PointD::new(
                    fma(-abs_delta, n.x, pj.x as f64),
                    fma(-abs_delta, n.y, pj.y as f64),
                ),
                PointD::new(
                    fma(abs_delta, n.x, pj.x as f64),
                    fma(abs_delta, n.y, pj.y as f64),
                ),
            )
        } else {
            let gd = self.group_delta;
            let (nk, nj) = (self.norms[k], self.norms[j]);
            (
                PointD::new(fma(gd, nk.x, pj.x as f64), fma(gd, nk.y, pj.y as f64)),
                PointD::new(fma(gd, nj.x, pj.x as f64), fma(gd, nj.y, pj.y as f64)),
            )
        };
        self.path_out.push(Point64::from_pointd(pt1));
        self.path_out.push(Point64::from_pointd(pt2));
    }

    fn do_square(&mut self, path: &[Point64], j: usize, k: usize) {
        let nj = self.norms[j];
        let nk = self.norms[k];
        let vec = if j == k {
            PointD::new(nj.y, -nj.x)
        } else {
            get_avg_unit_vector(PointD::new(-nk.y, nk.x), PointD::new(nj.y, -nj.x))
        };
        let gd = self.group_delta;
        let abs_delta = gd.abs();
        let mut pt_q = PointD::from_point64(path[j]);
        pt_q = translate_point(pt_q, abs_delta * vec.x, abs_delta * vec.y);
        let pt1 = translate_point(pt_q, gd * vec.y, gd * -vec.x);
        let pt2 = translate_point(pt_q, gd * -vec.y, gd * vec.x);
        let pt3 = get_perpendic_d(path[k], nk, gd);
        if j == k {
            let pt4 = PointD::new(fma(vec.x, gd, pt3.x), fma(vec.y, gd, pt3.y));
            let mut pt = pt_q;
            get_segment_intersect_pt_d(pt1, pt2, pt3, pt4, &mut pt);
            self.path_out
                .push(Point64::from_pointd(reflect_point(pt, pt_q)));
            self.path_out.push(Point64::from_pointd(pt));
        } else {
            let pt4 = get_perpendic_d(path[j], nk, gd);
            let mut pt = pt_q;
            get_segment_intersect_pt_d(pt1, pt2, pt3, pt4, &mut pt);
            self.path_out.push(Point64::from_pointd(pt));
            self.path_out
                .push(Point64::from_pointd(reflect_point(pt, pt_q)));
        }
    }

    fn do_miter(&mut self, path: &[Point64], j: usize, k: usize, cos_a: f64) {
        let q = self.group_delta / (cos_a + 1.0);
        let (nk, nj, pj) = (self.norms[k], self.norms[j], path[j]);
        self.path_out.push(Point64::from_f64(
            fma(nk.x + nj.x, q, pj.x as f64),
            fma(nk.y + nj.y, q, pj.y as f64),
        ));
    }

    fn do_round(&mut self, path: &[Point64], j: usize, k: usize, angle: f64) {
        let pt = path[j];
        let nk = self.norms[k];
        let mut offset_vec = PointD::new(nk.x * self.group_delta, nk.y * self.group_delta);
        if j == k {
            offset_vec = PointD::new(-offset_vec.x, -offset_vec.y);
        }
        self.path_out.push(Point64::from_f64(
            pt.x as f64 + offset_vec.x,
            pt.y as f64 + offset_vec.y,
        ));
        let steps = (self.steps_per_rad * angle.abs()).ceil() as i32;
        let (sc, ss) = (self.step_cos, self.step_sin);
        for _ in 1..steps.max(1) {
            offset_vec = PointD::new(
                fma(offset_vec.x, sc, -(ss * offset_vec.y)),
                fma(offset_vec.x, ss, offset_vec.y * sc),
            );
            self.path_out.push(Point64::from_f64(
                pt.x as f64 + offset_vec.x,
                pt.y as f64 + offset_vec.y,
            ));
        }
        self.path_out
            .push(get_perpendic(path[j], self.norms[j], self.group_delta));
    }

    #[allow(clippy::manual_clamp)]
    fn offset_point(&mut self, path: &[Point64], j: usize, k: usize) {
        if path[j] == path[k] {
            return;
        }
        let mut sin_a = cross_product_vec(self.norms[j], self.norms[k]);
        let cos_a = dot_product_vec(self.norms[j], self.norms[k]);
        if sin_a > 1.0 {
            sin_a = 1.0;
        } else if sin_a < -1.0 {
            sin_a = -1.0;
        }
        if self.group_delta.abs() <= FLOATING_POINT_TOLERANCE {
            self.path_out.push(path[j]);
            return;
        }
        if cos_a > -0.999 && (sin_a * self.group_delta < 0.0) {
            self.path_out
                .push(get_perpendic(path[j], self.norms[k], self.group_delta));
            self.path_out.push(path[j]);
            self.path_out
                .push(get_perpendic(path[j], self.norms[j], self.group_delta));
        } else if cos_a > 0.999 && self.join_type != JoinType::Round {
            self.do_miter(path, j, k, cos_a);
        } else if self.join_type == JoinType::Miter {
            if cos_a > self.temp_lim - 1.0 {
                self.do_miter(path, j, k, cos_a);
            } else {
                self.do_square(path, j, k);
            }
        } else if self.join_type == JoinType::Round {
            self.do_round(path, j, k, sin_a.atan2(cos_a));
        } else if self.join_type == JoinType::Bevel {
            self.do_bevel(path, j, k);
        } else {
            self.do_square(path, j, k);
        }
    }

    fn offset_polygon(&mut self, path: &[Point64], solution: &mut Paths64) {
        self.path_out.clear();
        let n = path.len();
        let mut k = n.wrapping_sub(1);
        for j in 0..n {
            self.offset_point(path, j, k);
            k = j;
        }
        solution.push(self.path_out.clone());
    }

    fn offset_open_joined(&mut self, path: &[Point64], solution: &mut Paths64) {
        self.offset_polygon(path, solution);
        let mut reverse_path: Path64 = path.to_vec();
        reverse_path.reverse();
        self.norms.reverse();
        let n0 = self.norms[0];
        self.norms.push(n0);
        self.norms.remove(0);
        for p in self.norms.iter_mut() {
            p.x = -p.x;
            p.y = -p.y;
        }
        self.offset_polygon(&reverse_path, solution);
    }

    fn offset_open_path(&mut self, path: &[Point64], solution: &mut Paths64) {
        if self.group_delta.abs() <= FLOATING_POINT_TOLERANCE {
            self.path_out.push(path[0]);
        } else {
            match self.end_type {
                EndType::Butt => self.do_bevel(path, 0, 0),
                EndType::Round => self.do_round(path, 0, 0, PI),
                _ => self.do_square(path, 0, 0),
            }
        }
        let high_i = path.len() - 1;
        let mut k = 0;
        for j in 1..high_i {
            self.offset_point(path, j, k);
            k = j;
        }
        for i in (1..=high_i).rev() {
            let p = self.norms[i - 1];
            self.norms[i] = PointD::new(-p.x, -p.y);
        }
        self.norms[0] = self.norms[high_i];
        if self.group_delta.abs() <= FLOATING_POINT_TOLERANCE {
            self.path_out.push(path[high_i]);
        } else {
            match self.end_type {
                EndType::Butt => self.do_bevel(path, high_i, high_i),
                EndType::Round => self.do_round(path, high_i, high_i, PI),
                _ => self.do_square(path, high_i, high_i),
            }
        }
        let mut k = high_i;
        let mut j = high_i - 1;
        while j > 0 {
            self.offset_point(path, j, k);
            k = j;
            j -= 1;
        }
        solution.push(self.path_out.clone());
    }

    fn do_group_offset(&mut self, gi: usize, solution: &mut Paths64) {
        let (end_type, join_type, is_reversed, has_lowest) = {
            let g = &self.groups[gi];
            (
                g.end_type,
                g.join_type,
                g.is_reversed,
                g.lowest_path_idx.is_some(),
            )
        };
        if end_type == EndType::Polygon {
            if !has_lowest {
                self.delta = self.delta.abs();
            }
            self.group_delta = if is_reversed { -self.delta } else { self.delta };
        } else {
            self.group_delta = self.delta.abs();
        }
        let abs_delta = self.group_delta.abs();
        self.join_type = join_type;
        self.end_type = end_type;

        if join_type == JoinType::Round || end_type == EndType::Round {
            let arc_tol = if self.arc_tolerance > FLOATING_POINT_TOLERANCE {
                abs_delta.min(self.arc_tolerance)
            } else {
                abs_delta * ARC_CONST
            };
            let steps_per_360 = (PI / (1.0 - arc_tol / abs_delta).acos()).min(abs_delta * PI);
            self.step_sin = (2.0 * PI / steps_per_360).sin();
            self.step_cos = (2.0 * PI / steps_per_360).cos();
            if self.group_delta < 0.0 {
                self.step_sin = -self.step_sin;
            }
            self.steps_per_rad = steps_per_360 / (2.0 * PI);
        }

        let paths = self.groups[gi].paths_in.clone();
        for path in &paths {
            let path_len = path.len();
            self.path_out.clear();
            if path_len == 1 {
                if self.group_delta < 1.0 {
                    continue;
                }
                let pt = path[0];
                if join_type == JoinType::Round {
                    let radius = abs_delta;
                    let steps = if self.steps_per_rad > 0.0 {
                        (self.steps_per_rad * 2.0 * PI).ceil() as usize
                    } else {
                        0
                    };
                    self.path_out = ellipse(pt, radius, radius, steps);
                } else {
                    let d = abs_delta.ceil() as i32 as i64;
                    let r = Rect64::new(
                        pt.x.wrapping_sub(d),
                        pt.y.wrapping_sub(d),
                        pt.x.wrapping_add(d),
                        pt.y.wrapping_add(d),
                    );
                    self.path_out = r.as_path();
                }
                solution.push(self.path_out.clone());
                continue;
            }
            if path_len == 2 && end_type == EndType::Joined {
                self.end_type = if join_type == JoinType::Round {
                    EndType::Round
                } else {
                    EndType::Square
                };
            }
            self.build_normals(path);
            if self.end_type == EndType::Polygon {
                self.offset_polygon(path, solution);
            } else if self.end_type == EndType::Joined {
                self.offset_open_joined(path, solution);
            } else {
                self.offset_open_path(path, solution);
            }
        }
    }

    fn check_reverse_orientation(&self) -> bool {
        for g in &self.groups {
            if g.end_type == EndType::Polygon {
                return g.is_reversed;
            }
        }
        false
    }

    /// `ExecuteInternal` into a `PolyTree64` (`Execute(delta, PolyTree64&)`).
    pub fn execute_tree(&mut self, delta: f64) -> PolyPath64 {
        self.error_code = 0;
        let mut solution = Paths64::new();
        if self.groups.is_empty() {
            return PolyPath64::default();
        }
        if delta.abs() < 0.5 {
            for g in &self.groups {
                solution.extend(g.paths_in.iter().cloned());
            }
        } else {
            self.temp_lim = if self.miter_limit <= 1.0 {
                2.0
            } else {
                2.0 / (self.miter_limit * self.miter_limit)
            };
            self.delta = delta;
            for gi in 0..self.groups.len() {
                self.do_group_offset(gi, &mut solution);
                if self.error_code == 0 {
                    continue;
                }
                solution.clear();
            }
        }
        if solution.is_empty() {
            return PolyPath64::default();
        }
        let paths_reversed = self.check_reverse_orientation();
        let mut c = Clipper64::new();
        c.preserve_collinear = self.preserve_collinear;
        c.reverse_solution = self.reverse_solution != paths_reversed;
        c.add_paths(&solution, PathType::Subject, false);
        let fr = if paths_reversed {
            FillRule::Negative
        } else {
            FillRule::Positive
        };
        // Clipper64::Execute(ct, fr, PolyTree64&): the tree is cleared first
        // and left empty when the sweep fails.
        c.execute_tree(ClipType::Union, fr)
            .map(|(t, _)| t)
            .unwrap_or_default()
    }
}

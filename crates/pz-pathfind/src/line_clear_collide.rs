//! `LineClearCollide` and `LiangBarsky`: whether a straight walk between
//! two points on one level is unobstructed. Used for the direct-path
//! shortcut of `findPath` and by `smoothPath`.
//!
//! The shipped library's `LineClearCollide::isNotClear` is the old
//! square-based test (`isNotClearOld`); this is that, with the native
//! additions (farming-plant flag, vehicle sweep at `RADIUS`).

use crate::square::bits::*;
use crate::util::{fastfloor, Dir};
use crate::world::World;

/// `PolygonalMap2::RADIUS`.
pub const RADIUS: f32 = 0.3;
/// `PolygonalMap2::RADIUS_DIAGONAL` (`sqrt(0.18f)`).
pub const RADIUS_DIAGONAL: f32 = 0.424_264_07;

/// Scratch state of `LineClearCollide`.
#[derive(Default)]
pub struct LineClearCollide {
    pts: Vec<(i32, i32)>,
}

/// `LiangBarsky::lineRectIntersect`.
#[allow(clippy::too_many_arguments)]
pub fn line_rect_intersect(
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> bool {
    let p = [-(dx as f64), dx as f64, -(dy as f64), dy as f64];
    let q = [
        (x - left) as f64,
        (right - x) as f64,
        (y - top) as f64,
        (bottom - y) as f64,
    ];
    let mut u1 = 0.0f64;
    let mut u2 = 1.0f64;
    for i in 0..4 {
        if p[i] == 0.0 {
            if q[i] < 0.0 {
                return false;
            }
            continue;
        }
        let t = q[i] / p[i];
        if p[i] < 0.0 && u1 < t {
            u1 = t;
        } else if p[i] > 0.0 && u2 > t {
            u2 = t;
        }
    }
    u1 < u2
}

/// `PolygonalMap2::supercover`: every square touched by the segment,
/// appended to `pts` without duplicates.
pub(crate) fn supercover(x0: f32, y0: f32, x1: f32, y1: f32, pts: &mut Vec<(i32, i32)>) {
    let dy_abs = (y1 - y0).abs() as f64;
    let dx_abs = (x1 - x0).abs() as f64;
    let mut x = fastfloor(x0);
    let mut y = fastfloor(y0);
    let mut n: i32;
    let x_inc: i32;
    let y_inc: i32;
    let mut error: f64;
    if x1 - x0 == 0.0 {
        x_inc = 0;
        n = 1;
        error = f64::INFINITY;
    } else if x0 < x1 {
        x_inc = 1;
        n = fastfloor(x1) - x + 1;
        error = dy_abs * ((fastfloor(x0) + 1) as f32 - x0) as f64;
    } else {
        x_inc = -1;
        n = x - fastfloor(x1) + 1;
        error = dy_abs * (x0 - fastfloor(x0) as f32) as f64;
    }
    if y1 - y0 == 0.0 {
        y_inc = 0;
        error += f64::NEG_INFINITY;
    } else if y1 <= y0 {
        y_inc = -1;
        n = n + y - fastfloor(y1);
        let frac = (y0 - fastfloor(y0) as f32) as f64;
        error = (-frac).mul_add(dx_abs, error);
    } else {
        y_inc = 1;
        n = n - y + fastfloor(y1);
        let frac = ((fastfloor(y0) + 1) as f32 - y0) as f64;
        error = (-frac).mul_add(dx_abs, error);
    }
    while n > 0 {
        if !pts.contains(&(x, y)) {
            pts.push((x, y));
        }
        if error <= 0.0 {
            x += x_inc;
            error += dy_abs;
        } else {
            y += y_inc;
            error += -dx_abs;
        }
        n -= 1;
    }
}

/// The 0.3 margins of a wall test: `(w, n, e, s)`.
struct Margins {
    w: f32,
    n: f32,
    e: f32,
    s: f32,
}

impl Margins {
    fn full() -> Self {
        Margins {
            w: RADIUS,
            n: RADIUS,
            e: RADIUS,
            s: RADIUS,
        }
    }
}

/// `LineClearCollide::testCollisionVertical`.
#[allow(clippy::too_many_arguments)]
fn test_collision_vertical(
    fx: f32,
    fy: f32,
    tx: f32,
    ty: f32,
    px: f32,
    py: f32,
    skip_w: bool,
    skip_e: bool,
) -> bool {
    let mut m = Margins::full();
    if fx < px && tx < px {
        m.w = 0.0;
    } else if fx >= px && tx >= px {
        m.e = 0.0;
    }
    if fy < py && ty < py {
        m.n = 0.0;
    } else if fy >= py + 1.0 && ty >= py + 1.0 {
        m.s = 0.0;
    }
    if skip_w {
        m.w = 0.0;
    }
    if skip_e {
        m.e = 0.0;
    }
    line_rect_intersect(
        fx,
        fy,
        tx - fx,
        ty - fy,
        px - m.w,
        py - m.n,
        px + m.e,
        py + 1.0 + m.s,
    )
}

/// `LineClearCollide::testCollisionHorizontal`.
#[allow(clippy::too_many_arguments)]
fn test_collision_horizontal(
    fx: f32,
    fy: f32,
    tx: f32,
    ty: f32,
    px: f32,
    py: f32,
    skip_n: bool,
    skip_s: bool,
) -> bool {
    let mut m = Margins::full();
    if fx < px && tx < px {
        m.w = 0.0;
    } else if fx >= px + 1.0 && tx >= px + 1.0 {
        m.e = 0.0;
    }
    if fy < py && ty < py {
        m.n = 0.0;
    } else if fy >= py && ty >= py {
        m.s = 0.0;
    }
    if skip_n {
        m.n = 0.0;
    }
    if skip_s {
        m.s = 0.0;
    }
    line_rect_intersect(
        fx,
        fy,
        tx - fx,
        ty - fy,
        px - m.w,
        py - m.n,
        px + 1.0 + m.e,
        py + m.s,
    )
}

impl World {
    /// `LineClearCollide::canStandAt` (the `CLOSE_TO_WALLS` variant).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lcc_can_stand_at(
        &self,
        from_x: f32,
        from_y: f32,
        x: f32,
        y: f32,
        z: f32,
        ignore_vehicle: Option<crate::visibility::VehicleId>,
    ) -> bool {
        if (fastfloor(from_x) != fastfloor(x) || fastfloor(from_y) != fastfloor(y))
            && self.is_blocked_in_all_directions(fastfloor(x), fastfloor(y), fastfloor(z))
        {
            return false;
        }
        let min_x = fastfloor(x - RADIUS);
        let min_y = fastfloor(y - RADIUS);
        let max_x = (x + RADIUS).ceil() as i32;
        let max_y = (y + RADIUS).ceil() as i32;
        for sy in min_y..max_y {
            for sx in min_x..max_x {
                let target =
                    x >= sx as f32 && y >= sy as f32 && x < (sx + 1) as f32 && y < (sy + 1) as f32;
                let blocked = match self.get_square(sx, sy, fastfloor(z)) {
                    None => true,
                    Some(s) => {
                        let sq = self.sq(s);
                        sq.is_really_solid(self) || !sq.has(SOLID_FLOOR)
                    }
                };
                if blocked && target {
                    return false;
                }
            }
        }
        for &v in &self.vis.vehicles {
            if Some(v) == ignore_vehicle {
                continue;
            }
            let poly = &self.vis.vehicles_pool[v].poly_plus_radius;
            if fastfloor(poly.z) == fastfloor(z) && poly.contains_point(x, y) {
                return false;
            }
        }
        true
    }

    /// `LineClearCollide::isNotClear` (= `isNotClearOld`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lcc_is_not_clear(
        &mut self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        flags: i32,
    ) -> bool {
        if !self.lcc_can_stand_at(from_x, from_y, to_x, to_y, z as f32, None) {
            return true;
        }
        let dx = to_x - from_x;
        let dy = to_y - from_y;
        let len = dy.mul_add(dy, dx * dx).sqrt();
        let (p1x, p1y, p2x, p2y) = if len >= 0.001 {
            (dy / len, -dx / len, -dy / len, dx / len)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let rd = RADIUS_DIAGONAL;
        let mut pts = std::mem::take(&mut self.lcc.pts);
        pts.clear();
        pts.push((fastfloor(from_x), fastfloor(from_y)));
        if fastfloor(from_x) != fastfloor(to_x) || fastfloor(from_y) != fastfloor(to_y) {
            pts.push((fastfloor(to_x), fastfloor(to_y)));
        }
        supercover(
            p1x.mul_add(rd, from_x),
            p1y.mul_add(rd, from_y),
            p1x.mul_add(rd, to_x),
            p1y.mul_add(rd, to_y),
            &mut pts,
        );
        supercover(
            p2x.mul_add(rd, from_x),
            p2y.mul_add(rd, from_y),
            p2x.mul_add(rd, to_x),
            p2y.mul_add(rd, to_y),
            &mut pts,
        );
        let blocked = self.lcc_test_squares(&pts, from_x, from_y, to_x, to_y, z, flags);
        self.lcc.pts = pts;
        if blocked {
            return true;
        }
        self.lcc_test_vehicles(from_x, from_y, to_x, to_y, len, dx, dy, z)
    }

    #[allow(clippy::too_many_arguments)]
    fn lcc_test_squares(
        &self,
        pts: &[(i32, i32)],
        fx: f32,
        fy: f32,
        tx: f32,
        ty: f32,
        z: i32,
        flags: i32,
    ) -> bool {
        let (dx, dy) = (tx - fx, ty - fy);
        for &(px, py) in pts {
            let s = self.get_square(px, py, z);
            if flags & 4 != 0 {
                if let Some(s) = s {
                    if self.sq(s).cost > 0 {
                        return true;
                    }
                }
            }
            if flags & 0x20 != 0 {
                if let Some(s) = s {
                    if self.sq(s).has(FARMING_PLANT) {
                        return true;
                    }
                }
            }
            let (pxf, pyf) = (px as f32, py as f32);
            let s = match s {
                Some(s) if !self.sq(s).is_really_solid(self) && self.sq(s).has(SOLID_FLOOR) => s,
                _ => {
                    let mut m = Margins::full();
                    if fx < pxf && tx < pxf {
                        m.w = 0.0;
                    } else if fx >= (px + 1) as f32 && tx >= (px + 1) as f32 {
                        m.e = 0.0;
                    }
                    if fy < pyf && ty < pyf {
                        m.n = 0.0;
                    } else if fy >= (py + 1) as f32 && ty >= (py + 1) as f32 {
                        m.s = 0.0;
                    }
                    if line_rect_intersect(
                        fx,
                        fy,
                        dx,
                        dy,
                        pxf - m.w,
                        pyf - m.n,
                        pxf + 1.0 + m.e,
                        pyf + 1.0 + m.s,
                    ) {
                        return true;
                    }
                    continue;
                }
            };
            let sq = self.sq(s);
            if sq.has(ALL_STAIRS) {
                if sq.has(STAIR_TN | STAIR_MN | STAIR_BN) {
                    if test_collision_vertical(fx, fy, tx, ty, pxf, pyf, false, true) {
                        return true;
                    }
                    if test_collision_vertical(fx, fy, tx, ty, (px + 1) as f32, pyf, true, false) {
                        return true;
                    }
                    if sq.has(STAIR_TN)
                        && test_collision_horizontal(fx, fy, tx, ty, pxf, pyf, false, true)
                    {
                        return true;
                    }
                }
                if !sq.has(STAIR_TW | STAIR_MW | STAIR_BW) {
                    continue;
                }
                if test_collision_horizontal(fx, fy, tx, ty, pxf, pyf, false, true) {
                    return true;
                }
                if test_collision_horizontal(fx, fy, tx, ty, (px + 1) as f32, pyf, true, false) {
                    return true;
                }
                if sq.has(STAIR_TW)
                    && test_collision_vertical(fx, fy, tx, ty, pxf, pyf, false, true)
                {
                    return true;
                }
                continue;
            }
            if let Some(dir) = sq.slope_dir {
                let adj = |d: Dir| self.adjacent_square(s, d).map(|a| self.sq(a));
                if dir == Dir::N || dir == Dir::S {
                    let differs = |o: Option<&crate::square::Square>| {
                        o.map(|o| !sq.has_identical_sloped_surface(o))
                            .unwrap_or(true)
                    };
                    if differs(adj(Dir::W))
                        && test_collision_vertical(fx, fy, tx, ty, pxf, pyf, false, true)
                    {
                        return true;
                    }
                    if differs(adj(Dir::E))
                        && test_collision_vertical(
                            fx,
                            fy,
                            tx,
                            ty,
                            (px + 1) as f32,
                            pyf,
                            true,
                            false,
                        )
                    {
                        return true;
                    }
                    let n = adj(Dir::N);
                    if dir == Dir::N
                        && n.map(|n| sq.slope_max != n.sloped_surface_height_at(0.5, 1.0))
                            .unwrap_or(true)
                        && test_collision_horizontal(fx, fy, tx, ty, pxf, pyf, false, true)
                    {
                        return true;
                    }
                    let so = adj(Dir::S);
                    if dir == Dir::S
                        && so
                            .map(|so| sq.slope_max != so.sloped_surface_height_at(0.5, 0.0))
                            .unwrap_or(true)
                        && test_collision_horizontal(
                            fx,
                            fy,
                            tx,
                            ty,
                            pxf,
                            (py + 1) as f32,
                            true,
                            false,
                        )
                    {
                        return true;
                    }
                }
                if dir != Dir::W && dir != Dir::E {
                    continue;
                }
                let differs = |o: Option<&crate::square::Square>| {
                    o.map(|o| !sq.has_identical_sloped_surface(o))
                        .unwrap_or(true)
                };
                if differs(adj(Dir::N))
                    && test_collision_horizontal(fx, fy, tx, ty, pxf, pyf, false, true)
                {
                    return true;
                }
                if differs(adj(Dir::S))
                    && test_collision_horizontal(fx, fy, tx, ty, pxf, (py + 1) as f32, true, false)
                {
                    return true;
                }
                let w = adj(Dir::W);
                if dir == Dir::W
                    && w.map(|w| sq.slope_max != w.sloped_surface_height_at(1.0, 0.5))
                        .unwrap_or(true)
                    && test_collision_vertical(fx, fy, tx, ty, pxf, pyf, false, true)
                {
                    return true;
                }
                let e = adj(Dir::E);
                if dir == Dir::E
                    && e.map(|e| sq.slope_max != e.sloped_surface_height_at(0.0, 0.5))
                        .unwrap_or(true)
                    && test_collision_vertical(fx, fy, tx, ty, (px + 1) as f32, pyf, true, false)
                {
                    return true;
                }
                continue;
            }
            if sq.is_collide_w(self) {
                let mut m = Margins::full();
                if fx < pxf && tx < pxf {
                    m.w = 0.0;
                } else if fx >= pxf && tx >= pxf {
                    m.e = 0.0;
                }
                if fy < pyf && ty < pyf {
                    m.n = 0.0;
                } else if fy >= (py + 1) as f32 && ty >= (py + 1) as f32 {
                    m.s = 0.0;
                }
                if line_rect_intersect(
                    fx,
                    fy,
                    dx,
                    dy,
                    pxf - m.w,
                    pyf - m.n,
                    pxf + m.e,
                    pyf + 1.0 + m.s,
                ) {
                    return true;
                }
            }
            if !sq.is_collide_n(self) {
                continue;
            }
            let mut m = Margins::full();
            if fx < pxf && tx < pxf {
                m.w = 0.0;
            } else if fx >= (px + 1) as f32 && tx >= (px + 1) as f32 {
                m.e = 0.0;
            }
            if fy < pyf && ty < pyf {
                m.n = 0.0;
            } else if fy >= pyf && ty >= pyf {
                m.s = 0.0;
            }
            if line_rect_intersect(
                fx,
                fy,
                dx,
                dy,
                pxf - m.w,
                pyf - m.n,
                pxf + 1.0 + m.e,
                pyf + m.s,
            ) {
                return true;
            }
        }
        false
    }

    /// The vehicle sweep at the end of `isNotClearOld`.
    #[allow(clippy::too_many_arguments)]
    fn lcc_test_vehicles(
        &self,
        fx: f32,
        fy: f32,
        tx: f32,
        ty: f32,
        len: f32,
        dx: f32,
        dy: f32,
        z: i32,
    ) -> bool {
        if self.vis.vehicles.is_empty() {
            return false;
        }
        let r = RADIUS;
        let (p1x, p1y) = if len >= 0.001 {
            (dy / len, -dx / len)
        } else {
            (0.0, 0.0)
        };
        let x0 = p1x.mul_add(r, fx);
        let y0 = p1y.mul_add(r, fy);
        let x1 = p1x.mul_add(r, tx);
        let y1 = p1y.mul_add(r, ty);
        let (p2x, p2y) = if len >= 0.001 {
            (-dy / len, dx / len)
        } else {
            (0.0, 0.0)
        };
        let x2 = p2x.mul_add(r, fx);
        let y2 = p2y.mul_add(r, fy);
        let x3 = p2x.mul_add(r, tx);
        let y3 = p2y.mul_add(r, ty);
        let fmin = |a: f32, b: f32| if a < b { a } else { b };
        let fmax = |a: f32, b: f32| if b < a { a } else { b };
        let min_x = fmin(fmin(fmin(x3, x2), x1), x0);
        let min_y = fmin(fmin(fmin(y3, y2), y1), y0);
        let max_x = fmax(fmax(fmax(x2, x3), x1), x0);
        let max_y = fmax(fmax(fmax(y2, y3), y1), y0);
        let sweep_x = fastfloor(min_x);
        let sweep_y = fastfloor(min_y);
        let sweep_r = max_x.ceil() as i32;
        let sweep_b = max_y.ceil() as i32;
        let _ = z;
        let quad = [(x0, y0), (x1, y1), (x3, y3), (x2, y2)];
        for &v in &self.vis.vehicles {
            let poly = &self.vis.vehicles_pool[v].poly;
            let rect = poly.aabb();
            if rect.x < sweep_r
                && rect.x + rect.w > sweep_x
                && rect.y < sweep_b
                && rect.y + rect.h > sweep_y
                && poly.intersects_quad(&quad)
            {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supercover_line() {
        let mut pts = Vec::new();
        supercover(0.5, 0.5, 3.5, 0.5, &mut pts);
        assert_eq!(pts, vec![(0, 0), (1, 0), (2, 0), (3, 0)]);
        let mut pts = Vec::new();
        supercover(0.5, 0.5, 2.5, 2.5, &mut pts);
        assert_eq!(pts.first(), Some(&(0, 0)));
        assert_eq!(pts.last(), Some(&(2, 2)));
    }

    #[test]
    fn liang_barsky() {
        assert!(line_rect_intersect(0.0, 0.5, 3.0, 0.0, 1.0, 0.0, 2.0, 1.0));
        assert!(!line_rect_intersect(0.0, 2.5, 3.0, 0.0, 1.0, 0.0, 2.0, 1.0));
    }
}

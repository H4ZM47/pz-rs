//! `zombie.pathfind.LineClearCollideMain` and the `PolygonalMap2` methods
//! the main thread calls it through.

use pz_world::chunk::IsoGridSquare;
use pz_world::{EdgeFacing, GetSquare, IsoDirections};

use super::{ceil_i, fastfloor, normalize, MainThreadMap};
use crate::game_vehicle::{poly_contains_point, GameVehicle};
use crate::visibility::VehiclePoly;

/// Flag: ignore closed doors.
pub const FLAG_IGNORE_DOORS: i32 = 1;
/// Flag: only the target square must be free (`closeToWalls`).
pub const FLAG_CLOSE_TO_WALLS: i32 = 2;
/// Flag: squares with a pathfinding cost block.
pub const FLAG_CHECK_COST: i32 = 4;
/// Flag: debug rendering (no effect on results here).
pub const FLAG_RENDER: i32 = 8;

/// `PolygonalMap2.RADIUS_DIAGONAL`: `(float)Math.sqrt(0.18f)`.
fn radius_diagonal() -> f32 {
    (0.18f32 as f64).sqrt() as f32
}

/// `LiangBarsky.lineRectIntersect(x, y, dx, dy, left, top, right, bottom)`.
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
            continue;
        }
        if p[i] > 0.0 && u2 > t {
            u2 = t;
        }
    }
    u1 < u2
}

/// `PolygonalMap2.supercover(x0, y0, x1, y1, z, pool, pts)`: the squares a
/// line crosses, appended to `pts` unless already there.
pub fn supercover(x0: f32, y0: f32, x1: f32, y1: f32, pts: &mut Vec<(i32, i32)>) {
    let dx = (x1 - x0).abs() as f64;
    let dy = (y1 - y0).abs() as f64;
    let mut x = fastfloor(x0);
    let mut y = fastfloor(y0);
    let mut n = 1;
    let x_inc;
    let y_inc;
    let mut error;
    if dx == 0.0 {
        x_inc = 0;
        error = f64::INFINITY;
    } else if x1 > x0 {
        x_inc = 1;
        n += fastfloor(x1) - x;
        error = ((fastfloor(x0) + 1) as f32 - x0) as f64 * dy;
    } else {
        x_inc = -1;
        n += x - fastfloor(x1);
        error = (x0 - fastfloor(x0) as f32) as f64 * dy;
    }
    if dy == 0.0 {
        y_inc = 0;
        error -= f64::INFINITY;
    } else if y1 > y0 {
        y_inc = 1;
        n += fastfloor(y1) - y;
        error -= ((fastfloor(y0) + 1) as f32 - y0) as f64 * dx;
    } else {
        y_inc = -1;
        n += y - fastfloor(y1);
        error -= (y0 - fastfloor(y0) as f32) as f64 * dx;
    }
    while n > 0 {
        if !pts.contains(&(x, y)) {
            pts.push((x, y));
        }
        if error > 0.0 {
            y += y_inc;
            error -= dx;
        } else {
            x += x_inc;
            error += dy;
        }
        n -= 1;
    }
}

/// `PZMath.lineSegmentsIntersect(x1, y1, x2, y2, x3, y3, x4, y4)`.
#[allow(clippy::too_many_arguments)]
fn line_segments_intersect(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> bool {
    let d = (y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1);
    if d == 0.0 {
        return false;
    }
    let yd = y1 - y3;
    let xd = x1 - x3;
    let ua = ((x4 - x3) * yd - (y4 - y3) * xd) / d;
    if !(0.0..=1.0).contains(&ua) {
        return false;
    }
    let ub = ((x2 - x1) * yd - (y2 - y1) * xd) / d;
    (0.0..=1.0).contains(&ub)
}

/// `VehicleRect` of `VehiclePoly.getAABB` / the sweep box:
/// `(left, top, right, bottom)`.
type Rect = (i32, i32, i32, i32);

fn rect(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Rect {
    let (x, y) = (fastfloor(min_x), fastfloor(min_y));
    let (w, h) = (ceil_i(max_x) - x, ceil_i(max_y) - y);
    (x, y, x + w, y + h)
}

/// `VehicleRect.intersects(other)` (levels are not compared).
fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.0 < b.2 && a.2 > b.0 && a.1 < b.3 && a.3 > b.1
}

fn poly_aabb(p: &VehiclePoly) -> Rect {
    let min_x = p.x1.min(p.x2.min(p.x3.min(p.x4)));
    let min_y = p.y1.min(p.y2.min(p.y3.min(p.y4)));
    let max_x = p.x1.max(p.x2.max(p.x3.max(p.x4)));
    let max_y = p.y1.max(p.y2.max(p.y3.max(p.y4)));
    rect(min_x, min_y, max_x, max_y)
}

/// The closest point of `[lo, hi]` to `v` (`LineClearCollideMain.clamp`).
#[inline]
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    let mut f = v;
    if f < lo {
        f = lo;
    }
    if f > hi {
        f = hi;
    }
    f
}

/// Whether `(x, y)` is closer than the mover radius (0.3, compared
/// squared against 0.09) to `(cx, cy)`.
#[inline]
fn near(x: f32, y: f32, cx: f32, cy: f32) -> bool {
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy < 0.09f32
}

/// `LineClearCollideMain`: the main thread's straight-line collision tests
/// (see the module doc of [`super`]).
pub struct LineClearCollideMain<'m, 'w, 'a, G: GetSquare> {
    pub map: &'m MainThreadMap<'w, 'a, G>,
}

impl<'m, 'w, 'a: 'w, G: GetSquare> LineClearCollideMain<'m, 'w, 'a, G> {
    pub fn new(map: &'m MainThreadMap<'w, 'a, G>) -> Self {
        LineClearCollideMain { map }
    }

    /// `bHasFloor` of `canStandAtOld` / `isNotClearOld`: a solid floor, or
    /// an empty square over a sloped surface reaching above 0.9.
    fn has_floor(&self, square: Option<&IsoGridSquare>, x: i32, y: i32, z: i32) -> bool {
        let m = self.map;
        let mut has = square.is_some_and(|s| s.treat_as_solid_floor(m.sprites()));
        if square.is_none_or(|s| s.objects.is_empty()) {
            if let Some(below) = m.square(x, y, z - 1) {
                if below.get_sloped_surface_height_at(0.5, 0.5) > 0.9 {
                    has = true;
                }
            }
        }
        has
    }

    /// The blocking test shared by `canStandAtOld` and `isNotClearOld`.
    fn blocks(&self, square: Option<&IsoGridSquare>, has_floor: bool) -> bool {
        let m = self.map;
        match square {
            None => true,
            Some(s) => {
                s.is_solid(m.sprites())
                    || s.is_solid_trans(m.sprites())
                        && !s.is_adjacent_to_window(m.sprites(), m.squares(), m.hooks())
                        && !s.is_adjacent_to_hoppable(m.sprites(), m.squares(), m.hooks())
                    || !has_floor
            }
        }
    }

    /// `LineClearCollideMain.canStandAtOld(map, x, y, z, ignoreVehicle,
    /// flags)`: whether a mover of radius 0.3 fits at `(x, y)` on level
    /// `z`.
    pub fn can_stand_at_old(
        &self,
        x: f32,
        y: f32,
        z: f32,
        ignore_vehicle: Option<i16>,
        flags: i32,
    ) -> bool {
        let m = self.map;
        let ignore_doors = flags & FLAG_IGNORE_DOORS != 0;
        let close_to_walls = flags & FLAG_CLOSE_TO_WALLS != 0;
        let min_x = fastfloor(x - 0.3);
        let min_y = fastfloor(y - 0.3);
        let max_x = ceil_i(x + 0.3);
        let max_y = ceil_i(y + 0.3);
        let zi = fastfloor(z);
        for sy in min_y..max_y {
            for sx in min_x..max_x {
                let (fx, fy) = (sx as f32, sy as f32);
                let target = x >= fx && y >= fy && x < (sx + 1) as f32 && y < (sy + 1) as f32;
                let square = m.square(sx, sy, zi);
                let has_floor = self.has_floor(square, sx, sy, zi);
                let Some(sq) = square.filter(|_| !self.blocks(square, has_floor)) else {
                    let hit = if close_to_walls {
                        target
                    } else {
                        near(
                            x,
                            y,
                            clamp(x, fx, (sx + 1) as f32),
                            clamp(y, fy, (sy + 1) as f32),
                        )
                    };
                    if hit {
                        return false;
                    }
                    continue;
                };
                let edges = |blocked: &dyn Fn(IsoDirections) -> bool| {
                    let cx = clamp(x, fx, (sx + 1) as f32);
                    let cy = clamp(y, fy, (sy + 1) as f32);
                    blocked(IsoDirections::N) && near(x, y, cx, fy)
                        || blocked(IsoDirections::S) && near(x, y, cx, (sy + 1) as f32)
                        || blocked(IsoDirections::W) && near(x, y, fx, cy)
                        || blocked(IsoDirections::E) && near(x, y, (sx + 1) as f32, cy)
                };
                if sq.has_stairs(m.sprites()) {
                    if close_to_walls || target {
                        continue;
                    }
                    if edges(&|d| sq.is_stairs_edge_blocked(d, m.sprites(), m.squares())) {
                        return false;
                    }
                    continue;
                }
                if sq.has_sloped_surface() {
                    if close_to_walls || target {
                        continue;
                    }
                    if edges(&|d| sq.is_sloped_surface_edge_blocked(d, m.squares())) {
                        return false;
                    }
                    continue;
                }
                if close_to_walls {
                    continue;
                }
                let t = &m.bits.t;
                if (sq.has(t.collideW)
                    || !ignore_doors && sq.has_blocked_door(EdgeFacing::EastWest, m.hooks()))
                    && near(x, y, fx, clamp(y, fy, (sy + 1) as f32))
                {
                    return false;
                }
                if (sq.has(t.collideN)
                    || !ignore_doors && sq.has_blocked_door(EdgeFacing::NorthSouth, m.hooks()))
                    && near(x, y, clamp(x, fx, (sx + 1) as f32), fy)
                {
                    return false;
                }
            }
        }
        let chunk_min_x = (fastfloor(x) - 4) / 8 - 1;
        let chunk_min_y = (fastfloor(y) - 4) / 8 - 1;
        let chunk_max_x = ceil_i((x + 4.0) / 8.0) + 1;
        let chunk_max_y = ceil_i((y + 4.0) / 8.0) + 1;
        let mut blocked = false;
        for cy in chunk_min_y..chunk_max_y {
            for cx in chunk_min_x..chunk_max_x {
                m.vehicles
                    .for_each_vehicle_in_chunk(cx, cy, &mut |v: &dyn GameVehicle| {
                        if Some(v.vehicle_id()) == ignore_vehicle
                            || !v.added_to_world()
                            || fastfloor(v.z()) != zi
                            || !poly_contains_point(&v.poly_plus_radius(), x, y)
                        {
                            return false;
                        }
                        blocked = true;
                        true
                    });
                if blocked {
                    return false;
                }
            }
        }
        true
    }

    /// The four corners of the line swept `radius` to either side
    /// (`x0, y0, x1, y1` on one side, `x2, y2, x3, y3` on the other).
    fn sweep(from_x: f32, from_y: f32, to_x: f32, to_y: f32, radius: f32) -> [f32; 8] {
        let perp_x = to_y - from_y;
        let perp_y = -(to_x - from_x);
        let (px, py) = normalize(perp_x, perp_y);
        let x0 = from_x + px * radius;
        let y0 = from_y + py * radius;
        let x1 = to_x + px * radius;
        let y1 = to_y + py * radius;
        let (px, py) = normalize(-perp_x, -perp_y);
        let x2 = from_x + px * radius;
        let y2 = from_y + py * radius;
        let x3 = to_x + px * radius;
        let y3 = to_y + py * radius;
        [x0, y0, x1, y1, x2, y2, x3, y3]
    }

    /// The squares `isNotClearOld` and `getCollidepoint` test: the start
    /// and end squares, then the supercover of both sides of the swept
    /// line.
    fn candidate_squares(from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> Vec<(i32, i32)> {
        let [x0, y0, x1, y1, x2, y2, x3, y3] =
            Self::sweep(from_x, from_y, to_x, to_y, radius_diagonal());
        let mut pts = vec![(fastfloor(from_x), fastfloor(from_y))];
        if fastfloor(from_x) != fastfloor(to_x) || fastfloor(from_y) != fastfloor(to_y) {
            pts.push((fastfloor(to_x), fastfloor(to_y)));
        }
        supercover(x0, y0, x1, y1, &mut pts);
        supercover(x2, y2, x3, y3, &mut pts);
        pts
    }

    /// The square's box grown by 0.3 except on sides both ends are beyond:
    /// `(w, n, e, s)` margins for a full square at `pt`.
    fn margins_square(from: (f32, f32), to: (f32, f32), pt: (i32, i32)) -> (f32, f32, f32, f32) {
        let (mut w, mut n, mut e, mut s) = (0.3f32, 0.3f32, 0.3f32, 0.3f32);
        let (px, py) = (pt.0 as f32, pt.1 as f32);
        if from.0 < px && to.0 < px {
            w = 0.0;
        } else if from.0 >= (pt.0 + 1) as f32 && to.0 >= (pt.0 + 1) as f32 {
            e = 0.0;
        }
        if from.1 < py && to.1 < py {
            n = 0.0;
        } else if from.1 >= (pt.1 + 1) as f32 && to.1 >= (pt.1 + 1) as f32 {
            s = 0.0;
        }
        (w, n, e, s)
    }

    /// Margins for the west edge of the square at `pt`.
    fn margins_w(from: (f32, f32), to: (f32, f32), pt: (i32, i32)) -> (f32, f32, f32, f32) {
        let (mut w, mut n, mut e, mut s) = (0.3f32, 0.3f32, 0.3f32, 0.3f32);
        let (px, py) = (pt.0 as f32, pt.1 as f32);
        if from.0 < px && to.0 < px {
            w = 0.0;
        } else if from.0 >= px && to.0 >= px {
            e = 0.0;
        }
        if from.1 < py && to.1 < py {
            n = 0.0;
        } else if from.1 >= (pt.1 + 1) as f32 && to.1 >= (pt.1 + 1) as f32 {
            s = 0.0;
        }
        (w, n, e, s)
    }

    /// Margins for the north edge of the square at `pt`.
    fn margins_n(from: (f32, f32), to: (f32, f32), pt: (i32, i32)) -> (f32, f32, f32, f32) {
        let (mut w, mut n, mut e, mut s) = (0.3f32, 0.3f32, 0.3f32, 0.3f32);
        let (px, py) = (pt.0 as f32, pt.1 as f32);
        if from.0 < px && to.0 < px {
            w = 0.0;
        } else if from.0 >= (pt.0 + 1) as f32 && to.0 >= (pt.0 + 1) as f32 {
            e = 0.0;
        }
        if from.1 < py && to.1 < py {
            n = 0.0;
        } else if from.1 >= py && to.1 >= py {
            s = 0.0;
        }
        (w, n, e, s)
    }

    /// `testCollisionHorizontal(fromX, fromY, toX, toY, ptX, ptY, bSkipN,
    /// bSkipS, z, render)`: the line against the horizontal edge at `ptY`.
    #[allow(clippy::too_many_arguments)]
    pub fn test_collision_horizontal(
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        pt_x: f32,
        pt_y: f32,
        skip_n: bool,
        skip_s: bool,
    ) -> bool {
        let (mut w, mut n, mut e, mut s) = (0.3f32, 0.3f32, 0.3f32, 0.3f32);
        if from_x < pt_x && to_x < pt_x {
            w = 0.0;
        } else if from_x >= pt_x + 1.0 && to_x >= pt_x + 1.0 {
            e = 0.0;
        }
        if from_y < pt_y && to_y < pt_y {
            n = 0.0;
        } else if from_y >= pt_y && to_y >= pt_y {
            s = 0.0;
        }
        if skip_n {
            n = 0.0;
        }
        if skip_s {
            s = 0.0;
        }
        line_rect_intersect(
            from_x,
            from_y,
            to_x - from_x,
            to_y - from_y,
            pt_x - w,
            pt_y - n,
            pt_x + 1.0 + e,
            pt_y + s,
        )
    }

    /// `testCollisionVertical(...)`: the line against the vertical edge at
    /// `ptX`.
    #[allow(clippy::too_many_arguments)]
    pub fn test_collision_vertical(
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        pt_x: f32,
        pt_y: f32,
        skip_w: bool,
        skip_e: bool,
    ) -> bool {
        let (mut w, mut n, mut e, mut s) = (0.3f32, 0.3f32, 0.3f32, 0.3f32);
        if from_x < pt_x && to_x < pt_x {
            w = 0.0;
        } else if from_x >= pt_x && to_x >= pt_x {
            e = 0.0;
        }
        if from_y < pt_y && to_y < pt_y {
            n = 0.0;
        } else if from_y >= pt_y + 1.0 && to_y >= pt_y + 1.0 {
            s = 0.0;
        }
        if skip_w {
            w = 0.0;
        }
        if skip_e {
            e = 0.0;
        }
        line_rect_intersect(
            from_x,
            from_y,
            to_x - from_x,
            to_y - from_y,
            pt_x - w,
            pt_y - n,
            pt_x + e,
            pt_y + 1.0 + s,
        )
    }

    /// `LineClearCollideMain.isNotClearOld(map, fromX, fromY, toX, toY, z,
    /// ignoreVehicle, flags)` (what `isNotClear` runs): whether a mover of
    /// radius 0.3 walking straight from `from` to `to` on level `z` hits
    /// something.
    #[allow(clippy::too_many_arguments)]
    pub fn is_not_clear(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        ignore_vehicle: Option<i16>,
        flags: i32,
    ) -> bool {
        let m = self.map;
        let ignore_doors = flags & FLAG_IGNORE_DOORS != 0;
        let check_cost = flags & FLAG_CHECK_COST != 0;
        if !self.can_stand_at_old(to_x, to_y, z as f32, ignore_vehicle, flags) {
            return true;
        }
        let pts = Self::candidate_squares(from_x, from_y, to_x, to_y);
        let (from, to) = ((from_x, from_y), (to_x, to_y));
        let hit = |mg: (f32, f32, f32, f32), l: f32, t: f32, r: f32, b: f32| {
            line_rect_intersect(
                from_x,
                from_y,
                to_x - from_x,
                to_y - from_y,
                l - mg.0,
                t - mg.1,
                r + mg.2,
                b + mg.3,
            )
        };
        let horiz = Self::test_collision_horizontal;
        let vert = Self::test_collision_vertical;
        for &pt in &pts {
            let (px, py) = (pt.0 as f32, pt.1 as f32);
            let square = m.square(pt.0, pt.1, z);
            if check_cost {
                if let Some(s) = square {
                    if m.bits.get_cost(s) > 0 {
                        return true;
                    }
                }
            }
            let has_floor = self.has_floor(square, pt.0, pt.1, z);
            let Some(sq) = square.filter(|_| !self.blocks(square, has_floor)) else {
                let mg = Self::margins_square(from, to, pt);
                if hit(mg, px, py, px + 1.0, py + 1.0) {
                    return true;
                }
                continue;
            };
            if sq.has_stairs(m.sprites()) {
                let t = &m.bits.t;
                if sq.has_stairs_north(m.sprites())
                    && (vert(from_x, from_y, to_x, to_y, px, py, false, true)
                        || vert(from_x, from_y, to_x, to_y, px + 1.0, py, true, false)
                        || sq.has_type(t.stairsTN)
                            && horiz(from_x, from_y, to_x, to_y, px, py, false, true))
                {
                    return true;
                }
                if sq.has_stairs_west(m.sprites())
                    && (horiz(from_x, from_y, to_x, to_y, px, py, false, true)
                        || horiz(from_x, from_y, to_x, to_y, px, py + 1.0, true, false)
                        || sq.has_type(t.stairsTW)
                            && vert(from_x, from_y, to_x, to_y, px, py, false, true))
                {
                    return true;
                }
                continue;
            }
            if let Some(dir) = sq.sloped_surface_direction {
                let g = m.bits.squares;
                let adj = |d: IsoDirections| sq.get_adjacent_square(d, g);
                let differs =
                    |d: IsoDirections| adj(d).is_none_or(|a| !sq.has_identical_sloped_surface(a));
                let max = sq.get_sloped_surface_height_max();
                if dir == IsoDirections::N || dir == IsoDirections::S {
                    if differs(IsoDirections::W)
                        && vert(from_x, from_y, to_x, to_y, px, py, false, true)
                    {
                        return true;
                    }
                    if differs(IsoDirections::E)
                        && vert(from_x, from_y, to_x, to_y, px + 1.0, py, true, false)
                    {
                        return true;
                    }
                    if dir == IsoDirections::N
                        && adj(IsoDirections::N)
                            .is_none_or(|a| max != a.get_sloped_surface_height_at(0.5, 1.0))
                        && horiz(from_x, from_y, to_x, to_y, px, py, false, true)
                    {
                        return true;
                    }
                    if dir == IsoDirections::S
                        && adj(IsoDirections::S)
                            .is_none_or(|a| max != a.get_sloped_surface_height_at(0.5, 0.0))
                        && horiz(from_x, from_y, to_x, to_y, px, py + 1.0, true, false)
                    {
                        return true;
                    }
                }
                if dir != IsoDirections::W && dir != IsoDirections::E {
                    continue;
                }
                if differs(IsoDirections::N)
                    && horiz(from_x, from_y, to_x, to_y, px, py, false, true)
                {
                    return true;
                }
                if differs(IsoDirections::S)
                    && horiz(from_x, from_y, to_x, to_y, px, py + 1.0, true, false)
                {
                    return true;
                }
                if dir == IsoDirections::W
                    && adj(IsoDirections::W)
                        .is_none_or(|a| max != a.get_sloped_surface_height_at(1.0, 0.5))
                    && vert(from_x, from_y, to_x, to_y, px, py, false, true)
                {
                    return true;
                }
                if dir == IsoDirections::E
                    && adj(IsoDirections::E)
                        .is_none_or(|a| max != a.get_sloped_surface_height_at(0.0, 0.5))
                    && vert(from_x, from_y, to_x, to_y, px + 1.0, py, true, false)
                {
                    return true;
                }
                continue;
            }
            let t = &m.bits.t;
            if (sq.has(t.collideW)
                || !ignore_doors && sq.has_blocked_door(EdgeFacing::EastWest, m.hooks()))
                && hit(Self::margins_w(from, to, pt), px, py, px, py + 1.0)
            {
                return true;
            }
            if (sq.has(t.collideN)
                || !ignore_doors && sq.has_blocked_door(EdgeFacing::NorthSouth, m.hooks()))
                && hit(Self::margins_n(from, to, pt), px, py, px + 1.0, py)
            {
                return true;
            }
        }
        self.sweep_hits_vehicle(from_x, from_y, to_x, to_y, z, ignore_vehicle)
    }

    /// The vehicle part of `isNotClearOld`: the line swept 0.15 to either
    /// side against every registered vehicle's `getPoly()` in the chunks
    /// around it.
    fn sweep_hits_vehicle(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        _z: i32,
        ignore_vehicle: Option<i16>,
    ) -> bool {
        let [x0, y0, x1, y1, x2, y2, x3, y3] = Self::sweep(from_x, from_y, to_x, to_y, 0.15);
        let min_x = x0.min(x1.min(x2.min(x3)));
        let min_y = y0.min(y1.min(y2.min(y3)));
        let max_x = x0.max(x1.max(x2.max(x3)));
        let max_y = y0.max(y1.max(y2.max(y3)));
        let sweep_aabb = rect(min_x, min_y, max_x, max_y);
        let poly_vec = [(x0, y0), (x1, y1), (x3, y3), (x2, y2)];
        let chunk_min_x = sweep_aabb.0 / 8 - 1;
        let chunk_min_y = sweep_aabb.1 / 8 - 1;
        let chunk_max_x = ceil_i(sweep_aabb.2 as f32 / 8.0) + 1;
        let chunk_max_y = ceil_i(sweep_aabb.3 as f32 / 8.0) + 1;
        let mut collided = false;
        for cy in chunk_min_y..chunk_max_y {
            for cx in chunk_min_x..chunk_max_x {
                self.map
                    .vehicles
                    .for_each_vehicle_in_chunk(cx, cy, &mut |v: &dyn GameVehicle| {
                        if Some(v.vehicle_id()) == ignore_vehicle || v.vehicle_id() == -1 {
                            return false;
                        }
                        let poly = v.poly();
                        if rects_intersect(poly_aabb(&poly), sweep_aabb)
                            && poly_vehicle_intersect(&poly_vec, &poly)
                        {
                            collided = true;
                            return true;
                        }
                        false
                    });
                if collided {
                    return true;
                }
            }
        }
        false
    }

    /// `LineClearCollideMain.getCollidepoint(map, fromX, fromY, toX, toY,
    /// z, ignoreVehicle, flags)`: the first blocking square along the line
    /// (candidate squares sorted by Manhattan distance from the start), as
    /// a point to aim at; `to` when nothing blocks. Vehicles are not
    /// tested.
    ///
    /// The Java sort uses an inconsistent comparator
    /// (`fastfloor(d1 - d2)`); fewer than 32 candidates are ordered by the
    /// same binary insertion sort `java.util.TimSort` uses for short runs,
    /// so the order is exact there; longer lines (TimSort merging) use a
    /// stable sort with the same comparator, which can differ.
    #[allow(clippy::too_many_arguments)]
    pub fn get_collidepoint(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        _ignore_vehicle: Option<i16>,
        flags: i32,
    ) -> (f32, f32) {
        let m = self.map;
        let ignore_doors = flags & FLAG_IGNORE_DOORS != 0;
        let check_cost = flags & FLAG_CHECK_COST != 0;
        let mut pts = Self::candidate_squares(from_x, from_y, to_x, to_y);
        let manhattan = |p: &(i32, i32)| (p.0 as f32 - from_x).abs() + (p.1 as f32 - from_y).abs();
        java_list_sort(&mut pts, &|a, b| fastfloor(manhattan(a) - manhattan(b)));
        let (from, to) = ((from_x, from_y), (to_x, to_y));
        let hit = |mg: (f32, f32, f32, f32), l: f32, t: f32, r: f32, b: f32| {
            line_rect_intersect(
                from_x,
                from_y,
                to_x - from_x,
                to_y - from_y,
                l - mg.0,
                t - mg.1,
                r + mg.2,
                b + mg.3,
            )
        };
        for &pt in &pts {
            let (px, py) = (pt.0 as f32, pt.1 as f32);
            let square = m.square(pt.0, pt.1, z);
            if check_cost {
                if let Some(s) = square {
                    if m.bits.get_cost(s) > 0 {
                        return (px + 0.5, py + 0.5);
                    }
                }
            }
            let blocked = match square {
                None => true,
                Some(s) => {
                    s.is_solid(m.sprites())
                        || s.is_solid_trans(m.sprites())
                            && !s.is_adjacent_to_window(m.sprites(), m.squares(), m.hooks())
                            && !s.is_adjacent_to_hoppable(m.sprites(), m.squares(), m.hooks())
                        || s.has_stairs(m.sprites())
                        || !s.treat_as_solid_floor(m.sprites())
                }
            };
            if blocked {
                if hit(
                    Self::margins_square(from, to, pt),
                    px,
                    py,
                    px + 1.0,
                    py + 1.0,
                ) {
                    return (px + 0.5, py + 0.5);
                }
                continue;
            }
            let sq = square.unwrap();
            let t = &m.bits.t;
            if (sq.has(t.collideW)
                || !ignore_doors && sq.has_blocked_door(EdgeFacing::EastWest, m.hooks()))
                && hit(Self::margins_w(from, to, pt), px, py, px, py + 1.0)
            {
                let ox = if from_x - to_x < 0.0 { -0.5 } else { 0.5 };
                return (px + ox, py + 0.5);
            }
            if (sq.has(t.collideN)
                || !ignore_doors && sq.has_blocked_door(EdgeFacing::NorthSouth, m.hooks()))
                && hit(Self::margins_n(from, to, pt), px, py, px + 1.0, py)
            {
                let oy = if from_y - to_y < 0.0 { -0.5 } else { 0.5 };
                return (px + 0.5, py + oy);
            }
        }
        (to_x, to_y)
    }

    // ----- PolygonalMap2 -----

    /// `PolygonalMap2.canStandAt(x, y, z, ignoreVehicle, bIgnoreDoors,
    /// bCloseToWalls)`. `ignore_vehicle` is the vehicle to ignore: for a
    /// character, its `getVehicle()`.
    pub fn can_stand_at(
        &self,
        x: f32,
        y: f32,
        z: i32,
        ignore_vehicle: Option<i16>,
        ignore_doors: bool,
        close_to_walls: bool,
    ) -> bool {
        let mut flags = 0;
        if ignore_doors {
            flags |= FLAG_IGNORE_DOORS;
        }
        if close_to_walls {
            flags |= FLAG_CLOSE_TO_WALLS;
        }
        self.can_stand_at_old(x, y, z as f32, ignore_vehicle, flags)
    }

    /// `PolygonalMap2.lineClearCollide(fromX, fromY, toX, toY, z,
    /// ignoreVehicle, bIgnoreDoors, bCloseToWalls)`: **true when blocked**
    /// (the Java name notwithstanding). The shorter overloads pass
    /// `ignore_doors = close_to_walls = true`.
    #[allow(clippy::too_many_arguments)]
    pub fn line_clear_collide(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        ignore_vehicle: Option<i16>,
        ignore_doors: bool,
        close_to_walls: bool,
    ) -> bool {
        let mut flags = 0;
        if ignore_doors {
            flags |= FLAG_IGNORE_DOORS;
        }
        if close_to_walls {
            flags |= FLAG_CLOSE_TO_WALLS;
        }
        self.is_not_clear(from_x, from_y, to_x, to_y, z, ignore_vehicle, flags)
    }

    /// `PolygonalMap2.getApparentZ(square)`: the level plus the stair
    /// height (top 0.75, middle 0.5, bottom 0.25).
    pub fn get_apparent_z(&self, square: &IsoGridSquare) -> f32 {
        let t = &self.map.bits.t;
        if square.has_type(t.stairsTW) || square.has_type(t.stairsTN) {
            return square.z as f32 + 0.75;
        }
        if square.has_type(t.stairsMW) || square.has_type(t.stairsMN) {
            return square.z as f32 + 0.5;
        }
        if square.has_type(t.stairsBW) || square.has_type(t.stairsBN) {
            return square.z as f32 + 0.25;
        }
        square.z as f32
    }
}

/// `LineClearCollideMain.polyVehicleIntersect(poly, render)`: whether any
/// edge of the swept quad crosses an edge of the vehicle polygon.
fn poly_vehicle_intersect(poly_vec: &[(f32, f32); 4], p: &VehiclePoly) -> bool {
    let veh = [(p.x1, p.y1), (p.x2, p.y2), (p.x3, p.y3), (p.x4, p.y4)];
    for i in 0..4 {
        let a = poly_vec[i];
        let b = poly_vec[(i + 1) % 4];
        for j in 0..4 {
            let c = veh[j];
            let d = veh[(j + 1) % 4];
            if line_segments_intersect(a.0, a.1, b.0, b.1, c.0, c.1, d.0, d.1) {
                return true;
            }
        }
    }
    false
}

/// `PolygonalMap2.intersectLineWithVehicle(x1, y1, x2, y2, vehicle, out)`:
/// the crossing of the segment with the vehicle's `getPoly()` edges nearest
/// to `(x1, y1)`. As in Java, a vehicle edge parallel to the segment ends the
/// search with no result.
pub fn intersect_line_with_vehicle(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    vehicle: Option<&dyn GameVehicle>,
) -> Option<(f32, f32)> {
    let v = vehicle?;
    if !v.has_script() {
        return None;
    }
    let p = v.poly();
    let ff = [p.x1, p.y1, p.x2, p.y2, p.x3, p.y3, p.x4, p.y4];
    let mut nearest = f32::MAX;
    let mut out = None;
    for i in (0..8).step_by(2) {
        let y4 = ff[(i + 3) % 8];
        let y3 = ff[(i + 1) % 8];
        let x4 = ff[(i + 2) % 8];
        let x3 = ff[i % 8];
        let denom = ((y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1)) as f64;
        if denom == 0.0 {
            return None;
        }
        let ua = ((x4 - x3) * (y1 - y3) - (y4 - y3) * (x1 - x3)) as f64 / denom;
        let ub = ((x2 - x1) * (y1 - y3) - (y2 - y1) * (x1 - x3)) as f64 / denom;
        if !(0.0..=1.0).contains(&ua) || !(0.0..=1.0).contains(&ub) {
            continue;
        }
        let ix = (x1 as f64 + ua * (x2 - x1) as f64) as f32;
        let iy = (y1 as f64 + ua * (y2 - y1) as f64) as f32;
        let ddx = (ix - x1) as f64;
        let ddy = (iy - y1) as f64;
        let dist = (ddx * ddx + ddy * ddy).sqrt() as f32;
        if dist < nearest {
            out = Some((ix, iy));
            nearest = dist;
        }
    }
    if nearest < f32::MAX {
        out
    } else {
        None
    }
}

/// `List.sort(comparator)` (`java.util.TimSort`) for the short lists it
/// sorts with a binary insertion sort after extending the leading run
/// (`countRunAndMakeAscending`); longer lists fall back to a stable sort.
fn java_list_sort<T: Copy>(a: &mut [T], cmp: &dyn Fn(&T, &T) -> i32) {
    let n = a.len();
    if n < 2 {
        return;
    }
    if n >= 32 {
        a.sort_by(|x, y| cmp(x, y).cmp(&0));
        return;
    }
    // countRunAndMakeAscending
    let mut run_hi = 1;
    if cmp(&a[run_hi], &a[0]) < 0 {
        run_hi += 1;
        while run_hi < n && cmp(&a[run_hi], &a[run_hi - 1]) < 0 {
            run_hi += 1;
        }
        a[..run_hi].reverse();
    } else {
        run_hi += 1;
        while run_hi < n && cmp(&a[run_hi], &a[run_hi - 1]) >= 0 {
            run_hi += 1;
        }
    }
    // binarySort(a, lo, hi, start = lo + runLen)
    for start in run_hi..n {
        let pivot = a[start];
        let mut left = 0;
        let mut right = start;
        while left < right {
            let mid = (left + right) >> 1;
            if cmp(&pivot, &a[mid]) < 0 {
                right = mid;
            } else {
                left = mid + 1;
            }
        }
        a.copy_within(left..start, left + 1);
        a[left] = pivot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supercover_java() {
        let mut pts = Vec::new();
        supercover(0.5, 0.5, 3.5, 0.5, &mut pts);
        assert_eq!(pts, vec![(0, 0), (1, 0), (2, 0), (3, 0)]);
        let mut pts = vec![(1, 1)];
        supercover(0.5, 0.5, 1.5, 1.5, &mut pts);
        // Exactly diagonal: the error is 0, so x steps first.
        assert_eq!(pts, vec![(1, 1), (0, 0), (1, 0)]);
        let mut pts = Vec::new();
        supercover(2.5, 2.5, 2.5, 2.5, &mut pts);
        assert_eq!(pts, vec![(2, 2)]);
    }

    #[test]
    fn liang_barsky() {
        assert!(line_rect_intersect(0.0, 0.5, 3.0, 0.0, 1.0, 0.0, 2.0, 1.0));
        assert!(!line_rect_intersect(0.0, 2.5, 3.0, 0.0, 1.0, 0.0, 2.0, 1.0));
        // Touching an edge only is no intersection.
        assert!(!line_rect_intersect(0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 2.0, 1.0));
    }

    #[test]
    fn short_list_sort_matches_binary_insertion() {
        // fastfloor(a - b) on halves: 0.5 apart compares "equal" one way and
        // "less" the other; the result depends on the algorithm.
        let mut v = vec![1.5f32, 1.0, 0.5, 2.0, 0.0];
        java_list_sort(&mut v, &|a, b| super::super::fastfloor(a - b));
        assert_eq!(v, vec![0.0, 0.5, 1.0, 1.5, 2.0]);
        let mut v = vec![1.0f32, 1.5, 0.0];
        java_list_sort(&mut v, &|a, b| super::super::fastfloor(a - b));
        assert_eq!(v, vec![0.0, 1.0, 1.5]);
    }

    struct Box4(VehiclePoly);
    impl GameVehicle for Box4 {
        fn vehicle_id(&self) -> i16 {
            1
        }
        fn poly(&self) -> VehiclePoly {
            self.0
        }
        fn poly_plus_radius(&self) -> VehiclePoly {
            self.0
        }
        fn up_vector_dot(&self) -> f32 {
            1.0
        }
        fn crawl_offsets(&self) -> Vec<f32> {
            Vec::new()
        }
        fn z(&self) -> f32 {
            0.0
        }
        fn added_to_world(&self) -> bool {
            true
        }
        fn has_script(&self) -> bool {
            true
        }
    }

    #[test]
    fn vehicle_polygon_queries() {
        let p = VehiclePoly {
            x1: 2.0,
            y1: 2.0,
            x2: 4.0,
            y2: 2.0,
            x3: 4.0,
            y3: 6.0,
            x4: 2.0,
            y4: 6.0,
            z: 0.0,
        };
        assert!(crate::game_vehicle::poly_contains_point(&p, 3.0, 3.0));
        assert!(!crate::game_vehicle::poly_contains_point(&p, 5.0, 3.0));
        let v = Box4(p);
        assert_eq!(
            intersect_line_with_vehicle(0.0, 3.0, 10.0, 3.1, Some(&v)),
            Some((2.0, 3.02))
        );
        // A vehicle edge parallel to the line ends the search.
        assert_eq!(
            intersect_line_with_vehicle(0.0, 3.0, 10.0, 3.0, Some(&v)),
            None
        );
        assert!(poly_vehicle_intersect(
            &[(0.0, 2.9), (10.0, 2.9), (10.0, 3.1), (0.0, 3.1)],
            &p
        ));
    }
}

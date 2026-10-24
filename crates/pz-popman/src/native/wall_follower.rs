//! `WallFollower`: the library's square-by-square path walker (a "bug"
//! algorithm: walk the straight line, follow walls on contact, give up on
//! loops). Used by repopulation paths, `MapCollisionData.n_pathTask` and the
//! Last Stand virtual zombies.

use super::mcd::{Mcd, NativeMetaGrid, BIT_WALLN, BIT_WALLW, BLOCK_MASK};
use super::GameState;

pub const NONE: i32 = i32::MIN;

/// `WallFollower::update` results.
pub const CONTINUE: i32 = 0;
pub const BLOCKED: i32 = 1;
pub const LOOPED: i32 = 2;
pub const REACHED: i32 = 3;

const MAX_PATH: i32 = 89999;

#[derive(Clone, Default)]
pub struct WallFollower {
    pub start: (i32, i32),
    pub end: (i32, i32),
    /// Where the straight line hit a wall (`NONE` while walking the line).
    pub hit: (i32, i32),
    pub wall_steps: i32,
    pub hit_returns: i32,
    /// The wall square being followed.
    pub wall: (i32, i32),
    pub cur: (i32, i32),
    pub done: bool,
    pub failed: bool,
    /// Squares left behind.
    pub path: Vec<(i32, i32)>,
}

fn floor_f(v: f32) -> i32 {
    let t = v as i32;
    t - ((v < t as f32) as i32)
}

struct Ctx<'a> {
    mcd: &'a mut Mcd,
    gs: &'a GameState,
    meta: &'a NativeMetaGrid,
}

impl Ctx<'_> {
    fn blocked(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
        self.mcd.is_blocked(x1, y1, x2, y2, BLOCK_MASK, self.gs, self.meta)
    }

    fn bits(&mut self, x: i32, y: i32) -> u8 {
        self.mcd.get_bits(x, y, self.gs, self.meta)
    }
}

impl WallFollower {
    /// `initPath`.
    pub fn init_path(&mut self, sx: i32, sy: i32, ex: i32, ey: i32) {
        self.start = (sx, sy);
        self.end = (ex, ey);
        self.hit = (NONE, NONE);
        self.cur = (sx, sy);
        self.done = false;
        self.failed = false;
        self.path.clear();
    }

    fn push_cur(&mut self, x: i32) {
        self.path.push((x, self.cur.1));
    }

    /// `isPointOnLine(x, y)`: whether (x, y) lies on the line from the hit
    /// point to the end.
    fn is_point_on_line(&self, x: i32, y: i32) -> bool {
        let (hx, hy) = self.hit;
        let (ex, ey) = self.end;
        let ddy = ey.wrapping_sub(hy);
        let ddx = ex.wrapping_sub(hx);
        if ddx.unsigned_abs() > ddy.unsigned_abs() {
            let mut w8 = hx.wrapping_sub(ex);
            if w8 == 0 {
                return false;
            }
            let slope = ddy as f32 / ddx as f32;
            let sx = (ddx >> 31) | 1;
            let step = slope * sx as f32;
            let mut fy = hy as f32 + 0.5;
            let mut w10 = x.wrapping_sub(hx).wrapping_sub(sx);
            loop {
                fy += step;
                if w10 == 0 && floor_f(fy) == y {
                    return true;
                }
                w10 = w10.wrapping_sub(sx);
                w8 = w8.wrapping_add(sx);
                if w8 == 0 {
                    return false;
                }
            }
        } else {
            let slope = ddx as f32 / ddy as f32;
            let mut fx = hx as f32 + 0.5;
            let sy = (ddy >> 31) | 1;
            let step = slope * sy as f32;
            let mut w11 = y.wrapping_sub(hy).wrapping_sub(sy);
            let mut w9 = hy.wrapping_sub(ey);
            loop {
                let r = w9 != 0;
                if w9 == 0 {
                    return r;
                }
                fx += step;
                let hit = floor_f(fx) == x && w11 == 0;
                w11 = w11.wrapping_sub(sy);
                w9 = w9.wrapping_add(sy);
                if hit {
                    return r;
                }
            }
        }
    }

    /// `directStep`: one square along the straight line to the end.
    fn direct_step(&mut self, c: &mut Ctx) {
        let (cx, cy) = self.cur;
        let (ex, ey) = self.end;
        let dy = ey.wrapping_sub(cy);
        let dx = ex.wrapping_sub(cx);
        let (nx, ny);
        if dx.unsigned_abs() > dy.unsigned_abs() {
            if ex == cx {
                return;
            }
            let sx = (dx >> 31) | 1;
            let f = (cy as f32 + 0.5) + (dy as f32 / dx as f32) * sx as f32;
            nx = sx.wrapping_add(cx);
            ny = floor_f(f);
        } else {
            if ey == cy {
                return;
            }
            let sy = (dy >> 31) | 1;
            let f = (cx as f32 + 0.5) + (dx as f32 / dy as f32) * sy as f32;
            ny = sy.wrapping_add(cy);
            nx = floor_f(f);
        }
        if !c.blocked(cx, cy, nx, ny) {
            self.push_cur(cx);
            self.cur = (nx, ny);
            return;
        }
        self.hit = (cx, cy);
        self.wall_steps = 0;
        self.hit_returns = 0;
        self.wall = (nx, ny);
        if nx == cx || ny == cy {
            return;
        }
        let ddx = nx - cx;
        let ddy = ny - cy;
        if ddx == 1 && ddy == -1 {
            if !c.blocked(cx + 1, cy, nx, ny) {
                self.wall.0 -= 1;
            }
        } else if ddx == -1 && ddy == -1 {
            if !c.blocked(cx, cy - 1, nx, ny) {
                self.wall.1 += 1;
            }
        } else if ddx == -1 && ddy == 1 {
            if !c.blocked(cx - 1, cy, nx, ny) {
                self.wall.0 += 1;
            }
        } else if ddx == 1 && ddy == 1 && !c.blocked(cx, cy + 1, self.wall.0, ny) {
            self.wall.1 -= 1;
        }
    }

    /// `wallStep`: one square along the wall being followed.
    fn wall_step(&mut self, c: &mut Ctx) {
        let (tx, ty) = self.wall;
        let (cx, cy) = self.cur;
        let dx = tx.wrapping_sub(cx);
        let dy = ty.wrapping_sub(cy);
        let _ = c.blocked(cx, cy, tx, ty);
        if dx == 1 && dy == -1 {
            // East then north.
            if c.blocked(cx, cy, cx + 1, cy) {
                self.wall = (cx + 1, cy);
            } else {
                self.push_cur(cx);
                self.cur = (cx + 1, cy);
            }
        } else if dx == 1 && ty == cy {
            if c.blocked(cx, cy, cx, cy + 1) {
                self.wall = (cx, cy + 1);
                return;
            }
            if c.bits(cx + 1, cy) & BIT_WALLW != 0 && !c.blocked(cx, cy + 1, cx + 1, cy + 1) {
                self.wall = (cx, cy);
            }
            self.push_cur(cx);
            self.cur = (cx, cy + 1);
        } else if dx == 1 && dy == 1 {
            if c.blocked(cx, cy, cx, cy + 1) {
                self.wall = (cx, cy + 1);
            } else {
                self.push_cur(cx);
                self.cur = (cx, cy + 1);
            }
        } else if tx == cx && dy == 1 {
            if c.blocked(cx, cy, cx - 1, cy) {
                self.wall = (cx - 1, cy);
                return;
            }
            if c.bits(cx, cy + 1) & BIT_WALLN != 0 && !c.blocked(cx - 1, cy, cx - 1, cy + 1) {
                self.wall = (cx, cy);
            }
            self.push_cur(cx);
            self.cur = (cx - 1, cy);
        } else if dx == -1 && dy == 1 {
            if c.blocked(cx, cy, cx - 1, cy) {
                self.wall = (cx - 1, cy);
            } else {
                self.push_cur(cx);
                self.cur = (cx - 1, cy);
            }
        } else if dx == -1 && ty == cy {
            if c.blocked(cx, cy, cx, cy - 1) {
                self.wall = (cx, cy - 1);
                return;
            }
            if c.bits(cx, cy) & BIT_WALLW != 0 && !c.blocked(cx, cy - 1, cx - 1, cy - 1) {
                self.wall = (cx, cy);
            }
            self.push_cur(cx);
            self.cur = (cx, cy - 1);
        } else if dx == -1 && dy == -1 {
            if c.blocked(cx, cy, cx, cy - 1) {
                self.wall = (cx, cy - 1);
            } else {
                self.push_cur(cx);
                self.cur = (cx, cy - 1);
            }
        } else if tx == cx && dy == -1 {
            if c.blocked(cx, cy, cx + 1, cy) {
                self.wall = (cx + 1, cy);
                return;
            }
            if c.bits(cx, cy) & BIT_WALLN != 0 && !c.blocked(cx + 1, cy, cx + 1, cy - 1) {
                self.wall = (cx, cy);
            }
            self.push_cur(cx);
            self.cur = (cx + 1, cy);
        }
    }

    fn surrounded(&self, c: &mut Ctx) -> bool {
        let (x, y) = self.cur;
        c.blocked(x, y, x - 1, y) && c.blocked(x, y, x, y - 1) && c.blocked(x, y, x + 1, y) && c.blocked(x, y, x, y + 1)
    }

    /// `update`: one step; `CONTINUE` until the walk ends.
    pub fn update(&mut self, mcd: &mut Mcd, gs: &GameState, meta: &NativeMetaGrid) -> i32 {
        let mut c = Ctx { mcd, gs, meta };
        if self.done {
            return BLOCKED;
        }
        if self.failed {
            return LOOPED;
        }
        if c.bits(self.cur.0, self.cur.1) & BLOCK_MASK != 0 {
            return BLOCKED;
        }
        if self.cur == self.end {
            return REACHED;
        }
        let n = self.path.len() as i32;
        if n > MAX_PATH {
            return BLOCKED;
        }
        if self.hit.0 == NONE {
            self.direct_step(&mut c);
            if !self.path.is_empty() && self.cur == self.start {
                self.failed = true;
                return LOOPED;
            }
        } else {
            self.wall_step(&mut c);
            if n < self.path.len() as i32 {
                self.wall_steps += 1;
            }
            let (hx, hy) = self.hit;
            let (x, y) = self.cur;
            let mut ix = x;
            if x != hx || y != hy {
                ix = hx;
                if self.is_point_on_line(x, y) {
                    self.hit = (NONE, NONE);
                    ix = NONE;
                }
            }
            if self.wall_steps > 0 && x == ix && y == self.hit.1 {
                let r = self.hit_returns;
                self.hit_returns += 1;
                if r > 0 {
                    self.failed = true;
                    return LOOPED;
                }
            }
        }
        if c.bits(self.cur.0, self.cur.1) & BLOCK_MASK != 0 {
            return BLOCKED;
        }
        if self.surrounded(&mut c) {
            return BLOCKED;
        }
        if self.cur == self.end {
            return REACHED;
        }
        if self.path.len() as i32 > MAX_PATH {
            BLOCKED
        } else {
            CONTINUE
        }
    }
}

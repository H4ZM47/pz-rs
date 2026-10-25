//! `zombie.iso.LosUtil.lineClear`: whether vision passes in a straight line
//! between two squares, stepping square by square with
//! [`IsoGridSquare::test_vision_adjacent`].

use crate::chunk::IsoGridSquare;
use crate::java::fastfloor;
use crate::square::{GetSquare, VisionEnv};

/// `LosUtil.TestResults`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestResults {
    Clear,
    ClearThroughOpenDoor,
    ClearThroughWindow,
    Blocked,
    ClearThroughClosedDoor,
}

/// `LosUtil.lineClear(cell, x0, y0, z0, x1, y1, z1, bIgnoreDoors)` (with the
/// default `rangeTillWindows` of 10000).
#[allow(clippy::too_many_arguments)]
pub fn line_clear<G: GetSquare>(
    env: &VisionEnv<'_, G>,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    ignore_doors: bool,
) -> TestResults {
    line_clear_range(env, x0, y0, z0, x1, y1, z1, ignore_doors, 10000)
}

/// `LosUtil.lineClear(cell, x0, y0, z0, x1, y1, z1, bIgnoreDoors,
/// rangeTillWindows)`.
#[allow(clippy::too_many_arguments)]
pub fn line_clear_range<G: GetSquare>(
    env: &VisionEnv<'_, G>,
    mut x0: i32,
    mut y0: i32,
    mut z0: i32,
    x1: i32,
    y1: i32,
    mut z1: i32,
    ignore_doors: bool,
    range_till_windows: i32,
) -> TestResults {
    let g = env.squares;
    if z1 == z0 - 1 {
        if let Some(sq) = g.get_grid_square(x1, y1, z1) {
            if sq.has_elevated_floor(env.sprites) {
                z1 = z0;
            }
        }
    }
    let mut w = Walk {
        test: TestResults::Clear,
        b: g.get_grid_square(x0, y0, z0),
        dist: 0,
    };
    let mut dy = y1 - y0;
    let mut dx = x1 - x0;
    let mut dz = z1 - z0;
    let mut t = 0.5f32;
    let mut t2 = 0.5f32;
    if dx.abs() > dy.abs() && dx.abs() > dz.abs() {
        let mut m = dy as f32 / dx as f32;
        let mut m2 = dz as f32 / dx as f32;
        t += y0 as f32;
        t2 += z0 as f32;
        dx = if dx < 0 { -1 } else { 1 };
        m *= dx as f32;
        m2 *= dx as f32;
        while x0 != x1 {
            x0 += dx;
            t += m;
            t2 += m2;
            let a = g.get_grid_square(x0, fastfloor(t), fastfloor(t2));
            if let Some(r) = w.step(env, a, ignore_doors, range_till_windows) {
                return r;
            }
        }
    } else if dy.abs() >= dx.abs() && dy.abs() > dz.abs() {
        let mut m = dx as f32 / dy as f32;
        let mut m2 = dz as f32 / dy as f32;
        t += x0 as f32;
        t2 += z0 as f32;
        dy = if dy < 0 { -1 } else { 1 };
        m *= dy as f32;
        m2 *= dy as f32;
        while y0 != y1 {
            t += m;
            y0 += dy;
            t2 += m2;
            let a = g.get_grid_square(fastfloor(t), y0, fastfloor(t2));
            if let Some(r) = w.step(env, a, ignore_doors, range_till_windows) {
                return r;
            }
        }
    } else {
        let mut m = dx as f32 / dz as f32;
        let mut m2 = dy as f32 / dz as f32;
        t += x0 as f32;
        t2 += y0 as f32;
        dz = if dz < 0 { -1 } else { 1 };
        m *= dz as f32;
        m2 *= dz as f32;
        while z0 != z1 {
            t += m;
            t2 += m2;
            z0 += dz;
            let a = g.get_grid_square(fastfloor(t), fastfloor(t2), z0);
            if let Some(r) = w.step(env, a, ignore_doors, range_till_windows) {
                return r;
            }
        }
    }
    w.test
}

/// The walk state of `lineClear`: the result so far, the previous square
/// and the distance since the last window.
struct Walk<'s> {
    test: TestResults,
    b: Option<&'s IsoGridSquare>,
    dist: i32,
}

impl<'s> Walk<'s> {
    /// One step onto square `a`; `Some` ends the walk with that result.
    fn step<G: GetSquare>(
        &mut self,
        env: &VisionEnv<'_, G>,
        a: Option<&'s IsoGridSquare>,
        ignore_doors: bool,
        range_till_windows: i32,
    ) -> Option<TestResults> {
        if let (Some(a), Some(b)) = (a, self.b) {
            let new_test =
                a.test_vision_adjacent(b.x - a.x, b.y - a.y, b.z - a.z, true, ignore_doors, env);
            let window_change = new_test == TestResults::ClearThroughWindow;
            #[allow(clippy::if_same_then_else)] // the Java branches, kept apart
            if new_test == TestResults::Blocked
                || self.test == TestResults::Clear
                || new_test == TestResults::ClearThroughWindow
                    && self.test == TestResults::ClearThroughOpenDoor
            {
                self.test = new_test;
            } else if new_test == TestResults::ClearThroughClosedDoor
                && self.test == TestResults::ClearThroughOpenDoor
            {
                self.test = new_test;
            }
            if self.test == TestResults::Blocked {
                return Some(TestResults::Blocked);
            }
            if window_change {
                if self.dist > range_till_windows {
                    return Some(TestResults::Blocked);
                }
                self.dist = 0;
            }
        }
        self.b = a;
        self.dist += 1;
        None
    }
}

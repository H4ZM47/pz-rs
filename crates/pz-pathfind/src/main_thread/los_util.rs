//! `zombie.iso.LosUtil.lineClear`: whether vision passes in a straight line
//! between two squares, stepping square by square with
//! `IsoGridSquare.testVisionAdjacent`.

use pz_world::{GetSquare, IsoGridSquare};

use super::{fastfloor, MainThreadMap};

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
    map: &MainThreadMap<'_, '_, G>,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    ignore_doors: bool,
) -> TestResults {
    line_clear_range(map, x0, y0, z0, x1, y1, z1, ignore_doors, 10000)
}

/// `LosUtil.lineClear(cell, x0, y0, z0, x1, y1, z1, bIgnoreDoors,
/// rangeTillWindows)`.
#[allow(clippy::too_many_arguments)]
pub fn line_clear_range<G: GetSquare>(
    map: &MainThreadMap<'_, '_, G>,
    mut x0: i32,
    mut y0: i32,
    mut z0: i32,
    x1: i32,
    y1: i32,
    mut z1: i32,
    ignore_doors: bool,
    range_till_windows: i32,
) -> TestResults {
    if z1 == z0 - 1 {
        if let Some(sq) = map.square(x1, y1, z1) {
            if map.has_elevated_floor(sq) {
                z1 = z0;
            }
        }
    }
    let mut test = TestResults::Clear;
    let mut dy = y1 - y0;
    let mut dx = x1 - x0;
    let mut dz = z1 - z0;
    let mut t = 0.5f32;
    let mut t2 = 0.5f32;
    let mut b = map.square(x0, y0, z0);
    let mut dist = 0;

    let mut w = Walk {
        test: &mut test,
        b: &mut b,
        dist: &mut dist,
    };

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
            let a = map.square(x0, fastfloor(t), fastfloor(t2));
            if let Some(r) = w.step(map, a, ignore_doors, range_till_windows) {
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
            let a = map.square(fastfloor(t), y0, fastfloor(t2));
            if let Some(r) = w.step(map, a, ignore_doors, range_till_windows) {
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
            let a = map.square(fastfloor(t), fastfloor(t2), z0);
            if let Some(r) = w.step(map, a, ignore_doors, range_till_windows) {
                return r;
            }
        }
    }
    test
}

/// The walk state of `lineClear`: the result so far, the previous square
/// and the distance since the last window.
struct Walk<'r, 's> {
    test: &'r mut TestResults,
    b: &'r mut Option<&'s IsoGridSquare>,
    dist: &'r mut i32,
}

impl<'s> Walk<'_, 's> {
    /// One step onto square `a`; `Some` ends the walk with that result.
    fn step<G: GetSquare>(
        &mut self,
        map: &MainThreadMap<'_, '_, G>,
        a: Option<&'s IsoGridSquare>,
        ignore_doors: bool,
        range_till_windows: i32,
    ) -> Option<TestResults> {
        let mut window_change = false;
        if let (Some(a), Some(b)) = (a, *self.b) {
            let new_test =
                map.test_vision_adjacent(a, b.x - a.x, b.y - a.y, b.z - a.z, true, ignore_doors);
            if new_test == TestResults::ClearThroughWindow {
                window_change = true;
            }
            let test = &mut *self.test;
            #[allow(clippy::if_same_then_else)] // the Java branches, kept apart
            if new_test == TestResults::Blocked
                || *test == TestResults::Clear
                || new_test == TestResults::ClearThroughWindow
                    && *test == TestResults::ClearThroughOpenDoor
            {
                *test = new_test;
            } else if new_test == TestResults::ClearThroughClosedDoor
                && *test == TestResults::ClearThroughOpenDoor
            {
                *test = new_test;
            }
            if *test == TestResults::Blocked {
                return Some(TestResults::Blocked);
            }
            if window_change {
                if *self.dist > range_till_windows {
                    return Some(TestResults::Blocked);
                }
                *self.dist = 0;
            }
        }
        *self.b = a;
        *self.dist += 1;
        None
    }
}

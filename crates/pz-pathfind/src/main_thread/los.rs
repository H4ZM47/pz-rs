//! `zombie.iso.LosUtil.lineClearCollide`: the square-stepping collision
//! test (`CalculateCollide` between consecutive squares), as opposed to
//! the vision test `LosUtil.lineClear` ([`super::line_clear`]).

use pz_world::chunk::IsoGridSquare;
use pz_world::GetSquare;

use super::{fastfloor, MainThreadMap};

/// `LosUtil.lineClearCollide(x1, y1, z1, x0, y0, z0, bIgnoreDoors)`:
/// **true when blocked**. Steps square by square from `(x0, y0, z0)` (the
/// second triple, as in Java) towards `(x1, y1, z1)` and stops at the first
/// pair of squares `CalculateCollide(prev, false, false, true, true)`
/// separates, or (unless `ignore_doors`, and not on the vertical walk) that
/// a closed door separates. Missing squares are stepped over.
#[allow(clippy::too_many_arguments)]
pub fn los_line_clear_collide<G: GetSquare>(
    map: &MainThreadMap<'_, '_, G>,
    x1: i32,
    y1: i32,
    z1: i32,
    mut x0: i32,
    mut y0: i32,
    mut z0: i32,
    ignore_doors: bool,
) -> bool {
    let g = map.squares();
    let m = map.sprites();
    let mut dy = y1 - y0;
    let mut dx = x1 - x0;
    let mut dz = z1 - z0;
    let mut t = 0.5f32;
    let mut t2 = 0.5f32;
    let mut b: Option<&IsoGridSquare> = map.square(x0, y0, z0);
    let blocked = |a: &IsoGridSquare, b: &IsoGridSquare, doors: bool| {
        a.calculate_collide(Some(b), false, false, true, true, m, g)
            || doors && !ignore_doors && a.is_door_blocked_to(Some(b), g, map.hooks())
    };
    if dx.abs() > dy.abs() && dx.abs() > dz.abs() {
        let mut k = dy as f32 / dx as f32;
        let mut k2 = dz as f32 / dx as f32;
        t += y0 as f32;
        t2 += z0 as f32;
        dx = if dx < 0 { -1 } else { 1 };
        k *= dx as f32;
        k2 *= dx as f32;
        while x0 != x1 {
            x0 += dx;
            t += k;
            t2 += k2;
            let a = map.square(x0, fastfloor(t), fastfloor(t2));
            if let (Some(a), Some(b)) = (a, b) {
                if blocked(a, b, true) {
                    return true;
                }
            }
            b = a;
        }
    } else if dy.abs() >= dx.abs() && dy.abs() > dz.abs() {
        let mut k = dx as f32 / dy as f32;
        let mut k2 = dz as f32 / dy as f32;
        t += x0 as f32;
        t2 += z0 as f32;
        dy = if dy < 0 { -1 } else { 1 };
        k *= dy as f32;
        k2 *= dy as f32;
        while y0 != y1 {
            y0 += dy;
            t += k;
            t2 += k2;
            let a = map.square(fastfloor(t), y0, fastfloor(t2));
            if let (Some(a), Some(b)) = (a, b) {
                if blocked(a, b, true) {
                    return true;
                }
            }
            b = a;
        }
    } else {
        let mut k = dx as f32 / dz as f32;
        let mut k2 = dy as f32 / dz as f32;
        t += x0 as f32;
        t2 += y0 as f32;
        dz = if dz < 0 { -1 } else { 1 };
        k *= dz as f32;
        k2 *= dz as f32;
        while z0 != z1 {
            z0 += dz;
            t += k;
            t2 += k2;
            let a = map.square(fastfloor(t), fastfloor(t2), z0);
            if let (Some(a), Some(b)) = (a, b) {
                if blocked(a, b, false) {
                    return true;
                }
            }
            b = a;
        }
    }
    false
}

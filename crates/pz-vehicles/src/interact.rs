//! `IsoPlayer.getUseableVehicle` and `IsoPlayer.getNearVehicle`: the vehicle
//! a player could use or bump into, found by walking the chunks around the
//! player, plus the `BaseVehicle` queries they make (`getUseablePart`,
//! `getBestSeat`, `getTargetAlpha`, `PolygonalMap2.intersectLineWithVehicle`).
//!
//! The chunk lists are `IsoChunk.vehicles` of a pz-world cell. An entry
//! that chunk loading has not yet replaced with a runtime vehicle (a
//! [`LoadedVehicle`]) is a vehicle the original would test, and this port
//! cannot, so meeting one is an [`Unsupported`] error instead of "no
//! vehicle".

use crate::jmath::{dist_manhatten, fastfloor, Vector2};
use crate::part::PartRef;
use crate::vehicle::{BaseVehicle, VehicleRef};
use pz_world::chunk_vehicles::LoadedVehicle;
use pz_world::IsoCell;

/// What the search needs that the original reads from the `IsoPlayer`
/// (`IsoGameCharacter`): `getX/Y/Z`, `getForwardDirection`, `getVehicle`,
/// `isLocalPlayer` and `playerIndex`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interactor {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub forward: (f32, f32),
    /// `getVehicle() != null`.
    pub in_vehicle: bool,
    /// `isLocalPlayer()`.
    pub is_local_player: bool,
    /// `playerIndex`.
    pub player_index: usize,
}

/// A step the original takes that this port cannot yet: the answer would
/// need state the port does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl BaseVehicle {
    /// `getBestSeat(chr)`: this build's `BaseVehicle` answers -1 for every
    /// character.
    pub fn best_seat(&self, _chr: &Interactor) -> i32 {
        -1
    }

    /// `IsoObject.getTargetAlpha(playerIndex)`.
    pub fn target_alpha(&self, player_index: usize) -> f32 {
        self.st().target_alpha.get(player_index).copied().unwrap_or(0.0)
    }

    /// `PolygonalMap2.intersectLineWithVehicle(x1, y1, x2, y2, vehicle, out)`:
    /// the crossing of the segment with the polygon's edges nearest to
    /// `(x1, y1)`. As in the original, an edge parallel to the segment ends
    /// the search with no result, and a vehicle without a script has none.
    pub fn intersect_line_with_vehicle(
        &self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> Option<(f32, f32)> {
        self.script()?;
        let p = self.poly();
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

    /// `PolygonalMap2.instance.lineClearCollide(x1, y1, x2, y2, z, this,
    /// false, true)` from `chr`'s side: true when the line is blocked.
    fn line_blocked(&self, x1: f32, y1: f32, x2: f32, y2: f32, z: i32) -> Result<bool, Unsupported> {
        crate::env::world()
            .line_clear_collide_with(x1, y1, x2, y2, z, self, false, true)
            .ok_or_else(|| {
                Unsupported("PolygonalMap2.lineClearCollide: no pathfinding map for vehicles".into())
            })
    }

    /// `getUseablePart(chr, checkDir)`: the first part with a `use` Lua
    /// function whose area holds the character, that the character faces
    /// (when `check_dir`) and can reach in a straight line.
    ///
    /// Two exits in the original are `break`s that read like `continue`s:
    /// a part the character does not face, or one behind an obstacle, ends
    /// the search with no part.
    pub fn useable_part(&self, chr: &Interactor, check_dir: bool) -> Result<Option<PartRef>, Unsupported> {
        if chr.in_vehicle {
            return Ok(None);
        }
        if fastfloor(self.z()) != fastfloor(chr.z) {
            return Ok(None);
        }
        // `DistTo(IsoMovingObject)` is the Manhattan distance.
        if dist_manhatten(chr.x, chr.y, self.x(), self.y()) > 6.0 {
            return Ok(None);
        }
        let Some(script) = self.script() else {
            return Ok(None);
        };
        let (min_z, max_z) = {
            let s = script.borrow();
            let (ext, com) = (s.extents(), s.center_of_mass_offset());
            (com.z - ext.z / 2.0, com.z + ext.z / 2.0)
        };
        for part in self.parts() {
            let Some(area_id) = part.area() else { continue };
            if !self.is_in_area(&area_id, chr.x, chr.y) {
                continue;
            }
            match part.lua_function("use") {
                Some(f) if !f.is_empty() => {}
                _ => continue,
            }
            let Some(area) = self.area(&area_id) else { continue };
            let pos = self.area_position_local(&area);
            // An area beside the vehicle is reached along its x, one at
            // either end along its length (z).
            let (mut lx, mut lz) = (0.0, 0.0);
            if pos.y >= max_z {
                lx = pos.x;
            } else if !(pos.y <= min_z) {
                lz = pos.y;
            } else {
                lx = pos.x;
            }
            if !check_dir {
                return Ok(Some(part));
            }
            let w = self.world_pos(lx, 0.0, lz);
            let mut to_part = Vector2::new(w.x - chr.x, w.y - chr.y);
            to_part.normalize();
            if !(to_part.dot(chr.forward.0, chr.forward.1) > 0.5) {
                break;
            }
            if self.line_blocked(chr.x, chr.y, w.x, w.y, fastfloor(chr.z))? {
                break;
            }
            return Ok(Some(part));
        }
        Ok(None)
    }
}

/// The chunks `getUseableVehicle` and `getNearVehicle` walk: a window of
/// chunk coordinates around the player, `(x0, y0, x1, y1)` with the upper
/// bounds exclusive.
fn chunk_window(chr: &Interactor) -> (i32, i32, i32, i32) {
    (
        (fastfloor(chr.x) - 4) / 8 - 1,
        (fastfloor(chr.y) - 4) / 8 - 1,
        (((chr.x + 4.0) / 8.0) as f64).ceil() as i32 + 1,
        (((chr.y + 4.0) / 8.0) as f64).ceil() as i32 + 1,
    )
}

/// Calls `f` with each vehicle of the window's chunks (rows by `y`, then
/// `x`, as the original's loops go) until it answers `Some`.
fn find_in_window<T>(
    cell: &IsoCell,
    chr: &Interactor,
    mut f: impl FnMut(&VehicleRef) -> Result<Option<T>, Unsupported>,
) -> Result<Option<T>, Unsupported> {
    let (x0, y0, x1, y1) = chunk_window(chr);
    for cy in y0..y1 {
        for cx in x0..x1 {
            // `getChunk` is null for a chunk that is not loaded.
            let Some(list) = cell.chunk_vehicles(cx, cy) else {
                continue;
            };
            for handle in list.iter() {
                match handle.clone().downcast::<BaseVehicle>() {
                    Ok(v) => {
                        if let Some(found) = f(&v)? {
                            return Ok(Some(found));
                        }
                    }
                    Err(h) if h.is::<LoadedVehicle>() => {
                        return Err(Unsupported(format!(
                            "IsoChunk.vehicles({cx}, {cy}) holds a vehicle that is not yet a runtime BaseVehicle"
                        )));
                    }
                    Err(_) => {}
                }
            }
        }
    }
    Ok(None)
}

/// `IsoPlayer.getUseableVehicle()`: the first vehicle with a usable part
/// in reach. (The original goes on to a "best seat" vehicle when none has
/// a part, but `BaseVehicle.getBestSeat` is always -1 in this build, so
/// that branch is never taken.)
pub fn get_useable_vehicle(cell: &IsoCell, chr: &Interactor) -> Result<Option<VehicleRef>, Unsupported> {
    if chr.in_vehicle {
        return Ok(None);
    }
    find_in_window(cell, chr, |v| {
        Ok(v.useable_part(chr, true)?.map(|_| v.clone()))
    })
}

/// `IsoPlayer.getNearVehicle()`: the first vehicle on the player's level,
/// not hidden from the player, within four squares, that the player's
/// forward line crosses and can reach.
pub fn get_near_vehicle(cell: &IsoCell, chr: &Interactor) -> Result<Option<VehicleRef>, Unsupported> {
    if chr.in_vehicle {
        return Ok(None);
    }
    find_in_window(cell, chr, |v| {
        if v.script().is_none() || fastfloor(chr.z) != fastfloor(v.z()) {
            return Ok(None);
        }
        if chr.is_local_player && v.target_alpha(chr.player_index) == 0.0 {
            return Ok(None);
        }
        // `DistToSquared(vehicle)`: from the closest point of its polygon.
        let (d, _) = v.closest_point_on_poly(chr.x, chr.y);
        if d >= 16.0 {
            return Ok(None);
        }
        let (fx, fy) = chr.forward;
        let Some((ix, iy)) = v.intersect_line_with_vehicle(
            chr.x,
            chr.y,
            chr.x + fx * 4.0,
            chr.y + fy * 4.0,
        ) else {
            return Ok(None);
        };
        if v.line_blocked(chr.x, chr.y, ix, iy, fastfloor(chr.z))? {
            return Ok(None);
        }
        Ok(Some(v.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> Interactor {
        Interactor {
            x,
            y,
            z: 0.0,
            forward: (1.0, 0.0),
            in_vehicle: false,
            is_local_player: true,
            player_index: 0,
        }
    }

    #[test]
    fn the_window_is_the_chunks_within_four_squares_plus_one() {
        // Inside a chunk: (84 - 4) / 8 - 1 = 9 up to ceil(88.5 / 8) + 1 = 13.
        assert_eq!(chunk_window(&at(84.5, 84.5)), (9, 9, 13, 13));
        // `idiv` truncates toward zero, so west of the origin the lower
        // bound is not the floor.
        assert_eq!(chunk_window(&at(-0.5, 2.0)), (-1, -1, 2, 2));
    }

    #[test]
    fn best_seat_is_minus_one() {
        assert_eq!(BaseVehicle::new().best_seat(&at(0.0, 0.0)), -1);
    }

    #[test]
    fn target_alpha_starts_at_one_per_local_player() {
        let v = BaseVehicle::new();
        assert_eq!(v.target_alpha(3), 1.0);
        assert_eq!(v.target_alpha(4), 0.0);
    }
}

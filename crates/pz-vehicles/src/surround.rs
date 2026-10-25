//! `zombie.vehicles.SurroundVehicle`: the spots around a vehicle zombies
//! attacking its passengers stand at, each held by at most one zombie.
//!
//! Zombies are seen through [`SurroundZombie`] snapshots and named by a
//! key; the occupiers' current state is read back through a lookup when
//! the original reads it. `PolygonalMap2.lineClearCollide` / `canStandAt`
//! (the blocked check) come from [`crate::env::VehicleWorld`].

use crate::env;
use crate::jmath::{dist_sq, fastfloor, Vector3f};
use crate::vehicle::BaseVehicle;

/// What `SurroundVehicle` reads of an `IsoZombie` (and its target).
#[derive(Clone, Debug, PartialEq)]
pub struct SurroundZombie {
    /// Identity (the original compares objects).
    pub key: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `isOnFloor()`.
    pub on_floor: bool,
    /// `isCanWalk()`.
    pub can_walk: bool,
    /// `isDead()`.
    pub dead: bool,
    /// `VirtualZombieManager.instance.isReused(zombie)`.
    pub reused: bool,
    /// `getTarget()`: its position, and `vehicle.getSeat(target)` when it
    /// is a character (`-1` otherwise).
    pub target: Option<SurroundTarget>,
}

/// A zombie's target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurroundTarget {
    pub x: f32,
    pub y: f32,
    /// `vehicle.getSeat(target)` (-1 when not in this vehicle or not a
    /// character).
    pub seat: i32,
}

/// `SurroundVehicle.PositionSide`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionSide {
    Front,
    Rear,
    Left,
    Right,
}

/// `SurroundVehicle.Position`.
#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub pos_local: (f32, f32),
    pub pos_world: Vector3f,
    pub pos_axis: Vector3f,
    pub side: PositionSide,
    /// The occupying zombie's key.
    pub zombie: Option<u64>,
    pub target_x: f32,
    pub target_y: f32,
    pub blocked: bool,
}

/// `SurroundVehicle` state (one per vehicle).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurroundVehicle {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub x3: f32,
    pub y3: f32,
    pub x4: f32,
    pub y4: f32,
    p: [f32; 8],
    moved: bool,
    pub positions: Vec<Position>,
    update_ms: i64,
}

impl SurroundVehicle {
    /// `reset()`.
    pub fn reset(&mut self) {
        self.positions.clear();
    }

    /// `calcPositionsLocal()`.
    fn calc_positions_local(&mut self, v: &BaseVehicle) {
        self.positions.clear();
        let Some(script) = v.script() else { return };
        let (ext, com) = {
            let s = script.borrow();
            let e = s.extents();
            let c = s.center_of_mass_offset();
            ((e.x, e.y, e.z), (c.x, c.y, c.z))
        };
        let width = ext.0;
        let length = ext.2;
        let min_x = com.0 - width / 2.0 - 0.155;
        let min_y = com.2 - length / 2.0 - 0.155;
        let max_x = com.0 + width / 2.0 + 0.155;
        let max_y = com.2 + length / 2.0 + 0.155;
        let half = length / 2.0;
        self.add_positions(
            v,
            min_x,
            com.2 - half,
            min_x,
            com.2 + half,
            PositionSide::Right,
        );
        self.add_positions(
            v,
            max_x,
            com.2 - half,
            max_x,
            com.2 + half,
            PositionSide::Left,
        );
        self.add_positions(v, min_x, min_y, max_x, min_y, PositionSide::Rear);
        self.add_positions(v, min_x, max_y, max_x, max_y, PositionSide::Front);
    }

    /// `addPositions(x1, y1, x2, y2, side)`.
    fn add_positions(
        &mut self,
        v: &BaseVehicle,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        side: PositionSide,
    ) {
        let Some(passenger) = passenger_local_pos(v, 0) else {
            return;
        };
        if matches!(side, PositionSide::Left | PositionSide::Right) {
            let target_y = passenger.z;
            let mut y = target_y;
            while y >= y1 + 0.3 {
                self.add_position(x1, y, side);
                y -= 0.6;
            }
            let mut y = target_y + 0.6;
            while y < y2 - 0.3 {
                self.add_position(x1, y, side);
                y += 0.6;
            }
        } else {
            let mut x = 0.0f32;
            while x >= x1 + 0.3 {
                self.add_position(x, y1, side);
                x -= 0.6;
            }
            let mut x = 0.6f32;
            while x < x2 - 0.3 {
                self.add_position(x, y1, side);
                x += 0.6;
            }
        }
    }

    fn add_position(&mut self, x: f32, y: f32, side: PositionSide) {
        self.positions.push(Position {
            pos_local: (x, y),
            pos_world: Vector3f::default(),
            pos_axis: Vector3f::default(),
            side,
            zombie: None,
            target_x: 0.0,
            target_y: 0.0,
            blocked: false,
        });
    }

    /// `calcPositionsWorld()`.
    fn calc_positions_world(&mut self, v: &BaseVehicle) {
        for p in &mut self.positions {
            p.pos_world = v.world_pos(p.pos_local.0, 0.0, p.pos_local.1);
            p.pos_axis = match p.side {
                PositionSide::Front | PositionSide::Rear => v.world_pos(p.pos_local.0, 0.0, 0.0),
                PositionSide::Left | PositionSide::Right => v.world_pos(0.0, 0.0, p.pos_local.1),
            };
        }
        let poly = v.poly();
        self.p = [
            poly.x1, poly.x2, poly.x3, poly.x4, poly.y1, poly.y2, poly.y3, poly.y4,
        ];
    }

    /// `getClosestPositionFor(zombie)`.
    fn closest_position_for(
        &self,
        z: &SurroundZombie,
        lookup: &dyn Fn(u64) -> Option<SurroundZombie>,
    ) -> Option<usize> {
        let target = z.target?;
        let mut closest = f32::MAX;
        let mut best = None;
        for (i, p) in self.positions.iter().enumerate() {
            if p.blocked {
                continue;
            }
            let mover = dist_sq(z.x, z.y, p.pos_world.x, p.pos_world.y);
            if let Some(o) = p.zombie.and_then(lookup) {
                if dist_sq(o.x, o.y, p.pos_world.x, p.pos_world.y) < mover {
                    continue;
                }
            }
            let d = dist_sq(target.x, target.y, p.pos_world.x, p.pos_world.y);
            if !(d < closest) {
                continue;
            }
            closest = d;
            best = Some(i);
        }
        best
    }

    /// `getPositionForZombie(zombie, out)`: where `z` should stand, or
    /// `None` when every spot is taken or blocked.
    pub fn position_for_zombie(
        &mut self,
        v: &BaseVehicle,
        z: &SurroundZombie,
        lookup: &dyn Fn(u64) -> Option<SurroundZombie>,
    ) -> Option<(f32, f32)> {
        if (z.on_floor && !z.can_walk) || fastfloor(z.z) != fastfloor(v.z()) {
            return Some((v.x(), v.y()));
        }
        if dist_sq(z.x, z.y, v.x(), v.y()) > 100.0 {
            return Some((v.x(), v.y()));
        }
        if self.check_position(v) {
            self.moved = true;
        }
        for p in &mut self.positions {
            if p.blocked {
                p.zombie = None;
            }
            if p.zombie == Some(z.key) {
                return Some((p.pos_world.x, p.pos_world.y));
            }
        }
        let i = self.closest_position_for(z, lookup)?;
        let t = z.target?;
        let p = &mut self.positions[i];
        p.zombie = Some(z.key);
        p.target_x = t.x;
        p.target_y = t.y;
        Some((p.pos_world.x, p.pos_world.y))
    }

    /// `checkPosition()`: whether the polygon moved (recomputing the spots).
    fn check_position(&mut self, v: &BaseVehicle) -> bool {
        if v.script().is_none() {
            return false;
        }
        if self.positions.is_empty() {
            self.calc_positions_local(v);
            self.x1 = -1.0;
        }
        let poly = v.poly();
        if self.x1 != poly.x1
            || self.x2 != poly.x2
            || self.x3 != poly.x3
            || self.x4 != poly.x4
            || self.y1 != poly.y1
            || self.y2 != poly.y2
            || self.y3 != poly.y3
            || self.y4 != poly.y4
        {
            self.x1 = poly.x1;
            self.x2 = poly.x2;
            self.x3 = poly.x3;
            self.x4 = poly.x4;
            self.y1 = poly.y1;
            self.y2 = poly.y2;
            self.y3 = poly.y3;
            self.y4 = poly.y4;
            self.calc_positions_world(v);
            return true;
        }
        false
    }

    fn moved_since_positions_were_calculated(&self, v: &BaseVehicle) -> bool {
        let poly = v.poly();
        self.p
            != [
                poly.x1, poly.x2, poly.x3, poly.x4, poly.y1, poly.y2, poly.y3, poly.y4,
            ]
    }

    fn has_occupied_positions(&self) -> bool {
        self.positions.iter().any(|p| p.zombie.is_some())
    }

    /// `update()` (called while the vehicle has passengers).
    pub fn update(&mut self, v: &BaseVehicle, lookup: &dyn Fn(u64) -> Option<SurroundZombie>) {
        if self.has_occupied_positions() && self.check_position(v) {
            self.moved = true;
        }
        let now = env::now_ms();
        if now - self.update_ms < 1000 {
            return;
        }
        self.update_ms = now;
        if self.moved {
            self.moved = false;
            for p in &mut self.positions {
                p.zombie = None;
            }
        }
        let moved = self.moved_since_positions_were_calculated(v);
        let world = env::world();
        let level = fastfloor(v.z());
        let (vx, vy) = (v.x(), v.y());
        for p in &mut self.positions {
            if !moved {
                p.blocked = world.line_clear_collide(
                    p.pos_world.x,
                    p.pos_world.y,
                    p.pos_axis.x,
                    p.pos_axis.y,
                    level,
                    v,
                );
                if !p.blocked {
                    p.blocked = !world.can_stand_at(p.pos_world.x, p.pos_world.y, level, v);
                }
            }
            let Some(key) = p.zombie else { continue };
            let Some(z) = lookup(key) else {
                p.zombie = None;
                continue;
            };
            if dist_sq(z.x, z.y, vx, vy) > 100.0 {
                p.zombie = None;
                continue;
            }
            let target = z.target.filter(|t| t.seat != -1);
            let Some(t) = target.filter(|_| !z.dead && !z.reused && !z.on_floor) else {
                p.zombie = None;
                continue;
            };
            if dist_sq(p.target_x, p.target_y, t.x, t.y) > 0.1 {
                p.zombie = None;
            }
        }
    }
}

/// `getPassengerLocalPos(seat, out)`: the model offset plus the seat's
/// offset.
pub fn passenger_local_pos(v: &BaseVehicle, seat: i32) -> Option<Vector3f> {
    let off = v.st().passengers.get(seat as usize)?.offset;
    let m = v.script().and_then(|s| {
        s.borrow()
            .model()
            .map(|m| (m.offset.x, m.offset.y, m.offset.z))
    })?;
    Some(Vector3f::new(m.0 + off.x, m.1 + off.y, m.2 + off.z))
}

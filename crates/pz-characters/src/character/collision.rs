//! `IsoMovingObject.postupdate`'s movement and collision: the per-frame
//! collision flag resets, impulses, the obstacle clip
//! (`PolygonalMap2.resolveCollision`, pz-pathfind's `CollideWithObstacles`
//! through [`StateWorld::resolve_collision`]), the square-edge collision
//! passes (`DoCollide(1)` / `DoCollide(2)`, `Collided()`), the movement
//! modifier and the move itself, then the current square
//! (`findCurrentGridSquare`).
//!
//! The squares are the world's: `testCollideAdjacent` and the square
//! lookups go through [`StateWorld`]. Not ported here (the world's):
//! stairs and sloped surfaces (`doStairs`, `handleSlopedSurface`),
//! `checkHitWall` and the hoppable checks (reported through
//! [`StateWorld::collided_with_hoppables`]), `slideHeadAwayFromWalls`, and
//! zombie virtualization at the edge of the loaded area
//! ([`StateWorld::virtualize_if_unloaded`]).
//!
//! [`StateWorld`]: super::state::StateWorld
//! [`StateWorld::resolve_collision`]: super::state::StateWorld::resolve_collision
//! [`StateWorld::collided_with_hoppables`]: super::state::StateWorld::collided_with_hoppables
//! [`StateWorld::virtualize_if_unloaded`]: super::state::StateWorld::virtualize_if_unloaded

use super::state::StateCx;
use super::states::ids;
use super::GameCharacter;

/// `PZMath.fastfloor(float)`.
fn ff(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

/// `java.awt.geom.Line2D.relativeCCW`.
fn relative_ccw(x1: f64, y1: f64, mut x2: f64, mut y2: f64, mut px: f64, mut py: f64) -> i32 {
    x2 -= x1;
    y2 -= y1;
    px -= x1;
    py -= y1;
    let mut ccw = px * y2 - py * x2;
    if ccw == 0.0 {
        ccw = px * x2 + py * y2;
        if ccw > 0.0 {
            px -= x2;
            py -= y2;
            ccw = px * x2 + py * y2;
            if ccw < 0.0 {
                ccw = 0.0;
            }
        }
    }
    if ccw < 0.0 {
        -1
    } else if ccw > 0.0 {
        1
    } else {
        0
    }
}

/// `Line2D.linesIntersect`.
#[allow(clippy::too_many_arguments)]
pub fn lines_intersect(x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64, x4: f64, y4: f64) -> bool {
    relative_ccw(x1, y1, x2, y2, x3, y3) * relative_ccw(x1, y1, x2, y2, x4, y4) <= 0
        && relative_ccw(x3, y3, x4, y4, x1, y1) * relative_ccw(x3, y3, x4, y4, x2, y2) <= 0
}

/// `Path.crossesSquare(x, y, z)` (node levels in game terms).
pub fn path_crosses_square(path: &pz_pathfind::Path, x: i32, y: i32, z: i32) -> bool {
    let n = &path.nodes;
    for i in 0..n.len().saturating_sub(1) {
        let (a, b) = (&n[i], &n[i + 1]);
        if ff(a.level()) != z && ff(b.level()) != z {
            continue;
        }
        let (ax, ay, bx, by) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64);
        let (x, y) = (x as f64, y as f64);
        if lines_intersect(ax, ay, bx, by, x, y, x + 1.0, y)
            || lines_intersect(ax, ay, bx, by, x + 1.0, y, x + 1.0, y + 1.0)
            || lines_intersect(ax, ay, bx, by, x + 1.0, y + 1.0, x, y + 1.0)
            || lines_intersect(ax, ay, bx, by, x, y + 1.0, x, y)
        {
            return true;
        }
    }
    false
}

impl<I: 'static> GameCharacter<I> {
    /// `preupdate()`: the next position starts at the position.
    pub fn moving_preupdate(&mut self) {
        self.vars.next_x = self.vars.x;
        self.vars.next_y = self.vars.y;
    }

    /// `moveUnmodded(dx, dy)` (`IsoMovingObject.moveUnmoddedInternal`;
    /// zombies first shorten a sprinter's step at its target).
    pub fn move_unmodded_step(&mut self, mut dx: f32, mut dy: f32, cx: &mut StateCx<'_, I>) {
        if self.is_zombie() && self.zombie.speed_type == 1 {
            let attacking = [ids::LUNGE, ids::LUNGE_NETWORK, ids::ATTACK, ids::ATTACK_NETWORK, ids::STAGGER_BACK, ids::ZOMBIE_HIT_REACTION]
                .iter()
                .any(|s| self.is_current_state_id(*s));
            if attacking {
                if let Some((tnx, tny, tw)) =
                    self.vars.target.and_then(|t| cx.world.character_mut(t)).map(|t| (t.vars.next_x, t.vars.next_y, t.zombie.width))
                {
                    let (ex, ey) = (tnx - self.vars.x, tny - self.vars.y);
                    let mut dist = ((ex * ex + ey * ey) as f64).sqrt() as f32;
                    dist -= self.zombie.width + tw - 0.1;
                    dist = 0.0f32.max(dist);
                    let len = ((dx * dx + dy * dy) as f64).sqrt() as f32;
                    if len > dist {
                        dx = dx * dist / len;
                        dy = dy * dist / len;
                    }
                }
            }
        }
        self.vars.next_x += dx;
        self.vars.next_y += dy;
        self.vars.req_movement = (dx, dy);
    }

    /// `setCurrentSquareFromPosition(x, y, z)`: the square there, else the
    /// first one below.
    fn set_current_from_position(&mut self, x: f32, y: f32, z: f32, cx: &StateCx<'_, I>) {
        let (xi, yi) = (ff(x), ff(y));
        let mut cur = cx.world.square(xi, yi, ff(z)).map(|s| (s.x, s.y, s.z));
        if cur.is_none() {
            let mut n = ff(z);
            while n >= 0 {
                cur = cx.world.square(xi, yi, n).map(|s| (s.x, s.y, s.z));
                if cur.is_some() {
                    break;
                }
                n -= 1;
            }
        }
        self.path.current_square = cur;
    }

    fn ignore_collide(&self, from: (i32, i32, i32), to: (i32, i32, i32)) -> bool {
        self.current_state().is_some_and(|s| s.is_ignore_collide(self, from, to))
    }

    fn set_collided_from(&mut self, a: (i32, i32, i32), b: (i32, i32, i32)) {
        if a.0 < b.0 {
            self.vars.collided_e = true;
        }
        if a.0 > b.0 {
            self.vars.collided_w = true;
        }
        if a.1 < b.1 {
            self.vars.collided_s = true;
        }
        if a.1 > b.1 {
            self.vars.collided_n = true;
        }
    }

    /// The diagonal rule: pass 2 keeps east/west, pass 1 keeps north/south.
    fn collide_pass_rule(&mut self, pass: i32) {
        let v = &mut self.vars;
        let ns = v.collided_s || v.collided_n;
        let ew = v.collided_e || v.collided_w;
        if pass == 2 && ns && ew {
            v.collided_s = false;
            v.collided_n = false;
        }
        if pass == 1 && ns && ew {
            v.collided_w = false;
            v.collided_e = false;
        }
    }

    /// `DoCollide(pass)`.
    fn do_collide(&mut self, pass: i32, cx: &mut StateCx<'_, I>) -> bool {
        let (nx, ny, z) = (self.vars.next_x, self.vars.next_y, self.vars.z);
        self.set_current_from_position(nx, ny, z, cx);
        if self.vars.ragdoll_simulation_active {
            return false;
        }
        if let Some(cur) = self.path.current_square {
            let solid = cx.world.square(cur.0, cur.1, cur.2).is_some_and(|s| s.solid_floor);
            if !solid {
                self.set_current_from_position(nx, ny, z, cx);
            }
            if self.path.current_square.is_none() {
                return false;
            }
            self.set_current_from_position(nx, ny, z, cx);
        }
        let mover = pz_pathfind::mover::PathMover::mover_id(self);
        let (cur, last) = (self.path.current_square, self.vars.last_square);
        if let (Some(cur), Some(last)) = (cur, last) {
            if cur != last {
                if self.ignore_collide(last, cur) {
                    return false;
                }
                let (dx, dy, dz) = (cur.0 - last.0, cur.1 - last.1, cur.2 - last.2);
                let hit = cx.world.test_collide_adjacent(mover, last, dx, dy, dz);
                if hit {
                    self.set_collided_from(last, cur);
                    self.path.current_square = Some(last);
                    cx.world.collided_with_hoppables(self.id, Some(last));
                    self.collide_pass_rule(pass);
                    self.vars.collided_this_frame = true;
                    return true;
                }
                return false;
            }
        }
        if self.vars.next_x == self.vars.last_x && self.vars.next_y == self.vars.last_y {
            return false;
        }
        if self.is_zombie() && cx.ctx.game_mode() == "Tutorial" {
            return true;
        }
        let Some(cur) = self.path.current_square else {
            let v = &mut self.vars;
            if v.next_x < v.last_x {
                v.collided_w = true;
            }
            if v.next_x > v.last_x {
                v.collided_e = true;
            }
            if v.next_y < v.last_y {
                v.collided_n = true;
            }
            if v.next_y > v.last_y {
                v.collided_s = true;
            }
            v.next_x = v.last_x;
            v.next_y = v.last_y;
            self.path.current_square = self.vars.last_square;
            self.vars.collided_this_frame = true;
            return true;
        };
        if let Some(path) = &self.path.path2 {
            let (tx, ty, tz) = cx.world.path_behavior(self.id).map_or((0.0, 0.0, 0.0), |b| (b.target_x(), b.target_y(), b.target_z()));
            if ff(tx) == ff(self.vars.x) && ff(ty) == ff(self.vars.y) && ff(tz) == ff(self.vars.z) {
                let _ = path;
                return false;
            }
        }
        if self.sitting_on_furniture {
            return false;
        }
        // `getFeelerTile(feelersize)`.
        let (fx, fy) = {
            let (vx, vy) = (self.vars.next_x - self.vars.last_x, self.vars.next_y - self.vars.last_y);
            let len_sq = vx * vx + vy * vy;
            let (ux, uy) = if (1.0 - len_sq).abs() < 1.0e-5 {
                (vx, vy)
            } else if len_sq == 0.0 {
                (0.0, 0.0)
            } else {
                let l = (len_sq as f64).sqrt() as f32;
                (vx / l, vy / l)
            };
            (ux * 0.5, uy * 0.5)
        };
        let fsq = (ff(self.vars.x + fx), ff(self.vars.y + fy), ff(self.vars.z));
        let mut feeler = cx.world.square(fsq.0, fsq.1, fsq.2).map(|s| (s.x, s.y, s.z));
        if self.vars.climbing {
            feeler = Some(cur);
        }
        if let Some(f) = feeler {
            if f != cur {
                if let Some(path) = &self.path.path2 {
                    if !path_crosses_square(path, f.0, f.1, f.2) {
                        feeler = Some(cur);
                    }
                }
            }
        }
        let Some(f) = feeler else { return false };
        if f == cur {
            return false;
        }
        if self.ignore_collide(cur, f) {
            return false;
        }
        if !cx.world.test_collide_adjacent(mover, cur, f.0 - cur.0, f.1 - cur.1, f.2 - cur.2) {
            return false;
        }
        if self.vars.last_square.is_some() {
            self.set_collided_from(cur, f);
            cx.world.collided_with_hoppables(self.id, Some(cur));
            self.collide_pass_rule(pass);
        }
        self.vars.collided_this_frame = true;
        true
    }

    fn collide_axes(&mut self) -> bool {
        let mut did = false;
        if self.vars.collided_n || self.vars.collided_s {
            self.vars.next_y = self.vars.last_y;
            did = true;
        }
        if self.vars.collided_w || self.vars.collided_e {
            self.vars.next_x = self.vars.last_x;
            did = true;
        }
        did
    }

    /// `IsoMovingObject.postupdate()`'s movement and collision (see the
    /// module doc for what stays with the world).
    pub fn moving_postupdate(&mut self, cx: &mut StateCx<'_, I>) {
        if self.seen.last_targetted_by.is_some_and(|z| cx.world.character_mut(z).is_none_or(|z| z.is_dead())) {
            self.seen.last_targetted_by = None;
        }
        if self.seen.last_targetted_by.is_some() && self.seen.time_since_zombie_attack > 120 {
            self.seen.last_targetted_by = None;
        }
        self.seen.time_since_zombie_attack += 1;
        if self.is_player() {
            self.vars.last_collided_w = self.vars.collided_w;
            self.vars.last_collided_n = self.vars.collided_n;
        }
        if self.vars.destroyed {
            return;
        }
        let v = &mut self.vars;
        v.collided_this_frame = false;
        v.collided_n = false;
        v.collided_s = false;
        v.collided_w = false;
        v.collided_e = false;
        v.collided_with_door = false;
        v.last_square = self.path.current_square;
        v.next_x += v.impulse_x;
        v.next_y += v.impulse_y;
        let (tx, ty) = (v.next_x - v.x, v.next_y - v.y);
        if (((tx * tx + ty * ty) as f64).sqrt() as f32) > 1.0 {
            let len_sq = tx * tx + ty * ty;
            let l = (len_sq as f64).sqrt() as f32;
            v.next_x = v.x + tx / l;
            v.next_y = v.y + ty / l;
        }
        v.impulse_x = 0.0;
        v.impulse_y = 0.0;
        if self.is_zombie()
            && ff(self.vars.z) == 0
            && (self.is_current_state_id(ids::PATH_FIND) || self.is_current_state_id(ids::WALK_TOWARD))
            && cx.world.virtualize_if_unloaded(self.id, ff(self.vars.next_x), ff(self.vars.next_y))
        {
            return;
        }
        self.vars.collided_with_vehicle = false;
        if !self.vars.on_floor && self.path.vehicle.is_none() && self.path.collidable && !self.vars.no_clip {
            let from = (ff(self.vars.x), ff(self.vars.y), ff(self.vars.z));
            let to = (ff(self.vars.next_x), ff(self.vars.next_y), from.2);
            if !self.ignore_collide(from, to) {
                let (nx, ny) = (self.vars.next_x, self.vars.next_y);
                let (rx, ry) = cx.world.resolve_collision(self, nx, ny);
                if rx != nx || ry != ny {
                    self.vars.next_x = rx;
                    self.vars.next_y = ry;
                    self.vars.collided_with_vehicle = true;
                }
            }
        }
        let (onx, ony) = (self.vars.next_x, self.vars.next_y);
        if self.path.collidable {
            let (first, second) = if self.vars.alt_collide { (2, 1) } else { (1, 2) };
            self.do_collide(first, cx);
            self.collide_axes();
            self.do_collide(second, cx);
            self.vars.alt_collide = !self.vars.alt_collide;
            let did = self.collide_axes();
            let len = (self.vars.next_x - self.vars.last_x).abs() + (self.vars.next_y - self.vars.last_y).abs();
            let (lnx, lny) = (self.vars.next_x, self.vars.next_y);
            self.vars.next_x = onx;
            self.vars.next_y = ony;
            if did {
                let (first, second) = if self.vars.alt_collide { (2, 1) } else { (1, 2) };
                self.do_collide(first, cx);
                self.collide_axes();
                self.do_collide(second, cx);
                self.collide_axes();
                if (self.vars.next_x - self.vars.last_x).abs() + (self.vars.next_y - self.vars.last_y).abs() < len {
                    self.vars.next_x = lnx;
                    self.vars.next_y = lny;
                }
            }
        }
        if self.vars.collided_this_frame {
            self.path.current_square = self.vars.last_square;
        }
        let v = &self.vars;
        if self.is_player()
            && !self.is_current_state_id(ids::COLLIDE_WITH_WALL)
            && !(v.collided_n || v.collided_s || v.collided_w || v.collided_e)
        {
            self.vars.collide_type = String::new();
        }
        let mut dx = self.vars.next_x - self.vars.x;
        let mut dy = self.vars.next_y - self.vars.y;
        // `getGlobalMovementMod()`: 0.75 on stairs (the vegetation rustle
        // and tree noises are the world's); ghosts and no-clip players 1.
        let movement_mod = if dx.abs() > 0.0 || dy.abs() > 0.0 {
            let ghost = self.is_player() && (self.is_ghost_mode(cx.ctx) || self.vars.no_clip);
            let stairs = self.path.current_square.and_then(|c| cx.world.square(c.0, c.1, c.2)).is_some_and(|s| s.has_stairs);
            if !ghost && stairs {
                0.75
            } else {
                1.0
            }
        } else {
            0.0
        };
        if dx.abs() > 0.01 || dy.abs() > 0.01 {
            dx *= movement_mod;
            dy *= movement_mod;
        }
        self.vars.x += dx;
        self.vars.y += dy;
        // `findCurrentGridSquare()`.
        let (xi, yi, mut zi) = (ff(self.vars.x), ff(self.vars.y), ff(self.vars.z));
        let sign = zi.signum();
        let mut found = cx.world.square(xi, yi, zi).map(|s| (s.x, s.y, s.z));
        while found.is_none() {
            zi -= sign;
            if zi == -sign {
                break;
            }
            found = cx.world.square(xi, yi, zi).map(|s| (s.x, s.y, s.z));
        }
        self.path.current_square = found;
        if let Some(c) = found {
            self.vars.z = self.vars.z.min(c.2 as f32 + 0.99999);
            self.vars.last_z = self.vars.z;
        }
        if found.is_none() {
            if let Some(last) = self.vars.last_square {
                self.path.current_square = Some(last);
                self.vars.x = last.0 as f32 + 0.5;
                self.vars.y = last.1 as f32 + 0.5;
                self.vars.z = self.vars.z.min(last.2 as f32 + 0.99999);
                self.vars.last_z = self.vars.z;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::state::{SquareInfo, StateWorld};
    use crate::character::tests::{fixture, player, Recorder};
    use pz_pathfind::mover::MoverId;

    /// Squares 0..10 on level 0, with a wall between x = 4 and x = 5.
    struct Walled;
    impl StateWorld<()> for Walled {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            ((0..10).contains(&x) && (0..10).contains(&y) && z == 0).then(|| SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
        }
        fn test_collide_adjacent(&self, _m: MoverId, from: (i32, i32, i32), dx: i32, _dy: i32, _dz: i32) -> bool {
            from.0 == 4 && dx == 1 || from.0 == 5 && dx == -1
        }
    }

    #[test]
    fn walls_and_edges_stop_the_move() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Walled;
        let mut c = player(&sm, &mut chars);
        c.path.collidable = true;
        c.vars.x = 4.8;
        c.vars.y = 3.5;
        c.vars.last_x = 4.8;
        c.vars.last_y = 3.5;
        c.path.current_square = Some((4, 3, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.moving_preupdate();
        c.vars.next_x += 0.3;
        c.vars.next_y += 0.2;
        c.moving_postupdate(&mut cx);
        // Through the east wall: x stays, y moves; collided east.
        assert!(c.vars.collided_e && c.vars.collided_this_frame);
        assert_eq!(c.vars.x, 4.8);
        assert_eq!(c.vars.y, 3.7);
        assert_eq!(c.path.current_square, Some((4, 3, 0)));
        // A free move resets the flags.
        c.vars.last_x = c.vars.x;
        c.vars.last_y = c.vars.y;
        c.moving_preupdate();
        c.vars.next_y += 0.5;
        c.moving_postupdate(&mut cx);
        assert!(!c.vars.collided_e && !c.vars.collided_this_frame);
        assert_eq!(c.path.current_square, Some((4, 4, 0)));
        // Off the map's edge.
        c.vars.x = 0.2;
        c.vars.last_x = 0.2;
        c.path.current_square = Some((0, 4, 0));
        c.moving_preupdate();
        c.vars.next_x -= 0.5;
        c.moving_postupdate(&mut cx);
        assert!(c.vars.collided_w);
        assert_eq!(c.vars.x, 0.2);
        assert!(path_crosses_square(
            &pz_pathfind::Path { nodes: vec![pz_pathfind::PathNode { x: 0.5, y: 0.5, z: 32.0, flags: 0 }, pz_pathfind::PathNode { x: 2.5, y: 0.5, z: 32.0, flags: 0 }] },
            1,
            0,
            0
        ));
    }
}

//! `IsoMovingObject.postupdate`'s movement and collision: the per-frame
//! collision flag resets, impulses, the obstacle clip
//! (`PolygonalMap2.resolveCollision`, pz-pathfind's `CollideWithObstacles`
//! through [`StateWorld::resolve_collision`]), the square-edge collision
//! passes (`DoCollide(1)` / `DoCollide(2)`, `Collided()`), the movement
//! modifier and the move itself, then the current square
//! (`findCurrentGridSquare`).
//!
//! Then a sprinting player's wall hit (`checkHitWall`, with the automatic
//! vault of `checkVaultOver`), the stairs (`doStairs`) and sloped surfaces
//! (`handleSlopedSurface`).
//!
//! The squares are the world's: `testCollideAdjacent` and the square
//! lookups go through [`StateWorld`]. The hoppable checks are the
//! character's [`super::collide`]; zombie virtualization at the edge of the
//! loaded area is the world's ([`StateWorld::virtualize_if_unloaded`]).
//!
//! [`StateWorld`]: super::state::StateWorld
//! [`StateWorld::resolve_collision`]: super::state::StateWorld::resolve_collision
//! [`StateWorld::virtualize_if_unloaded`]: super::state::StateWorld::virtualize_if_unloaded
//! [`StateWorld::resolve_collision_radius`]: super::state::StateWorld::resolve_collision_radius

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
        if ff(a.z) != z && ff(b.z) != z {
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
        if cx.has_host_error() { return false; }
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
        let (cur, last) = (self.path.current_square, self.vars.last_square);
        if let (Some(cur), Some(last)) = (cur, last) {
            if cur != last {
                if self.ignore_collide(last, cur) {
                    return false;
                }
                let (dx, dy, dz) = (cur.0 - last.0, cur.1 - last.1, cur.2 - last.2);
                let hit = self.test_collide_adjacent_with_mover(last, (dx, dy, dz), cx);
                if cx.has_host_error() { return false; }
                if hit {
                    self.set_collided_from(last, cur);
                    self.path.current_square = Some(last);
                    self.check_hoppables(last, cx);
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
        let fsq = self.feeler_tile();
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
        if !self.test_collide_adjacent_with_mover(cur, (f.0 - cur.0, f.1 - cur.1, f.2 - cur.2), cx) {
            return false;
        }
        if self.vars.last_square.is_some() {
            self.set_collided_from(cur, f);
            self.check_hoppables(cur, cx);
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
        if cx.has_host_error() { return; }
        self.slide_head_away_from_walls(true, cx);
        if cx.has_host_error() { return; }
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
        v.collided_object = None;
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
            if cx.has_host_error() { return; }
            self.collide_axes();
            self.do_collide(second, cx);
            if cx.has_host_error() { return; }
            self.vars.alt_collide = !self.vars.alt_collide;
            let did = self.collide_axes();
            let len = (self.vars.next_x - self.vars.last_x).abs() + (self.vars.next_y - self.vars.last_y).abs();
            let (lnx, lny) = (self.vars.next_x, self.vars.next_y);
            self.vars.next_x = onx;
            self.vars.next_y = ony;
            if did {
                let (first, second) = if self.vars.alt_collide { (2, 1) } else { (1, 2) };
                self.do_collide(first, cx);
                if cx.has_host_error() { return; }
                self.collide_axes();
                self.do_collide(second, cx);
                if cx.has_host_error() { return; }
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
        self.check_hit_wall(cx);
        let v = &self.vars;
        if self.is_player()
            && !self.is_current_state_id(ids::COLLIDE_WITH_WALL)
            && !(v.collided_n || v.collided_s || v.collided_w || v.collided_e)
        {
            self.vars.collide_type = String::new();
        }
        let mut dx = self.vars.next_x - self.vars.x;
        let mut dy = self.vars.next_y - self.vars.y;
        let movement_mod = if dx.abs() > 0.0 || dy.abs() > 0.0 { self.global_movement_mod(cx) } else { 0.0 };
        if dx.abs() > 0.01 || dy.abs() > 0.01 {
            dx *= movement_mod;
            dy *= movement_mod;
        }
        self.set_x(self.vars.x + dx);
        self.set_y(self.vars.y + dy);
        self.do_stairs(cx);
        self.handle_sloped_surface(cx);
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

impl<I: 'static> GameCharacter<I> {
    /// `IsoZombie.shouldSlideHeadAwayFromWalls()` (other characters:
    /// never).
    fn should_slide_head_away_from_walls(&self) -> bool {
        if !self.is_zombie() {
            return false;
        }
        let v = &self.vars;
        let idle_or_staggering = self.is_current_state_id(ids::STAGGER_BACK) || self.is_current_state_id(ids::ZOMBIE_IDLE);
        if ((!v.on_floor && !v.knocked_down) || v.sit_against_wall) && !idle_or_staggering {
            return false;
        }
        if v.crawling && (self.path.path2.is_some() || v.moving) {
            return false;
        }
        !self.is_current_state_id(ids::CLIMB_OVER_FENCE) && !self.is_current_state_id(ids::CLIMB_THROUGH_WINDOW)
    }

    /// `IsoZombie.slideHeadAwayFromWalls(instant)`: a zombie on the floor
    /// keeps its head 0.15 from the walls.
    pub fn slide_head_away_from_walls(&mut self, instant: bool, cx: &mut StateCx<'_, I>) {
        if self.path.current_square.is_none() || !self.should_slide_head_away_from_walls() {
            return;
        }
        self.slide_away_from_walls(0.15, instant, false, cx);
    }

    /// `IsoGameCharacter.slideAwayFromWalls(radius, instant,
    /// includePolyCollisions)` (`onRagdollSimulationStarted` uses 0.5,
    /// true, true; `onHitByVehicle` 0.25, false, true).
    pub fn slide_away_from_walls(&mut self, radius: f32, instant: bool, include_poly_collisions: bool, cx: &mut StateCx<'_, I>) {
        if cx.has_host_error() { return; }
        let mut new_pos = (0.0, 0.0);
        let mut collided = false;
        if include_poly_collisions {
            let (hit, at) = cx.world.resolve_collision_radius(&*self, radius);
            collided = hit;
            new_pos = at;
        }
        // `newPos` is written once the circle reaches past the square,
        // collided or not.
        if let Some((hit, at)) = self.resolve_collision_with_neighboring_squares(radius, cx) {
            new_pos = at;
            collided |= hit;
        }
        if cx.has_host_error() { return; }
        if collided {
            self.slide_away_to_collision_pos(new_pos.0, new_pos.1, instant);
        }
    }

    /// `resolveCollisionWithNeighboringSquares(radius, newPos)`: the next
    /// position pushed `radius` back from the current square's blocked
    /// edges, when a circle there would cross one. `None` before `newPos`
    /// is written; otherwise whether it collided and `newPos`.
    fn resolve_collision_with_neighboring_squares(&mut self, radius: f32, cx: &mut StateCx<'_, I>) -> Option<(bool, (f32, f32))> {
        let (x, y, z) = (self.vars.x, self.vars.y, self.vars.z);
        let (nx, ny) = (self.vars.next_x, self.vars.next_y);
        let (sx, sy, sz) = (ff(x), ff(y), ff(z));
        let sign_x = sign(nx - (0.5 + sx as f32));
        let sign_y = sign(ny - (0.5 + sy as f32));
        let x2 = nx + radius * sign_x as f32;
        let y2 = ny + radius * sign_y as f32;
        if ff(x2) == sx && ff(y2) == sy {
            return None;
        }
        cx.world.square(sx, sy, sz)?;
        let from = (sx, sy, sz);
        let mut collided = false;
        let mut at = (nx, ny);
        if sign_x != 0 && self.test_collide_adjacent_with_mover(from, (sign_x, 0, 0), cx) {
            collided = true;
            at.0 = if sign_x == -1 { sx as f32 + radius } else { sx as f32 + 1.0 - radius };
        }
        if cx.has_host_error() { return None; }
        if sign_y != 0 && self.test_collide_adjacent_with_mover(from, (0, sign_y, 0), cx) {
            collided = true;
            at.1 = if sign_y == -1 { sy as f32 + radius } else { sy as f32 + 1.0 - radius };
        }
        if cx.has_host_error() { return None; }
        Some((collided, at))
    }

    /// `IsoMovingObject.slideAwayToCollisionPos(x, y, instant)`: along each
    /// axis the move keeps its direction and takes the longer of the two
    /// offsets.
    pub fn slide_away_to_collision_pos(&mut self, cx_: f32, cy: f32, instant: bool) {
        let (x1, y1) = (self.vars.x, self.vars.y);
        let (ovx, ovy) = (self.vars.next_x - x1, self.vars.next_y - y1);
        let (osx, osy) = (sign(ovx), sign(ovy));
        let (mut px, mut py) = (cx_, cy);
        let (cvx, cvy) = (cx_ - x1, cy - y1);
        let (csx, csy) = (sign(cvx), sign(cvy));
        if osx == csx {
            px = x1 + fmax(ovx * osx as f32, cvx * csx as f32) * osx as f32;
        }
        if osy == csy {
            py = y1 + fmax(ovy * osy as f32, cvy * csy as f32) * osy as f32;
        }
        self.vars.next_x = px;
        self.vars.next_y = py;
        if instant {
            self.vars.x = px;
            self.vars.y = py;
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `getFeelerTile(feelersize)`'s coordinates: half a square from the
    /// position along the move from the last position.
    fn feeler_tile(&self) -> (i32, i32, i32) {
        let (vx, vy) = (self.vars.next_x - self.vars.last_x, self.vars.next_y - self.vars.last_y);
        let (ux, uy) = crate::character::zombie::math::normalize(vx, vy);
        (ff(self.vars.x + ux * 0.5), ff(self.vars.y + uy * 0.5), ff(self.vars.z))
    }

    /// `getGlobalMovementMod(true)`: 1 while climbing (and for ghost or
    /// no-clip players); otherwise the vegetation on the square and the
    /// feeler square rustles ([`StateWorld::rustle_vegetation`]) and stairs
    /// slow to 0.75.
    ///
    /// [`StateWorld::rustle_vegetation`]: super::state::StateWorld::rustle_vegetation
    fn global_movement_mod(&mut self, cx: &mut StateCx<'_, I>) -> f32 {
        if [ids::CLIMB_OVER_FENCE, ids::CLIMB_THROUGH_WINDOW, ids::CLIMB_OVER_WALL].iter().any(|s| self.is_current_state_id(*s)) {
            return 1.0;
        }
        if self.is_player() && (self.is_ghost_mode(cx.ctx) || self.vars.no_clip) {
            return 1.0;
        }
        let cur = self.path.current_square;
        if let Some(c) = cur {
            if self.vars.z - (ff(self.vars.z) as f32) < 0.5 {
                cx.world.rustle_vegetation(self.id, c, cx.ctx);
                let f = self.feeler_tile();
                if let Some(f) = cx.world.square(f.0, f.1, f.2).map(|s| (s.x, s.y, s.z)) {
                    if f != c {
                        cx.world.rustle_vegetation(self.id, f, cx.ctx);
                    }
                }
            }
        }
        if cur.and_then(|c| cx.world.square(c.0, c.1, c.2)).is_some_and(|s| s.has_stairs) {
            0.75
        } else {
            1.0
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `setX(x)`: the position and the next position.
    pub fn set_x(&mut self, x: f32) {
        self.vars.x = x;
        self.vars.next_x = x;
    }

    /// `setY(y)`.
    pub fn set_y(&mut self, y: f32) {
        self.vars.y = y;
        self.vars.next_y = y;
    }

    /// `setZ(z)`: clamped to -32..31, and the last z.
    pub fn set_z(&mut self, z: f32) {
        let z = z.clamp(-32.0, 31.0);
        self.vars.z = z;
        self.vars.last_z = z;
    }

    /// `IsoMovingObject.doStairs()`: the height on a staircase, and the
    /// slide off its open sides at the top.
    fn do_stairs(&mut self, cx: &mut StateCx<'_, I>) {
        let (Some(cur), Some(last)) = (self.path.current_square, self.vars.last_square) else { return };
        let w = &*cx.world;
        let sq = |p: (i32, i32, i32)| w.square(p.0, p.1, p.2);
        let Some(mut current) = sq(cur) else { return };
        let Some(last) = sq(last) else { return };
        if current.stairs.top() && self.vars.z - (ff(self.vars.z) as f32) < 0.1 {
            if let Some(below) = sq((current.x, current.y, current.z - 1)).filter(|b| b.stairs.top()) {
                current = below;
            }
        }
        if last.stairs.top() {
            self.set_z(super::xp::java_round(self.vars.z) as f32);
        }
        let mut z = self.vars.z;
        if current.has_stairs {
            let (dx, dy) = (self.vars.x - current.x as f32, self.vars.y - current.y as f32);
            let at = (current.x, current.y, current.z);
            z = current.apparent_z(dx, dy, |dx, dy| w.sloped_surface_height(at, dx, dy));
        }
        let frac = |v: f32| v as f64 - (v as f64).floor();
        let floor = |v: f32| (v as f64).floor() as f32;
        let above = |s: &crate::character::state::SquareInfo| sq((s.x, s.y, s.z + 1));
        let rel = |s: &crate::character::state::SquareInfo, dx: i32, dy: i32, dz: i32| sq((s.x + dx, s.y + dy, s.z + dz));
        if last.stairs.tn && above(&last).is_some_and(|u| u.wall_n) {
            if frac(self.vars.x) > 0.8f32 as f64 && !rel(&last, 1, 0, 0).is_some_and(|e| e.stairs.tn) {
                self.set_x(floor(self.vars.x) + 0.8);
            }
            if !current.has_stairs {
                self.set_y(super::xp::java_round(self.vars.y) as f32);
            }
        }
        if last.stairs.tw && above(&last).is_some_and(|u| u.wall_w) {
            if frac(self.vars.y) > 0.8f32 as f64 && !rel(&last, 0, 1, 0).is_some_and(|s| s.stairs.tw) {
                self.set_y(floor(self.vars.y) + 0.8);
            }
            if !current.has_stairs {
                self.set_x(super::xp::java_round(self.vars.x) as f32);
            }
        }
        if current.wall_n {
            if let Some(d) = rel(&current, 0, 0, -1).filter(|d| d.stairs.tn) {
                if frac(self.vars.x) > 0.8f32 as f64 && !rel(&d, 1, 0, 0).is_some_and(|e| e.stairs.tn) {
                    self.set_x(floor(self.vars.x) + 0.8);
                }
                if frac(self.vars.x) < 0.2f32 as f64 && !rel(&d, -1, 0, 0).is_some_and(|w| w.stairs.tn) {
                    self.set_x(floor(self.vars.x) + 0.2);
                }
            }
        }
        if current.wall_w {
            if let Some(d) = rel(&current, 0, 0, -1).filter(|d| d.stairs.tw) {
                if frac(self.vars.y) > 0.8f32 as f64 && !rel(&d, 0, 1, 0).is_some_and(|s| s.stairs.tw) {
                    self.set_y(floor(self.vars.y) + 0.8);
                }
                if frac(self.vars.y) < 0.2f32 as f64 && !rel(&d, 0, -1, 0).is_some_and(|n| n.stairs.tw) {
                    self.set_y(floor(self.vars.y) + 0.2);
                }
            }
        }
        if self.is_current_state_id(ids::CLIMB_OVER_FENCE) || self.is_current_state_id(ids::CLIMB_THROUGH_WINDOW) {
            if current.has_stairs && self.vars.z > z {
                let lowered = self.vars.z - 0.075 * cx.ctx.multiplier();
                self.set_z(if z > lowered { z } else { lowered });
            }
            return;
        }
        if (z - self.vars.z).abs() < 0.95 {
            self.set_z(z);
        }
    }

    /// `IsoMovingObject.handleSlopedSurface()`.
    fn handle_sloped_surface(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(cur) = self.path.current_square else { return };
        if self.path.vehicle.is_some() {
            return;
        }
        if let Some(last) = self.vars.last_square.filter(|&l| l != cur) {
            if cx.world.square(last.0, last.1, last.2).is_some_and(|s| s.sloped_surface && s.sloped_surface_height_max == 1.0) {
                self.set_z(super::xp::java_round(self.vars.z) as f32);
            }
        }
        let dz = cx.world.sloped_surface_height(cur, self.vars.x % 1.0, self.vars.y % 1.0);
        if dz <= 0.0 {
            return;
        }
        self.set_z(cur.2 as f32 + dz);
    }

    /// `IsoMovingObject.checkHitWall()`: a sprinting local player running
    /// into a wall it faces collides with it (or vaults a low fence there).
    fn check_hit_wall(&mut self, cx: &mut StateCx<'_, I>) {
        let v = &self.vars;
        if !(v.collided_n || v.collided_s || v.collided_e || v.collided_w) {
            return;
        }
        let Some(cur) = self.path.current_square else { return };
        if !self.is_player() || !self.vars.collide_type.is_empty() {
            return;
        }
        let mut valid = false;
        let wall_type = cx.world.wall_type(cur);
        if self.vars.collided_with_door {
            if let Some(o) = self.vars.collided_object {
                if let Some(i) = cx.world.thump_info(crate::character::state::ThumpTarget::Object(o), self.id).filter(|i| i.kind == crate::character::state::ThumpKind::Door) {
                    let v = &self.vars;
                    if i.north && (v.collided_n || v.collided_s) || !i.north && (v.collided_e || v.collided_w) {
                        valid = true;
                    }
                }
            }
        }
        let (v, d) = (&self.vars, self.vars.dir);
        use pz_world::square::IsoDirections as D;
        if wall_type & 1 != 0 && v.collided_n && d == D::N
            || wall_type & 2 != 0 && v.collided_s && d == D::S
            || wall_type & 4 != 0 && v.collided_w && d == D::W
            || wall_type & 8 != 0 && v.collided_e && d == D::E
        {
            valid = true;
        }
        if self.check_vault_over(cur, cx) {
            valid = false;
        }
        if valid && self.is_sprinting(cx.sm, cx.ctx) && self.is_local_player(cx.ctx) {
            self.vars.collide_type = "wall".into();
            self.report_event("collideWithWall");
            self.vars.last_collide_time = 70.0;
        }
    }

    /// `IsoMovingObject.checkVaultOver()`: a running player into a low
    /// fence vaults it.
    fn check_vault_over(&mut self, cur: (i32, i32, i32), cx: &mut StateCx<'_, I>) -> bool {
        if self.is_current_state_id(ids::CLIMB_OVER_FENCE) || self.vars.ignore_auto_vault {
            return false;
        }
        let remote_obstacle = !self.is_local_player(cx.ctx) && self.vars.has_obstacle_on_path;
        if !(self.is_running(cx.sm) || self.is_sprinting(cx.sm, cx.ctx) || remote_obstacle) {
            return false;
        }
        let v = &self.vars;
        let at = (ff(v.x), ff(v.y), ff(v.z));
        let Some((dir, f)) = cx.world.vault_over(cur, at, v.dir, (v.collided_n, v.collided_s, v.collided_w, v.collided_e)) else {
            return false;
        };
        crate::character::states::climb_fence::set_params(self, dir, f, cx);
        self.report_event("EventClimbFence");
        true
    }
}

/// `PZMath.max(float, float)`.
fn fmax(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `PZMath.sign(float)`.
fn sign(v: f32) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
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
            &pz_pathfind::Path { nodes: vec![pz_pathfind::PathNode { x: 0.5, y: 0.5, z: 0.0, flags: 0 }, pz_pathfind::PathNode { x: 2.5, y: 0.5, z: 0.0, flags: 0 }] },
            1,
            0,
            0
        ));
    }

    #[test]
    fn a_zombie_on_the_floor_keeps_its_head_off_the_wall() {
        let (sm, chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Walled;
        let mut z = GameCharacter::<()>::new(&sm, crate::character::CharacterKind::Zombie);
        z.vars.on_floor = true;
        z.vars.x = 4.9;
        z.vars.y = 3.5;
        z.vars.next_x = 4.95;
        z.vars.next_y = 3.5;
        z.path.current_square = Some((4, 3, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        z.slide_head_away_from_walls(true, &mut cx);
        assert_eq!((z.vars.x, z.vars.next_x), (4.0 + 1.0 - 0.15, 4.0 + 1.0 - 0.15));
        assert_eq!(z.vars.y, 3.5);
        // Standing up: no slide.
        z.vars.on_floor = false;
        z.vars.x = 4.9;
        z.vars.next_x = 4.95;
        z.slide_head_away_from_walls(true, &mut cx);
        assert_eq!(z.vars.x, 4.9);
    }

    /// A north staircase at x = 2: bottom (2, 5), middle (2, 4), top (2, 3).
    struct Stairs;
    impl StateWorld<()> for Stairs {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            if !(0..10).contains(&x) || !(0..10).contains(&y) || !(0..2).contains(&z) {
                return None;
            }
            let mut s = SquareInfo { x, y, z, solid_floor: true, ..Default::default() };
            if z == 0 && x == 2 && (3..=5).contains(&y) {
                s.has_stairs = true;
                s.has_stairs_north = true;
                s.stairs = crate::character::state::StairsPieces { bn: y == 5, mn: y == 4, tn: y == 3, ..Default::default() };
            }
            Some(s)
        }
    }

    #[test]
    fn stairs_lift_the_character() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Stairs;
        let mut c = player(&sm, &mut chars);
        c.path.collidable = true;
        c.vars.x = 2.5;
        c.vars.y = 6.1;
        c.vars.last_x = 2.5;
        c.vars.last_y = 6.1;
        c.path.current_square = Some((2, 6, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.moving_preupdate();
        c.vars.next_y -= 0.3;
        c.moving_postupdate(&mut cx);
        // getApparentZ on stairsBN at dy = 0.8: lerp(0.01, 0.4333, 0.2).
        assert_eq!(c.path.current_square, Some((2, 5, 0)));
        let dy = c.vars.y - 5.0;
        assert_eq!(c.vars.z, 0.01 + (0.3333f32 + 0.1 - 0.01) * (1.0 - dy));
        // Up to the top piece: still level 0, almost a whole level up.
        while c.vars.y > 3.3 {
            c.vars.last_x = c.vars.x;
            c.vars.last_y = c.vars.y;
            c.moving_preupdate();
            c.vars.next_y -= 0.25;
            c.moving_postupdate(&mut cx);
        }
        // The stairs slow the move to 0.75.
        assert_eq!(c.path.current_square, Some((2, 3, 0)));
        assert!(c.vars.z > 0.9, "z {}", c.vars.z);
    }

    /// The wall between x = 4 and x = 5 is the east wall of (4, y).
    struct WalledType;
    impl StateWorld<()> for WalledType {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Walled.square(x, y, z)
        }
        fn test_collide_adjacent(&self, m: MoverId, from: (i32, i32, i32), dx: i32, dy: i32, dz: i32) -> bool {
            StateWorld::<()>::test_collide_adjacent(&Walled, m, from, dx, dy, dz)
        }
        fn wall_type(&self, sq: (i32, i32, i32)) -> i32 {
            if sq.0 == 4 {
                8
            } else {
                0
            }
        }
    }

    #[test]
    fn a_sprinting_player_hits_the_wall_it_faces() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = WalledType;
        let mut c = player(&sm, &mut chars);
        c.path.collidable = true;
        c.environment.sprinting = true;
        c.vars.dir = pz_world::square::IsoDirections::E;
        c.vars.x = 4.8;
        c.vars.y = 3.5;
        c.vars.last_x = 4.8;
        c.vars.last_y = 3.5;
        c.path.current_square = Some((4, 3, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.moving_preupdate();
        c.vars.next_x += 0.3;
        c.moving_postupdate(&mut cx);
        assert!(c.vars.collided_e);
        assert_eq!(c.vars.collide_type, "wall");
        assert_eq!(c.vars.last_collide_time, 70.0);
        // Facing away from the wall: no hit.
        c.vars.collide_type.clear();
        c.vars.last_collide_time = 0.0;
        c.vars.dir = pz_world::square::IsoDirections::N;
        c.moving_preupdate();
        c.vars.next_x += 0.3;
        c.moving_postupdate(&mut cx);
        assert!(c.vars.collided_e);
        assert_eq!(c.vars.collide_type, "");
    }
}

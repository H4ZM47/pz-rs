//! `BaseVehicle` geometry: local/world positions, the footprint polygons,
//! script areas, passenger positions, attachments and the collision and
//! closest-point tests.

use crate::env;
use crate::jmath::{
    self, closest_point_on_line_segment, closest_points_on_line_segments, fastfloor, Vector2,
    Vector3f,
};
use crate::poly::{poly_poly_intersects, quadrangles_intersect, VehiclePoly};
use crate::vehicle::{BaseVehicle, LEVEL_HEIGHT};
use pz_scripts::objects::vehicles::Area;

fn v3(v: pz_scripts::math::Vector3) -> Vector3f {
    Vector3f::new(v.x, v.y, v.z)
}

/// A world position of a passenger slot (`VehicleScript.Position`).
#[derive(Clone, Debug, PartialEq)]
pub struct PassengerPosition {
    pub offset: Vector3f,
    pub area: Option<String>,
}

impl BaseVehicle {
    fn script_extents(&self) -> Option<(Vector3f, Vector3f)> {
        let s = self.script()?;
        let s = s.borrow();
        Some((v3(s.extents()), v3(s.center_of_mass_offset())))
    }

    /// `getWorldPos(localX, localY, localZ, out)`: world `x`, world `y`
    /// and level `z`.
    pub fn world_pos(&self, lx: f32, ly: f32, lz: f32) -> Vector3f {
        let mut xfrm = self.world_transform();
        xfrm.origin = Vector3f::default();
        let mut w = Vector3f::new(lx, ly, lz);
        xfrm.transform(&mut w);
        let o = self.st().jni_transform.origin;
        let (ox, oy) = env::offset();
        let px = o.x + ox;
        let py = o.z + oy;
        let pz = o.y / LEVEL_HEIGHT;
        Vector3f::new(px + w.x, py + w.z, pz + w.y)
    }

    /// `getLocalPos(worldX, worldY, worldZ, out)`.
    pub fn local_pos(&self, wx: f32, wy: f32, _wz: f32) -> Vector3f {
        let mut xfrm = self.world_transform();
        xfrm.inverse();
        let (ox, oy) = env::offset();
        let mut l = Vector3f::new(wx - ox, 0.0, wy - oy);
        xfrm.transform(&mut l);
        l
    }

    /// `VehiclePoly.init(vehicle, radius)`.
    fn init_poly(&self, radius: f32) -> VehiclePoly {
        let mut p = VehiclePoly::default();
        let Some((ext, com)) = self.script_extents() else {
            return p;
        };
        p.t = self.world_transform();
        let q = p.t.rotation();
        let width = (ext.x * 1.0 + radius * 2.0) / 2.0;
        let length = (ext.z * 1.0 + radius * 2.0) / 2.0;
        let height = (ext.y * 1.0 + radius * 2.0) / 2.0;
        let mut c = [Vector2::default(); 4];
        let pts = if q.x < 0.0 {
            [
                (com.x - width, 0.0, com.z + length),
                (com.x + width, height, com.z + length),
                (com.x + width, height, com.z - length),
                (com.x - width, 0.0, com.z - length),
            ]
        } else {
            [
                (com.x - width, height, com.z + length),
                (com.x + width, 0.0, com.z + length),
                (com.x + width, 0.0, com.z - length),
                (com.x - width, height, com.z - length),
            ]
        };
        for (i, (x, y, z)) in pts.iter().enumerate() {
            let w = self.world_pos(*x, *y, *z);
            c[i] = Vector2::new(w.x, w.y);
        }
        p.z = self.z();
        let mut sum = 0i32;
        for i in 0..4 {
            let v1 = c[i];
            let v2 = c[(i + 1) % 4];
            sum = (sum as f32 + (v2.x - v1.x) * (v2.y + v1.y)) as i32;
        }
        if sum < 0 {
            c.swap(1, 3);
        }
        p.borders = c;
        p.x1 = c[0].x;
        p.y1 = c[0].y;
        p.x2 = c[1].x;
        p.y2 = c[1].y;
        p.x3 = c[2].x;
        p.y3 = c[2].y;
        p.x4 = c[3].x;
        p.y4 = c[3].y;
        p
    }

    fn refresh_polys(&self) {
        if !self.st().poly_dirty {
            return;
        }
        if self.st().poly_garage_check && self.st().square.is_some() {
            let (x, y, z) = self.st().square.unwrap();
            let garage = env::world()
                .square(x, y, z)
                .and_then(|s| s.room_def)
                .is_some_and(|r| r.contains("garagestorage"));
            let mut st = self.st_mut();
            st.radius_reduction_in_garage = if garage { -0.3 } else { 0.0 };
            st.poly_garage_check = false;
        }
        let poly = self.init_poly(0.0);
        let r = 0.15 + self.st().radius_reduction_in_garage;
        let plus = self.init_poly(r);
        let shadow = self.init_shadow_poly();
        let mut st = self.st_mut();
        st.poly = poly;
        st.poly_plus_radius = plus;
        st.poly_dirty = false;
        st.poly_plus_radius_min_x = -123.0;
        st.shadow_coord = shadow;
    }

    /// `initShadowPoly()`.
    fn init_shadow_poly(&self) -> VehiclePoly {
        let mut p = VehiclePoly::default();
        let Some(script) = self.script() else {
            return p;
        };
        let (ext, off) = {
            let s = script.borrow();
            (s.shadow_extents(), s.shadow_offset())
        };
        let width = ext.x / 2.0;
        let length = ext.y / 2.0;
        let a = self.world_pos(off.x - width, 0.0, off.y + length);
        let b = self.world_pos(off.x + width, 0.0, off.y + length);
        let c = self.world_pos(off.x + width, 0.0, off.y - length);
        let d = self.world_pos(off.x - width, 0.0, off.y - length);
        p.x1 = a.x;
        p.y1 = a.y;
        p.x2 = b.x;
        p.y2 = b.y;
        p.x3 = c.x;
        p.y3 = c.y;
        p.x4 = d.x;
        p.y4 = d.y;
        p
    }

    /// `getPoly()`.
    pub fn poly(&self) -> VehiclePoly {
        self.refresh_polys();
        self.st().poly
    }

    /// `getPolyPlusRadius()`.
    pub fn poly_plus_radius(&self) -> VehiclePoly {
        self.refresh_polys();
        self.st().poly_plus_radius
    }

    /// `getClosestPointOnPoly(x, y, out)`: the squared distance and the
    /// point.
    pub fn closest_point_on_poly(&self, x: f32, y: f32) -> (f32, (f32, f32)) {
        if self.script().is_none() {
            return (0.0, (x, y));
        }
        let p = self.poly();
        let c = p.corners();
        let mut dist = f32::MAX as f64;
        let mut out = (0.0f32, 0.0f32);
        for i in 0..4 {
            let a = c[i];
            let b = c[(i + 1) % 4];
            let (d, pt) = closest_point_on_line_segment(a.0, a.1, b.0, b.1, x, y, 0.0);
            if d < dist {
                dist = d;
                out = pt;
            }
        }
        (dist as f32, out)
    }

    /// `getClosestPointOnPoly(other, pointSelf, pointOther)`.
    pub fn closest_point_on_poly_vehicle(
        &self,
        other: &BaseVehicle,
    ) -> (f32, (f32, f32), (f32, f32)) {
        if self.script().is_none() && other.script().is_none() {
            let a = (self.x(), self.y());
            let b = (other.x(), other.y());
            return (jmath::dist_sq(a.0, a.1, b.0, b.1), a, b);
        }
        if self.script().is_none() {
            let (d, p) = other.closest_point_on_poly(self.x(), self.y());
            return (d, (self.x(), self.y()), p);
        }
        if other.script().is_none() {
            let (d, p) = self.closest_point_on_poly(other.x(), other.y());
            return (d, p, (other.x(), other.y()));
        }
        let p1 = self.poly().borders;
        let p2 = other.poly().borders;
        let mut best = f64::MAX;
        let mut a = (0.0, 0.0);
        let mut b = (0.0, 0.0);
        for i in 0..4 {
            let (s, e) = (p1[i], p1[(i + 1) % 4]);
            for j in 0..4 {
                let (s2, e2) = (p2[j], p2[(j + 1) % 4]);
                let (d, q1, q2) =
                    closest_points_on_line_segments(s.x, s.y, e.x, e.y, s2.x, s2.y, e2.x, e2.y);
                if d < best {
                    best = d;
                    a = q1;
                    b = q2;
                }
            }
        }
        (best as f32, a, b)
    }

    /// `getClosestPointOnExtents(x, y, closest)`.
    pub fn closest_point_on_extents(&self, x: f32, y: f32) -> (f32, (f32, f32)) {
        let Some((ext, com)) = self.script_extents() else {
            return (0.0, (x, y));
        };
        let pos = self.local_pos(x, y, 0.0);
        let (x_min, x_max) = (com.x - ext.x / 2.0, com.x + ext.x / 2.0);
        let (y_min, y_max) = (com.z - ext.z / 2.0, com.z + ext.z / 2.0);
        let (lx, ly) = (pos.x, pos.z);
        let mut dist = 3.4028234663852886e38f64;
        let mut out = (0.0f32, 0.0f32);
        for (x1, y1, x2, y2) in [
            (x_min, y_min, x_max, y_min),
            (x_max, y_min, x_max, y_max),
            (x_min, y_max, x_max, y_max),
            (x_min, y_min, x_min, y_max),
        ] {
            let (d, p) = closest_point_on_line_segment(x1, y1, x2, y2, lx, ly, 0.0);
            if d < dist {
                dist = d;
                out = p;
            }
        }
        let w = self.world_pos(out.0, 0.0, out.1);
        (dist as f32, (w.x, w.y))
    }

    /// `isInBounds(worldX, worldY)`.
    pub fn is_in_bounds(&self, x: f32, y: f32) -> bool {
        self.poly().contains_point(x, y)
    }

    /// `isIntersectingSquare(x, y, z)`.
    pub fn is_intersecting_square(&self, x: i32, y: i32, z: i32) -> bool {
        if z != self.zi() {
            return false;
        }
        if self.st().removed_from_world || self.st().current.is_none() || !self.has_controller() {
            return false;
        }
        let t = VehiclePoly {
            x1: x as f32,
            x4: x as f32,
            y1: y as f32,
            y2: y as f32,
            x2: x as f32 + 1.0,
            x3: x as f32 + 1.0,
            y3: y as f32 + 1.0,
            y4: y as f32 + 1.0,
            ..Default::default()
        };
        poly_poly_intersects(&t, &self.poly())
    }

    /// `circleIntersects(x, y, z, radius)`.
    pub fn circle_intersects(&self, x: f32, y: f32, z: f32, radius: f32) -> bool {
        if !self.has_controller() || fastfloor(z) != self.zi() {
            return false;
        }
        if jmath::dist_2d(x, y, self.x(), self.y()) > 5.0 {
            return false;
        }
        let Some((ext, com)) = self.script_extents() else {
            return false;
        };
        let c = self.local_pos(x, y, z);
        let (x_min, x_max) = (com.x - ext.x / 2.0, com.x + ext.x / 2.0);
        let (y_min, y_max) = (com.z - ext.z / 2.0, com.z + ext.z / 2.0);
        if c.x > x_min && c.x < x_max && c.z > y_min && c.z < y_max {
            return true;
        }
        let cx = jmath::clamp(c.x, x_min, x_max);
        let cy = jmath::clamp(c.z, y_min, y_max);
        let dx = c.x - cx;
        let dy = c.z - cy;
        dx * dx + dy * dy < radius * radius
    }

    /// `blocked(x, y, z)`.
    pub fn blocked(&self, x: i32, y: i32, z: i32) -> bool {
        if self.st().removed_from_world || self.st().current.is_none() || !self.has_controller() {
            return false;
        }
        if z != self.zi() {
            return false;
        }
        if jmath::dist_2d(x as f32 + 0.5, y as f32 + 0.5, self.x(), self.y()) > 5.0 {
            return false;
        }
        let Some((ext, com)) = self.script_extents() else {
            return false;
        };
        let mut xfrm = self.world_transform();
        xfrm.inverse();
        let (ox, oy) = env::offset();
        let mut c = Vector3f::new(x as f32 + 0.5 - ox, 0.0, y as f32 + 0.5 - oy);
        xfrm.transform(&mut c);
        let cx = jmath::clamp(c.x, com.x - ext.x / 2.0, com.x + ext.x / 2.0);
        let cy = jmath::clamp(c.z, com.z - ext.z / 2.0, com.z + ext.z / 2.0);
        let dx = c.x - cx;
        let dy = c.z - cy;
        dx * dx + dy * dy < 0.09
    }

    /// `isCharacterAdjacentTo(chr)` for a character at `(x, y, z)`.
    pub fn is_character_adjacent_to(&self, x: f32, y: f32, z: f32) -> bool {
        if fastfloor(z) != self.zi() {
            return false;
        }
        let Some((ext, com)) = self.script_extents() else {
            return false;
        };
        let mut xfrm = self.world_transform();
        xfrm.inverse();
        let (ox, oy) = env::offset();
        let mut c = Vector3f::new(x - ox, 0.0, y - oy);
        xfrm.transform(&mut c);
        let (x_min, x_max) = (com.x - ext.x / 2.0, com.x + ext.x / 2.0);
        let (y_min, y_max) = (com.z - ext.z / 2.0, com.z + ext.z / 2.0);
        c.x >= x_min - 0.5 && c.x < x_max + 0.5 && c.z >= y_min - 0.5 && c.z < y_max + 0.5
    }

    /// `isPositionOnLeftOrRight(x, y)`.
    pub fn is_position_on_left_or_right(&self, x: f32, y: f32) -> bool {
        let lx = self.local_pos(x, y, 0.0).x;
        let Some((ext, com)) = self.script_extents() else {
            return false;
        };
        let x_min = com.x - ext.x / 2.0;
        let x_max = com.x + ext.x / 2.0;
        lx < x_min * 0.98 || lx > x_max * 0.98
    }

    /// `isPointLeftOfCenter(x, y)`.
    pub fn is_point_left_of_center(&self, x: f32, y: f32) -> bool {
        let f = self.forward_vector();
        jmath::is_left(self.x(), self.y(), self.x() + f.x, self.y() + f.z, x, y) < 0.0
    }

    /// `testCollisionWithVehicle(other)`.
    pub fn test_collision_with_vehicle(&self, other: &BaseVehicle) -> bool {
        let (Some((ea, ca)), Some((eb, cb))) = (self.script_extents(), other.script_extents())
        else {
            return false;
        };
        let quad = |v: &BaseVehicle, e: Vector3f, c: Vector3f| {
            let pts = [
                (c.x + e.x * 0.5, c.z + e.z * 0.5),
                (c.x - e.x * 0.5, c.z + e.z * 0.5),
                (c.x - e.x * 0.5, c.z - e.z * 0.5),
                (c.x + e.x * 0.5, c.z - e.z * 0.5),
            ];
            pts.map(|(x, z)| {
                let w = v.world_pos(x, 0.0, z);
                Vector2::new(w.x, w.y)
            })
        };
        quadrangles_intersect(&quad(self, ea, ca), &quad(other, eb, cb))
    }

    /// `testCollisionWithCharacter(chr, circleRadius, out)` for a character
    /// whose next position is `(nx, ny)` and distance (`DistToProper`) is
    /// `dist`.
    pub fn test_collision_with_character(
        &self,
        nx: f32,
        ny: f32,
        dist: f32,
        circle_radius: f32,
    ) -> Option<(f32, f32)> {
        if !self.has_controller() {
            return None;
        }
        let (ext, com) = self.script_extents()?;
        if dist > (ext.x / 2.0).max(ext.z / 2.0) + circle_radius + 1.0 {
            return None;
        }
        self.collision_test_local(self.local_pos(nx, ny, 0.0), ext, com, circle_radius)
    }

    /// `testCollisionWithObject` for an object at `(x, y)`.
    pub fn test_collision_with_point(
        &self,
        x: f32,
        y: f32,
        circle_radius: f32,
    ) -> Option<(f32, f32)> {
        if !self.has_controller() {
            return None;
        }
        let (ext, com) = self.script_extents()?;
        let r = (ext.x / 2.0).max(ext.z / 2.0) + circle_radius + 1.0;
        if jmath::dist_sq(self.x(), self.y(), x, y) > r * r {
            return None;
        }
        self.collision_test_local(self.local_pos(x, y, 0.0), ext, com, circle_radius)
    }

    fn collision_test_local(
        &self,
        c: Vector3f,
        ext: Vector3f,
        com: Vector3f,
        circle_radius: f32,
    ) -> Option<(f32, f32)> {
        let (x_min, x_max) = (com.x - ext.x / 2.0, com.x + ext.x / 2.0);
        let (y_min, y_max) = (com.z - ext.z / 2.0, com.z + ext.z / 2.0);
        let finish = |mut v: Vector3f| {
            let mut xfrm = self.world_transform();
            xfrm.origin = Vector3f::default();
            xfrm.transform(&mut v);
            v.x += self.x();
            v.z += self.y();
            let mut st = self.st_mut();
            st.collide_x = v.x;
            st.collide_y = v.z;
            Some((v.x, v.z))
        };
        if c.x > x_min && c.x < x_max && c.z > y_min && c.z < y_max {
            let dw = c.x - x_min;
            let de = x_max - c.x;
            let dn = c.z - y_min;
            let ds = y_max - c.z;
            let mut v = Vector3f::default();
            if dw < de && dw < dn && dw < ds {
                v.set(x_min - circle_radius - 0.015, 0.0, c.z);
            } else if de < dw && de < dn && de < ds {
                v.set(x_max + circle_radius + 0.015, 0.0, c.z);
            } else if dn < dw && dn < de && dn < ds {
                v.set(c.x, 0.0, y_min - circle_radius - 0.015);
            } else if ds < dw && ds < de && ds < dn {
                v.set(c.x, 0.0, y_max + circle_radius + 0.015);
            }
            return finish(v);
        }
        let cx = jmath::clamp(c.x, x_min, x_max);
        let cy = jmath::clamp(c.z, y_min, y_max);
        let dx = c.x - cx;
        let dy = c.z - cy;
        if dx * dx + dy * dy < circle_radius * circle_radius {
            if dx == 0.0 && dy == 0.0 {
                return Some((-1.0, -1.0));
            }
            let mut v = Vector3f::new(dx, 0.0, dy);
            v.normalize();
            v.mul(circle_radius + 0.015);
            v.x += cx;
            v.z += cy;
            return finish(v);
        }
        None
    }

    // ---- areas ------------------------------------------------------------

    pub(crate) fn area(&self, id: &str) -> Option<Area> {
        let s = self.script()?;
        let s = s.borrow();
        s.area_by_id(Some(id)).cloned()
    }

    /// `areaPositionWorld(area)`.
    pub fn area_position_world(&self, area: &Area) -> Vector2 {
        let w = self.world_pos(area.x, 0.0, area.y);
        Vector2::new(w.x, w.y)
    }

    /// `areaPositionLocal(area)`.
    pub fn area_position_local(&self, area: &Area) -> Vector2 {
        let c = self.area_position_world(area);
        let l = self.local_pos(c.x, c.y, 0.0);
        Vector2::new(l.x, l.z)
    }

    /// `areaPositionWorld4PlayerInteract(area)`.
    pub fn area_position_world_4_player_interact(&self, area: &Area) -> Vector2 {
        let Some((ext, com)) = self.script_extents() else {
            return Vector2::default();
        };
        let p = self.area_position_world(area);
        let mut v = self.local_pos(p.x, p.y, 0.0);
        if area.x > com.x + ext.x / 2.0 || area.x < com.x - ext.x / 2.0 {
            if area.x > 0.0 {
                v.x -= area.w * 0.3;
            } else {
                v.x += area.w * 0.3;
            }
        } else if area.y > 0.0 {
            v.z -= area.h * 0.3;
        } else {
            v.z += area.h * 0.3;
        }
        let w = self.world_pos(v.x, v.y, v.z);
        Vector2::new(w.x, w.y)
    }

    /// `getAreaCenter(areaId)`.
    pub fn area_center(&self, area_id: &str) -> Option<Vector2> {
        let a = self.area(area_id)?;
        Some(self.area_position_world(&a))
    }

    /// `isInArea(areaId, x, y)`.
    pub fn is_in_area(&self, area_id: &str, x: f32, y: f32) -> bool {
        let Some(a) = self.area(area_id) else {
            return false;
        };
        let c = self.area_position_local(&a);
        let l = self.local_pos(x, y, self.z());
        let (min_x, min_y) = (c.x - a.w / 2.0, c.y - a.h / 2.0);
        let (max_x, max_y) = (c.x + a.w / 2.0, c.y + a.h / 2.0);
        l.x >= min_x && l.x < max_x && l.z >= min_y && l.z < max_y
    }

    /// `getAreaDist(areaId, x, y, z)`.
    pub fn area_dist(&self, area_id: &str, x: f32, y: f32, z: f32) -> f32 {
        let Some(a) = self.area(area_id) else {
            return 999.0;
        };
        let l = self.local_pos(x, y, z);
        let min_x = (a.x - a.w / 2.0).abs();
        let min_y = (a.y - a.h / 2.0).abs();
        (l.x + min_x).abs() + (l.z + min_y).abs()
    }

    /// `getFacingPosition(x, y, z, out)`.
    pub fn facing_position(&self, x: f32, y: f32, z: f32) -> Vector2 {
        let c = self.local_pos(x, y, z);
        let Some((ext, com)) = self.script_extents() else {
            return Vector2::default();
        };
        let (x_min, x_max) = (com.x - ext.x / 2.0, com.x + ext.x / 2.0);
        let (y_min, y_max) = (com.z - ext.z / 2.0, com.z + ext.z / 2.0);
        let (mut lx, mut ly) = (0.0, 0.0);
        if (c.x <= 0.0 || c.x > 0.0) && c.z >= y_min && c.z <= y_max {
            ly = c.z;
        } else if (c.z <= 0.0 || c.z > 0.0) && c.x >= x_min && c.x <= x_max {
            lx = c.x;
        }
        let w = self.world_pos(lx, 0.0, ly);
        Vector2::new(w.x, w.y)
    }

    // ---- passengers ---------------------------------------------------------

    /// `getPassengerPosition(seat, id)`.
    pub fn passenger_position(&self, seat: i32, id: &str) -> Option<PassengerPosition> {
        self.script_passenger(seat, |p| {
            p.position_by_id(id).map(|q| PassengerPosition {
                offset: v3(q.offset),
                area: q.area.clone(),
            })
        })?
    }

    /// `getPassengerPositionWorldPos(x, y, z, out)`.
    pub fn passenger_position_world_pos_xyz(&self, x: f32, y: f32, z: f32) -> Vector3f {
        let off = self
            .script()
            .and_then(|s| s.borrow().model().map(|m| v3(m.offset)))
            .unwrap_or_default();
        let mut out = off;
        out.add(x, y, z);
        let mut w = self.world_pos(out.x, out.y, out.z);
        w.z = self.zi() as f32;
        w
    }

    /// `getPassengerPositionWorldPos(posn, out)`.
    pub fn passenger_position_world_pos(&self, p: &PassengerPosition) -> Vector3f {
        self.passenger_position_world_pos_xyz(p.offset.x, p.offset.y, p.offset.z)
    }

    /// `getPassengerWorldPos(seat, out)`.
    pub fn passenger_world_pos(&self, seat: i32) -> Option<Vector3f> {
        let off = self.st().passengers.get(seat as usize)?.offset;
        Some(self.passenger_position_world_pos_xyz(off.x, off.y, off.z))
    }

    /// `getEnterSeatDistance(seat, x, y)`.
    pub fn enter_seat_distance(&self, seat: i32, x: f32, y: f32) -> f32 {
        for id in ["outside", "outside2"] {
            if let Some(p) = self.passenger_position(seat, id) {
                let w = self.passenger_position_world_pos(&p);
                return jmath::dist_sq(x, y, w.x, w.y);
            }
        }
        -1.0
    }

    // ---- attachments --------------------------------------------------------

    /// `getAttachmentLocalPos(name, v)`.
    pub fn attachment_local_pos(&self, name: &str) -> Option<Vector3f> {
        let s = self.script()?;
        let s = s.borrow();
        let a = s.attachment_by_id(Some(name))?;
        let o = a.offset();
        let mut v = Vector3f::new(o.x, o.y, o.z);
        if let Some(m) = s.model() {
            v.add(m.offset.x, m.offset.y, m.offset.z);
        }
        Some(v)
    }

    /// `getAttachmentWorldPos(name, v)`.
    pub fn attachment_world_pos(&self, name: &str) -> Option<Vector3f> {
        let l = self.attachment_local_pos(name)?;
        Some(self.world_pos(l.x, l.y, l.z))
    }

    /// `isBeingTowedBackwards()`.
    pub fn is_being_towed_backwards(&self) -> bool {
        if self.vehicle_towed_by().is_none() {
            return false;
        }
        let a = self.st().tow_attachment_self.clone();
        a.and_then(|a| self.attachment_local_pos(&a))
            .is_some_and(|l| l.z < 0.0)
    }

    /// `getAngleX/Y/Z()` in degrees.
    pub fn angles(&self) -> Vector3f {
        let e = self.st().jni_transform.rotation().euler_angles_xyz();
        Vector3f::new(e.x * 57.295776, e.y * 57.295776, e.z * 57.295776)
    }

    /// `postupdate()`: `current`/`square` from the position.
    pub fn postupdate(&self) {
        let (x, y, z) = (self.xi(), self.yi(), self.zi());
        let sq = env::world().square(x, y, z).map(|_| (x, y, z));
        let mut st = self.st_mut();
        st.current = sq;
        st.square = sq;
    }
}

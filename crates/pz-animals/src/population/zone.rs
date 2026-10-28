//! `AnimalZone`'s polyline geometry and `AnimalZoneJunction`, over the
//! zones the meta grid holds (`pz_world::zones::Zone` with
//! `ZoneKind::Animal`).

use pz_world::zones::{AnimalZoneData, AnimalZoneJunction, Zone, ZoneId, ZoneKind};
use pz_world::IsoMetaGrid;

/// JOML `Vector2f.length(x, y)`.
fn vlen(x: f32, y: f32) -> f32 {
    ((x * x + y * y) as f64).sqrt() as f32
}

/// The square-centred point `i` (`points.get(i) + 0.5f`, `i` modulo the
/// list as the original's `(i + 2) % size`).
fn pt(z: &Zone, i: usize) -> f32 {
    z.points[i % z.points.len()] as f32 + 0.5
}

fn segment(z: &Zone, i: usize) -> (f32, f32, f32, f32) {
    (pt(z, i), pt(z, i + 1), pt(z, i + 2), pt(z, i + 3))
}

/// The animal fields of a zone (`None` when it is not an `AnimalZone`).
pub fn animal(z: &Zone) -> Option<&AnimalZoneData> {
    match &z.kind {
        ZoneKind::Animal(a) => Some(a),
        _ => None,
    }
}

fn animal_mut(z: &mut Zone) -> Option<&mut AnimalZoneData> {
    match &mut z.kind {
        ZoneKind::Animal(a) => Some(a),
        _ => None,
    }
}

/// `zone.action`.
pub fn action(z: &Zone) -> Option<&str> {
    animal(z)?.action.as_deref()
}

/// `getPolylineLength()`.
pub fn polyline_length(z: &Zone) -> f32 {
    z.get_polyline_length()
}

/// `getPolylineSegment(t)`: the index of the first coordinate of the
/// segment at `t`, or -1.
pub fn polyline_segment(z: &Zone, t: f32) -> i32 {
    let t = t.clamp(0.0, 1.0);
    let length = polyline_length(z);
    if length <= 0.0 {
        return -1;
    }
    let distance_from_start = length * t;
    let mut segment_start = 0.0f32;
    let mut i = 0usize;
    while (i as i32) < z.points.len() as i32 - 2 {
        let (x1, y1, x2, y2) = segment(z, i);
        let segment_length = vlen(x2 - x1, y2 - y1);
        if segment_start + segment_length >= distance_from_start {
            return i as i32;
        }
        segment_start += segment_length;
        i += 2;
    }
    -1
}

/// `getPointOnPolyline(t, out)`: whether a point was found, and `out`
/// (which the original zeroes first).
pub fn point_on_polyline(z: &Zone, t: f32) -> (bool, (f32, f32)) {
    let t = t.clamp(0.0, 1.0);
    let length = polyline_length(z);
    if length <= 0.0 {
        return (false, (0.0, 0.0));
    }
    let distance_from_start = length * t;
    let mut segment_start = 0.0f32;
    let mut i = 0usize;
    while (i as i32) < z.points.len() as i32 - 2 {
        let (x1, y1, x2, y2) = segment(z, i);
        let segment_length = vlen(x2 - x1, y2 - y1);
        if segment_start + segment_length >= distance_from_start {
            let f = (distance_from_start - segment_start) / segment_length;
            return (true, (x1 + (x2 - x1) * f, y1 + (y2 - y1) * f));
        }
        segment_start += segment_length;
        i += 2;
    }
    (false, (0.0, 0.0))
}

/// `getClosestPointOnPolyline(px, py, out)`: the fraction along the
/// polyline (-1 for a zone that is neither a polyline nor has three
/// points, leaving `out` alone).
pub fn closest_point_on_polyline(z: &Zone, px: f32, py: f32, out: &mut (f32, f32)) -> f32 {
    if !z.is_polyline() && z.points.len() < 6 {
        return -1.0;
    }
    let mut closest_dist = f32::MAX;
    let mut distance_from_start = 0.0f32;
    let mut length = 0.0f32;
    let mut i = 0usize;
    while (i as i32) < z.points.len() as i32 - 2 {
        let (x1, y1, x2, y2) = segment(z, i);
        // Vector2f.distance(x1, y1, x2, y2).
        let (dx, dy) = (x1 - x2, y1 - y2);
        let segment_length = ((dx * dx + dy * dy) as f64).sqrt() as f32;
        let mut u = ((px - x1) * (x2 - x1) + (py - y1) * (y2 - y1)) as f64
            / (((x2 - x1) as f64).powf(2.0) + ((y2 - y1) as f64).powf(2.0));
        let mut xu = x1 as f64 + u * (x2 - x1) as f64;
        let mut yu = y1 as f64 + u * (y2 - y1) as f64;
        if u <= 0.0 {
            xu = x1 as f64;
            yu = y1 as f64;
            u = 0.0;
        } else if u >= 1.0 {
            xu = x2 as f64;
            yu = y2 as f64;
            u = 1.0;
        }
        // IsoUtils.DistanceToSquared.
        let (ex, ey) = (xu as f32 - px, yu as f32 - py);
        let dist = ex * ex + ey * ey;
        if dist < closest_dist {
            closest_dist = dist;
            *out = (xu as f32, yu as f32);
            distance_from_start = length + (u * segment_length as f64) as f32;
        }
        length += segment_length;
        i += 2;
    }
    distance_from_start / length
}

/// `getDistanceOfPointFromStart(pointIndex)`.
pub fn distance_of_point_from_start(z: &Zone, point_index: i32) -> f32 {
    let mut length = 0.0f32;
    let mut i = 0i32;
    while i < point_index.wrapping_mul(2) {
        let (x1, y1, x2, y2) = segment(z, i as usize);
        length += vlen(x2 - x1, y2 - y1);
        i += 2;
    }
    length
}

/// `getDirectionOnPolyline(t, out)`: the normalised direction of the
/// segment at `t` (`None`: `out` is left alone).
pub fn direction_on_polyline(z: &Zone, t: f32) -> Option<(f32, f32)> {
    let i = polyline_segment(z, t);
    if i == -1 {
        return None;
    }
    let (x1, y1, x2, y2) = segment(z, i as usize);
    let (x, y) = (x2 - x1, y2 - y1);
    // Vector2f.normalize: 1 / (float) sqrt(x * x + y * y).
    let inv = 1.0f32 / ((x * x + y * y) as f64).sqrt() as f32;
    Some((x * inv, y * inv))
}

/// `getIndexOfPoint(x, y)`.
pub fn index_of_point(z: &Zone, x: i32, y: i32) -> i32 {
    let mut i = 0;
    while i + 1 < z.points.len() {
        if z.points[i] == x && z.points[i + 1] == y {
            return (i / 2) as i32;
        }
        i += 2;
    }
    -1
}

/// An `AnimalZoneJunction` with the zone it belongs to (`zoneSelf`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Junction {
    pub zone_self: ZoneId,
    pub point_index_self: i32,
    pub zone_other: ZoneId,
    pub point_index_other: i32,
    /// `distanceFromStart` (set by the constructor).
    pub distance_from_start: f32,
    /// `isFirstPointOnZone1()` / `isLastPointOnZone1()`.
    pub first_point_on_zone1: bool,
    pub last_point_on_zone1: bool,
    /// `isFirstPointOnZone2()` / `isLastPointOnZone2()`.
    pub first_point_on_zone2: bool,
    pub last_point_on_zone2: bool,
}

impl Junction {
    fn of(meta: &IsoMetaGrid, zone_self: ZoneId, j: &AnimalZoneJunction) -> Junction {
        let zs = meta.zone(zone_self);
        let zo = meta.zone(j.zone_other);
        Junction {
            zone_self,
            point_index_self: j.point_index_self,
            zone_other: j.zone_other,
            point_index_other: j.point_index_other,
            distance_from_start: distance_of_point_from_start(zs, j.point_index_self),
            first_point_on_zone1: j.point_index_self == 0,
            last_point_on_zone1: j.point_index_self == zs.points.len() as i32 / 2 - 1,
            first_point_on_zone2: j.point_index_other == 0,
            last_point_on_zone2: j.point_index_other == zo.points.len() as i32 / 2 - 1,
        }
    }

    /// `getX()` / `getY()`.
    pub fn xy(&self, meta: &IsoMetaGrid) -> (i32, i32) {
        let z = meta.zone(self.zone_self);
        let i = self.point_index_self as usize * 2;
        (z.points[i], z.points[i + 1])
    }
}

/// `zone.junctions` (empty when the zone has none, or is not an animal
/// zone).
pub fn junctions(meta: &IsoMetaGrid, zone: ZoneId) -> Vec<Junction> {
    animal(meta.zone(zone))
        .and_then(|a| a.junctions.as_ref())
        .map(|js| js.iter().map(|j| Junction::of(meta, zone, j)).collect())
        .unwrap_or_default()
}

/// `getJunctionsBetween(t1, t2, junctions)`. The original reads
/// `this.junctions` without a null check; a zone without junctions has
/// none between any two points here.
pub fn junctions_between(meta: &IsoMetaGrid, zone: ZoneId, t1: f32, t2: f32) -> Vec<Junction> {
    let length = polyline_length(meta.zone(zone));
    junctions(meta, zone)
        .into_iter()
        .filter(|j| j.distance_from_start / length >= t1 && j.distance_from_start / length <= t2)
        .collect()
}

fn has_junction(z: &Zone, pis: i32, other: ZoneId, pio: i32) -> bool {
    animal(z)
        .and_then(|a| a.junctions.as_ref())
        .is_some_and(|js| {
            js.iter().any(|j| j.point_index_self == pis && j.zone_other == other && j.point_index_other == pio)
        })
}

/// `addJunction(pointIndexSelf, other, pointIndexOther)`.
pub fn add_junction(meta: &mut IsoMetaGrid, zone: ZoneId, pis: i32, other: ZoneId, pio: i32) {
    if has_junction(meta.zone(zone), pis, other, pio) {
        return;
    }
    if pis < 0 || pis >= meta.zone(zone).points.len() as i32 / 2 {
        return;
    }
    if other == zone || pio < 0 || pio >= meta.zone(other).points.len() as i32 / 2 {
        return;
    }
    if let Some(a) = animal_mut(meta.zone_mut(zone)) {
        a.junctions.get_or_insert_with(Vec::new).push(AnimalZoneJunction {
            point_index_self: pis,
            zone_other: other,
            point_index_other: pio,
        });
    }
}

/// `addJunctionsWithOtherZone(other)`.
pub fn add_junctions_with_other_zone(meta: &mut IsoMetaGrid, zone: ZoneId, other: ZoneId) {
    let points = meta.zone(zone).points.clone();
    let mut i = 0;
    while i + 1 < points.len() {
        let index = index_of_point(meta.zone(other), points[i], points[i + 1]);
        if index != -1 {
            add_junction(meta, zone, (i / 2) as i32, other, index);
            add_junction(meta, other, index, zone, (i / 2) as i32);
        }
        i += 2;
    }
}

/// `animalZoneHandler.getZone(uuid)`.
pub fn zone_by_uuid(meta: &IsoMetaGrid, msb: i64, lsb: i64) -> Option<ZoneId> {
    meta.animal_zones
        .iter()
        .copied()
        .find(|&id| meta.zone(id).id.is_some_and(|u| u.msb == msb && u.lsb == lsb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::zones::ZoneGeometryType;

    pub(crate) fn polyline(points: &[i32], action: &str) -> Zone {
        let mut z = Zone::default();
        z.geometry_type = ZoneGeometryType::Polyline;
        z.points = points.to_vec();
        z.kind = ZoneKind::Animal(AnimalZoneData {
            action: Some(action.into()),
            ..Default::default()
        });
        z
    }

    #[test]
    fn polyline_points_and_directions() {
        let z = polyline(&[0, 0, 10, 0, 10, 10], "Follow");
        assert_eq!(polyline_length(&z), 20.0);
        assert_eq!(point_on_polyline(&z, 0.25), (true, (5.5, 0.5)));
        assert_eq!(point_on_polyline(&z, 0.75), (true, (10.5, 5.5)));
        assert_eq!(direction_on_polyline(&z, 0.75), Some((0.0, 1.0)));
        assert_eq!(polyline_segment(&z, 0.0), 0);
        let mut out = (0.0, 0.0);
        let t = closest_point_on_polyline(&z, 4.5, 3.0, &mut out);
        assert_eq!(out, (4.5, 0.5));
        assert_eq!(t, 0.2);
        assert_eq!(distance_of_point_from_start(&z, 2), 20.0);
        assert_eq!(index_of_point(&z, 10, 10), 2);
        let empty = polyline(&[3, 3], "Follow");
        assert_eq!(point_on_polyline(&empty, 0.5), (false, (0.0, 0.0)));
        assert_eq!(direction_on_polyline(&empty, 0.5), None);
    }
}

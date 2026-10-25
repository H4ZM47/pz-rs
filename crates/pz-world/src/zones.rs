//! Map zones: `zombie.iso.zones.Zone` and the subclasses map loading creates
//! (`VehicleZone`, `IsoMannequin.MannequinZone`, `WorldGenZone`,
//! `AnimalZone`), `RoomTone`, plus the pure geometry they depend on
//! (`LiangBarsky`, `Vector2` length/normalize).
//!
//! Zones live in an arena owned by [`crate::IsoMetaGrid`] and are referred to
//! by [`ZoneId`]; Java compares zones by identity, which the id reproduces.
//!
//! Integer arithmetic on zone coordinates wraps like Java `int` (a polygon
//! registered through `registerGeometryZone` has bounds `Integer.MAX_VALUE`
//! with width 2, and the original relies on the overflow).
//!
//! The polyline outline and polygon clipping are done by the native
//! `PZClipper` library upstream; they are supplied through [`ClipperOps`].

use std::collections::HashMap;

use crate::java::fastfloor;

/// Arena index of a [`Zone`] inside an `IsoMetaGrid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ZoneId(pub u32);

/// `IsoMetaGrid.ANY_Z`.
pub const ANY_Z: i32 = i32::MAX;

/// `ZoneGeometryType` (ordinal order matters for save files).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZoneGeometryType {
    #[default]
    Invalid,
    Point,
    Polyline,
    Polygon,
}

/// Values of a Lua properties table that zone constructors look at.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Nil,
    Bool(bool),
    Number(f64),
    Str(String),
    /// Any other Lua value (tables, functions...).
    Other,
}

/// Read access to a Kahlua properties table (`KahluaTable.rawget`).
pub trait ZoneProps {
    fn rawget(&self, key: &str) -> PropValue;
}

impl ZoneProps for HashMap<String, PropValue> {
    fn rawget(&self, key: &str) -> PropValue {
        self.get(key).cloned().unwrap_or(PropValue::Nil)
    }
}

/// Game tables the zone code consults. These are constants in the Java
/// classes and are read from `projectzomboid.jar` at runtime, never
/// committed.
#[derive(Clone, Debug, Default)]
pub struct ZoneTables {
    /// `Zone.s_PreferredZoneTypes`.
    pub preferred_zone_types: Vec<String>,
    /// `IsoDirections` constant names in ordinal order (`valueOf`).
    pub iso_direction_names: Vec<String>,
}

impl ZoneTables {
    /// LOADER STUB: to be filled from `projectzomboid.jar` through
    /// `pz-classfile` (`zombie.iso.zones.Zone.<clinit>` and the
    /// `zombie.iso.IsoDirections` enum). Returns `None` until that lands.
    pub fn load_from_game_jar(_jar: &std::path::Path) -> Option<ZoneTables> {
        None
    }

    /// `Zone.isPreferredZoneForSquare(type)`.
    pub fn is_preferred_zone_for_square(&self, zone_type: &str) -> bool {
        self.preferred_zone_types.iter().any(|t| t == zone_type)
    }

    /// `IsoDirections.valueOf(name)` as an ordinal.
    pub fn direction_value_of(&self, name: &str) -> Result<usize, ZoneError> {
        self.iso_direction_names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| ZoneError::InvalidDirection(name.to_string()))
    }
}

/// Exceptions the Java code would throw while registering zones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneError {
    /// `IllegalArgumentException("unknown zone geometry type")`.
    UnknownGeometry(String),
    /// Points table with an odd length (`NullPointerException` upstream).
    OddPointCount,
    /// `IsoDirections.valueOf` failed.
    InvalidDirection(String),
    /// `ClassCastException` reading a table value.
    WrongPropertyType(&'static str),
    /// `NullPointerException`: missing properties table.
    MissingProperties,
    /// `NullPointerException`: `registerRoomTone` outside any meta cell.
    NoCell,
}

impl std::fmt::Display for ZoneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ZoneError {}

/// The native `PZClipper` operations zones use.
pub trait ClipperOps {
    /// `ClipperOffset`: `addPath(points, Miter, Butt)`, `execute(delta)`,
    /// then polygon 0 as flat `x, y` floats. `None` when no polygon results.
    fn offset_polyline(&mut self, points: &[f32], delta: f64) -> Option<Vec<f32>>;

    /// `Clipper`: `addPath(polygon, false)`, `clipAABB(x1, y1, x2, y2)`,
    /// `generatePolygons()`; every resulting polygon as flat `x, y` floats.
    fn clip_polygon_aabb(
        &mut self,
        points: &[f32],
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> Vec<Vec<f32>>;
}

/// `VehicleZone` extra fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleZoneData {
    /// `IsoDirections` ordinal.
    pub dir: Option<usize>,
    pub flags: i16,
}

impl VehicleZoneData {
    pub const VZF_FACE_DIRECTION: i16 = 1;
}

/// A `VehicleZone` as the vehicle code reads it (`IsoChunk.AddVehicles`,
/// `BaseVehicle`): the `Zone` fields plus `dir` and the face-direction
/// flag. A copy of the zone at the time it was taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleZone {
    pub name: String,
    pub zone_type: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    /// `dir` (the `Direction` property).
    pub dir: Option<crate::IsoDirections>,
    /// `VZF_FaceDirection`.
    pub face_direction: bool,
    pub geometry_type: ZoneGeometryType,
    /// Polyline or polygon points (x, y pairs); empty for a rectangle.
    pub points: Vec<i32>,
    pub polyline_width: i32,
}

impl VehicleZone {
    /// The view of `zone`, `None` unless it is a `VehicleZone`.
    pub fn from_zone(zone: &Zone) -> Option<VehicleZone> {
        let v = zone.vehicle()?;
        Some(VehicleZone {
            name: zone.name.clone(),
            zone_type: zone.zone_type.clone(),
            x: zone.x,
            y: zone.y,
            z: zone.z,
            w: zone.w,
            h: zone.h,
            dir: v
                .dir
                .and_then(|d| crate::IsoDirections::ALL.get(d).copied()),
            face_direction: zone.is_face_direction(),
            geometry_type: zone.geometry_type,
            points: zone.points.clone(),
            polyline_width: zone.polyline_width,
        })
    }

    /// `isPolyline()`.
    pub fn is_polyline(&self) -> bool {
        self.geometry_type == ZoneGeometryType::Polyline
    }

    /// `isRectangle()`.
    pub fn is_rectangle(&self) -> bool {
        self.geometry_type == ZoneGeometryType::Invalid
    }

    /// `getPolylineLength()`.
    pub fn polyline_length(&self) -> f32 {
        if !self.is_polyline() || self.points.is_empty() {
            return 0.0;
        }
        let mut length = 0.0f32;
        let mut i = 0usize;
        while (i as i32) < self.points.len() as i32 - 2 {
            let dx = self.points[i + 2].wrapping_sub(self.points[i]) as f32;
            let dy = self.points[i + 3].wrapping_sub(self.points[i + 1]) as f32;
            length += vec_length(dx, dy);
            i += 2;
        }
        length
    }
}

/// `IsoMannequin.MannequinZone` extra fields.
#[derive(Clone, Debug, PartialEq)]
pub struct MannequinZoneData {
    pub female: i32,
    pub dir: Option<usize>,
    pub mannequin_script: Option<String>,
    pub pose: Option<String>,
    pub skin: Option<String>,
    pub outfit: Option<String>,
}

/// `AnimalZone` fields: the map data, `spawnedAnimals` and the junctions
/// with other animal zones (the animal runtime's paths are not ported).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalZoneData {
    pub action: Option<String>,
    pub animal_type: Option<String>,
    /// `spawnAnimal` (true unless the map says `SpawnAnimals = false`).
    pub spawn_animal: bool,
    /// `spawnedAnimals`.
    pub spawned_animals: bool,
    /// `junctions` (`null` until the first one).
    pub junctions: Option<Vec<AnimalZoneJunction>>,
}

impl Default for AnimalZoneData {
    /// `new AnimalZone()`: `spawnAnimal = true`.
    fn default() -> Self {
        AnimalZoneData {
            action: None,
            animal_type: None,
            spawn_animal: true,
            spawned_animals: false,
            junctions: None,
        }
    }
}

/// `AnimalZoneJunction`: a point this zone shares with another animal zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimalZoneJunction {
    pub point_index_self: i32,
    pub zone_other: ZoneId,
    pub point_index_other: i32,
}

/// Which Java class the zone is.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ZoneKind {
    #[default]
    Zone,
    Vehicle(VehicleZoneData),
    Mannequin(MannequinZoneData),
    /// `WorldGenZone.rocks`.
    WorldGen {
        rocks: bool,
    },
    Animal(AnimalZoneData),
}

/// `zombie.iso.zones.RoomTone`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomTone {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub enum_value: Option<String>,
    pub entire_building: bool,
}

/// `zombie.iso.zones.Zone` (map-data fields and geometry).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zone {
    pub name: String,
    pub zone_type: String,
    original_name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    pub geometry_type: ZoneGeometryType,
    pub points: Vec<i32>,
    pub polyline_width: i32,
    pub polyline_outline_points: Option<Vec<f32>>,
    pub is_preferred_zone_for_square: bool,
    pub hour_last_seen: i32,
    pub last_action_timestamp: i32,
    pub have_construction: bool,
    pub kind: ZoneKind,
    /// `id`. The original draws a random UUID in the constructor; here it
    /// stays unset until a save assigns one (see `zone_save`).
    pub id: Option<pz_kahlua::java_io::JavaUuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PolygonHit {
    Inside,
    Outside,
}

/// `Vector2.getLength()` of `(x, y)`.
fn vec_length(x: f32, y: f32) -> f32 {
    ((x * x + y * y) as f64).sqrt() as f32
}

/// `Vector2.normalize()` applied to `(x, y)`.
fn vec_normalize(x: f32, y: f32) -> (f32, f32) {
    let len_sq = x * x + y * y;
    if (1.0f32 - len_sq).abs() < 1.0e-5 {
        return (x, y);
    }
    if len_sq == 0.0 {
        return (0.0, 0.0);
    }
    let len = (len_sq as f64).sqrt() as f32;
    (x / len, y / len)
}

/// `LiangBarsky.lineRectIntersect`; `Some((t1, t2))` on intersection.
#[allow(clippy::too_many_arguments)]
pub fn line_rect_intersect(
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> Option<(f64, f64)> {
    let p = [(-dx) as f64, dx as f64, (-dy) as f64, dy as f64];
    let q = [
        (x - left) as f64,
        (right - x) as f64,
        (y - top) as f64,
        (bottom - y) as f64,
    ];
    let mut u1 = 0.0f64;
    let mut u2 = 1.0f64;
    for i in 0..4 {
        if p[i] == 0.0 {
            if q[i] < 0.0 {
                return None;
            }
            continue;
        }
        let t = q[i] / p[i];
        if p[i] < 0.0 && u1 < t {
            u1 = t;
            continue;
        }
        if p[i] > 0.0 && u2 > t {
            u2 = t;
        }
    }
    if u1 >= u2 {
        return None;
    }
    Some((u1, u2))
}

/// `PZMath.ceil(float)`.
pub fn pz_ceil(val: f32) -> f32 {
    if val >= 0.0 {
        (val + 0.9999999f32) as i32 as f32
    } else {
        (val - 1.0e-7f32) as i32 as f32
    }
}

fn is_left(x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
}

/// Winding-number test over a closed ring of `(x, y)` pairs.
fn winding_hit(n: usize, pt: impl Fn(usize) -> (f32, f32), x: f32, y: f32) -> PolygonHit {
    let mut wn = 0i32;
    let mut i = 0;
    while i < n {
        let (x1, y1) = pt(i);
        let (x2, y2) = pt((i + 2) % n);
        if y1 <= y {
            if y2 > y && is_left(x1, y1, x2, y2, x, y) > 0.0 {
                wn += 1;
            }
        } else if y2 <= y && is_left(x1, y1, x2, y2, x, y) < 0.0 {
            wn -= 1;
        }
        i += 2;
    }
    if wn == 0 {
        PolygonHit::Outside
    } else {
        PolygonHit::Inside
    }
}

/// Shared body of `lineSegmentIntersects` / `polylineOutlineSegmentIntersects`.
fn ring_segment_intersects(
    n: usize,
    pt: &impl Fn(usize) -> (f32, f32),
    sx: f32,
    sy: f32,
    ex: f32,
    ey: f32,
) -> bool {
    let (vx, vy) = (ex - sx, ey - sy);
    let segment_length = vec_length(vx, vy);
    let (dir_x, dir_y) = vec_normalize(vx, vy);
    let mut j = 0;
    while j < n {
        let (node1x, node1y) = pt(j);
        let (node2x, node2y) = pt((j + 2) % n);
        let dba_x = node2x - node1x;
        let doa_y = sy - node1y;
        let dba_y = node2y - node1y;
        let doa_x = sx - node1x;
        let inv_dba_dir = 1.0f32 / (dba_y * dir_x - dba_x * dir_y);
        let t = (dba_x * doa_y - dba_y * doa_x) * inv_dba_dir;
        if t >= 0.0 && t <= segment_length {
            let t2 = (doa_y * dir_x - doa_x * dir_y) * inv_dba_dir;
            if (0.0..=1.0).contains(&t2) {
                return true;
            }
        }
        j += 2;
    }
    winding_hit(n, pt, (sx + ex) / 2.0, (sy + ey) / 2.0) != PolygonHit::Outside
}

impl Zone {
    /// `new Zone(name, type, x, y, z, w, h)`. Does not set
    /// `is_preferred_zone_for_square` (the Java constructor does not either).
    pub fn new(name: &str, zone_type: &str, x: i32, y: i32, z: i32, w: i32, h: i32) -> Zone {
        Zone {
            name: name.to_string(),
            zone_type: zone_type.to_string(),
            original_name: Some(name.to_string()),
            x,
            y,
            z,
            w,
            h,
            ..Zone::default()
        }
    }

    /// The ten-argument constructor (geometry plus preferred-type flag).
    #[allow(clippy::too_many_arguments)]
    pub fn with_geometry(
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        geometry_type: ZoneGeometryType,
        points: Option<&[i32]>,
        polyline_width: i32,
        tables: &ZoneTables,
    ) -> Zone {
        let mut zone = Zone::new(name, zone_type, x, y, z, w, h);
        zone.geometry_type = geometry_type;
        if let Some(points) = points {
            zone.points.extend_from_slice(points);
            zone.polyline_width = polyline_width;
        }
        zone.is_preferred_zone_for_square = tables.is_preferred_zone_for_square(zone_type);
        zone
    }

    pub fn get_original_name(&self) -> Option<&str> {
        self.original_name.as_deref()
    }
    pub fn set_original_name(&mut self, name: Option<String>) {
        self.original_name = name;
    }

    pub fn is_point(&self) -> bool {
        self.geometry_type == ZoneGeometryType::Point
    }
    pub fn is_polygon(&self) -> bool {
        self.geometry_type == ZoneGeometryType::Polygon
    }
    pub fn is_polyline(&self) -> bool {
        self.geometry_type == ZoneGeometryType::Polyline
    }
    pub fn is_rectangle(&self) -> bool {
        self.geometry_type == ZoneGeometryType::Invalid
    }

    pub fn vehicle(&self) -> Option<&VehicleZoneData> {
        match &self.kind {
            ZoneKind::Vehicle(v) => Some(v),
            _ => None,
        }
    }

    /// `VehicleZone.isFaceDirection()`.
    pub fn is_face_direction(&self) -> bool {
        self.vehicle()
            .is_some_and(|v| v.flags & VehicleZoneData::VZF_FACE_DIRECTION != 0)
    }

    fn pt(&self, i: usize) -> (f32, f32) {
        (self.points[i] as f32, self.points[i + 1] as f32)
    }

    fn outline_pt(outline: &[f32], i: usize) -> (f32, f32) {
        (outline[i], outline[i + 1])
    }

    /// `checkPolylineOutline()`: the outline is cached once computed; a
    /// failed offset leaves it unset (and is retried on the next call).
    pub fn check_polyline_outline(&mut self, clipper: Option<&mut (dyn ClipperOps + '_)>) {
        if self.polyline_outline_points.is_some() || !self.is_polyline() || self.polyline_width <= 0
        {
            return;
        }
        let Some(clipper) = clipper else { return };
        let input = polyline_offset_input(&self.points, self.polyline_width);
        let delta = (self.polyline_width as f32 / 2.0f32) as f64;
        self.polyline_outline_points = clipper.offset_polyline(&input, delta);
    }

    /// `contains(x, y, z)`. Polylines with a width need their outline
    /// computed first ([`Zone::check_polyline_outline`]).
    pub fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        if z != self.z {
            return false;
        }
        if x < self.x || x >= self.x.wrapping_add(self.w) {
            return false;
        }
        if y < self.y || y >= self.y.wrapping_add(self.h) {
            return false;
        }
        if self.is_point() {
            return false;
        }
        if self.is_polyline() {
            if self.polyline_width > 0 {
                return self.is_point_in_polyline_winding_number(x as f32 + 0.5, y as f32 + 0.5)
                    == PolygonHit::Inside;
            }
            return false;
        }
        if self.is_polygon() {
            return self.is_point_in_polygon_winding_number(x as f32 + 0.5, y as f32 + 0.5)
                == PolygonHit::Inside;
        }
        true
    }

    /// `intersects(x, y, z, w, h)`; `z == ANY_Z` matches every level.
    pub fn intersects(&self, x: i32, y: i32, z: i32, w: i32, h: i32) -> bool {
        if z != ANY_Z && self.z != z {
            return false;
        }
        if x.wrapping_add(w) <= self.x || x >= self.x.wrapping_add(self.w) {
            return false;
        }
        if y.wrapping_add(h) <= self.y || y >= self.y.wrapping_add(self.h) {
            return false;
        }
        if self.is_polygon() {
            return self.polygon_rect_intersect(x, y, w, h);
        }
        if self.is_polyline() {
            if self.polyline_width > 0 {
                return self.polyline_outline_rect_intersect(x, y, w, h);
            }
            let mut i = 0usize;
            while (i as i32) < self.points.len() as i32 - 2 {
                let (x1, y1) = (self.points[i], self.points[i + 1]);
                let (x2, y2) = (self.points[i + 2], self.points[i + 3]);
                if line_rect_intersect(
                    x1 as f32,
                    y1 as f32,
                    x2.wrapping_sub(x1) as f32,
                    y2.wrapping_sub(y1) as f32,
                    x as f32,
                    y as f32,
                    x.wrapping_add(w) as f32,
                    y.wrapping_add(h) as f32,
                )
                .is_some()
                {
                    return true;
                }
                i += 2;
            }
            return false;
        }
        true
    }

    /// `difference(x, y, z, w, h, result)`: `None` when the zone does not
    /// intersect the rectangle, else the pieces that remain outside it.
    /// Polygon pieces come from the native clipper; without one they are
    /// empty. Pieces are built with the seven-argument constructor.
    pub fn difference(
        &self,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        clipper: Option<&mut (dyn ClipperOps + '_)>,
    ) -> Option<Vec<Zone>> {
        if !self.intersects(x, y, z, w, h) {
            return None;
        }
        let mut result = Vec::new();
        let (sx, sy, sw, sh) = (self.x, self.y, self.w, self.h);
        let piece = |px, py, pw, ph| Zone::new(&self.name, &self.zone_type, px, py, self.z, pw, ph);
        if self.is_rectangle() {
            if sx < x {
                let top = y.max(sy);
                let bottom = y.wrapping_add(h).min(sy.wrapping_add(sh));
                result.push(piece(sx, top, x.wrapping_sub(sx), bottom.wrapping_sub(top)));
            }
            if x.wrapping_add(w) < sx.wrapping_add(sw) {
                let top = y.max(sy);
                let bottom = y.wrapping_add(h).min(sy.wrapping_add(sh));
                result.push(piece(
                    x.wrapping_add(w),
                    top,
                    sx.wrapping_add(sw).wrapping_sub(x.wrapping_add(w)),
                    bottom.wrapping_sub(top),
                ));
            }
            if sy < y {
                result.push(piece(sx, sy, sw, y.wrapping_sub(sy)));
            }
            if y.wrapping_add(h) < sy.wrapping_add(sh) {
                result.push(piece(
                    sx,
                    y.wrapping_add(h),
                    sw,
                    sy.wrapping_add(sh).wrapping_sub(y.wrapping_add(h)),
                ));
            }
            return Some(result);
        }
        if self.is_polygon() {
            if let Some(clipper) = clipper {
                let input: Vec<f32> = self.points.iter().map(|&p| p as f32).collect();
                let polys = clipper.clip_polygon_aabb(
                    &input,
                    x as f32,
                    y as f32,
                    x.wrapping_add(w) as f32,
                    y.wrapping_add(h) as f32,
                );
                for poly in polys {
                    if poly.len() / 2 < 3 {
                        continue;
                    }
                    let mut zone = piece(sx, sy, sw, sh);
                    zone.geometry_type = ZoneGeometryType::Polygon;
                    zone.points = poly.iter().map(|&f| f as i32).collect();
                    result.push(zone);
                }
            }
        }
        Some(result)
    }

    fn is_point_in_polygon_winding_number(&self, x: f32, y: f32) -> PolygonHit {
        winding_hit(self.points.len(), |i| self.pt(i), x, y)
    }

    fn is_point_in_polyline_winding_number(&self, x: f32, y: f32) -> PolygonHit {
        match &self.polyline_outline_points {
            None => PolygonHit::Outside,
            Some(o) => winding_hit(o.len(), |i| Self::outline_pt(o, i), x, y),
        }
    }

    fn polygon_rect_intersect(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        let (xw, yh) = (x.wrapping_add(w), y.wrapping_add(h));
        if self.x >= x
            && self.x.wrapping_add(self.w) <= xw
            && self.y >= y
            && self.y.wrapping_add(self.h) <= yh
        {
            return true;
        }
        let n = self.points.len();
        let pt = |i| self.pt(i);
        let (fx, fy, fxw, fyh) = (x as f32, y as f32, xw as f32, yh as f32);
        ring_segment_intersects(n, &pt, fx, fy, fxw, fy)
            || ring_segment_intersects(n, &pt, fxw, fy, fxw, fyh)
            || ring_segment_intersects(n, &pt, fxw, fyh, fx, fyh)
            || ring_segment_intersects(n, &pt, fx, fyh, fx, fy)
    }

    fn polyline_outline_rect_intersect(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        let Some(o) = &self.polyline_outline_points else {
            return false;
        };
        let (xw, yh) = (x.wrapping_add(w), y.wrapping_add(h));
        if self.x >= x
            && self.x.wrapping_add(self.w) <= xw
            && self.y >= y
            && self.y.wrapping_add(self.h) <= yh
        {
            return true;
        }
        let n = o.len();
        let pt = |i| Self::outline_pt(o, i);
        let (fx, fy, fxw, fyh) = (x as f32, y as f32, xw as f32, yh as f32);
        ring_segment_intersects(n, &pt, fx, fy, fxw, fy)
            || ring_segment_intersects(n, &pt, fxw, fy, fxw, fyh)
            || ring_segment_intersects(n, &pt, fxw, fyh, fx, fyh)
            || ring_segment_intersects(n, &pt, fx, fyh, fx, fy)
    }

    /// `isClockwise()` (polygons only).
    pub fn is_clockwise(&self) -> bool {
        if !self.is_polygon() {
            return false;
        }
        let n = self.points.len();
        let mut sum = 0.0f32;
        let mut i = 0;
        while i < n {
            let (p1x, p1y) = (self.points[i], self.points[i + 1]);
            let (p2x, p2y) = (self.points[(i + 2) % n], self.points[(i + 3) % n]);
            sum += p2x.wrapping_sub(p1x).wrapping_mul(p2y.wrapping_add(p1y)) as f32;
            i += 2;
        }
        sum as f64 > 0.0
    }

    /// `getPolylineLength()`.
    pub fn get_polyline_length(&self) -> f32 {
        if !self.is_polyline() || self.points.is_empty() {
            return 0.0;
        }
        let mut length = 0.0f32;
        let mut i = 0usize;
        while (i as i32) < self.points.len() as i32 - 2 {
            let dx = self.points[i + 2].wrapping_sub(self.points[i]) as f32;
            let dy = self.points[i + 3].wrapping_sub(self.points[i + 1]) as f32;
            length += vec_length(dx, dy);
            i += 2;
        }
        length
    }

    /// `getClippedSegmentOfPolyline`: index of the first segment crossing
    /// the rectangle, with its `(t1, t2)`.
    pub fn get_clipped_segment_of_polyline(
        &self,
        clip_x1: i32,
        clip_y1: i32,
        clip_x2: i32,
        clip_y2: i32,
    ) -> Option<(i32, f64, f64)> {
        if !self.is_polyline() {
            return None;
        }
        let dxy = if self.polyline_width % 2 == 0 {
            0.0
        } else {
            0.5
        };
        let mut i = 0usize;
        while (i as i32) < self.points.len() as i32 - 2 {
            let (x1, y1) = (self.points[i], self.points[i + 1]);
            let (x2, y2) = (self.points[i + 2], self.points[i + 3]);
            if let Some((t1, t2)) = line_rect_intersect(
                x1 as f32 + dxy,
                y1 as f32 + dxy,
                x2.wrapping_sub(x1) as f32,
                y2.wrapping_sub(y1) as f32,
                clip_x1 as f32,
                clip_y1 as f32,
                clip_x2 as f32,
                clip_y2 as f32,
            ) {
                return Some(((i / 2) as i32, t1, t2));
            }
            i += 2;
        }
        None
    }

    /// `getTotalArea()` for shapes that need no triangulation; polygons and
    /// wide polylines are triangulated natively upstream (`None` here).
    pub fn get_total_area_simple(&self) -> Option<f32> {
        if self.is_rectangle() || self.is_point() || self.is_polyline() && self.polyline_width <= 0
        {
            return Some(self.w.wrapping_mul(self.h) as f32);
        }
        None
    }

    /// `getPointsToLua()`.
    pub fn points(&self) -> &[i32] {
        &self.points
    }
}

/// Points handed to `ClipperOffset` for a polyline: `(float)p + dxy`.
pub fn polyline_offset_input(points: &[i32], polyline_width: i32) -> Vec<f32> {
    let dxy = if polyline_width % 2 == 0 {
        0.0f32
    } else {
        0.5f32
    };
    points.iter().map(|&p| p as f32 + dxy).collect()
}

/// `IsoMetaGrid.calculatePolylineOutlineBounds`: `[minX, minY, maxX, maxY]`,
/// all zero if the offset produced no polygon.
pub fn calculate_polyline_outline_bounds(
    clipper: Option<&mut (dyn ClipperOps + '_)>,
    points: &[i32],
    polyline_width: i32,
) -> [i32; 4] {
    let mut bounds = [0i32; 4];
    let Some(clipper) = clipper else {
        return bounds;
    };
    let input = polyline_offset_input(points, polyline_width);
    let Some(outline) = clipper.offset_polyline(&input, (polyline_width as f32 / 2.0f32) as f64)
    else {
        return bounds;
    };
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (-f32::MAX, -f32::MAX);
    for &[x, y] in outline.as_chunks::<2>().0 {
        min_x = if min_x > x { x } else { min_x };
        min_y = if min_y > y { y } else { min_y };
        max_x = if max_x > x { max_x } else { x };
        max_y = if max_y > y { max_y } else { y };
    }
    bounds[0] = fastfloor(min_x) as f32 as i32;
    bounds[1] = fastfloor(min_y) as f32 as i32;
    bounds[2] = pz_ceil(max_x) as i32;
    bounds[3] = pz_ceil(max_y) as i32;
    bounds
}

/// `KahluaTable.getString`: a string, nil, or a `ClassCastException`.
pub(crate) fn prop_string(
    props: &dyn ZoneProps,
    key: &'static str,
) -> Result<Option<String>, ZoneError> {
    match props.rawget(key) {
        PropValue::Nil => Ok(None),
        PropValue::Str(s) => Ok(Some(s)),
        _ => Err(ZoneError::WrongPropertyType(key)),
    }
}

/// `instanceof String` read of a property.
fn prop_if_string(props: &dyn ZoneProps, key: &str) -> Option<String> {
    match props.rawget(key) {
        PropValue::Str(s) => Some(s),
        _ => None,
    }
}

impl VehicleZoneData {
    /// The `VehicleZone` constructor's property reads.
    pub fn from_props(
        props: Option<&dyn ZoneProps>,
        tables: &ZoneTables,
    ) -> Result<Self, ZoneError> {
        let mut data = VehicleZoneData::default();
        if let Some(p) = props {
            if let Some(s) = prop_if_string(p, "Direction") {
                data.dir = Some(tables.direction_value_of(&s)?);
            }
            if p.rawget("FaceDirection") == PropValue::Bool(true) {
                data.flags |= Self::VZF_FACE_DIRECTION;
            }
        }
        Ok(data)
    }
}

impl MannequinZoneData {
    /// The `MannequinZone` constructor's property reads.
    pub fn from_props(
        props: Option<&dyn ZoneProps>,
        tables: &ZoneTables,
    ) -> Result<Self, ZoneError> {
        let mut data = MannequinZoneData {
            female: -1,
            dir: None,
            mannequin_script: None,
            pose: None,
            skin: None,
            outfit: None,
        };
        if let Some(p) = props {
            if let PropValue::Bool(b) = p.rawget("Female") {
                data.female = i32::from(b);
            }
            if let Some(s) = prop_if_string(p, "Direction") {
                data.dir = Some(tables.direction_value_of(&s)?);
            }
            if let Some(s) = prop_if_string(p, "Outfit") {
                data.outfit = Some(s);
            }
            if let Some(s) = prop_if_string(p, "Script") {
                data.mannequin_script = Some(s);
            }
            if let Some(s) = prop_if_string(p, "Skin") {
                data.skin = Some(s);
            }
            if let Some(s) = prop_if_string(p, "Pose") {
                data.pose = Some(s);
            }
        }
        Ok(data)
    }
}

impl AnimalZoneData {
    /// The `AnimalZone(name, type, x, y, z, w, h, properties)` reads.
    pub fn from_props(props: Option<&dyn ZoneProps>) -> Self {
        let mut data = AnimalZoneData::default();
        if let Some(p) = props {
            data.action = prop_if_string(p, "Action");
            data.animal_type = prop_if_string(p, "AnimalType");
            if let PropValue::Bool(b) = p.rawget("SpawnAnimals") {
                data.spawn_animal = b;
            }
        }
        data
    }
}

/// `WorldGenZone` constructor: `rocks` defaults to true.
pub fn world_gen_rocks(props: Option<&dyn ZoneProps>) -> bool {
    match props.map(|p| p.rawget("Rocks")) {
        Some(PropValue::Bool(b)) => b,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Zone {
        Zone::new("n", "T", x, y, 0, w, h)
    }

    /// Square ring outline: the input polyline offset by a fixed amount
    /// as an axis-aligned box (hand-written stand-in for the native lib).
    struct BoxOffset;
    impl ClipperOps for BoxOffset {
        fn offset_polyline(&mut self, p: &[f32], d: f64) -> Option<Vec<f32>> {
            let d = d as f32;
            let xs = p.iter().step_by(2);
            let ys = p.iter().skip(1).step_by(2);
            let (x0, x1) = xs.fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            let (y0, y1) = ys.fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            Some(vec![
                x0 - d,
                y0 - d,
                x1 + d,
                y0 - d,
                x1 + d,
                y1 + d,
                x0 - d,
                y1 + d,
            ])
        }
        fn clip_polygon_aabb(
            &mut self,
            _: &[f32],
            _: f32,
            _: f32,
            _: f32,
            _: f32,
        ) -> Vec<Vec<f32>> {
            vec![vec![0.0, 0.0, 4.0, 0.0], vec![0.0, 0.0, 4.0, 0.0, 4.0, 4.0]]
        }
    }

    #[test]
    fn rectangle_contains_and_intersects() {
        let z = rect(10, 20, 5, 3);
        assert!(z.contains(10, 20, 0));
        assert!(z.contains(14, 22, 0));
        assert!(!z.contains(15, 22, 0));
        assert!(!z.contains(10, 20, 1));
        assert!(z.intersects(0, 0, ANY_Z, 11, 21));
        assert!(!z.intersects(0, 0, ANY_Z, 10, 21));
        assert!(!z.intersects(0, 0, 1, 100, 100));
    }

    #[test]
    fn polygon_winding_and_rect() {
        // Triangle (0,0) (10,0) (0,10).
        let mut z = rect(0, 0, 11, 11);
        z.geometry_type = ZoneGeometryType::Polygon;
        z.points = vec![0, 0, 10, 0, 0, 10];
        assert!(z.contains(1, 1, 0));
        assert!(z.contains(4, 4, 0));
        assert!(!z.contains(5, 5, 0));
        assert!(!z.contains(9, 9, 0));
        assert!(z.intersects(0, 0, 0, 8, 8));
        assert!(!z.intersects(8, 8, 0, 3, 3));
        assert!(z.intersects(-5, -5, 0, 40, 40));
        assert!(!z.is_clockwise());
    }

    #[test]
    fn polyline_without_width() {
        let mut z = rect(0, 0, 21, 1);
        z.geometry_type = ZoneGeometryType::Polyline;
        z.points = vec![0, 0, 20, 0];
        assert!(!z.contains(3, 0, 0));
        assert!(z.intersects(8, -4, 0, 8, 8));
        assert!(!z.intersects(8, 1, 0, 8, 8));
        assert_eq!(z.get_polyline_length(), 20.0);
        assert_eq!(
            z.get_clipped_segment_of_polyline(5, -1, 6, 1),
            Some((0, 0.25, 0.3))
        );
    }

    #[test]
    fn polyline_with_outline() {
        let mut z = rect(0, 0, 21, 4);
        z.geometry_type = ZoneGeometryType::Polyline;
        z.points = vec![0, 2, 20, 2];
        z.polyline_width = 2;
        assert!(!z.contains(3, 2, 0));
        z.check_polyline_outline(Some(&mut BoxOffset));
        assert!(z.contains(3, 2, 0));
        assert!(z.contains(3, 1, 0));
        assert!(!z.contains(3, 3, 0));
        assert_eq!(
            calculate_polyline_outline_bounds(Some(&mut BoxOffset), &[0, 2, 20, 2], 3),
            [-1, 1, 23, 5]
        );
        // PZMath.ceil adds 0.9999999f, which rounds integral values >= 16 up.
        assert_eq!(pz_ceil(22.0), 23.0);
        assert_eq!(calculate_polyline_outline_bounds(None, &[0, 0], 3), [0; 4]);
    }

    #[test]
    fn rectangle_difference() {
        let z = rect(0, 0, 10, 10);
        assert!(z.difference(20, 20, ANY_Z, 5, 5, None).is_none());
        let d = z.difference(3, 3, ANY_Z, 4, 4, None).unwrap();
        let b: Vec<_> = d.iter().map(|z| (z.x, z.y, z.w, z.h)).collect();
        assert_eq!(
            b,
            vec![(0, 3, 3, 4), (7, 3, 3, 4), (0, 0, 10, 3), (0, 7, 10, 3)]
        );
        assert!(d.iter().all(|z| !z.is_preferred_zone_for_square));
    }

    #[test]
    fn polygon_difference_skips_degenerate() {
        let mut z = rect(0, 0, 10, 10);
        z.geometry_type = ZoneGeometryType::Polygon;
        z.points = vec![0, 0, 9, 0, 9, 9, 0, 9];
        let d = z.difference(5, 5, 0, 1, 1, Some(&mut BoxOffset)).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].points, vec![0, 0, 4, 0, 4, 4]);
        assert_eq!((d[0].x, d[0].w), (0, 10));
    }

    #[test]
    fn liang_barsky_and_ceil() {
        assert_eq!(
            line_rect_intersect(0.0, 0.0, 10.0, 0.0, 2.0, -1.0, 4.0, 1.0),
            Some((0.2, 0.4))
        );
        assert_eq!(
            line_rect_intersect(0.0, 5.0, 10.0, 0.0, 2.0, -1.0, 4.0, 1.0),
            None
        );
        assert_eq!(pz_ceil(1.0), 1.0);
        assert_eq!(pz_ceil(1.5), 2.0);
        assert_eq!(pz_ceil(-1.5), -1.0);
    }

    #[test]
    fn props() {
        let tables = ZoneTables {
            preferred_zone_types: vec!["Wood".into()],
            iso_direction_names: vec!["Up".into(), "Down".into()],
        };
        let mut p: HashMap<String, PropValue> = HashMap::new();
        p.insert("Direction".into(), PropValue::Str("Down".into()));
        p.insert("FaceDirection".into(), PropValue::Bool(true));
        let v = VehicleZoneData::from_props(Some(&p), &tables).unwrap();
        assert_eq!((v.dir, v.flags), (Some(1), 1));
        p.insert("Direction".into(), PropValue::Str("Sideways".into()));
        assert!(VehicleZoneData::from_props(Some(&p), &tables).is_err());
        assert!(world_gen_rocks(None));
        assert!(tables.is_preferred_zone_for_square("Wood"));
    }
}

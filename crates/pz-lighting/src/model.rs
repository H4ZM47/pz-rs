//! Plain data types shared by the engine: square flags, transmissions,
//! the per-square light lists, lights, room lights, torches and the
//! character the lighting is computed for.

/// Number of vertical levels the library tracks. World level `z` is stored
/// as `z + 32`, so valid native levels are `0..64`.
pub const NUM_LEVELS: usize = 64;

/// The level index of world `z = 0`.
pub const GROUND_LEVEL: i32 = 32;

/// Bits of the packed per-square flag word built by `squareSet`.
pub mod flags {
    /// Vision to the west neighbour is not blocked.
    pub const W: u16 = 1 << 0;
    /// Vision to the north neighbour is not blocked.
    pub const N: u16 = 1 << 1;
    /// Vision to the east neighbour is not blocked.
    pub const E: u16 = 1 << 2;
    /// Vision to the south neighbour is not blocked.
    pub const S: u16 = 1 << 3;
    /// Vision to the north-west neighbour is not blocked.
    pub const NW: u16 = 1 << 4;
    /// Vision to the north-east neighbour is not blocked.
    pub const NE: u16 = 1 << 5;
    /// Vision to the south-east neighbour is not blocked.
    pub const SE: u16 = 1 << 6;
    /// Vision to the south-west neighbour is not blocked.
    pub const SW: u16 = 1 << 7;
    /// Vision to the square above is not blocked.
    pub const UP: u16 = 1 << 8;
    /// Vision to the square below is not blocked.
    pub const DOWN: u16 = 1 << 9;
    /// The square is open to the sky.
    pub const OPEN_AIR: u16 = 1 << 10;
    /// The square has an elevated floor (stairs).
    pub const ELEVATED: u16 = 1 << 11;
    /// The player has already seen the square.
    pub const SEEN: u16 = 1 << 12;
    /// Set while the square is a sky seed of the sunlight flood.
    pub const SUN_SEEDED: u16 = 1 << 13;
}

/// Bits of the open-edge mask derived from the flags of a square and its
/// cardinal neighbours (an edge is open when either side sees through it).
pub(crate) mod open {
    pub const N: u32 = 1;
    pub const W: u32 = 1 << 2;
    pub const S: u32 = 1 << 4;
    pub const E: u32 = 1 << 6;
}

/// Bits of the per-square visibility byte returned in slot 0 of
/// [`crate::SquareLighting`].
pub mod vis {
    /// The square has been seen at some point.
    pub const SEEN: u8 = 1;
    /// The square is currently inside the vision cone and visible.
    pub const CAN_SEE: u8 = 2;
    /// The square is in line of sight (ignoring the cone).
    pub const COULD_SEE: u8 = 4;
}

/// Light transmission through one edge of a square (closed curtains,
/// doors, barricades): an RGB filter, an intensity and a mix factor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transmission {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub intensity: f32,
    pub mix: f32,
}

impl Transmission {
    pub(crate) const NONE: Transmission =
        Transmission { r: 0.0, g: 0.0, b: 0.0, intensity: 0.0, mix: 0.0 };
    pub(crate) const OPEN: Transmission =
        Transmission { r: 0.0, g: 0.0, b: 0.0, intensity: 1.0, mix: 0.0 };

    /// Applies this transmission to a colour in `0..=1`.
    #[inline]
    pub(crate) fn apply(&self, r: f32, g: f32, b: f32) -> (f32, f32, f32) {
        let i = self.intensity;
        if i <= 0.0 {
            return (r, g, b);
        }
        if self.r > 0.0 || self.g > 0.0 || self.b > 0.0 {
            let k = i * lt_sel(r, lt_sel(g, b));
            let mix = self.mix;
            let rm = r * mix;
            let gm = g * mix;
            let tr = self.r * k;
            let tg = self.g * k;
            let r2 = if tr > rm { tr } else { rm };
            let g2 = if tg > gm { tg } else { gm };
            let bm = b * mix;
            let tb = k * self.b;
            let b2 = if bm < tb { tb } else { bm };
            (r2, g2, b2)
        } else {
            (r * i, g * i, i * b)
        }
    }
}

/// `a < b ? b : a`, the select the compiled code uses for channel maxima.
#[inline]
pub(crate) fn lt_sel(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}

/// One entry of a square's light list.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResultLight {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: u8,
    /// `r | g << 8 | b << 16 | flags << 24` (flags: 1 room light,
    /// 2 torch, 0 point light).
    pub packed: u32,
    pub dist: f32,
}

/// A square's light list: at most five lights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResultLights {
    pub count: i32,
    pub lights: [ResultLight; 5],
}

impl ResultLights {
    /// The slot `ResultLights::addLight` writes a light to, or `None` when
    /// the list is left unchanged. A known id is replaced only by a closer
    /// entry; when the list is full, the entry with the largest
    /// `dist / radius` is evicted if the new light is relatively closer.
    fn slot_for(&mut self, id: i32, radius: i32, dist: f32) -> Option<usize> {
        let n = self.count;
        if n >= 1 {
            let n = n as usize;
            for i in 0..n {
                if self.lights[i].id == id {
                    return (self.lights[i].dist > dist).then_some(i);
                }
            }
            if n >= 5 {
                let mut best = 0.0f32;
                let mut bi: i32 = -1;
                for i in 0..n {
                    let v = self.lights[i].dist / f32::from(self.lights[i].radius);
                    if !(v <= best) || bi == -1 {
                        best = v;
                        bi = i as i32;
                    }
                }
                let e = &self.lights[bi as usize];
                let worst = e.dist / f32::from(e.radius);
                return (worst > dist / radius as f32).then_some(bi as usize);
            }
        }
        self.count = n + 1;
        Some(n as usize)
    }

    /// Writes an entry with an already packed colour (room lights).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_packed(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, packed: u32, dist: f32) -> bool {
        let Some(slot) = self.slot_for(id, radius, dist) else { return false };
        self.lights[slot] = ResultLight { id, x, y, z, radius: radius as u8, packed, dist };
        true
    }

    /// `ResultLights::addLight`. Returns whether the list changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_light(
        &mut self,
        id: i32,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        r: f32,
        g: f32,
        b: f32,
        light_flags: u8,
        dist: f32,
    ) -> bool {
        let Some(slot) = self.slot_for(id, radius, dist) else { return false };
        let r = if r > 1.0 { 1.0 } else { r };
        let g = if g > 1.0 { 1.0 } else { g };
        let b = if b > 1.0 { 1.0 } else { b };
        let pr = ((r * 255.0) as i32 as u32) & 0xff;
        let pg = ((g * 255.0) as i32 as u32) & 0xff;
        let pb = ((b * 255.0) as i32 as u32) & 0xff;
        let packed = pr | (pg << 8) | (pb << 16) | (u32::from(light_flags) << 24);
        self.lights[slot] = ResultLight { id, x, y, z, radius: radius as u8, packed, dist };
        true
    }
}

/// A light-blocking object attached to a square (`squareAdd*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LightingObject {
    pub owner: u32,
    pub kind: ObjectKind,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    Curtain,
    Door,
    Thumpable,
    Window,
}

/// What a single object does to vision between two squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VisionResult {
    NoEffect,
    Blocked,
    Unblocked,
}

/// `LosUtil.TestResults`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TestResult {
    Clear = 0,
    ClearThroughOpenDoor = 1,
    ClearThroughWindow = 2,
    Blocked = 3,
    ClearThroughClosedDoor = 4,
}

/// The player the lighting state is computed for (`playerSet`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Character {
    pub x: f32,
    pub y: f32,
    /// Native level (`z + 32`).
    pub z: f32,
    pub look_x: f32,
    pub look_y: f32,
    /// "See everything": every square is marked seen, visible and in line
    /// of sight, with a target darkness of 1. Java never sets it. Use it
    /// for renders without a player (noon map frames).
    pub see_all: bool,
    pub reanimated: bool,
    pub ghost: bool,
    pub short_sighted: bool,
    pub fatigue: f32,
    pub perception: f32,
    /// `(degrees - 180) / 180`.
    pub cone: f32,
}

/// A point light (`addLight` / `addTempLight`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Light {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub active: bool,
    pub life: i32,
    pub life_max: i32,
    /// Per player: the light was handed to that player's state.
    pub added: [bool; 4],
    /// Per player: `active` as of the last hand-off.
    pub active_snap: [bool; 4],
}

/// A room light: lights a rectangle at full strength and falls off over
/// seven squares outside it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RoomLight {
    pub id: i32,
    /// Stored for completeness; the native code never reads it.
    #[allow(dead_code)]
    pub building_id: i64,
    /// Stored for completeness; the native code never reads it.
    #[allow(dead_code)]
    pub room_id: i64,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    pub active: bool,
    pub added: [bool; 4],
    pub active_snap: [bool; 4],
}

/// A flashlight or vehicle headlight (`updateTorch`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Torch {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub angle_x: f32,
    pub angle_y: f32,
    pub dist: f32,
    pub strength: f32,
    pub cone: bool,
    pub dot: f32,
    pub focusing: i32,
    pub added: [bool; 4],
}

/// `fastfloor`: `(int)f - (f < (float)(int)f)`.
#[inline]
pub(crate) fn fastfloor(f: f32) -> i32 {
    let i = f as i32;
    i - i32::from(f < i as f32)
}

/// `DistanceSquared(a, b, c, d)`: `(c - a)^2 + (d - b)^2` with the first
/// product fused.
#[inline]
pub(crate) fn distance_squared(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let dx = c - a;
    let dy = d - b;
    dx.mul_add(dx, dy * dy)
}

/// `DistanceTo`: integer distance with the level difference weighted x3.
#[inline]
pub(crate) fn distance_to(x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> f32 {
    let dx = x1.wrapping_sub(x0);
    let dy = y1.wrapping_sub(y0);
    let dz = z1.wrapping_sub(z0).wrapping_mul(3);
    let s = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)).wrapping_add(dz.wrapping_mul(dz));
    (s as u32 as f32).sqrt()
}

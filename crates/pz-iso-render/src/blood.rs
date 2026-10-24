//! `IsoFloorBloodSplat` and `FBORenderCell.renderOneLevel_Blood`: a chunk
//! level's floor blood, recorded into its chunk texture after the floors
//! and snow, from the chunk itself and the splats of its eight neighbours
//! that reach within a square of it.

use pz_render::iso::{fastfloor, x_to_screen, y_to_screen};
use pz_render::{gl, DrawCmd, Rgba, WorldShader};

use crate::alpha::CutawayView;
use crate::chunk::ChunkTexture;
use crate::color::{self, ColorInfo};
use crate::texd::TexDraw;
use crate::weather::Surfaces;

/// `IsoFloorBloodSplat.FLOOR_BLOOD_TYPES`.
pub const FLOOR_BLOOD_TYPES: [&str; 21] = [
    "blood_floor_small_01",
    "blood_floor_small_02",
    "blood_floor_small_03",
    "blood_floor_small_04",
    "blood_floor_small_05",
    "blood_floor_small_06",
    "blood_floor_small_07",
    "blood_floor_small_08",
    "blood_floor_med_01",
    "blood_floor_med_02",
    "blood_floor_med_03",
    "blood_floor_med_04",
    "blood_floor_med_05",
    "blood_floor_med_06",
    "blood_floor_med_07",
    "blood_floor_med_08",
    "blood_floor_large_01",
    "blood_floor_large_02",
    "blood_floor_large_03",
    "blood_floor_large_04",
    "blood_floor_large_05",
];

/// `IsoFloorBloodSplat.FADE_HOURS`.
pub const FADE_HOURS: f32 = 72.0;

/// `IsoChunk.renderByIndex`: which splat indices (1-10) each blood decal
/// option (1-10) shows.
pub const RENDER_BY_INDEX: [[u8; 10]; 10] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 1, 0, 0, 0, 0],
    [1, 0, 0, 1, 0, 0, 1, 0, 0, 0],
    [1, 0, 0, 1, 0, 1, 0, 0, 1, 0],
    [1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [1, 1, 0, 1, 1, 0, 1, 1, 0, 0],
    [1, 1, 0, 1, 1, 0, 1, 1, 0, 1],
    [1, 1, 1, 1, 0, 1, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

/// `IsoFloorBloodSplat`: a splat at chunk-local `(x, y, z)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FloorBloodSplat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Index into [`FLOOR_BLOOD_TYPES`].
    pub kind: i32,
    /// `worldAge`: the world age in hours when it was made.
    pub world_age: f32,
    /// 1-10: thinned out by the blood decal option.
    pub index: i32,
    /// `fade`: frames left fading out (0 when not fading).
    pub fade: i32,
}

/// A chunk's `floorBloodSplatsFade` and `floorBloodSplats`. The fading
/// splats count down each time they are drawn (the chunk texture being
/// recorded), as the original's do, so they sit in a `RefCell`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkBlood {
    pub fade: std::cell::RefCell<Vec<FloorBloodSplat>>,
    pub splats: Vec<FloorBloodSplat>,
    /// `nextSplatIndex`.
    pub next_splat_index: i32,
}

/// `floorBloodSplats`' capacity (`BoundedQueue(1000)`).
pub const MAX_SPLATS: usize = 1000;

impl ChunkBlood {
    /// `IsoChunk.addBloodSplat(x, y, z, type)` for chunk `(wx, wy)`, with
    /// world coordinates: kept when it lands on the chunk's solid floor
    /// (`solid_floor_at(x, y, z)` for the chunk-local square). Returns the
    /// level to invalidate with `dirty::BLOOD`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_splat(&mut self, wx: i32, wy: i32, x: f32, y: f32, z: f32, kind: i32, world_age: f32, lock_fps: i32, solid_floor_at: impl Fn(i32, i32, i32) -> bool) -> Option<i32> {
        let (cx, cy) = ((wx * 8) as f32, (wy * 8) as f32);
        if x < cx || x >= cx + 8.0 || y < cy || y >= cy + 8.0 {
            return None;
        }
        let (lx, ly, lz) = (fastfloor(x - cx), fastfloor(y - cy), fastfloor(z));
        if !solid_floor_at(lx, ly, lz) {
            return None;
        }
        let mut b = FloorBloodSplat { x: x - cx, y: y - cy, z, kind, world_age, index: 0, fade: 0 };
        if kind < 8 {
            self.next_splat_index += 1;
            b.index = self.next_splat_index;
            if self.next_splat_index >= 10 {
                self.next_splat_index = 0;
            }
        }
        if self.splats.len() >= MAX_SPLATS {
            let mut old = self.splats.remove(0);
            old.fade = lock_fps * 5;
            self.fade.get_mut().push(old);
        }
        self.splats.push(b);
        Some(lz)
    }
}

/// The colour of a splat before the square's light: darkened by a hash of
/// its position and type and by age, fading out over `fade` frames.
pub fn splat_color(b: &FloorBloodSplat, world_age: f32, lock_fps: i32) -> ColorInfo {
    let mut inf = ColorInfo { r: 1.0, g: 1.0, b: 1.0, a: 0.27 };
    let t = (b.kind + 1) as f32;
    let mut aa = (b.x + b.y / b.x) * t;
    let mut bb = aa * b.x / b.y * t / (aa + b.y);
    let mut cc = bb * aa * bb * b.x / (b.y + 2.0);
    aa *= 42367.543;
    bb *= 6367.123;
    cc *= 23367.133;
    aa %= 1000.0;
    bb %= 1000.0;
    cc %= 1000.0;
    aa /= 1000.0;
    bb /= 1000.0;
    cc /= 1000.0;
    if aa > 0.25 {
        aa = 0.25;
    }
    inf.r -= aa * 2.0;
    inf.g -= aa * 2.0;
    inf.b -= aa * 2.0;
    inf.r += bb / 3.0;
    inf.g -= cc / 3.0;
    inf.b -= cc / 3.0;
    let delta = world_age - b.world_age;
    if (0.0..FADE_HOURS).contains(&delta) {
        let f = 1.0 - delta / FADE_HOURS;
        inf.r *= 0.2 + f * 0.8;
        inf.g *= 0.2 + f * 0.8;
        inf.b *= 0.2 + f * 0.8;
        inf.a *= 0.25 + f * 0.75;
    } else {
        inf.r *= 0.2;
        inf.g *= 0.2;
        inf.b *= 0.2;
        inf.a *= 0.25;
    }
    if b.fade > 0 {
        inf.a *= b.fade as f32 / (lock_fps as f32 * 5.0);
    }
    inf
}

/// The splats of chunk `(wx, wy)` drawn into `tex` at `level` within
/// `min..=max` world squares, in `splatByType` order.
/// Fading splats carry their index in `floorBloodSplatsFade`.
fn collect(s: &Surfaces<'_>, wx: i32, wy: i32, level: i32, min: (i32, i32), max: (i32, i32)) -> Vec<(FloorBloodSplat, Option<usize>)> {
    let Some(blood) = s.world.chunk_blood(wx, wy) else { return Vec::new() };
    let option = s.settings.blood_decals;
    let (cx, cy) = ((wx * 8) as f32, (wy * 8) as f32);
    let keep = |b: &&FloorBloodSplat| {
        !((1..=10).contains(&b.index) && RENDER_BY_INDEX[(option - 1) as usize][(b.index - 1) as usize] == 0)
            && !(cx + b.x < min.0 as f32 || cx + b.x > max.0 as f32 || cy + b.y < min.1 as f32 || cy + b.y > max.1 as f32)
            && fastfloor(b.z) == level
            && (0..FLOOR_BLOOD_TYPES.len() as i32).contains(&b.kind)
    };
    let mut by_type: Vec<Vec<(FloorBloodSplat, Option<usize>)>> = vec![Vec::new(); FLOOR_BLOOD_TYPES.len()];
    let fade = blood.fade.borrow();
    for (i, b) in fade.iter().enumerate() {
        if keep(&b) {
            by_type[b.kind as usize].push((*b, Some(i)));
        }
    }
    for b in blood.splats.iter().filter(keep) {
        by_type[b.kind as usize].push((*b, None));
    }
    by_type.into_iter().flatten().collect()
}

/// `renderOneLevel_Blood(chunk, level, minX, minY, maxX, maxY)` for the
/// splats of chunk `(wx, wy)`.
#[allow(clippy::too_many_arguments)]
fn render_chunk(s: &Surfaces<'_>, cv: Option<&CutawayView<'_>>, tex: ChunkTexture, wx: i32, wy: i32, level: i32, min: (i32, i32), max: (i32, i32), out: &mut Vec<DrawCmd>) {
    if s.settings.blood_decals == 0 {
        return;
    }
    let (xoff, yoff) = tex.record_offset();
    let mut faded = Vec::new();
    for (b, fade_index) in collect(s, wx, wy, level, min, max) {
        let mut inf = splat_color(&b, s.world_age_hours, s.settings.lock_fps);
        // `--b.fade`; at 0 the splat leaves floorBloodSplatsFade.
        if let (Some(i), true) = (fade_index, b.fade > 0) {
            if let Some(blood) = s.world.chunk_blood(wx, wy) {
                let mut f = blood.fade.borrow_mut();
                f[i].fade -= 1;
                if f[i].fade == 0 {
                    faded.push(i);
                }
            }
        }
        let Some(t) = s.textures.texture(FLOOR_BLOOD_TYPES[b.kind as usize]) else { continue };
        let Some(sq) = s.world.square(wx * 8 + fastfloor(b.x), wy * 8 + fastfloor(b.y), fastfloor(b.z)) else { continue };
        if cv.is_some_and(|cv| !cv.is_visible(sq)) {
            continue;
        }
        let l = &sq.vert_lights;
        inf.r *= (0..4).map(|i| color::red(l[i])).sum::<f32>() / 4.0;
        inf.g *= (0..4).map(|i| color::green(l[i])).sum::<f32>() / 4.0;
        inf.b *= (0..4).map(|i| color::blue(l[i])).sum::<f32>() / 4.0;
        // IsoSprite.renderBloodSplat, relative to the chunk being recorded.
        let x = (wx * 8) as f32 + b.x - (tex.wx * 8) as f32;
        let y = (wy * 8) as f32 + b.y - (tex.wy * 8) as f32;
        let mut sx = x_to_screen(x, y) as i32 as f32;
        let mut sy = y_to_screen(x, y, b.z, 0) as i32 as f32;
        sx -= t.width as f32 / 2.0 * pz_render::iso::TILE_SCALE as f32;
        sy -= t.height as f32 / 2.0 * pz_render::iso::TILE_SCALE as f32;
        sx += xoff;
        sy += yoff;
        out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
        out.push(DrawCmd::BlendFuncSeparate(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE_MINUS_DST_ALPHA, gl::ONE));
        let Some(d) = TexDraw::new(&t, sx + t.offset_x, sy + t.offset_y, t.width as f32, t.height as f32, Rgba(inf.r, inf.g, inf.b, inf.a)) else { continue };
        out.push(d.into_cmd(WorldShader::Default { z: 0.0 }));
    }
    if !faded.is_empty() {
        if let Some(blood) = s.world.chunk_blood(wx, wy) {
            faded.sort_unstable();
            let mut f = blood.fade.borrow_mut();
            for i in faded.into_iter().rev() {
                f.remove(i);
            }
        }
    }
}

/// The blood pass of `renderOneLevel`: chunk `tex`'s own splats on `level`,
/// then those of its eight neighbours within a square of it. Skipped when
/// the camera is underground and the level above it.
pub fn render_level_blood(s: &Surfaces<'_>, cv: Option<&CutawayView<'_>>, cam_z: f32, tex: ChunkTexture, level: i32, out: &mut Vec<DrawCmd>) {
    if !(cam_z >= 0.0 || level <= fastfloor(cam_z)) {
        return;
    }
    let (wx, wy) = (tex.wx, tex.wy);
    render_chunk(s, cv, tex, wx, wy, level, (wx * 8, wy * 8), ((wx + 1) * 8, (wy + 1) * 8), out);
    for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
        render_chunk(s, cv, tex, wx + dx, wy + dy, level, (wx * 8 - 1, wy * 8 - 1), ((wx + 1) * 8 + 1, (wy + 1) * 8 + 1), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_splats_are_dark_and_faint() {
        let b = FloorBloodSplat { x: 3.5, y: 2.25, z: 0.0, kind: 4, world_age: 0.0, index: 1, fade: 0 };
        let fresh = splat_color(&b, 0.0, 60);
        let old = splat_color(&b, 100.0, 60);
        assert!((fresh.a - 0.27).abs() < 1e-6);
        assert!((old.a - 0.27 * 0.25).abs() < 1e-6);
        assert!((old.r - fresh.r * 0.2).abs() < 1e-6);
        let fading = splat_color(&FloorBloodSplat { fade: 150, ..b }, 0.0, 60);
        assert!((fading.a - 0.27 * 0.5).abs() < 1e-6);
        // The 1001st splat pushes the first out to fade over 5 seconds.
        let mut c = ChunkBlood::default();
        for i in 0..=MAX_SPLATS {
            assert_eq!(c.add_splat(1, 0, 8.5, 0.5, 0.0, (i % 8) as i32, 0.0, 60, |_, _, _| true), Some(0));
        }
        assert_eq!(c.splats.len(), MAX_SPLATS);
        assert_eq!(c.fade.borrow()[0].fade, 300);
        assert_eq!(c.fade.borrow()[0].index, 1);
        assert_eq!(c.add_splat(1, 0, 7.5, 0.5, 0.0, 0, 0.0, 60, |_, _, _| true), None);
        // Option 1 shows only index 1.
        assert_eq!(RENDER_BY_INDEX[0], [1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
}

//! One world frame as `IngameState.renderInternal` produces it for a
//! player: `Core.StartFrame` (offscreen buffer bound, zoomed projection,
//! cleared), the chunk textures recorded and composited
//! (`FBORenderCell.performRenderTiles`), `Core.EndFrame`, then
//! `Core.RenderOffScreenBuffer` through the `screen` shader.

use pz_render::{gl, DrawCmd, ScreenParams};

use crate::cell::{ChunkRenderer, LevelSquares};
use crate::chunk::{pow2, ChunkTexture};

/// One chunk texture to record this frame, with its squares by level.
pub struct ChunkLevels<'s> {
    pub texture: ChunkTexture,
    pub levels: Vec<(i32, LevelSquares<'s>)>,
    /// Whether the texture must be re-recorded (it is dirty or new).
    pub dirty: bool,
}

/// A loaded chunk: its level range and squares by level.
pub struct ChunkInput<'s> {
    pub wx: i32,
    pub wy: i32,
    pub min_level: i32,
    pub max_level: i32,
    pub levels: Vec<(i32, LevelSquares<'s>)>,
}

/// `FBORenderCell.performRenderTiles`' chunk loop with the texture cache:
/// for each chunk (in `checkNewlyOnScreenChunks` order) and level pair on
/// screen, the texture, recorded when the cache says so; pairs off screen
/// or gone are freed into `out`.
pub fn plan_chunks<'s>(cache: &mut crate::cache::ChunkCache, cam: &crate::Camera, size: FrameSize, chunks: &[ChunkInput<'s>], out: &mut Vec<DrawCmd>) -> Vec<ChunkLevels<'s>> {
    let mut plan = Vec::new();
    let mut keep = Vec::new();
    for c in chunks {
        let mut z = pz_render::iso::calculate_min_level(c.min_level);
        while z <= c.max_level {
            let level = z.max(c.min_level);
            if crate::cache::level_on_screen(cam, size.width, size.height, c.wx, c.wy, level) {
                let (texture, dirty) = cache.begin_level(c.wx, c.wy, level, c.min_level, c.max_level);
                keep.push((c.wx, c.wy, pz_render::iso::calculate_min_level(level)));
                let levels = c.levels.iter().filter(|(l, _)| *l >= texture.min_level && *l <= texture.top_level).cloned().collect();
                plan.push(ChunkLevels { texture, levels, dirty });
            }
            z += 2;
        }
    }
    cache.free_except(&keep, out);
    plan
}

/// The screen-size parameters of a frame.
#[derive(Clone, Copy, Debug)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    /// `MultiTextureFBO2`'s buffer size: the next powers of two.
    pub fn offscreen(&self) -> (u32, u32) {
        (pow2(self.width as i32) as u32, pow2(self.height as i32) as u32)
    }
}

/// Records a world frame: every chunk texture in `chunks` (recorded when
/// dirty, composited always) drawn into the offscreen buffer, which is
/// then drawn to the screen.
pub fn world_frame(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], screen: ScreenParams) -> Vec<DrawCmd> {
    world_frame_into(r, size, chunks, screen, Vec::new())
}

/// [`world_frame`] appended to `out` (which may hold texture frees).
pub fn world_frame_into(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], screen: ScreenParams, mut out: Vec<DrawCmd>) -> Vec<DrawCmd> {
    let (ow, oh) = size.offscreen();
    let (w, h) = (size.width, size.height);
    let z = r.cam.zoom;
    out.extend([
        DrawCmd::OffscreenBegin { width: ow, height: oh },
        // glDoStartFrame: blending, depth func ALWAYS, zoomed projection.
        DrawCmd::GlEnable(gl::BLEND),
        DrawCmd::DepthFunc(gl::ALWAYS),
        DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h },
        DrawCmd::ColorMask(true, true, true, true),
        DrawCmd::DepthMask(true),
        DrawCmd::StencilMask(0xFF),
        DrawCmd::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT | gl::STENCIL_BUFFER_BIT),
    ]);
    let mut recorded = Vec::new();
    for c in chunks {
        if c.dirty {
            r.record(c.texture, &c.levels, &mut out);
            out.push(DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h });
        }
        recorded.push(c.texture);
    }
    r.composite(&recorded, w, h, &mut out);
    let squares: Vec<&crate::world::RenderSquare> = chunks.iter().flat_map(|c| c.levels.iter().flat_map(|(_, l)| l.iter().flatten().copied())).collect();
    // performRenderTiles after the composite: open water.
    if let Some(s) = r.surfaces.as_ref() {
        crate::water::render_water(r, s, &squares, &mut out);
    }
    // The per-level translucent passes, over every on-screen square
    // (recorded this frame or not): translucent floors, shores (from level
    // 0), then translucent objects.
    if let (Some(lo), Some(hi)) = (squares.iter().map(|s| s.z).min(), squares.iter().map(|s| s.z).max()) {
        let (lo, hi) = if r.surfaces.is_some() { (lo.min(0), hi.max(0)) } else { (lo, hi) };
        for z in lo..=hi {
            r.render_translucent_floors(&squares, z, &mut out);
            if let Some(s) = r.surfaces.as_ref() {
                if z == 0 {
                    crate::water::render_water_shore(r, s, &squares, &mut out);
                }
            }
            r.render_translucent_objects(&squares, z, &mut out);
        }
    }
    out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    out.push(DrawCmd::OffscreenEnd);
    // Core.RenderOffScreenBuffer: 1:1 screen projection, no blending.
    out.push(DrawCmd::Ortho { left: 0.0, right: w as f32, bottom: h as f32, top: 0.0, width: w, height: h });
    out.push(DrawCmd::ScreenPass { x: 0.0, y: 0.0, w: w as f32, h: h as f32, tex: [0.0, 0.0, w as f32, h as f32], params: screen });
    out.push(DrawCmd::GlEnable(gl::BLEND));
    out
}

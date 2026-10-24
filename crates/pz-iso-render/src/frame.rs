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
    let (ow, oh) = size.offscreen();
    let (w, h) = (size.width, size.height);
    let z = r.cam.zoom;
    let mut out = vec![
        DrawCmd::OffscreenBegin { width: ow, height: oh },
        // glDoStartFrame: blending, depth func ALWAYS, zoomed projection.
        DrawCmd::GlEnable(gl::BLEND),
        DrawCmd::DepthFunc(gl::ALWAYS),
        DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h },
        DrawCmd::ColorMask(true, true, true, true),
        DrawCmd::DepthMask(true),
        DrawCmd::StencilMask(0xFF),
        DrawCmd::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT | gl::STENCIL_BUFFER_BIT),
    ];
    let mut recorded = Vec::new();
    for c in chunks {
        if c.dirty {
            r.record(c.texture, &c.levels, &mut out);
            out.push(DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h });
        }
        recorded.push(c.texture);
    }
    r.composite(&recorded, w, h, &mut out);
    // performRenderTiles' per-level translucent passes.
    let squares: Vec<&crate::world::RenderSquare> = chunks.iter().flat_map(|c| c.levels.iter().flat_map(|(_, l)| l.iter().flatten().copied())).collect();
    if let (Some(lo), Some(hi)) = (squares.iter().map(|s| s.z).min(), squares.iter().map(|s| s.z).max()) {
        for z in lo..=hi {
            r.render_translucent(&squares, z, &mut out);
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

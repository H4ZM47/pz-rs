//! One world frame as `IngameState.renderInternal` produces it for a
//! player: `Core.StartFrame` (offscreen buffer bound, zoomed projection,
//! cleared), the chunk textures recorded and composited
//! (`FBORenderCell.performRenderTiles`), the moving objects and their
//! shadows ([`crate::objects`]) and the per-level translucent passes,
//! `Core.EndFrame`, then `Core.RenderOffScreenBuffer` through the `screen`
//! shader.

use pz_render::{gl, DrawCmd, ScreenParams, Texture};

use crate::cell::{ChunkRenderer, LevelSquares};
use crate::chunk::{pow2, ChunkTexture};
use crate::models::ModelCamera;
use crate::objects::FrameObjects;
use crate::shadows::Shadows;

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
pub fn world_frame_into(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], screen: ScreenParams, out: Vec<DrawCmd>) -> Vec<DrawCmd> {
    world_frame_with_objects(r, size, chunks, &FrameObjects::default(), None, screen, out)
}

/// [`world_frame_into`] with the frame's moving objects: after the chunk
/// composite the players, corpses, items and other moving objects draw
/// (adding their shadows), and each level's shadows draw between its
/// translucent floors and translucent objects. `drop_shadow` is
/// `media/textures/NewShadow.png`; without it no shadows are drawn.
pub fn world_frame_with_objects(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, mut out: Vec<DrawCmd>) -> Vec<DrawCmd> {
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
    // FBORenderShadows.clear, then renderPlayers .. renderMovingObjects.
    let model_cam = ModelCamera::new(&r.cam, w as f32 * z, h as f32 * z);
    let mut shadows = Shadows::default();
    shadows.clear(drop_shadow);
    objects.render(&model_cam, &mut shadows, &mut out);
    // performRenderTiles' per-level translucent passes, over every
    // on-screen square (recorded this frame or not), with each level's
    // shadows between its translucent floors and objects.
    let squares: Vec<&crate::world::RenderSquare> = chunks.iter().flat_map(|c| c.levels.iter().flat_map(|(_, l)| l.iter().flatten().copied())).collect();
    let levels = squares.iter().map(|s| s.z).chain(shadows.shadows.iter().map(|s| pz_render::iso::fastfloor(s.oz)));
    if let (Some(lo), Some(hi)) = (levels.clone().min(), levels.max()) {
        for z in lo..=hi {
            r.render_translucent_floors(&squares, z, &mut out);
            shadows.render_main(z, &model_cam, &mut out);
            r.render_translucent_objects(&squares, z, &mut out);
        }
    }
    shadows.end_render();
    out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    out.push(DrawCmd::OffscreenEnd);
    // Core.RenderOffScreenBuffer: 1:1 screen projection, no blending.
    out.push(DrawCmd::Ortho { left: 0.0, right: w as f32, bottom: h as f32, top: 0.0, width: w, height: h });
    out.push(DrawCmd::ScreenPass { x: 0.0, y: 0.0, w: w as f32, h: h as f32, tex: [0.0, 0.0, w as f32, h as f32], params: screen });
    out.push(DrawCmd::GlEnable(gl::BLEND));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{ObjectRender, ShadowRequest};
    use crate::shadows::ShadowParams;
    use crate::world::{DepthMaps, DepthPreset};
    use crate::Camera;

    struct NoMaps;

    impl DepthMaps for NoMaps {
        fn preset(&self, _: DepthPreset) -> Option<Texture> {
            None
        }
        fn default_texture(&self) -> Option<Texture> {
            None
        }
    }

    /// The object pass follows the composite; each level's shadows come
    /// after the translucent floors and before the end of the frame.
    #[test]
    fn objects_and_shadows_take_their_place_in_the_frame() {
        let cam = Camera::player(10.5, 10.5, 0.0, 640, 480, 1.0);
        let r = ChunkRenderer::new(cam, &NoMaps, (640, 480));
        let marker = DrawCmd::StencilMask(0x5A);
        let shadow = ShadowRequest::Character { pos: [10.5, 10.5, 1.0], forward: [1.0, 0.0], params: ShadowParams::default(), rgb: [1.0; 3], alpha: 1.0, animal: false, slope: None };
        let objects = FrameObjects { players: vec![ObjectRender { shadow: Some(shadow), draws: vec![marker] }], ..Default::default() };
        let mut tex = Texture::empty("NewShadow");
        tex.id = Some(1);
        let cmds = world_frame_with_objects(&r, FrameSize { width: 640, height: 480 }, &[], &objects, Some(tex), ScreenParams { desaturation: 0.0, timer: 0.0 }, Vec::new());
        let pos = |f: &dyn Fn(&DrawCmd) -> bool| cmds.iter().position(f).unwrap();
        let clear = pos(&|c| matches!(c, DrawCmd::Clear(_)));
        let model = pos(&|c| matches!(c, DrawCmd::StencilMask(0x5A)));
        let quad = pos(&|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. }));
        let end = pos(&|c| matches!(c, DrawCmd::OffscreenEnd));
        assert!(clear < model && model < quad && quad < end);
        // Without the drop-shadow texture there are no shadows.
        let cmds = world_frame_with_objects(&r, FrameSize { width: 640, height: 480 }, &[], &objects, None, ScreenParams { desaturation: 0.0, timer: 0.0 }, Vec::new());
        assert!(!cmds.iter().any(|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. })));
    }
}

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
use crate::outlines::ModelOutlines;
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

/// `FBORenderCell.renderAnimatedAttachments` (after the water): each
/// on-screen chunk holding a fireplace's or barbecue's flames or smoke,
/// its squares level by level.
pub fn render_animated_attachments(r: &ChunkRenderer<'_>, chunks: &[ChunkLevels<'_>], out: &mut Vec<DrawCmd>) {
    let mut done: Vec<(i32, i32)> = Vec::new();
    for c in chunks {
        let k = (c.texture.wx, c.texture.wy);
        if done.contains(&k) {
            continue;
        }
        done.push(k);
        let mut squares: Vec<&crate::world::RenderSquare> = chunks
            .iter()
            .filter(|o| (o.texture.wx, o.texture.wy) == k)
            .flat_map(|o| o.levels.iter().flat_map(|(_, l)| l.iter().flatten().copied()))
            .collect();
        if !squares.iter().any(|s| s.objects.iter().any(crate::hearth::has_animated_attachments)) {
            continue;
        }
        squares.sort_by_key(|s| s.z);
        r.render_animated_attachments(&squares, out);
    }
}

/// [`world_frame`] appended to `out` (which may hold texture frees).
pub fn world_frame_into(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], screen: ScreenParams, out: Vec<DrawCmd>) -> Vec<DrawCmd> {
    world_frame_into_with(r, size, chunks, screen, out, &mut |_| {})
}

/// [`world_frame_into`] with `late` recording what `IsoWorld.render` draws
/// into the offscreen buffer after the cell (the weather effects,
/// [`crate::weather_fx_mask::render_weather_fx`]), in the world projection.
pub fn world_frame_into_with(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], screen: ScreenParams, out: Vec<DrawCmd>, late: &mut dyn FnMut(&mut Vec<DrawCmd>)) -> Vec<DrawCmd> {
    world_frame_full(r, size, chunks, &FrameObjects::default(), None, screen, out, late)
}

/// [`world_frame_into`] with the frame's moving objects: after the chunk
/// composite the players, corpses, items and other moving objects draw
/// (adding their shadows), and each level's shadows draw between its
/// translucent floors and translucent objects. `drop_shadow` is
/// `media/textures/NewShadow.png`; without it no shadows are drawn.
pub fn world_frame_with_objects(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, out: Vec<DrawCmd>) -> Vec<DrawCmd> {
    world_frame_full(r, size, chunks, objects, drop_shadow, screen, out, &mut |_| {})
}

/// The whole frame: moving objects and shadows, then `late` (the weather
/// effects) in the world projection before the offscreen buffer ends.
#[allow(clippy::too_many_arguments)]
pub fn world_frame_full(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, out: Vec<DrawCmd>, late: &mut dyn FnMut(&mut Vec<DrawCmd>)) -> Vec<DrawCmd> {
    world_frame_for_player_with(r, size, None, chunks, objects, drop_shadow, screen, out, late)
}

/// One player of a split screen: `player` of `num_players` on a window of
/// `screen` pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerView {
    pub player: u32,
    pub num_players: u32,
    pub screen: (u32, u32),
}

impl PlayerView {
    /// `DoStartFrameStuff(w, h, zoom, player)`'s viewport in GL window
    /// coordinates: `ow * (player % 2)`, and `oh` up for players 2 and 3,
    /// `ow` x `oh` being the screen halved per axis as the player count
    /// needs.
    pub fn viewport(&self) -> [u32; 4] {
        let (sw, sh) = self.screen;
        let ow = if self.num_players > 1 { sw / 2 } else { sw };
        let oh = if self.num_players > 2 { sh / 2 } else { sh };
        [ow * (self.player % 2), if self.player >= 2 { oh } else { 0 }, ow, oh]
    }
}

/// [`world_frame_with_objects`] for one player of a split screen (`size`
/// is the player's part of the screen): the world draws inside the
/// player's viewport of the shared offscreen buffer (as large as the whole
/// screen), the outline passes inside its screen rectangle, and the whole
/// buffer then goes to the screen.
#[allow(clippy::too_many_arguments)]
pub fn world_frame_for_player(r: &ChunkRenderer<'_>, size: FrameSize, view: Option<PlayerView>, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, out: Vec<DrawCmd>) -> Vec<DrawCmd> {
    world_frame_for_player_with(r, size, view, chunks, objects, drop_shadow, screen, out, &mut |_| {})
}

/// [`world_frame_for_player`] with `late` (see [`world_frame_full`]).
#[allow(clippy::too_many_arguments)]
pub fn world_frame_for_player_with(r: &ChunkRenderer<'_>, size: FrameSize, view: Option<PlayerView>, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, mut out: Vec<DrawCmd>, late: &mut dyn FnMut(&mut Vec<DrawCmd>)) -> Vec<DrawCmd> {
    let full = view.map_or(size, |v| FrameSize { width: v.screen.0, height: v.screen.1 });
    let (ow, oh) = full.offscreen();
    let (w, h) = (size.width, size.height);
    let z = r.cam.zoom;
    out.extend([
        DrawCmd::OffscreenBegin { width: ow, height: oh },
        // glDoStartFrame: blending, depth func ALWAYS, zoomed projection.
        DrawCmd::GlEnable(gl::BLEND),
        DrawCmd::DepthFunc(gl::ALWAYS),
        DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h },
    ]);
    if let Some(v) = view {
        let [x, y, width, height] = v.viewport();
        out.push(DrawCmd::Viewport { x, y, width, height });
    }
    out.extend([
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
    // FBORenderCorpses.update: each job into its chunk texture (not
    // cleared), then the screen frame again.
    for job in &objects.chunk_models {
        out.extend(job.texture.begin(false));
        out.extend(job.draws.iter().cloned());
        out.push(DrawCmd::ChunkEnd);
        out.push(DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h });
    }
    r.composite(&recorded, w, h, &mut out);
    let squares: Vec<&crate::world::RenderSquare> = chunks.iter().flat_map(|c| c.levels.iter().flat_map(|(_, l)| l.iter().flatten().copied())).collect();
    // performRenderTiles after the composite: puddles (quality 0 and 1),
    // then open water.
    if let Some(s) = r.surfaces.as_ref() {
        if s.settings.puddles_quality < 2 {
            crate::puddles::render_puddles(r, s, &squares, &mut out);
        }
        crate::water::render_water(r, s, &squares, &mut out);
    }
    // FBORenderShadows.clear, then renderPlayers .. renderMovingObjects.
    let model_cam = ModelCamera::new(&r.cam, w as f32 * z, h as f32 * z);
    let mut shadows = Shadows::default();
    shadows.clear(drop_shadow);
    // ModelOutlines.startFrameMain .. endFrameMain around IsoWorld.render.
    let mut outlines = ModelOutlines::new(full.width, full.height, z);
    outlines.view = view.map(|v| crate::outlines::player_screen(v.player, v.num_players, v.screen.0, v.screen.1));
    outlines.start_frame();
    objects.render_outlined(&model_cam, &mut shadows, &mut outlines, &mut out);
    render_animated_attachments(r, chunks, &mut out);
    // The per-level translucent passes, over every on-screen square
    // (recorded this frame or not): translucent floors, puddles on them,
    // shores (from level 0), the level's shadows, then translucent objects.
    let levels = squares.iter().map(|s| s.z).chain(shadows.shadows.iter().map(|s| pz_render::iso::fastfloor(s.oz)));
    if let (Some(lo), Some(hi)) = (levels.clone().min(), levels.max()) {
        let (lo, hi) = if r.surfaces.is_some() { (lo.min(0), hi.max(0)) } else { (lo, hi) };
        for z in lo..=hi {
            r.render_translucent_floors(&squares, z, &mut out);
            if let Some(s) = r.surfaces.as_ref() {
                crate::puddles::render_puddles_translucent_floors(r, s, &squares, z, &mut out);
                if z == 0 {
                    crate::water::render_water_shore(r, s, &squares, &mut out);
                }
                // renderRainSplashes: each on-screen chunk holding level z.
                let mut seen = Vec::new();
                for c in chunks.iter().filter(|c| (c.texture.min_level..=c.texture.top_level).contains(&z)) {
                    let k = (c.texture.wx, c.texture.wy);
                    if !seen.contains(&k) {
                        seen.push(k);
                        crate::splashes::render_rain_splashes(r, s, k.0, k.1, z, &mut out);
                    }
                }
            }
            shadows.render_main(z, &model_cam, &mut out);
            r.render_translucent_objects(&squares, z, &mut out);
        }
    }
    shadows.end_render();
    // FBORenderObjectOutline.render, after the fog.
    crate::object_outline::render_object_outlines(&objects.object_outlines, &mut outlines, &mut out);
    out.push(DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h });
    late(&mut out);
    outlines.end_frame(&mut out);
    out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    out.push(DrawCmd::OffscreenEnd);
    // Core.RenderOffScreenBuffer: 1:1 screen projection over the whole
    // screen, no blending.
    let (w, h) = (full.width, full.height);
    if view.is_some() {
        out.push(DrawCmd::Viewport { x: 0, y: 0, width: w, height: h });
    }
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
        let objects = FrameObjects { players: vec![ObjectRender { shadow: Some(shadow), draws: vec![marker], ..Default::default() }], ..Default::default() };
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

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

    /// `IsoCamera.getScreenLeft/Top/Width/Height(player)`: the player's
    /// part of the window, top-down (left half the window for players 1
    /// and 3, top half for 2 and 3, whatever the player count).
    pub fn screen_rect(&self) -> [u32; 4] {
        let (sw, sh) = self.screen;
        let left = if self.player == 1 || self.player == 3 { sw / 2 } else { 0 };
        let top = if self.player == 2 || self.player == 3 { sh / 2 } else { 0 };
        let w = if self.num_players > 1 { sw / 2 } else { sw };
        let h = if self.num_players > 2 { sh / 2 } else { sh };
        [left, top, w, h]
    }
}

/// [`world_frame_with_objects`] for one player of a split screen (`size`
/// is the player's part of the screen): the world draws inside the
/// player's viewport of the shared offscreen buffer (as large as the whole
/// screen), the outline passes inside its screen rectangle, and the
/// screen pass draws that rectangle of the buffer with the player's
/// `screen` values (`MultiTextureFBO2.render`'s per-player loop).
#[allow(clippy::too_many_arguments)]
pub fn world_frame_for_player(r: &ChunkRenderer<'_>, size: FrameSize, view: Option<PlayerView>, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, out: Vec<DrawCmd>) -> Vec<DrawCmd> {
    world_frame_for_player_with(r, size, view, chunks, objects, drop_shadow, screen, out, &mut |_| {})
}

/// [`world_frame_for_player`] with `late` (see [`world_frame_full`]).
#[allow(clippy::too_many_arguments)]
pub fn world_frame_for_player_with(r: &ChunkRenderer<'_>, size: FrameSize, view: Option<PlayerView>, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, screen: ScreenParams, out: Vec<DrawCmd>, late: &mut dyn FnMut(&mut Vec<DrawCmd>)) -> Vec<DrawCmd> {
    world_frame_for_player_extras(r, size, view, chunks, objects, drop_shadow, &mut FrameExtras::default(), screen, out, late)
}

/// The rest of what `performRenderTiles` and `IsoWorld.renderInternal`
/// draw into a player's offscreen buffer, each from its gameplay inputs;
/// `None` (the default) draws none of it.
#[derive(Default)]
pub struct FrameExtras<'a> {
    /// `renderFlies`: the corpse flies over rotting corpses
    /// (`RenderSquare::flies`) with `CorpseFlies`' animation frame.
    pub flies: Option<crate::flies::CorpseFlies>,
    /// `FBORenderObjectHighlight.render`: the objects whose
    /// [`crate::world::RenderObject::highlight`] is set, drawn over the
    /// chunk textures in their highlight colour.
    pub highlight: Option<crate::highlight::HighlightFrame>,
    /// `renderOpaqueObjectsEvent`: Lua's `RenderOpaqueObjectsInWorld`
    /// handlers (build ghosts, fishing bobbers), called with the square
    /// under the cursor (or the player, on a joypad) before the moving
    /// objects draw.
    pub opaque_event: Option<crate::markers::OpaqueObjectsEvent<'a>>,
    /// `VisibilityPolygon2.renderMain`: the vision cone, drawn on the
    /// camera character's level.
    pub vision: Option<crate::vispoly::VisionFrame>,
    /// `WorldMarkers.renderGridSquareMarkers` (`FBORenderWorldMarkers`)
    /// and `IsoMarkers.renderIsoMarkers`, per level.
    pub markers: Option<crate::markers::MarkersFrame>,
    /// `renderFog`: `ImprovedFog` over levels 0 and 1.
    pub fog: Option<crate::fog::FogFrame>,
    /// `FBORenderAreaHighlights.render` (after the weather) and
    /// `LineDrawer.render`'s lines.
    pub area_highlights: Option<crate::area_highlights::AreaHighlightsFrame>,
}

/// [`world_frame_full`] with the [`FrameExtras`] passes, each at its place
/// in `performRenderTiles` / `IsoWorld.renderInternal`.
#[allow(clippy::too_many_arguments)]
pub fn world_frame_extras(r: &ChunkRenderer<'_>, size: FrameSize, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, extras: &mut FrameExtras<'_>, screen: ScreenParams, out: Vec<DrawCmd>, late: &mut dyn FnMut(&mut Vec<DrawCmd>)) -> Vec<DrawCmd> {
    world_frame_for_player_extras(r, size, None, chunks, objects, drop_shadow, extras, screen, out, late)
}

/// [`world_frame_for_player_with`] with the [`FrameExtras`] passes.
#[allow(clippy::too_many_arguments)]
pub fn world_frame_for_player_extras(r: &ChunkRenderer<'_>, size: FrameSize, view: Option<PlayerView>, chunks: &[ChunkLevels<'_>], objects: &FrameObjects, drop_shadow: Option<Texture>, extras: &mut FrameExtras<'_>, screen: ScreenParams, mut out: Vec<DrawCmd>, late: &mut dyn FnMut(&mut Vec<DrawCmd>)) -> Vec<DrawCmd> {
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
    // FBORenderCell.performRenderTiles after the chunk composite
    // (FBORenderChunkManager.endFrame), in its order:
    // FBORenderShadows.clear; renderPlayers, renderCorpseShadows,
    // renderMannequinShadows, renderCorpsesInWorld, renderItemsInWorld;
    // renderPuddles (quality 0 and 1); renderOpaqueObjectsEvent;
    // renderMovingObjects; renderWater; renderAnimatedAttachments;
    // renderFlies; FBORenderObjectHighlight.render; then the per-level
    // translucent passes and renderFog.
    let model_cam = ModelCamera::new(&r.cam, w as f32 * z, h as f32 * z);
    let mut shadows = Shadows::default();
    shadows.clear(drop_shadow);
    // ModelOutlines.startFrameMain .. endFrameMain around IsoWorld.render.
    let mut outlines = ModelOutlines::new(full.width, full.height, z);
    outlines.view = view.map(|v| crate::outlines::player_screen(v.player, v.num_players, v.screen.0, v.screen.1));
    outlines.start_frame();
    objects.render_outlined_until_moving(&model_cam, &mut shadows, &mut outlines, &mut out);
    if let Some(s) = r.surfaces.as_ref() {
        if s.settings.puddles_quality < 2 {
            crate::puddles::render_puddles(r, s, &squares, &mut out);
        }
    }
    if let Some(e) = extras.opaque_event.as_mut() {
        e.trigger(&r.cam, &mut out);
    }
    objects.render_outlined_moving(&model_cam, &mut shadows, &mut outlines, &mut out);
    if let Some(s) = r.surfaces.as_ref() {
        crate::water::render_water(r, s, &squares, &mut out);
    }
    render_animated_attachments(r, chunks, &mut out);
    if let Some(f) = extras.flies.as_ref() {
        crate::flies::render_flies(r, f, chunks, &mut out);
    }
    if let Some(hl) = extras.highlight.as_ref() {
        crate::highlight::render(r, hl, &squares, &mut out);
    }
    // The per-level translucent passes, over every on-screen square
    // (recorded this frame or not): translucent floors, puddles on them,
    // shores (from level 0), shadows, the vision cone (on the camera
    // character's level), the grid-square and iso markers, then
    // translucent objects.
    let cam_level = pz_render::iso::fastfloor(r.cam.z);
    let mut levels: Vec<i32> = squares.iter().map(|s| s.z).chain(shadows.shadows.iter().map(|s| pz_render::iso::fastfloor(s.oz))).collect();
    if r.surfaces.is_some() {
        levels.push(0);
    }
    if extras.vision.is_some() {
        levels.push(cam_level);
    }
    if let Some(m) = extras.markers.as_ref() {
        levels.extend(m.levels());
    }
    if let (Some(&lo), Some(&hi)) = (levels.iter().min(), levels.iter().max()) {
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
            if z == cam_level {
                if let Some(v) = extras.vision.as_ref() {
                    crate::vispoly::render_main(r, v, size, &mut out);
                }
            }
            if let Some(m) = extras.markers.as_mut() {
                m.render_level(r, &model_cam, z, &mut out);
            }
            r.render_translucent_objects(&squares, z, &mut out);
        }
    }
    shadows.end_render();
    if let Some(f) = extras.fog.as_ref() {
        crate::fog::render_fog(r, f, chunks, size, &mut out);
    }
    // FBORenderObjectOutline.render, after the fog.
    crate::object_outline::render_object_outlines(&objects.object_outlines, &mut outlines, &mut out);
    out.push(DrawCmd::Ortho { left: 0.0, right: w as f32 * z, bottom: h as f32 * z, top: 0.0, width: w, height: h });
    // IsoWorld.renderInternal after the cell: the weather
    // (renderWeatherFX), FBORenderAreaHighlights.render, LineDrawer.render;
    // then IngameState's ModelOutlines.endFrameMain.
    late(&mut out);
    let mut lines = extras.markers.as_mut().map(|m| std::mem::take(&mut m.lines)).unwrap_or_default();
    if let Some(a) = extras.area_highlights.as_ref() {
        a.render(&r.cam, &model_cam, &mut lines, &mut out);
    }
    crate::area_highlights::render_lines(&r.cam, &lines, &mut out);
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
    // MultiTextureFBO2.render: each player's screen rectangle from the
    // same texels (`rendershader2(sx, sy, sw, sh, sx, sy, sw, sh)`) with
    // that player's WeatherShader values.
    let [sx, sy, sw, sh] = view.map_or([0, 0, w, h], |v| v.screen_rect()).map(|v| v as f32);
    out.push(DrawCmd::ScreenPass { x: sx, y: sy, w: sw, h: sh, tex: [sx, sy, sw, sh], params: screen });
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

    /// `IsoCamera.getScreenLeft/Top/Width/Height` per player.
    #[test]
    fn player_screen_rects() {
        let v = |player, num_players| PlayerView { player, num_players, screen: (1280, 720) }.screen_rect();
        assert_eq!(v(0, 1), [0, 0, 1280, 720]);
        assert_eq!([v(0, 2), v(1, 2)], [[0, 0, 640, 720], [640, 0, 640, 720]]);
        assert_eq!([v(2, 4), v(3, 4)], [[0, 360, 640, 360], [640, 360, 640, 360]]);
        // Player 2 of three: the bottom-left quarter.
        assert_eq!(v(2, 3), [0, 360, 640, 360]);
    }

    /// Each player's screen pass covers its own rectangle, from the same
    /// texels, with its own values.
    #[test]
    fn split_screen_pass_covers_the_player_rectangle() {
        let view = PlayerView { player: 1, num_players: 2, screen: (640, 480) };
        let cam = Camera::player(10.5, 10.5, 0.0, 320, 480, 1.0);
        let r = ChunkRenderer::new(cam, &NoMaps, (320, 480));
        let screen = ScreenParams { desaturation: 0.25, offscreen_left: 320.0, ..ScreenParams::default() };
        let cmds = world_frame_for_player(&r, FrameSize { width: 320, height: 480 }, Some(view), &[], &FrameObjects::default(), None, screen, Vec::new());
        let pass = cmds.iter().find_map(|c| match c {
            DrawCmd::ScreenPass { x, y, w, h, tex, params } => Some(([*x, *y, *w, *h], *tex, *params)),
            _ => None,
        });
        assert_eq!(pass, Some(([320.0, 0.0, 320.0, 480.0], [320.0, 0.0, 320.0, 480.0], screen)));
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
        let cmds = world_frame_with_objects(&r, FrameSize { width: 640, height: 480 }, &[], &objects, Some(tex), ScreenParams::default(), Vec::new());
        let pos = |f: &dyn Fn(&DrawCmd) -> bool| cmds.iter().position(f).unwrap();
        let clear = pos(&|c| matches!(c, DrawCmd::Clear(_)));
        let model = pos(&|c| matches!(c, DrawCmd::StencilMask(0x5A)));
        let quad = pos(&|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. }));
        let end = pos(&|c| matches!(c, DrawCmd::OffscreenEnd));
        assert!(clear < model && model < quad && quad < end);
        // Without the drop-shadow texture there are no shadows.
        let cmds = world_frame_with_objects(&r, FrameSize { width: 640, height: 480 }, &[], &objects, None, ScreenParams::default(), Vec::new());
        assert!(!cmds.iter().any(|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. })));
    }

    /// `performRenderTiles`' order after the chunk composite: players,
    /// corpses and items, then the puddles, the other moving objects, the
    /// water, the fireplace flames (`renderAnimatedAttachments`), the
    /// per-level passes (here the player's shadow), and last the weather
    /// (`late`, `IsoWorld.renderWeatherFX`) before the offscreen pass ends.
    #[test]
    fn the_frame_follows_perform_render_tiles() {
        use crate::cutaways::GridWorld;
        use crate::puddles::{PuddleSquare, PuddlesGeometry};
        use crate::water::{WaterGeometry, WaterInit};
        use crate::weather::{ClimateInputs, PuddlesState, SkyReflection, SurfaceSettings, SurfaceTextures, Surfaces, WaterState};
        use crate::world::{ObjectKind, RenderObject, RenderSquare};

        struct NoTextures;
        impl SurfaceTextures for NoTextures {
            fn texture(&self, _: &str) -> Option<Texture> {
                None
            }
        }

        let hearth = crate::hearth::tests::fireplace_square(Some(crate::hearth::tests::tex("DEPTH_fireplace", 128, 256, 30)));
        let mut world = GridWorld::default();
        let mut puddle = RenderSquare::new(11, 6, 0);
        let floor = crate::world::SpriteInfo {
            name: "street".into(),
            texture: Some(crate::hearth::tests::tex("street", 128, 256, 31)),
            flags: crate::world::SpriteFlags { solidfloor: true, exterior: true, ..Default::default() },
            tint_mod: [1.0; 3],
            ..Default::default()
        };
        puddle.objects.push(RenderObject::new(ObjectKind::Plain, Some(std::rc::Rc::new(floor))));
        let p = PuddleSquare { exterior: true, puddles_dir: 8, ..Default::default() };
        puddle.puddles = Some(PuddlesGeometry::init(&p, 0, |_, _| Some(p)));
        let mut water = RenderSquare::new(12, 6, 0);
        let tile = crate::world::SpriteInfo {
            name: "blends_natural_02_0".into(),
            texture: Some(crate::hearth::tests::tex("blends_natural_02_0", 128, 256, 32)),
            flags: crate::world::SpriteFlags { solidfloor: true, water: true, exterior: true, ..Default::default() },
            tint_mod: [1.0; 3],
            ..Default::default()
        };
        water.objects.push(RenderObject::new(ObjectKind::Plain, Some(std::rc::Rc::new(tile))));
        water.water = WaterGeometry::init(&WaterInit { water: true, exterior: true, shore_zone: 1, floor_sprite: Some("blends_natural_02_0"), neighbours: [Some(false); 8], flow: [(0.0, 0.0); 4], shader: true }).filter(|w| w.is_valid());
        assert!(water.water.is_some());
        world.squares.insert((11, 6, 0), puddle);
        world.squares.insert((12, 6, 0), water);
        world.squares.insert((10, 6, 0), hearth);
        let mut puddles = PuddlesState::default();
        puddles.wet_ground.set(1.0);
        puddles.puddles_size.set(0.7);
        let surfaces = Surfaces {
            settings: SurfaceSettings { puddles_quality: 1, ..Default::default() },
            climate: ClimateInputs::default(),
            water: WaterState::default(),
            puddles,
            sky: SkyReflection::default(),
            world_age_hours: 0.0,
            meta_grid_min_x: 0,
            world: &world,
            textures: &NoTextures,
            frame: Default::default(),
            rain: None,
            fish: None,
        };
        let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
        let maps = crate::hearth::tests::Maps;
        let r = ChunkRenderer::new(cam, &maps, (800, 600)).with_surfaces(Some(surfaces));
        let mut levels: crate::cell::LevelSquares<'_> = [None; 64];
        for x in 10..13 {
            levels[(x - 8) as usize + 6 * 8] = world.squares.get(&(x, 6, 0));
        }
        let chunks = [ChunkLevels { texture: crate::chunk::ChunkTexture::new(1, 0, 0, 0, 0), levels: vec![(0, levels)], dirty: false }];

        let mark = |n: u32| DrawCmd::StencilMask(n);
        let shadow = ShadowRequest::Character { pos: [10.5, 6.5, 0.0], forward: [1.0, 0.0], params: ShadowParams::default(), rgb: [1.0; 3], alpha: 1.0, animal: false, slope: None };
        let objects = FrameObjects {
            players: vec![ObjectRender { shadow: Some(shadow), draws: vec![mark(0x51)], ..Default::default() }],
            corpses: vec![mark(0x52)],
            items: vec![mark(0x53)],
            moving: vec![ObjectRender { draws: vec![mark(0x54)], ..Default::default() }],
            ..Default::default()
        };
        let mut drop_shadow = Texture::empty("NewShadow");
        drop_shadow.id = Some(1);
        let size = FrameSize { width: 800, height: 600 };
        let screen = ScreenParams::default();
        let cmds = world_frame_full(&r, size, &chunks, &objects, Some(drop_shadow), screen, Vec::new(), &mut |out| out.push(mark(0x59)));

        let mut alone = Vec::new();
        r.render_animated_attachments(&[world.squares.get(&(10, 6, 0)).unwrap()], &mut alone);
        let flame = crate::hearth::tests::sprites(&alone)[0].0.clone();
        let pos = |what: &str, f: &dyn Fn(&DrawCmd) -> bool| cmds.iter().position(f).unwrap_or_else(|| panic!("no {what}"));
        let order = [
            pos("composite", &|c| matches!(c, DrawCmd::Clear(_))),
            pos("player", &|c| matches!(c, DrawCmd::StencilMask(0x51))),
            pos("corpse", &|c| matches!(c, DrawCmd::StencilMask(0x52))),
            pos("item", &|c| matches!(c, DrawCmd::StencilMask(0x53))),
            pos("puddles", &|c| matches!(c, DrawCmd::Surface(d) if d.program == pz_render::SurfaceProgram::PuddlesMq)),
            pos("moving object", &|c| matches!(c, DrawCmd::StencilMask(0x54))),
            pos("water", &|c| matches!(c, DrawCmd::Surface(d) if d.program.is_water())),
            pos("flames", &|c| matches!(c, DrawCmd::WorldSprite(s, _) if *s == flame)),
            pos("shadow", &|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. })),
            pos("weather", &|c| matches!(c, DrawCmd::StencilMask(0x59))),
            pos("end", &|c| matches!(c, DrawCmd::OffscreenEnd)),
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    }

    /// The extras in `performRenderTiles`' and `IsoWorld.renderInternal`'s
    /// order: the opaque-objects event between the items and the moving
    /// objects; flies and the object highlight after them; on the camera
    /// character's level the vision cone, then the grid-square markers;
    /// the fog after the level loop; after the weather the area highlights
    /// and the queued lines.
    #[test]
    fn the_extras_follow_perform_render_tiles() {
        use crate::area_highlights::{AreaHighlight, AreaHighlightsFrame};
        use crate::color::ColorInfo;
        use crate::markers::{GridSquareMarker, MarkersFrame, OpaqueObjectsEvent};
        use crate::world::{ObjectHighlight, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo};
        use std::rc::Rc;

        let tex = |name: &str, id: u32| {
            let mut t = Texture::empty(name);
            t.id = Some(id);
            t.width = 128;
            t.height = 256;
            t.width_hw = 128;
            t.height_hw = 256;
            t.ready = true;
            t
        };
        let floor = Rc::new(SpriteInfo { name: "floor".into(), texture: Some(tex("floor", 40)), flags: SpriteFlags { solidfloor: true, exterior: true, ..Default::default() }, tint_mod: [1.0; 3], ..Default::default() });
        let mut flies = RenderSquare::new(10, 6, 0);
        flies.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
        flies.flies = true;
        let mut lit = RenderSquare::new(11, 6, 0);
        lit.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor)));
        let mut crate_ = RenderObject::new(ObjectKind::Plain, Some(Rc::new(SpriteInfo { name: "crate".into(), texture: Some(tex("crate", 41)), tint_mod: [1.0; 3], ..Default::default() })));
        crate_.highlight = Some(ObjectHighlight { color: ColorInfo::new(1.0, 0.0, 0.0, 0.5), alpha_scale: 1.0 });
        lit.objects.push(crate_);
        let mut levels: LevelSquares<'_> = [None; 64];
        levels[2 + 6 * 8] = Some(&flies);
        levels[3 + 6 * 8] = Some(&lit);
        let chunks = [ChunkLevels { texture: crate::chunk::ChunkTexture::new(1, 0, 0, 0, 0), levels: vec![(0, levels)], dirty: false }];
        let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
        let r = ChunkRenderer::new(cam, &NoMaps, (800, 600));
        let mark = |n: u32| DrawCmd::StencilMask(n);
        let objects = FrameObjects {
            players: vec![ObjectRender { draws: vec![mark(0x51)], ..Default::default() }],
            moving: vec![ObjectRender { draws: vec![mark(0x54)], ..Default::default() }],
            ..Default::default()
        };
        let valid = |_: i32, _: i32, _: i32| true;
        let mut handler = |_: i32, _: i32, _: i32, out: &mut Vec<DrawCmd>| out.push(DrawCmd::StencilMask(0x60));
        let mut fog = crate::fog::ImprovedFog::default();
        fog.init(1.0);
        let mut extras = FrameExtras {
            flies: Some(crate::flies::CorpseFlies { texture: Some(tex("CorpseFlies", 42)), ..Default::default() }),
            highlight: Some(Default::default()),
            opaque_event: Some(OpaqueObjectsEvent { joypad: true, picked_tile: None, valid_square: &valid, handler: &mut handler }),
            vision: Some(crate::vispoly::VisionFrame { vision_cone: -0.3, look_vector: [1.0, 0.0], chunk_map: [0, 0, 32, 32], ..Default::default() }),
            markers: Some(MarkersFrame {
                grid_square_markers: vec![GridSquareMarker { orig: [10.0, 7.0, 0.0], z: 0.0, size: 1.0, rgb: [1.0; 3], alpha: 1.0, active: true, texture_name: "circle_center".into(), overlay_texture_name: None }],
                circle_center: Some(tex("circle_center", 43)),
                chunk_map: Some([0, 0, 32, 32]),
                ..Default::default()
            }),
            fog: Some(crate::fog::FogFrame { fog, ..Default::default() }),
            area_highlights: Some(AreaHighlightsFrame {
                white: Some(tex("white", 44)),
                chunk_map: Some([0, 0, 32, 32]),
                user_defined: vec![(None, AreaHighlight { x1: 8, y1: 4, x2: 12, y2: 8, z: 0, rgba: [0.0, 1.0, 0.0, 0.3] })],
                ..Default::default()
            }),
        };
        let size = FrameSize { width: 800, height: 600 };
        let cmds = world_frame_extras(&r, size, &chunks, &objects, None, &mut extras, ScreenParams::default(), Vec::new(), &mut |out| out.push(mark(0x59)));
        let sprite_of = |c: &DrawCmd, id: u32| matches!(c, DrawCmd::WorldSprite(s, _) if s.texture.as_ref().and_then(|t| t.id) == Some(id));
        let quad_of = |c: &DrawCmd, id: u32| matches!(c, DrawCmd::ProjectedDepthQuad { texture, .. } if texture.id == Some(id));
        let pos = |what: &str, f: &dyn Fn(&DrawCmd) -> bool| cmds.iter().position(f).unwrap_or_else(|| panic!("no {what}"));
        let order = [
            pos("composite", &|c| matches!(c, DrawCmd::Clear(_))),
            pos("player", &|c| matches!(c, DrawCmd::StencilMask(0x51))),
            pos("opaque event", &|c| matches!(c, DrawCmd::StencilMask(0x60))),
            pos("moving object", &|c| matches!(c, DrawCmd::StencilMask(0x54))),
            pos("flies", &|c| sprite_of(c, 42)),
            pos("highlight", &|c| sprite_of(c, 41)),
            pos("vision", &|c| matches!(c, DrawCmd::VisibilityPolygon(_))),
            pos("grid marker", &|c| quad_of(c, 43)),
            pos("fog", &|c| matches!(c, DrawCmd::ImprovedFog(_))),
            pos("weather", &|c| matches!(c, DrawCmd::StencilMask(0x59))),
            pos("area highlight", &|c| quad_of(c, 44)),
            pos("area outline", &|c| matches!(c, DrawCmd::Sprite(_))),
            pos("end", &|c| matches!(c, DrawCmd::OffscreenEnd)),
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    }
}

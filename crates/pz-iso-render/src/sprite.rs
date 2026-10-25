//! `IsoSprite.renderCurrentAnim_FBORender`: sprite placement
//! (`prepareToRenderSprite`), the depth-texture choice (`setupTileDepth*`,
//! `startTileDepthShader`) and the quad (`performRenderFrame`), either while
//! recording a chunk texture or in the per-frame translucent pass
//! (`renderTranslucentOnly`).

use pz_render::iso::{self, coordmodulof, fastfloor, x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::{gl, DrawCmd, Rgba, Texture, WorldShader};

use crate::chunk::ChunkTexture;
use crate::color::ColorInfo;
use crate::texd::{self, FloorShaper, TexDraw, WallPart, WallShaper};
use crate::seam::{SeamModifier, SeamTile};
use crate::hearth::HearthRender;
use crate::world::{DepthMaps, DepthPreset, ObjectKind, RenderObject, SpriteInfo};
use crate::Camera;

/// The `Consumer<TextureDraw>` a sprite is drawn with.
#[derive(Clone, Copy, Debug)]
pub enum Modifier {
    None,
    /// `FloorShaperDeDiamond`.
    FloorDeDiamond(FloorShaper),
    /// `FloorShaperDiamond`.
    FloorDiamond(FloorShaper),
    /// `FloorShaperAttachedSprites`.
    FloorAttached(FloorShaper),
    Wall(WallShaper),
}

impl Modifier {
    fn apply(&self, d: &mut TexDraw) {
        match self {
            Modifier::None => {}
            Modifier::FloorDeDiamond(f) => f.accept_de_diamond(d),
            Modifier::FloorDiamond(f) => f.accept_diamond(d, 1),
            Modifier::FloorAttached(f) => f.accept(d),
            Modifier::Wall(w) => w.accept(d),
        }
    }

    fn wall(&self) -> Option<&WallShaper> {
        match self {
            Modifier::Wall(w) => Some(w),
            _ => None,
        }
    }
}

/// `IsoDirections` as the wall paths pass it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    N,
    W,
    NW,
    SE,
    Other,
}

/// What a sprite draw needs from its surroundings.
pub struct SpriteCtx<'a> {
    pub cam: &'a Camera,
    /// The chunk texture being recorded, or `None` in the translucent pass.
    pub chunk: Option<ChunkTexture>,
    pub depth_maps: &'a dyn DepthMaps,
    pub out: &'a mut Vec<DrawCmd>,
    /// `FBORenderObjectHighlight.isRendering()`: the highlight pass, whose
    /// sprites sit just in front of the chunk textures' copies.
    pub highlight_pass: bool,
}

/// One sprite draw: `IsoSprite.render(inst, obj, x, y, z, dir, offsetX,
/// offsetY, col, true, modifier)`.
pub struct SpriteDraw<'a> {
    pub sprite: &'a SpriteInfo,
    pub obj: Option<&'a RenderObject>,
    /// Whether `sprite` is the object's main sprite (not attached).
    pub main: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub dir: Dir,
    pub offset_x: f32,
    pub offset_y: f32,
    pub col: ColorInfo,
    /// The sprite instance's alpha and flip.
    pub alpha: f32,
    pub flip: bool,
    pub modifier: Modifier,
    /// `IsoSprite.seamFix2` and the seam modifier's vertex colours.
    pub seam: Option<(SeamTile, [u32; 4])>,
}

/// The steady-state instance scale `performRenderFrame` settles on: 64x128
/// textures are drawn at twice their size at tile scale 2.
fn instance_scale(tex: &Texture) -> f32 {
    frame_scale(tex, 1.0)
}

/// `performRenderFrame`'s scale for an instance at `inst`: 64x128 textures
/// go to 2, and 128x256 textures at 2 back to 1, at tile scale 2.
fn frame_scale(tex: &Texture, inst: f32) -> f32 {
    let mut s = inst;
    if TILE_SCALE == 2 && tex.width_orig() == 64 && tex.height_orig() == 128 {
        s = 2.0;
    }
    if TILE_SCALE == 2 && s == 2.0 && tex.width_orig() == 128 && tex.height_orig() == 256 {
        s = 1.0;
    }
    s
}

/// `IsoSprite.startTileDepthShader` while recording: the front and far
/// depths relative to the chunk texture.
fn tile_depths(ctx: &SpriteCtx<'_>, mut x: f32, mut y: f32, z: f32, z2: f32) -> (f32, f32) {
    if ctx.chunk.is_none() {
        x += ctx.cam.fix_jiggly_square_x;
        y += ctx.cam.fix_jiggly_square_y;
    }
    let (cx, cy) = (fastfloor(ctx.cam.x), fastfloor(ctx.cam.y));
    let mut far = iso::square_depth(cx, cy, x, y, z);
    let mut front = iso::square_depth(cx, cy, x + 1.0, y + 1.0, z + 1.0);
    if z != z2 {
        far -= (z2 - z) * iso::LEVEL_DEPTH;
        front -= (z2 - z) * iso::LEVEL_DEPTH;
    }
    if ctx.chunk.is_none() {
        if ctx.highlight_pass {
            front -= 5.0e-5;
            far -= 5.0e-5;
        }
        return (front, far);
    }
    let chunk_depth = iso::chunk_depth_data(ctx.cam.chunk_x(), ctx.cam.chunk_y(), fastfloor(x / 8.0), fastfloor(y / 8.0), fastfloor(z)).depth_start;
    (front - chunk_depth, far - chunk_depth)
}

/// `getParentSpriteDepthTextureToUse`.
fn parent_depth_texture(d: &SpriteDraw<'_>) -> Option<Texture> {
    if d.main {
        return None;
    }
    let s = d.sprite;
    let use_object = s.depth_flags & 1 != 0 || (s.flags.wall_overlay && (s.flags.attached_n || s.flags.attached_w));
    if !use_object {
        return None;
    }
    d.obj?.sprite.as_ref()?.depth_texture.clone()
}

/// `TileDepthModifier.setupWallDepth`'s preset choice.
fn wall_preset(sprite: &SpriteInfo, dir: Dir) -> DepthPreset {
    let f = &sprite.flags;
    match dir {
        Dir::N if f.door_wall_n && !f.door_n => DepthPreset::NDoorFrame,
        Dir::N => DepthPreset::NWall,
        Dir::NW => DepthPreset::NWWall,
        Dir::W if f.wall_se => DepthPreset::SEWall,
        Dir::W if f.door_wall_w && !f.door_w => DepthPreset::WDoorFrame,
        Dir::W => DepthPreset::WWall,
        Dir::SE => DepthPreset::SEWall,
        Dir::Other => DepthPreset::Floor,
    }
}

/// `setupWallDepth`: the sprite's own depth texture, else the preset.
fn wall_depth(ctx: &SpriteCtx<'_>, sprite: &SpriteInfo, dir: Dir) -> Option<Texture> {
    sprite.depth_texture.clone().or_else(|| ctx.depth_maps.preset(wall_preset(sprite, dir)))
}

/// `setupTileDepth`: the depth texture for a non-wall draw, or `None` for
/// sprites drawn with the plain shader (trees, items).
fn setup_tile_depth(ctx: &SpriteCtx<'_>, d: &SpriteDraw<'_>) -> Option<Texture> {
    if let Some(obj) = d.obj {
        match obj.kind {
            ObjectKind::Tree | ObjectKind::WorldInventory => return None,
            // The billboard depth texture, if not empty.
            ObjectKind::Fire if !d.main => return ctx.depth_maps.billboard(),
            _ => {}
        }
        // A barbecue's smoke: the plain shader.
        if !d.main && obj.sprite.is_some() && matches!(obj.hearth, Some(HearthRender::Barbecue)) {
            return None;
        }
    }
    let s = d.sprite;
    let f = &s.flags;
    if let Some(t) = &s.depth_texture {
        return Some(t.clone());
    }
    if f.solidfloor || f.floor_overlay || s.render_layer == 1 {
        return ctx.depth_maps.preset(DepthPreset::Floor);
    }
    if let Some(t) = parent_depth_texture(d) {
        return Some(t);
    }
    let window_dir = |n: bool| matches!(d.obj.map(|o| o.kind), Some(ObjectKind::Window { .. })) && d.obj.and_then(|o| o.sprite.as_ref()).is_some_and(|s| if n { s.flags.window_n } else { s.flags.window_w });
    if f.window_n || window_dir(true) {
        return wall_depth(ctx, s, Dir::N);
    }
    if f.window_w || window_dir(false) {
        return wall_depth(ctx, s, Dir::W);
    }
    if f.wall_overlay && f.attached_w {
        return wall_depth(ctx, s, Dir::W);
    }
    if f.wall_overlay && f.attached_n {
        return wall_depth(ctx, s, Dir::N);
    }
    // A fireplace's flames and logs take the fireplace's depth texture.
    if let Some(obj) = d.obj.filter(|_| !d.main) {
        if let (Some(HearthRender::Fireplace { uses_our_depth: true, .. }), Some(t)) = (obj.hearth, obj.sprite.as_ref().and_then(|s| s.depth_texture.as_ref())) {
            return Some(t.clone());
        }
    }
    ctx.depth_maps.default_texture()
}

/// `setupTileDepthWall`.
fn setup_tile_depth_wall(ctx: &SpriteCtx<'_>, d: &SpriteDraw<'_>) -> Option<Texture> {
    if let Some(t) = &d.sprite.depth_texture {
        return Some(t.clone());
    }
    if let Some(t) = parent_depth_texture(d) {
        return Some(t);
    }
    let mut sprite = d.sprite;
    if !d.main && d.sprite.depth_flags & 1 != 0 {
        if let Some(s) = d.obj.and_then(|o| o.sprite.as_deref()) {
            sprite = s;
        }
    }
    wall_depth(ctx, sprite, d.dir)
}

/// `IsoSprite.render` into the chunk texture being recorded.
pub fn render(ctx: &mut SpriteCtx<'_>, d: &SpriteDraw<'_>) {
    render_inst(ctx, d, None);
}

/// [`render`] for an instance whose scale was set (`setScale(scale,
/// scale)`) before the draw, as `IsoFire` does for its animations.
pub fn render_scaled(ctx: &mut SpriteCtx<'_>, d: &SpriteDraw<'_>, scale: f32) {
    render_inst(ctx, d, Some(scale));
}

fn render_inst(ctx: &mut SpriteCtx<'_>, d: &SpriteDraw<'_>, inst_scale: Option<f32>) {
    let s = d.sprite;
    // performRenderFrame skips shore blends the water shader replaces.
    if s.hide_for_water_render {
        return;
    }
    let Some(tex) = s.texture.as_ref() else { return };
    let mut info = d.col;
    if s.flags.unlit {
        info.r = 1.0;
        info.g = 1.0;
        info.b = 1.0;
    }
    // prepareToRenderSprite: chunk-local position while recording, else
    // the camera offset.
    let ((gox, goy), lx, ly) = match ctx.chunk {
        Some(c) => (c.record_offset(), coordmodulof(d.x, 8), coordmodulof(d.y, 8)),
        None => ((-ctx.cam.off_x as f32, -ctx.cam.off_y as f32), d.x, d.y),
    };
    let mut sx = x_to_screen(lx, ly);
    let mut sy = y_to_screen(lx, ly, d.z, 0);
    sx -= d.offset_x;
    sy -= d.offset_y;
    sx += gox;
    sy += goy;
    sx += s.soff_x as f32;
    sy += s.soff_y as f32;
    if ctx.chunk.is_none() {
        sx += ctx.cam.fix_jiggly_x * ctx.cam.zoom;
        sy += ctx.cam.fix_jiggly_y * ctx.cam.zoom;
    }
    info.a = d.alpha;
    if s.tint_mod != [1.0; 3] {
        info.r *= s.tint_mod[0];
        info.g *= s.tint_mod[1];
        info.b *= s.tint_mod[2];
    }
    let mut sz = iso::calculate_depth(lx, ly, d.z);
    if matches!(d.obj.map(|o| o.kind), Some(ObjectKind::Tree)) {
        sz = iso::calculate_depth(lx + 0.99, ly + 0.99, d.z);
    }
    ctx.out.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
    ctx.out.push(DrawCmd::DepthFunc(gl::LEQUAL));
    if ctx.chunk.is_none() {
        ctx.out.push(DrawCmd::DepthMask(false));
    }

    if let Some((tile, col)) = d.seam {
        render_seam(ctx, d, tex, tile, col, info, sx, sy);
        return;
    }
    let wall = d.modifier.wall().is_some_and(|w| w.part == WallPart::Whole || w.part == WallPart::SliceN || w.part == WallPart::SliceW);
    let depth: Option<(Texture, f32, f32, (f32, f32))> = if d.dir == Dir::NW && d.modifier.wall().is_some() {
        setup_tile_depth_wall(ctx, d).map(|t| {
            let (front, far) = tile_depths(ctx, d.x, d.y, d.z, d.z);
            (t, front, far, (1.0, 1.0))
        })
    } else if !wall {
        let mut z2 = d.z;
        if let Some(o) = d.obj {
            if o.render_y_offset != 0.0 {
                z2 += o.render_y_offset / 96.0;
            }
        }
        setup_tile_depth(ctx, d).map(|t| {
            // Fire animations take the billboard's depth half a square
            // back (`startTileDepthShader(obj, x - 0.5, y - 0.5, …)`).
            let back = if !d.main && matches!(d.obj.map(|o| o.kind), Some(ObjectKind::Fire)) { 0.5 } else { 0.0 };
            let (mut front, mut far) = tile_depths(ctx, d.x - back, d.y - back, d.z, z2);
            // In the per-frame passes a fireplace's attached sprites sit
            // just in front of the fireplace.
            if ctx.chunk.is_none() && !ctx.highlight_pass && !d.main && matches!(d.obj.and_then(|o| o.hearth), Some(HearthRender::Fireplace { .. })) {
                front -= 5.0e-5;
                far -= 5.0e-5;
            }
            // TileDepthModifier.setSpriteScale(def.scaleX, def.scaleY),
            // before performRenderFrame adjusts the instance.
            let sc = inst_scale.unwrap_or_else(|| instance_scale(tex));
            (t, front, far, (sc, sc))
        })
    } else {
        let wall_shader = d.modifier.wall().expect("wall");
        let (x, y) = match wall_shader.part {
            WallPart::SliceN => (d.x + 1.0, d.y),
            WallPart::SliceW => (d.x, d.y + 1.0),
            _ => (d.x, d.y),
        };
        let t = if matches!(wall_shader.part, WallPart::SliceN | WallPart::SliceW) {
            // setupTileDepthWall2: the sprite's own depth texture or the
            // wall preset.
            wall_depth(ctx, d.sprite, d.dir)
        } else {
            parent_depth_texture(d).or_else(|| setup_tile_depth_wall(ctx, d))
        };
        t.map(|t| {
            let (front, far) = tile_depths(ctx, x, y, d.z, d.z);
            (t, front, far, (1.0, 1.0))
        })
    };
    if depth.is_none() && wall {
        return;
    }

    // performRenderFrame.
    let scale = match inst_scale {
        Some(i) => frame_scale(tex, i),
        None => instance_scale(tex),
    };
    if scale <= 0.0 {
        return;
    }
    let (mut w, mut h) = (tex.width as f32, tex.height as f32);
    let (mut tx, mut ty) = (sx, sy);
    if scale != 1.0 {
        tx += tex.offset_x * (scale - 1.0);
        w *= scale;
        ty += tex.offset_y * (scale - 1.0);
        h *= scale;
    }
    let mut tex = tex.clone();
    if d.flip {
        tex.flip = !tex.flip;
    }
    let Some(mut draw) = TexDraw::new(&tex, tx + tex.offset_x, ty + tex.offset_y, w, h, Rgba(info.r, info.g, info.b, info.a)) else { return };
    let shader = match depth {
        Some((dt, front, far, sc)) => {
            texd::tile_depth_modifier(&mut draw, &dt, (s.soff_x, s.soff_y), sc);
            WorldShader::TileDepth { depth: dt, uv1: [[0.0; 2]; 4], front, far, opaque: s.depth_flags & 4 != 0 }
        }
        None => {
            // The translucent pass adds the chunk's depth (`nextChunkDepth`)
            // in GL clip space; in a chunk texture the composite adds it.
            let mut chunk_depth = 0.0;
            if ctx.chunk.is_none() {
                chunk_depth = iso::chunk_depth_data(ctx.cam.chunk_x(), ctx.cam.chunk_y(), fastfloor(d.x / 8.0), fastfloor(d.y / 8.0), fastfloor(d.z)).depth_start;
                if s.soff_y != 0 && !matches!(d.obj.map(|o| o.kind), Some(ObjectKind::Fire)) {
                    chunk_depth += s.soff_y as f32 / 96.0 * iso::LEVEL_DEPTH;
                }
                chunk_depth += 0.5;
                chunk_depth = chunk_depth * 2.0 - 1.0;
            }
            WorldShader::Default { z: chunk_depth + (sz * 2.0 - 1.0) }
        }
    };
    d.modifier.apply(&mut draw);
    ctx.out.push(draw.into_cmd(shader));
}

/// A sprite drawn with `seamFix2` set: `TileSeamModifier` in place of the
/// other modifiers and the seam shader.
#[allow(clippy::too_many_arguments)]
fn render_seam(ctx: &mut SpriteCtx<'_>, d: &SpriteDraw<'_>, tex: &Texture, tile: SeamTile, col: [u32; 4], info: ColorInfo, sx: f32, sy: f32) {
    let Some(mask) = ctx.depth_maps.seam_mask(tile) else { return };
    let s = d.sprite;
    let (depth, z2) = if tile.is_floor() {
        // setupTileDepth: the sprite's own depth texture, else the floor
        // preset.
        let mut z2 = d.z;
        if let Some(o) = d.obj {
            if o.render_y_offset != 0.0 {
                z2 += o.render_y_offset / 96.0;
            }
        }
        let t = match &s.depth_texture {
            Some(t) => Some(t.clone()),
            None if s.flags.solidfloor || s.flags.floor_overlay || s.render_layer == 1 => ctx.depth_maps.preset(DepthPreset::Floor),
            None => None,
        };
        (t, z2)
    } else {
        // TileSeamModifier.setupWallDepth.
        (s.depth_texture.clone().or_else(|| ctx.depth_maps.preset(wall_preset(s, d.dir))), d.z)
    };
    let Some(depth) = depth else { return };
    let (front, far) = tile_depths(ctx, d.x, d.y, d.z, z2);
    let scale = instance_scale(tex);
    let (mut w, mut h) = (tex.width as f32, tex.height as f32);
    let (mut tx, mut ty) = (sx, sy);
    if scale != 1.0 {
        tx += tex.offset_x * (scale - 1.0);
        w *= scale;
        ty += tex.offset_y * (scale - 1.0);
        h *= scale;
    }
    let Some(mut draw) = TexDraw::new(tex, tx + tex.offset_x, ty + tex.offset_y, w, h, Rgba(info.r, info.g, info.b, info.a)) else { return };
    let m = if tile.is_floor() { SeamModifier::floor(tile, depth.clone(), mask.clone(), col) } else { SeamModifier { tile, depth: depth.clone(), mask: mask.clone(), col } };
    m.accept(&mut draw);
    ctx.out.push(draw.into_cmd(WorldShader::Seam { depth, uv1: [[0.0; 2]; 4], mask, uv2: [[0.0; 2]; 4], front, far }));
}

/// `SpriteRenderer.WallShaderTexRender`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallTexRender {
    All,
    LeftOnly,
    RightOnly,
}

/// One `DoCutawayShader` draw: the stencil cell and the wall's depth preset.
pub struct CutawayDraw {
    pub stencil: Texture,
    /// `setCutawayTexture(tex, x, y, w, h)`.
    pub region: [i32; 4],
    pub depth: Texture,
    pub tex_render: WallTexRender,
    /// `discardValue` (1 for the wall, 0 for window-frame outlines).
    pub discard_value: f32,
}

/// `IsoObject.renderWallTileOnly(null, …, CircleStencilShader, shaper)`:
/// the wall quad cut to the intersection of the sprite, stencil cell and
/// depth texture (`GenericSpriteRenderState.render` with cutaway textures),
/// coloured by the wall shaper, drawn with `CircleStencil`.
pub fn render_cutaway(ctx: &mut SpriteCtx<'_>, d: &SpriteDraw<'_>, shaper: &WallShaper, c: &CutawayDraw) {
    let s = d.sprite;
    let Some(tex0) = s.texture.as_ref() else { return };
    let mut info = d.col;
    if s.flags.unlit {
        info.r = 1.0;
        info.g = 1.0;
        info.b = 1.0;
    }
    info.a = d.alpha;
    if info.a == 0.0 {
        return;
    }
    let ((gox, goy), lx, ly) = match ctx.chunk {
        Some(ch) => (ch.record_offset(), coordmodulof(d.x, 8), coordmodulof(d.y, 8)),
        None => ((-ctx.cam.off_x as f32, -ctx.cam.off_y as f32), d.x, d.y),
    };
    let sx = x_to_screen(lx, ly) - d.offset_x + gox + s.soff_x as f32;
    let sy = y_to_screen(lx, ly, d.z, 0) - d.offset_y + goy + s.soff_y as f32;
    let (front, far) = tile_depths(ctx, d.x, d.y, d.z, d.z);
    let (t1, t2) = (&c.stencil, &c.depth);
    let col = crate::color::to_abgr(info.r, info.g, info.b, info.a);
    let q = cutaway_quad(tex0, t1, c.region, Some(t2), c.tex_render, sx, sy, col);
    let mut sprite = q.sprite;
    if d.flip {
        sprite.flipped = true;
    }
    let (uv_stencil, uv_depth) = (q.uv_stencil, q.uv_depth);
    let mut draw = TexDraw { sprite, tex1: Some(t2.clone()), uv1: uv_depth, tex2: Some(t1.clone()), uv2: uv_stencil };
    shaper.accept(&mut draw);
    ctx.out.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
    ctx.out.push(DrawCmd::DepthFunc(gl::LEQUAL));
    ctx.out.push(draw.into_cmd(WorldShader::Cutaway { depth: t2.clone(), uv1: [[0.0; 2]; 4], stencil: t1.clone(), uv2: [[0.0; 2]; 4], front, far, discard_value: c.discard_value }));
}

/// `GenericSpriteRenderState.render`'s cutaway branch: the quad of `tex0`
/// drawn at `left, top` (the position minus the trim offset) cut to the
/// intersection of the texture, the stencil cell `region`
/// (`setCutawayTexture`) and `tex2` (`setCutawayTexture2`, 128x256 at no
/// offset when there is none), halved by `tex_render`, with the stencil's
/// and `tex2`'s corner coordinates.
pub struct CutawayQuad {
    pub sprite: pz_render::Sprite,
    pub uv_stencil: [[f32; 2]; 4],
    pub uv_depth: [[f32; 2]; 4],
}

#[allow(clippy::too_many_arguments)]
pub fn cutaway_quad(tex0: &Texture, stencil: &Texture, region: [i32; 4], tex2: Option<&Texture>, tex_render: WallTexRender, left: f32, top: f32, col: u32) -> CutawayQuad {
    let [cx, cy, cw, ch] = region;
    let t1x = (cx % 128) as f32;
    let t1y = (cy % 226) as f32;
    let (t2x, t2y, t2w, t2h) = tex2.map_or((0.0, 0.0, 128.0, 256.0), |t| (t.offset_x, t.offset_y, t.width as f32, t.height as f32));
    let mut x0 = tex0.offset_x.max(t1x).max(t2x);
    let mut x1 = (tex0.offset_x + tex0.width as f32).min(t1x + cw as f32).min(t2x + t2w);
    let y0 = tex0.offset_y.max(t1y).max(t2y);
    let y2 = (tex0.offset_y + tex0.height as f32).min(t1y + ch as f32).min(t2y + t2h);
    match tex_render {
        WallTexRender::LeftOnly => x1 = x1.min(63.0),
        WallTexRender::RightOnly => x0 = x0.max(63.0),
        WallTexRender::All => {}
    }
    let xs = [x0, x1, x1, x0];
    let ys = [y0, y0, y2, y2];
    let u = |t: &Texture, x: f32, off: f32| (t.x_start * t.width_hw as f32 + (x - off)) / t.width_hw as f32;
    let v = |t: &Texture, y: f32, off: f32| (t.y_start * t.height_hw as f32 + (y - off)) / t.height_hw as f32;
    let sprite = pz_render::Sprite {
        texture: Some(tex0.clone()),
        x: xs.map(|x| left + x),
        y: ys.map(|y| top + y),
        u: xs.map(|x| u(tex0, x, tex0.offset_x)),
        v: ys.map(|y| v(tex0, y, tex0.offset_y)),
        color: [col; 4],
        flipped: false,
    };
    let uv_stencil = std::array::from_fn(|i| [(cx as f32 + (xs[i] - t1x)) / stencil.width_hw as f32, (cy as f32 + (ys[i] - t1y)) / stencil.height_hw as f32]);
    let uv_depth = match tex2 {
        Some(t2) => std::array::from_fn(|i| [u(t2, xs[i], t2.offset_x), v(t2, ys[i], t2.offset_y)]),
        None => [[0.0; 2]; 4],
    };
    CutawayQuad { sprite, uv_stencil, uv_depth }
}

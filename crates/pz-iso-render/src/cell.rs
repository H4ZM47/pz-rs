//! `FBORenderCell`: which objects go into a chunk texture and the order
//! they are drawn in (`renderOneLevel`), floors (`renderFloor`), walls and
//! doors (`renderMinusFloor_DoorOrWall` with `IsoGridSquare.DoWallLighting*`)
//! and other objects (`renderMinusFloor_NotDoorOrWall`, `IsoObject.render`).
//!
//! Not yet ported: the per-frame translucent pass (windows, open doors,
//! trees, fading objects), cutaways and building collapse, occlusion,
//! snow, blood, puddles, seam fixes and overlay sprites.

use pz_render::{gl, DrawCmd};

use crate::chunk::ChunkTexture;
use crate::color::{self, ColorInfo};
use crate::sprite::{self, Dir, Modifier, SpriteCtx, SpriteDraw};
use crate::texd::{FloorShaper, WallPart, WallShaper};
use crate::world::{DepthMaps, ObjectKind, RenderObject, RenderSquare, TileType};
use crate::Camera;

/// `ObjectRenderLayer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    None,
    WorldInventoryObject,
    TranslucentFloor,
    Floor,
    Vegetation,
    MinusFloor,
    MinusFloorSE,
    Translucent,
    TranslucentSE,
}

/// `isObjectRenderLayer_TranslucentFloor` and friends, for a square with
/// no cutaways, no fading and grass not flattened.
pub fn object_layer(square: &RenderSquare, obj: &RenderObject) -> Layer {
    let sprite = obj.sprite.as_deref();
    if sprite.is_some_and(|s| s.flags.invisible) {
        return Layer::None;
    }
    if obj.kind == ObjectKind::WorldInventory {
        return Layer::WorldInventoryObject;
    }
    let f = obj.flags();
    if f.transparent_floor {
        return Layer::TranslucentFloor;
    }
    // Floor.
    let mut floor = !sprite.is_some_and(|s| !s.flags.solidfloor && s.render_layer != 1);
    if obj.kind == ObjectKind::Fire {
        floor = false;
    }
    if floor && !sprite.is_some_and(|s| s.sloped) {
        return Layer::Floor;
    }
    let translucent_kind = matches!(obj.kind, ObjectKind::Window { .. }) || (matches!(obj.kind, ObjectKind::Door { .. }) && f.door_trans);
    // MinusFloor.
    let depth2 = sprite.is_some_and(|s| s.depth_flags & 2 != 0);
    if !depth2 && obj.kind != ObjectKind::Tree {
        let mut ok = !sprite.is_some_and(|s| (s.flags.solidfloor || s.render_layer == 1) && !s.sloped);
        if obj.kind == ObjectKind::Fire || square.is_sprite_on_south_or_east_wall(obj) || translucent_kind {
            ok = false;
        }
        if ok {
            return Layer::MinusFloor;
        }
    }
    // MinusFloorSE.
    let mut se = !sprite.is_some_and(|s| s.flags.solidfloor || s.render_layer == 1);
    if obj.kind == ObjectKind::Fire || sprite.is_some_and(|s| s.is_grass_etc()) || !square.is_sprite_on_south_or_east_wall(obj) || translucent_kind {
        se = false;
    }
    if se {
        return if depth2 { Layer::TranslucentSE } else { Layer::MinusFloorSE };
    }
    if depth2 || obj.kind == ObjectKind::Fire || obj.kind == ObjectKind::Tree || translucent_kind {
        return Layer::Translucent;
    }
    Layer::None
}

/// `calculateObjectTargetAlpha` with no cutaways: open doors (not garage
/// doors) are drawn at 0.6, everything else opaque.
pub fn target_alpha(obj: &RenderObject, layer: Layer) -> f32 {
    if !matches!(layer, Layer::MinusFloor | Layer::MinusFloorSE | Layer::Translucent | Layer::TranslucentSE) {
        return 1.0;
    }
    if matches!(obj.kind, ObjectKind::Door { open: true }) && !obj.flags().garage_door {
        return 0.6;
    }
    1.0
}

/// One object's render layer after the target-alpha adjustment
/// (`calculateObjectRenderInfo`).
pub fn render_layer(square: &RenderSquare, obj: &RenderObject) -> Layer {
    let layer = object_layer(square, obj);
    if target_alpha(obj, layer) < 1.0 {
        return match layer {
            Layer::MinusFloor => Layer::Translucent,
            Layer::MinusFloorSE => Layer::TranslucentSE,
            l => l,
        };
    }
    layer
}

/// Records chunk textures and composites them.
pub struct ChunkRenderer<'a> {
    pub cam: Camera,
    pub depth_maps: &'a dyn DepthMaps,
}

/// The squares of one chunk level, `[x + y * 8]`.
pub type LevelSquares<'s> = [Option<&'s RenderSquare>; 64];

impl ChunkRenderer<'_> {
    /// Records chunk texture `tex`: `renderOneLevel` for each of its levels,
    /// between `FBORenderChunkStart(clear)` and `FBORenderChunkEnd`.
    pub fn record(&self, tex: ChunkTexture, levels: &[(i32, LevelSquares<'_>)], out: &mut Vec<DrawCmd>) {
        out.extend(tex.begin(true));
        for (level, squares) in levels {
            if *level < tex.min_level || *level > tex.top_level {
                continue;
            }
            self.render_one_level(tex, squares, out);
        }
        // renderOneChunk: depth writes off, depth func ALWAYS.
        out.push(DrawCmd::DepthMask(false));
        out.push(DrawCmd::DepthFunc(gl::ALWAYS));
        out.push(DrawCmd::ChunkEnd);
    }

    fn render_one_level(&self, tex: ChunkTexture, squares: &LevelSquares<'_>, out: &mut Vec<DrawCmd>) {
        let mut solid_floor = Vec::new();
        let mut minus_floor = Vec::new();
        for sq in squares.iter().flatten() {
            if sq.objects.is_empty() {
                continue;
            }
            let mut flags = 0;
            for obj in &sq.objects {
                match render_layer(sq, obj) {
                    Layer::Floor => flags |= 1,
                    Layer::Vegetation => flags |= 2,
                    Layer::MinusFloor | Layer::MinusFloorSE | Layer::WorldInventoryObject => flags |= 4,
                    _ => {}
                }
            }
            if flags & 1 != 0 {
                solid_floor.push(*sq);
            }
            if flags & 4 != 0 {
                minus_floor.push(*sq);
            }
        }
        let mut ctx = SpriteCtx { cam: &self.cam, chunk: tex, depth_maps: self.depth_maps, out };
        for sq in &solid_floor {
            for (i, obj) in sq.objects.iter().enumerate() {
                if render_layer(sq, obj) == Layer::Floor {
                    render_floor(&mut ctx, sq, i);
                }
            }
        }
        for sq in &minus_floor {
            for layer in [Layer::MinusFloor, Layer::MinusFloorSE] {
                for obj in &sq.objects {
                    if render_layer(sq, obj) == layer {
                        render_minus_floor(&mut ctx, sq, obj);
                    }
                }
            }
        }
    }

    /// `FBORenderChunkManager.endFrame`: every recorded chunk texture
    /// composited into the offscreen buffer under a screen-pixel projection,
    /// then the zoomed projection restored.
    pub fn composite(&self, textures: &[ChunkTexture], width: u32, height: u32, out: &mut Vec<DrawCmd>) {
        if textures.is_empty() {
            return;
        }
        out.push(DrawCmd::Ortho { left: 0.0, right: width as f32, bottom: height as f32, top: 0.0, width, height });
        out.push(DrawCmd::GlEnable(gl::BLEND));
        out.push(DrawCmd::DepthFunc(gl::ALWAYS));
        for t in textures {
            out.extend(t.composite(&self.cam));
        }
        let z = self.cam.zoom;
        out.push(DrawCmd::Ortho { left: 0.0, right: width as f32 * z, bottom: height as f32 * z, top: 0.0, width, height });
    }
}

fn set_blend(out: &mut Vec<DrawCmd>) {
    // IsoGridSquare.setBlendFunc with fboRenderChunk.
    out.push(DrawCmd::BlendFuncSeparate(gl::ONE, gl::ONE_MINUS_SRC_ALPHA, gl::ONE_MINUS_DST_ALPHA, gl::ONE));
}

/// `IsoObject.prepareToRender`'s colour (`stCol`) for an object with no
/// highlight.
fn prepare_color(obj: &RenderObject, col: ColorInfo) -> ColorInfo {
    let mut c = col;
    if let Some(cc) = obj.custom_color {
        c.r *= cc.r;
        c.g *= cc.g;
        c.b *= cc.b;
    }
    c
}

/// `FBORenderCell.renderFloor(object)` and `IsoObject.renderFloorTile`.
fn render_floor(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, index: usize) {
    let obj = &sq.objects[index];
    let Some(sprite) = obj.sprite.as_deref() else { return };
    ctx.out.push(DrawCmd::AlphaFunc(gl::GREATER, 0.0));
    ctx.out.push(DrawCmd::DepthMask(true));
    let v = &sq.vert_lights;
    let shaper = FloorShaper { col: [v[0], v[1], v[2], v[3]], tint: 0, shore: false, water_depth: [0.0; 4] };
    let main = if sprite.flags.diamond_floor || sprite.flags.water { Modifier::FloorDiamond(shaper) } else { Modifier::FloorDeDiamond(shaper) };
    set_blend(ctx.out);
    let col = prepare_color(obj, ColorInfo::WHITE);
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    sprite::render(
        ctx,
        &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: main },
    );
    render_attached(ctx, obj, x, y, z, Dir::Other, ColorInfo::WHITE, false, Modifier::FloorAttached(shaper));
}

/// `IsoObject.renderAttachedSprites`.
#[allow(clippy::too_many_arguments)]
fn render_attached(ctx: &mut SpriteCtx<'_>, obj: &RenderObject, x: f32, y: f32, z: f32, dir: Dir, col: ColorInfo, wall_lighting_pass: bool, modifier: Modifier) {
    if obj.attached.is_empty() {
        return;
    }
    let floor = obj.sprite.as_ref().is_some_and(|s| s.flags.solidfloor);
    ctx.out.push(DrawCmd::DepthMask(!floor));
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    for a in &obj.attached {
        let f = &a.sprite.flags;
        if wall_lighting_pass && f.no_wall_lighting {
            continue;
        }
        let mut m = modifier;
        if let Modifier::Wall(w) = modifier {
            match w.part {
                WallPart::W => {
                    if (f.attached_n || f.window_n) && a.sprite.texture.as_ref().is_some_and(|t| (t.width as i32) < 32 * pz_render::iso::TILE_SCALE) {
                        continue;
                    }
                    if f.attached_w || f.window_w {
                        m = Modifier::Wall(WallShaper { part: WallPart::Whole, ..w });
                    }
                }
                WallPart::N => {
                    if f.attached_w || f.window_w {
                        continue;
                    }
                    if f.attached_n || f.window_n {
                        m = Modifier::Wall(WallShaper { part: WallPart::Whole, ..w });
                    }
                }
                _ => {}
            }
        }
        sprite::render(
            ctx,
            &SpriteDraw { sprite: &a.sprite, obj: Some(obj), main: false, x, y, z, dir, offset_x: obj.offset_x, offset_y, col: ColorInfo { a: a.alpha, ..col }, alpha: a.alpha, flip: false, modifier: m },
        );
    }
    ctx.out.push(DrawCmd::DepthMask(true));
}

/// `FBORenderCell.renderMinusFloor(object)`.
fn render_minus_floor(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let t = obj.tile_type();
    let west = t == TileType::DoorFrW || t == TileType::DoorW || obj.sprite.as_ref().is_some_and(|s| s.cut_w);
    let north = t == TileType::DoorFrN || t == TileType::DoorN || obj.sprite.as_ref().is_some_and(|s| s.cut_n);
    ctx.out.push(DrawCmd::AlphaFunc(gl::GREATER, 0.0));
    set_blend(ctx.out);
    if obj.sprite.is_some() && (west || north) {
        render_door_or_wall(ctx, sq, obj, t);
    } else {
        render_not_door_or_wall(ctx, sq, obj);
    }
}

/// `renderMinusFloor_DoorOrWall` without cutaways.
fn render_door_or_wall(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, t: TileType) {
    let sprite = obj.sprite.as_deref().expect("sprite");
    ctx.out.push(DrawCmd::DepthMask(true));
    if obj.is_wall_se() {
        wall_lighting_w(ctx, sq, obj);
    } else if sprite.cut_w && sprite.cut_n {
        wall_lighting_nw(ctx, sq, obj);
    } else if t == TileType::DoorFrW || t == TileType::DoorW || sprite.cut_w {
        wall_lighting_w(ctx, sq, obj);
    } else if t == TileType::DoorFrN || t == TileType::DoorN || sprite.cut_n {
        wall_lighting_n(ctx, sq, obj);
    }
}

/// The ±4.5% contrast tweak `DoWallLightingN/W` apply between a wall's top
/// and bottom lights: `a` adjusted against `b`.
fn wall_contrast(a: u32, b: u32) -> u32 {
    let ch = |a: f32, b: f32| color::clamp(a * if a >= b { 1.045 } else { 0.955 }, 0.0, 1.0);
    color::to_abgr(ch(color::red(a), color::red(b)), ch(color::green(a), color::green(b)), ch(color::blue(a), color::blue(b)), 1.0)
}

/// `IsoGridSquare.DoWallLightingN`.
fn wall_lighting_n(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let v = &sq.vert_lights;
    let (colu, coll, colu2, coll2) = (v[0], v[1], v[4], v[5]);
    let colu_new = wall_contrast(colu, colu2);
    let coll2_new = wall_contrast(coll2, coll);
    let shaper = WallShaper { col: [colu2, coll2_new, coll, colu_new], tint: 0, part: WallPart::Whole };
    perform_draw_wall(ctx, sq, obj, Dir::N, shaper);
}

/// `IsoGridSquare.DoWallLightingW`.
fn wall_lighting_w(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let v = &sq.vert_lights;
    let (colu, coll, colu2, coll2) = (v[0], v[3], v[4], v[7]);
    let colu_new = wall_contrast(colu, colu2);
    let coll2_new = wall_contrast(coll2, coll);
    let shaper = WallShaper { col: [coll2_new, colu2, colu_new, coll], tint: 0, part: WallPart::Whole };
    let dir = if obj.is_wall_se() { Dir::SE } else { Dir::W };
    perform_draw_wall(ctx, sq, obj, dir, shaper);
}

/// `IsoGridSquare.DoWallLightingNW`: the corner piece drawn as its north
/// (right) and west (left) halves.
fn wall_lighting_nw(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let v = &sq.vert_lights;
    let (colu, coll, colr, colu2, coll2, colr2) = (v[0], v[3], v[1], v[4], v[7], v[5]);
    perform_draw_wall(ctx, sq, obj, Dir::NW, WallShaper { col: [colu2, colr2, colr, colu], tint: 0, part: WallPart::N });
    perform_draw_wall(ctx, sq, obj, Dir::NW, WallShaper { col: [coll2, colu2, colu, coll], tint: 0, part: WallPart::W });
}

/// `performDrawWall` and `IsoObject.renderWallTile`.
fn perform_draw_wall(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, dir: Dir, shaper: WallShaper) {
    let sprite = obj.sprite.as_deref().expect("sprite");
    let t = sprite.tile_type;
    let is_door = matches!(t, TileType::DoorFrN | TileType::DoorN | TileType::DoorFrW | TileType::DoorW);
    let _ = is_door;
    let no_wall_lighting = sprite.flags.no_wall_lighting;
    let light = sq.light_info;
    let col = if no_wall_lighting { light } else { ColorInfo::WHITE };
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    ctx.out.push(DrawCmd::GlEnable(gl::ALPHA_TEST));
    let col = prepare_color(obj, col);
    sprite::render(
        ctx,
        &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::Wall(shaper) },
    );
    render_attached(ctx, obj, x, y, z, dir, col, !no_wall_lighting, Modifier::Wall(shaper));
    // renderAttachedSpritesWithNoWallLighting: those attached sprites with
    // the square's light and no shaper.
    for a in obj.attached.iter().filter(|a| a.sprite.flags.no_wall_lighting) {
        let c = ColorInfo { r: light.r, g: light.g, b: light.b, a: a.alpha };
        sprite::render(
            ctx,
            &SpriteDraw { sprite: &a.sprite, obj: Some(obj), main: false, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col: c, alpha: a.alpha, flip: false, modifier: Modifier::None },
        );
    }
}

/// `renderMinusFloor_NotDoorOrWall` and `IsoObject.render`, without roof
/// seams.
fn render_not_door_or_wall(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    ctx.out.push(DrawCmd::DepthMask(true));
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    if let ObjectKind::Window { north } = obj.kind {
        let wall = sq.objects.iter().find(|o| o.sprite.as_ref().is_some_and(|s| if north { s.cut_n } else { s.cut_w }));
        if wall.is_some_and(|w| std::ptr::eq(w, obj)) {
            if north {
                wall_lighting_n(ctx, sq, obj);
            } else {
                wall_lighting_w(ctx, sq, obj);
            }
            return;
        }
    }
    let Some(sprite) = obj.sprite.as_deref() else { return };
    let mut col = prepare_color(obj, sq.light_info);
    if sprite.force_ambient {
        // RenderSettings ambient for the player; noon is full ambient.
        col.r = obj.tint[0];
        col.g = obj.tint[1];
        col.b = obj.tint[2];
    }
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    sprite::render(
        ctx,
        &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::None },
    );
    render_attached(ctx, obj, x, y, z, Dir::Other, sq.light_info, false, Modifier::None);
}

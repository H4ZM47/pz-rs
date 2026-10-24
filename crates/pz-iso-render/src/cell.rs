//! `FBORenderCell`: which objects go into a chunk texture and the order
//! they are drawn in (`renderOneLevel`), floors (`renderFloor`), walls and
//! doors (`renderMinusFloor_DoorOrWall` with `IsoGridSquare.DoWallLighting*`)
//! and other objects (`renderMinusFloor_NotDoorOrWall`, `IsoObject.render`).
//!
//! The per-frame translucent pass (`renderTranslucentFloorObjects`,
//! `renderTranslucentObjects`) draws windows, open doors and other
//! translucent objects over the composited chunks.
//!
//! With a [`CutawayView`], squares hidden by building collapse, orphan
//! structures or occlusion are skipped, objects take the cutaway target
//! alphas and cut doors and windows are lit as `DoWallLighting*` does.
//!
//! Trees are drawn in the translucent pass through `FBORenderTrees`
//! ([`crate::trees`]); floors and walls get `renderSeamFix2_*`
//! ([`crate::seam`]).
//!
//! With [`crate::weather::Surfaces`], low-quality puddles
//! ([`crate::puddles`]) are recorded after the other objects, and water
//! floors follow the water rules ([`crate::water`]).

use pz_render::{gl, DrawCmd};

use crate::alpha::{self, CutawayView, WallDraw, WallSide};
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
    let water = crate::water::ground_water(square).filter(|w| w.shader);
    // TranslucentFloor: transparent floors, and solid floors on a shore
    // (faded towards the water by `renderFloor`).
    if f.transparent_floor || (sprite.is_some_and(|s| s.flags.solidfloor) && water.is_some_and(|w| w.shore)) {
        return Layer::TranslucentFloor;
    }
    // Floor.
    let mut floor = !sprite.is_some_and(|s| !s.flags.solidfloor && s.render_layer != 1);
    if obj.kind == ObjectKind::Fire {
        floor = false;
    }
    if let Some(w) = water.filter(|w| w.is_valid()) {
        // Water tiles are drawn by the water shader except on shores; on
        // open water only the objects above the water object are floors.
        if sprite.is_some_and(|s| s.flags.water) {
            floor = w.shore;
        }
        if floor && !w.shore {
            let index = square.objects.iter().position(|o| std::ptr::eq(o, obj));
            floor = square.water_object().is_some_and(|wi| index.is_some_and(|i| wi < i));
        }
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
    /// This frame's cutaways; `None` renders every square uncut.
    pub cutaways: Option<CutawayView<'a>>,
    /// The screen size, for the tree camera.
    pub view_size: (u32, u32),
    /// This frame's trees (`FBORenderTrees`, with its depth stagger).
    pub trees: std::cell::RefCell<crate::trees::Trees>,
    /// Water, puddles, snow and blood; `None` draws none of them.
    pub surfaces: Option<crate::weather::Surfaces<'a>>,
}

impl<'a> ChunkRenderer<'a> {
    pub fn new(cam: Camera, depth_maps: &'a dyn DepthMaps, view_size: (u32, u32)) -> Self {
        ChunkRenderer { cam, depth_maps, view_size, trees: Default::default(), cutaways: None, surfaces: None }
    }

    /// This renderer with this frame's water, puddles, snow and blood.
    pub fn with_surfaces(self, surfaces: Option<crate::weather::Surfaces<'a>>) -> Self {
        ChunkRenderer { surfaces, ..self }
    }

    /// This renderer with this frame's cutaways.
    pub fn with_cutaways(self, cutaways: Option<CutawayView<'a>>) -> Self {
        ChunkRenderer { cutaways, ..self }
    }
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
            self.render_one_level(tex, *level, squares, out);
        }
        // renderOneChunk: depth writes off, depth func ALWAYS.
        out.push(DrawCmd::DepthMask(false));
        out.push(DrawCmd::DepthFunc(gl::ALWAYS));
        out.push(DrawCmd::ChunkEnd);
    }

    fn render_one_level(&self, tex: ChunkTexture, level: i32, squares: &LevelSquares<'_>, out: &mut Vec<DrawCmd>) {
        let mut solid_floor = Vec::new();
        let mut minus_floor = Vec::new();
        let cv = self.cutaways.as_ref();
        for sq in squares.iter().flatten() {
            if cv.is_some_and(|cv| !cv.is_visible(sq)) {
                continue;
            }
            if sq.objects.is_empty() {
                continue;
            }
            let mut flags = 0;
            for obj in &sq.objects {
                match alpha::render_layer(cv, sq, obj) {
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
        let mut ctx = SpriteCtx { cam: &self.cam, chunk: Some(tex), depth_maps: self.depth_maps, out };
        for sq in &solid_floor {
            for (i, obj) in sq.objects.iter().enumerate() {
                if alpha::render_layer(cv, sq, obj) == Layer::Floor {
                    render_floor(&mut ctx, sq, i);
                }
            }
        }
        for sq in &minus_floor {
            for layer in [Layer::MinusFloor, Layer::MinusFloorSE] {
                for obj in &sq.objects {
                    if alpha::render_layer(cv, sq, obj) == layer && !(layer == Layer::MinusFloor && alpha::skip_on_force_render_square(cv, sq, obj)) {
                        render_minus_floor(&mut ctx, sq, obj, cv);
                    }
                }
            }
        }
        if let Some(s) = self.surfaces.as_ref().filter(|s| s.settings.puddles_quality == 2) {
            let all: Vec<&RenderSquare> = squares.iter().flatten().copied().collect();
            crate::puddles::render_puddles_to_chunk(self, s, tex, &all, level, ctx.out);
        }
    }

    /// The translucent pass for level `z`: `renderTranslucentFloorObjects`
    /// then `renderTranslucentObjects` over `squares` (every on-screen
    /// square of that level, in any order).
    pub fn render_translucent(&self, squares: &[&RenderSquare], z: i32, out: &mut Vec<DrawCmd>) {
        self.render_translucent_floors(squares, z, out);
        self.render_translucent_objects(squares, z, out);
    }

    /// `renderTranslucentFloorObjects` for level `z`.
    pub fn render_translucent_floors(&self, squares: &[&RenderSquare], z: i32, out: &mut Vec<DrawCmd>) {
        let mut ctx = SpriteCtx { cam: &self.cam, chunk: None, depth_maps: self.depth_maps, out };
        let cv = self.cutaways.as_ref();
        let at_level: Vec<&RenderSquare> = squares.iter().copied().filter(|s| s.z == z && cv.is_none_or(|cv| cv.is_visible(s))).collect();
        let has = |sq: &RenderSquare, l: &[Layer]| sq.objects.iter().any(|o| l.contains(&alpha::render_layer(cv, sq, o)));
        // Translucent floors, in chunk-cache order.
        for sq in at_level.iter().filter(|s| has(s, &[Layer::TranslucentFloor])) {
            begin_translucent(&mut ctx);
            for obj in sq.objects.iter().filter(|o| alpha::render_layer(cv, sq, o) == Layer::TranslucentFloor) {
                render_translucent(&mut ctx, sq, obj, cv);
            }
        }
        ctx.out.push(DrawCmd::DepthMask(false));
        ctx.out.push(DrawCmd::DepthFunc(gl::ALWAYS));
    }

    /// `renderTranslucentObjects` for level `z`.
    pub fn render_translucent_objects(&self, squares: &[&RenderSquare], z: i32, out: &mut Vec<DrawCmd>) {
        let mut ctx = SpriteCtx { cam: &self.cam, chunk: None, depth_maps: self.depth_maps, out };
        let cv = self.cutaways.as_ref();
        let at_level: Vec<&RenderSquare> = squares.iter().copied().filter(|s| s.z == z && cv.is_none_or(|cv| cv.is_visible(s))).collect();
        let has = |sq: &RenderSquare, l: &[Layer]| sq.objects.iter().any(|o| l.contains(&alpha::render_layer(cv, sq, o)));
        let begin = begin_translucent;
        // Translucent objects, sorted by x + y * worldRight. Trees are
        // collected and drawn (FBORenderTrees) before the next other
        // object and at the end.
        let mut sorted: Vec<&RenderSquare> = at_level.iter().copied().filter(|s| has(s, &[Layer::Translucent, Layer::TranslucentSE])).collect();
        sorted.sort_by_key(|s| (s.y, s.x));
        let mut trees = self.trees.borrow_mut();
        let (vw, vh) = (self.view_size.0 as f32 * self.cam.zoom, self.view_size.1 as f32 * self.cam.zoom);
        for sq in sorted {
            begin(&mut ctx);
            for layer in [Layer::Translucent, Layer::TranslucentSE] {
                for obj in sq.objects.iter().filter(|o| alpha::render_layer(cv, sq, o) == layer) {
                    if obj.kind == ObjectKind::Tree {
                        add_tree(&mut trees, sq, obj);
                        continue;
                    }
                    if !trees.trees.is_empty() {
                        flush_trees(&mut trees, &self.cam, vw, vh, ctx.out);
                    }
                    render_translucent(&mut ctx, sq, obj, cv);
                }
            }
        }
        flush_trees(&mut trees, &self.cam, vw, vh, ctx.out);
        ctx.out.push(DrawCmd::DepthMask(false));
        ctx.out.push(DrawCmd::DepthFunc(gl::ALWAYS));
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

/// The depth state each translucent square starts with.
fn begin_translucent(ctx: &mut SpriteCtx<'_>) {
    ctx.out.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
    ctx.out.push(DrawCmd::DepthFunc(gl::LEQUAL));
    ctx.out.push(DrawCmd::DepthMask(false));
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
    // The shore fade: alpha 1 - water depth at each corner.
    let water = crate::water::ground_water(sq);
    let shore = water.is_some_and(|w| w.shore && w.shader);
    let d = water.map_or([0.0; 4], |w| [w.depth[0], w.depth[3], w.depth[2], w.depth[1]]);
    let shaper = FloorShaper { col: [v[0], v[1], v[2], v[3]], tint: 0, shore, water_depth: d };
    let main = if sprite.flags.diamond_floor || sprite.flags.water { Modifier::FloorDiamond(shaper) } else { Modifier::FloorDeDiamond(shaper) };
    set_blend(ctx.out);
    let col = prepare_color(obj, ColorInfo::WHITE);
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    if sq.floor() == Some(index) && ctx.chunk.is_some() {
        seam_fix2_floor(ctx, sq, obj, sprite, col, [v[0], v[1], v[2], v[3]]);
    }
    sprite::render(
        ctx,
        &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: main, seam: None },
    );
    render_overlay(ctx, sq, obj, x, y, z, ColorInfo::WHITE);
    render_attached(ctx, obj, x, y, z, Dir::Other, ColorInfo::WHITE, false, Modifier::FloorAttached(shaper));
}

/// `FBORenderCell.renderSeamFix2_Floor` (shore seams not ported).
fn seam_fix2_floor(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, sprite: &crate::world::SpriteInfo, col: ColorInfo, seam_col: [u32; 4]) {
    use crate::seam::SeamTile;
    let f = &sprite.flags;
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let oy = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32 - 3.0;
    let pick = |base: SeamTile, one: SeamTile, two: SeamTile| if f.floor_height_two_thirds { two } else if f.floor_height_one_third { one } else { base };
    let draw = |ctx: &mut SpriteCtx<'_>, tile: SeamTile, ox: f32| {
        sprite::render(
            ctx,
            &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir: Dir::Other, offset_x: ox, offset_y: oy, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::None, seam: Some((tile, seam_col)) },
        );
    };
    if sq.y.rem_euclid(8) == 7 && sq.seam.s_floor && !sq.seam.s_water {
        draw(ctx, pick(SeamTile::FloorSouth, SeamTile::FloorSouthOneThird, SeamTile::FloorSouthTwoThirds), obj.offset_x + 6.0);
    }
    if sq.x.rem_euclid(8) == 7 && sq.seam.e_floor && !sq.seam.e_water {
        draw(ctx, pick(SeamTile::FloorEast, SeamTile::FloorEastOneThird, SeamTile::FloorEastTwoThirds), obj.offset_x - 6.0);
    }
}

/// `FBORenderCell.renderSeamFix2_Wall`.
fn seam_fix2_wall(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, col: ColorInfo, shaper: WallShaper, seam_col: [u32; 4]) {
    use crate::seam::SeamTile;
    let Some(sprite) = obj.sprite.as_deref() else { return };
    let f = &sprite.flags;
    if f.hoppable_n || f.hoppable_w {
        return;
    }
    if (80..=82).contains(&sprite.tile_sheet_index) && sprite.tileset == "carpentry_02" {
        return;
    }
    if sprite.tileset == "walls_logs" {
        return;
    }
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let oy = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32 - 3.0;
    let draw = |ctx: &mut SpriteCtx<'_>, tile: SeamTile, dir: Dir, ox: f32| {
        sprite::render(
            ctx,
            &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir, offset_x: ox, offset_y: oy, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::Wall(shaper), seam: Some((tile, seam_col)) },
        );
    };
    let south = sq.y.rem_euclid(8) == 7 && sq.seam.s_wall_w;
    let east = sq.x.rem_euclid(8) == 7 && sq.seam.e_wall_n;
    if f.wall_nw && shaper.part == WallPart::W && south {
        draw(ctx, SeamTile::WallSouth, Dir::NW, obj.offset_x + 6.0);
    }
    if f.wall_nw && shaper.part == WallPart::N && east {
        draw(ctx, SeamTile::WallEast, Dir::NW, obj.offset_x - 6.0);
    }
    if (f.wall_w || f.window_w) && south {
        draw(ctx, SeamTile::WallSouth, Dir::W, obj.offset_x + 6.0);
    }
    if (f.wall_n || f.window_n) && east {
        draw(ctx, SeamTile::WallEast, Dir::N, obj.offset_x - 6.0);
    }
}

/// `IsoObject.renderOverlaySprites`: the overlay drawn over the object with
/// no shaper, in its own colour times the square's light when it has one.
fn render_overlay(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, x: f32, y: f32, z: f32, col: ColorInfo) {
    let Some(ov) = obj.overlay.as_ref() else { return };
    ctx.out.push(DrawCmd::DepthMask(true));
    let mut c = col;
    if let Some(oc) = ov.color {
        c = oc;
        c.r *= sq.light_info.r;
        c.g *= sq.light_info.g;
        c.b *= sq.light_info.b;
    }
    // The overlay's instance copies the object's alpha; a translucent
    // overlay colour scales it.
    let alpha = if c.a != 1.0 { obj.alpha * c.a } else { obj.alpha };
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    sprite::render(
        ctx,
        &SpriteDraw { sprite: &ov.sprite, obj: Some(obj), main: false, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col: c, alpha, flip: false, modifier: Modifier::None, seam: None },
    );
}

/// `IsoObject.renderAttachedAndOverlaySprites` in the per-frame pass:
/// overlay then attached sprites in the square's light, no shaper.
pub fn render_attached_and_overlay(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, col: ColorInfo) {
    if obj.sprite.as_ref().is_some_and(|s| s.flags.invisible) {
        return;
    }
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    render_overlay(ctx, sq, obj, x, y, z, col);
    render_attached(ctx, obj, x, y, z, Dir::Other, col, false, Modifier::None);
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
            &SpriteDraw { sprite: &a.sprite, obj: Some(obj), main: false, x, y, z, dir, offset_x: obj.offset_x, offset_y, col: ColorInfo { a: a.alpha, ..col }, alpha: a.alpha, flip: false, modifier: m, seam: None },
        );
    }
    ctx.out.push(DrawCmd::DepthMask(true));
}

/// `IsoTree.render` in the per-frame pass: the trunk/base texture and the
/// first attached sprite's (foliage) texture as one billboard, lit by the
/// square.
fn add_tree(trees: &mut crate::trees::Trees, sq: &RenderSquare, obj: &RenderObject) {
    let base = obj.sprite.as_ref().and_then(|s| s.texture.clone());
    let foliage = obj.attached.first().and_then(|a| a.sprite.texture.clone());
    let c = sq.light_info;
    trees.add(base, foliage, sq.x as f32, sq.y as f32, sq.z as f32, [c.r, c.g, c.b, c.a], false, 1.0, obj.alpha);
}

/// `SpriteRenderer.drawGeneric(FBORenderTrees.current)`, then the GL state
/// the following draws expect.
fn flush_trees(trees: &mut crate::trees::Trees, cam: &Camera, w: f32, h: f32, out: &mut Vec<DrawCmd>) {
    if trees.trees.is_empty() {
        return;
    }
    trees.render(cam, w, h, out);
    trees.trees.clear();
    out.push(DrawCmd::DepthMask(false));
    out.push(DrawCmd::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA));
}

/// `FBORenderCell.renderTranslucent(object)`: with the default blend, the
/// object at its target alpha.
fn render_translucent(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, cv: Option<&CutawayView<'_>>) {
    ctx.out.push(DrawCmd::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA));
    let f = obj.flags();
    if f.transparent_floor || (f.solidfloor && sq.water.is_some_and(|w| w.shore)) {
        let i = sq.objects.iter().position(|o| std::ptr::eq(o, obj)).expect("object");
        render_floor(ctx, sq, i);
        return;
    }
    let mut o = obj.clone();
    o.alpha = alpha::target_alpha(cv, sq, obj);
    let t = o.tile_type();
    let wallish = matches!(o.kind, ObjectKind::Door { .. }) || matches!(t, TileType::DoorFrW | TileType::DoorFrN) || o.sprite.as_ref().is_some_and(|s| s.cut_n || s.cut_w);
    ctx.out.push(DrawCmd::AlphaFunc(gl::GREATER, 0.0));
    if o.sprite.is_some() && wallish {
        render_door_or_wall(ctx, sq, &o, t, cv);
    } else {
        render_not_door_or_wall(ctx, sq, &o);
    }
}

/// `FBORenderCell.renderMinusFloor(object)`.
fn render_minus_floor(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, cv: Option<&CutawayView<'_>>) {
    let t = obj.tile_type();
    let west = t == TileType::DoorFrW || t == TileType::DoorW || obj.sprite.as_ref().is_some_and(|s| s.cut_w);
    let north = t == TileType::DoorFrN || t == TileType::DoorN || obj.sprite.as_ref().is_some_and(|s| s.cut_n);
    ctx.out.push(DrawCmd::AlphaFunc(gl::GREATER, 0.0));
    set_blend(ctx.out);
    if obj.sprite.is_some() && (west || north) {
        render_door_or_wall(ctx, sq, obj, t, cv);
    } else {
        render_not_door_or_wall(ctx, sq, obj);
    }
}

/// How a wall is drawn without cutaways: lit by the wall shaper unless
/// its sprite is `NoWallLighting`.
fn plain_wall(obj: &RenderObject) -> WallDraw {
    WallDraw::Normal { no_wall_lighting: obj.flags().no_wall_lighting }
}

/// `renderMinusFloor_DoorOrWall`.
fn render_door_or_wall(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, t: TileType, cv: Option<&CutawayView<'_>>) {
    let sprite = obj.sprite.as_deref().expect("sprite");
    let draw = |side: WallSide| match cv {
        Some(cv) => cv.wall_draw(sq, obj, side, &cv.wall_cutaway(sq, obj)),
        None => plain_wall(obj),
    };
    ctx.out.push(DrawCmd::DepthMask(true));
    // Cut-away north and west walls go through DoCutawayShader.
    if let Some(cv) = cv {
        let wc = cv.wall_cutaway(sq, obj);
        let side = if !obj.is_wall_se() && !(sprite.cut_w && sprite.cut_n) {
            if t == TileType::DoorFrW || t == TileType::DoorW || sprite.cut_w {
                Some(WallSide::W)
            } else if t == TileType::DoorFrN || t == TileType::DoorN || sprite.cut_n {
                Some(WallSide::N)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(side) = side {
            if cv.wall_draw(sq, obj, side, &wc) == WallDraw::Cutaway {
                do_cutaway_shader(ctx, sq, obj, if side == WallSide::N { CutDir::N } else { CutDir::W }, &wc, cv);
                return;
            }
        }
        let circle = wc.circle_stencil && sq.z == cv.state.player_z;
        if sprite.cut_w && sprite.cut_n && !obj.is_wall_se() {
            // DoWallLightingNW: calculateWallAlphaAndCircleStencilCorner.
            let f = &sprite.flags;
            let is_door_n = matches!(t, TileType::DoorFrN | TileType::DoorN);
            let mut c = circle && !is_door_n;
            if c && t == TileType::Wall && (f.transparent_n || f.transparent_w) && !f.exterior && !f.window_n && !f.window_w {
                c = false;
            }
            if c && wc.cutaway_self != 0 {
                do_cutaway_shader(ctx, sq, obj, CutDir::NW, &wc, cv);
                return;
            }
        }
        if obj.is_wall_se() {
            // DoWallLightingW for a south-east wall: cut when the west
            // neighbour's north wall or the north neighbour's west wall is.
            let f = &sprite.flags;
            let is_cut = wc.cutaway_w & 1 != 0 || wc.cutaway_n & 2 != 0;
            let mut c = circle;
            if c && t == TileType::Wall && f.transparent_w && !f.exterior && !f.window_w && sprite.tileset != "walls_burnt_01" {
                c = false;
            }
            if c && is_cut {
                do_cutaway_shader(ctx, sq, obj, CutDir::SE, &wc, cv);
                return;
            }
        }
    }
    if obj.is_wall_se() {
        wall_lighting_w(ctx, sq, obj, draw(WallSide::W));
    } else if sprite.cut_w && sprite.cut_n {
        // DoWallLightingNW's corner rules (calculateWallAlphaAndCircleStencilCorner)
        // are not ported; corners are drawn whole.
        wall_lighting_nw(ctx, sq, obj);
    } else if t == TileType::DoorFrW || t == TileType::DoorW || sprite.cut_w {
        wall_lighting_w(ctx, sq, obj, draw(WallSide::W));
    } else if t == TileType::DoorFrN || t == TileType::DoorN || sprite.cut_n {
        wall_lighting_n(ctx, sq, obj, draw(WallSide::N));
    }
}

/// `hasCutawayCapableWallNorth` / `West`.
fn has_cutaway_capable_wall(sq: Option<&RenderSquare>, north: bool) -> bool {
    let Some(sq) = sq else { return false };
    let any = |f: &dyn Fn(&crate::world::SpriteFlags) -> bool| sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| f(&s.flags)));
    if any(&|f| f.wall_se) {
        return false;
    }
    let wall = sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| if north { s.cut_n } else { s.cut_w }));
    let (window, wall_flag, door_wall) = if north {
        (any(&|f| f.window_n), any(&|f| f.wall_n || f.wall_nw), any(&|f| f.door_wall_n))
    } else {
        (any(&|f| f.window_w), any(&|f| f.wall_w || f.wall_nw), any(&|f| f.door_wall_w))
    };
    let wall_like = (wall || window) && (wall_flag || door_wall || window);
    wall_like || sq.objects.iter().any(|o| o.flags().garage_door && o.sprite.as_ref().is_some_and(|s| if north { s.cut_n } else { s.cut_w }))
}

/// The wall direction `DoCutawayShader` is called with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CutDir {
    N,
    W,
    NW,
    SE,
}

/// `IsoGridSquare.DoCutawayShader`: the north part (N, NW), the west part
/// (W, NW) or the south-east post. Attached sprites on cut walls
/// (`DoCutawayShaderAttached`) are not ported.
fn do_cutaway_shader(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, dir: CutDir, wc: &crate::alpha::WallCutaway, cv: &CutawayView<'_>) {
    let parts: &[(WallSide, sprite::WallTexRender)] = match dir {
        CutDir::N => &[(WallSide::N, sprite::WallTexRender::All)],
        CutDir::W => &[(WallSide::W, sprite::WallTexRender::All)],
        CutDir::NW => &[(WallSide::N, sprite::WallTexRender::RightOnly), (WallSide::W, sprite::WallTexRender::LeftOnly)],
        CutDir::SE => {
            do_cutaway_se(ctx, sq, obj, wc, cv);
            return;
        }
    };
    for &(side, tex_render) in parts {
        do_cutaway_part(ctx, sq, obj, side, tex_render, dir == CutDir::NW, wc, cv);
    }
}

/// `DoCutawayShader`'s south-east branch.
fn do_cutaway_se(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, wc: &crate::alpha::WallCutaway, cv: &CutawayView<'_>) {
    let sprite = obj.sprite.as_deref().expect("sprite");
    let Some(stencil) = ctx.depth_maps.cutaway_stencil(0) else { return };
    let Some(depth) = ctx.depth_maps.preset(crate::world::DepthPreset::SEWall) else { return };
    let at = |dx: i32, dy: i32| cv.world.square(sq.x + dx, sq.y + dy, sq.z);
    let x = if (wc.cutaway_w & 1 == 0 && has_cutaway_capable_wall(at(-1, 0), true)) || (wc.cutaway_n & 2 == 0 && has_cutaway_capable_wall(at(0, -1), false)) { 1212 } else { 1084 };
    let v = &sq.vert_lights;
    let shaper = WallShaper { col: [v[7], v[4], v[0], v[3]], tint: 0, part: WallPart::Whole };
    let col = prepare_color(obj, if sprite.flags.no_wall_lighting { sq.light_info } else { ColorInfo::WHITE });
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    ctx.out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    let d = SpriteDraw { sprite, obj: Some(obj), main: true, x: sq.x as f32, y: sq.y as f32, z: sq.z as f32, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::None, seam: None };
    let c = sprite::CutawayDraw { stencil, region: [x, 0, 6, 196], depth, tex_render: sprite::WallTexRender::All, discard_value: 1.0 };
    sprite::render_cutaway(ctx, &d, &shaper, &c);
    set_blend(ctx.out);
}

/// One wall face of `DoCutawayShader`.
#[allow(clippy::too_many_arguments)]
fn do_cutaway_part(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, side: WallSide, tex_render: sprite::WallTexRender, corner: bool, wc: &crate::alpha::WallCutaway, cv: &CutawayView<'_>) {
    let sprite = obj.sprite.as_deref().expect("sprite");
    let Some(mut stencil) = ctx.depth_maps.cutaway_stencil(0) else { return };
    let at = |dx: i32, dy: i32| cv.world.square(sq.x + dx, sq.y + dy, sq.z);
    let (sq_n, sq_s, sq_w, sq_e) = (at(0, -1), at(0, 1), at(-1, 0), at(1, 0));
    let hint = sprite.cutaway_hint.as_deref();
    // The lowest door/window object on the square gets the door/window
    // stencil rows.
    let lowest = |north: bool| {
        sq.objects.iter().position(|o| {
            let Some(s) = o.sprite.as_ref() else { return false };
            let window = matches!(o.kind, ObjectKind::Window { .. }) || if north { s.flags.window_n } else { s.flags.window_w };
            let door = if north { matches!(s.tile_type, TileType::DoorFrN | TileType::DoorN) || s.flags.door_wall_n } else { matches!(s.tile_type, TileType::DoorFrW | TileType::DoorW) || s.flags.door_wall_w };
            window || door
        })
    };
    let is_lowest = |north: bool| lowest(north).is_some_and(|i| std::ptr::eq(&sq.objects[i], obj));
    let window_row = |stencil: &mut Option<usize>, y: &mut i32, left: &str, right: &str, swap: bool| {
        *y = 226;
        match hint {
            Some(h) if h == left => *y = if swap { 452 } else { 678 },
            Some(h) if h == right => *y = if swap { 678 } else { 452 },
            Some(_) => {}
            None => {
                if let Some(n) = sprite.window_shape {
                    let (tex, row) = match n {
                        1..=9 => (1, n - 1),
                        10..=18 => (2, n - 10),
                        19..=21 => (3, n - 19),
                        _ => return,
                    };
                    *stencil = Some(tex);
                    *y = 226 * row;
                }
            }
        }
    };
    let mut stencil_index = None;
    let v = &sq.vert_lights;
    let (cut_x, mut cut_y, cols, preset) = match side {
        WallSide::N => {
            let mut x = 700;
            if wc.cutaway_self & 1 != 0 {
                if wc.cutaway_e & 1 == 0 && has_cutaway_capable_wall(sq_e, true) {
                    x = 444;
                }
                if (wc.cutaway_w & 1 == 0 && has_cutaway_capable_wall(sq_w, true)) || (wc.cutaway_n & 2 == 0 && has_cutaway_capable_wall(sq_n, false)) {
                    x = 956;
                }
            } else {
                x = if wc.cutaway_e & 1 == 0 { 828 } else { 956 };
            }
            (x, 0, [v[4], v[5], v[1], v[0]], if corner { crate::world::DepthPreset::NWWall } else { crate::world::DepthPreset::NWall })
        }
        WallSide::W => {
            let mut x = 512;
            if wc.cutaway_s & 2 != 0 {
                if (wc.cutaway_n & 2 == 0 && has_cutaway_capable_wall(sq_n, false)) || (wc.cutaway_w & 1 == 0 && has_cutaway_capable_wall(sq_w, true)) || (wc.cutaway_self & 1 == 0 && has_cutaway_capable_wall(Some(sq), true)) {
                    x = 768;
                }
            } else if wc.cutaway_self & 2 == 0 {
                x = 896;
            } else if wc.cutaway_n & 2 == 0 && has_cutaway_capable_wall(sq_n, false) {
                x = 768;
            } else if has_cutaway_capable_wall(sq_s, false) {
                x = 256;
            } else if wc.cutaway_w & 1 == 0 && has_cutaway_capable_wall(sq_w, true) {
                x = 768;
            }
            (x, 0, [v[7], v[4], v[0], v[3]], if corner { crate::world::DepthPreset::NWWall } else { crate::world::DepthPreset::WWall })
        }
    };
    let mut cut_x = cut_x;
    let north = side == WallSide::N;
    let (has_door, has_window) = if north { (wc.has_seen_door_n, wc.has_seen_window_n) } else { (wc.has_seen_door_w, wc.has_seen_window_w) };
    if is_lowest(north) {
        if has_door {
            cut_y = 904;
            match (hint, north) {
                (Some("DoubleDoorLeft"), true) | (Some("DoubleDoorRight"), false) => cut_y = 1130,
                (Some("DoubleDoorRight"), true) | (Some("DoubleDoorLeft"), false) => cut_y = 1356,
                (Some("GarageDoorLeft"), _) => {
                    cut_x = if north { 444 } else { 0 };
                    cut_y = 1808;
                }
                (Some("GarageDoorMiddle"), _) => {
                    cut_x = if north { 572 } else { 128 };
                    cut_y = 1808;
                }
                (Some("GarageDoorRight"), _) => {
                    cut_x = if north { 700 } else { 256 };
                    cut_y = 1808;
                }
                _ => {}
            }
        } else if has_window {
            window_row(&mut stencil_index, &mut cut_y, "DoubleWindowLeft", "DoubleWindowRight", !north);
        }
    }
    if let Some(i) = stencil_index {
        match ctx.depth_maps.cutaway_stencil(i) {
            Some(t) => stencil = t,
            None => return,
        }
    }
    let Some(depth) = ctx.depth_maps.preset(preset) else { return };
    let no_wall_lighting = sprite.flags.no_wall_lighting;
    let col = prepare_color(obj, if no_wall_lighting { sq.light_info } else { ColorInfo::WHITE });
    let shaper = WallShaper { col: cols, tint: 0, part: WallPart::Whole };
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    // The stencils have no mipmaps: drawn with (SRC_ALPHA, 1 - SRC_ALPHA).
    ctx.out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    let d = SpriteDraw { sprite, obj: Some(obj), main: true, x: sq.x as f32, y: sq.y as f32, z: sq.z as f32, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::None, seam: None };
    let c = sprite::CutawayDraw { stencil, region: [cut_x, cut_y, 66, 226], depth, tex_render, discard_value: 1.0 };
    sprite::render_cutaway(ctx, &d, &shaper, &c);
    set_blend(ctx.out);
}

/// The ±4.5% contrast tweak `DoWallLightingN/W` apply between a wall's top
/// and bottom lights: `a` adjusted against `b`.
fn wall_contrast(a: u32, b: u32) -> u32 {
    let ch = |a: f32, b: f32| color::clamp(a * if a >= b { 1.045 } else { 0.955 }, 0.0, 1.0);
    color::to_abgr(ch(color::red(a), color::red(b)), ch(color::green(a), color::green(b)), ch(color::blue(a), color::blue(b)), 1.0)
}

/// `IsoGridSquare.DoWallLightingN`.
fn wall_lighting_n(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, draw: WallDraw) {
    let v = &sq.vert_lights;
    let (colu, coll, colu2, coll2) = (v[0], v[1], v[4], v[5]);
    let colu_new = wall_contrast(colu, colu2);
    let coll2_new = wall_contrast(coll2, coll);
    let shaper = WallShaper { col: [colu2, coll2_new, coll, colu_new], tint: 0, part: WallPart::Whole };
    perform_draw_wall(ctx, sq, obj, Dir::N, shaper, [coll2_new, colu2, colu_new, coll], draw);
}

/// `IsoGridSquare.DoWallLightingW`.
fn wall_lighting_w(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, draw: WallDraw) {
    let v = &sq.vert_lights;
    let (colu, coll, colu2, coll2) = (v[0], v[3], v[4], v[7]);
    let colu_new = wall_contrast(colu, colu2);
    let coll2_new = wall_contrast(coll2, coll);
    let shaper = WallShaper { col: [coll2_new, colu2, colu_new, coll], tint: 0, part: WallPart::Whole };
    let dir = if obj.is_wall_se() { Dir::SE } else { Dir::W };
    perform_draw_wall(ctx, sq, obj, dir, shaper, [coll2_new, colu2, colu_new, coll], draw);
}

/// `IsoGridSquare.DoWallLightingNW`: the corner piece drawn as its north
/// (right) and west (left) halves.
fn wall_lighting_nw(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let v = &sq.vert_lights;
    let (colu, coll, colr, colu2, coll2, colr2) = (v[0], v[3], v[1], v[4], v[7], v[5]);
    perform_draw_wall(ctx, sq, obj, Dir::NW, WallShaper { col: [colu2, colr2, colr, colu], tint: 0, part: WallPart::N }, [colu2, colr2, colr, colu], plain_wall(obj));
    perform_draw_wall(ctx, sq, obj, Dir::NW, WallShaper { col: [coll2, colu2, colu, coll], tint: 0, part: WallPart::W }, [coll2, colu2, colu, coll], plain_wall(obj));
}

/// `performDrawWall` and `IsoObject.renderWallTile`. A
/// [`WallDraw::Cutaway`] wall is drawn whole until `DoCutawayShader` is
/// ported.
fn perform_draw_wall(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject, dir: Dir, shaper: WallShaper, seam_col: [u32; 4], draw: WallDraw) {
    let sprite = obj.sprite.as_deref().expect("sprite");
    let t = sprite.tile_type;
    let is_door = matches!(t, TileType::DoorFrN | TileType::DoorN | TileType::DoorFrW | TileType::DoorW);
    let _ = is_door;
    let no_wall_lighting = match draw {
        WallDraw::Normal { no_wall_lighting } => no_wall_lighting,
        WallDraw::Cutaway => sprite.flags.no_wall_lighting,
    };
    let light = sq.light_info;
    let col = if no_wall_lighting { light } else { ColorInfo::WHITE };
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let offset_y = obj.offset_y + obj.render_y_offset * pz_render::iso::TILE_SCALE as f32;
    ctx.out.push(DrawCmd::GlEnable(gl::ALPHA_TEST));
    let col = prepare_color(obj, col);
    if ctx.chunk.is_some() {
        seam_fix2_wall(ctx, sq, obj, col, shaper, seam_col);
    }
    sprite::render(
        ctx,
        &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::Wall(shaper), seam: None },
    );
    render_overlay(ctx, sq, obj, x, y, z, col);
    render_attached(ctx, obj, x, y, z, dir, col, !no_wall_lighting, Modifier::Wall(shaper));
    // renderAttachedSpritesWithNoWallLighting: those attached sprites with
    // the square's light and no shaper.
    for a in obj.attached.iter().filter(|a| a.sprite.flags.no_wall_lighting) {
        let c = ColorInfo { r: light.r, g: light.g, b: light.b, a: a.alpha };
        sprite::render(
            ctx,
            &SpriteDraw { sprite: &a.sprite, obj: Some(obj), main: false, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col: c, alpha: a.alpha, flip: false, modifier: Modifier::None, seam: None },
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
                wall_lighting_n(ctx, sq, obj, plain_wall(obj));
            } else {
                wall_lighting_w(ctx, sq, obj, plain_wall(obj));
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
        &SpriteDraw { sprite, obj: Some(obj), main: true, x, y, z, dir: Dir::Other, offset_x: obj.offset_x, offset_y, col, alpha: obj.alpha, flip: obj.flip, modifier: Modifier::None, seam: None },
    );
    render_overlay(ctx, sq, obj, x, y, z, sq.light_info);
    render_attached(ctx, obj, x, y, z, Dir::Other, sq.light_info, false, Modifier::None);
}

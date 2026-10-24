//! `IsoSprite` to [`SpriteInfo`]: the render flags from the sprite's
//! properties, its texture and depth texture.

use std::collections::HashMap;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_iso_render::world::StairPart;
use pz_iso_render::{SpriteFlags, SpriteInfo, TileType};
use pz_render::tile_depth::{
    TileDepthTextureAssignmentManager, TileDepthTextureManager, TileGeometryManager,
};
use pz_render::{Texture, TextureManager};
use pz_world::sprite::{IsoFlagType, SpriteIdx};
use pz_world::IsoSpriteManager;

/// Where sprite textures come from.
pub trait SpriteTextures {
    /// `Texture.getSharedTexture(name)`.
    fn texture(&mut self, name: &str) -> Option<Texture>;
    /// The sprite's own non-empty depth texture
    /// (`TileDepthTextureManager.getTextureForTile` via the assignments).
    fn depth_texture(&mut self, name: &str, has_tileset: bool) -> Option<Texture>;
    /// `TileGeometryManager` depth flags of the sprite.
    fn depth_flags(&self, name: &str) -> i32;
}

/// No textures: flags only (tests, headless analysis).
pub struct NoTextures;

impl SpriteTextures for NoTextures {
    fn texture(&mut self, _name: &str) -> Option<Texture> {
        None
    }

    fn depth_texture(&mut self, _name: &str, _has_tileset: bool) -> Option<Texture> {
        None
    }

    fn depth_flags(&self, _name: &str) -> i32 {
        0
    }
}

/// The installed game's textures.
pub struct GameTextures<'a> {
    pub fs: &'a GameFs,
    pub textures: &'a mut TextureManager,
    pub depth: &'a TileDepthTextureManager,
    pub assignments: &'a TileDepthTextureAssignmentManager,
    pub geometry: &'a TileGeometryManager,
}

impl SpriteTextures for GameTextures<'_> {
    fn texture(&mut self, name: &str) -> Option<Texture> {
        self.textures.shared_texture(self.fs, name)
    }

    fn depth_texture(&mut self, name: &str, has_tileset: bool) -> Option<Texture> {
        let d = self
            .depth
            .sprite_depth_texture(name, has_tileset, self.assignments)?;
        if d.is_empty() {
            return None;
        }
        d.uploaded_texture().cloned()
    }

    fn depth_flags(&self, name: &str) -> i32 {
        self.geometry.depth_flags(name)
    }
}

/// Flags looked up by name (not in pz-world's `Flags` table).
struct NamedFlags {
    water: Option<IsoFlagType>,
    unlit: Option<IsoFlagType>,
    attached_e: Option<IsoFlagType>,
    attached_s: Option<IsoFlagType>,
    door_wall_n: Option<IsoFlagType>,
    door_wall_w: Option<IsoFlagType>,
    wall_n: Option<IsoFlagType>,
    wall_w: Option<IsoFlagType>,
    wall_nw: Option<IsoFlagType>,
    force_render: Option<IsoFlagType>,
    attached_surface: Option<IsoFlagType>,
    attached_ceiling: Option<IsoFlagType>,
}

/// [`SpriteInfo`]s by sprite index, built on first use.
#[derive(Default)]
pub struct SpriteCache {
    cache: HashMap<SpriteIdx, Rc<SpriteInfo>>,
    named: Option<NamedFlags>,
}

impl SpriteCache {
    pub fn new() -> Self {
        SpriteCache::default()
    }

    /// Forget every built sprite (after texture packs change).
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// The render view of sprite `idx`.
    pub fn get(
        &mut self,
        sprites: &IsoSpriteManager,
        idx: SpriteIdx,
        tex: &mut dyn SpriteTextures,
    ) -> Rc<SpriteInfo> {
        if let Some(s) = self.cache.get(&idx) {
            return s.clone();
        }
        let t = &sprites.tables;
        let named = self.named.get_or_insert_with(|| {
            let n = |s: &str| t.flag_from_string(s);
            NamedFlags {
                water: n("water"),
                unlit: n("unlit"),
                attached_e: n("attachedE"),
                attached_s: n("attachedS"),
                door_wall_n: n("DoorWallN"),
                door_wall_w: n("DoorWallW"),
                wall_n: n("WallN"),
                wall_w: n("WallW"),
                wall_nw: n("WallNW"),
                force_render: n("forceRender"),
                attached_surface: n("attachedSurface"),
                attached_ceiling: n("attachedCeiling"),
            }
        });
        let info = Rc::new(build(sprites, idx, named, tex));
        self.cache.insert(idx, info.clone());
        info
    }
}

fn build(
    sprites: &IsoSpriteManager,
    idx: SpriteIdx,
    named: &NamedFlags,
    tex: &mut dyn SpriteTextures,
) -> SpriteInfo {
    let sp = &sprites.sprites[idx];
    let t = &sprites.tables;
    let f = &t.flag;
    let p = &sp.properties;
    let has = |fl: IsoFlagType| p.has(fl);
    let opt = |fl: Option<IsoFlagType>| fl.is_some_and(|fl| p.has(fl));
    let ty = &t.ty;
    // Types not in pz-world's table; `MAX` when the game has no such name.
    let named_type =
        |n: &str| Some(t.object_type_from_string(n)).filter(|&o| o != t.object_type_max());
    let tt = Some(sp.tile_type);
    let tile_type = match sp.tile_type {
        x if x == ty.wall => TileType::Wall,
        x if x == ty.doorFrN => TileType::DoorFrN,
        x if x == ty.doorN => TileType::DoorN,
        x if x == ty.doorFrW => TileType::DoorFrW,
        x if x == ty.doorW => TileType::DoorW,
        x if x == ty.tree => TileType::Tree,
        _ if tt == named_type("WestRoofB") => TileType::WestRoofB,
        _ if tt == named_type("WestRoofM") => TileType::WestRoofM,
        _ if tt == named_type("WestRoofT") => TileType::WestRoofT,
        _ => TileType::Other,
    };
    let flags = SpriteFlags {
        solidfloor: has(f.solidfloor),
        floor_overlay: has(f.FloorOverlay),
        diamond_floor: has(f.diamondFloor),
        water: opt(named.water),
        transparent_floor: has(f.transparentFloor),
        unlit: opt(named.unlit),
        invisible: has(f.invisible),
        no_wall_lighting: has(f.NoWallLighting),
        wall_se: has(f.WallSE),
        wall_overlay: has(f.WallOverlay),
        attached_n: has(f.attachedN),
        attached_w: has(f.attachedW),
        attached_e: opt(named.attached_e),
        attached_s: opt(named.attached_s),
        window_n: has(f.WindowN),
        window_w: has(f.WindowW),
        door_wall_n: opt(named.door_wall_n),
        door_wall_w: opt(named.door_wall_w),
        door_n: has(f.doorN),
        door_w: has(f.doorW),
        never_cutaway: has(f.NeverCutaway),
        door_trans: p.has_name(&sprites.alias, "doorTrans"),
        garage_door: p.has_name(&sprites.alias, "GarageDoor"),
        attached_surface: opt(named.attached_surface),
        attached_ceiling: opt(named.attached_ceiling),
        // PropertyContainer.isTableTop (surfaceFlags & 8).
        table_top: p.has_name(&sprites.alias, "IsTableTop"),
        wall_n: opt(named.wall_n),
        wall_w: opt(named.wall_w),
        wall_nw: opt(named.wall_nw),
        hoppable_n: has(f.HoppableN),
        hoppable_w: has(f.HoppableW),
        floor_height_one_third: has(f.FloorHeightOneThird),
        floor_height_two_thirds: has(f.FloorHeightTwoThirds),
        force_render: opt(named.force_render),
        transparent_n: has(f.transparentN),
        transparent_w: has(f.transparentW),
        exterior: has(f.exterior),
        vegitation: has(f.vegitation),
    };
    let name = sp.name().to_string();
    let grid = sp.sprite_grid.map(|g| &sprites.grids[g]);
    SpriteInfo {
        texture: tex.texture(&name),
        depth_texture: tex.depth_texture(&name, sp.tileset_name.is_some()),
        depth_flags: tex.depth_flags(&name),
        tileset: sp.tileset_name.clone().unwrap_or_default(),
        tile_sheet_index: sp.tile_sheet_index,
        cutaway_hint: p.get(&sprites.alias, "CutawayHint").map(str::to_string),
        window_shape: p
            .get(&sprites.alias, "WindowShape")
            .and_then(|v| v.parse().ok()),
        render_layer: sp.render_layer as i32,
        cut_n: sp.cut_n,
        cut_w: sp.cut_w,
        tile_type,
        stairs: match sp.tile_type {
            x if x == ty.stairsTN => Some(StairPart::TopN),
            x if x == ty.stairsMN => Some(StairPart::MiddleN),
            x if x == ty.stairsBN => Some(StairPart::BottomN),
            x if x == ty.stairsTW => Some(StairPart::TopW),
            x if x == ty.stairsMW => Some(StairPart::MiddleW),
            x if x == ty.stairsBW => Some(StairPart::BottomW),
            _ => None,
        },
        flags,
        is_bush: sp.is_bush,
        can_be_removed: sp.can_be_removed,
        attached_floor: sp.attached_floor,
        always_draw: sp.always_draw,
        force_ambient: sp.force_ambient,
        solid: sp.solid,
        solid_trans: sp.solid_trans,
        sloped: p.get(&sprites.alias, "SlopedSurfaceDirection").is_some(),
        tint_mod: [1.0; 3],
        grid_levels: grid.map_or(0, |g| g.levels),
        grid_pos_z: grid.and_then(|g| g.sprite_pos(idx)).map_or(0, |p| p.2),
        name,
        ..Default::default()
    }
}

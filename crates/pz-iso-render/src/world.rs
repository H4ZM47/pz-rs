//! What the renderer reads from the world: squares with their objects and
//! lighting, objects with their sprites, and the sprites' render
//! properties. The world crate fills these from its `IsoGridSquare`,
//! `IsoObject` and `IsoSprite` data.

use std::rc::Rc;

use pz_render::Texture;

use crate::color::ColorInfo;

/// `IsoObjectType` values the render path distinguishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TileType {
    Wall,
    DoorFrN,
    DoorN,
    DoorFrW,
    DoorW,
    WestRoofB,
    WestRoofM,
    WestRoofT,
    Tree,
    #[default]
    Other,
}

/// The `IsoFlagType` properties the render path tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpriteFlags {
    pub solidfloor: bool,
    pub floor_overlay: bool,
    pub diamond_floor: bool,
    pub water: bool,
    pub transparent_floor: bool,
    pub unlit: bool,
    pub invisible: bool,
    pub no_wall_lighting: bool,
    pub wall_se: bool,
    pub wall_overlay: bool,
    pub attached_n: bool,
    pub attached_w: bool,
    pub attached_e: bool,
    pub attached_s: bool,
    pub window_n: bool,
    pub window_w: bool,
    pub door_wall_n: bool,
    pub door_wall_w: bool,
    pub door_n: bool,
    pub door_w: bool,
    pub never_cutaway: bool,
    pub door_trans: bool,
    pub garage_door: bool,
    pub wall_n: bool,
    pub wall_w: bool,
    pub wall_nw: bool,
    pub hoppable_n: bool,
    pub hoppable_w: bool,
    pub floor_height_one_third: bool,
    pub floor_height_two_thirds: bool,
    /// `forceRender`: never collapsed by a cutaway.
    pub force_render: bool,
    /// `transparentN` / `transparentW`, `exterior`: read by the circle
    /// stencil test of cut walls.
    pub transparent_n: bool,
    pub transparent_w: bool,
    pub exterior: bool,
    pub attached_surface: bool,
    pub attached_ceiling: bool,
    /// `vegitation` (rain splashes skip such squares).
    pub vegitation: bool,
    /// `PropertyContainer.isTableTop()`.
    pub table_top: bool,
}

/// An `IsoSprite` as the renderer needs it.
#[derive(Clone, Debug)]
pub struct SpriteInfo {
    pub name: String,
    pub tileset: String,
    /// `tileSheetIndex`.
    pub tile_sheet_index: i32,
    /// The `CutawayHint` property (`DoubleDoorLeft`, `GarageDoorMiddle`, …).
    pub cutaway_hint: Option<String>,
    /// The `WindowShape` property, parsed.
    pub window_shape: Option<i32>,
    /// The texture of the frame drawn (`getTextureForFrame`).
    pub texture: Option<Texture>,
    pub soff_x: i32,
    pub soff_y: i32,
    pub render_layer: i32,
    pub depth_flags: i32,
    pub cut_n: bool,
    pub cut_w: bool,
    pub tile_type: TileType,
    pub flags: SpriteFlags,
    pub is_bush: bool,
    pub can_be_removed: bool,
    pub attached_floor: bool,
    pub always_draw: bool,
    pub force_ambient: bool,
    pub solid: bool,
    pub solid_trans: bool,
    /// `getProperties().getSlopedSurfaceDirection() != null`.
    pub sloped: bool,
    /// `tintMod`.
    pub tint_mod: [f32; 3],
    /// `isStairsNorth`/`isStairsWest`: which stair piece the sprite is.
    pub stairs: Option<StairPart>,
    /// The sprite's own non-empty depth texture (`depthTexture`).
    pub depth_texture: Option<Texture>,
    /// `getSpriteGrid().getLevels()` (0 without a sprite grid) and
    /// `getSpriteGridPosZ(sprite)`: multi-level objects.
    pub grid_levels: i32,
    pub grid_pos_z: i32,
    /// `hideForWaterRender` while the water shader is on: a shore blend
    /// the water shader draws instead (see
    /// [`crate::water::hides_for_water_render`]).
    pub hide_for_water_render: bool,
}

impl Default for SpriteInfo {
    /// A blank sprite; `tintMod` starts white, as `IsoSprite`'s does.
    fn default() -> Self {
        SpriteInfo {
            name: String::new(),
            tileset: String::new(),
            tile_sheet_index: 0,
            cutaway_hint: None,
            window_shape: None,
            texture: None,
            soff_x: 0,
            soff_y: 0,
            render_layer: 0,
            depth_flags: 0,
            cut_n: false,
            cut_w: false,
            tile_type: TileType::default(),
            flags: SpriteFlags::default(),
            is_bush: false,
            can_be_removed: false,
            attached_floor: false,
            always_draw: false,
            force_ambient: false,
            solid: false,
            solid_trans: false,
            sloped: false,
            tint_mod: [1.0; 3],
            stairs: None,
            depth_texture: None,
            grid_levels: 0,
            grid_pos_z: 0,
            hide_for_water_render: false,
        }
    }
}

impl SpriteInfo {
    pub fn is_grass_etc(&self) -> bool {
        self.is_bush || self.can_be_removed || self.attached_floor
    }
}

/// The `IsoObjectType` stair pieces (`stairsTN` … `stairsBW`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StairPart {
    TopN,
    MiddleN,
    BottomN,
    TopW,
    MiddleW,
    BottomW,
}

impl StairPart {
    pub fn north(self) -> bool {
        matches!(self, StairPart::TopN | StairPart::MiddleN | StairPart::BottomN)
    }
}

/// The `IsoObject` subclasses the render path treats specially.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjectKind {
    #[default]
    Plain,
    Window {
        north: bool,
    },
    Door {
        open: bool,
    },
    Tree,
    Fire,
    WorldInventory,
}

/// An object's overlay sprite (`overlaySprite`, e.g. grime or graffiti)
/// and its colour.
#[derive(Clone, Debug)]
pub struct Overlay {
    pub sprite: Rc<SpriteInfo>,
    /// `overlaySpriteColor`: when set, multiplied by the square's light.
    pub color: Option<ColorInfo>,
}

/// An `IsoSpriteInstance` attached to an object (`attachedAnimSprite`).
#[derive(Clone, Debug)]
pub struct AttachedSprite {
    pub sprite: Rc<SpriteInfo>,
    pub alpha: f32,
}

/// An `IsoObject` as the renderer needs it.
#[derive(Clone, Debug)]
pub struct RenderObject {
    /// The world object's id within its square (`IsoObject.id`), for the
    /// per-object render info ([`crate::render_info`]).
    pub id: u32,
    pub kind: ObjectKind,
    pub sprite: Option<Rc<SpriteInfo>>,
    pub attached: Vec<AttachedSprite>,
    /// `getAlpha(playerIndex)`.
    pub alpha: f32,
    pub render_y_offset: f32,
    pub custom_color: Option<ColorInfo>,
    /// `tintr/g/b`, used with `forceAmbient` sprites.
    pub tint: [f32; 3],
    /// `offsetX/offsetY`.
    pub offset_x: f32,
    pub offset_y: f32,
    /// Whether the main sprite instance is flipped.
    pub flip: bool,
    /// For a garage door segment: its orientation and the span of the
    /// whole door along its wall (`getGarageDoorPrev/Next` walked to the
    /// ends).
    pub garage_door: Option<GarageDoorSpan>,
    /// For a fascia: `getFasciaAttachedSquare()`.
    pub fascia_attached: Option<[i32; 3]>,
    /// `getContainerCount()`.
    pub container_count: u32,
    /// `overlaySprite` and `overlaySpriteColor`.
    pub overlay: Option<Overlay>,
    /// For an `IsoFire`: what drawing its animations reads.
    pub fire: Option<crate::fire::FireRender>,
    /// For an `IsoFireplace` or `IsoBarbecue`: which of `attached` are
    /// animated and how they are drawn.
    pub hearth: Option<crate::hearth::HearthRender>,
    /// `isHighlighted(playerIndex)` with `getHighlightColor(playerIndex)`
    /// and `isBlink()` for the viewing player (owner: the gameplay code
    /// calling `setHighlighted`, e.g. context menus, build and fishing
    /// UIs). Drawn by [`crate::highlight`].
    pub highlight: Option<ObjectHighlight>,
}

/// An object's highlight for the viewing player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectHighlight {
    /// `getHighlightColor(playerIndex)`.
    pub color: ColorInfo,
    /// What the highlight's alpha is multiplied by: `Core.blinkAlpha`
    /// while `isBlink()`, else 1.
    pub alpha_scale: f32,
}

/// A garage door segment's place in its door.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GarageDoorSpan {
    pub north: bool,
    /// The first and last segment's x (north doors) or y (west doors).
    pub first: i32,
    pub last: i32,
}

impl RenderObject {
    pub fn new(kind: ObjectKind, sprite: Option<Rc<SpriteInfo>>) -> Self {
        RenderObject {
            id: 0,
            kind,
            sprite,
            attached: Vec::new(),
            alpha: 1.0,
            render_y_offset: 0.0,
            custom_color: None,
            tint: [1.0; 3],
            offset_x: (32 * pz_render::iso::TILE_SCALE) as f32,
            offset_y: (96 * pz_render::iso::TILE_SCALE) as f32,
            flip: false,
            garage_door: None,
            fascia_attached: None,
            container_count: 0,
            overlay: None,
            fire: None,
            hearth: None,
            highlight: None,
        }
    }

    pub fn flags(&self) -> SpriteFlags {
        self.sprite.as_ref().map(|s| s.flags).unwrap_or_default()
    }

    pub fn tile_type(&self) -> TileType {
        self.sprite.as_ref().map_or(TileType::Other, |s| s.tile_type)
    }

    /// `isWallSE()`.
    pub fn is_wall_se(&self) -> bool {
        self.flags().wall_se
    }
}

/// An `IsoGridSquare` as the renderer needs it.
#[derive(Clone, Debug)]
pub struct RenderSquare {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub objects: Vec<RenderObject>,
    /// `getLightInfo(playerIndex)`: the square's light colour.
    pub light_info: ColorInfo,
    /// `getVertLight(0..8)`: floor corners 0-3, wall tops 4-7 (ABGR).
    pub vert_lights: [u32; 8],
    /// `getLightLevel(playerIndex)`.
    pub light_level: f32,
    /// What the seam fixes need to know about the south and east
    /// neighbours.
    pub seam: SeamNeighbours,
    /// Rooms, buildings and visibility for cutaways and occlusion.
    pub cut: SquareCutawayInfo,
    /// `getWater()`: the square's water geometry, if any.
    pub water: Option<crate::water::WaterGeometry>,
    /// `getPuddles()`, initialised (`IsoPuddlesGeometry.init`); `None`
    /// draws no puddles.
    pub puddles: Option<crate::puddles::PuddlesGeometry>,
    /// `hasFlies()`: `FliesSound` plays its corpse flies here (owner:
    /// pz-world's `IsoGridSquare::has_flies`, set by
    /// `dead_body::flies_sound`; pz-world-render copies it into the
    /// snapshot).
    pub flies: bool,
}

/// The neighbour tests `renderSeamFix2_Floor` and `renderSeamFix2_Wall`
/// make.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeamNeighbours {
    /// The south square exists and has a floor.
    pub s_floor: bool,
    /// The east square exists and has a floor.
    pub e_floor: bool,
    /// The south/east square has the `water` flag (water quality not 2).
    pub s_water: bool,
    pub e_water: bool,
    /// The south square has a west wall (`getWallType() & 4`), an
    /// east-west window frame or `DoorWallW`.
    pub s_wall_w: bool,
    /// The east square has a north wall (`getWallType() & 1`), a
    /// north-south window frame or `DoorWallN`.
    pub e_wall_n: bool,
}

/// A `BuildingDef` (and its `IsoBuilding`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuildingId(pub u32);

/// A room id (`getRoomID()`, never -1 here).
pub type RoomId = i64;

/// `getSlopedSurfaceDirection()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlopeDir {
    N,
    S,
    E,
    W,
}

/// What cutaways and occlusion read from an `IsoGridSquare` beyond its
/// objects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquareCutawayInfo {
    /// `getRoomID()`.
    pub room: Option<RoomId>,
    /// `getBuilding()`: the building of the square's room.
    pub building: Option<BuildingId>,
    /// `roofHideBuilding`.
    pub roof_hide_building: Option<BuildingId>,
    /// `associatedBuilding`.
    pub associated_building: Option<BuildingId>,
    /// `lighting[playerIndex]`: `bCanSee`, `bCouldSee`, `bSeen` and
    /// `darkMulti`.
    pub can_see: bool,
    pub could_see: bool,
    pub seen: bool,
    pub dark_multi: f32,
    /// `getPlayerBuiltFloor() != null`.
    pub player_built_floor: bool,
    /// `HasStairs()`.
    pub has_stairs: bool,
    /// `getStairPillar() != null`.
    pub stair_pillar: bool,
    /// `getSlopedSurfaceDirection()`.
    pub sloped: Option<SlopeDir>,
    /// `getVisionMatrix(0, 0, -1)`: vision down to the square below is
    /// blocked.
    pub vision_blocked_below: bool,
}

impl Default for SquareCutawayInfo {
    fn default() -> Self {
        SquareCutawayInfo {
            room: None,
            building: None,
            roof_hide_building: None,
            associated_building: None,
            can_see: true,
            could_see: true,
            seen: true,
            dark_multi: 1.0,
            player_built_floor: false,
            has_stairs: false,
            stair_pillar: false,
            sloped: None,
            vision_blocked_below: false,
        }
    }
}

impl RenderSquare {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        RenderSquare { x, y, z, objects: Vec::new(), light_info: ColorInfo::WHITE, vert_lights: [0xFFFF_FFFF; 8], light_level: 1.0, seam: SeamNeighbours::default(), cut: SquareCutawayInfo::default(), water: None, puddles: None, flies: false }
    }

    /// `getProperties().has(flag)`: any object's sprite has it.
    pub fn has(&self, f: impl Fn(&SpriteFlags) -> bool) -> bool {
        self.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| f(&s.flags)))
    }

    /// `getWaterObject()`: the first object with a `water` sprite.
    pub fn water_object(&self) -> Option<usize> {
        self.objects.iter().position(|o| o.flags().water)
    }

    /// `getFloor()`: the first solid-floor object.
    pub fn floor(&self) -> Option<usize> {
        self.objects.iter().position(|o| o.flags().solidfloor)
    }

    /// `isSpriteOnSouthOrEastWall`.
    pub fn is_sprite_on_south_or_east_wall(&self, obj: &RenderObject) -> bool {
        let f = obj.flags();
        f.attached_e || f.attached_s
    }
}

pub use pz_render::tile_depth::TileDepthPreset as DepthPreset;

/// The shared depth textures: presets and `TileDepthTextureManager`'s
/// default.
pub trait DepthMaps {
    fn preset(&self, preset: DepthPreset) -> Option<Texture>;
    /// `getDefaultDepthTexture()` when not empty.
    fn default_texture(&self) -> Option<Texture>;
    /// `TileSeamManager.getTexture(tile)`.
    fn seam_mask(&self, _tile: crate::seam::SeamTile) -> Option<Texture> {
        None
    }
    /// The cutaway stencils `DoCutawayShader` loads: 0 is
    /// `media/wallcutaways.png`, 1-3 `media/windowframe_cutaways.png`,
    /// `_2` and `_3` (flags 3, GL_NEAREST).
    fn cutaway_stencil(&self, _index: usize) -> Option<Texture> {
        None
    }
    /// `getBillboardDepthTexture()` when not empty (fire sprites).
    fn billboard(&self) -> Option<Texture> {
        None
    }
}

impl DepthMaps for pz_render::tile_depth::TileDepthTextureManager {
    fn preset(&self, preset: DepthPreset) -> Option<Texture> {
        Some(self.texture_for_preset(preset).clone())
    }

    fn default_texture(&self) -> Option<Texture> {
        let t = self.default_depth_texture()?;
        if t.is_empty() {
            return None;
        }
        t.uploaded_texture().cloned()
    }

    fn billboard(&self) -> Option<Texture> {
        let t = self.billboard_depth_texture()?;
        if t.is_empty() {
            return None;
        }
        t.uploaded_texture().cloned()
    }
}

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
}

/// An `IsoSprite` as the renderer needs it.
#[derive(Clone, Debug, Default)]
pub struct SpriteInfo {
    pub name: String,
    pub tileset: String,
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
    /// The sprite's own non-empty depth texture (`depthTexture`).
    pub depth_texture: Option<Texture>,
}

impl SpriteInfo {
    pub fn is_grass_etc(&self) -> bool {
        self.is_bush || self.can_be_removed || self.attached_floor
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

/// An `IsoSpriteInstance` attached to an object (`attachedAnimSprite`).
#[derive(Clone, Debug)]
pub struct AttachedSprite {
    pub sprite: Rc<SpriteInfo>,
    pub alpha: f32,
}

/// An `IsoObject` as the renderer needs it.
#[derive(Clone, Debug)]
pub struct RenderObject {
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
}

impl RenderObject {
    pub fn new(kind: ObjectKind, sprite: Option<Rc<SpriteInfo>>) -> Self {
        RenderObject {
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
}

impl RenderSquare {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        RenderSquare { x, y, z, objects: Vec::new(), light_info: ColorInfo::WHITE, vert_lights: [0xFFFF_FFFF; 8], light_level: 1.0 }
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

/// `TileDepthMapManager.TileDepthPreset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthPreset {
    Floor,
    WDoorFrame,
    NDoorFrame,
    WWall,
    NWall,
    NWWall,
    SEWall,
}

/// The shared depth textures: presets and `TileDepthTextureManager`'s
/// default.
pub trait DepthMaps {
    fn preset(&self, preset: DepthPreset) -> Option<Texture>;
    /// `getDefaultDepthTexture()` when not empty.
    fn default_texture(&self) -> Option<Texture>;
}

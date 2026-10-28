//! `TileDepthModifier`: which depth texture a sprite draw uses and the depth
//! texture coordinates (`tex1U0..3`, `tex1V0..3`) of its corners.

use std::rc::Rc;

use crate::{Texture, TextureManager};

use super::manager::{TileDepthPreset, TileDepthTextureManager};
use super::texture::TileDepthTexture;

/// The `IsoDirections` `setupWallDepth` distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallDirection {
    N,
    NW,
    W,
    SE,
    /// Any other direction: the floor preset.
    Other,
}

/// The sprite properties `setupWallDepth` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WallFlags {
    /// `IsoFlagType.DoorWallN`.
    pub door_wall_n: bool,
    /// `IsoFlagType.doorN`.
    pub door_n: bool,
    /// `IsoFlagType.DoorWallW`.
    pub door_wall_w: bool,
    /// `IsoFlagType.doorW`.
    pub door_w: bool,
    /// `IsoSprite.isWallSE()` (`IsoFlagType.WallSE`).
    pub wall_se: bool,
}

/// The preset `setupWallDepth` picks for a sprite without its own depth.
pub fn wall_preset(dir: WallDirection, f: WallFlags) -> TileDepthPreset {
    match dir {
        WallDirection::N if f.door_wall_n && !f.door_n => TileDepthPreset::NDoorFrame,
        WallDirection::N => TileDepthPreset::NWall,
        WallDirection::NW => TileDepthPreset::NWWall,
        WallDirection::W if f.wall_se => TileDepthPreset::SEWall,
        WallDirection::W if f.door_wall_w && !f.door_w => TileDepthPreset::WDoorFrame,
        WallDirection::W => TileDepthPreset::WWall,
        WallDirection::SE => TileDepthPreset::SEWall,
        WallDirection::Other => TileDepthPreset::Floor,
    }
}

/// The state `TileDepthModifier.instance` holds between a `setup*` call and
/// the draw it modifies.
#[derive(Clone, Debug)]
pub struct TileDepthModifier {
    /// `depthTexture` (`tex1` of the draw).
    pub depth: Texture,
    /// `sprite.soffX`, `sprite.soffY`.
    pub soff_x: i16,
    pub soff_y: i16,
    /// `setSpriteScale`; 1 after every `setup*`.
    pub scale_x: f32,
    pub scale_y: f32,
}

impl TileDepthModifier {
    fn with(depth: Texture, soff_x: i16, soff_y: i16) -> Self {
        TileDepthModifier { depth, soff_x, soff_y, scale_x: 1.0, scale_y: 1.0 }
    }

    /// `setupFloorDepth(sprite)`: the floor preset.
    pub fn floor(mgr: &TileDepthTextureManager, soff_x: i16, soff_y: i16) -> Self {
        Self::with(mgr.texture_for_preset(TileDepthPreset::Floor).clone(), soff_x, soff_y)
    }

    /// `setupWallDepth(sprite, dir)`: the sprite's own depth texture
    /// (`sprite_depth`, even when empty), else the wall preset for `dir`.
    pub fn wall(
        mgr: &mut TileDepthTextureManager,
        tm: &mut TextureManager,
        sprite_depth: Option<&Rc<TileDepthTexture>>,
        dir: WallDirection,
        flags: WallFlags,
        soff_x: i16,
        soff_y: i16,
    ) -> Self {
        let depth = match sprite_depth {
            Some(t) => mgr.texture_of(t, tm),
            None => mgr.texture_for_preset(wall_preset(dir, flags)).clone(),
        };
        Self::with(depth, soff_x, soff_y)
    }

    /// `setupTileDepthTexture(sprite, depthTexture)`.
    pub fn tile(mgr: &mut TileDepthTextureManager, tm: &mut TextureManager, depth: &TileDepthTexture, soff_x: i16, soff_y: i16) -> Self {
        Self::with(mgr.texture_of(depth, tm), soff_x, soff_y)
    }

    /// `setSpriteScale(scaleX, scaleY)`.
    pub fn set_sprite_scale(&mut self, scale_x: f32, scale_y: f32) {
        self.scale_x = scale_x;
        self.scale_y = scale_y;
    }

    /// `accept(textureDraw)` for a draw of `tex0`: the depth coordinates of
    /// the corners in [`crate::DrawCmd::WorldSprite`] order (top-left,
    /// top-right, bottom-right, bottom-left).
    pub fn uv1(&self, tex0: &Texture) -> [[f32; 2]; 4] {
        depth_uv1(tex0, &self.depth, self.soff_x, self.soff_y, self.scale_x, self.scale_y)
    }
}

/// `TileDepthModifier.accept`: the part of the sprite's frame both the
/// sprite texture (placed at `soff` and scaled) and the depth texture
/// cover, in depth texture coordinates. Corner order: top-left, top-right,
/// bottom-right, bottom-left.
pub fn depth_uv1(tex0: &Texture, tex1: &Texture, soff_x: i16, soff_y: i16, scale_x: f32, scale_y: f32) -> [[f32; 2]; 4] {
    let max = |a: f32, b: f32| if a > b { a } else { b };
    let min = |a: f32, b: f32| if a > b { b } else { a };
    let (sx, sy) = (soff_x as f32, soff_y as f32);
    let left = max(sx + tex0.offset_x * scale_x, tex1.offset_x);
    let right = min(sx + (tex0.offset_x + tex0.width as f32) * scale_x, tex1.offset_x + tex1.width as f32);
    let top = max(sy + tex0.offset_y * scale_y, tex1.offset_y);
    let bottom = min(sy + (tex0.offset_y + tex0.height as f32) * scale_y, tex1.offset_y + tex1.height as f32);
    let (w_hw, h_hw) = (tex1.width_hw as f32, tex1.height_hw as f32);
    let u = |x: f32| tex1.x_start + (x - tex1.offset_x) / w_hw;
    let v = |y: f32| tex1.y_start + (y - tex1.offset_y) / h_hw;
    [[u(left), v(top)], [u(right), v(top)], [u(right), v(bottom)], [u(left), v(bottom)]]
}

//! `ObjectRenderInfo`: what the chunk renderer recorded about each object
//! it drew, per player, for the object picker (`FBORenderObjectPicker`).
//!
//! The original keeps one `ObjectRenderInfo` per player on each
//! `IsoObject` (`getRenderInfo(playerIndex)`). `FBORenderCell.renderOneLevel`
//! resets it for every square of a chunk level it records
//! (`calculateObjectRenderInfo`: layer, target alpha, no cutaway, an empty
//! rectangle; `setNotRendered` for squares cut away or occluded),
//! `IsoGridSquare.DoCutawayShader` marks cut walls, `renderTranslucent`
//! refreshes the target alpha, and every main-sprite draw
//! (`IsoSprite.performRenderFrame`, outside the highlight pass) stores the
//! drawn rectangle: chunk-texture relative while recording, screen
//! relative (`sx - offX`) in the per-frame translucent pass. The values
//! persist until the next recording, as on the objects.
//!
//! Here the store is [`RenderInfos`], owned next to the chunk cache and
//! handed to the [`crate::ChunkRenderer`] with
//! [`crate::ChunkRenderer::with_render_infos`]. Objects are keyed by
//! square and index in the square's object list, and checked against the
//! object's id ([`crate::RenderObject::id`]) so an object list edited after
//! the last recording reads the default info, as a new `IsoObject` would.
//! The chunk textures' composite positions (`FBORenderChunk.renderX/Y`) are
//! recorded by [`crate::ChunkRenderer::composite`].

use std::cell::RefCell;
use std::collections::HashMap;

use crate::cell::Layer;

/// `IsoPlayer.MAX`.
pub const MAX_PLAYERS: usize = 4;

/// `ObjectRenderInfo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectRenderInfo {
    /// The object's id (stands in for the `object` back-reference).
    pub object: u32,
    pub layer: Layer,
    pub target_alpha: f32,
    pub cutaway_outline: bool,
    pub cutaway: bool,
    pub render_x: f32,
    pub render_y: f32,
    pub render_width: f32,
    pub render_height: f32,
    pub render_scale_x: f32,
    pub render_scale_y: f32,
    pub render_alpha: f32,
}

impl ObjectRenderInfo {
    /// A fresh `new ObjectRenderInfo(object)`.
    pub fn new(object: u32) -> Self {
        ObjectRenderInfo {
            object,
            layer: Layer::None,
            target_alpha: 1.0,
            cutaway_outline: false,
            cutaway: false,
            render_x: 0.0,
            render_y: 0.0,
            render_width: 0.0,
            render_height: 0.0,
            render_scale_x: 0.0,
            render_scale_y: 0.0,
            render_alpha: 0.0,
        }
    }
}

/// The object a sprite draw belongs to: its square, index in the square's
/// objects and id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectSlot {
    pub square: [i32; 3],
    pub index: usize,
    pub id: u32,
}

#[derive(Default)]
struct PlayerInfos {
    /// Each square's objects' infos, by object index.
    squares: HashMap<[i32; 3], Vec<ObjectRenderInfo>>,
    /// `FBORenderChunk.renderX/Y` of the chunk textures composited this
    /// frame, by chunk and the texture's first level.
    chunks: HashMap<(i32, i32, i32), (f32, f32)>,
}

/// Every object's render info for each player (owner: the frame, next to
/// the chunk texture cache; world-render thread).
#[derive(Default)]
pub struct RenderInfos {
    players: [RefCell<PlayerInfos>; MAX_PLAYERS],
}

impl RenderInfos {
    pub fn new() -> Self {
        Self::default()
    }

    /// `object.getRenderInfo(player)` for the object `id` at `index` on
    /// `square`: the default info when none was recorded for that object.
    pub fn get(&self, player: usize, square: [i32; 3], index: usize, id: u32) -> ObjectRenderInfo {
        let p = self.players[player].borrow();
        p.squares
            .get(&square)
            .and_then(|v| v.get(index))
            .filter(|i| i.object == id)
            .copied()
            .unwrap_or_else(|| ObjectRenderInfo::new(id))
    }

    /// Changes the info of `slot` (created fresh if it has none).
    pub fn update(&self, player: usize, slot: ObjectSlot, f: impl FnOnce(&mut ObjectRenderInfo)) {
        let mut p = self.players[player].borrow_mut();
        let v = p.squares.entry(slot.square).or_default();
        if v.len() <= slot.index {
            v.resize(slot.index + 1, ObjectRenderInfo::new(u32::MAX));
        }
        let info = &mut v[slot.index];
        if info.object != slot.id {
            *info = ObjectRenderInfo::new(slot.id);
        }
        f(info);
    }

    /// Drops the infos of the objects past the first `len` on `square`.
    pub(crate) fn truncate(&self, player: usize, square: [i32; 3], len: usize) {
        if let Some(v) = self.players[player].borrow_mut().squares.get_mut(&square) {
            v.truncate(len);
        }
    }

    /// `renderLevels.getFBOForLevel(z, zoom)`'s `renderX`, `renderY` for
    /// the chunk holding world square `(x, y, z)`: `None` when its texture
    /// was not composited this frame.
    pub fn chunk_render_xy(&self, player: usize, x: i32, y: i32, z: i32) -> Option<(f32, f32)> {
        let wx = pz_render::iso::fastfloor(x as f32 / 8.0);
        let wy = pz_render::iso::fastfloor(y as f32 / 8.0);
        let min = pz_render::iso::calculate_min_level(z);
        let p = self.players[player].borrow();
        p.chunks.get(&(wx, wy, min)).copied()
    }

    /// Forgets the composite positions before a frame's composite.
    pub(crate) fn clear_chunks(&self, player: usize) {
        self.players[player].borrow_mut().chunks.clear();
    }

    pub(crate) fn set_chunk(
        &self,
        player: usize,
        wx: i32,
        wy: i32,
        min_level: i32,
        xy: (f32, f32),
    ) {
        let min = pz_render::iso::calculate_min_level(min_level);
        self.players[player]
            .borrow_mut()
            .chunks
            .insert((wx, wy, min), xy);
    }

    /// Forgets every object of chunk `(wx, wy)` (the chunk was unloaded).
    pub fn forget_chunk(&self, wx: i32, wy: i32) {
        for p in &self.players {
            p.borrow_mut()
                .squares
                .retain(|[x, y, _], _| x.div_euclid(8) != wx || y.div_euclid(8) != wy);
        }
    }
}

/// A [`RenderInfos`] being written by one player's frame, and the object
/// whose sprites are being drawn.
#[derive(Clone, Copy)]
pub struct Recording<'a> {
    pub infos: &'a RenderInfos,
    pub player: usize,
    pub current: Option<ObjectSlot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_guard_stale_slots() {
        let infos = RenderInfos::new();
        let slot = ObjectSlot {
            square: [3, 4, 0],
            index: 2,
            id: 7,
        };
        infos.update(0, slot, |i| {
            i.layer = Layer::MinusFloor;
            i.render_width = 128.0;
        });
        assert_eq!(infos.get(0, [3, 4, 0], 2, 7).layer, Layer::MinusFloor);
        // Another object now at that index, another player, a missing slot.
        assert_eq!(infos.get(0, [3, 4, 0], 2, 8), ObjectRenderInfo::new(8));
        assert_eq!(infos.get(1, [3, 4, 0], 2, 7).layer, Layer::None);
        assert_eq!(infos.get(0, [3, 4, 0], 0, u32::MAX).render_width, 0.0);
        infos.forget_chunk(0, 0);
        assert_eq!(infos.get(0, [3, 4, 0], 2, 7), ObjectRenderInfo::new(7));
    }

    #[test]
    fn chunk_positions_by_level_pair() {
        let infos = RenderInfos::new();
        infos.set_chunk(0, 1, -1, 2, (10.0, 20.0));
        assert_eq!(infos.chunk_render_xy(0, 8, -1, 3), Some((10.0, 20.0)));
        assert_eq!(infos.chunk_render_xy(0, 8, -1, 1), None);
        infos.clear_chunks(0);
        assert_eq!(infos.chunk_render_xy(0, 8, -1, 3), None);
    }
}

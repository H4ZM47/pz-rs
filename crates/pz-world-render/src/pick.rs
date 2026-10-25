//! [`PickWorld`] over the world: `FBORenderObjectPicker.ContextPick`'s
//! reads of the squares under the mouse, each object's
//! `getRenderInfo(0)` (recorded by the chunk renderer into
//! [`RenderInfos`]), the sprites' click masks (`IsoObject.isMaskClicked`)
//! and the chunk textures' composite positions.
//!
//! What the picker needs from systems this crate does not own (the
//! player, corpses, square Lua objects, door adjacency, entity UIs) comes
//! through [`PickHooks`].

use std::rc::Rc;

use pz_iso_render::cell::Layer;
use pz_iso_render::{Camera, RenderInfos};
use pz_kahlua::Value;
use pz_lua_api::hud::picking::{
    MaskFn, PickKind, PickObject, PickPlayer, PickWorld, RenderInfo, RenderLayer, SpriteFlags,
};
use pz_render::Texture;
use pz_world::square::EdgeFacing;
use pz_world::{IsoGridSquare, IsoObject, ObjectKind};

use crate::render_world::{RenderWorld, WorldView};

/// The picker's inputs owned by other systems (world thread).
pub trait PickHooks {
    /// `IsoPlayer.players[0]`; `None` picks nothing.
    fn player0(&self) -> Option<PickPlayer>;
    /// The square's Lua object, for the tooltip.
    fn square_value(&self, _sq: &IsoGridSquare) -> Value {
        Value::Nil
    }
    /// `IsoDoor` / door `IsoThumpable`: `isAdjacentToSquare(player's
    /// current square)` for the object at `index` on `sq`.
    fn adjacent_to_player(&self, _sq: &IsoGridSquare, _index: usize) -> bool {
        false
    }
    /// `ClickObject.isInteractiveEntity` (the entity's `UiConfig`, through
    /// the multi-square master) for the object at `index` on `sq`.
    fn interactive_entity(&self, _sq: &IsoGridSquare, _index: usize) -> bool {
        false
    }
    /// The `IsoDeadBody`s among the square's `getStaticMovingObjects()`,
    /// with their `isMouseOver`.
    fn corpses(&self, _x: i32, _y: i32, _z: i32) -> Vec<PickObject> {
        Vec::new()
    }
    /// `DebugOptions.instance.fboRenderChunk.combinedFbo`.
    fn combined_fbo(&self) -> bool {
        false
    }
}

/// No player: nothing is picked.
pub struct NoPickHooks;

impl PickHooks for NoPickHooks {
    fn player0(&self) -> Option<PickPlayer> {
        None
    }
}

/// The world as player 0's picker sees it this frame.
pub struct WorldPick<'a, H: PickHooks> {
    pub view: WorldView<'a>,
    pub render: &'a RenderWorld,
    pub infos: &'a RenderInfos,
    /// Player 0's camera this frame (`Core.getZoom(0)`,
    /// `IsoCamera.getOffX(0)/getOffY(0)`).
    pub camera: Camera,
    pub hooks: H,
}

fn layer(l: Layer) -> RenderLayer {
    match l {
        Layer::None => RenderLayer::None,
        Layer::WorldInventoryObject => RenderLayer::WorldInventoryObject,
        Layer::TranslucentFloor => RenderLayer::TranslucentFloor,
        Layer::Floor => RenderLayer::Floor,
        Layer::Vegetation => RenderLayer::Vegetation,
        Layer::MinusFloor => RenderLayer::MinusFloor,
        Layer::MinusFloorSE => RenderLayer::MinusFloorSE,
        Layer::Translucent => RenderLayer::Translucent,
        Layer::TranslucentSE => RenderLayer::TranslucentSE,
    }
}

/// `IsoSprite.isMaskClicked(dir, x, y, flip)` on the sprite's current
/// texture.
fn texture_mask_clicked(t: &Texture, x: i32, y: i32, flip: bool) -> bool {
    let Some(mask) = t.mask.as_ref() else {
        return false;
    };
    let (mut x, y) = if flip {
        let x = (x as f32 - ((t.width_orig() - t.width as i32) as f32 - t.offset_x)) as i32;
        (x, (y as f32 - t.offset_y) as i32)
    } else {
        (
            (x as f32 - t.offset_x) as i32,
            (y as f32 - t.offset_y) as i32,
        )
    };
    if flip {
        x = t.width as i32 - x;
    }
    if x < 0 || y < 0 || x > t.width as i32 || y > t.height as i32 {
        return false;
    }
    mask.get(x, y)
}

impl<H: PickHooks> WorldPick<'_, H> {
    fn sprite_flags(&self, o: &IsoObject) -> Option<SpriteFlags> {
        let sp = &self.view.sprites.sprites[o.sprite?];
        let t = &self.view.sprites.tables;
        let p = &sp.properties;
        let named = |n: &str| t.flag_from_string(n).is_some_and(|f| p.has(f));
        let named_type =
            |n: &str| Some(t.object_type_from_string(n)).filter(|&o| o != t.object_type_max());
        let tt = Some(sp.tile_type);
        Some(SpriteFlags {
            cut_w: p.has(t.flag.cutW),
            cut_n: p.has(t.flag.cutN),
            water: named("water"),
            bed: named("bed"),
            water_piped: named("waterPiped"),
            solid_floor: p.has(t.flag.solidfloor),
            west_roof: ["WestRoofB", "WestRoofM", "WestRoofT"]
                .iter()
                .any(|n| tt == named_type(n)),
        })
    }

    fn kind(&self, sq: &IsoGridSquare, o: &IsoObject) -> PickKind {
        match &o.kind {
            ObjectKind::Door(_) => PickKind::Door,
            ObjectKind::Window(_) => PickKind::Window,
            ObjectKind::WindowFrame { north } => PickKind::WindowFrame {
                has_window: sq.get_window_facing(EdgeFacing::of_north(*north)).is_some(),
            },
            ObjectKind::Curtain(_) => PickKind::Curtain,
            ObjectKind::LightSwitch(_) => PickKind::LightSwitch,
            ObjectKind::Generator(_) => PickKind::Generator,
            ObjectKind::Radio(_) | ObjectKind::Television(_) => PickKind::WaveSignal,
            ObjectKind::Thumpable(t) => {
                let t_ = &self.view.sprites.tables;
                let window = o.sprite.is_some_and(|s| {
                    let p = &self.view.sprites.sprites[s].properties;
                    [t_.flag.WindowN, t_.flag.WindowW].iter().any(|&f| p.has(f))
                });
                PickKind::Thumpable {
                    door: t.is_door,
                    window,
                    square_window_on_edge: sq
                        .get_window_facing(EdgeFacing::of_north(t.north))
                        .is_some(),
                    // `getLightSource() != null`: a thumpable light is
                    // created for a positive radius.
                    light_source: t.light_source_radius > 0,
                }
            }
            _ => PickKind::Object,
        }
    }

    /// The click mask of the object at `index` on `sq` (its overlay
    /// sprite, then its sprite), from the render square's textures.
    fn mask(&self, sq: &IsoGridSquare, index: usize, id: u32) -> Option<MaskFn> {
        let ro = self
            .render
            .squares
            .get(&(sq.x, sq.y, sq.z))
            .and_then(|rs| rs.objects.get(index))
            .filter(|ro| ro.id == id)?;
        let main = ro.sprite.as_ref().and_then(|s| s.texture.clone());
        let overlay = ro.overlay.as_ref().and_then(|o| o.sprite.texture.clone());
        Some(Rc::new(move |x, y, flip| {
            if overlay
                .as_ref()
                .is_some_and(|t| texture_mask_clicked(t, x, y, flip))
            {
                return true;
            }
            main.as_ref()
                .is_some_and(|t| texture_mask_clicked(t, x, y, flip))
        }))
    }

    fn pick_object(&self, sq: &IsoGridSquare, index: usize, o: &IsoObject) -> PickObject {
        let ri = self.infos.get(0, [sq.x, sq.y, sq.z], index, o.id);
        let ro = self
            .render
            .squares
            .get(&(sq.x, sq.y, sq.z))
            .and_then(|rs| rs.objects.get(index))
            .filter(|ro| ro.id == o.id);
        let (offset_x, offset_y) = ro.map_or((64.0, 192.0), |ro| (ro.offset_x, ro.offset_y));
        PickObject {
            id: (u64::from(sq.x as u32) << 40)
                ^ (u64::from(sq.y as u32) << 16)
                ^ (u64::from(sq.z as u8) << 56)
                ^ u64::from(o.id),
            square: [sq.x, sq.y, sq.z],
            object_index: index as i32,
            kind: self.kind(sq, o),
            sprite: self.sprite_flags(o),
            has_container: o.container.is_some(),
            interactive_entity: self.hooks.interactive_entity(sq, index),
            adjacent_to_player: self.hooks.adjacent_to_player(sq, index),
            offset_x,
            offset_y,
            render: RenderInfo {
                layer: layer(ri.layer),
                target_alpha: ri.target_alpha,
                cutaway: ri.cutaway,
                render_x: ri.render_x,
                render_y: ri.render_y,
                render_width: ri.render_width,
                render_height: ri.render_height,
                render_scale_x: ri.render_scale_x,
                render_scale_y: ri.render_scale_y,
            },
            mask: if o.sprite.is_some() {
                self.mask(sq, index, o.id)
            } else {
                None
            },
            reroute_mask: None,
            have_special_tooltip: o.special_tooltip,
            square_value: self.hooks.square_value(sq),
            room: (sq.room_id != -1 && sq.room_def(self.view.meta).is_some()).then_some(sq.room_id),
            mouse_over: None,
        }
    }
}

impl<H: PickHooks> PickWorld for WorldPick<'_, H> {
    fn player0(&self) -> Option<PickPlayer> {
        self.hooks.player0()
    }

    fn zoom(&self) -> f32 {
        self.camera.zoom
    }

    fn camera_offset(&self) -> (f32, f32) {
        (self.camera.off_x as f32, self.camera.off_y as f32)
    }

    fn objects(&self, x: i32, y: i32, z: i32) -> Option<Vec<PickObject>> {
        let sq = self.view.cell.get_grid_square(x, y, z)?;
        Some(
            sq.objects
                .iter()
                .enumerate()
                .map(|(i, o)| self.pick_object(sq, i, o))
                .collect(),
        )
    }

    fn corpses(&self, x: i32, y: i32, z: i32) -> Vec<PickObject> {
        self.hooks.corpses(x, y, z)
    }

    fn chunk_render_xy(&self, square: [i32; 3]) -> Option<(f32, f32)> {
        self.infos
            .chunk_render_xy(0, square[0], square[1], square[2])
    }

    fn combined_fbo(&self) -> bool {
        self.hooks.combined_fbo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(w: u32, h: u32, ox: f32, oy: f32, mask: pz_render::Mask) -> Texture {
        let mut t = Texture::empty("t");
        t.width = w;
        t.height = h;
        t.width_orig = 128;
        t.height_orig = 256;
        t.offset_x = ox;
        t.offset_y = oy;
        t.mask = Some(Rc::new(mask));
        t
    }

    #[test]
    fn mask_offsets_bounds_and_flip() {
        // A 10x20 image at (30, 40) in its 128x256 frame, only its left
        // column set.
        let mut g = pz_render::image::BooleanGrid::new(10, 20);
        for y in 0..20 {
            g.set_value(0, y, true);
        }
        let t = tex(10, 20, 30.0, 40.0, pz_render::Mask::copy_of(&g, 10, 20));
        assert!(texture_mask_clicked(&t, 30, 45, false));
        assert!(!texture_mask_clicked(&t, 31, 45, false));
        assert!(!texture_mask_clicked(&t, 29, 45, false));
        assert!(!texture_mask_clicked(&t, 30, 39, false));
        // Flipped: x -> width - (x - ((128 - 10) - 30)).
        assert!(texture_mask_clicked(&t, 98, 45, true));
        assert!(!texture_mask_clicked(&t, 30, 45, true));
        let mut no_mask = t.clone();
        no_mask.mask = None;
        assert!(!texture_mask_clicked(&no_mask, 30, 45, false));
    }
}

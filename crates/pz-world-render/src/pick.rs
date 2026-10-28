//! What `FBORenderObjectPicker.ContextPick` reads from the world and the
//! renderer, as plain data: the objects of a square with their
//! `getRenderInfo(player)` (recorded by the chunk renderer into
//! [`RenderInfos`]), the class and sprite properties the picker tests,
//! the click masks (`IsoObject.isMaskClicked`), and the chunk textures'
//! composite positions.
//!
//! The HUD's `PickWorld` (pz-lua-api) is implemented over [`WorldPick`]
//! by the engine, which also supplies the inputs other systems own (the
//! player, corpses, square Lua objects, door adjacency, entity UIs).

use std::rc::Rc;

use pz_iso_render::{Camera, ObjectRenderInfo, RenderInfos};
use pz_render::Texture;
use pz_world::{IsoGridSquare, IsoObject, ObjectKind, SquareRef};

use crate::render_world::{RenderWorld, WorldView};
use crate::{HostResult, RuntimeHostError};
use pz_world::sprite::runtime::ObjectSprite;

/// The `IsoObject` subclass facts the picker's filters and score test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickClass {
    #[default]
    Object,
    Door,
    /// `IsoThumpable`: `isDoor()`, `isWindow()`, the square's window on
    /// its facing edge, `getLightSource() != null`.
    Thumpable {
        door: bool,
        window: bool,
        square_window_on_edge: bool,
        light_source: bool,
    },
    Window,
    /// `IsoWindowFrame` and `hasWindow()`.
    WindowFrame {
        has_window: bool,
    },
    Curtain,
    LightSwitch,
    Generator,
    /// `IsoWaveSignal` (radios, televisions).
    WaveSignal,
}

/// The sprite properties the picker tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickSpriteFlags {
    pub cut_w: bool,
    pub cut_n: bool,
    pub water: bool,
    pub bed: bool,
    pub water_piped: bool,
    pub solid_floor: bool,
    /// `getTileType()` is `WestRoofB`, `WestRoofM` or `WestRoofT`.
    pub west_roof: bool,
}

/// `isMaskClicked(x, y, flip)`.
pub type ClickMask = Rc<dyn Fn(i32, i32, bool) -> bool>;

/// One object on a square as the picker reads it.
#[derive(Clone)]
pub struct ObjectPick {
    pub square: [i32; 3],
    /// `getObjectIndex()`.
    pub index: usize,
    /// `IsoObject.id`.
    pub id: u32,
    pub class: PickClass,
    /// `sprite != null` and its properties.
    pub sprite: Option<PickSpriteFlags>,
    /// `getContainer() != null`.
    pub has_container: bool,
    /// `offsetX` / `offsetY`.
    pub offset_x: f32,
    pub offset_y: f32,
    /// `getRenderInfo(player)`.
    pub render: ObjectRenderInfo,
    pub mask: Option<ClickMask>,
    /// `haveSpecialTooltip()`.
    pub have_special_tooltip: bool,
    /// The square's room (`getRoom()`, `None` outside).
    pub room: Option<i64>,
}

/// The world and one player's recorded render state.
pub struct WorldPick<'a> {
    pub view: WorldView<'a>,
    pub render: &'a RenderWorld,
    pub infos: &'a RenderInfos,
    /// The player whose render info is read (0 for `ContextPick`).
    pub player: usize,
    /// That player's camera this frame (`Core.getZoom(player)`,
    /// `IsoCamera.getOffX(player)/getOffY(player)`).
    pub camera: Camera,
}

/// `IsoSprite.isMaskClicked(dir, x, y, flip)` on the sprite's current
/// texture.
pub fn texture_mask_clicked(t: &Texture, x: i32, y: i32, flip: bool) -> bool {
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

impl WorldPick<'_> {
    /// `Core.getZoom(player)`.
    pub fn zoom(&self) -> f32 {
        self.camera.zoom
    }

    /// `IsoCamera.getOffX(player)` / `getOffY(player)`.
    pub fn camera_offset(&self) -> (f32, f32) {
        (self.camera.off_x as f32, self.camera.off_y as f32)
    }

    /// The square at `x, y, z`.
    pub fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        self.view.cell.get_grid_square(x, y, z)
    }

    /// `square.getObjects()` of the square at `x, y, z`, in order; `None`
    /// without a square.
    pub fn objects(&self, x: i32, y: i32, z: i32) -> HostResult<Option<Vec<ObjectPick>>> {
        let Some(square) = self.square(x, y, z) else {
            return Ok(None);
        };
        let square = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("pick square"))?;
        let mut out = Vec::with_capacity(square.objects.len());
        for (index, object) in square.objects.iter().enumerate() {
            let object = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("pick object"))?;
            out.push(self.object(&square, index, &object)?);
        }
        Ok(Some(out))
    }

    /// `renderLevels.getFBOForLevel(z, zoom)`'s `renderX`, `renderY` for
    /// the chunk texture holding `square`.
    pub fn chunk_render_xy(&self, square: [i32; 3]) -> Option<(f32, f32)> {
        self.infos
            .chunk_render_xy(self.player, square[0], square[1], square[2])
    }

    fn sprite_flags(&self, o: &IsoObject) -> HostResult<Option<PickSpriteFlags>> {
        let Some(sp) = o.sprite.try_read(self.view.sprites)? else {
            return Ok(None);
        };
        let t = &self.view.sprites.tables;
        let p = &sp.properties;
        let named = |n: &str| t.flag_from_string(n).is_some_and(|f| p.has(f));
        let named_type =
            |n: &str| Some(t.object_type_from_string(n)).filter(|&o| o != t.object_type_max());
        let tt = Some(sp.tile_type);
        Ok(Some(PickSpriteFlags {
            cut_w: p.has(t.flag.cutW),
            cut_n: p.has(t.flag.cutN),
            water: named("water"),
            bed: named("bed"),
            water_piped: named("waterPiped"),
            solid_floor: p.has(t.flag.solidfloor),
            west_roof: ["WestRoofB", "WestRoofM", "WestRoofT"]
                .iter()
                .any(|n| tt == named_type(n)),
        }))
    }

    fn class(&self, sq: &IsoGridSquare, o: &IsoObject) -> HostResult<PickClass> {
        Ok(match &o.kind {
            ObjectKind::Door(_) => PickClass::Door,
            ObjectKind::Window(_) => PickClass::Window,
            ObjectKind::WindowFrame { north } => PickClass::WindowFrame {
                has_window: window_on_edge(sq, *north)?,
            },
            ObjectKind::Curtain(_) => PickClass::Curtain,
            ObjectKind::LightSwitch(_) => PickClass::LightSwitch,
            ObjectKind::Generator(_) => PickClass::Generator,
            ObjectKind::Radio(_) | ObjectKind::Television(_) => PickClass::WaveSignal,
            ObjectKind::Thumpable(t) => {
                let f = &self.view.sprites.tables.flag;
                let window = {
                    let sp = o.sprite.try_read(self.view.sprites)?;
                    sp.as_deref().is_some_and(|sp| {
                        sp.properties.has(f.WindowN) || sp.properties.has(f.WindowW)
                    })
                };
                PickClass::Thumpable {
                    door: t.is_door,
                    window,
                    square_window_on_edge: window_on_edge(sq, t.north)?,
                    // `getLightSource() != null`: a thumpable light is
                    // created for a positive radius.
                    light_source: t.light_source_radius > 0,
                }
            }
            _ => PickClass::Object,
        })
    }

    fn object(&self, sq: &IsoGridSquare, index: usize, o: &IsoObject) -> HostResult<ObjectPick> {
        let class = self.class(sq, o)?;
        let sprite = self.sprite_flags(o)?;
        if matches!(
            o.sprite,
            ObjectSprite::Native(_) | ObjectSprite::ArchiveUnmanaged(_)
        ) {
            return Err(RuntimeHostError::Unsupported {
                operation: "native sprite picking/mask host",
                kind: None,
            });
        }
        let square = [sq.x, sq.y, sq.z];
        let ro = self
            .render
            .squares
            .get(&(sq.x, sq.y, sq.z))
            .and_then(|rs| rs.objects.get(index))
            .filter(|ro| ro.id == o.id);
        let (offset_x, offset_y) = ro.map_or((64.0, 192.0), |ro| (ro.offset_x, ro.offset_y));
        // isMaskClicked: the overlay sprite, then the sprite.
        let mask = ro.filter(|_| o.sprite.is_some()).map(|ro| {
            let main = ro.sprite.as_ref().and_then(|s| s.texture.clone());
            let overlay = ro.overlay.as_ref().and_then(|o| o.sprite.texture.clone());
            Rc::new(move |x, y, flip| {
                overlay
                    .as_ref()
                    .is_some_and(|t| texture_mask_clicked(t, x, y, flip))
                    || main
                        .as_ref()
                        .is_some_and(|t| texture_mask_clicked(t, x, y, flip))
            }) as ClickMask
        });
        Ok(ObjectPick {
            square,
            index,
            id: o.id,
            class,
            sprite,
            has_container: o.container.is_some(),
            offset_x,
            offset_y,
            render: self.infos.get(self.player, square, index, o.id),
            mask,
            have_special_tooltip: o.special_tooltip,
            room: (sq.room_id != -1 && sq.room_def(self.view.meta).is_some()).then_some(sq.room_id),
        })
    }
}

fn window_on_edge(square: &IsoGridSquare, north: bool) -> HostResult<bool> {
    for id in &square.special_objects {
        for object in &square.objects {
            let object = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("pick window object"))?;
            if object.id == *id {
                if matches!(&object.kind,ObjectKind::Window(window) if window.north==north) {
                    return Ok(true);
                }
                break;
            }
        }
    }
    Ok(false)
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

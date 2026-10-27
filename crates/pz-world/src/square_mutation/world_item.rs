//! Items put on the floor: `IsoGridSquare.AddWorldInventoryItem(item, x,
//! y, height)` with the `IsoWorldInventoryObject` it makes,
//! `getApparentZ`, `IsoGameCharacter.getSolidFloorAt`, and
//! [`FloorDropEnv`], the [`pz_inventory::floor_drop::FloorWorld`] a
//! character's items drop into (`setWornItem`'s `forceDropTooHeavy`,
//! `dropHeavyItems`, `dropHandItems`).
//!
//! The world object retains the caller's shared item, including changes
//! to its rotations and the fluid container moving to the object. Single player: the
//! server's `transmitCompleteItemToClients` is not made.

use crate::object_slot::ObjectSlot;
use pz_inventory::floor_drop::{FloorWorld, HostResult, RuntimeHostError};
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{InventoryItem, ItemClass};

use super::{MutationEnv, ObjectRef, SquarePos};
use crate::chunk::{IsoCell, IsoGridSquare};
use crate::floor_item::FloorItem;
use crate::objects::{IsoObject, IsoWorldInventoryObject, ObjectKind, ObjectName};
use crate::save::object_io::UNREGISTERED_SPRITE_ID;
use crate::sprite::runtime::{self, ObjectSprite, SpriteManagerRef};
use crate::sprite::IsoSpriteManager;
use crate::square::IsoDirections;
use crate::world_objects::{WorldObjectEntry, WorldObjectsReadError};

/// `IsoChunk.invalidateRenderChunkLevel` flags of `AddWorldInventoryItem`.
pub const WORLD_ITEM_ADDED: u64 = 68;

/// What `AddWorldInventoryItem` made of the item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldItemPlaced {
    /// An `IsoWorldInventoryObject` on the square.
    Object(ObjectRef),
    /// A corpse item became the dead body `staticMovingObjects[index]`
    /// (`tryAddCorpseToWorld`).
    Corpse { pos: SquarePos, index: usize },
    /// A generator item became an `IsoGenerator` (`AddSpecialObject`).
    Generator(ObjectRef),
    /// An `AnimalInventoryItem` became an `IsoAnimal`
    /// ([`super::SquareMutationHooks::animal_dropped`]).
    Animal,
    /// No square, or a corpse or generator its hook could not build.
    NotPlaced,
}

#[derive(Debug)]
pub enum WorldItemPlacementError {
    Component(pz_entity::runtime_load::RuntimeHostError),
    Binding(crate::object_slot::ObjectBindingError),
}

impl std::fmt::Display for WorldItemPlacementError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "world item placement: {self:?}")
    }
}

impl std::error::Error for WorldItemPlacementError {}

/// Shared actual receiver prefix; native visual support must complete before
/// square/time/component/list/backlink publication.
fn construct_floor_prefix(
    cell: &mut IsoCell,
    rand: &mut dyn crate::GameRand,
    item: &ItemRef,
    x: f32,
    y: f32,
    height: f32,
    data_projection_age: Option<f64>,
) -> Result<ObjectSlot, WorldItemPlacementError> {
    {
        let mut i = item.try_borrow_mut().map_err(|_| {
            WorldItemPlacementError::Component(pz_entity::runtime_load::RuntimeHostError::Borrowed(
                "floor constructor item",
            ))
        })?;
        i.world_x_rotation = 0.0;
        i.world_y_rotation = 0.0;
        if i.world_z_rotation < 0.0 {
            i.world_z_rotation = rand.next_range(0, 360) as f32;
        }
    }
    shared::set_container(item, None);
    let xoff = if x == 0.0 {
        rand.next(1000) as f32 / 1000.0
    } else {
        x
    };
    let yoff = if y == 0.0 {
        rand.next(1000) as f32 / 1000.0
    } else {
        y
    };
    let mut obj = IsoObject::new(
        0,
        ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
            IsoWorldInventoryObject {
                xoff,
                yoff,
                zoff: height,
                offset_x: 0.0,
                offset_y: 0.0,
                item: FloorItem::retain(item),
                drop_time: data_projection_age.unwrap_or(-1.0),
                ignore_remove_sandbox: false,
                extended_placement: false,
            },
        )),
        None,
    );
    if data_projection_age.is_some() {
        obj.sprite = ObjectSprite::ArchiveUnmanaged(UNREGISTERED_SPRITE_ID);
    }
    obj.pending_floor_visual_tail = Some(if data_projection_age.is_some() {
        crate::objects::FloorVisualTail::ConstructorUpdateSprite
    } else {
        crate::objects::FloorVisualTail::ConstructorCreateSprite
    });
    obj.outline_on_mouseover = true;
    let slot = ObjectSlot::try_new(obj).map_err(|failure| {
        cell.retire_object(ObjectSlot::retain(&failure.object));
        WorldItemPlacementError::Binding(failure.error)
    })?;
    Ok(slot)
}

/// Required actual resources and later host phases. Every call is made
/// without item/object/sprite body guards; no default branch invents success.
pub trait NativeFloorHost {
    fn sprite_manager(&self) -> HostResult<SpriteManagerRef>;
    fn sprite_tables(&self) -> HostResult<crate::sprite::SpriteTables>;
    fn world_age_hours(&self) -> HostResult<f64>;
    fn tile_scale(&self) -> HostResult<i32>;
    fn world_texture_enabled(&self) -> HostResult<bool>;
    fn item_display_name(&self, item: &ItemRef) -> HostResult<String>;
    fn add_to_world(
        &mut self,
        object: &crate::IsoObjectRef,
        square: &crate::SquareRef,
        retired: &mut pz_entity::RetiredComponents,
    ) -> HostResult<()>;
    fn designation_item_on_ground(
        &mut self,
        object: &crate::IsoObjectRef,
        square: &crate::SquareRef,
    ) -> HostResult<()>;
}

struct UnavailableFloorHost;
impl NativeFloorHost for UnavailableFloorHost {
    fn sprite_manager(&self) -> HostResult<SpriteManagerRef> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual floor sprite manager",
            kind: None,
        })
    }
    fn sprite_tables(&self) -> HostResult<crate::sprite::SpriteTables> {
        Err(RuntimeHostError::Unsupported {
            operation: "installed sprite class metadata",
            kind: None,
        })
    }
    fn world_age_hours(&self) -> HostResult<f64> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual floor world age",
            kind: None,
        })
    }
    fn tile_scale(&self) -> HostResult<i32> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual floor tile scale",
            kind: None,
        })
    }
    fn world_texture_enabled(&self) -> HostResult<bool> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual world texture server GUI gate",
            kind: None,
        })
    }
    fn item_display_name(&self, _: &ItemRef) -> HostResult<String> {
        Err(RuntimeHostError::Unsupported {
            operation: "native floor item getName",
            kind: None,
        })
    }
    fn add_to_world(
        &mut self,
        _: &crate::IsoObjectRef,
        _: &crate::SquareRef,
        _: &mut pz_entity::RetiredComponents,
    ) -> HostResult<()> {
        Err(RuntimeHostError::Unsupported {
            operation: "native floor addToWorld",
            kind: None,
        })
    }
    fn designation_item_on_ground(
        &mut self,
        _: &crate::IsoObjectRef,
        _: &crate::SquareRef,
    ) -> HostResult<()> {
        Err(RuntimeHostError::Unsupported {
            operation: "native floor designation",
            kind: None,
        })
    }
}

fn native_null() -> RuntimeHostError {
    RuntimeHostError::NativeException(pz_entity::EntityError("NullPointerException".into()))
}

fn actual_sprite(object: &crate::IsoObjectRef) -> HostResult<runtime::RuntimeSpriteRef> {
    let object = object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor actual sprite pointer"))?;
    match &object.sprite {
        ObjectSprite::Native(sprite) => Ok(sprite.clone()),
        ObjectSprite::None => Err(native_null()),
        _ => Err(RuntimeHostError::Unsupported {
            operation: "floor visual requires actual native sprite",
            kind: None,
        }),
    }
}
fn item_texture_name(item: &ItemRef) -> HostResult<String> {
    let texture = pz_inventory::item_texture::get_tex_ref(item)?.ok_or_else(native_null)?;
    let name = texture
        .0
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor item texture name"))?
        .name
        .to_string();
    Ok(name)
}

/// The actual installed updateSprite phase order. Temporary owned values
/// capture only a single native read; no item or sprite guard spans providers.
pub fn update_floor_sprite(
    object: &crate::IsoObjectRef,
    host: &dyn NativeFloorHost,
    retired: &mut pz_entity::RetiredComponents,
) -> HostResult<()> {
    let item = crate::floor_object::runtime::item_ref(object)?.ok_or_else(native_null)?;
    let sprite = actual_sprite(object)?;
    runtime::set_tint_mod(&sprite, pz_inventory::item_texture::item_tint_ref(&item)?)?;
    if !host.world_texture_enabled()? {
        object
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor visual server continuation"))?
            .pending_floor_visual_tail = None;
        return Ok(());
    }
    // This getTex().getName is OUTSIDE the Java try region [96,116).
    let mut name = Some(item_texture_name(&item)?);
    if pz_inventory::item_texture::is_use_world_item_ref(&item)? {
        name = pz_inventory::item_texture::get_world_texture_ref(&item)?;
    }
    let selected = (|| {
        let name = name.as_deref().ok_or(RuntimeHostError::Unsupported {
            operation: "native nullable shared texture FileIO diagnostic",
            kind: None,
        })?;
        let texture = pz_render::get_shared_texture_ref(name).map_err(runtime::texture_error)?;
        if let Some(texture) = texture {
            retired.push_erased(texture);
        } else {
            return Ok(Some(item_texture_name(&item)?));
        }
        Ok(Some(name.to_string()))
    })();
    let name = match selected {
        Ok(name) => name,
        Err(RuntimeHostError::NativeException(_)) => {
            Some("media/inventory/world/WItem_Sack.png".to_string())
        }
        Err(error) => return Err(error),
    };
    let name = name.as_deref().ok_or(RuntimeHostError::Unsupported {
        operation: "native nullable shared texture FileIO diagnostic",
        kind: None,
    })?;
    let texture = runtime::load_single_texture(&sprite, name, retired)?;
    if pz_inventory::item_texture::get_script_item_ref(&item)?.is_none() {
        let def = sprite
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor def pointer"))?
            .def_slot()
            .cloned()
            .ok_or_else(native_null)?;
        let texture = texture.ok_or_else(native_null)?;
        let width = texture
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor texture widthOrig"))?
            .width_orig() as f32;
        let height = texture
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor texture heightOrig"))?
            .height_orig() as f32;
        let target_w = 16_i32.wrapping_mul(host.tile_scale()?) as f32;
        let target_h = 16_i32.wrapping_mul(host.tile_scale()?) as f32;
        def.try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor sprite scaleAspect"))?
            .scale_aspect(width, height, target_w, target_h);
    } else {
        let script =
            pz_inventory::item_texture::get_script_item_ref(&item)?.ok_or_else(native_null)?;
        let scale = pz_inventory::item_texture::script_scale_world_icon(&script)?;
        let scale = scale * (host.tile_scale()? as f32 / 2.0);
        runtime::set_scale(&sprite, scale, scale)?;
    }
    object
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor visual continuation"))?
        .pending_floor_visual_tail = None;
    Ok(())
}

/// `load` has a different null contract from updateSprite: initial getTex
/// null selects Sack, and a null LoadSingleTexture returns before scaling or
/// duplicate Fluid cleanup. It never invokes updateSprite as a substitute.
pub fn load_floor_sprite_visual(
    object: &crate::IsoObjectRef,
    host: &dyn NativeFloorHost,
    retired: &mut pz_entity::RetiredComponents,
) -> HostResult<bool> {
    if !host.world_texture_enabled()? {
        return Ok(false);
    }
    let item = crate::floor_object::runtime::item_ref(object)?.ok_or_else(native_null)?;
    let sprite = actual_sprite(object)?;
    let mut name = if pz_inventory::item_texture::get_tex_ref(&item)?.is_none() {
        Some("media/inventory/world/WItem_Sack.png".to_string())
    } else {
        Some(item_texture_name(&item)?)
    };
    if pz_inventory::item_texture::is_use_world_item_ref(&item)? {
        name = pz_inventory::item_texture::get_world_texture_ref(&item)?;
    }
    let selected = (|| {
        let name = name.as_deref().ok_or(RuntimeHostError::Unsupported {
            operation: "native nullable shared texture FileIO diagnostic",
            kind: None,
        })?;
        let texture = pz_render::get_shared_texture_ref(name).map_err(runtime::texture_error)?;
        if let Some(texture) = texture {
            retired.push_erased(texture);
        } else {
            return Ok(item_texture_name(&item)?);
        }
        Ok(name.to_string())
    })();
    let name = match selected {
        Ok(name) => name,
        Err(RuntimeHostError::NativeException(_)) => {
            "media/inventory/world/WItem_Sack.png".to_string()
        }
        Err(error) => return Err(error),
    };
    let Some(texture) = runtime::load_single_texture(&sprite, &name, retired)? else {
        return Ok(false);
    };
    if pz_inventory::item_texture::get_script_item_ref(&item)?.is_none() {
        let def = sprite
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("loaded floor def pointer"))?
            .def_slot()
            .cloned()
            .ok_or_else(native_null)?;
        let width = texture
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("loaded floor texture widthOrig"))?
            .width_orig() as f32;
        let height = texture
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("loaded floor texture heightOrig"))?
            .height_orig() as f32;
        let target_w = 16_i32.wrapping_mul(host.tile_scale()?) as f32;
        let target_h = 16_i32.wrapping_mul(host.tile_scale()?) as f32;
        def.try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("loaded floor scaleAspect"))?
            .scale_aspect(width, height, target_w, target_h);
    } else {
        let script =
            pz_inventory::item_texture::get_script_item_ref(&item)?.ok_or_else(native_null)?;
        let scale = pz_inventory::item_texture::script_scale_world_icon(&script)?
            * (host.tile_scale()? as f32 / 2.0);
        runtime::set_scale(&sprite, scale, scale)?;
    }
    object
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("loaded floor visual continuation"))?
        .pending_floor_visual_tail = None;
    Ok(true)
}

fn finish_native_supplied(
    square: &crate::SquareRef,
    slot: ObjectSlot,
    item: &ItemRef,
    host: &mut dyn NativeFloorHost,
    retired: &mut pz_entity::RetiredComponents,
) -> HostResult<WorldItemPlaced> {
    let object = slot.ref_handle();
    let name = host.item_display_name(item)?;
    object
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor native name destination"))?
        .name = Some(ObjectName::Str(name));
    let key = {
        let item = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor native key"))?;
        item.get_key_id()
    };
    {
        object
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor native key destination"))?
            .key_id = key;
    }
    let (id, pos) = {
        let mut square = square
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor retained main list"))?;
        let id = square.new_object_id();
        object
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor native ID assignment"))?
            .id = id;
        square.objects.push(slot);
        let list = square.world_objects.clone();
        let pos = (square.x, square.y, square.z);
        drop(square);
        list.try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor retained worldObjects list"))?
            .push_object(&object);
        (id, pos)
    };
    crate::floor_object::runtime::publish_item_backlink(&object, item, retired)?;
    crate::object_slot::observe_object(&object, "zombie.iso.objects.IsoWorldInventoryObject");
    host.add_to_world(&object, square, retired)?;
    host.designation_item_on_ground(&object, square)?;
    Ok(WorldItemPlaced::Object(ObjectRef { pos, id }))
}

/// Only the render operation needed before the strict native constructor.
/// The host borrows its actual renderer exactly when PC20 requests it.
pub trait NativeFloorRender {
    fn invalidate_render_chunk_level(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        flags: u64,
    ) -> HostResult<()>;
}

pub struct NativeFloorPlacement<'a> {
    pub cell: &'a mut IsoCell,
    pub rand: &'a mut dyn crate::GameRand,
    pub render: &'a mut dyn NativeFloorRender,
}
impl NativeFloorPlacement<'_> {
    pub fn place(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        let square = self
            .cell
            .get_grid_square(pos.0, pos.1, pos.2)
            .ok_or_else(|| {
                WorldItemPlacementError::Component(RuntimeHostError::InvalidHostState(
                    "native floor placement requires its actual square",
                ))
            })?;
        self.place_retained(&square, item, x, y, height, &mut UnavailableFloorHost)
    }

    pub fn place_retained(
        &mut self,
        square: &crate::SquareRef,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        host: &mut dyn NativeFloorHost,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        let pos = {
            let body = square.try_borrow().map_err(|_| {
                WorldItemPlacementError::Component(RuntimeHostError::Borrowed(
                    "native retained floor square",
                ))
            })?;
            (body.x, body.y, body.z)
        };
        let corpse = {
            let item = item.try_borrow().map_err(|_| {
                WorldItemPlacementError::Component(RuntimeHostError::Borrowed(
                    "floor placement item",
                ))
            })?;
            item.is_human_corpse() || item.is_animal_corpse()
        };
        if corpse {
            return Err(WorldItemPlacementError::Component(
                RuntimeHostError::Unsupported {
                    operation: "native corpse floor placement",
                    kind: None,
                },
            ));
        }
        // Supplied overload PC20, after corpse attempt and BEFORE generator,
        // animal and ordinary constructor. No unrelated manager is borrowed.
        self.render
            .invalidate_render_chunk_level(
                pos.0.div_euclid(8),
                pos.1.div_euclid(8),
                pos.2,
                WORLD_ITEM_ADDED,
            )
            .map_err(WorldItemPlacementError::Component)?;
        {
            let item = item.try_borrow().map_err(|_| {
                WorldItemPlacementError::Component(RuntimeHostError::Borrowed(
                    "floor placement item",
                ))
            })?;
            if item.get_full_type().contains(".Generator")
                || item.has_tag("base:generator") && item.template.world_object_sprite.is_some()
            {
                return Err(WorldItemPlacementError::Component(
                    RuntimeHostError::Unsupported {
                        operation: "native generator floor placement",
                        kind: None,
                    },
                ));
            }
            if item.template.class == ItemClass::Animal {
                return Err(WorldItemPlacementError::Component(
                    RuntimeHostError::Unsupported {
                        operation: "native animal floor placement",
                        kind: None,
                    },
                ));
            }
        }
        let slot = construct_floor_prefix(self.cell, self.rand, item, x, y, height, None)?;
        let object = slot.ref_handle();
        self.cell.retire_object(ObjectSlot::retain(&object));
        let mut retired = pz_entity::RetiredComponents::new();
        let result = (|| {
            let manager = host.sprite_manager()?;
            let sprite =
                runtime::create_runtime_sprite(&manager, &host.sprite_tables()?, &mut retired)?;
            {
                let mut object = object
                    .try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("floor sprite publication"))?;
                let old = std::mem::replace(&mut object.sprite, ObjectSprite::Native(sprite));
                retired.push_payload(Box::new(old));
                object.pending_floor_visual_tail =
                    Some(crate::objects::FloorVisualTail::ConstructorUpdateSprite);
            }
            crate::object_slot::observe_object(
                &object,
                "zombie.iso.objects.IsoWorldInventoryObject",
            );
            update_floor_sprite(&object, host, &mut retired)?;
            object
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("floor actual square publication"))?
                .replace_square(Some(square), &mut retired);
            crate::object_slot::observe_object(
                &object,
                "zombie.iso.objects.IsoWorldInventoryObject",
            );
            let floor = crate::floor_object::runtime::floor_ref(&object)?;
            {
                let mut floor = floor
                    .try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("floor constructor offsets"))?;
                floor.offset_x = 0.0;
                floor.offset_y = 0.0;
            }
            let age = host.world_age_hours()?;
            floor
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("floor drop time"))?
                .drop_time = age;
            crate::floor_object::runtime::fluid_from_item_before_world(
                item,
                &object,
                &mut retired,
            )?;
            let placed = finish_native_supplied(square, slot, item, host, &mut retired)?;
            let render_square = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("floor render square pointer"))?
                .square_slot()
                .cloned();
            if let Some(render_square) = render_square {
                let position = {
                    let square = render_square
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("floor render square"))?;
                    (square.x, square.y, square.z)
                };
                if matches!(
                    (position.0.rem_euclid(8), position.1.rem_euclid(8)),
                    (0, 7) | (7, 0)
                ) {
                    return Err(RuntimeHostError::Unsupported {
                        operation: "actual retained adjacent floor render square",
                        kind: None,
                    });
                }
                self.render.invalidate_render_chunk_level(
                    position.0.div_euclid(8),
                    position.1.div_euclid(8),
                    position.2,
                    WORLD_ITEM_ADDED,
                )?;
            }
            Ok(placed)
        })();
        self.cell.retire_components(retired);
        result.map_err(WorldItemPlacementError::Component)
    }
}

struct NativeRenderAdapter<'a>(&'a mut dyn super::RenderSquareHooks);
impl NativeFloorRender for NativeRenderAdapter<'_> {
    fn invalidate_render_chunk_level(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        flags: u64,
    ) -> HostResult<()> {
        self.0
            .try_invalidate_render_chunk_level(wx, wy, level, flags)
    }
}

impl IsoGridSquare {
    /// `getApparentZ(dx, dy)`: the stairs' height (with the 0.1 fudge when
    /// `fbo_render_chunk`), else the sloped surface's, above the level.
    pub fn get_apparent_z(
        &self,
        dx: f32,
        dy: f32,
        m: &IsoSpriteManager,
        fbo_render_chunk: bool,
    ) -> f32 {
        let dx = dx.clamp(0.0, 1.0);
        let dy = dy.clamp(0.0, 1.0);
        let fudge = if fbo_render_chunk { 0.1f32 } else { 0.0 };
        // `PZMath.lerp(a, b, t)`: `a + (b - a) * t`.
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let z = self.z as f32;
        let ty = &m.tables.ty;
        if self.has_type(ty.stairsTN) {
            z + lerp(0.6666 + fudge, 1.0, 1.0 - dy)
        } else if self.has_type(ty.stairsTW) {
            z + lerp(0.6666 + fudge, 1.0, 1.0 - dx)
        } else if self.has_type(ty.stairsMN) {
            z + lerp(0.3333 + fudge, 0.6666 + fudge, 1.0 - dy)
        } else if self.has_type(ty.stairsMW) {
            z + lerp(0.3333 + fudge, 0.6666 + fudge, 1.0 - dx)
        } else if self.has_type(ty.stairsBN) {
            z + lerp(0.01, 0.3333 + fudge, 1.0 - dy)
        } else if self.has_type(ty.stairsBW) {
            z + lerp(0.01, 0.3333 + fudge, 1.0 - dx)
        } else {
            z + self.get_sloped_surface_height_at(dx, dy)
        }
    }

    /// `getTotalWeightOfItemsOnFloor()`: the sum of `getUnequippedWeight()`
    /// (`weight`, given the world object, which holds a dropped item's
    /// fluid container: `ItemOwner::world_item_fluid_amount`) of the items
    /// of the independent `worldObjects` list, including duplicate and detached
    /// objects. Each iteration reads the live list's current size and entry.
    /// `weight` is a synchronous Rust calculation, never a Lua callback: the
    /// object and item read guards cover it, but no list or floor guard does.
    pub fn get_total_weight_of_items_on_floor(
        &self,
        weight: impl Fn(&IsoObject, &InventoryItem) -> f32,
    ) -> Result<f32, WorldObjectsReadError> {
        let mut total = 0.0f32;
        let mut index = 0;
        loop {
            let entry = {
                let list = self
                    .world_objects
                    .try_borrow()
                    .map_err(|_| WorldObjectsReadError::BorrowedList { index })?;
                if index >= list.len() {
                    break;
                }
                list.entry(index).expect("entry below current list size")
            };
            let owner = match entry {
                WorldObjectEntry::Object(owner) => owner,
                WorldObjectEntry::Foreign(pz_kahlua::Value::Nil) => {
                    return Err(WorldObjectsReadError::NullEntry { index });
                }
                WorldObjectEntry::Foreign(_) => {
                    return Err(WorldObjectsReadError::WrongType { index });
                }
            };
            let object = owner
                .try_borrow()
                .map_err(|_| WorldObjectsReadError::BorrowedObject { index })?;
            let ObjectKind::WorldInventory(floor) = &object.kind else {
                return Err(WorldObjectsReadError::WrongType { index });
            };
            let floor_owner = floor.ref_handle();
            let item = floor_owner
                .try_borrow()
                .map_err(|_| WorldObjectsReadError::BorrowedFloor { index })?
                .item
                .item_ref();
            if let Some(item) = item {
                let item = item
                    .try_borrow()
                    .map_err(|_| WorldObjectsReadError::BorrowedItem { index })?;
                total += weight(&object, &item);
            }
            index += 1;
        }
        Ok(total)
    }
}

impl IsoCell {
    /// `IsoGameCharacter.getSolidFloorAt(x, y, z)`: the first square from
    /// level `z` down to 0 that exists and `TreatAsSolidFloor()`.
    pub fn get_solid_floor_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        m: &IsoSpriteManager,
    ) -> Option<SquarePos> {
        self.try_get_solid_floor_at(x, y, z, m)
            .expect("synchronous archive floor query requires unborrowed squares")
    }

    pub fn try_get_solid_floor_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        m: &IsoSpriteManager,
    ) -> HostResult<Option<SquarePos>> {
        self.try_get_solid_floor_ref_at(x, y, z, m)
            .map(|square| square.map(|(pos, _)| pos))
    }

    pub fn try_get_solid_floor_ref_at(
        &self,
        x: i32,
        y: i32,
        mut z: i32,
        m: &IsoSpriteManager,
    ) -> HostResult<Option<(SquarePos, crate::SquareRef)>> {
        while z >= 0 {
            if let Some(square) = self.get_grid_square(x, y, z) {
                let body = square
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("solid floor square"))?;
                if body.treat_as_solid_floor(m) {
                    drop(body);
                    return Ok(Some(((x, y, z), square)));
                }
            }
            z -= 1;
        }
        Ok(None)
    }

    pub fn try_get_apparent_z(
        &self,
        pos: SquarePos,
        dx: f32,
        dy: f32,
        sprites: &IsoSpriteManager,
        fbo_render_chunk: bool,
    ) -> HostResult<f32> {
        let square =
            self.get_grid_square(pos.0, pos.1, pos.2)
                .ok_or(RuntimeHostError::InvalidHostState(
                    "apparent floor square disappeared",
                ))?;
        let square = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("apparent floor square"))?;
        Ok(square.get_apparent_z(dx, dy, sprites, fbo_render_chunk))
    }
}

/// `IsoWorldInventoryObject.getRenderSquare()`: the square south of one
/// at chunk-local (0, 7), east of one at (7, 0), else the square.
pub fn render_square(cell: &IsoCell, pos: SquarePos) -> Option<SquarePos> {
    let (lx, ly) = (pos.0.rem_euclid(8), pos.1.rem_euclid(8));
    let dir = match (lx, ly) {
        (0, 7) => IsoDirections::S,
        (7, 0) => IsoDirections::E,
        _ => return Some(pos),
    };
    let (dx, dy) = dir.delta();
    let p = (pos.0 + dx, pos.1 + dy, pos.2);
    cell.get_grid_square(p.0, p.1, p.2).map(|_| p)
}

/// `ComponentType.FluidContainer`'s id.
impl MutationEnv<'_> {
    /// `AddWorldInventoryItem(item, x, y, height)` (with `transmit`, which
    /// only matters on a server) on the square at `pos`; `world_age_hours`
    /// is `GameTime.getWorldAgeHours()` (the object's `dropTime`).
    ///
    /// A corpse item becomes a dead body (`tryAddCorpseToWorld`), a
    /// generator an `IsoGenerator`, an animal an `IsoAnimal` (through the
    /// hooks). Anything else becomes an `IsoWorldInventoryObject`: the
    /// item's world x/y rotations are reset, a negative z rotation becomes
    /// `Rand.Next(0, 360)`, a zero `x` or `y` offset `Rand.Next(1000) /
    /// 1000`, its fluid container moves to the object, and the object
    /// (named after the item, with its key id) is appended to `objects`
    /// and `worldObjects` without the recalculation `AddTileObject` makes,
    /// then joins the world.
    ///
    /// The name is `item.getName()`, the display name, from
    /// [`super::SquareMutationHooks::item_display_name`] (the stored name
    /// without it).
    ///
    /// STUB: `updateSprite()` (the world texture and scale; render state).
    /// Legacy synchronous archive/fixture adapter. Production callers use the
    /// fallible try_add_world_inventory_item path and propagate Host faults.
    pub fn add_world_inventory_item_data_projection(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> WorldItemPlaced {
        self.try_add_world_inventory_item_data_projection(pos, item, x, y, height, world_age_hours)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Explicit Rust data/renderer projection. This is not native constructor
    /// completion; detached owners and failed prefixes still retire safely.
    /// Production native callers use try_add_world_inventory_item instead.
    pub fn try_add_world_inventory_item_data_projection(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        self.try_add_world_inventory_item_profile(pos, item, x, y, height, Some(world_age_hours))
    }

    /// Actual native constructor profile. Unimplemented visual phases stop
    /// before square/time/component/list/backlink publication.
    pub fn try_add_world_inventory_item(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        NativeFloorPlacement {
            cell: self.cell,
            rand: self.rand,
            render: &mut NativeRenderAdapter(self.render),
        }
        .place(pos, item, x, y, height)
    }

    fn try_add_world_inventory_item_profile(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        data_projection_age: Option<f64>,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        if self.sq(pos).is_none() {
            if data_projection_age.is_some() {
                return Ok(WorldItemPlaced::NotPlaced);
            }
            return Err(WorldItemPlacementError::Component(
                RuntimeHostError::InvalidHostState(
                    "native floor placement requires its actual square",
                ),
            ));
        }
        // `tryAddCorpseToWorld(item, x, y)`.
        let corpse = {
            let i = item.try_borrow().map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                )
            })?;
            i.is_human_corpse() || i.is_animal_corpse()
        };
        if corpse {
            if data_projection_age.is_none() {
                return Err(WorldItemPlacementError::Component(
                    RuntimeHostError::Unsupported {
                        operation: "native corpse floor placement",
                        kind: None,
                    },
                ));
            }
            let body = self.world.corpse_from_item(
                pos,
                &*item.try_borrow().map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                    )
                })?,
            );
            let Some(mut body) = body else {
                return Ok(WorldItemPlaced::NotPlaced);
            };
            body.base.x = pos.0 as f32 + x;
            body.base.y = pos.1 as f32 + y;
            body.base.z = self
                .apparent_z(pos, x, y)
                .map_err(WorldItemPlacementError::Component)?;
            self.add_corpse(pos, body);
            self.world
                .process_items_remove(&*item.try_borrow().map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                    )
                })?);
            let index = self
                .sq(pos)
                .map_or(0, |sq| sq.borrow().static_moving.len() - 1);
            return Ok(WorldItemPlaced::Corpse { pos, index });
        }
        self.invalidate_render(pos, WORLD_ITEM_ADDED);
        let generator = {
            let i = item.try_borrow().map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                )
            })?;
            i.get_full_type().contains(".Generator")
                || i.has_tag("base:generator") && i.template.world_object_sprite.is_some()
        };
        if generator {
            if data_projection_age.is_none() {
                return Err(WorldItemPlacementError::Component(
                    RuntimeHostError::Unsupported {
                        operation: "native generator floor placement",
                        kind: None,
                    },
                ));
            }
            let obj = self.world.generator_from_item(
                pos,
                &*item.try_borrow().map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                    )
                })?,
            );
            let Some(obj) = obj else {
                return Ok(WorldItemPlaced::NotPlaced);
            };
            let id = self
                .add_special_object(pos, obj, -1)
                .map_err(WorldItemPlacementError::Component)?;
            self.world
                .process_items_remove(&*item.try_borrow().map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                    )
                })?);
            return Ok(match id {
                Some(id) => WorldItemPlaced::Generator(ObjectRef { pos, id }),
                None => WorldItemPlaced::NotPlaced,
            });
        }
        if item
            .try_borrow()
            .map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                )
            })?
            .template
            .class
            == ItemClass::Animal
        {
            if data_projection_age.is_none() {
                return Err(WorldItemPlacementError::Component(
                    RuntimeHostError::Unsupported {
                        operation: "native animal floor placement",
                        kind: None,
                    },
                ));
            }
            self.world.animal_dropped(
                pos,
                &*item.try_borrow().map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                    )
                })?,
            );
            self.world
                .process_items_remove(&*item.try_borrow().map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement item"),
                    )
                })?);
            return Ok(WorldItemPlaced::Animal);
        }
        let slot =
            self.construct_world_inventory_item_profile(item, x, y, height, data_projection_age)?;
        self.finish_world_inventory_item(pos, slot, false)
    }

    /// Explicit Rust data/renderer constructor projection. No native visual
    /// completion is claimed. It deliberately does not publish worldItem;
    /// projection fixtures can finish through the named created continuation.
    pub fn construct_world_inventory_item_data_projection(
        &mut self,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> Result<ObjectSlot, WorldItemPlacementError> {
        self.construct_world_inventory_item_profile(item, x, y, height, Some(world_age_hours))
    }

    pub fn construct_world_inventory_item(
        &mut self,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> Result<ObjectSlot, WorldItemPlacementError> {
        self.construct_world_inventory_item_profile(item, x, y, height, None)
    }

    fn construct_world_inventory_item_profile(
        &mut self,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        data_projection_age: Option<f64>,
    ) -> Result<ObjectSlot, WorldItemPlacementError> {
        let slot = construct_floor_prefix(
            self.cell,
            self.rand,
            item,
            x,
            y,
            height,
            data_projection_age,
        )?;
        let object = slot.ref_handle();
        // Native transfer sees the canonical whole receiver, not a future
        // success-only owner or a by-value component staging container.
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(pz_entity::EntityOwner::from_ref(&object));
        let result =
            crate::floor_object::runtime::fluid_from_item_before_world(item, &object, &mut retired);
        self.cell.retire_components(retired);
        result.map_err(WorldItemPlacementError::Component)?;
        Ok(slot)
    }

    /// Explicit data-projection created continuation, retaining the native
    /// relative backlink/list order without claiming constructor/autoAge or
    /// Sprite completion. Never used by the production native floor service.
    pub fn finish_created_world_inventory_item_data_projection(
        &mut self,
        pos: SquarePos,
        slot: ObjectSlot,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        self.finish_world_inventory_item(pos, slot, true)
    }

    fn finish_world_inventory_item(
        &mut self,
        pos: SquarePos,
        slot: ObjectSlot,
        created: bool,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        let object = slot.ref_handle();
        // Pin the incoming receiver before any failure can discard the last
        // local owner inside an outer WORLD/sprite guard.
        self.cell.retire_object(ObjectSlot::retain(&object));
        let item = crate::floor_object::runtime::item_ref(&object)
            .map_err(WorldItemPlacementError::Component)?
            .ok_or_else(|| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::InvalidHostState(
                        "null floor placement item",
                    ),
                )
            })?;
        let mut retired = pz_entity::RetiredComponents::new();
        if created {
            let result =
                crate::floor_object::runtime::publish_item_backlink(&object, &item, &mut retired);
            self.cell.retire_components(std::mem::take(&mut retired));
            result.map_err(WorldItemPlacementError::Component)?;
        }
        let (name, key_id) = {
            let i = item.try_borrow().map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("floor item name/key"),
                )
            })?;
            (
                self.world
                    .item_display_name(&i)
                    .unwrap_or_else(|| i.get_name().to_string()),
                i.get_key_id(),
            )
        };
        {
            let mut obj = object.try_borrow_mut().map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed(
                        "floor name/key destination",
                    ),
                )
            })?;
            if created {
                obj.key_id = key_id;
                obj.name = Some(ObjectName::Str(name));
            } else {
                obj.name = Some(ObjectName::Str(name));
                obj.key_id = key_id;
            }
        }
        let sq_owner = self.sq_mut(pos).ok_or_else(|| {
            WorldItemPlacementError::Component(
                pz_entity::runtime_load::RuntimeHostError::InvalidHostState(
                    "floor placement square disappeared",
                ),
            )
        })?;
        let id = {
            let mut sq = sq_owner.try_borrow_mut().map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement square"),
                )
            })?;
            let id = sq.new_object_id();
            object
                .try_borrow_mut()
                .map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement id"),
                    )
                })?
                .id = id;
            sq.objects.push(slot);
            sq.world_objects
                .try_borrow_mut()
                .map_err(|_| {
                    WorldItemPlacementError::Component(
                        pz_entity::runtime_load::RuntimeHostError::Borrowed(
                            "floor placement worldObjects",
                        ),
                    )
                })?
                .push_object(&object);
            id
        };
        if !created {
            let result =
                crate::floor_object::runtime::publish_item_backlink(&object, &item, &mut retired);
            self.cell.retire_components(retired);
            result.map_err(WorldItemPlacementError::Component)?;
        }
        let r = ObjectRef { pos, id };
        if !created {
            super::lifecycle::add_to_world_handle(self, r, &object)
                .map_err(WorldItemPlacementError::Component)?;
            self.world.designation_zone_item_on_ground(r, object);
        }
        if let Some(rs) = render_square(self.cell, pos) {
            self.invalidate_render(rs, WORLD_ITEM_ADDED);
        }
        Ok(WorldItemPlaced::Object(r))
    }

    fn apparent_z(&self, pos: SquarePos, dx: f32, dy: f32) -> HostResult<f32> {
        self.cell
            .try_get_apparent_z(pos, dx, dy, self.sprites, self.render.fbo_render_chunk())
    }
}

/// The world a character's items drop into
/// ([`pz_inventory::floor_drop`]): the square mutations, the world age for
/// `dropTime`, and what each `AddWorldInventoryItem` made (`placed`, in
/// order). `getApparentZ`'s fudge follows
/// [`super::RenderSquareHooks::fbo_render_chunk`].
pub struct FloorDropEnv<'e, 'a> {
    pub env: &'e mut MutationEnv<'a>,
    data_projection_age: Option<f64>,
    native_host: Option<&'e mut dyn NativeFloorHost>,
    pub placed: Vec<WorldItemPlaced>,
}

impl<'e, 'a> FloorDropEnv<'e, 'a> {
    pub fn new_data_projection(env: &'e mut MutationEnv<'a>, world_age_hours: f64) -> Self {
        FloorDropEnv {
            env,
            data_projection_age: Some(world_age_hours),
            native_host: None,
            placed: Vec::new(),
        }
    }

    pub fn new_native(env: &'e mut MutationEnv<'a>) -> Self {
        FloorDropEnv {
            env,
            data_projection_age: None,
            native_host: None,
            placed: Vec::new(),
        }
    }

    pub fn new_native_with_host(
        env: &'e mut MutationEnv<'a>,
        host: &'e mut dyn NativeFloorHost,
    ) -> Self {
        Self {
            env,
            data_projection_age: None,
            native_host: Some(host),
            placed: Vec::new(),
        }
    }
}

impl FloorWorld for FloorDropEnv<'_, '_> {
    fn solid_floor_ref_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> HostResult<Option<pz_inventory::floor_drop::FloorSquare>> {
        let square = self
            .env
            .cell
            .try_get_solid_floor_ref_at(x, y, z, self.env.sprites)?;
        Ok(
            square.map(|(position, square)| match self.data_projection_age {
                Some(_) => pz_inventory::floor_drop::FloorSquare::data_projection(position),
                None => pz_inventory::floor_drop::FloorSquare::actual(
                    position,
                    pz_entity::EntityOwner::from_ref(&square),
                ),
            }),
        )
    }

    fn apparent_z_on(
        &self,
        square: &pz_inventory::floor_drop::FloorSquare,
        dx: f32,
        dy: f32,
    ) -> HostResult<f32> {
        if self.data_projection_age.is_some() {
            if square.owner_slot().is_some() {
                return Err(RuntimeHostError::InvalidHostState(
                    "data projection floor target has actual owner",
                ));
            }
            return self.env.apparent_z(square.position(), dx, dy);
        }
        let square = square
            .owner_slot()
            .and_then(|owner| owner.try_handle::<IsoGridSquare>())
            .ok_or(RuntimeHostError::Unsupported {
                operation: "actual retained floor square",
                kind: None,
            })?;
        let body = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual apparent floor square"))?;
        Ok(body.get_apparent_z(dx, dy, self.env.sprites, self.env.render.fbo_render_chunk()))
    }

    fn add_world_inventory_item_on(
        &mut self,
        square: &pz_inventory::floor_drop::FloorSquare,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        let result = if let Some(age) = self.data_projection_age {
            if square.owner_slot().is_some() {
                return Err(RuntimeHostError::InvalidHostState(
                    "data projection floor target has actual owner",
                ));
            }
            self.env.try_add_world_inventory_item_data_projection(
                square.position(),
                item,
                x,
                y,
                height,
                age,
            )
        } else {
            let square = square
                .owner_slot()
                .and_then(|owner| owner.try_handle::<IsoGridSquare>())
                .ok_or(RuntimeHostError::Unsupported {
                    operation: "actual retained floor square",
                    kind: None,
                })?;
            let mut placement = NativeFloorPlacement {
                cell: self.env.cell,
                rand: self.env.rand,
                render: &mut NativeRenderAdapter(self.env.render),
            };
            match self.native_host.as_deref_mut() {
                Some(host) => placement.place_retained(&square, item, x, y, height, host),
                None => {
                    placement.place_retained(&square, item, x, y, height, &mut UnavailableFloorHost)
                }
            }
        };
        let placed = result.map_err(|error| match error {
            WorldItemPlacementError::Component(error) => error,
            WorldItemPlacementError::Binding(_) => {
                RuntimeHostError::InvalidHostState("floor constructor whole-owner binding failed")
            }
        })?;
        self.placed.push(placed);
        Ok(())
    }
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<SquarePos>> {
        self.env
            .cell
            .try_get_solid_floor_at(x, y, z, self.env.sprites)
    }

    fn apparent_z(&self, sq: SquarePos, dx: f32, dy: f32) -> HostResult<f32> {
        self.env.apparent_z(sq, dx, dy)
    }

    fn rand_float(&mut self, min: f32, max: f32) -> HostResult<f32> {
        Ok(self.env.rand.next_float_range(min, max))
    }

    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        let placed = match self.data_projection_age {
            Some(age) => self
                .env
                .try_add_world_inventory_item_data_projection(sq, item, x, y, height, age),
            None => self
                .env
                .try_add_world_inventory_item(sq, item, x, y, height),
        };
        let p = placed.map_err(|error| match error {
            WorldItemPlacementError::Component(error) => error,
            WorldItemPlacementError::Binding(_) => {
                pz_inventory::floor_drop::RuntimeHostError::InvalidHostState(
                    "floor constructor whole-owner binding failed",
                )
            }
        })?;
        self.placed.push(p);
        Ok(())
    }
}

#[cfg(test)]
mod visual_tests;

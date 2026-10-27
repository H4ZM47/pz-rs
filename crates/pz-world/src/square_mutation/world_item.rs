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
    obj.sprite_id_override = data_projection_age.map(|_| UNREGISTERED_SPRITE_ID);
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
    if data_projection_age.is_none() {
        // Native CreateSprite precedes updateSprite, square assignment,
        // world-age read and Fluid transfer. Retain this actual receiver
        // with its completed item/rotation/offset prefix on unsupported.
        cell.retire_object(slot);
        return Err(WorldItemPlacementError::Component(
            RuntimeHostError::Unsupported {
                operation: "native IsoSprite.CreateSprite/LoadSingleTexture constructor",
                kind: None,
            },
        ));
    }
    Ok(slot)
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
        if self.cell.get_grid_square(pos.0, pos.1, pos.2).is_none() {
            return Err(WorldItemPlacementError::Component(
                RuntimeHostError::InvalidHostState(
                    "native floor placement requires its actual square",
                ),
            ));
        }
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
        // A future real Sprite implementation must provide an owned native
        // continuation rather than accidentally treating the prefix as placed.
        self.cell.retire_object(slot);
        Err(WorldItemPlacementError::Component(
            RuntimeHostError::Unsupported {
                operation: "native floor post-constructor continuation",
                kind: None,
            },
        ))
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
        mut z: i32,
        m: &IsoSpriteManager,
    ) -> HostResult<Option<SquarePos>> {
        while z >= 0 {
            if let Some(square) = self.get_grid_square(x, y, z) {
                let square = square
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("solid floor square"))?;
                if square.treat_as_solid_floor(m) {
                    return Ok(Some((x, y, z)));
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
            let id = self.add_special_object(pos, obj, -1);
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
            super::lifecycle::add_to_world_handle(self, r, &object);
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
    pub placed: Vec<WorldItemPlaced>,
}

impl<'e, 'a> FloorDropEnv<'e, 'a> {
    pub fn new_data_projection(env: &'e mut MutationEnv<'a>, world_age_hours: f64) -> Self {
        FloorDropEnv {
            env,
            data_projection_age: Some(world_age_hours),
            placed: Vec::new(),
        }
    }

    pub fn new_native(env: &'e mut MutationEnv<'a>) -> Self {
        FloorDropEnv {
            env,
            data_projection_age: None,
            placed: Vec::new(),
        }
    }
}

impl FloorWorld for FloorDropEnv<'_, '_> {
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

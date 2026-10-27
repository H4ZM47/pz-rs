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
use pz_inventory::floor_drop::FloorWorld;
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
        mut z: i32,
        m: &IsoSpriteManager,
    ) -> Option<SquarePos> {
        while z >= 0 {
            if self
                .get_grid_square(x, y, z)
                .is_some_and(|sq| sq.borrow().treat_as_solid_floor(m))
            {
                return Some((x, y, z));
            }
            z -= 1;
        }
        None
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
    pub fn add_world_inventory_item(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> WorldItemPlaced {
        self.try_add_world_inventory_item(pos, item, x, y, height, world_age_hours)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Fallible actual-reference path. Detached owners and failed receiver
    /// pins are queued in the cell until an outer world/sprite safe boundary.
    /// Native world-manager/meta registration still requires its real host;
    /// component storage movement alone does not complete that lifecycle.
    pub fn try_add_world_inventory_item(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> Result<WorldItemPlaced, WorldItemPlacementError> {
        if self.sq(pos).is_none() {
            return Ok(WorldItemPlaced::NotPlaced);
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
            body.base.z = self.apparent_z(pos, x, y);
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
        let slot = self.construct_world_inventory_item(item, x, y, height, world_age_hours)?;
        self.finish_world_inventory_item(pos, slot, false)
    }

    /// The actual native floor constructor. It deliberately does not set the
    /// item's worldItem field. A string-overload caller can retain this owned
    /// continuation, perform genuine setAutoAge outside host guards, and then
    /// call finish_created_world_inventory_item. It must not use the supplied
    /// overload's corpse/generator/animal branches.
    pub fn construct_world_inventory_item(
        &mut self,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> Result<ObjectSlot, WorldItemPlacementError> {
        {
            let mut i = item.try_borrow_mut().map_err(|_| {
                WorldItemPlacementError::Component(
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("floor constructor item"),
                )
            })?;
            i.world_x_rotation = 0.0;
            i.world_y_rotation = 0.0;
            if i.world_z_rotation < 0.0 {
                i.world_z_rotation = self.rand.next_range(0, 360) as f32;
            }
        }
        shared::set_container(item, None);
        let xoff = if x == 0.0 {
            self.rand.next(1000) as f32 / 1000.0
        } else {
            x
        };
        let yoff = if y == 0.0 {
            self.rand.next(1000) as f32 / 1000.0
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
                    drop_time: world_age_hours,
                    ignore_remove_sandbox: false,
                    extended_placement: false,
                },
            )),
            None,
        );
        obj.sprite_id_override = Some(UNREGISTERED_SPRITE_ID);
        obj.pending_floor_visual_tail =
            Some(crate::objects::FloorVisualTail::ConstructorUpdateSprite);
        obj.outline_on_mouseover = true;
        let slot = ObjectSlot::try_new(obj).map_err(|failure| {
            self.cell.retire_object(ObjectSlot::retain(&failure.object));
            WorldItemPlacementError::Binding(failure.error)
        })?;
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

    /// Complete only the string overload's post-constructor continuation.
    /// Its real optional autoAge phase must already have run; no age/factory
    /// success is invented here. Backlink publication precedes both lists.
    pub fn finish_created_world_inventory_item(
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

    fn apparent_z(&self, pos: SquarePos, dx: f32, dy: f32) -> f32 {
        let fbo = self.render.fbo_render_chunk();
        self.sq(pos).map_or(pos.2 as f32, |sq| {
            sq.borrow().get_apparent_z(dx, dy, self.sprites, fbo)
        })
    }
}

/// The world a character's items drop into
/// ([`pz_inventory::floor_drop`]): the square mutations, the world age for
/// `dropTime`, and what each `AddWorldInventoryItem` made (`placed`, in
/// order). `getApparentZ`'s fudge follows
/// [`super::RenderSquareHooks::fbo_render_chunk`].
pub struct FloorDropEnv<'e, 'a> {
    pub env: &'e mut MutationEnv<'a>,
    pub world_age_hours: f64,
    pub placed: Vec<WorldItemPlaced>,
}

impl<'e, 'a> FloorDropEnv<'e, 'a> {
    pub fn new(env: &'e mut MutationEnv<'a>, world_age_hours: f64) -> Self {
        FloorDropEnv {
            env,
            world_age_hours,
            placed: Vec::new(),
        }
    }
}

impl FloorWorld for FloorDropEnv<'_, '_> {
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> Option<SquarePos> {
        self.env.cell.get_solid_floor_at(x, y, z, self.env.sprites)
    }

    fn apparent_z(&self, sq: SquarePos, dx: f32, dy: f32) -> f32 {
        self.env.apparent_z(sq, dx, dy)
    }

    fn rand_float(&mut self, min: f32, max: f32) -> f32 {
        self.env.rand.next_float_range(min, max)
    }

    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        let p = self
            .env
            .try_add_world_inventory_item(sq, item, x, y, height, self.world_age_hours)
            .map_err(|error| match error {
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

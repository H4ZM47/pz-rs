//! Items put on the floor: `IsoGridSquare.AddWorldInventoryItem(item, x,
//! y, height)` with the `IsoWorldInventoryObject` it makes,
//! `getApparentZ`, `IsoGameCharacter.getSolidFloorAt`, and
//! [`FloorDropEnv`], the [`pz_inventory::floor_drop::FloorWorld`] a
//! character's items drop into (`setWornItem`'s `forceDropTooHeavy`,
//! `dropHeavyItems`, `dropHandItems`).
//!
//! The world object holds a copy of the item ([`ItemBlob::Item`]); the
//! caller's shared item gets the same changes (rotations, the fluid
//! container moving to the object), so both agree. Single player: the
//! server's `transmitCompleteItemToClients` is not made.

use pz_inventory::floor_drop::FloorWorld;
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{InventoryItem, ItemClass};

use super::{MutationEnv, ObjectRef, SquarePos};
use crate::chunk::{IsoCell, IsoGridSquare};
use crate::objects::{IsoObject, IsoWorldInventoryObject, ObjectKind, ObjectName};
use crate::save::items::ItemBlob;
use crate::save::object_io::UNREGISTERED_SPRITE_ID;
use crate::sprite::IsoSpriteManager;
use crate::square::IsoDirections;

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
    /// of `worldObjects`.
    pub fn get_total_weight_of_items_on_floor(
        &self,
        weight: impl Fn(&IsoObject, &InventoryItem) -> f32,
    ) -> f32 {
        let mut total = 0.0f32;
        for id in &self.world_objects {
            let Some(o) = self.objects.iter().find(|o| o.id == *id) else {
                continue;
            };
            if let ObjectKind::WorldInventory(w) = &o.kind {
                if let Some(i) = w.item.as_item() {
                    total += weight(o, i);
                }
            }
        }
        total
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
                .is_some_and(|sq| sq.treat_as_solid_floor(m))
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
fn fluid_container_id() -> i16 {
    pz_entity::ComponentType::FluidContainer.id()
}

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
    pub fn add_world_inventory_item(
        &mut self,
        pos: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
        world_age_hours: f64,
    ) -> WorldItemPlaced {
        if self.sq(pos).is_none() {
            return WorldItemPlaced::NotPlaced;
        }
        // `tryAddCorpseToWorld(item, x, y)`.
        let corpse = {
            let i = item.borrow();
            i.is_human_corpse() || i.is_animal_corpse()
        };
        if corpse {
            let body = self.world.corpse_from_item(pos, &item.borrow());
            let Some(mut body) = body else {
                return WorldItemPlaced::NotPlaced;
            };
            body.base.x = pos.0 as f32 + x;
            body.base.y = pos.1 as f32 + y;
            body.base.z = self.apparent_z(pos, x, y);
            self.add_corpse(pos, body);
            self.world.process_items_remove(&item.borrow());
            let index = self.sq(pos).map_or(0, |sq| sq.static_moving.len() - 1);
            return WorldItemPlaced::Corpse { pos, index };
        }
        self.invalidate_render(pos, WORLD_ITEM_ADDED);
        let generator = {
            let i = item.borrow();
            i.get_full_type().contains(".Generator")
                || i.has_tag("base:generator") && i.template.world_object_sprite.is_some()
        };
        if generator {
            let obj = self.world.generator_from_item(pos, &item.borrow());
            let Some(obj) = obj else {
                return WorldItemPlaced::NotPlaced;
            };
            let id = self.add_special_object(pos, obj, -1);
            self.world.process_items_remove(&item.borrow());
            return match id {
                Some(id) => WorldItemPlaced::Generator(ObjectRef { pos, id }),
                None => WorldItemPlaced::NotPlaced,
            };
        }
        if item.borrow().template.class == ItemClass::Animal {
            self.world.animal_dropped(pos, &item.borrow());
            self.world.process_items_remove(&item.borrow());
            return WorldItemPlaced::Animal;
        }
        // `new IsoWorldInventoryObject(item, sq, x, y, height)`.
        let mut components = Vec::new();
        {
            let mut i = item.borrow_mut();
            i.world_x_rotation = 0.0;
            i.world_y_rotation = 0.0;
            if i.world_z_rotation < 0.0 {
                i.world_z_rotation = self.rand.next_range(0, 360) as f32;
            }
        }
        shared::set_container(item, None);
        let mut xoff = x;
        let mut yoff = y;
        if xoff == 0.0 {
            xoff = self.rand.next(1000) as f32 / 1000.0;
        }
        if yoff == 0.0 {
            yoff = self.rand.next(1000) as f32 / 1000.0;
        }
        {
            // `GameEntityFactory.TransferComponent(item, this,
            // FluidContainer)`.
            let mut i = item.borrow_mut();
            let fc = fluid_container_id();
            if let Some(k) = i.components.iter().position(|b| b.type_id == fc) {
                components.push(i.components.remove(k));
                i.component_cache = None;
            }
        }
        let (name, key_id, copy) = {
            let i = item.borrow();
            let name = self
                .world
                .item_display_name(&i)
                .unwrap_or_else(|| i.get_name().to_string());
            (name, i.get_key_id(), i.clone())
        };
        let mut obj = IsoObject::new(
            0,
            ObjectKind::WorldInventory(Box::new(IsoWorldInventoryObject {
                xoff,
                yoff,
                zoff: height,
                offset_x: 0.0,
                offset_y: 0.0,
                item: ItemBlob::Item(Box::new(copy)),
                drop_time: world_age_hours,
                ignore_remove_sandbox: false,
                extended_placement: false,
            })),
            None,
        );
        obj.sprite_id_override = Some(UNREGISTERED_SPRITE_ID);
        obj.outline_on_mouseover = true;
        obj.has_components = !components.is_empty();
        obj.components = components;
        obj.name = Some(ObjectName::Str(name));
        obj.key_id = key_id;
        let sq = self.sq_mut(pos).unwrap();
        obj.id = sq.new_object_id();
        let id = obj.id;
        sq.objects.push(obj);
        sq.world_objects.push(id);
        let r = ObjectRef { pos, id };
        super::lifecycle::add_to_world(self, r);
        {
            let MutationEnv { cell, world, .. } = self;
            if let Some(o) = cell
                .get_grid_square(pos.0, pos.1, pos.2)
                .and_then(|sq| sq.objects.iter().find(|o| o.id == id))
            {
                world.designation_zone_item_on_ground(r, o);
            }
        }
        if let Some(rs) = render_square(self.cell, pos) {
            self.invalidate_render(rs, WORLD_ITEM_ADDED);
        }
        WorldItemPlaced::Object(r)
    }

    fn apparent_z(&self, pos: SquarePos, dx: f32, dy: f32) -> f32 {
        let fbo = self.render.fbo_render_chunk();
        self.sq(pos).map_or(pos.2 as f32, |sq| {
            sq.get_apparent_z(dx, dy, self.sprites, fbo)
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
    ) {
        let p = self
            .env
            .add_world_inventory_item(sq, item, x, y, height, self.world_age_hours);
        self.placed.push(p);
    }
}

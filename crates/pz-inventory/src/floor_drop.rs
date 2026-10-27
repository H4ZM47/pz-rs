//! A character's items falling to the floor: `IsoGameCharacter`'s
//! `forceDropTooHeavy` branch of `setWornItem` (a player taking off an
//! item it has no room for drops it), `dropHeavyItems` and
//! `dropHandItems`, with the item rules they use (`isHeavyItem`,
//! `ItemContainer.hasRoomFor(chr, item)` / `isItemAllowed` /
//! `getCapacity` of the character's inventory, `Remove`, `DoRemoveItem`).
//!
//! The character is a [`DropCharacter`] (its inventory, hands, square and
//! weights; the Lua events and sounds) and the world a [`FloorWorld`]
//! (`getSolidFloorAt`, `getApparentZ`, `Rand` and
//! `IsoGridSquare.AddWorldInventoryItem`; `pz-world`'s `FloorDropEnv`
//! implements it over the square mutations).
//!
//! Single player only: the server's `dropHeldItems` and the packets
//! (`sendRemoveItemFromContainer`, `PlayerDropHeldItems`, `Equip`) are not
//! ported, and `GameClient.client` is false.

use std::rc::Rc;

// Public carrier for downstream character callers without a direct entity
// dependency. Host faults remain distinct from native absence/no-drop.
pub use pz_entity::runtime_load::{HostResult, RuntimeHostError};

use crate::item::{InventoryItem, ItemKind};
use crate::shared::{self, ContainerRef, ItemRef, SharedContainer};
use crate::template::ItemClass;

/// A square position `(x, y, z)`.
pub type SquarePos = (i32, i32, i32);

/// An actual retained square receiver plus its immutable geometric position.
/// The erased owner holds the square allocation itself across host callbacks.
/// Coordinate-only profiles must be constructed explicitly for data fixtures.
#[derive(Clone, Debug)]
pub struct FloorSquare {
    position: SquarePos,
    owner: Option<pz_entity::EntityOwner>,
}
impl FloorSquare {
    pub fn actual(position: SquarePos, owner: pz_entity::EntityOwner) -> Self {
        Self {
            position,
            owner: Some(owner),
        }
    }
    pub fn data_projection(position: SquarePos) -> Self {
        Self {
            position,
            owner: None,
        }
    }
    pub fn position(&self) -> SquarePos {
        self.position
    }
    pub fn owner_slot(&self) -> Option<&pz_entity::EntityOwner> {
        self.owner.as_ref()
    }
}

/// Where [`drop_on_floor`] put an item: the square and the offsets passed
/// to `AddWorldInventoryItem`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dropped {
    pub square: SquarePos,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// The world items fall into.
pub trait FloorWorld {
    /// Retain the same actual result before returning from the native query.
    /// Implementations must never resolve this receiver again by coordinates.
    fn solid_floor_ref_at(&self, _x: i32, _y: i32, _z: i32) -> HostResult<Option<FloorSquare>> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual retained floor query",
            kind: None,
        })
    }
    fn apparent_z_on(&self, _square: &FloorSquare, _dx: f32, _dy: f32) -> HostResult<f32> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual retained floor height",
            kind: None,
        })
    }
    fn add_world_inventory_item_on(
        &mut self,
        _square: &FloorSquare,
        _item: &ItemRef,
        _x: f32,
        _y: f32,
        _height: f32,
    ) -> HostResult<()> {
        Err(RuntimeHostError::Unsupported {
            operation: "actual retained floor placement",
            kind: None,
        })
    }

    /// `IsoGameCharacter.getSolidFloorAt(x, y, z)`: the first square from
    /// `z` down to 0 that exists and `TreatAsSolidFloor()`.
    /// An unavailable/borrowed actual host is Err, not a missing floor.
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<SquarePos>>;
    /// `getApparentZ(dx, dy)` of the square at `sq`.
    fn apparent_z(&self, sq: SquarePos, dx: f32, dy: f32) -> HostResult<f32>;
    /// `Rand.Next(min, max)` (floats).
    fn rand_float(&mut self, min: f32, max: f32) -> HostResult<f32>;
    /// `sq.AddWorldInventoryItem(item, x, y, height)` (the item has left
    /// its container already).
    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()>;
}

/// The character whose items drop (`IsoGameCharacter`), as the drops and
/// `hasRoomFor` read it. Its `inventory` is the container whose `parent`
/// is the character.
pub trait DropCharacter {
    /// `getInventory()`.
    fn inventory(&self) -> ContainerRef;
    /// `this instanceof IsoPlayer`.
    fn is_player(&self) -> bool;
    /// `IsoPlayer.isLocalPlayer()` (true for other characters).
    fn is_local_player(&self) -> bool {
        true
    }
    /// `getVehicle() != null`.
    fn is_in_vehicle(&self) -> bool;
    /// `getCurrentSquare()`: `None` without one.
    fn current_square(&self) -> Option<SquarePos>;
    /// `getInventory().getCapacityWeight()`: 0 for a player in ghost mode
    /// (debug) or with unlimited carry, else `getInventoryWeight()`.
    fn capacity_weight(&self) -> f32;
    /// `item.getUnequippedWeight()`.
    fn unequipped_weight(&self, item: &ItemRef) -> f32;
    /// The `acceptItemFunction` Lua function `function` called with the
    /// inventory and the item (`protectedCallBoolean(...) == TRUE`); true
    /// when there is no such function.
    fn accept_item(&mut self, _function: &str, _item: &ItemRef) -> bool {
        true
    }
    /// `getPrimaryHandItem()`.
    fn primary_hand_item(&self) -> Option<ItemRef>;
    /// `getSecondaryHandItem()`.
    fn secondary_hand_item(&self) -> Option<ItemRef>;
    /// `setPrimaryHandItem(item)` (the drops pass `None`).
    fn set_primary_hand_item(&mut self, item: Option<&ItemRef>);
    /// `setSecondaryHandItem(item)` (the drops pass `None`).
    fn set_secondary_hand_item(&mut self, item: Option<&ItemRef>);
    /// `removeFromHands(item)`: the item leaves the hands it is in.
    fn remove_from_hands(&mut self, item: &ItemRef) {
        if self
            .primary_hand_item()
            .is_some_and(|i| Rc::ptr_eq(&i, item))
        {
            self.set_primary_hand_item(None);
        }
        if self
            .secondary_hand_item()
            .is_some_and(|i| Rc::ptr_eq(&i, item))
        {
            self.set_secondary_hand_item(None);
        }
    }
    /// `LuaEventManager.triggerEvent("OnContainerUpdate")`.
    fn on_container_update(&mut self) {}
    /// `LuaEventManager.triggerEvent("onItemFall", item)`.
    fn on_item_fall(&mut self, _item: &ItemRef) {}
    /// `playDropItemSound(item)`.
    fn play_drop_item_sound(&mut self, _item: &ItemRef) {}
}

impl InventoryItem {
    /// `isHumanCorpse()`: the type is `CorpseFemale` or `CorpseMale`
    /// (ignoring case).
    pub fn is_human_corpse(&self) -> bool {
        let t = self.get_type();
        t.eq_ignore_ascii_case("CorpseFemale") || t.eq_ignore_ascii_case("CorpseMale")
    }

    /// `isAnimalCorpse()`: the type is `CorpseAnimal` (ignoring case).
    pub fn is_animal_corpse(&self) -> bool {
        self.get_type().eq_ignore_ascii_case("CorpseAnimal")
    }

    /// `getKeyId()` (a `Key` keeps its own).
    pub fn get_key_id(&self) -> i32 {
        match &self.kind {
            ItemKind::Key { key_id, .. } => *key_id,
            _ => self.key_id,
        }
    }

    /// `getCategory()`: the non-null `mainCategory`, otherwise the class's.
    pub fn get_category(&self) -> &str {
        // AnimalInventoryItem's override ignores mainCategory entirely.
        if self.template.class == ItemClass::Animal {
            return "Animal";
        }
        if let Some(category) = &self.main_category {
            return category;
        }
        match self.template.class {
            ItemClass::Food => "Food",
            ItemClass::Weapon => "Weapon",
            ItemClass::Clothing => "Clothing",
            ItemClass::AlarmClock | ItemClass::AlarmClockClothing => "AlarmClock",
            ItemClass::Container => "Container",
            ItemClass::Literature => "Literature",
            ItemClass::Key => "Key",
            ItemClass::WeaponPart => "WeaponPart",
            _ => "Item",
        }
    }
}

/// `IsoGameCharacter.isHeavyItem(item)`: a bag, a `HeavyItem`, a corpse,
/// an animal or a generator.
pub fn is_heavy_item(item: Option<&InventoryItem>) -> bool {
    let Some(item) = item else {
        return false;
    };
    if matches!(item.kind, ItemKind::Container(_)) {
        return true;
    }
    if item.has_tag("base:heavyitem") {
        return true;
    }
    if matches!(
        item.get_type(),
        "CorpseMale" | "CorpseFemale" | "Animal" | "CorpseAnimal"
    ) {
        return true;
    }
    item.get_type() == "Generator"
}

/// `ItemContainer.floatingPointCorrection(val)`.
pub fn floating_point_correction(val: f32) -> f32 {
    let tmp = val * 100.0f32;
    let r = if tmp - (tmp as i32) as f32 >= 0.5f32 {
        tmp + 1.0f32
    } else {
        tmp
    };
    (r as i32) as f32 / 100.0f32
}

/// `ItemContainer.getCapacity()` without a vehicle part: at most 50 for a
/// bag's inventory, else 100.
pub fn get_capacity(c: &SharedContainer) -> i32 {
    if c.containing_item.is_some() {
        return c.capacity.min(50);
    }
    c.capacity.min(100)
}

/// `isItemAllowed(item)` of the character's inventory (its parent is the
/// character, whose `isItemAllowedInContainer` is `IsoObject`'s `true`
/// and whose properties name no toaster or coffee maker).
pub fn is_item_allowed(chr: &mut dyn DropCharacter, item: &ItemRef) -> bool {
    let inv = chr.inventory();
    let (kind, only, accept) = {
        let s = inv.borrow();
        (
            s.kind.clone(),
            s.only_accept_category.clone(),
            s.accept_item_function.clone(),
        )
    };
    {
        let i = item.borrow();
        if i.template.class == ItemClass::Animal && kind != "floor" {
            return false;
        }
        if let Some(cat) = &only {
            if !cat.eq_ignore_ascii_case(i.get_category()) {
                return false;
            }
        }
    }
    if let Some(name) = accept {
        if !chr.accept_item(&name, item) {
            return false;
        }
    }
    // `isInvalidClothingRackItem`.
    let i = item.borrow();
    let clothing = matches!(
        i.kind,
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
    );
    !(kind == "clothingrack" && !clothing && !i.has_tag("base:fitsclothingrack"))
}

/// `getInventory().hasRoomFor(chr, item)` of the character `chr`: no
/// heavy item or human corpse while in a vehicle, the item allowed, and
/// `floatingPointCorrection(getCapacityWeight()) + item weight <=
/// getEffectiveCapacity(chr)` (the inventory is the character's, so the
/// floor weight and the traits do not count).
pub fn has_room_for(chr: &mut dyn DropCharacter, item: &ItemRef) -> bool {
    {
        let i = item.borrow();
        if chr.is_in_vehicle() && (i.has_tag("base:heavyitem") || i.is_human_corpse()) {
            return false;
        }
    }
    if !is_item_allowed(chr, item) {
        return false;
    }
    // `hasRoomFor(chr, weight)`: the inventory is in the character's
    // inventory, is not a bag's and has no vehicle part.
    let w = chr.unequipped_weight(item);
    let cap = get_capacity(&chr.inventory().borrow());
    floating_point_correction(chr.capacity_weight()) + w <= cap as f32
}

/// `item.OnBeforeRemoveFromContainer(container)`.
///
/// STUB: `Food.updateAge()` (and a bag's contents') is not ported.
fn on_before_remove_from_container(_item: &ItemRef) {}

/// `getInventory().Remove(item)`: the item leaves the character's hands
/// and the inventory. Returns whether it was there.
pub fn remove(chr: &mut dyn DropCharacter, item: &ItemRef) -> bool {
    try_remove(chr, item).expect("legacy unborrowed character Remove")
}
pub fn try_remove(chr: &mut dyn DropCharacter, item: &ItemRef) -> HostResult<bool> {
    shared::pin_owner(item);
    chr.remove_from_hands(item);
    let inv = chr.inventory();
    shared::pin_owner(&inv);
    let present = inv
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("character Remove inventory"))?
        .index_of(item)
        .is_some();
    if !present {
        return Ok(false);
    }
    on_before_remove_from_container(item);
    shared::try_remove_item(&inv, item)
}

/// Draw/dirty flags precede removal and the final direct item-container field
/// assignment. Typed host faults stop the action at the completed prefix.
pub fn try_do_remove_item(c: &ContainerRef, item: &ItemRef) -> HostResult<()> {
    shared::pin_owner(c);
    shared::pin_owner(item);
    {
        let mut body = c
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("DoRemoveItem flags"))?;
        body.draw_dirty = true;
        if body.parent.is_some() { body.dirty = true; }
    }
    shared::observe_container(c);
    on_before_remove_from_container(item);
    {
        let mut body = c
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("DoRemoveItem list"))?;
        if let Some(index) = body.index_of(item) {
            shared::pin_owner(&body.items.remove(index));
        }
    }
    shared::observe_container(c);
    shared::try_set_container(item, None)
}
pub fn do_remove_item(c: &ContainerRef, item: &ItemRef) {
    try_do_remove_item(c, item).expect("legacy unborrowed DoRemoveItem");
}

/// The drop the four call sites share: `Rand.Next(0.1f, 0.9f)` for x
/// then y, `sq.getApparentZ(x, y) - sq.z` and
/// `sq.AddWorldInventoryItem(item, x, y, z)`.
pub fn drop_on_floor(
    world: &mut dyn FloorWorld,
    sq: &FloorSquare,
    item: &ItemRef,
) -> HostResult<Dropped> {
    // Remove/hand clearing precedes placement. If placement fails before a
    // world allocation retains this item, the local hand pin could otherwise
    // be its final owner under the caller's character/WORLD guard.
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_owner(pz_entity::EntityOwner::from_ref(item));
    crate::component_host::retire_components(retired);
    if let Some(owner) = sq.owner_slot() {
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(owner.clone());
        crate::component_host::retire_components(retired);
    }
    let x = world.rand_float(0.1, 0.9)?;
    let y = world.rand_float(0.1, 0.9)?;
    let z = world.apparent_z_on(sq, x, y)? - sq.position().2 as f32;
    world.add_world_inventory_item_on(sq, item, x, y, z)?;
    Ok(Dropped {
        square: sq.position(),
        x,
        y,
        z,
    })
}

/// `getSolidFloorAt(sq.x, sq.y, sq.z)` after the caller's own
/// current-square gate. The forced worn-item branch checks its required
/// receiver separately because native code dereferences a null square.
fn floor_under(chr: &dyn DropCharacter, world: &dyn FloorWorld) -> HostResult<Option<FloorSquare>> {
    let Some((x, y, z)) = chr.current_square() else {
        return Ok(None);
    };
    let target = world.solid_floor_ref_at(x, y, z)?;
    if let Some(owner) = target.as_ref().and_then(FloorSquare::owner_slot) {
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(owner.clone());
        crate::component_host::retire_components(retired);
    }
    Ok(target)
}

/// The `forceDropTooHeavy` branch of `setWornItem(location, item,
/// forceDropTooHeavy)`, run after `wornItems.setItem` with `item_cur` the
/// item that was at the location: a player without room for it in its
/// inventory (`hasRoomFor`) drops it on the solid floor under its square
/// (`Remove`, then [`drop_on_floor`]). Returns where it went.
///
/// `setWornItem(location, item)` and `removeWornItem(item)` pass `true`.
pub fn force_drop_too_heavy(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    force_drop_too_heavy: bool,
    item_cur: Option<&ItemRef>,
) -> HostResult<Option<Dropped>> {
    let Some(item_cur) = item_cur else {
        return Ok(None);
    };
    if !force_drop_too_heavy || !chr.is_player() || has_room_for(chr, item_cur) {
        return Ok(None);
    }
    if chr.current_square().is_none() {
        // setWornItem(bool) PC125 dereferences the required square's x.
        // Its JVM-generated NullPointerException diagnostic is not ported.
        return Err(RuntimeHostError::Unsupported {
            operation: "forceDropTooHeavy null current-square diagnostic",
            kind: None,
        });
    }
    let Some(sq) = floor_under(chr, world)? else {
        return Ok(None);
    };
    try_remove(chr, item_cur)?;
    drop_on_floor(world, &sq, item_cur).map(Some)
}

/// One hand item dropping in `dropHeavyItems` / `dropHandItems`.
fn drop_hand(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    sq: &FloorSquare,
    item: &ItemRef,
) -> HostResult<Dropped> {
    try_do_remove_item(&chr.inventory(), item)?;
    let d = drop_on_floor(world, sq, item)?;
    chr.on_container_update();
    chr.on_item_fall(item);
    chr.play_drop_item_sound(item);
    Ok(d)
}

/// The hand items of `dropHeavyItems` (`heavy`) or `dropHandItems`.
fn drop_hands(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    heavy: bool,
) -> HostResult<Vec<Dropped>> {
    let mut out = Vec::new();
    if chr.current_square().is_none() {
        return Ok(out);
    }
    let item1 = chr.primary_hand_item();
    let item2 = chr.secondary_hand_item();
    if item1.is_none() && item2.is_none() {
        return Ok(out);
    }
    let Some(sq) = floor_under(chr, world)? else {
        return Ok(out);
    };
    let drops = |i: &Option<ItemRef>| {
        if heavy {
            is_heavy_item(i.as_ref().map(|i| i.borrow()).as_deref())
        } else {
            i.is_some()
        }
    };
    if drops(&item1) {
        let i = item1.as_ref().unwrap();
        chr.set_primary_hand_item(None);
        out.push(drop_hand(chr, world, &sq, i)?);
    }
    if drops(&item2) {
        let i = item2.as_ref().unwrap();
        chr.set_secondary_hand_item(None);
        let both = item1.as_ref().is_some_and(|a| Rc::ptr_eq(a, i));
        if !both {
            out.push(drop_hand(chr, world, &sq, i)?);
        }
    }
    Ok(out)
}

/// `dropHeavyItems()`: the heavy hand items ([`is_heavy_item`]) leave the
/// hands and the inventory for the solid floor under the character
/// (`OnContainerUpdate`, `onItemFall` and the drop sound after each; an
/// item in both hands drops once). Returns the drops in order.
pub fn drop_heavy_items(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
) -> HostResult<Vec<Dropped>> {
    drop_hands(chr, world, true)
}

/// `dropHandItems()`: nothing in the `Tutorial` game mode or for a player
/// that is not local; else [`drop_heavy_items`], then every remaining
/// hand item drops the same way.
pub fn drop_hand_items(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    tutorial: bool,
) -> HostResult<Vec<Dropped>> {
    if tutorial || chr.is_player() && !chr.is_local_player() {
        return Ok(Vec::new());
    }
    let mut out = drop_heavy_items(chr, world)?;
    out.extend(drop_hands(chr, world, false)?);
    Ok(out)
}

#[cfg(test)]
mod tests;

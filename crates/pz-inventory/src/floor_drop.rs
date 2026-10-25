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

use crate::item::{InventoryItem, ItemKind};
use crate::shared::{self, ContainerRef, ItemRef, SharedContainer};
use crate::template::ItemClass;

/// A square position `(x, y, z)`.
pub type SquarePos = (i32, i32, i32);

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
    /// `IsoGameCharacter.getSolidFloorAt(x, y, z)`: the first square from
    /// `z` down to 0 that exists and `TreatAsSolidFloor()`.
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> Option<SquarePos>;
    /// `getApparentZ(dx, dy)` of the square at `sq`.
    fn apparent_z(&self, sq: SquarePos, dx: f32, dy: f32) -> f32;
    /// `Rand.Next(min, max)` (floats).
    fn rand_float(&mut self, min: f32, max: f32) -> f32;
    /// `sq.AddWorldInventoryItem(item, x, y, height)` (the item has left
    /// its container already).
    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    );
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
    chr.remove_from_hands(item);
    let inv = chr.inventory();
    if inv.borrow().index_of(item).is_none() {
        return false;
    }
    on_before_remove_from_container(item);
    shared::remove_item(&inv, item)
}

/// `ItemContainer.DoRemoveItem(item)` of a container with a parent:
/// `drawDirty` and `dirty` are set and the item, when there, leaves it.
pub fn do_remove_item(c: &ContainerRef, item: &ItemRef) {
    {
        let mut s = c.borrow_mut();
        s.draw_dirty = true;
        s.dirty = true;
    }
    on_before_remove_from_container(item);
    let i = c.borrow().index_of(item);
    if let Some(i) = i {
        c.borrow_mut().items.remove(i);
    }
    shared::set_container(item, None);
}

/// The drop the four call sites share: `Rand.Next(0.1f, 0.9f)` for x
/// then y, `sq.getApparentZ(x, y) - sq.z` and
/// `sq.AddWorldInventoryItem(item, x, y, z)`.
pub fn drop_on_floor(world: &mut dyn FloorWorld, sq: SquarePos, item: &ItemRef) -> Dropped {
    let x = world.rand_float(0.1, 0.9);
    let y = world.rand_float(0.1, 0.9);
    let z = world.apparent_z(sq, x, y) - sq.2 as f32;
    world.add_world_inventory_item(sq, item, x, y, z);
    Dropped {
        square: sq,
        x,
        y,
        z,
    }
}

/// `getSolidFloorAt(sq.x, sq.y, sq.z)` of the current square (`None`
/// without one, where the original throws for `setWornItem`).
fn floor_under(chr: &dyn DropCharacter, world: &dyn FloorWorld) -> Option<SquarePos> {
    let (x, y, z) = chr.current_square()?;
    world.solid_floor_at(x, y, z)
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
) -> Option<Dropped> {
    let item_cur = item_cur?;
    if !force_drop_too_heavy || !chr.is_player() || has_room_for(chr, item_cur) {
        return None;
    }
    let sq = floor_under(chr, world)?;
    remove(chr, item_cur);
    Some(drop_on_floor(world, sq, item_cur))
}

/// One hand item dropping in `dropHeavyItems` / `dropHandItems`.
fn drop_hand(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    sq: SquarePos,
    item: &ItemRef,
) -> Dropped {
    do_remove_item(&chr.inventory(), item);
    let d = drop_on_floor(world, sq, item);
    chr.on_container_update();
    chr.on_item_fall(item);
    chr.play_drop_item_sound(item);
    d
}

/// The hand items of `dropHeavyItems` (`heavy`) or `dropHandItems`.
fn drop_hands(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    heavy: bool,
) -> Vec<Dropped> {
    let mut out = Vec::new();
    if chr.current_square().is_none() {
        return out;
    }
    let item1 = chr.primary_hand_item();
    let item2 = chr.secondary_hand_item();
    if item1.is_none() && item2.is_none() {
        return out;
    }
    let Some(sq) = floor_under(chr, world) else {
        return out;
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
        out.push(drop_hand(chr, world, sq, i));
    }
    if drops(&item2) {
        let i = item2.as_ref().unwrap();
        chr.set_secondary_hand_item(None);
        let both = item1.as_ref().is_some_and(|a| Rc::ptr_eq(a, i));
        if !both {
            out.push(drop_hand(chr, world, sq, i));
        }
    }
    out
}

/// `dropHeavyItems()`: the heavy hand items ([`is_heavy_item`]) leave the
/// hands and the inventory for the solid floor under the character
/// (`OnContainerUpdate`, `onItemFall` and the drop sound after each; an
/// item in both hands drops once). Returns the drops in order.
pub fn drop_heavy_items(chr: &mut dyn DropCharacter, world: &mut dyn FloorWorld) -> Vec<Dropped> {
    drop_hands(chr, world, true)
}

/// `dropHandItems()`: nothing in the `Tutorial` game mode or for a player
/// that is not local; else [`drop_heavy_items`], then every remaining
/// hand item drops the same way.
pub fn drop_hand_items(
    chr: &mut dyn DropCharacter,
    world: &mut dyn FloorWorld,
    tutorial: bool,
) -> Vec<Dropped> {
    if tutorial || chr.is_player() && !chr.is_local_player() {
        return Vec::new();
    }
    let mut out = drop_heavy_items(chr, world);
    out.extend(drop_hands(chr, world, false));
    out
}

#[cfg(test)]
mod tests;

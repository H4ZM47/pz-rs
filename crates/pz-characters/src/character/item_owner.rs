//! The character as the holder of its items, for `pz-inventory`'s
//! gameplay methods: [`ItemOwner`] for [`GameCharacter`] (what the
//! original reads through `item.getContainer().getParent()` when that is
//! an `IsoGameCharacter`/`IsoPlayer`), [`CharacterItemContext`] (the
//! [`ItemContext`] over a [`SimContext`] and the scripts), and applying
//! a `Use` outcome to the character's inventory and hands as the
//! original's `Use` does. Also `IHumanVisual` ([`HumanVisualOwner`]).
//!
//! Items are the ones directly in the shared `inventory` (the hand and
//! worn items are ids of those); an item inside a bag in the inventory has
//! a container without a parent.

use std::rc::Rc;

use pz_inventory::context::{ItemPlace, ItemOwner};
use pz_inventory::env::ItemEnv;
use pz_inventory::gameplay::UseOutcome;
use pz_inventory::visual::human::HumanVisualOwner;
use pz_inventory::{InventoryItem, ItemContainer, ItemContext, ItemKind, ItemTemplate};
use pz_scripts::objects::items::find_item;
use pz_scripts::ScriptManager;

use super::context::{SimContext, SimEvent};
use super::items::CharacterItems;
use super::{CharacterKind, GameCharacter};
use crate::survivor::CharacterGender;

/// Whether the item `id` is somewhere in `c` (directly or in a bag).
fn contains_anywhere(c: &ItemContainer, id: i32) -> bool {
    c.items.iter().filter_map(|i| i.as_item()).any(|i| {
        i.id == id
            || match &i.kind {
                ItemKind::Container(b) => contains_anywhere(&b.container, id),
                _ => false,
            }
    })
}

impl CharacterItems {
    /// The item is directly in `inventory` (its container's parent is
    /// the character).
    pub fn holds_directly(&self, id: i32) -> bool {
        self.item(id).is_some()
    }

    /// The item is in `inventory` or a bag in it.
    pub fn holds(&self, id: i32) -> bool {
        self.inventory.borrow().items.iter().any(|i| {
            let i = i.borrow();
            i.id == id || matches!(&i.kind, ItemKind::Container(b) if contains_anywhere(&b.container, id))
        })
    }
}

/// `IsoGameCharacter implements IHumanVisual` (`IsoPlayer`, `IsoZombie`).
impl<I> HumanVisualOwner for GameCharacter<I> {
    /// `isFemale()`: the descriptor's gender, `FEMALE` without one.
    fn is_female(&self) -> bool {
        self.descriptor
            .as_ref()
            .is_none_or(|d| d.borrow().character_gender == CharacterGender::Female)
    }
    fn is_zombie(&self) -> bool {
        GameCharacter::is_zombie(self)
    }
    /// `IsoPlayer.isSkeleton()` is false; `IsoZombie.isSkeleton()` its
    /// flag (the debug `forceSkeleton` option is not modelled).
    fn is_skeleton(&self) -> bool {
        self.kind == CharacterKind::Zombie && self.skeleton
    }
}

/// The character as the parent of its inventory.
impl<I> ItemOwner for GameCharacter<I> {
    /// `isEquipped()`: in the inventory and `chr.isEquipped(item)` (worn
    /// or in a hand).
    fn is_equipped(&self, item: &InventoryItem) -> bool {
        self.items.holds_directly(item.id) && self.items.is_equipped(item.id)
    }
    fn is_primary_hand_item(&self, item: &InventoryItem) -> bool {
        self.items.primary_hand_item == Some(item.id) && self.items.holds_directly(item.id)
    }
    fn is_secondary_hand_item(&self, item: &InventoryItem) -> bool {
        self.items.secondary_hand_item == Some(item.id) && self.items.holds_directly(item.id)
    }
    /// `Clothing.isWorn()`.
    fn is_worn(&self, item: &InventoryItem) -> bool {
        self.items.holds_directly(item.id) && self.items.is_worn(item.id)
    }
    /// `isInPlayerInventory()`: the outermost container is an
    /// `IsoPlayer`'s inventory.
    fn is_in_player_inventory(&self, item: &InventoryItem) -> bool {
        self.is_player() && self.items.holds(item.id)
    }
    /// `getOutermostContainer() == getContainer()` (both null for an item
    /// the character does not hold).
    fn is_in_outermost_container(&self, item: &InventoryItem) -> bool {
        !self.items.holds(item.id) || self.items.holds_directly(item.id)
    }
    fn has_container(&self, item: &InventoryItem) -> bool {
        self.items.holds(item.id)
    }
    /// `Clothing.getWetDryState()`'s view of the item's container.
    fn place(&self, item: &InventoryItem) -> ItemPlace {
        if self.items.holds_directly(item.id) {
            let env = &self.environment;
            ItemPlace::Character {
                sheltered_square: env
                    .has_current_square
                    .then_some(env.square_in_room || env.square_have_roof),
                // `getBed().isTent()` (the "Tent"/"Shelter" bed names are
                // not known here).
                in_tent: (self.asleep || self.is_resting) && env.bed_is_tent,
                // A roofed seat and a windshield window that is still
                // there (`isHittable`).
                in_roofed_vehicle: env.in_vehicle
                    && env.vehicle_has_roof
                    && !env.vehicle_windshield_destroyed,
            }
        } else if self.items.holds(item.id) {
            ItemPlace::NoParent
        } else {
            ItemPlace::Nowhere
        }
    }
    /// `ItemContainer.getTemperature()` of a character's inventory (no
    /// custom temperature, not a fridge or stove).
    fn container_temperature(&self) -> f32 {
        1.0
    }
}

/// `ScriptManager.FindItem(name).actualWeight` for item creation.
struct ScriptWeights<'a>(&'a ScriptManager);

impl ItemEnv for ScriptWeights<'_> {
    fn script_actual_weight(&self, name: &str) -> Option<f32> {
        find_item(self.0, name).map(|s| s.borrow().actual_weight)
    }
}

/// The game state the item methods read, from the simulation's
/// singletons and the scripts.
pub struct CharacterItemContext<'a> {
    pub sm: &'a ScriptManager,
    pub ctx: &'a dyn SimContext,
}

impl<'a> CharacterItemContext<'a> {
    pub fn new(sm: &'a ScriptManager, ctx: &'a dyn SimContext) -> Self {
        CharacterItemContext { sm, ctx }
    }
}

impl ItemContext for CharacterItemContext<'_> {
    fn world_age_hours(&self) -> f64 {
        self.ctx.world_age_hours()
    }
    fn multiplier(&self) -> f32 {
        self.ctx.multiplier()
    }
    fn minutes(&self) -> i32 {
        self.ctx.minutes()
    }
    fn rain_intensity(&self) -> f32 {
        self.ctx.rain_intensity()
    }
    fn is_raining(&self) -> bool {
        self.ctx.is_raining()
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        self.ctx.zomboid_globals().equipped_or_worn_encumbrance_multiplier
    }
    fn basic_mask_defense(&self) -> f32 {
        self.ctx.zomboid_globals().basic_mask_defense
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        self.ctx.zomboid_globals().improvised_gas_mask_defense
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        self.ctx.zomboid_globals().standard_gas_mask_defense
    }
    fn script(&self, name: &str) -> Option<Rc<ItemTemplate>> {
        let s = find_item(self.sm, name)?;
        let s = s.borrow();
        ItemTemplate::from_script(self.sm, &s)
    }
    /// `InventoryItemFactory.CreateItem(fullType)`: the id is
    /// `Rand.Next(2146250223) + 1233423`. STUB: the registry id is 0 (no
    /// `WorldDictionary`), and creation draws nothing else (no visuals).
    fn create_item(&self, full_type: &str) -> Option<InventoryItem> {
        let t = self.script(full_type)?;
        let mut item = pz_inventory::env::create_from_template(&ScriptWeights(self.sm), t, 0);
        item.id = self.ctx.rand_next(2146250223) + 1233423;
        Some(item)
    }
    fn is_client(&self) -> bool {
        self.ctx.is_client()
    }
}

impl<I> GameCharacter<I> {
    /// Runs `f` on the inventory item `id` with this character as its
    /// holder (on a copy, written back to the same item object after, so
    /// the owner's queries can read the inventory meanwhile).
    pub fn with_held_item<R>(&mut self, id: i32, f: impl FnOnce(&mut InventoryItem, &Self) -> R) -> Option<R> {
        let shared = self.items.item(id)?;
        let mut item = shared.borrow().clone();
        let r = f(&mut item, self);
        *shared.borrow_mut() = item;
        Some(r)
    }

    /// What the original's `Use` does to the container and the character
    /// for the inventory item `id`: `Clothing.Use`'s `Unwear()`, the
    /// `ReplaceOnUse`/`ReplaceOnDeplete` item added (taking the used
    /// item's hands for a drainable), `removeFromHands` and the removal
    /// from the inventory. Returns the new item's id.
    pub fn apply_use_outcome(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, id: i32, out: &mut UseOutcome) -> Option<i32> {
        if out.unwear && self.items.is_worn(id) {
            // `removeWornItem(this)` (no hair styles: see `set_worn_item`),
            // then `OnClothingUpdated` for a player.
            if self.remove_worn_item(sm, None, Some(id), true).is_ok() && self.is_player() {
                ctx.event(SimEvent::ClothingUpdated);
            }
        }
        let items = &mut self.items;
        let new_id = out.added.as_ref().filter(|a| a.item.is_some()).map(|a| (a.item.as_ref().unwrap().id, a.takes_hands));
        if let Some((new_id, true)) = new_id {
            if items.primary_hand_item == Some(id) {
                items.primary_hand_item = Some(new_id);
            }
            if items.secondary_hand_item == Some(id) {
                items.secondary_hand_item = Some(new_id);
            }
        }
        if out.remove_from_hands {
            // `chr.removeFromHands(item)` (a used-up `HandWeapon` leaves the
            // container but stays in the hand, as in the original).
            if items.primary_hand_item == Some(id) {
                items.primary_hand_item = None;
            }
            if items.secondary_hand_item == Some(id) {
                items.secondary_hand_item = None;
            }
        }
        let temperature = ItemOwner::container_temperature(self);
        pz_inventory::shared::apply_use(&self.items.inventory, id, out, temperature)
    }

    /// `item.Use(crafting, inContainer, needSync)` for the inventory item
    /// `id`, applied ([`Self::apply_use_outcome`]).
    pub fn use_held_item(
        &mut self,
        sm: &ScriptManager,
        ctx: &dyn SimContext,
        id: i32,
        crafting: bool,
        in_container: bool,
        need_sync: bool,
    ) -> Option<UseOutcome> {
        let ictx = CharacterItemContext::new(sm, ctx);
        let mut out = self.with_held_item(id, |item, owner| {
            item.use_with(crafting, in_container, need_sync, owner, &ictx)
        })?;
        self.apply_use_outcome(sm, ctx, id, &mut out);
        Some(out)
    }
}

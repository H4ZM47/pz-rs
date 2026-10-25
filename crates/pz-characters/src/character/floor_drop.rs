//! The character's items falling to the floor through `pz-inventory`'s
//! [`pz_inventory::floor_drop`]: [`CharacterDrop`] is the
//! [`DropCharacter`] over a [`GameCharacter`] (with the scripts and the
//! simulation context its weights and events need), and [`FloorCx`] the
//! world the items fall into. `setWornItem`'s `forceDropTooHeavy`
//! ([`GameCharacter::set_worn_item`]), `dropHeavyItems` and
//! `dropHandItems` go through them.
//!
//! Single player: `GameClient.client` is false (a client's drops are
//! the server's), and the `PlayerDropHeldItems`/`Equip` packets are not
//! sent.

use pz_inventory::floor_drop::{self, DropCharacter, Dropped, FloorWorld};
use pz_inventory::shared::{ContainerRef, ItemRef};
use pz_inventory::ItemKind;
use pz_scripts::objects::items::find_item;
use pz_scripts::ScriptManager;

use super::context::{SimContext, SimEvent};
use super::item_owner::CharacterItemContext;
use super::GameCharacter;

/// The world a character's items drop into, with the simulation context
/// the drop reads (`GameClient.client`, the weights, the events).
pub struct FloorCx<'a> {
    pub ctx: &'a dyn SimContext,
    pub floor: &'a mut dyn FloorWorld,
}

impl<'a> FloorCx<'a> {
    pub fn new(ctx: &'a dyn SimContext, floor: &'a mut dyn FloorWorld) -> Self {
        FloorCx { ctx, floor }
    }
}

/// A [`GameCharacter`] as the character whose items drop.
pub struct CharacterDrop<'c, I: 'static> {
    pub chr: &'c mut GameCharacter<I>,
    pub sm: &'c ScriptManager,
    pub ctx: &'c dyn SimContext,
}

impl<'c, I: 'static> CharacterDrop<'c, I> {
    pub fn new(chr: &'c mut GameCharacter<I>, sm: &'c ScriptManager, ctx: &'c dyn SimContext) -> Self {
        CharacterDrop { chr, sm, ctx }
    }
}

/// `item.getDropSound()`: the script's `DropSound`.
///
/// STUB: an animal corpse's breed `put_down_corpse` sound is not read.
fn drop_sound(sm: &ScriptManager, item: &ItemRef) -> Option<String> {
    let i = item.borrow();
    match &i.template.script {
        Some(s) => s.0.borrow().drop_sound.clone(),
        None => find_item(sm, &i.get_full_type()).and_then(|s| s.borrow().drop_sound.clone()),
    }
}

impl<I: 'static> DropCharacter for CharacterDrop<'_, I> {
    fn inventory(&self) -> ContainerRef {
        self.chr.items.inventory.clone()
    }
    fn is_player(&self) -> bool {
        self.chr.is_player()
    }
    /// `IsoPlayer.isLocalPlayer()`: not on a server, and one of
    /// `IsoPlayer.players`.
    fn is_local_player(&self) -> bool {
        !self.ctx.is_server() && self.chr.in_players
    }
    fn is_in_vehicle(&self) -> bool {
        self.chr.path.vehicle.is_some()
    }
    fn current_square(&self) -> Option<(i32, i32, i32)> {
        self.chr.path.current_square
    }
    fn capacity_weight(&self) -> f32 {
        self.chr.capacity_weight(self.sm, self.ctx)
    }
    /// `item.getUnequippedWeight()` with the character as its holder.
    fn unequipped_weight(&self, item: &ItemRef) -> f32 {
        let ictx = CharacterItemContext::new(self.sm, self.ctx);
        item.borrow().get_unequipped_weight(&ictx, &*self.chr)
    }
    fn primary_hand_item(&self) -> Option<ItemRef> {
        self.chr.items.primary_hand_item()
    }
    fn secondary_hand_item(&self) -> Option<ItemRef> {
        self.chr.items.secondary_hand_item()
    }
    fn set_primary_hand_item(&mut self, item: Option<&ItemRef>) {
        let id = item.map(|i| i.borrow().id);
        self.chr.set_primary_hand_item(self.sm, id);
    }
    fn set_secondary_hand_item(&mut self, item: Option<&ItemRef>) {
        let id = item.map(|i| i.borrow().id);
        self.chr.set_secondary_hand_item(self.sm, id);
    }
    fn on_container_update(&mut self) {
        self.ctx.event(SimEvent::ContainerUpdate);
    }
    fn on_item_fall(&mut self, item: &ItemRef) {
        self.ctx.event(SimEvent::ItemFall(item.clone()));
    }
    /// `playDropItemSound(item)`: the drop sound, `DropBag` for a bag
    /// without one.
    fn play_drop_item_sound(&mut self, item: &ItemRef) {
        let sound = drop_sound(self.sm, item)
            .or_else(|| matches!(item.borrow().kind, ItemKind::Container(_)).then(|| "DropBag".to_string()));
        if let Some(s) = sound {
            self.ctx.event(SimEvent::PlaySound(s));
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// The `forceDropTooHeavy` branch of `setWornItem` for the item that
    /// was at the location (`item_cur`, an inventory item id): a player
    /// with no room for it drops it on the floor
    /// ([`floor_drop::force_drop_too_heavy`]; not on a client).
    pub(crate) fn force_drop_too_heavy(
        &mut self,
        sm: &ScriptManager,
        cx: FloorCx<'_>,
        force: bool,
        item_cur: Option<i32>,
    ) -> Option<Dropped> {
        if cx.ctx.is_client() {
            return None;
        }
        let item = item_cur.and_then(|id| self.items.item(id))?;
        let mut chr = CharacterDrop::new(self, sm, cx.ctx);
        floor_drop::force_drop_too_heavy(&mut chr, cx.floor, force, Some(&item))
    }

    /// `dropHeavyItems()`: the heavy hand items (bags, `HeavyItem`s,
    /// corpses, animals, generators) fall to the floor under the
    /// character ([`floor_drop::drop_heavy_items`]).
    pub fn drop_heavy_items(&mut self, sm: &ScriptManager, cx: FloorCx<'_>) -> Vec<Dropped> {
        let mut chr = CharacterDrop::new(self, sm, cx.ctx);
        floor_drop::drop_heavy_items(&mut chr, cx.floor)
    }

    /// `dropHandItems()`: nothing in the `Tutorial` game mode or for a
    /// player that is not local; else the heavy hand items, then the rest
    /// ([`floor_drop::drop_hand_items`]).
    pub fn drop_hand_items(&mut self, sm: &ScriptManager, cx: FloorCx<'_>) -> Vec<Dropped> {
        let tutorial = cx.ctx.is_tutorial();
        let mut chr = CharacterDrop::new(self, sm, cx.ctx);
        floor_drop::drop_hand_items(&mut chr, cx.floor, tutorial)
    }
}

#[cfg(test)]
pub(crate) mod tests;

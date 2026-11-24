//! `IsoZombie.DoZombieInventory()`: what a killed zombie leaves in its
//! inventory for the corpse.

use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::worn_items::{add_items_to_item_container, set_from_item_visuals, WornItems};
use pz_inventory::api::tags;
use pz_inventory::shared::ItemRef;
use pz_inventory::InventoryItem;
use pz_kahlua::Thread;
use pz_scripts::{ScriptError, ScriptResult};

use crate::bind_character_sim::LuaGameCharacter;
use crate::world_mannequins::InventoryHost;

/// `IsoZombie.DoZombieInventory(false)` in single player (`onKilled` when
/// not a client): the worn items are made from the outfit's item visuals
/// and put in the emptied inventory with the attached items.
///
/// Not ported: `setSourceGrid` (the live container has no source grid) and
/// the server's `dressInPersistentOutfitID`. `itemsToSpawnAtDeath` are named
/// after the owner when flagged and placed into the zombie's container.
pub fn do_zombie_inventory(thread: &mut Thread, z: &mut LuaGameCharacter) -> ScriptResult<()> {
    if z.zombie.reanimated_player || z.zombie.was_fake_dead {
        return Ok(());
    }
    let Some(group) = z.items.worn_items.as_ref().map(|w| w.body_location_group().clone()) else {
        return Err(ScriptError::new("java.lang.NullPointerException: IsoZombie.wornItems"));
    };
    // `getInventory().removeAllItems()`.
    let container = z.items.inventory.clone();
    let old = {
        let mut c = container.try_borrow_mut().map_err(|_| ScriptError::new("ItemContainer is borrowed"))?;
        (std::mem::take(&mut c.items), std::mem::take(&mut c.raw_items))
    };
    for item in &old.0 {
        pz_inventory::shared::set_container(item, None);
    }
    drop(old);
    // `wornItems.setFromItemVisuals(itemVisuals)`,
    // `wornItems.addItemsToItemContainer(getInventory())`.
    let worn = Rc::new(RefCell::new(WornItems::<ItemRef>::new(group)));
    let visuals = z.zombie.item_visuals.clone();
    let mut host = InventoryHost(thread);
    set_from_item_visuals(&worn, &visuals, &mut host)
        .and_then(|_| add_items_to_item_container(&worn, &container, &mut host))
        .map_err(|e| ScriptError::new(e.to_string()))?;
    let ids = worn
        .borrow()
        .try_map_items(|item| item.try_borrow().map(|i| i.id))
        .map_err(|_| ScriptError::new("InventoryItem is borrowed"))?;
    z.items.worn_items = Some(ids);
    // The attached items not already in the inventory: named after the
    // zombie when their script asks, then added without `AddItem`.
    let names = z.descriptor.as_ref().map(|d| {
        let d = d.borrow();
        (d.forename.clone(), d.surname.clone())
    });
    for (_, item) in &z.zombie.attached_items {
        if container.borrow().items.iter().any(|i| i.borrow().id == item.id) {
            continue;
        }
        let mut item = item.clone();
        name_after_owner(&mut item, names.as_ref())?;
        let item: ItemRef = Rc::new(RefCell::new(item));
        pz_inventory::shared::set_container(&item, Some(&container));
        container.borrow_mut().items.push(item);
    }
    for mut item in z.zombie.items_to_spawn_at_death.drain(..) {
        if container.borrow().items.iter().any(|i| i.borrow().id == item.id) {
            continue;
        }
        name_after_owner(&mut item, names.as_ref())?;
        let item: ItemRef = Rc::new(RefCell::new(item));
        pz_inventory::shared::set_container(&item, Some(&container));
        container.borrow_mut().items.push(item);
    }
    Ok(())
}

/// `nameAfterDescriptor` for `ApplyOwnerName`, else `monogramAfterDescriptor`
/// for `MonogramOwnerName`, when the zombie has a descriptor.
fn name_after_owner(item: &mut InventoryItem, names: Option<&(Option<String>, Option<String>)>) -> ScriptResult<()> {
    let Some((fore, sur)) = names else {
        return Ok(());
    };
    let apply = item.has_tag(tags::APPLY_OWNER_NAME);
    if !apply && !item.has_tag(tags::MONOGRAM_OWNER_NAME) {
        return Ok(());
    }
    let display = crate::bind_items::script_display_name(&item.get_full_type())
        .ok_or_else(|| ScriptError::new("java.lang.NullPointerException: script item"))?;
    let display = crate::bind_text::translate(&display);
    if apply {
        item.name_after_descriptor(&display, fore.as_deref(), sur.as_deref());
    } else {
        item.monogram_after_descriptor(&display, fore.as_deref(), sur.as_deref())
            .map_err(ScriptError::new)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Invented zombies, item ids and item types.
    use super::*;
    use pz_characters::{CharacterKind, Characters, GameCharacter};
    use pz_inventory::{ItemClass, ItemTemplate};
    use pz_kahlua::TableRef;
    use pz_scripts::ScriptManager;

    fn item(id: i32) -> InventoryItem {
        let mut item = InventoryItem::create(Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Fixture")), 1);
        item.id = id;
        item
    }

    /// A zombie with item 5 in its inventory and item 7 attached.
    fn zombie() -> LuaGameCharacter {
        let sm = ScriptManager::new();
        let mut chars = Characters::default();
        let mut z: LuaGameCharacter = GameCharacter::new(&sm, CharacterKind::Zombie);
        z.init_worn_items(&sm, chars.body_locations.get_group("Human").unwrap());
        pz_inventory::shared::try_add_item(&z.items.inventory.clone(), &Rc::new(RefCell::new(item(5)))).unwrap();
        z.zombie.attached_items.push(("Belt Left".into(), item(7)));
        z
    }

    fn ids(z: &LuaGameCharacter) -> Vec<i32> {
        z.items.inventory.borrow().items.iter().map(|i| i.borrow().id).collect()
    }

    #[test]
    fn attached_items_go_into_the_emptied_inventory() {
        let mut z = zombie();
        let old = z.items.inventory.borrow().items[0].clone();

        do_zombie_inventory(&mut Thread::new(TableRef::new()), &mut z).unwrap();

        assert_eq!(ids(&z), vec![7]);
        assert!(pz_inventory::shared::container_of(&old).is_none());
        let attached = z.items.inventory.borrow().items[0].clone();
        assert!(pz_inventory::shared::container_of(&attached).is_some_and(|c| Rc::ptr_eq(&c, &z.items.inventory)));
        assert!(z.items.worn_items.as_ref().unwrap().items().is_empty());
    }

    #[test]
    fn fake_dead_zombie_keeps_its_inventory() {
        let mut z = zombie();
        z.zombie.was_fake_dead = true;

        do_zombie_inventory(&mut Thread::new(TableRef::new()), &mut z).unwrap();

        assert_eq!(ids(&z), vec![5]);
    }

    #[test]
    fn items_to_spawn_at_death_are_spawned_into_inventory() {
        let mut z = zombie();
        z.zombie.items_to_spawn_at_death.push(item(9));

        do_zombie_inventory(&mut Thread::new(TableRef::new()), &mut z).unwrap();

        assert!(z.zombie.items_to_spawn_at_death.is_empty());
        assert_eq!(ids(&z), vec![7, 9]);
    }
}

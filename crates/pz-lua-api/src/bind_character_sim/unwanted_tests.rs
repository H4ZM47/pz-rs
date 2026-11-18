use super::tests::run;
use super::*;
use pz_inventory::shared;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};

#[test]
fn unwanted_items_follow_the_player_mod_data_flag() {
    let (mut thread, character, _) = tests::setup_with(ScriptManager::new());
    crate::bind_items::register(&mut thread);
    crate::bind_item_container::register(&mut thread);
    let item = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "UnwantedFixture")),
        1,
    ))
    .unwrap();
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    run(&mut thread, r#"
        local inv = player:getInventory()
        assert(inv:isEmptyOrUnwanted(player) == true, "empty")
        assert(inv:isEmptyOrUnwanted(nil) == true, "empty, null player")
        assert(player:isUnwanted("Book1") == false)
        assert(player:isUnwanted(nil) == false)
    "#).unwrap();
    let inventory = character.borrow().items.inventory.clone();
    shared::add_item(&inventory, &item);
    run(&mut thread, r#"
        local inv = player:getInventory()
        item:getModData().literatureTitle = "Book1"
        assert(item:isUnwanted(player) == false)
        assert(inv:isEmptyOrUnwanted(player) == false, "a wanted item")
        player:getModData()["itemUnwanted:Book1"] = true
        assert(player:isUnwanted("Book1") == true)
        assert(item:isUnwanted(player) == true)
        assert(inv:isEmptyOrUnwanted(player) == true, "only unwanted items")
        assert(not pcall(item.isUnwanted, item, nil), "null player")
        assert(not pcall(inv.isEmptyOrUnwanted, inv, nil), "null player with items")
        player:getModData()["itemUnwanted:Book1"] = "yes"
        assert(not pcall(player.isUnwanted, player, "Book1"), "not a Boolean")
    "#).unwrap();
}

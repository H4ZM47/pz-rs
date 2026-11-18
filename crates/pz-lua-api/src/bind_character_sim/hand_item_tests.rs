use super::tests::{run, setup_with};
use super::*;
use pz_inventory::shared;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};

#[test]
fn hand_item_checks_compare_the_hands_and_no_item_is_attached() {
    let (mut thread, character, _) = setup_with(ScriptManager::new());
    crate::bind_items::register(&mut thread);
    let inventory = character.borrow().items.inventory.clone();
    for name in ["Bat", "Pan"] {
        let item = shared::try_new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", name)),
            1,
        ))
        .unwrap();
        item.borrow_mut().id = if name == "Bat" { 1 } else { 2 };
        shared::add_item(&inventory, &item);
        thread.globals().set_str(
            &name.to_lowercase(),
            crate::bind_items::try_item_value(&item).unwrap(),
        );
    }
    run(
        &mut thread,
        r#"
        assert(not player:isHandItem(bat) and not player:isItemInBothHands(bat))
        player:setPrimaryHandItem(bat)
        assert(player:isHandItem(bat) and not player:isItemInBothHands(bat))
        assert(not player:isHandItem(pan))
        player:setSecondaryHandItem(pan)
        assert(player:isHandItem(pan) and not player:isItemInBothHands(pan))
        player:setSecondaryHandItem(bat)
        assert(player:isItemInBothHands(bat) and not player:isHandItem(pan))
        assert(not player:isHandItem(nil) and not player:isItemInBothHands(nil))
        assert(not player:isAttachedItem(bat) and not player:isAttachedItem(nil))
        "#,
    )
    .unwrap();
}

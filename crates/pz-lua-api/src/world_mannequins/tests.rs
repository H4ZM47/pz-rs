use super::*;
use pz_inventory::{InventoryItem, ItemClass, ItemContainer, ItemTemplate, SavedItem};
use pz_world::{IsoObject, ObjectSlot};

struct RestoreCharacters(Rc<RefCell<pz_characters::Characters>>);
impl Drop for RestoreCharacters {
    fn drop(&mut self) {
        crate::bind_characters::install_characters(self.0.clone());
    }
}

#[test]
fn loaded_inventory_keeps_actual_container_and_worn_prefix_on_bad_location() {
    let _restore = RestoreCharacters(crate::bind_characters::shared_characters());
    crate::bind_characters::install_characters(Rc::new(RefCell::new(Default::default())));
    let scripts = ScriptManager::new();
    let registry = scripts.registries.get("item_body_location").unwrap();
    let hat = registry
        .borrow_mut()
        .register_id("test:hat", false)
        .unwrap();
    registry
        .borrow_mut()
        .register_id("test:unmapped", false)
        .unwrap();
    let group = crate::bind_characters::shared_characters()
        .borrow_mut()
        .body_locations
        .get_group("Human")
        .unwrap();
    group
        .borrow_mut()
        .get_or_create_location(Some(hat.clone()))
        .unwrap();
    let mut saved = ItemContainer::new(Some("mannequin"));
    saved.explored = true;
    // The raw record occupies a codec index but has no runtime ItemRef.
    saved.items.push(SavedItem::Raw(vec![0, 1, 0, 0, 0, 0, 1]));
    for id in [31, 32] {
        let mut item = InventoryItem::create(
            Rc::new(ItemTemplate::new(
                ItemClass::Normal,
                "Base",
                "LoadedFixture",
            )),
            1,
        );
        item.id = id;
        saved.items.push(SavedItem::Item(Box::new(item)));
    }
    let mut mannequin = IsoMannequin::new(7);
    mannequin.init = true;
    mannequin.container = Some(saved);
    mannequin.worn_items = vec![
        ("test:unmapped".into(), 1), // registered, absent from Human: skip
        ("test:hat".into(), 1),
        ("".into(), 2), // invalid location: caught after a valid prefix
        ("test:hat".into(), 2),
    ];
    let object = ObjectSlot::new(IsoObject::new(
        1,
        ObjectKind::Mannequin(Box::new(mannequin)),
        None,
    ));
    let object = object.ref_handle();
    promote_loaded_inventory(&object, &scripts).unwrap();
    let (container, worn) = {
        let object = object.borrow();
        let ObjectKind::Mannequin(m) = &object.kind else {
            unreachable!()
        };
        assert!(m.container.is_none() && m.worn_items.is_empty());
        (
            m.runtime_container.as_ref().unwrap().clone(),
            m.runtime_worn_items.as_ref().unwrap().clone(),
        )
    };
    assert_eq!(container.borrow().items.len(), 2);
    assert_eq!(container.borrow().raw_items.len(), 1);
    assert!(Rc::ptr_eq(worn.borrow().body_location_group(), &group));
    assert_eq!(worn.borrow().size(), 1);
    let member = worn.borrow().get(0).unwrap().clone();
    assert!(Rc::ptr_eq(member.location.as_ref().unwrap(), &hat));
    assert!(Rc::ptr_eq(&member.item, &container.borrow().items[0]));
    assert_eq!(member.item.borrow().id, 31);
    promote_loaded_inventory(&object, &scripts).unwrap();
    let object = object.borrow();
    let ObjectKind::Mannequin(m) = &object.kind else {
        unreachable!()
    };
    assert!(Rc::ptr_eq(
        m.runtime_container.as_ref().unwrap(),
        &container
    ));
    assert!(Rc::ptr_eq(m.runtime_worn_items.as_ref().unwrap(), &worn));
}

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

#[test]
fn save_wear_checks_current_container_before_visual_lookup() {
    let group = crate::bind_characters::shared_characters().borrow_mut()
        .body_locations.get_group("Human").unwrap();
    let worn = Rc::new(RefCell::new(WornItems::new(group)));
    let container = SharedContainer::default().into_ref();
    let mut mannequin = IsoMannequin::new(7);
    drop(mannequin.install_runtime_inventory(container.clone(), worn.clone()));
    let object = ObjectSlot::new(IsoObject::new(1, ObjectKind::Mannequin(Box::new(mannequin)), None)).ref_handle();
    let item = pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "NoClothingAsset")), 1,
    ));
    item.borrow_mut().visual = Some(ItemVisual::new("Base.NoClothingAsset"));
    wear_item_for_save(&object, &item).unwrap();
    assert!(item.borrow().visual.is_some(), "non-member must not run getVisual");
    container.borrow_mut().items.push(item.clone());
    wear_item_for_save(&object, &item).unwrap();
    assert!(item.borrow().visual.is_none(), "member getVisual clears a missing asset");
    assert_eq!(worn.borrow().size(), 0);
}

#[test]
fn visual_sync_rebuilds_from_current_worn_items() {
    let group = crate::bind_characters::shared_characters().borrow_mut()
        .body_locations.get_group("Human").unwrap();
    let worn = Rc::new(RefCell::new(WornItems::new(group)));
    let container = SharedContainer::default().into_ref();
    let mut mannequin = IsoMannequin::new(7);
    mannequin.model_script_name = Some("TestModel".into());
    mannequin.texture_name = Some("TestSkin".into());
    mannequin.item_visuals.push(ItemVisual::new("Base.Stale"));
    drop(mannequin.install_runtime_inventory(container, worn));
    let object = ObjectSlot::new(IsoObject::new(1, ObjectKind::Mannequin(Box::new(mannequin)), None)).ref_handle();
    sync_visuals(&object).unwrap();
    let object = object.borrow();
    let ObjectKind::Mannequin(m) = &object.kind else { unreachable!() };
    assert!(m.item_visuals.is_empty());
    assert_eq!(m.human_visual.force_model_script.as_deref(), Some("TestModel"));
    assert_eq!(m.human_visual.skin_texture_name.as_deref(), Some("TestSkin"));
}

#[test]
fn wear_keeps_detached_container_and_visual_owner_until_engine_drain() {
    use crate::bind_items::{item_env, install_item_env, ItemLuaEnv};
    use pz_inventory::{ItemContext, ItemOwner};

    struct Restore(Rc<dyn ItemLuaEnv>);
    impl Drop for Restore {
        fn drop(&mut self) {
            install_item_env(self.0.clone());
            pz_inventory::component_host::drain_retired_components();
        }
    }

    struct Holder;
    impl ItemOwner for Holder {}
    struct DetachOnOwner {
        object: IsoObjectRef,
        holder: std::cell::Cell<Option<Rc<Holder>>>,
    }
    impl ItemLuaEnv for DetachOnOwner {
        fn context(&self) -> &dyn ItemContext {
            &pz_inventory::api::Offline
        }
        fn owner(&self, _: &ItemRef) -> Rc<dyn ItemOwner> {
            let mut body = self.object.borrow_mut();
            let ObjectKind::Mannequin(m) = &mut body.kind else { unreachable!() };
            drop(m.runtime_container.take());
            self.holder.take().unwrap()
        }
    }

    let _restore = Restore(item_env());
    let container = SharedContainer::default().into_ref();
    let item = pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "NoAsset")), 1,
    ));
    container.borrow_mut().items.push(item.clone());
    let weak_container = Rc::downgrade(&container);
    let mut mannequin = IsoMannequin::new(7);
    mannequin.runtime_container = Some(container);
    let object = ObjectSlot::new(IsoObject::new(1, ObjectKind::Mannequin(Box::new(mannequin)), None)).ref_handle();
    let holder = Rc::new(Holder);
    let weak_owner = Rc::downgrade(&holder);
    install_item_env(Rc::new(DetachOnOwner {
        object: object.clone(),
        holder: std::cell::Cell::new(Some(holder)),
    }));

    // The callback detaches the container. No visual triggers an early return.
    assert!(!wear_item_for_save(&object, &item).unwrap());
    assert!(weak_container.upgrade().is_some());
    assert!(weak_owner.upgrade().is_some());
    pz_inventory::component_host::drain_retired_components();
    assert!(weak_container.upgrade().is_none());
    assert!(weak_owner.upgrade().is_none());
}

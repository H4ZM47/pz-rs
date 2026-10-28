use super::*;
use pz_entity::{Component, Components, FluidContainer};
use pz_inventory::{ItemClass, ItemTemplate};
use pz_kahlua::Value;

struct Restore {
    item: Option<pz_inventory::shared::ItemObserver>,
    container: Option<pz_inventory::shared::ContainerObserver>,
    saved: Option<fn(&SavedInventoryListRef)>,
}
impl Restore {
    fn new() -> Self {
        pz_inventory::component_host::drain_retired_components();
        let _ = gc::collect_tables();
        Self {
            item: pz_inventory::shared::install_item_observer(Some(observe_item)),
            container: pz_inventory::shared::install_container_observer(Some(observe_container)),
            saved: pz_characters::character::save::install_saved_inventory_observer(Some(
                observe_saved_inventory,
            )),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        pz_inventory::shared::install_item_observer(self.item);
        pz_inventory::shared::install_container_observer(self.container);
        pz_characters::character::save::install_saved_inventory_observer(self.saved);
        pz_inventory::component_host::drain_retired_components();
        retry_pending();
        let _ = gc::collect_tables();
    }
}

fn graph() -> (ItemRef, ContainerRef, ItemRef, ComponentRef) {
    let bag = pz_inventory::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Container,
            "Base",
            "GcActualBag",
        )),
        1,
    ))
    .unwrap();
    let container = {
        let bag = bag.borrow();
        let ItemKind::Container(data) = &bag.kind else {
            panic!("bag fixture")
        };
        data.container.slot().unwrap().clone()
    };
    let mut child = InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Normal,
            "Base",
            "GcActualChild",
        )),
        2,
    );
    child.components =
        pz_inventory::item_components::ItemComponents::from_values_unowned(Components {
            list: vec![Component::FluidContainer(FluidContainer::default())],
        })
        .unwrap();
    let child = pz_inventory::shared::try_new_item(child).unwrap();
    pz_inventory::shared::add_item(&container, &child);
    // Each actual slot is counted, including repeated aliases and nulls in
    // the independent includingObsoleteItems field. All owners were already
    // registered by their genuine allocation observers before these backedges.
    container.borrow_mut().including_obsolete_items =
        vec![Some(child.clone()), None, Some(child.clone())];
    let component = child.borrow().get_fluid_ref().unwrap().unwrap();
    pz_inventory::component_host::drain_retired_components();
    (bag, container, child, component)
}

#[test]
fn actual_bag_child_container_and_userdata_roots_keep_all_alias_slots_then_collect() {
    let _restore = Restore::new();
    for root_kind in 0..6 {
        let (bag, container, child, component) = graph();
        let wb = Rc::downgrade(&bag);
        let wc = Rc::downgrade(&container);
        let wi = Rc::downgrade(&child);
        let wf = Rc::downgrade(&component);
        let retained: Box<dyn std::any::Any> = match root_kind {
            0 => Box::new(bag.clone()),
            1 => Box::new(container.clone()),
            2 => Box::new(child.clone()),
            3 => Box::new(crate::bind_items::try_item_value(&child).unwrap()),
            4 => Box::new(crate::bind_item_container::container_value(&container)),
            _ => Box::new(component.clone()),
        };
        drop((bag, container, child, component));
        assert!(matches!(
            gc::collect_tables(),
            gc::CollectionResult::Collected(_)
        ));
        {
            let bag = wb.upgrade().expect("retained native bag owner");
            let container = wc.upgrade().expect("retained actual child container");
            let child = wi.upgrade().expect("retained actual child item");
            let body = container.borrow();
            assert!(Rc::ptr_eq(body.containing_item.as_ref().unwrap(), &bag));
            assert!(Rc::ptr_eq(body.inventory_container.as_ref().unwrap(), &bag));
            assert!(Rc::ptr_eq(&body.items[0], &child));
            assert!(Rc::ptr_eq(
                body.including_obsolete_items[0].as_ref().unwrap(),
                &child
            ));
            assert!(body.including_obsolete_items[1].is_none());
            assert!(Rc::ptr_eq(
                body.including_obsolete_items[2].as_ref().unwrap(),
                &child
            ));
            assert!(Rc::ptr_eq(
                child.borrow().container_slot().unwrap(),
                &container
            ));
            assert!(wf.upgrade().is_some());
        }
        drop(retained);
        assert!(matches!(
            gc::collect_tables(),
            gc::CollectionResult::Collected(_)
        ));
        assert!(wb.upgrade().is_none() && wc.upgrade().is_none());
        assert!(wi.upgrade().is_none() && wf.upgrade().is_none());
    }
}

#[test]
fn retained_child_backlink_keeps_bag_after_membership_slots_are_removed() {
    let _restore = Restore::new();
    let (bag, container, child, component) = graph();
    let wb = Rc::downgrade(&bag);
    let wc = Rc::downgrade(&container);
    // Model only removal of these actual list slots. Native clear's separate
    // item.container field is not implicitly reset or rebound by this test.
    let retired = {
        let mut body = container.borrow_mut();
        (
            std::mem::take(&mut body.items),
            std::mem::take(&mut body.including_obsolete_items),
        )
    };
    drop(retired);
    drop((bag, container, component));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    {
        let container = wc.upgrade().expect("actual child retains former container");
        let bag = wb
            .upgrade()
            .expect("container's strong containingItem retains bag");
        assert!(Rc::ptr_eq(
            child.borrow().container_slot().unwrap(),
            &container
        ));
        assert!(Rc::ptr_eq(
            container.borrow().containing_item.as_ref().unwrap(),
            &bag
        ));
        assert!(container.borrow().items.is_empty());
    }
    drop(child);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(wb.upgrade().is_none() && wc.upgrade().is_none());
}

#[test]
fn held_actual_container_or_item_defers_without_changing_any_owner_slot() {
    let _restore = Restore::new();
    let (bag, container, child, component) = graph();
    {
        let _held = container.borrow_mut();
        assert_eq!(
            register_container(&container).unwrap(),
            MetadataStatus::Deferred
        );
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    {
        let _held = child.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    retry_pending();
    assert!(Rc::ptr_eq(
        child.borrow().container_slot().unwrap(),
        &container
    ));
    assert!(Rc::ptr_eq(
        container.borrow().containing_item.as_ref().unwrap(),
        &bag
    ));
    assert_eq!(container.borrow().including_obsolete_items.len(), 3);
    let weak = Rc::downgrade(&container);
    drop((bag, container, child, component));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak.upgrade().is_none());
}

#[test]
fn collecting_bag_graph_does_not_native_reset_held_actual_fluid_color() {
    let _restore = Restore::new();
    let (bag, container, child, component) = graph();
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    *color.borrow_mut() = [0.2, 0.3, 0.4, 0.8];
    let wb = Rc::downgrade(&bag);
    let wc = Rc::downgrade(&container);
    let wi = Rc::downgrade(&child);
    let wf = Rc::downgrade(&component);
    drop((bag, container, child, component));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(wb.upgrade().is_none() && wc.upgrade().is_none());
    assert!(wi.upgrade().is_none() && wf.upgrade().is_none());
    assert_eq!(*color.borrow(), [0.2, 0.3, 0.4, 0.8]);
}

#[test]
fn canonical_container_userdata_under_held_body_retries_and_cache_prunes_amortized() {
    let _restore = Restore::new();
    let (bag, container, child, component) = graph();
    let held = container.borrow_mut();
    let first = crate::bind_item_container::container_value(&container);
    let second = crate::bind_item_container::container_value(&container);
    assert!(first.raw_equals(&second));
    assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    drop(held);
    retry_pending();
    let Value::Object(object) = &first else {
        panic!("actual container userdata")
    };
    let userdata = (&**object as &dyn std::any::Any)
        .downcast_ref::<crate::bind_item_container::LuaItemContainer>()
        .unwrap();
    let cache_guard = userdata.items_list.borrow_mut();
    assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    drop(cache_guard);
    let before = crate::bind_item_container::container_cache_prune_count();
    let values: Vec<_> = (0..512)
        .map(|_| {
            let container = SharedContainer::default().into_ref();
            let first = crate::bind_item_container::container_value(&container);
            for _ in 0..8 {
                assert!(first.raw_equals(&crate::bind_item_container::container_value(&container)));
            }
            first
        })
        .collect();
    assert!(
        crate::bind_item_container::container_cache_prune_count() - before <= 8,
        "canonical cache must not rescan all live headers at every allocation/getter"
    );
    drop(values);
    let weak = Rc::downgrade(&container);
    drop((first, second, bag, container, child, component));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak.upgrade().is_none());
}

#[test]
fn fluid_owner_walk_preserves_typed_held_item_and_bag_faults() {
    let _restore = Restore::new();
    let (bag, _container, child, _component) = graph();
    {
        let _held = child.borrow_mut();
        assert!(matches!(
            crate::bind_item_container::on_fluid_update(&child),
            Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
                "item container getter"
            ))
        ));
    }
    {
        let _held = bag.borrow_mut();
        assert!(matches!(
            crate::bind_item_container::on_fluid_update(&child),
            Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
                "item container getter"
            ))
        ));
    }
}

#[test]
fn actual_saved_list_aliases_nulls_and_capture_cycle_are_traced_without_native_clear() {
    struct SavedCapture(Option<SavedInventoryListRef>);
    impl gc::TracePayload for SavedCapture {
        fn trace(&self, visitor: &mut TraceVisitor) {
            if let Some(saved) = &self.0 {
                visitor.owner(saved);
            }
        }
        fn detach(&mut self) -> Option<Box<dyn std::any::Any>> {
            Some(Box::new(self.0.take()))
        }
    }
    let _restore = Restore::new();
    let (bag, container, child, component) = graph();
    let authority = pz_characters::character::save::SavedInventoryItems::actual_empty();
    let list = authority.actual_refs().unwrap().clone();
    *list.borrow_mut() = vec![Some(child.clone()), None, Some(child.clone())];
    observe_saved_inventory(&list);
    drop(authority);
    let callback = pz_kahlua::NativeFunction::new_traced_capture(
        "gc-only-saved-list-capture",
        SavedCapture(Some(list.clone())),
        |_, _| panic!("collection must not execute the fixture callback"),
    );
    pz_inventory::moddata::set_mod_data_value_ref(
        &child,
        "savedListBackedge",
        Some(Value::Native(callback.clone())),
    )
    .unwrap();
    pz_inventory::component_host::drain_retired_components();
    let wb = Rc::downgrade(&bag);
    let wc = Rc::downgrade(&container);
    let wi = Rc::downgrade(&child);
    let wl = Rc::downgrade(&list);
    let wf = Rc::downgrade(&callback);
    drop((bag, container, child, component, callback));
    {
        let _held = list.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    {
        let body = list.borrow();
        assert_eq!(body.len(), 3);
        assert!(body[1].is_none());
        assert!(Rc::ptr_eq(
            body[0].as_ref().unwrap(),
            body[2].as_ref().unwrap()
        ));
        assert!(wb.upgrade().is_some() && wc.upgrade().is_some() && wi.upgrade().is_some());
    }
    drop(list);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(wb.upgrade().is_none() && wc.upgrade().is_none() && wi.upgrade().is_none());
    assert!(wl.upgrade().is_none() && wf.upgrade().is_none());
}

#[test]
fn distinct_inventory_container_and_containing_item_slots_retain_both_actual_bags() {
    let _restore = Restore::new();
    let (original, container, child, component) = graph();
    let other = pz_inventory::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Container,
            "Base",
            "OtherGcActualBag",
        )),
        3,
    ))
    .unwrap();
    let old = container
        .borrow_mut()
        .containing_item
        .replace(other.clone());
    drop(old);
    observe_container(&container);
    pz_inventory::component_host::drain_retired_components();
    let wo = Rc::downgrade(&original);
    let wb = Rc::downgrade(&other);
    let wc = Rc::downgrade(&container);
    drop((original, other, component));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    {
        let body = container.borrow();
        assert!(Rc::ptr_eq(
            body.inventory_container.as_ref().unwrap(),
            &wo.upgrade().unwrap()
        ));
        assert!(Rc::ptr_eq(
            body.containing_item.as_ref().unwrap(),
            &wb.upgrade().unwrap()
        ));
        assert!(!Rc::ptr_eq(
            body.inventory_container.as_ref().unwrap(),
            body.containing_item.as_ref().unwrap()
        ));
    }
    drop((container, child));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(wo.upgrade().is_none() && wb.upgrade().is_none() && wc.upgrade().is_none());
}

#[test]
fn real_dual_bag_cycle_releases_opaque_parent_only_after_user_root_and_all_host_guards() {
    struct ReentrantParent {
        dropped: Rc<std::cell::Cell<usize>>,
        sprites: Weak<RefCell<pz_world::sprite::IsoSpriteManager>>,
    }
    impl Drop for ReentrantParent {
        fn drop(&mut self) {
            crate::bind_world::with_world(|_| {
                let sprites = self.sprites.upgrade().unwrap();
                let _guard = sprites
                    .try_borrow_mut()
                    .expect("sprite guard survived collection");
                self.dropped.set(self.dropped.get() + 1);
            });
        }
    }
    let _restore = Restore::new();
    let sprites = Rc::new(RefCell::new(pz_world::sprite::IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    )));
    let dropped = Rc::new(std::cell::Cell::new(0));
    let (bag, container, child, component) = graph();
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    *color.borrow_mut() = [0.12, 0.34, 0.56, 0.78];
    container.borrow_mut().parent = Some(Rc::new(ReentrantParent {
        dropped: dropped.clone(),
        sprites: Rc::downgrade(&sprites),
    }));
    let wb = Rc::downgrade(&bag);
    let wc = Rc::downgrade(&container);
    let wi = Rc::downgrade(&child);
    let wf = Rc::downgrade(&component);
    let user_root = child.clone();
    drop((bag, container, child, component));
    pz_inventory::component_host::drain_retired_components();
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert_eq!(
        dropped.get(),
        0,
        "a retained real child keeps the whole actual bag graph"
    );
    {
        let container = wc.upgrade().unwrap();
        let body = container.borrow();
        let bag = wb.upgrade().unwrap();
        assert!(Rc::ptr_eq(body.containing_item.as_ref().unwrap(), &bag));
        assert!(Rc::ptr_eq(body.inventory_container.as_ref().unwrap(), &bag));
        assert_eq!(
            (body.items.len(), body.including_obsolete_items.len()),
            (1, 3)
        );
    }
    {
        let _held = user_root.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    drop(user_root);
    assert!(
        wb.upgrade().is_some(),
        "real Native strong edges remain until collection"
    );
    crate::bind_world::with_world(|_| {
        let _sprites = sprites.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
        assert_eq!(dropped.get(), 0);
    });
    assert_eq!(dropped.get(), 0);
    pz_inventory::component_host::drain_retired_components();
    retry_pending();
    assert!(matches!(
        gc::collect_pending(),
        gc::CollectionResult::Collected(_)
    ));
    assert_eq!(dropped.get(), 1);
    assert!(wb.upgrade().is_none() && wc.upgrade().is_none());
    assert!(wi.upgrade().is_none() && wf.upgrade().is_none());
    assert_eq!(
        *color.borrow(),
        [0.12, 0.34, 0.56, 0.78],
        "GC never invokes Native fluid reset/release"
    );
}

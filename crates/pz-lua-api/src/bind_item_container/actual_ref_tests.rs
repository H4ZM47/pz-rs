use super::*;
use pz_inventory::ItemTemplate;
use pz_kahlua::TableRef;
use pz_scripts::ScriptManager;

struct Restore {
    item: Option<shared::ItemObserver>,
    container: Option<shared::ContainerObserver>,
}
impl Restore {
    fn new() -> Self {
        Self {
            item: shared::install_item_observer(Some(crate::component_gc::observe_item)),
            container: shared::install_container_observer(Some(
                crate::component_gc::observe_container,
            )),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        shared::install_item_observer(self.item);
        shared::install_container_observer(self.container);
        pz_inventory::component_host::drain_retired_components();
        crate::component_gc::retry_pending();
        let _ = pz_kahlua::gc::collect_tables();
    }
}
fn setup() -> (Thread, ItemRef, ItemRef, ContainerRef) {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_java_util::register(&mut thread);
    crate::bind_globals::register(&mut thread);
    crate::bind_registry::register(&mut thread, Rc::new(ScriptManager::new()));
    crate::bind_items::register(&mut thread);
    register(&mut thread);
    let bag = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Container,
            "Base",
            "ActualGetterBag",
        )),
        1,
    ))
    .unwrap();
    let inventory = {
        let body = bag.borrow();
        let ItemKind::Container(data) = &body.kind else {
            panic!("bag fixture")
        };
        data.container.slot().unwrap().clone()
    };
    let child = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Normal,
            "Base",
            "ActualGetterChild",
        )),
        2,
    ))
    .unwrap();
    shared::add_item(&inventory, &child);
    thread.globals().set_str(
        "actualBag",
        crate::bind_items::try_item_value(&bag).unwrap(),
    );
    thread.globals().set_str(
        "actualChild",
        crate::bind_items::try_item_value(&child).unwrap(),
    );
    pz_inventory::component_host::drain_retired_components();
    (thread, bag, child, inventory)
}
fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual-container-getters"),
        thread.globals().clone(),
    )
    .unwrap();
    // Unwind the real compiled Lua call frame while preserving its typed error.
    thread.pcall_ex(&function, &[]).map_err(|(_, _, error)| error)
}
fn is_borrowed(result: LuaResult<Vec<Value>>) {
    assert!(
        matches!(
            result,
            Err(LuaError::Runtime {
                class: "pz.host.Borrowed",
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn actual_lua_container_getters_preserve_identity_and_typed_item_borrows() {
    let _restore = Restore::new();
    let (mut thread, bag, child, inventory) = setup();
    let expected = container_value(&inventory);
    {
        let _held = child.borrow_mut();
        is_borrowed(call(&mut thread, "return actualChild:getContainer()"));
        is_borrowed(call(
            &mut thread,
            "return actualChild:getOutermostContainer()",
        ));
    }
    {
        let _held = bag.borrow_mut();
        is_borrowed(call(&mut thread, "return actualBag:getInventory()"));
        is_borrowed(call(
            &mut thread,
            "return actualChild:getOutermostContainer()",
        ));
    }
    let results = call(&mut thread, "return actualChild:getContainer(), actualBag:getInventory(), actualChild:getOutermostContainer()").unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|value| value.raw_equals(&expected)));
}

#[test]
fn held_container_body_does_not_block_pointer_getters_but_faults_outermost_query() {
    let _restore = Restore::new();
    let (mut thread, _bag, _child, inventory) = setup();
    let expected = container_value(&inventory);
    let _held = inventory.borrow_mut();
    let results = call(
        &mut thread,
        "return actualChild:getContainer(), actualBag:getInventory()",
    )
    .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|value| value.raw_equals(&expected)));
    is_borrowed(call(
        &mut thread,
        "return actualChild:getOutermostContainer()",
    ));
}

#[test]
fn actual_null_bag_field_returns_native_null_only_for_direct_field_getter() {
    let _restore = Restore::new();
    let (mut thread, bag, _child, _inventory) = setup();
    let old = {
        let mut body = bag.borrow_mut();
        let ItemKind::Container(data) = &mut body.kind else {
            panic!("bag fixture")
        };
        data.container.take_for_gc()
    };
    let results = call(
        &mut thread,
        "return actualBag:getInventory(), actualBag:getItemContainer()",
    )
    .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(Value::is_nil));
    assert!(
        matches!(call(&mut thread, "return actualBag:getCapacity()"),
        Err(LuaError::Runtime { class: "pz.host.Unsupported", message })
        if message == "native null InventoryContainer.container receiver")
    );
    drop(old);
}

#[test]
fn direct_bag_setter_accepts_null_and_held_old_or_incoming_container_without_parent_rebinding() {
    let _restore = Restore::new();
    let (mut thread, bag, _child, original) = setup();
    let replacement = SharedContainer::default().into_ref();
    let original_value = container_value(&original);
    let replacement_value = container_value(&replacement);
    thread.globals().set_str("original", original_value.clone());
    thread
        .globals()
        .set_str("replacement", replacement_value.clone());
    assert!(call(&mut thread, "return actualBag:setItemContainer(nil)")
        .unwrap()
        .is_empty());
    assert!(call(&mut thread, "return actualBag:getInventory()").unwrap()[0].is_nil());
    {
        let held = original.borrow_mut();
        assert!(call(
            &mut thread,
            "return actualBag:setItemContainer(replacement)"
        )
        .unwrap()
        .is_empty());
        assert!(
            call(&mut thread, "return actualBag:getInventory()").unwrap()[0]
                .raw_equals(&replacement_value)
        );
        assert!(Rc::ptr_eq(held.containing_item.as_ref().unwrap(), &bag));
        assert!(Rc::ptr_eq(held.inventory_container.as_ref().unwrap(), &bag));
    }
    {
        let held = replacement.borrow_mut();
        assert!(
            call(&mut thread, "return actualBag:setItemContainer(original)")
                .unwrap()
                .is_empty()
        );
        assert!(held.containing_item.is_none() && held.inventory_container.is_none());
    }
    assert!(
        call(&mut thread, "return actualBag:getItemContainer()").unwrap()[0]
            .raw_equals(&original_value)
    );
}

struct ReentrantContainerParent {
    dropped: Rc<Cell<usize>>,
    bag: Weak<RefCell<InventoryItem>>,
    sprites: Option<Weak<RefCell<pz_world::sprite::IsoSpriteManager>>>,
}
impl Drop for ReentrantContainerParent {
    fn drop(&mut self) {
        crate::bind_world::with_world(|_| {
            let bag = self.bag.upgrade().expect("live receiver fixture");
            let _bag = bag
                .try_borrow_mut()
                .expect("receiver guard survived retirement");
            let sprites = self
                .sprites
                .as_ref()
                .map(|sprites| sprites.upgrade().unwrap());
            let _sprites = sprites.as_ref().map(|sprites| {
                sprites
                    .try_borrow_mut()
                    .expect("sprite guard survived retirement")
            });
            self.dropped.set(self.dropped.get() + 1);
        });
    }
}

#[test]
fn rejected_bag_setter_retains_last_incoming_owner_until_receiver_guard_ends() {
    let _restore = Restore::new();
    let (mut thread, bag, _child, original) = setup();
    let dropped = Rc::new(Cell::new(0));
    let replacement = SharedContainer::default().into_ref();
    replacement.borrow_mut().parent = Some(Rc::new(ReentrantContainerParent {
        dropped: dropped.clone(),
        bag: Rc::downgrade(&bag),
        sprites: None,
    }));
    let weak = Rc::downgrade(&replacement);
    thread
        .globals()
        .set_str("replacement", container_value(&replacement));
    drop(replacement);
    {
        let _held = bag.borrow_mut();
        is_borrowed(call(&mut thread, "actualBag:setItemContainer(replacement)"));
        thread.globals().set_str("replacement", Value::Nil);
        assert!(weak.upgrade().is_some());
        assert_eq!(dropped.get(), 0);
    }
    pz_inventory::component_host::drain_retired_components();
    assert!(weak.upgrade().is_none());
    assert_eq!(dropped.get(), 1);
    let body = bag.borrow();
    let ItemKind::Container(data) = &body.kind else {
        panic!("bag fixture")
    };
    assert!(Rc::ptr_eq(data.container.slot().unwrap(), &original));
}

#[test]
fn direct_bag_setter_retires_displaced_owner_after_world_sprite_and_item_guards() {
    let sprites = Rc::new(RefCell::new(pz_world::sprite::IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    )));
    let _restore = Restore::new();
    let (mut thread, _fixture_bag, _child, _fixture_inventory) = setup();
    let bag = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Container,
            "Base",
            "EmptySetterBag",
        )),
        3,
    ))
    .unwrap();
    thread.globals().set_str(
        "actualBag",
        crate::bind_items::try_item_value(&bag).unwrap(),
    );
    let old = actual_bag_inventory(&bag).unwrap().unwrap();
    pz_inventory::component_host::drain_retired_components();
    let dropped = Rc::new(Cell::new(0));
    old.borrow_mut().parent = Some(Rc::new(ReentrantContainerParent {
        dropped: dropped.clone(),
        bag: Rc::downgrade(&bag),
        sprites: Some(Rc::downgrade(&sprites)),
    }));
    let weak = Rc::downgrade(&old);
    drop(old);
    let replacement = SharedContainer::default().into_ref();
    thread
        .globals()
        .set_str("replacement", container_value(&replacement));
    crate::bind_world::with_world(|_| {
        let _held = sprites.borrow_mut();
        assert!(call(
            &mut thread,
            "return actualBag:setItemContainer(replacement)"
        )
        .unwrap()
        .is_empty());
        assert!(weak.upgrade().is_some());
        assert_eq!(dropped.get(), 0);
    });
    assert_eq!(dropped.get(), 0);
    pz_inventory::component_host::drain_retired_components();
    assert!(weak.upgrade().is_none());
    assert_eq!(dropped.get(), 1);
    let body = replacement.borrow();
    assert!(body.containing_item.is_none() && body.inventory_container.is_none());
}

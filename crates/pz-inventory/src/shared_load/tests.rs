use super::*;
use crate::io::Writer;
use crate::item_texture::{CapturedScriptTextures, ItemTextures};
use crate::{InventoryItem, ItemClass, ItemTemplate};
use std::cell::Cell;

struct Restore(
    Option<pz_render::TextureRefLoader>,
    Option<shared::ItemObserver>,
);
impl Restore {
    fn new() -> Self {
        Self(
            pz_render::install_texture_ref_loader(None),
            shared::install_item_observer(None),
        )
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        drop(pz_render::install_texture_ref_loader(self.0.take()));
        shared::install_item_observer(self.1.take());
        crate::component_host::drain_retired_components();
    }
}
struct Env(Vec<Rc<ItemTemplate>>);
impl ItemEnv for Env {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        usize::try_from(id - 1)
            .ok()
            .and_then(|id| self.0.get(id))
            .cloned()
    }
}
fn template(class: ItemClass, name: &str) -> Rc<ItemTemplate> {
    let mut t = ItemTemplate::new(class, "Base", name);
    t.texture_capture = Some(CapturedScriptTextures::new(None, vec![]));
    Rc::new(t)
}
fn value(t: &Rc<ItemTemplate>, registry: i16, id: i32) -> InventoryItem {
    let mut value = InventoryItem::create(t.clone(), registry);
    value.id = id;
    value
}
fn write_record(w: &mut Writer, value: InventoryItem, env: &dyn ItemEnv) {
    // Explicit archive fixture encoder supplies wire bytes; the loader never
    // adopts this value and constructs its own actual receiver before body.
    crate::SavedItem::Item(Box::new(value))
        .save(w, env)
        .unwrap();
}
fn header(w: &mut Writer) {
    w.string("loaded");
    w.bool(true);
}
fn tail(w: &mut Writer) {
    w.bool(true);
    w.i32(77);
}
fn target() -> ContainerRef {
    shared::SharedContainer::new(9, "old").into_ref()
}
fn ptr(a: &Option<ItemRef>, b: &ItemRef) {
    assert!(Rc::ptr_eq(a.as_ref().unwrap(), b));
}
fn child(item: &ItemRef) -> ContainerRef {
    shared::inventory_of(&item.borrow()).unwrap()
}
fn break_graph(container: &ContainerRef) {
    let mut work = vec![container.clone()];
    let mut visited = std::collections::HashSet::new();
    while let Some(container) = work.pop() {
        if !visited.insert(Rc::as_ptr(&container) as usize) {
            continue;
        }
        let (items, obsolete, containing, inventory) = {
            let mut body = container.borrow_mut();
            (
                std::mem::take(&mut body.items),
                std::mem::take(&mut body.including_obsolete_items),
                body.containing_item.take(),
                body.inventory_container.take(),
            )
        };
        for item in items
            .into_iter()
            .chain(obsolete.into_iter().flatten())
            .chain(containing)
            .chain(inventory)
        {
            let (old, next) = {
                let mut body = item.borrow_mut();
                let old = body.take_container_for_gc();
                let next = match &mut body.kind {
                    ItemKind::Container(bag) => bag.container.take_for_gc(),
                    _ => None,
                };
                (old, next)
            };
            work.extend(old);
            work.extend(next);
        }
    }
}

#[test]
fn repeated_records_construct_distinct_actual_items_and_link_only_after_complete_compression() {
    let _restore = Restore::new();
    let t = template(ItemClass::Key, "Key");
    let env = Env(vec![t.clone()]);
    let mut w = Writer::new();
    header(&mut w);
    w.i16(1);
    w.i32(3);
    write_record(&mut w, value(&t, 1, 101), &env);
    w.i32(202);
    w.i32(303);
    tail(&mut w);
    let target = target();
    let mut reader = Reader::new(&w.buf);
    let saved = load_container_ref(&target, &mut reader, 249, &env).unwrap();
    assert_eq!(reader.pos, w.buf.len());
    let body = target.borrow();
    assert_eq!(body.items.len(), 3);
    assert_eq!(body.including_obsolete_items.len(), 3);
    assert_eq!(body.kind, "loaded");
    assert!(body.explored && body.has_been_looted);
    assert_eq!(body.capacity, 77);
    assert!(!body.dirty);
    for (index, id) in [101, 202, 303].into_iter().enumerate() {
        let item = &body.items[index];
        ptr(&saved[index], item);
        ptr(&body.including_obsolete_items[index], item);
        assert_eq!(item.borrow().id, id);
        assert!(matches!(item.borrow().textures, ItemTextures::Live(_)));
        assert!(crate::item_texture::get_tex_ref(item).unwrap().is_none());
        assert!(Rc::ptr_eq(
            &shared::try_container_of(item).unwrap().unwrap(),
            &target
        ));
    }
    assert!(!Rc::ptr_eq(&body.items[0], &body.items[1]));
    drop(body);
    break_graph(&target);
}

#[test]
fn nested_bag_reuses_actual_child_and_retains_constructor_asset_capture() {
    let _restore = Restore::new();
    let mut bag = (*template(ItemClass::Container, "Bag")).clone();
    bag.capacity = 17;
    bag.weight_reduction = 42;
    let bag = Rc::new(bag);
    let mut food = (*template(ItemClass::Food, "Food")).clone();
    let base = Rc::new(pz_render::LuaTexture::new(pz_render::Texture::empty(
        "Item_Base",
    )));
    let cooked = Rc::new(pz_render::LuaTexture::new(pz_render::Texture::empty(
        "Item_Cooked",
    )));
    food.texture_capture = Some(CapturedScriptTextures::new(
        Some(base),
        vec![None, Some(cooked.clone()), None],
    ));
    let food = Rc::new(food);
    let env = Env(vec![bag.clone(), food.clone()]);
    let outer = value(&bag, 1, 123);
    let ItemKind::Container(data) = &outer.kind else {
        unreachable!()
    };
    let mut inner = value(&food, 2, 456);
    let ItemKind::Food(data_food) = &mut inner.kind else {
        unreachable!()
    };
    data_food.cooked = true;
    data_food.cooking_time = 9.0;
    data.container.borrow_mut().id = 88;
    data.container
        .borrow_mut()
        .items
        .push(shared::new_item(inner));
    let mut w = Writer::new();
    header(&mut w);
    w.i16(1);
    w.i32(1);
    write_record(&mut w, outer, &env);
    tail(&mut w);
    let target = target();
    let saved = load_container_ref(&target, &mut Reader::new(&w.buf), 249, &env).unwrap();
    let actual_bag = saved[0].as_ref().unwrap();
    let inventory = child(actual_bag);
    let body = inventory.borrow();
    assert_eq!(body.id, 88);
    assert_eq!(body.capacity, 17);
    assert_eq!(body.weight_reduction, 42);
    assert!(Rc::ptr_eq(
        body.containing_item.as_ref().unwrap(),
        actual_bag
    ));
    let actual_food = body.items[0].clone();
    ptr(&body.including_obsolete_items[0], &actual_food);
    assert!(Rc::ptr_eq(
        &crate::item_texture::get_tex_ref(&actual_food)
            .unwrap()
            .unwrap(),
        &cooked
    ));
    assert!(Rc::ptr_eq(
        &shared::try_container_of(&actual_food).unwrap().unwrap(),
        &inventory
    ));
    let time = {
        let body = actual_food.borrow();
        let ItemKind::Food(food) = &body.kind else {
            unreachable!()
        };
        food.cooking_time
    };
    assert_eq!(time, 9.0);
    drop(body);
    break_graph(&target);
}

#[test]
fn missing_first_record_publishes_null_saved_positions_and_seeks_duplicate_ids_once() {
    let _restore = Restore::new();
    let t = template(ItemClass::Normal, "Known");
    let env = Env(vec![t.clone()]);
    let mut w = Writer::new();
    header(&mut w);
    w.i16(2);
    w.i32(3);
    write_record(&mut w, value(&t, 99, 1), &env);
    w.i32(2);
    w.i32(3);
    w.i32(1);
    write_record(&mut w, value(&t, 1, 4), &env);
    tail(&mut w);
    let target = target();
    let mut reader = Reader::new(&w.buf);
    let saved = load_container_ref(&target, &mut reader, 249, &env).unwrap();
    assert_eq!(reader.pos, w.buf.len());
    assert_eq!(saved.len(), 4);
    assert!(saved[..3].iter().all(Option::is_none));
    let body = target.borrow();
    assert_eq!(body.items.len(), 1);
    assert_eq!(body.including_obsolete_items.len(), 4);
    assert!(body.including_obsolete_items[..3]
        .iter()
        .all(Option::is_none));
    ptr(&saved[3], &body.items[0]);
    drop(body);
    break_graph(&target);
}

#[test]
fn zero_and_negative_runs_still_construct_and_load_first_record_without_publication() {
    let _restore = Restore::new();
    struct Count {
        template: Rc<ItemTemplate>,
        calls: Cell<usize>,
    }
    impl ItemEnv for Count {
        fn template(&self, _: i16) -> Option<Rc<ItemTemplate>> {
            self.calls.set(self.calls.get() + 1);
            Some(self.template.clone())
        }
    }
    let t = template(ItemClass::Normal, "Count");
    let env = Count {
        template: t.clone(),
        calls: Cell::new(0),
    };
    let mut w = Writer::new();
    header(&mut w);
    w.i16(2);
    for count in [0, -2] {
        w.i32(count);
        write_record(&mut w, value(&t, 1, 5), &env);
    }
    tail(&mut w);
    let target = target();
    let mut reader = Reader::new(&w.buf);
    assert!(load_container_ref(&target, &mut reader, 249, &env)
        .unwrap()
        .is_empty());
    assert_eq!(env.calls.get(), 2);
    assert_eq!(reader.pos, w.buf.len());
    assert!(target.borrow().items.is_empty());
}

#[test]
fn duplicate_id_underflow_preserves_published_prefix_without_eager_backlinks_or_tail() {
    let _restore = Restore::new();
    let t = template(ItemClass::Normal, "Prefix");
    let env = Env(vec![t.clone()]);
    let mut w = Writer::new();
    header(&mut w);
    w.i16(1);
    w.i32(3);
    write_record(&mut w, value(&t, 1, 1), &env);
    w.i32(2);
    let target = target();
    target.borrow_mut().has_been_looted = false;
    let failure = load_container_ref(&target, &mut Reader::new(&w.buf), 249, &env).unwrap_err();
    assert_eq!(failure.phase, ContainerLoadPhase::DuplicateIds);
    assert_eq!(failure.position, w.buf.len());
    assert!(matches!(
        failure.error,
        RuntimeHostError::NativeException(_)
    ));
    let body = target.borrow();
    assert_eq!(body.items.len(), 3);
    assert_eq!(body.including_obsolete_items.len(), 3);
    assert_eq!(
        body.items.iter().map(|i| i.borrow().id).collect::<Vec<_>>(),
        vec![1, 2, 1]
    );
    for item in &body.items {
        assert!(shared::try_container_of(item).unwrap().is_none());
    }
    assert!(!body.has_been_looted);
    assert_eq!(body.capacity, 50);
    assert!(body.dirty);
    drop(body);
    break_graph(&target);
}

#[test]
fn later_null_load_stops_at_main_publication_with_real_first_item_retained() {
    let _restore = Restore::new();
    struct Sometimes {
        t: Rc<ItemTemplate>,
        calls: Cell<usize>,
    }
    impl ItemEnv for Sometimes {
        fn template(&self, _: i16) -> Option<Rc<ItemTemplate>> {
            let n = self.calls.get();
            self.calls.set(n + 1);
            (n == 0).then(|| self.t.clone())
        }
    }
    let t = template(ItemClass::Normal, "Sometimes");
    let env = Sometimes {
        t: t.clone(),
        calls: Cell::new(0),
    };
    let mut w = Writer::new();
    header(&mut w);
    w.i16(1);
    w.i32(2);
    write_record(&mut w, value(&t, 1, 11), &env);
    let end = w.pos();
    w.i32(22);
    tail(&mut w);
    let target = target();
    let mut reader = Reader::new(&w.buf);
    let failure = load_container_ref(&target, &mut reader, 249, &env).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Unsupported {
            operation: "actual main item list null publication",
            ..
        }
    ));
    assert_eq!(failure.phase, ContainerLoadPhase::PublishMain);
    assert_eq!(reader.pos, end);
    assert_eq!(env.calls.get(), 2);
    assert_eq!(target.borrow().items.len(), 1);
    assert_eq!(target.borrow().including_obsolete_items.len(), 1);
    assert!(shared::try_container_of(&target.borrow().items[0])
        .unwrap()
        .is_none());
    break_graph(&target);
}

#[test]
fn malformed_type_and_held_container_preserve_native_header_cursor_without_clearing_lists() {
    let _restore = Restore::new();
    let target = target();
    let t = template(ItemClass::Normal, "Old");
    let old = shared::new_item(value(&t, 1, 7));
    target.borrow_mut().items.push(old.clone());
    let bytes = [0, 10, b'x'];
    let mut reader = Reader::new(&bytes);
    let failure = load_container_ref(&target, &mut reader, 249, &Env(vec![])).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::NativeException(_)
    ));
    assert_eq!(reader.pos, 2);
    assert_eq!(target.borrow().kind, "old");
    assert_eq!(target.borrow().items.len(), 1);
    let mut w = Writer::new();
    header(&mut w);
    w.i16(0);
    tail(&mut w);
    let mut reader = Reader::new(&w.buf);
    let held = target.borrow_mut();
    let failure = load_container_ref(&target, &mut reader, 249, &Env(vec![])).unwrap_err();
    assert!(matches!(failure.error, RuntimeHostError::Borrowed(_)));
    assert_eq!(failure.phase, ContainerLoadPhase::Type);
    assert_eq!(reader.pos, 8);
    drop(held);
    break_graph(&target);
}

#[test]
fn strong_container_links_archive_copy_clear_and_collector_null_is_physical() {
    let _restore = Restore::new();
    let t = template(ItemClass::Normal, "Owner");
    let item = shared::new_item(value(&t, 1, 1));
    let container = target();
    let weak = Rc::downgrade(&container);
    shared::try_set_container(&item, Some(&container)).unwrap();
    drop(container);
    crate::component_host::drain_retired_components();
    assert!(weak.upgrade().is_some());
    let copy = item.borrow().try_archive_copy().unwrap();
    assert!(copy.container_slot().is_none());
    let foreign = item.borrow_mut();
    assert!(matches!(
        shared::try_container_of(&item),
        Err(RuntimeHostError::Borrowed(_))
    ));
    drop(foreign);
    let old = item.borrow_mut().take_container_for_gc().unwrap();
    assert!(item.borrow().container_slot().is_none());
    drop(old);
    assert!(weak.upgrade().is_none());
    let bag_t = template(ItemClass::Container, "Bag");
    let bag = crate::env::create_from_template_ref(&Env(vec![]), bag_t, 2).unwrap();
    let inventory = child(&bag);
    assert!(Rc::ptr_eq(
        inventory.borrow().containing_item.as_ref().unwrap(),
        &bag
    ));
    let taken = {
        let mut body = bag.borrow_mut();
        let ItemKind::Container(data) = &mut body.kind else {
            unreachable!()
        };
        data.container.take_for_gc().unwrap()
    };
    assert!(shared::inventory_of(&bag.borrow()).is_none());
    assert!(Rc::ptr_eq(&taken, &inventory));
    let owners = {
        let mut body = inventory.borrow_mut();
        (body.containing_item.take(), body.inventory_container.take())
    };
    drop(owners);
}

#[test]
fn bag_load_publishes_weight_and_child_clear_before_truncated_nested_header_and_handles_null_without_allocation(
) {
    let _restore = Restore::new();
    let t = template(ItemClass::Container, "Bag");
    let env = Env(vec![t.clone()]);
    let bag = crate::env::create_from_template_ref(&env, t, 1).unwrap();
    let inventory = child(&bag);
    {
        let mut body = bag.borrow_mut();
        let ItemKind::Container(data) = &mut body.kind else {
            unreachable!()
        };
        data.capacity = 23;
    }
    let old = shared::new_item(value(&template(ItemClass::Normal, "Old"), 2, 8));
    inventory.borrow_mut().items.push(old.clone());
    let mut w = Writer::new();
    w.i32(99);
    w.i32(150);
    w.string("new");
    w.bool(true); // missing compressed count, AFTER clearboth.
    let held_child = inventory.borrow_mut();
    let mut held_reader = Reader::new(&w.buf);
    assert!(matches!(
        load_bag_tail_ref(&bag, &mut held_reader, 249, &env),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert_eq!(held_reader.pos, 8);
    assert_eq!(held_child.id, -1);
    assert_eq!(held_child.items.len(), 1);
    {
        let body = bag.borrow();
        let ItemKind::Container(data) = &body.kind else {
            unreachable!()
        };
        assert_eq!(data.weight_reduction, 100);
    }
    drop(held_child);
    assert!(matches!(
        load_bag_tail_ref(&bag, &mut Reader::new(&w.buf), 249, &env),
        Err(RuntimeHostError::NativeException(_))
    ));
    assert!(Rc::ptr_eq(&child(&bag), &inventory));
    let body = inventory.borrow();
    assert!(body.items.is_empty());
    assert!(body.draw_dirty && body.dirty);
    assert_eq!(body.weight_reduction, 100);
    assert_eq!(body.capacity, 23);
    assert_eq!(body.id, 99);
    assert_eq!(body.kind, "new");
    drop(body);
    let detached = {
        let mut body = bag.borrow_mut();
        let ItemKind::Container(data) = &mut body.kind else {
            unreachable!()
        };
        data.container.take_for_gc()
    };
    let mut r = Reader::new(&w.buf);
    assert!(matches!(
        load_bag_tail_ref(&bag, &mut r, 249, &env),
        Err(RuntimeHostError::Unsupported {
            operation: "native null bag inventory load diagnostic",
            ..
        })
    ));
    assert_eq!(r.pos, 8);
    assert!(shared::inventory_of(&bag.borrow()).is_none());
    drop(detached);
    let owners = {
        let mut body = inventory.borrow_mut();
        (body.containing_item.take(), body.inventory_container.take())
    };
    drop(owners);
}

#[test]
fn count_failure_retires_old_owners_after_outer_guard_and_preserves_old_backlink() {
    let _restore = Restore::new();
    struct Probe {
        foreign: Rc<std::cell::RefCell<()>>,
        dropped: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(self.foreign.try_borrow_mut().is_ok());
            self.dropped.set(true);
        }
    }
    let foreign = Rc::new(std::cell::RefCell::new(()));
    let dropped = Rc::new(Cell::new(false));
    let probe = Rc::new(std::cell::RefCell::new(Probe {
        foreign: foreign.clone(),
        dropped: dropped.clone(),
    }));
    let old = shared::new_item(value(&template(ItemClass::Normal, "Retired"), 1, 1));
    // An erased owning slot is enough to exercise physical final-drop timing;
    // this fixture invokes no Native world method/type/lifecycle assumption.
    crate::world_item::set_world_item_ref(&old, Some(&pz_entity::EntityOwner::from_ref(&probe)))
        .unwrap();
    drop(probe);
    let target = target();
    shared::try_add_item(&target, &old).unwrap();
    target
        .borrow_mut()
        .including_obsolete_items
        .push(Some(old.clone()));
    let weak = Rc::downgrade(&old);
    drop(old);
    let mut w = Writer::new();
    header(&mut w);
    let guard = foreign.borrow_mut();
    let failure =
        load_container_ref(&target, &mut Reader::new(&w.buf), 249, &Env(vec![])).unwrap_err();
    assert_eq!(failure.phase, ContainerLoadPhase::GroupCount);
    assert!(target.borrow().items.is_empty());
    assert!(target.borrow().including_obsolete_items.is_empty());
    let retained = weak.upgrade().unwrap();
    assert!(Rc::ptr_eq(
        &shared::try_container_of(&retained).unwrap().unwrap(),
        &target
    ));
    drop(retained);
    assert!(!dropped.get());
    drop(guard);
    crate::component_host::drain_retired_components();
    assert!(dropped.get());
    assert!(weak.upgrade().is_none());
}

thread_local! {
    static CREATED: std::cell::RefCell<Vec<std::rc::Weak<std::cell::RefCell<InventoryItem>>>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn track_created(item: &ItemRef) {
    CREATED.with(|items| items.borrow_mut().push(Rc::downgrade(item)));
}

#[test]
fn nested_texture_provider_runs_without_parent_item_or_container_guards_and_keeps_error_prefix() {
    let _restore = Restore::new();
    let bag_t = template(ItemClass::Container, "Bag");
    let inner_t = template(ItemClass::Normal, "Inside");
    let env = Env(vec![bag_t.clone(), inner_t.clone()]);
    let outer = value(&bag_t, 1, 1);
    let ItemKind::Container(data) = &outer.kind else {
        unreachable!()
    };
    let mut inside = value(&inner_t, 2, 2);
    inside.textures = ItemTextures::Archive(crate::item_texture::ArchiveTextureProfile {
        saved_name: Some("Custom".into()),
    });
    data.container
        .borrow_mut()
        .items
        .push(shared::new_item(inside));
    let mut w = Writer::new();
    header(&mut w);
    w.i16(1);
    w.i32(1);
    write_record(&mut w, outer, &env);
    tail(&mut w);
    let target = target();
    let target_for_provider = target.clone();
    CREATED.with(|items| items.borrow_mut().clear());
    shared::install_item_observer(Some(track_created));
    let entered = Rc::new(Cell::new(false));
    let entered_callback = entered.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        assert_eq!(name, "Custom");
        assert!(target_for_provider.try_borrow_mut().is_ok());
        CREATED.with(|items| {
            for item in items.borrow().iter().filter_map(std::rc::Weak::upgrade) {
                let inventory = {
                    let body = item
                        .try_borrow_mut()
                        .expect("no parent item guard during nested texture callback");
                    shared::inventory_of(&body)
                };
                if let Some(inventory) = inventory {
                    assert!(inventory.try_borrow_mut().is_ok());
                }
            }
        });
        entered_callback.set(true);
        Err(pz_render::TextureHostError::Borrowed(
            "nested provider sentinel",
        ))
    })));
    let failure = load_container_ref(&target, &mut Reader::new(&w.buf), 249, &env).unwrap_err();
    assert!(entered.get());
    assert!(matches!(
        failure.error,
        RuntimeHostError::Borrowed("nested provider sentinel")
    ));
    assert_eq!(failure.phase, ContainerLoadPhase::Record);
    assert!(target.borrow().items.is_empty());
    assert!(target.borrow().including_obsolete_items.is_empty());
    assert!(!target.borrow().has_been_looted);
}

#[test]
fn actual_bag_capacity_defaults_survive_constructor_failure_and_warning_stops_before_child_publication(
) {
    let _restore = Restore::new();
    let mut uncaptured = ItemTemplate::new(ItemClass::Container, "Base", "Uncaptured");
    uncaptured.capacity = 17;
    uncaptured.weight_reduction = 73;
    let failure =
        crate::env::create_from_template_ref(&Env(vec![]), Rc::new(uncaptured), 1).unwrap_err();
    let receiver = failure.item.as_ref().unwrap();
    let body = receiver.borrow();
    let ItemKind::Container(data) = &body.kind else {
        unreachable!()
    };
    assert_eq!(data.capacity, 0);
    assert_eq!(data.weight_reduction, 0);
    assert_eq!(data.container.borrow().capacity, 50);
    assert_eq!(body.item_capacity, -1.0);
    drop(body);
    break_graph(&child(receiver));
    let _ = failure.into_error_and_retire();
    let mut too_large = (*template(ItemClass::Container, "Large")).clone();
    too_large.capacity = 100;
    too_large.weight_reduction = 73;
    let failure =
        crate::env::create_from_template_ref(&Env(vec![]), Rc::new(too_large), 1).unwrap_err();
    assert!(matches!(
        &failure.error,
        RuntimeHostError::Unsupported {
            operation: "native bag capacity identity warning",
            ..
        }
    ));
    let receiver = failure.item.as_ref().unwrap();
    let body = receiver.borrow();
    let ItemKind::Container(data) = &body.kind else {
        unreachable!()
    };
    assert_eq!(data.capacity, 100);
    assert_eq!(data.weight_reduction, 0);
    assert_eq!(data.container.borrow().capacity, 50);
    assert_eq!(body.item_capacity, 100.0);
    drop(body);
    break_graph(&child(receiver));
    let _ = failure.into_error_and_retire();
}

#[test]
fn actual_bag_factory_reads_current_argument_capacity_and_weight_not_template_or_live_proxy() {
    use pz_scripts::ScriptObject;
    let _restore = Restore::new();
    let sm = pz_scripts::ScriptManager::new();
    sm.parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {} ")
        .unwrap();
    sm.enable_texture_capture();
    let module = sm.get_module("Base").unwrap();
    let mut script = pz_scripts::objects::items::Item::new();
    script.set_module(&module);
    script.init_load_pp(&sm, "Bag");
    script
        .load(
            &sm,
            "Bag",
            "item Bag { Capacity = 49, WeightReduction = 42, ItemCapacity = 7, Weight = 2, }",
        )
        .unwrap();
    assert!(script.texture_capture_complete());
    let script = crate::template::ScriptItemRef(Rc::new(std::cell::RefCell::new(script)));
    let mut t = (*template(ItemClass::Container, "Bag")).clone();
    t.script = Some(script.clone());
    t.capacity = 123;
    t.actual_weight = 123.0;
    t.item_capacity = 999;
    t.weight_reduction = 9;
    let bag = crate::env::create_from_template_ref(&Env(vec![]), Rc::new(t), 1).unwrap();
    {
        let body = bag.borrow();
        let ItemKind::Container(data) = &body.kind else {
            unreachable!()
        };
        assert_eq!(data.capacity, 49);
        assert_eq!(data.weight_reduction, 42);
        assert_eq!(body.item_capacity, 7.0);
        assert_eq!(body.actual_weight, 2.0);
        assert_eq!(body.weight, 2.0);
    }
    let inventory = child(&bag);
    assert_eq!(inventory.borrow().capacity, 49);
    assert_eq!(inventory.borrow().weight_reduction, 42);
    script.0.borrow_mut().capacity = 31;
    script.0.borrow_mut().weight_reduction = 73;
    {
        let body = bag.borrow();
        let ItemKind::Container(data) = &body.kind else {
            unreachable!()
        };
        assert_eq!(data.capacity, 49);
        assert_eq!(data.weight_reduction, 42);
    }
    break_graph(&inventory);
}

#[test]
fn actual_bag_common_weight_setters_preserve_nan_and_signed_zero_and_clamp_only_ordered_negative() {
    let _restore = Restore::new();
    for (weight, expected) in [
        (f32::from_bits(0x7fc1_2345), 0x7fc1_2345),
        (-0.0, (-0.0f32).to_bits()),
        (-3.0, 0.0f32.to_bits()),
    ] {
        let mut t = (*template(ItemClass::Container, "Weighted")).clone();
        t.actual_weight = weight;
        let bag = crate::env::create_from_template_ref(&Env(vec![]), Rc::new(t), 1).unwrap();
        {
            let body = bag.borrow();
            assert_eq!(body.actual_weight.to_bits(), expected);
            assert_eq!(body.weight.to_bits(), expected);
        }
        break_graph(&child(&bag));
    }
}

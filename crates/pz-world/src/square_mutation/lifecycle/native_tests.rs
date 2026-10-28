use super::*;
use crate::sprite::runtime::{create_runtime_sprite, ObjectSprite, RuntimeSpriteRef};
use crate::{IsoChunk, IsoGridSquare, IsoSpriteManager, SquareRef, SquareSlot};
use pz_entity::runtime_load::RuntimeHostError;
use pz_inventory::world_update::WorldItemUpdateContext as _;
use std::cell::RefCell;
use std::rc::Rc;

fn fixture() -> (
    IsoObjectRef,
    RuntimeSpriteRef,
    SquareRef,
    IsoChunk,
    pz_entity::RetiredComponents,
) {
    let tables = crate::sprite::test_tables();
    let manager = Rc::new(RefCell::new(IsoSpriteManager::new(tables.clone())));
    let mut retired = pz_entity::RetiredComponents::new();
    let sprite = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(SquareSlot::new(IsoGridSquare::new(1, 1, 0))));
    let square = chunk.get_grid_square(1, 1, 0).unwrap();
    // Genuine nullable item branch of the native WorldInventory subclass.
    let payload = crate::objects::IsoWorldInventoryObject {
        xoff: 0.5,
        yoff: 0.5,
        zoff: 0.0,
        offset_x: 0.0,
        offset_y: 0.0,
        item: crate::floor_item::FloorItem::null(),
        drop_time: 1.0,
        ignore_remove_sandbox: false,
        extended_placement: false,
    };
    let mut object = IsoObject::new(
        1,
        ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(payload)),
        None,
    );
    object.sprite = ObjectSprite::Native(sprite.clone());
    object.replace_square(Some(&square), &mut retired);
    let object = ObjectSlot::new(object).ref_handle();
    (object, sprite, square, chunk, retired)
}

fn detach(sprite: &RuntimeSpriteRef) {
    let slots = { sprite.borrow_mut().take_slots_for_gc() };
    drop(slots);
}

#[test]
fn actual_empty_component_lifecycle_allocates_event_sets_flag_and_hot_save() {
    let (object, sprite, square, chunk, mut retired) = fixture();
    let pool = pz_entity::runtime_event::entity_event_pool();
    assert!(pool.is_empty().unwrap());
    native_floor_add_to_world(
        &object,
        &square,
        &crate::sprite::test_tables(),
        &mut retired,
    )
    .unwrap();
    assert!(object.borrow().added_to_world_or_equipped);
    assert!(!object.borrow().added_to_entity_manager);
    assert!(chunk.hot_save.borrow().requires_hot_save);
    assert_eq!(pool.len(), Ok(1));
    let event = pool
        .alloc(
            pz_entity::runtime_event::EntityEventType::AddedToWorld,
            &pz_entity::EntityOwner::from_ref(&object),
            &mut retired,
        )
        .unwrap();
    assert!(event
        .borrow()
        .entity_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&object)));
    detach(&sprite);
}

#[test]
fn actual_sprite_borrow_failure_preserves_completed_entity_and_event_prefix() {
    let (object, sprite, square, chunk, mut retired) = fixture();
    let guard = sprite.borrow_mut();
    assert_eq!(
        native_floor_add_to_world(
            &object,
            &square,
            &crate::sprite::test_tables(),
            &mut retired
        ),
        Err(RuntimeHostError::Borrowed("floor createContainers sprite"))
    );
    assert!(object.borrow().added_to_world_or_equipped);
    assert!(!object.borrow().added_to_entity_manager);
    assert_eq!(pz_entity::runtime_event::entity_event_pool().len(), Ok(1));
    assert!(!chunk.hot_save.borrow().requires_hot_save);
    drop(guard);
    detach(&sprite);
}

#[test]
fn actual_hot_save_fault_does_not_undo_earlier_entity_flag_or_event_release() {
    let (object, sprite, square, chunk, mut retired) = fixture();
    let guard = chunk.hot_save.borrow();
    assert_eq!(
        native_floor_add_to_world(
            &object,
            &square,
            &crate::sprite::test_tables(),
            &mut retired
        ),
        Err(RuntimeHostError::Borrowed("flagForHotSave actual chunk"))
    );
    assert!(object.borrow().added_to_world_or_equipped);
    assert_eq!(pz_entity::runtime_event::entity_event_pool().len(), Ok(1));
    assert!(!guard.requires_hot_save);
    drop(guard);
    detach(&sprite);
}

#[test]
fn native_floor_dispatch_never_uses_tile_absence_for_nonempty_properties() {
    let (object, sprite, _square, chunk, mut retired) = fixture();
    let flag = crate::sprite::test_tables().flag.container;
    sprite.borrow_mut().data.properties.set_flag(flag);
    assert_eq!(
        try_native_add_to_world_dispatch(&object, &crate::sprite::test_tables(), &mut retired),
        Err(RuntimeHostError::Unsupported {
            operation: "native nonempty floor sprite property evaluation",
            kind: None,
        })
    );
    assert!(object.borrow().added_to_world_or_equipped);
    assert_eq!(pz_entity::runtime_event::entity_event_pool().len(), Ok(1));
    assert!(!chunk.hot_save.borrow().requires_hot_save);
    assert_eq!(object.borrow().container_count(), 0);
    detach(&sprite);
}

#[test]
fn other_native_sprite_class_stops_before_legacy_registration_or_flag() {
    let (floor, sprite, _square, _chunk, mut retired) = fixture();
    let mut body = IsoObject::new(2, ObjectKind::Object, None);
    body.sprite = ObjectSprite::Native(sprite.clone());
    let object = ObjectSlot::new(body).ref_handle();
    assert_eq!(
        try_native_add_to_world_dispatch(&object, &crate::sprite::test_tables(), &mut retired),
        Err(RuntimeHostError::Unsupported {
            operation: "actual native sprite subclass addToWorld",
            kind: None,
        })
    );
    assert!(!object.borrow().added_to_world_or_equipped);
    assert!(!object.borrow().added_to_entity_manager);
    assert!(pz_entity::runtime_event::entity_event_pool()
        .is_empty()
        .unwrap());
    drop(floor);
    detach(&sprite);
}

fn clear_world_entries(square: &SquareRef) {
    let list = square.borrow().world_objects.clone();
    let entries = { list.borrow_mut().take_entries() };
    drop(entries);
}

#[test]
fn world_index_uses_retained_square_and_first_actual_pointer_after_coordinate_replacement() {
    let (object, sprite, square, mut chunk, mut retired) = fixture();
    let tables = crate::sprite::test_tables();
    let context = PlainFloorContext { tables: &tables };
    let owner = pz_entity::EntityOwner::from_ref(&object);
    let other =
        ObjectSlot::new(IsoObject::new(object.borrow().id, ObjectKind::Object, None)).ref_handle();
    {
        let list = square.borrow().world_objects.clone();
        let mut list = list.borrow_mut();
        list.push_entry(crate::world_objects::WorldObjectEntry::Foreign(
            pz_kahlua::Value::Nil,
        ));
        list.push_object(&other);
        list.push_object(&object);
        list.push_object(&object);
    }
    square
        .borrow_mut()
        .properties
        .set_flag(tables.flag.exterior);
    // Native reads the stored property, irrespective of room metadata.
    square.borrow_mut().room_id = 42;
    chunk.set_square(1, 1, 0, Some(SquareSlot::new(IsoGridSquare::new(1, 1, 0))));
    let replacement = chunk.get_grid_square(1, 1, 0).unwrap();
    assert!(!Rc::ptr_eq(&square, &replacement));
    replacement
        .borrow()
        .world_objects
        .borrow_mut()
        .push_object(&object);
    let held_other = other.borrow_mut();
    assert_eq!(context.world_object_index(&owner), Ok(2));
    drop(held_other);
    assert_eq!(context.world_square_outside(&owner), Ok(Some(true)));

    object
        .borrow_mut()
        .replace_square(Some(&replacement), &mut retired);
    assert_eq!(context.world_object_index(&owner), Ok(0));
    assert_eq!(context.world_square_outside(&owner), Ok(Some(false)));
    // The current list field, rather than a cached membership result, is read.
    let old_list = {
        let mut body = replacement.borrow_mut();
        std::mem::replace(
            &mut body.world_objects,
            crate::world_objects::WorldObjects::new_ref(),
        )
    };
    assert_eq!(context.world_object_index(&owner), Ok(-1));
    let entries = { old_list.borrow_mut().take_entries() };
    drop(entries);
    clear_world_entries(&square);
    clear_world_entries(&replacement);
    detach(&sprite);
}

#[test]
fn null_square_and_non_world_inventory_return_minus_one_before_square_or_list_borrow() {
    let (object, sprite, square, _chunk, mut retired) = fixture();
    let tables = crate::sprite::test_tables();
    let context = PlainFloorContext { tables: &tables };
    let mut plain = IsoObject::new(9, ObjectKind::Object, None);
    plain.replace_square(Some(&square), &mut retired);
    let plain = ObjectSlot::new(plain).ref_handle();
    let plain_owner = pz_entity::EntityOwner::from_ref(&plain);
    let held = square.borrow_mut();
    assert_eq!(context.world_object_index(&plain_owner), Ok(-1));
    assert_eq!(
        context.world_square_outside(&plain_owner),
        Err(RuntimeHostError::Borrowed(
            "floor predicate square.properties"
        ))
    );
    drop(held);
    object.borrow_mut().replace_square(None, &mut retired);
    let owner = pz_entity::EntityOwner::from_ref(&object);
    let list = square.borrow().world_objects.clone();
    let held = list.borrow_mut();
    assert_eq!(context.world_object_index(&owner), Ok(-1));
    assert_eq!(context.world_square_outside(&owner), Ok(None));
    drop(held);
    let wrong = pz_entity::EntityOwner::from_ref(&Rc::new(RefCell::new(1_i32)));
    assert!(matches!(
        context.world_object_index(&wrong),
        Err(RuntimeHostError::InvalidHostState(_))
    ));
    assert!(matches!(
        context.world_square_outside(&wrong),
        Err(RuntimeHostError::InvalidHostState(_))
    ));
    detach(&sprite);
}

#[test]
fn current_world_field_queries_report_each_held_body_without_false_absence() {
    let (object, sprite, square, _chunk, _retired) = fixture();
    let tables = crate::sprite::test_tables();
    let context = PlainFloorContext { tables: &tables };
    let owner = pz_entity::EntityOwner::from_ref(&object);
    let held = object.borrow_mut();
    assert_eq!(
        context.world_object_index(&owner),
        Err(RuntimeHostError::Borrowed("floor predicate object.square"))
    );
    assert_eq!(
        context.world_square_outside(&owner),
        Err(RuntimeHostError::Borrowed("floor predicate object.square"))
    );
    drop(held);
    let held = square.borrow_mut();
    assert_eq!(
        context.world_object_index(&owner),
        Err(RuntimeHostError::Borrowed(
            "floor predicate square.worldObjects"
        ))
    );
    assert_eq!(
        context.world_square_outside(&owner),
        Err(RuntimeHostError::Borrowed(
            "floor predicate square.properties"
        ))
    );
    drop(held);
    let list = square.borrow().world_objects.clone();
    let held = list.borrow_mut();
    assert_eq!(
        context.world_object_index(&owner),
        Err(RuntimeHostError::Borrowed("floor predicate worldObjects"))
    );
    assert_eq!(context.world_square_outside(&owner), Ok(Some(false)));
    drop(held);
    square
        .borrow_mut()
        .properties
        .set_flag(tables.flag.exterior);
    assert_eq!(context.world_square_outside(&owner), Ok(Some(true)));
    square
        .borrow_mut()
        .properties
        .unset_flag(tables.flag.exterior);
    assert_eq!(context.world_square_outside(&owner), Ok(Some(false)));
    detach(&sprite);
}

#[test]
fn actual_floor_predicate_uses_current_square_before_process_registry_and_manager_body() {
    let (object, sprite, square, chunk, mut retired) = fixture();
    let tables = crate::sprite::test_tables();
    let item = pz_inventory::shared::new_item(pz_inventory::InventoryItem::create(
        Rc::new(pz_inventory::ItemTemplate::new(
            pz_inventory::ItemClass::Normal,
            "Base",
            "BathTowelWet",
        )),
        1,
    ));
    let floor = crate::floor_object::runtime::floor_ref(&object).unwrap();
    floor.borrow_mut().item = crate::floor_item::FloorItem::retain(&item);
    crate::floor_object::runtime::publish_item_backlink(&object, &item, &mut retired).unwrap();
    square
        .borrow()
        .world_objects
        .borrow_mut()
        .push_object(&object);
    square
        .borrow_mut()
        .properties
        .set_flag(tables.flag.exterior);
    let decoy = SquareSlot::new(IsoGridSquare::new(1, 1, 0)).ref_handle();
    let manager = sprite.borrow().parent_manager_slot().unwrap().clone();
    let held_manager = manager.borrow_mut();
    assert_eq!(
        native_floor_add_to_world(&object, &decoy, &tables, &mut retired),
        Err(RuntimeHostError::Unsupported {
            operation: "actual cell processWorldItems registry",
            kind: None
        })
    );
    assert!(!object.borrow().added_to_world_or_equipped);
    assert!(!chunk.hot_save.borrow().requires_hot_save);
    // Reread the current exterior bit; the supplied same-coordinate square
    // and a held sprite-manager body cannot supply or hide this predicate.
    square
        .borrow_mut()
        .properties
        .unset_flag(tables.flag.exterior);
    native_floor_add_to_world(&object, &decoy, &tables, &mut retired).unwrap();
    assert!(object.borrow().added_to_world_or_equipped);
    assert!(chunk.hot_save.borrow().requires_hot_save);
    drop(held_manager);
    clear_world_entries(&square);
    let owner = { item.borrow_mut().take_world_item_for_gc() };
    drop(owner);
    detach(&sprite);
}

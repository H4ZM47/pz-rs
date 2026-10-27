use super::*;
use crate::sprite::runtime::{create_runtime_sprite, ObjectSprite, RuntimeSpriteRef};
use crate::{IsoChunk, IsoGridSquare, IsoSpriteManager, SquareRef, SquareSlot};
use pz_entity::runtime_load::RuntimeHostError;
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
    native_floor_add_to_world(&object, &square, &mut retired).unwrap();
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
        native_floor_add_to_world(&object, &square, &mut retired),
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
        native_floor_add_to_world(&object, &square, &mut retired),
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
        try_native_add_to_world_dispatch(&object, &mut retired),
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
        try_native_add_to_world_dispatch(&object, &mut retired),
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

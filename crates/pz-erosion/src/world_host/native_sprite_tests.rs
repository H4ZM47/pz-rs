use super::*;
use pz_world::sprite::{runtime, PropertyValueMap, TilePropertyAliasMap};
use pz_world::{IsoGridSquare, IsoSpriteManager};
use std::{cell::RefCell, rc::Rc};

#[test]
fn erosion_reads_current_actual_sprite_metadata_and_retires_replaced_owner() {
    let tables = pz_world::sprite::test_tables();
    let manager = Rc::new(RefCell::new(IsoSpriteManager::new(tables.clone())));
    let arena_count = manager.borrow().sprites.len();
    let mut retired = pz_entity::RetiredComponents::new();
    let sprite = runtime::create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    let mut values = PropertyValueMap::default();
    values.put("CustomName", vec!["First".into(), "Second".into()]);
    manager.borrow_mut().alias = TilePropertyAliasMap::generate(&values, &[]);
    {
        let mut body = sprite.borrow_mut();
        body.data.name = Some("live_first".into());
        body.data.tile_sheet_index = 7;
        body.data
            .properties
            .set_raw(&manager.borrow().alias, "CustomName", "First");
    }
    let mut object = IsoObject::new(17, ObjectKind::Object, None);
    object.sprite = runtime::ObjectSprite::Native(sprite.clone());
    let mut square = IsoGridSquare::new(1, 1, 0);
    square.objects.push(object.into());
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(square.into()));
    let world = WorldView::CHUNK_ONLY;
    let mut sprites = manager.borrow_mut();
    let mut host = ChunkHost {
        chunk: &mut chunk,
        sprites: &mut sprites,
        world: &world,
        logs_per_size: None,
    };
    assert_eq!(
        host.sprite_name((1, 1, 0), 17).as_deref(),
        Some("live_first")
    );
    assert_eq!(host.sprite_tile_sheet_index((1, 1, 0), 17), 7);
    assert_eq!(
        host.sprite_custom_name((1, 1, 0), 17).as_deref(),
        Some("First")
    );
    {
        let mut body = sprite.borrow_mut();
        body.data.name = Some("live_second".into());
        body.data.tile_sheet_index = 19;
        body.data
            .properties
            .set_raw(&host.sprites.alias, "CustomName", "Second");
    }
    assert_eq!(host.sprites.sprites.len(), arena_count);
    assert_eq!(
        host.sprite_name((1, 1, 0), 17).as_deref(),
        Some("live_second")
    );
    assert_eq!(host.sprite_tile_sheet_index((1, 1, 0), 17), 19);
    assert_eq!(
        host.sprite_custom_name((1, 1, 0), 17).as_deref(),
        Some("Second")
    );
    // Break the native parent/default-instance cycle through physical GC
    // detachment only, so the displaced owner's retirement is observable.
    let detached = sprite.borrow_mut().take_slots_for_gc();
    drop(detached);
    drop(retired);
    let weak = Rc::downgrade(&sprite);
    host.set_object_sprite((1, 1, 0), 17, "managed_replacement");
    drop(sprite);
    assert!(weak.upgrade().is_some());
    drop(host);
    drop(sprites);
    pz_inventory::component_host::drain_retired_components();
    assert!(weak.upgrade().is_none());
}

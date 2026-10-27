use super::*;
use pz_inventory::component::{ComponentCodec, EntityCodec};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use std::rc::Rc;

struct Env<'a> {
    codec: EntityCodec<'a>,
    template: Rc<ItemTemplate>,
}
impl pz_inventory::ItemEnv for Env<'_> {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        (id == 1).then(|| self.template.clone())
    }
    fn exists(&self, id: i16) -> bool {
        id == 1
    }
    fn components(&self) -> &dyn ComponentCodec {
        &self.codec
    }
}
fn bytes(
    env: &Env<'_>,
    tables: &super::super::SaveTables,
    drop_time: Option<f64>,
    flags: Option<u8>,
) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.i8(1);
    writer.i8(tables.class_id("WorldInventoryItem"));
    for value in [0.25, 0.5, 0.75, 111.0, 222.0] {
        writer.f32(value);
    }
    let mut item = InventoryItem::create(env.template.clone(), 1);
    item.id = 901;
    item.custom_color = true;
    item.col = [0.2, 0.3, 0.4, 1.0];
    item.color_red = 0.9;
    item.save_with_size(&mut writer, env).unwrap();
    if let Some(time) = drop_time {
        writer.f64(time);
    }
    if let Some(flags) = flags {
        writer.i8(flags as i8);
    }
    writer.buf
}

#[test]
fn actual_floor_load_failure_after_item_keeps_same_backlink_tint_and_cursor_prefix() {
    let context = pz_entity::BasicContext::default();
    let env = Env {
        codec: EntityCodec { ctx: &context },
        template: Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "FloorLoad")),
    };
    let sprites = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = super::super::SaveTables::for_tests();
    let settings = super::super::SaveSettings::default();
    let ctx = super::super::SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &env,
        hooks: &super::super::NoSaveHooks,
        meta: None,
    };
    let bytes = bytes(&env, &tables, None, None);
    let mut reader = Reader::new(&bytes);
    let mut failure = load_object_slot(&mut reader, &ctx, 249, 4).unwrap_err();
    assert_eq!(failure.phase, "dropTime");
    assert_eq!(reader.pos, bytes.len());
    assert_eq!(failure.position, bytes.len());
    let object = failure.object.as_ref().unwrap().clone();
    let floor = crate::floor_object::runtime::floor_ref(&object).unwrap();
    let item = floor.borrow().item.item_ref().unwrap();
    assert_eq!(item.borrow().id, 901);
    assert!(item
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&object)));
    assert_eq!(
        (
            floor.borrow().xoff,
            floor.borrow().yoff,
            floor.borrow().zoff
        ),
        (0.25, 0.5, 0.75)
    );
    assert_eq!(
        (
            floor.borrow().offset_x,
            floor.borrow().offset_y,
            floor.borrow().drop_time
        ),
        (0.0, 0.0, -1.0)
    );
    let col = item.borrow().col;
    assert_eq!(
        (
            object.borrow().tint_r,
            object.borrow().tint_g,
            object.borrow().tint_b
        ),
        (col[0], col[1], col[2])
    );
    item.borrow_mut()
        .replace_world_item(None, &mut failure.retired);
}

#[test]
fn actual_floor_load_publishes_each_offset_and_retains_receiver_on_truncation() {
    let context = pz_entity::BasicContext::default();
    let env = Env {
        codec: EntityCodec { ctx: &context },
        template: Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "FloorLoad")),
    };
    let sprites = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = super::super::SaveTables::for_tests();
    let settings = super::super::SaveSettings::default();
    let ctx = super::super::SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &env,
        hooks: &super::super::NoSaveHooks,
        meta: None,
    };
    let mut writer = Writer::new();
    writer.i8(1);
    writer.i8(tables.class_id("WorldInventoryItem"));
    writer.f32(0.625);
    writer.i8(99);
    let mut reader = Reader::new(&writer.buf);
    let failure = load_object_slot(&mut reader, &ctx, 249, 17).unwrap_err();
    assert_eq!(reader.pos, 6);
    assert_eq!(failure.position, 6);
    let object = failure.object.as_ref().unwrap();
    assert_eq!(object.borrow().id, 17);
    let floor = crate::floor_object::runtime::floor_ref(object).unwrap();
    assert_eq!(floor.borrow().xoff, 0.625);
    assert_eq!(floor.borrow().yoff, 0.0);
    assert!(floor.borrow().item.is_null());
    assert_eq!(reader.i8().unwrap(), 99);
}

#[test]
fn actual_floor_null_item_consumes_trailer_without_fabricating_raw_or_entity_fields() {
    let context = pz_entity::BasicContext::default();
    let env = Env {
        codec: EntityCodec { ctx: &context },
        template: Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "FloorLoad")),
    };
    let sprites = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = super::super::SaveTables::for_tests();
    let settings = super::super::SaveSettings::default();
    let ctx = super::super::SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &env,
        hooks: &super::super::NoSaveHooks,
        meta: None,
    };
    let mut writer = Writer::new();
    writer.i8(1);
    writer.i8(tables.class_id("WorldInventoryItem"));
    for value in [0.25, 0.5, 0.75, 111.0, 222.0] {
        writer.f32(value);
    }
    writer.i32(7);
    writer.i16(99);
    writer.i8(0);
    writer.i32(44);
    writer.f64(123.0);
    writer.i8(7);
    let end = writer.buf.len();
    writer.i32(0x12345678);
    let mut reader = Reader::new(&writer.buf);
    let slot = load_object_slot(&mut reader, &ctx, 249, 4)
        .unwrap()
        .unwrap();
    assert_eq!(reader.pos, end);
    assert_eq!(reader.i32().unwrap(), 0x12345678);
    let object = slot.ref_handle();
    let floor = crate::floor_object::runtime::floor_ref(&object).unwrap();
    assert!(floor.borrow().item.is_null());
    assert!(!floor.borrow().item.is_raw());
    assert_eq!(floor.borrow().drop_time, -1.0);
    assert!(!floor.borrow().ignore_remove_sandbox && !floor.borrow().extended_placement);
    assert!(object.borrow().components.is_empty());
}

#[test]
fn actual_floor_data_success_marks_named_native_visual_continuation() {
    let context = pz_entity::BasicContext::default();
    let env = Env {
        codec: EntityCodec { ctx: &context },
        template: Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "FloorLoad")),
    };
    let sprites = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = super::super::SaveTables::for_tests();
    let settings = super::super::SaveSettings::default();
    let ctx = super::super::SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &env,
        hooks: &super::super::NoSaveHooks,
        meta: None,
    };
    let bytes = bytes(&env, &tables, Some(12.5), Some(5));
    let mut reader = Reader::new(&bytes);
    let slot = load_object_slot(&mut reader, &ctx, 249, 4)
        .unwrap()
        .unwrap();
    assert_eq!(reader.pos, bytes.len());
    assert_eq!(
        slot.borrow().pending_floor_visual_tail,
        Some(FloorVisualTail::LoadedTextureAndSprite)
    );
    let object = slot.ref_handle();
    let floor = crate::floor_object::runtime::floor_ref(&object).unwrap();
    assert_eq!(floor.borrow().drop_time, 12.5);
    assert!(floor.borrow().ignore_remove_sandbox && floor.borrow().extended_placement);
    let item = floor.borrow().item.item_ref().unwrap();
    let mut retired = pz_entity::RetiredComponents::new();
    item.borrow_mut().replace_world_item(None, &mut retired);
    pz_inventory::component_host::drain_retired_components();
}

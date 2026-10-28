use super::*;
use crate::archive::ArchiveCopyError;
use crate::{ItemClass, ItemEnv, ItemTemplate};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

struct Env;
impl ItemEnv for Env {}
fn value() -> InventoryItem {
    InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "ModDataTest")),
        1,
    )
}
fn actual() -> ItemRef {
    shared::try_new_item(value()).unwrap()
}
fn set(table: &TableRef, key: Value, value: Value) {
    let mut retired = RetiredTableValues::new();
    table.try_rawset(&key, &value, &mut retired).unwrap();
    retire_values(retired);
}
thread_local! { static OBSERVATIONS: Cell<usize> = const { Cell::new(0) }; }
fn observe(item: &ItemRef) {
    // Metadata callbacks must never receive an item guard.
    let body = item.try_borrow_mut().expect("observer outside item guard");
    let _ = body
        .mod_data_slot()
        .as_ref()
        .and_then(ItemModData::as_live_slot)
        .map(TableRef::ptr_id);
    OBSERVATIONS.with(|n| n.set(n.get() + 1));
}

#[test]
fn getter_is_one_actual_allocation_even_with_held_table_body_and_has_ignores_metatable() {
    let item = actual();
    let old = shared::install_item_observer(Some(observe));
    OBSERVATIONS.with(|n| n.set(0));
    assert!(!has_mod_data_ref(&item).unwrap());
    assert!(item.borrow().table.is_none());
    let table = get_mod_data_ref(&item).unwrap();
    let meta = TableRef::new();
    set(&meta, Value::str("meta"), Value::Num(1.0));
    let mut retired = RetiredTableValues::new();
    table.try_set_metatable(&Some(meta), &mut retired).unwrap();
    assert!(!has_mod_data_ref(&item).unwrap());
    let guard = table.try_borrow_body_mut().unwrap();
    assert!(get_mod_data_ref(&item).unwrap().ptr_eq(&table));
    assert!(matches!(
        has_mod_data_ref(&item),
        Err(ModDataError::Borrowed(_))
    ));
    drop(guard);
    assert_eq!(OBSERVATIONS.with(Cell::get), 5);
    shared::install_item_observer(old);
}

#[test]
fn archive_adoption_moves_authority_once_and_rejects_unknown_unsaveable_value() {
    let mut source = value();
    source.table = Some(
        LuaTable {
            entries: vec![(LuaKey::Str("x".into()), LuaValue::Num(3.0))],
        }
        .into(),
    );
    let source = shared::try_new_item(source).unwrap();
    let table = get_mod_data_ref(&source).unwrap();
    set(&table, Value::str("x"), Value::Num(4.0));
    assert!(matches!(
        source.borrow().mod_data_value("x").unwrap(),
        Some(Value::Num(4.0))
    ));
    assert!(get_mod_data_ref(&source).unwrap().ptr_eq(&table));
    let mut unknown = value();
    unknown.table = Some(
        LuaTable {
            entries: vec![(LuaKey::Str("unknown".into()), LuaValue::Unsaveable)],
        }
        .into(),
    );
    let unknown = shared::try_new_item(unknown).unwrap();
    assert!(matches!(
        get_mod_data_ref(&unknown),
        Err(ModDataError::Unsupported(_))
    ));
    assert!(matches!(
        unknown.borrow().table,
        Some(ItemModData::Archive(_))
    ));
}

#[test]
fn copy_preserves_destination_and_metatable_null_and_self_and_copies_nested_values_per_occurrence()
{
    let item = actual();
    copy_mod_data_ref(&item, None).unwrap();
    assert!(item.borrow().table.is_none());
    let destination = get_mod_data_ref(&item).unwrap();
    let meta = TableRef::new();
    set(&meta, Value::str("fallback"), Value::Bool(true));
    let mut retired = RetiredTableValues::new();
    destination
        .try_set_metatable(&Some(meta.clone()), &mut retired)
        .unwrap();
    let source = TableRef::new();
    let child = TableRef::new();
    set(&child, Value::str("v"), Value::Num(2.0));
    set(&source, Value::str("a"), Value::Table(child.clone()));
    set(&source, Value::str("b"), Value::Table(child.clone()));
    copy_mod_data_ref(&item, Some(&source)).unwrap();
    assert!(get_mod_data_ref(&item).unwrap().ptr_eq(&destination));
    assert!(destination.try_metatable().unwrap().unwrap().ptr_eq(&meta));
    let a = destination
        .try_get_own(&Value::str("a"))
        .unwrap()
        .as_table()
        .unwrap()
        .clone();
    let b = destination
        .try_get_own(&Value::str("b"))
        .unwrap()
        .as_table()
        .unwrap()
        .clone();
    assert!(!a.ptr_eq(&b) && !a.ptr_eq(&child) && !b.ptr_eq(&child));
    copy_mod_data_ref(&item, Some(&destination)).unwrap();
    assert!(destination.try_is_empty().unwrap());
    assert!(destination.try_metatable().unwrap().unwrap().ptr_eq(&meta));
    set(&destination, Value::str("z"), Value::Num(1.0));
    copy_mod_data_ref(&item, None).unwrap();
    assert!(destination.try_is_empty().unwrap());
    assert!(get_mod_data_ref(&item).unwrap().ptr_eq(&destination));
}

#[derive(Debug)]
struct DropProbe {
    item: Weak<RefCell<InventoryItem>>,
    drops: Rc<Cell<usize>>,
}
impl pz_kahlua::LuaObject for DropProbe {
    fn class_name(&self) -> &str {
        "test.DropProbe"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        if let Some(item) = self.item.upgrade() {
            assert!(
                item.try_borrow_mut().is_ok(),
                "last value dropped under item guard"
            );
        }
        self.drops.set(self.drops.get() + 1);
    }
}
#[test]
fn writes_retire_replaced_and_failed_input_values_past_outer_item_guard() {
    crate::component_host::drain_retired_components();
    let item = actual();
    let drops = Rc::new(Cell::new(0));
    let probe = || {
        Value::Object(Rc::new(DropProbe {
            item: Rc::downgrade(&item),
            drops: drops.clone(),
        }))
    };
    set_mod_data_value_ref(&item, "probe", Some(probe())).unwrap();
    crate::component_host::drain_retired_components();
    {
        let mut body = item.borrow_mut();
        body.set_mod_data_value("probe", None).unwrap();
        assert_eq!(drops.get(), 0);
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);
    let table = get_mod_data_ref(&item).unwrap();
    {
        let _table = table.try_borrow_body_mut().unwrap();
        let mut body = item.borrow_mut();
        assert!(body.set_mod_data_value("probe", Some(probe())).is_err());
        assert_eq!(drops.get(), 1);
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 2);
    {
        let _item = item.borrow_mut();
        assert!(set_mod_data_value_ref(&item, "probe", Some(probe())).is_err());
        assert_eq!(drops.get(), 2);
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 3);
}

fn record_table_prefix() -> Writer {
    let mut writer = Writer::new();
    writer.i32(17);
    writer.i8(64);
    writer.i32(1);
    writer
}
#[test]
fn native_load_publishes_new_table_and_keeps_completed_prefix_cursor_and_old_table_on_failure() {
    let item = actual();
    let oldtable = get_mod_data_ref(&item).unwrap();
    set(&oldtable, Value::str("old"), Value::Bool(true));
    let mut writer = record_table_prefix();
    writer.i32(2);
    writer.i8(0);
    writer.string("ok");
    writer.i8(1);
    writer.f64(9.0);
    writer.i8(0);
    writer.string("bad");
    writer.i8(99);
    let mut reader = Reader::new(&writer.buf);
    let oldobserver = shared::install_item_observer(Some(observe));
    OBSERVATIONS.with(|n| n.set(0));
    let failure = crate::component_host::load_item_ref(&item, &mut reader, 249, &Env).unwrap_err();
    assert_eq!(reader.pos, writer.buf.len());
    assert_eq!(failure.position, reader.pos);
    let published = get_mod_data_ref(&item).unwrap();
    assert!(!published.ptr_eq(&oldtable));
    assert!(matches!(
        published.try_get_own(&Value::str("ok")).unwrap(),
        Value::Num(9.0)
    ));
    assert!(published.try_get_own(&Value::str("bad")).unwrap().is_nil());
    assert!(matches!(
        oldtable.try_get_own(&Value::str("old")).unwrap(),
        Value::Bool(true)
    ));
    assert!(OBSERVATIONS.with(Cell::get) >= 2);
    assert!(!failure.retired.is_empty());
    crate::component_host::retire_components(failure.retired);
    shared::install_item_observer(oldobserver);
}
#[test]
fn load_resets_old_slot_before_later_header_fields_and_retains_old_table_without_wiping() {
    let item = actual();
    let old = get_mod_data_ref(&item).unwrap();
    set(&old, Value::str("retained"), Value::Num(5.0));
    let mut writer = Writer::new();
    writer.i32(1);
    writer.i8(32); // missing itemCapacity float
    let mut reader = Reader::new(&writer.buf);
    let failure = crate::component_host::load_item_ref(&item, &mut reader, 249, &Env).unwrap_err();
    assert!(item.borrow().table.is_none());
    assert!(matches!(
        old.try_get_own(&Value::str("retained")).unwrap(),
        Value::Num(5.0)
    ));
    assert!(!failure.retired.is_empty());
    crate::component_host::retire_components(failure.retired);
}
#[test]
fn malformed_string_notice_and_cursor_survive_failed_actual_load() {
    let item = actual();
    let mut writer = record_table_prefix();
    writer.i32(1);
    writer.i8(0);
    writer.i16(20);
    writer.i8(b'x' as i8);
    let mut reader = Reader::new(&writer.buf);
    let mut retired = RetiredComponents::new();
    let mut notices = Vec::new();
    let result = item
        .borrow_mut()
        .load_prefix(&mut reader, 249, &Env, &mut retired, &mut notices);
    assert!(matches!(
        result,
        Err(ModDataError::Native {
            class: "java.lang.IllegalArgumentException",
            ..
        })
    ));
    assert_eq!(reader.pos, 16); // 9-byte item prefix + count/tag/short
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].native_message(),
        "GameWindow.StringUTF.load> numBytes:20 is higher than the remaining bytes in the buffer:1"
    );
    crate::component_host::retire_components(retired);
}
#[test]
fn save_writes_outer_prefix_on_nested_borrow_and_unsaveable_actual_float_keeps_presence_flag() {
    let table = TableRef::new();
    let nested = TableRef::new();
    set(&table, Value::str("first"), Value::Num(7.0));
    set(&table, Value::str("nested"), Value::Table(nested.clone()));
    let authority = ItemModData::Live(table);
    let mut writer = Writer::new();
    writer.i8(42);
    let guard = nested.try_borrow_body_mut().unwrap();
    assert!(matches!(
        authority.save(&mut writer),
        Err(ModDataError::Borrowed(_))
    ));
    assert_eq!(writer.buf[0], 42);
    assert!(writer.buf.len() > 5);
    assert_eq!(&writer.buf[1..5], &2i32.to_be_bytes());
    drop(guard);
    let mut item = value();
    item.set_mod_data_value("float", Some(pz_kahlua::table::BoxedFloat::value(0.5)))
        .unwrap();
    let mut writer = Writer::new();
    item.save(&mut writer, &Env).unwrap();
    let mut reader = Reader::new(&writer.buf);
    reader.i16().unwrap();
    reader.i8().unwrap();
    reader.i32().unwrap();
    let header = reader.i8().unwrap();
    assert_eq!(header & 64, 64);
    if header & 1 != 0 {
        reader.i32().unwrap();
    }
    if header & 4 != 0 {
        reader.i8().unwrap();
    }
    if header & 32 != 0 {
        reader.f32().unwrap();
    }
    assert_eq!(reader.i32().unwrap() & 1, 1);
    assert_eq!(reader.i32().unwrap(), 0);
}
#[test]
fn archive_copy_is_independent_over_key_value_and_metatable_tables_and_rejects_borrows_and_cycles()
{
    let mut item = value();
    let table = item.ensure_live_mod_data().unwrap();
    let child = TableRef::new();
    set(&child, Value::str("v"), Value::Num(2.0));
    set(
        &table,
        Value::Table(child.clone()),
        Value::Table(child.clone()),
    );
    let mut retired = RetiredTableValues::new();
    table
        .try_set_metatable(&Some(child.clone()), &mut retired)
        .unwrap();
    let copied = item.try_archive_copy().unwrap();
    let copied_table = copied.table.as_ref().unwrap().as_live_slot().unwrap();
    assert!(!copied_table.ptr_eq(&table));
    let (key, value) = copied_table.try_entries().unwrap().remove(0);
    let key = key.as_table().unwrap();
    let value = value.as_table().unwrap();
    assert!(key.ptr_eq(value));
    assert!(!key.ptr_eq(&child));
    assert!(copied_table.try_metatable().unwrap().unwrap().ptr_eq(key));
    set(key, Value::str("v"), Value::Num(10.0));
    assert!(matches!(
        child.try_get_own(&Value::str("v")).unwrap(),
        Value::Num(2.0)
    ));
    let guard = child.try_borrow_body_mut().unwrap();
    assert_eq!(
        item.try_archive_copy().unwrap_err(),
        ArchiveCopyError::BorrowedTable
    );
    drop(guard);
    set(&child, Value::str("cycle"), Value::Table(table.clone()));
    assert_eq!(
        item.try_archive_copy().unwrap_err(),
        ArchiveCopyError::CyclicTable
    );
    child.try_wipe(&mut retired).unwrap();
    retire_values(retired);
}
#[test]
fn invalid_actual_type_and_borrow_are_errors_not_missing_model_or_icon() {
    let mut item = value();
    item.set_mod_data_value("staticModel", Some(Value::Bool(true)))
        .unwrap();
    assert!(matches!(
        item.try_get_base_static_model(),
        Err(ModDataError::Native {
            class: "java.lang.ClassCastException",
            ..
        })
    ));
    let table = item.ensure_live_mod_data().unwrap();
    let _guard = table.try_borrow_body_mut().unwrap();
    assert!(matches!(
        item.try_custom_inventory_icon(),
        Err(ModDataError::Borrowed(_))
    ));
}

#[test]
fn used_delta_native_clamp_preserves_negative_zero_and_nan() {
    let mut item = value();
    item.try_set_used_delta(-0.0).unwrap();
    assert_eq!(
        item.try_used_delta().unwrap().to_bits(),
        (-0.0f32).to_bits()
    );
    item.try_set_used_delta(f32::NAN).unwrap();
    assert!(item.try_used_delta().unwrap().is_nan());
    item.try_set_used_delta(-1.0).unwrap();
    assert_eq!(item.try_used_delta().unwrap().to_bits(), 0.0f32.to_bits());
}

thread_local! { static NOTICE_RECEIVER: RefCell<Option<Weak<RefCell<InventoryItem>>>> = const { RefCell::new(None) }; }
fn rejecting_notice_logger(
    _: &pz_entity::runtime_fluid::RuntimeNativeNotice,
) -> pz_entity::runtime_load::HostResult<()> {
    NOTICE_RECEIVER.with(|slot| {
        let item = slot.borrow().as_ref().unwrap().upgrade().unwrap();
        let body = item.try_borrow_mut().expect("notice outside item guard");
        let table = body.table.as_ref().unwrap().as_live_slot().unwrap();
        assert!(
            table.try_borrow_body_mut().is_ok(),
            "notice outside table guard"
        );
    });
    Err(RuntimeHostError::Unsupported {
        operation: "test rejected native notice",
        kind: None,
    })
}
#[test]
fn failed_load_preserves_notice_logger_host_fault_and_completed_retirement_prefix() {
    let item = actual();
    get_mod_data_ref(&item).unwrap();
    NOTICE_RECEIVER.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(&item)));
    let old = crate::component_host::install_native_notice_logger(Some(rejecting_notice_logger));
    let mut writer = record_table_prefix();
    writer.i32(1);
    writer.i8(0);
    writer.i16(20);
    writer.i8(1);
    let mut reader = Reader::new(&writer.buf);
    let failure = crate::component_host::load_item_ref(&item, &mut reader, 249, &Env).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Unsupported {
            operation: "test rejected native notice",
            ..
        }
    ));
    assert_eq!(reader.pos, 16);
    assert!(!failure.retired.is_empty());
    crate::component_host::retire_components(failure.retired);
    crate::component_host::install_native_notice_logger(old);
    NOTICE_RECEIVER.with(|slot| *slot.borrow_mut() = None);
}
#[test]
fn custom_icon_uses_own_nonempty_gate_and_string_instanceof_fallback() {
    let mut item = value();
    let table = item.ensure_live_mod_data().unwrap();
    let meta = TableRef::new();
    set(
        &meta,
        Value::str("customInventoryIcon"),
        Value::str("inherited"),
    );
    let mut retired = RetiredTableValues::new();
    table.try_set_metatable(&Some(meta), &mut retired).unwrap();
    assert_eq!(item.try_custom_inventory_icon().unwrap(), None);
    set(&table, Value::str("unrelated"), Value::Bool(true));
    assert_eq!(
        item.try_custom_inventory_icon().unwrap().as_deref(),
        Some("inherited")
    );
    set(&table, Value::str("customInventoryIcon"), Value::Num(1.0));
    assert_eq!(item.try_custom_inventory_icon().unwrap(), None);
}
#[test]
fn clothing_preparation_preserves_absence_for_plain_items_and_allocates_only_eligible_clothing() {
    let plain = actual();
    prepare_clothing_mod_data_ref(&plain).unwrap();
    assert!(plain.borrow().table.is_none());
    let mut template = ItemTemplate::new(ItemClass::Clothing, "Base", "Mask");
    template.tags.push(crate::gameplay::tags::GAS_MASK.into());
    let mask = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    prepare_clothing_mod_data_ref(&mask).unwrap();
    assert!(matches!(mask.borrow().table, Some(ItemModData::Live(_))));
}

#[test]
fn static_model_preparation_respects_weapon_override_and_world_exception() {
    let plain = actual();
    prepare_static_model_mod_data_ref(&plain).unwrap();
    assert!(plain.borrow().table.is_some());
    let mut template = ItemTemplate::new(ItemClass::Weapon, "Base", "SpriteOnly");
    template.weapon_sprite = Some("Bat".into());
    let weapon = shared::try_new_item(InventoryItem::create(Rc::new(template.clone()), 1)).unwrap();
    prepare_static_model_mod_data_ref(&weapon).unwrap();
    assert!(weapon.borrow().table.is_none());
    template
        .tags
        .push(crate::model::TAG_USE_WORLD_STATIC_MODEL.into());
    let world_weapon = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    prepare_static_model_mod_data_ref(&world_weapon).unwrap();
    assert!(world_weapon.borrow().table.is_some());
}

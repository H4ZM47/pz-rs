use super::*;
use pz_inventory::{shared, ItemTemplate};
use pz_kahlua::{TableRef, Value};

fn thread() -> Thread {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_java_util::register(&mut thread);
    crate::bind_globals::register(&mut thread);
    register(&mut thread);
    thread
}

fn item(thread: &mut Thread, name: &str) -> ItemRef {
    let actual = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", name)),
        1,
    ))
    .unwrap();
    thread
        .globals()
        .set_str("item", try_item_value(&actual).unwrap());
    actual
}

fn run(thread: &mut Thread, source: &str) -> Vec<Value> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual-item-moddata"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall(&function, &[])
        .map_err(|(error, trace)| format!("{}\n{trace}", pz_kahlua::exec::java_value_of(&error)))
        .unwrap()
}

fn all_true(values: &[Value]) {
    assert!(!values.is_empty());
    assert!(
        values
            .iter()
            .all(|value| matches!(value, Value::Bool(true))),
        "{values:?}"
    );
}

#[test]
fn lua_item_mod_data_is_one_actual_table_and_writes_reach_inventory_authority() {
    let mut thread = thread();
    let actual = item(&mut thread, "ActualModData");
    all_true(&run(&mut thread, "return not item:hasModData()"));
    assert!(
        actual.borrow().mod_data_slot().is_none(),
        "presence query allocated a table"
    );
    all_true(&run(
        &mut thread,
        r#"
        retained = item:getModData()
        local empty = not item:hasModData()
        retained.fishing_LineType = 'FishingLine'
        retained.marker = 37
        return empty, retained == item:getModData(), item:hasModData()
    "#,
    ));
    assert_eq!(
        actual
            .borrow()
            .mod_data_value("marker")
            .unwrap()
            .unwrap()
            .as_num(),
        Some(37.0)
    );
    pz_inventory::moddata::set_mod_data_value_ref(&actual, "marker", Some(Value::Num(52.0)))
        .unwrap();
    all_true(&run(
        &mut thread,
        "return retained.marker == 52, retained.fishing_LineType == 'FishingLine'",
    ));
}

#[test]
fn lua_copy_preserves_destination_metatable_and_native_per_occurrence_value_copies() {
    let mut thread = thread();
    let _actual = item(&mut thread, "ActualModDataCopy");
    all_true(&run(
        &mut thread,
        r#"
        retained = item:getModData()
        local meta = {fallback='retained-meta'}
        setmetatable(retained, meta)
        local nested = {marker=11}
        local source = {first=nested, second=nested}
        item:copyModData(source)
        local copied = retained == item:getModData() and getmetatable(retained) == meta
            and retained.first ~= nested and retained.second ~= nested
            and retained.first ~= retained.second and retained.first.marker == 11
        item:CopyModData(nil)
        local cleared = retained == item:getModData() and not item:hasModData()
            and getmetatable(retained) == meta
        retained.marker = 1
        item:copyModData(retained)
        return copied, cleared, not item:hasModData(), retained == item:getModData()
    "#,
    ));
}

#[test]
fn held_table_body_allows_pointer_getter_but_rejects_body_queries_without_panic() {
    let mut thread = thread();
    let actual = item(&mut thread, "ActualModDataBorrow");
    let table = pz_inventory::moddata::get_mod_data_ref(&actual).unwrap();
    thread
        .globals()
        .set_str("same", Value::Table(table.clone()));
    let guard = table.try_borrow_body_mut().unwrap();
    all_true(&run(
        &mut thread,
        r#"
        local ok, message = pcall(function() return item:hasModData() end)
        return item:getModData() == same, not ok, message ~= nil
    "#,
    ));
    drop(guard);
    let guard = actual.borrow_mut();
    all_true(&run(
        &mut thread,
        r#"
        local ok, message = pcall(function() return item:getModData() end)
        return not ok, message ~= nil
    "#,
    ));
    drop(guard);
    all_true(&run(
        &mut thread,
        "return item:getModData() == same, not item:hasModData()",
    ));
}

#[test]
fn clothing_used_delta_preserves_native_nan_and_negative_zero_in_actual_mod_data() {
    let mut thread = thread();
    let actual = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Clothing,
            "Base",
            "ActualDelta",
        )),
        1,
    ))
    .unwrap();
    thread
        .globals()
        .set_str("item", try_item_value(&actual).unwrap());
    thread.globals().set_str("nan", Value::Num(f64::NAN));
    thread.globals().set_str("negativeZero", Value::Num(-0.0));
    let values = run(
        &mut thread,
        "item:setUsedDelta(nan); return item:getUsedDelta()",
    );
    assert!(values[0].as_num().unwrap().is_nan());
    let values = run(
        &mut thread,
        "item:setUsedDelta(negativeZero); return item:getUsedDelta()",
    );
    assert_eq!(values[0].as_num().unwrap().to_bits(), (-0.0f64).to_bits());
    assert_eq!(
        actual
            .borrow()
            .mod_data_value("usedDelta")
            .unwrap()
            .unwrap()
            .as_num()
            .unwrap()
            .to_bits(),
        (-0.0f64).to_bits()
    );
}

#[test]
fn held_actual_item_setter_reports_host_borrow_and_keeps_live_table_unchanged() {
    let mut thread = thread();
    let actual = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Clothing,
            "Base",
            "ActualHeldSetter",
        )),
        1,
    ))
    .unwrap();
    thread
        .globals()
        .set_str("item", try_item_value(&actual).unwrap());
    pz_inventory::moddata::set_mod_data_value_ref(&actual, "usedDelta", Some(Value::Num(0.5)))
        .unwrap();
    let guard = actual.borrow_mut();
    let values = run(
        &mut thread,
        "return pcall(function() item:setUsedDelta(0.75) end)",
    );
    assert!(matches!(values[0], Value::Bool(false)));
    assert!(pz_kahlua::exec::java_value_of(&values[1]).contains("pz.host.Borrowed"));
    drop(guard);
    assert_eq!(
        actual
            .borrow()
            .mod_data_value("usedDelta")
            .unwrap()
            .unwrap()
            .as_num(),
        Some(0.5)
    );
}

#[test]
fn scba_only_filter_and_gasmask_only_tank_entries_do_not_allocate_absent_mod_data() {
    let mut thread = thread();
    let mut template = ItemTemplate::new(ItemClass::Clothing, "Base", "ActualScbaGate");
    template.tags = vec!["base:scba".into()];
    let scba = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    thread
        .globals()
        .set_str("item", try_item_value(&scba).unwrap());
    all_true(&run(
        &mut thread,
        "item:drainGasMask(); item:getCorpseSicknessDefense(); return not item:hasFilter()",
    ));
    assert!(scba.borrow().mod_data_slot().is_none());

    let mut template = ItemTemplate::new(ItemClass::Clothing, "Base", "ActualMaskGate");
    template.tags = vec!["base:gasmask".into()];
    let mask = shared::try_new_item(InventoryItem::create(Rc::new(template), 2)).unwrap();
    thread
        .globals()
        .set_str("item", try_item_value(&mask).unwrap());
    all_true(&run(&mut thread, "return not item:hasTank()"));
    assert!(mask.borrow().mod_data_slot().is_none());
}

#[test]
fn broken_mask_defense_keeps_absence_but_positive_filter_query_reaches_real_table_allocation() {
    let mut thread = thread();
    let mut template = ItemTemplate::new(ItemClass::Clothing, "Base", "ActualBrokenMask");
    template.tags = vec!["base:gasmask".into()];
    let mask = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    mask.borrow_mut().condition = 0;
    thread
        .globals()
        .set_str("item", try_item_value(&mask).unwrap());
    all_true(&run(
        &mut thread,
        "return item:getCorpseSicknessDefense() == 0",
    ));
    assert!(mask.borrow().mod_data_slot().is_none());
    mask.borrow_mut().condition = 1;
    all_true(&run(&mut thread, "return not item:hasFilter()"));
    assert!(
        mask.borrow().mod_data_slot().is_some(),
        "eligible native query must allocate before returning"
    );
    let table = pz_inventory::moddata::get_mod_data_ref(&mask).unwrap();
    assert!(table.try_is_empty().unwrap());
}

#[test]
fn actual_native_item_load_replaces_table_while_retained_lua_table_stays_intact() {
    use pz_inventory::env::NoItemEnv;
    use pz_inventory::io::{Reader, Writer};
    let mut thread = thread();
    let actual = item(&mut thread, "ActualModDataLoad");
    run(
        &mut thread,
        "old = item:getModData(); old.marker = 'original'",
    );
    let source =
        shared::try_new_item(InventoryItem::create(actual.borrow().template.clone(), 2)).unwrap();
    pz_inventory::moddata::set_mod_data_value_ref(&source, "incoming", Some(Value::Num(42.0)))
        .unwrap();
    let mut output = Writer::new();
    source.borrow().save(&mut output, &NoItemEnv).unwrap();
    let mut input = Reader::new(&output.buf);
    input.i16().unwrap();
    input.i8().unwrap();
    let retired = pz_inventory::component_host::load_item_ref(&actual, &mut input, 249, &NoItemEnv)
        .unwrap_or_else(|failure| panic!("actual receiver load: {:?}", failure.error));
    pz_inventory::component_host::retire_components(retired);
    pz_inventory::component_host::drain_retired_components();
    assert_eq!(input.pos, output.buf.len());
    all_true(&run(
        &mut thread,
        r#"
        return old ~= item:getModData(), old.marker == 'original',
            item:getModData().incoming == 42, item:getModData().marker == nil
    "#,
    ));
}

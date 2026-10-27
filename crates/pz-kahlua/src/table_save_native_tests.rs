use super::*;
use crate::table::BoxedFloat;

fn pair(out: &mut Vec<u8>, key: &str, value: f64) {
    out.put_i8(SBYT_STRING);
    out.write_string_utf(Some(key));
    out.put_i8(SBYT_DOUBLE);
    out.put_f64(value);
}
fn is_native(error: TableCodecError, expected: &str) -> bool {
    matches!(error, TableCodecError::Native { class, .. } if class == expected)
}

#[test]
fn read_count_precedes_wipe_negative_count_wipes_and_held_body_consumes_only_count() {
    let table = TableRef::new();
    table.set_str("old", true);
    let mut retired = RetiredTableValues::new();
    let mut short = JavaReader::new(&[0, 0, 0]);
    assert!(is_native(
        try_load_table(&table, &mut short, 249, Flavor::ByteBuffer, &mut retired).unwrap_err(),
        "java.nio.BufferUnderflowException"
    ));
    assert_eq!(short.position(), 0);
    assert!(table.contains_own(&Value::str("old")));
    let guard = table.try_borrow_body().unwrap();
    let mut input = JavaReader::new(&[0, 0, 0, 0, 77]);
    assert!(matches!(
        try_load_table(&table, &mut input, 249, Flavor::ByteBuffer, &mut retired),
        Err(TableCodecError::Access(TableAccessError::Borrowed))
    ));
    assert_eq!(input.position(), 4);
    assert!(retired.is_empty());
    drop(guard);
    let mut input = JavaReader::new(&[255, 255, 255, 255, 77]);
    try_load_table(&table, &mut input, 249, Flavor::ByteBuffer, &mut retired).unwrap();
    assert_eq!(input.position(), 4);
    assert!(table.is_empty());
    assert_eq!(retired.len(), 2);
}

#[test]
fn unknown_key_or_value_tag_preserves_completed_pair_prefix_and_exact_cursor() {
    for key_error in [true, false] {
        let mut data = Vec::new();
        data.put_i32(2);
        pair(&mut data, "done", 3.0);
        if !key_error {
            data.put_i8(SBYT_STRING);
            data.write_string_utf(Some("bad"));
        }
        data.put_i8(99);
        let expected = data.len();
        data.extend([1, 2, 3]);
        let table = TableRef::new();
        let mut input = JavaReader::new(&data);
        let mut retired = RetiredTableValues::new();
        let error =
            try_load_table(&table, &mut input, 249, Flavor::ByteBuffer, &mut retired).unwrap_err();
        assert!(
            matches!(error, TableCodecError::Native { class: "java.lang.RuntimeException", message } if message == "invalid lua table type 99")
        );
        assert_eq!(input.position(), expected);
        assert_eq!(table.size(), 1);
        assert_eq!(table.get_str("done").as_num(), Some(3.0));
    }
}

#[test]
fn truncated_nested_value_never_publishes_partial_child_to_parent() {
    let mut data = Vec::new();
    data.put_i32(2);
    pair(&mut data, "done", 2.0);
    data.put_i8(SBYT_STRING);
    data.write_string_utf(Some("child"));
    data.put_i8(SBYT_TABLE);
    data.put_i32(2);
    pair(&mut data, "nestedDone", 4.0);
    data.put_i8(SBYT_STRING);
    data.write_string_utf(Some("broken"));
    data.put_i8(SBYT_DOUBLE);
    let expected = data.len();
    data.extend([0, 0, 0]);
    let table = TableRef::new();
    let mut input = JavaReader::new(&data);
    let mut retired = RetiredTableValues::new();
    assert!(is_native(
        try_load_table(&table, &mut input, 249, Flavor::ByteBuffer, &mut retired).unwrap_err(),
        "java.nio.BufferUnderflowException"
    ));
    assert_eq!(input.position(), expected);
    assert_eq!(table.size(), 1);
    assert_eq!(table.get_str("done").as_num(), Some(2.0));
    assert!(table.get_str("child").is_nil());
}

#[test]
fn truncated_nested_key_is_not_published_and_string_limit_failure_keeps_notice_and_cursor() {
    let mut data = Vec::new();
    data.put_i32(2);
    pair(&mut data, "done", 1.0);
    data.put_i8(SBYT_TABLE);
    data.put_i32(1);
    data.put_i8(SBYT_STRING);
    data.put_i16(30);
    let expected_position = data.len();
    data.push(b'x');
    let table = TableRef::new();
    let mut input = JavaReader::new(&data);
    let mut retired = RetiredTableValues::new();
    let error =
        try_load_table(&table, &mut input, 249, Flavor::ByteBuffer, &mut retired).unwrap_err();
    assert!(is_native(error, "java.lang.IllegalArgumentException"));
    assert_eq!(input.position(), expected_position);
    assert_eq!(input.limit(), data.len());
    assert_eq!(table.size(), 1);
    assert_eq!(table.get_str("done").as_num(), Some(1.0));
    let notices = input.take_notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].native_message(),
        "GameWindow.StringUTF.load> numBytes:30 is higher than the remaining bytes in the buffer:1"
    );
}

#[test]
fn known_boolean_and_table_keys_load_in_25_and_249_but_native_save_filters_them() {
    for version in [25, 249] {
        let mut data = Vec::new();
        data.put_i32(2);
        data.extend([SBYT_BOOLEAN as u8, 2, SBYT_STRING as u8]);
        data.write_string_utf(Some("boolean"));
        data.put_i8(SBYT_TABLE);
        data.put_i32(0);
        data.put_i8(SBYT_BOOLEAN);
        data.put_i8(1);
        let table = TableRef::new();
        let mut input = JavaReader::new(&data);
        let mut retired = RetiredTableValues::new();
        try_load_table(
            &table,
            &mut input,
            version,
            Flavor::ByteBuffer,
            &mut retired,
        )
        .unwrap();
        assert_eq!(input.position(), data.len());
        assert_eq!(table.size(), 2);
        assert_eq!(table.get_own(&Value::Bool(true)).as_str(), Some("boolean"));
        assert!(matches!(table.entries()[1].0, Value::Table(_)));
        let mut output = vec![55];
        try_save_table(&table, &mut output, Flavor::ByteBuffer).unwrap();
        assert_eq!(output, [55, 0, 0, 0, 0]);
        assert_eq!(table.size(), 2);
    }
}

#[test]
fn version_24_reads_value_tag_before_string_key_and_keeps_completed_prefix() {
    let mut data = Vec::new();
    data.put_i32(2);
    data.put_i8(SBYT_BOOLEAN);
    data.write_string_utf(Some("first"));
    data.put_i8(2);
    data.put_i8(99);
    data.write_string_utf(Some("second"));
    let expected = data.len();
    let table = TableRef::new();
    let mut input = JavaReader::new(&data);
    let mut retired = RetiredTableValues::new();
    assert!(is_native(
        try_load_table(&table, &mut input, 24, Flavor::ByteBuffer, &mut retired).unwrap_err(),
        "java.lang.RuntimeException"
    ));
    assert_eq!(input.position(), expected);
    assert_eq!(table.size(), 1);
    assert!(matches!(table.get_str("first"), Value::Bool(true)));
}

#[test]
fn save_nested_borrow_failure_keeps_outer_header_completed_pair_and_nested_tag() {
    let table = TableRef::new();
    table.set_str("first", 7.0);
    let child = TableRef::new();
    table.set_str("child", child.clone());
    let guard = child.try_borrow_body_mut().unwrap();
    let mut output = vec![123];
    assert!(matches!(
        try_save_table(&table, &mut output, Flavor::ByteBuffer),
        Err(TableCodecError::Access(TableAccessError::Borrowed))
    ));
    let mut expected = vec![123];
    expected.put_i32(2);
    pair(&mut expected, "first", 7.0);
    expected.put_i8(SBYT_STRING);
    expected.write_string_utf(Some("child"));
    expected.put_i8(SBYT_TABLE);
    assert_eq!(output, expected);
    drop(guard);
    assert_eq!(table.size(), 2);
}

#[test]
fn save_filters_real_boxed_float_and_arbitrary_values_without_replacing_table() {
    let table = TableRef::new();
    let float = BoxedFloat::value(0.75);
    let function = Value::Native(crate::NativeFunction::new("unsaved", |_| Ok(0)));
    table.set_str("float", float.clone());
    table.set_str("fn", function.clone());
    table.rawset(Value::Bool(true), Value::str("bool key"));
    table.rawset(Value::Table(table.clone()), Value::str("actual table key"));
    let mut output = Vec::new();
    try_save_table(&table, &mut output, Flavor::ByteBuffer).unwrap();
    assert_eq!(output, [0, 0, 0, 0]);
    assert_eq!(table.size(), 4);
    assert!(table.get_str("float").raw_equals(&float));
    assert!(table.get_str("fn").raw_equals(&function));
    table.rawset(Value::Table(table.clone()), Value::Nil);
}

#[test]
fn copy_preserves_destination_meta_splits_nested_aliases_and_keeps_actual_keys_objects() {
    let source = TableRef::new();
    let child = TableRef::new();
    child.set_str("n", 1.0);
    let key = Value::Table(TableRef::new());
    let function = Value::Native(crate::NativeFunction::new("same", |_| Ok(0)));
    source.rawset(key.clone(), function.clone());
    source.set_str("a", child.clone());
    source.set_str("b", child.clone());
    let child_meta = TableRef::new();
    child.set_metatable(Some(child_meta));
    let target = TableRef::new();
    let meta = TableRef::new();
    target.set_metatable(Some(meta.clone()));
    target.set_str("old", false);
    let mut retired = RetiredTableValues::new();
    copy_table_values(&target, Some(&source), &mut retired).unwrap();
    assert!(target.metatable().unwrap().ptr_eq(&meta));
    assert!(target.get_own(&key).raw_equals(&function));
    let a = target.get_str("a").as_table().unwrap().clone();
    let b = target.get_str("b").as_table().unwrap().clone();
    assert!(!a.ptr_eq(&b) && !a.ptr_eq(&child) && !b.ptr_eq(&child));
    assert!(a.metatable().is_none());
    a.set_str("n", 9.0);
    assert_eq!(b.get_str("n").as_num(), Some(1.0));
    assert_eq!(child.get_str("n").as_num(), Some(1.0));
    assert_eq!(retired.len(), 2);
}

#[test]
fn copy_null_and_self_wipe_existing_destination_without_replacing_meta_or_identity() {
    let target = TableRef::new();
    let alias = target.clone();
    let meta = TableRef::new();
    target.set_metatable(Some(meta.clone()));
    target.set_str("one", true);
    let mut retired = RetiredTableValues::new();
    copy_table_values(&target, None, &mut retired).unwrap();
    assert!(
        target.ptr_eq(&alias) && target.is_empty() && target.metatable().unwrap().ptr_eq(&meta)
    );
    target.set_str("again", true);
    copy_table_values(&target, Some(&target), &mut retired).unwrap();
    assert!(
        target.ptr_eq(&alias) && target.is_empty() && target.metatable().unwrap().ptr_eq(&meta)
    );
    assert_eq!(retired.len(), 4);
}

#[test]
fn copy_child_borrow_failure_preserves_previous_completed_target_pairs_and_old_retirements() {
    let source = TableRef::new();
    source.set_str("first", 2.0);
    let child = TableRef::new();
    source.set_str("child", child.clone());
    let target = TableRef::new();
    target.set_str("old", true);
    let guard = child.try_borrow_body_mut().unwrap();
    let mut retired = RetiredTableValues::new();
    assert!(matches!(
        copy_table_values(&target, Some(&source), &mut retired),
        Err(TableCodecError::Access(TableAccessError::Borrowed))
    ));
    assert_eq!(target.size(), 1);
    assert_eq!(target.get_str("first").as_num(), Some(2.0));
    assert!(target.get_str("child").is_nil());
    assert_eq!(retired.len(), 2);
    drop(guard);
}

#[test]
fn runtime_cycles_are_real_and_persistence_copy_stop_at_explicit_host_boundary() {
    let source = TableRef::new();
    source.set_str("first", 4.0);
    source.set_str("self", source.clone());
    let mut bytes = Vec::new();
    assert!(matches!(
        try_save_table(&source, &mut bytes, Flavor::ByteBuffer),
        Err(TableCodecError::Unsupported { .. })
    ));
    assert!(bytes.len() > MAX_DEPTH * 4);
    let target = TableRef::new();
    let mut retired = RetiredTableValues::new();
    assert!(matches!(
        copy_table_values(&target, Some(&source), &mut retired),
        Err(TableCodecError::Unsupported { .. })
    ));
    assert_eq!(target.size(), 1);
    assert_eq!(target.get_str("first").as_num(), Some(4.0));
    assert!(target.get_str("self").is_nil());
    assert!(source.get_str("self").as_table().unwrap().ptr_eq(&source));
    source.set_str("self", Value::Nil);
}

use pz_kahlua::{TableRef, Thread, Value};

use super::fetch::compare_type_for_test as compare_type;

#[test]
fn compare_type_matches_one_slash_separated_entry() {
    assert!(compare_type("Sledgehammer", "Sledgehammer"));
    assert!(!compare_type("Sledgehammer", "Sledgehammer2"));
    assert!(compare_type("Hammer/Sledgehammer", "Sledgehammer"));
    assert!(compare_type("Sledgehammer/Hammer", "Sledgehammer"));
    assert!(compare_type("A/Sledgehammer/B", "Sledgehammer"));
    assert!(!compare_type("A/Sledgehammer2", "Sledgehammer"));
    assert!(!compare_type("A/B", "C"));
}

#[test]
fn append_and_contains_value_use_identity() {
    let t = TableRef::new();
    let a = Value::Table(TableRef::new());
    let b = Value::Table(TableRef::new());
    super::append(&t, a.clone());
    assert!(super::contains_value(Some(&t), &a));
    assert!(!super::contains_value(Some(&t), &b));
    assert!(!super::contains_value(None, &a));
    super::append(&t, b.clone());
    assert_eq!(t.len(), 2);
    assert!(super::contains_key(&t, &Value::Num(2.0)));
}

#[test]
fn registers_the_static_class() {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    super::register(&mut t);
    let class = t.globals().rawget(&Value::str("ISWorldObjectContextMenuLogic"));
    let class = class.as_table().expect("global class table");
    for name in ["fetch", "createMenuEntries", "callCustomFunction"] {
        assert!(matches!(class.rawget(&Value::str(name)), Value::Native(_)), "{name}");
    }
}

#[test]
fn fetch_with_a_null_fetch_table_throws_a_null_pointer() {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    super::register(&mut t);
    let f = pz_kahlua::stdlib::load_string(
        "return pcall(ISWorldObjectContextMenuLogic.fetch, nil, nil, 0, true)",
        Some("test"),
        t.globals().clone(),
    )
    .unwrap();
    let r = t.call(&f, &[]).unwrap();
    assert!(matches!(r[0], Value::Bool(false)));
}

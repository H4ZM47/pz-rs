use super::*;

#[test]
fn direct_sink_first_failure_precedes_nested_body_access() {
    struct Sink(Vec<&'static str>);
    impl TableSaveWriter for Sink {
        fn write_i8(&mut self, _: i8) -> Result<(), TableCodecError> {
            self.0.push("tag");
            Ok(())
        }
        fn write_i32(&mut self, _: i32) -> Result<(), TableCodecError> {
            self.0.push("count");
            Ok(())
        }
        fn write_f64(&mut self, _: f64) -> Result<(), TableCodecError> {
            self.0.push("double");
            Ok(())
        }
        fn write_string(&mut self, _: &str, _: Flavor) -> Result<(), TableCodecError> {
            self.0.push("string");
            Err(TableCodecError::Unsupported {
                capability: "first sink string fault",
            })
        }
    }
    let root = TableRef::new();
    let child = TableRef::new();
    root.set_str("child", Value::Table(child.clone()));
    let held = child.try_borrow_body_mut().unwrap();
    let mut sink = Sink(Vec::new());
    assert!(matches!(
        try_save_table_to(&root, &mut sink, Flavor::ByteBuffer),
        Err(TableCodecError::Unsupported {
            capability: "first sink string fault"
        })
    ));
    assert_eq!(sink.0, ["count", "tag", "string"]);
    drop(held);
}

#[test]
fn direct_sink_keeps_parent_bytes_before_held_child_count() {
    struct Sink(Vec<u8>);
    impl TableSaveWriter for Sink {
        fn write_i8(&mut self, value: i8) -> Result<(), TableCodecError> {
            self.0.write_i8(value)
        }
        fn write_i32(&mut self, value: i32) -> Result<(), TableCodecError> {
            self.0.write_i32(value)
        }
        fn write_f64(&mut self, value: f64) -> Result<(), TableCodecError> {
            self.0.write_f64(value)
        }
        fn write_string(&mut self, value: &str, flavor: Flavor) -> Result<(), TableCodecError> {
            self.0.write_string(value, flavor)
        }
    }
    let root = TableRef::new();
    let child = TableRef::new();
    root.set_str("child", Value::Table(child.clone()));
    let held = child.try_borrow_body_mut().unwrap();
    let mut sink = Sink(vec![42]);
    assert!(matches!(
        try_save_table_to(&root, &mut sink, Flavor::ByteBuffer),
        Err(TableCodecError::Access(TableAccessError::Borrowed))
    ));
    assert_eq!(
        sink.0,
        [42, 0, 0, 0, 1, 0, 0, 5, b'c', b'h', b'i', b'l', b'd', 2]
    );
    drop(held);
}

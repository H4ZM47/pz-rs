//! `StringLib` (placeholder; being ported).
use crate::table::TableRef;
use crate::vm::Thread;

pub fn register(thread: &mut Thread, env: &TableRef) {
    let string = TableRef::new();
    string.set_str("__index", string.clone());
    thread.class_metatables.set_str("java.lang.String", string.clone());
    env.set_str("string", string);
}

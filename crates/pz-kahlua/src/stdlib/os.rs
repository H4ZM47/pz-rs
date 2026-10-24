//! `OsLib` (placeholder; being ported).
use crate::table::TableRef;

pub fn register(env: &TableRef) {
    env.set_str("os", TableRef::new());
}

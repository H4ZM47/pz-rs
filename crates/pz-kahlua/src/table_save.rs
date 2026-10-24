//! `KahluaTableImpl.save` / `load`: the binary table format of every save
//! file that carries Lua mod data (`map_t.bin`, `gos_*.bin`, ...).
//!
//! A table is a 32-bit entry count, then per entry a key and a value, each a
//! type byte (`0` string, `1` double, `2` table, `3` boolean) and its
//! payload. Keys can only be strings or numbers; entries with any other key
//! or value type are skipped. Entries are written in iteration (insertion)
//! order.
//!
//! The original has two copies of the code, for `ByteBuffer` and for
//! `DataOutputStream`, that differ in their string encoding (see
//! [`crate::java_io`]), in how a boolean is read (`!= 0` vs `== 1`), and in
//! that only the `ByteBuffer` load wipes the table first. [`Flavor`] picks
//! one.

use crate::java_io::{JavaReader, JavaWrite};
use crate::{TableRef, Value};
use std::io;

/// Which of the original's two serialisers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    /// `save(ByteBuffer)` / `load(ByteBuffer, int)`.
    ByteBuffer,
    /// `save(DataOutputStream)` / `load(DataInputStream, int)`.
    DataStream,
}

const SBYT_STRING: i8 = 0;
const SBYT_DOUBLE: i8 = 1;
const SBYT_TABLE: i8 = 2;
const SBYT_BOOLEAN: i8 = 3;

/// Deeper nesting than this is an error instead of a stack overflow (the
/// original throws `StackOverflowError` on a cyclic table).
const MAX_DEPTH: usize = 2000;

fn key_byte(v: &Value) -> Option<i8> {
    match v {
        Value::Str(_) => Some(SBYT_STRING),
        Value::Num(_) => Some(SBYT_DOUBLE),
        _ => None,
    }
}

fn value_byte(v: &Value) -> Option<i8> {
    match v {
        Value::Str(_) => Some(SBYT_STRING),
        Value::Num(_) => Some(SBYT_DOUBLE),
        Value::Bool(_) => Some(SBYT_BOOLEAN),
        Value::Table(_) => Some(SBYT_TABLE),
        _ => None,
    }
}

/// `KahluaTableImpl.canSave(key, value)`.
pub fn can_save(key: &Value, value: &Value) -> bool {
    key_byte(key).is_some() && value_byte(value).is_some()
}

/// `table.save(output)`.
pub fn save_table(t: &TableRef, out: &mut Vec<u8>, flavor: Flavor) -> io::Result<()> {
    save_inner(t, out, flavor, 0)
}

fn save_inner(t: &TableRef, out: &mut Vec<u8>, flavor: Flavor, depth: usize) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(
            "StackOverflowError: table nesting too deep",
        ));
    }
    // The original iterates a key snapshot twice: once to count, once to write.
    let entries = t.entries();
    let count = entries.iter().filter(|(k, v)| can_save(k, v)).count();
    out.put_i32(count as i32);
    for (k, v) in &entries {
        let (Some(kb), Some(vb)) = (key_byte(k), value_byte(v)) else {
            continue;
        };
        save_value(out, kb, k, flavor, depth)?;
        save_value(out, vb, v, flavor, depth)?;
    }
    Ok(())
}

fn save_value(
    out: &mut Vec<u8>,
    sbyt: i8,
    v: &Value,
    flavor: Flavor,
    depth: usize,
) -> io::Result<()> {
    out.put_i8(sbyt);
    match v {
        Value::Str(s) => match flavor {
            Flavor::ByteBuffer => out.write_string_utf(Some(s)),
            Flavor::DataStream => out.write_string_chars(Some(s)),
        },
        Value::Num(n) => out.put_f64(*n),
        Value::Bool(b) => out.put_bool(*b),
        Value::Table(t) => save_inner(t, out, flavor, depth + 1)?,
        _ => unreachable!("filtered by value_byte"),
    }
    Ok(())
}

/// `table.load(input, worldVersion)`.
pub fn load_table(
    t: &TableRef,
    input: &mut JavaReader<'_>,
    world_version: i32,
    flavor: Flavor,
) -> io::Result<()> {
    load_inner(t, input, world_version, flavor, 0)
}

fn load_inner(
    t: &TableRef,
    input: &mut JavaReader<'_>,
    world_version: i32,
    flavor: Flavor,
    depth: usize,
) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(
            "StackOverflowError: table nesting too deep",
        ));
    }
    let count = input.get_i32()?;
    if flavor == Flavor::ByteBuffer {
        t.wipe();
    }
    for _ in 0..count.max(0) {
        if world_version >= 25 {
            let kb = input.get_i8()?;
            let key = load_value(input, world_version, kb, flavor, depth)?;
            let vb = input.get_i8()?;
            let value = load_value(input, world_version, vb, flavor, depth)?;
            t.rawset(key, value);
        } else {
            let vb = input.get_i8()?;
            let key = Value::str(&read_string(input, flavor)?);
            let value = load_value(input, world_version, vb, flavor, depth)?;
            t.rawset(key, value);
        }
    }
    Ok(())
}

fn read_string(input: &mut JavaReader<'_>, flavor: Flavor) -> io::Result<String> {
    match flavor {
        Flavor::ByteBuffer => input.read_string_utf(),
        Flavor::DataStream => input.read_string_chars(),
    }
}

fn load_value(
    input: &mut JavaReader<'_>,
    world_version: i32,
    sbyt: i8,
    flavor: Flavor,
    depth: usize,
) -> io::Result<Value> {
    Ok(match sbyt {
        SBYT_STRING => Value::str(&read_string(input, flavor)?),
        SBYT_DOUBLE => Value::Num(input.get_f64()?),
        SBYT_BOOLEAN => {
            let b = input.get_i8()?;
            Value::Bool(match flavor {
                Flavor::ByteBuffer => b != 0,
                Flavor::DataStream => b == 1,
            })
        }
        SBYT_TABLE => {
            let sub = TableRef::new();
            load_inner(&sub, input, world_version, flavor, depth + 1)?;
            Value::Table(sub)
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid lua table type {other}"),
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TableRef {
        let t = TableRef::new();
        t.set_str("name", Value::str("x"));
        t.set_int(1, 2.5);
        t.set_str("flag", true);
        t.set_str(
            "fn",
            Value::Native(crate::NativeFunction::new("f", |_| Ok(0))),
        );
        let sub = TableRef::new();
        sub.set_str("a", false);
        t.set_str("sub", Value::Table(sub));
        t.rawset(Value::Bool(true), Value::str("bool key is skipped"));
        t
    }

    #[test]
    fn bytebuffer_layout() {
        let mut out = Vec::new();
        save_table(&sample(), &mut out, Flavor::ByteBuffer).unwrap();
        let mut want = vec![0, 0, 0, 4];
        want.extend([0, 0, 4, b'n', b'a', b'm', b'e', 0, 0, 1, b'x']);
        want.extend([1]);
        want.extend(1.0f64.to_be_bytes());
        want.extend([1]);
        want.extend(2.5f64.to_be_bytes());
        want.extend([0, 0, 4, b'f', b'l', b'a', b'g', 3, 1]);
        want.extend([
            0, 0, 3, b's', b'u', b'b', 2, 0, 0, 0, 1, 0, 0, 1, b'a', 3, 0,
        ]);
        assert_eq!(out, want);
    }

    #[test]
    fn round_trips_both_flavors() {
        for flavor in [Flavor::ByteBuffer, Flavor::DataStream] {
            let mut out = Vec::new();
            save_table(&sample(), &mut out, flavor).unwrap();
            let t = TableRef::new();
            load_table(&t, &mut JavaReader::new(&out), 249, flavor).unwrap();
            let mut again = Vec::new();
            save_table(&t, &mut again, flavor).unwrap();
            assert_eq!(out, again);
            assert_eq!(t.size(), 4);
        }
    }

    #[test]
    fn flavor_differences() {
        // A boolean byte of 2 is true for ByteBuffer, false for DataStream.
        let data = [0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 3, 2];
        let t = TableRef::new();
        load_table(&t, &mut JavaReader::new(&data), 249, Flavor::ByteBuffer).unwrap();
        assert!(matches!(t.get_int(0), Value::Bool(true)));
        // Only the ByteBuffer load wipes the table first.
        let t = TableRef::new();
        t.set_str("old", 1.0);
        load_table(&t, &mut JavaReader::new(&data), 249, Flavor::DataStream).unwrap();
        assert!(matches!(t.get_int(0), Value::Bool(false)));
        assert_eq!(t.size(), 2);
        load_table(&t, &mut JavaReader::new(&data), 249, Flavor::ByteBuffer).unwrap();
        assert_eq!(t.size(), 1);
    }

    #[test]
    fn old_format_has_string_keys_after_the_type() {
        // worldVersion < 25: value type, key string, value.
        let data = [0, 0, 0, 1, 3, 0, 0, 0, 1, 0, b'k', 1];
        let t = TableRef::new();
        load_table(&t, &mut JavaReader::new(&data), 24, Flavor::DataStream).unwrap();
        assert!(matches!(t.get_str("k"), Value::Bool(true)));
    }

    #[test]
    fn invalid_type_is_an_error() {
        let data = [0, 0, 0, 1, 9];
        assert!(load_table(
            &TableRef::new(),
            &mut JavaReader::new(&data),
            249,
            Flavor::ByteBuffer
        )
        .is_err());
    }
}

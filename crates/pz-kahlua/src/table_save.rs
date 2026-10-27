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
use crate::table::{RetiredTableValues, TableAccessError};
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

/// Explicit host safety boundary. Native has no such depth counter; this is
/// never reported as a measured native StackOverflowError.
const MAX_DEPTH: usize = 2000;

#[derive(Debug)]
pub enum TableCodecError {
    Access(TableAccessError),
    Native {
        class: &'static str,
        message: String,
    },
    Unsupported {
        capability: &'static str,
    },
}

impl std::fmt::Display for TableCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Access(error) => error.fmt(f),
            Self::Native { class, message } if message.is_empty() => f.write_str(class),
            Self::Native { class, message } => write!(f, "{class}: {message}"),
            Self::Unsupported { capability } => write!(f, "host unsupported: {capability}"),
        }
    }
}
impl std::error::Error for TableCodecError {}
impl From<TableAccessError> for TableCodecError {
    fn from(error: TableAccessError) -> Self {
        Self::Access(error)
    }
}

fn input_error(error: io::Error, flavor: Flavor) -> TableCodecError {
    let text = error.to_string();
    let class = if error.kind() == io::ErrorKind::UnexpectedEof {
        match flavor {
            Flavor::ByteBuffer => "java.nio.BufferUnderflowException",
            Flavor::DataStream => "java.io.EOFException",
        }
    } else if text.starts_with("IllegalArgumentException:") {
        "java.lang.IllegalArgumentException"
    } else if text.starts_with("NegativeArraySizeException:") {
        "java.lang.NegativeArraySizeException"
    } else {
        "java.lang.RuntimeException"
    };
    TableCodecError::Native {
        class,
        message: if error.kind() == io::ErrorKind::UnexpectedEof {
            // Native relative ByteBuffer getters/stream primitives throw
            // these exceptions without a diagnostic message.
            String::new()
        } else {
            text.strip_prefix("IllegalArgumentException: ")
                .or_else(|| text.strip_prefix("NegativeArraySizeException: "))
                .unwrap_or(&text)
                .to_string()
        },
    }
}
fn depth_check(depth: usize) -> Result<(), TableCodecError> {
    if depth > MAX_DEPTH {
        Err(TableCodecError::Unsupported {
            capability: "table nesting exceeds explicit host depth boundary (2000)",
        })
    } else {
        Ok(())
    }
}

fn legacy_io_error(error: TableCodecError) -> io::Error {
    match error {
        TableCodecError::Native { class, message } => {
            let kind = if class == "java.nio.BufferUnderflowException"
                || class == "java.io.EOFException"
            {
                io::ErrorKind::UnexpectedEof
            } else {
                io::ErrorKind::InvalidData
            };
            let message = if message.is_empty() {
                class.rsplit('.').next().unwrap_or(class).to_string()
            } else {
                message
            };
            io::Error::new(kind, message)
        }
        error => io::Error::other(error),
    }
}

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
    try_save_table(t, out, flavor).map_err(legacy_io_error)
}

struct SaveFrame {
    _table: TableRef,
    entries: std::vec::IntoIter<(Value, Value)>,
    depth: usize,
}

fn save_frame(
    table: &TableRef,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<SaveFrame, TableCodecError> {
    depth_check(depth)?;
    let entries = table.try_entries()?;
    out.put_i32(
        entries
            .iter()
            .filter(|(key, value)| can_save(key, value))
            .count() as i32,
    );
    Ok(SaveFrame {
        _table: table.clone(),
        entries: entries.into_iter(),
        depth,
    })
}

fn save_scalar(out: &mut Vec<u8>, sbyt: i8, value: &Value, flavor: Flavor) {
    out.put_i8(sbyt);
    match value {
        Value::Str(s) => match flavor {
            Flavor::ByteBuffer => out.write_string_utf(Some(s)),
            Flavor::DataStream => out.write_string_chars(Some(s)),
        },
        Value::Num(n) => out.put_f64(*n),
        Value::Bool(b) => out.put_bool(*b),
        _ => unreachable!("scalar tag selected only after native save filtering"),
    }
}

/// Saves directly into the caller's output, including the completed prefix
/// when a later nested table cannot be borrowed. No success-only staging.
pub fn try_save_table(
    table: &TableRef,
    out: &mut Vec<u8>,
    flavor: Flavor,
) -> Result<(), TableCodecError> {
    let mut stack = vec![save_frame(table, out, 0)?];
    while let Some(frame) = stack.last_mut() {
        let depth = frame.depth;
        let Some((key, value)) = frame.entries.next() else {
            stack.pop();
            continue;
        };
        let (Some(kb), Some(vb)) = (key_byte(&key), value_byte(&value)) else {
            continue;
        };
        save_scalar(out, kb, &key, flavor);
        if let Value::Table(child) = value {
            out.put_i8(vb);
            stack.push(save_frame(&child, out, depth + 1)?);
        } else {
            save_scalar(out, vb, &value, flavor);
        }
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
    // Compatibility wrapper. Actual hosts use try_load_table with an outer
    // retirement sink so removed arbitrary runtime values outlive guards.
    let mut retired = RetiredTableValues::new();
    try_load_table(t, input, world_version, flavor, &mut retired).map_err(legacy_io_error)
}

fn begin_load(
    table: &TableRef,
    input: &mut JavaReader<'_>,
    flavor: Flavor,
    depth: usize,
    retired: &mut RetiredTableValues,
) -> Result<i32, TableCodecError> {
    depth_check(depth)?;
    // Count is consumed before wipe, including for a held destination body.
    let count = input.get_i32().map_err(|e| input_error(e, flavor))?;
    if flavor == Flavor::ByteBuffer {
        table.try_wipe(retired)?;
    }
    Ok(count)
}

fn read_string(input: &mut JavaReader<'_>, flavor: Flavor) -> Result<String, TableCodecError> {
    match flavor {
        Flavor::ByteBuffer => input.read_string_utf(),
        Flavor::DataStream => input.read_string_chars(),
    }
    .map_err(|e| input_error(e, flavor))
}

fn load_scalar(
    input: &mut JavaReader<'_>,
    sbyt: i8,
    flavor: Flavor,
) -> Result<Value, TableCodecError> {
    Ok(match sbyt {
        SBYT_STRING => Value::str(&read_string(input, flavor)?),
        SBYT_DOUBLE => Value::Num(input.get_f64().map_err(|e| input_error(e, flavor))?),
        SBYT_BOOLEAN => {
            let b = input.get_i8().map_err(|e| input_error(e, flavor))?;
            Value::Bool(match flavor {
                Flavor::ByteBuffer => b != 0,
                Flavor::DataStream => b == 1,
            })
        }
        other => {
            return Err(TableCodecError::Native {
                class: "java.lang.RuntimeException",
                message: format!("invalid lua table type {other}"),
            });
        }
    })
}

enum LoadTask {
    Pair {
        table: TableRef,
        remaining: i32,
        depth: usize,
    },
    AfterKey {
        table: TableRef,
        remaining: i32,
        depth: usize,
        key: TableRef,
    },
    Value {
        table: TableRef,
        remaining: i32,
        depth: usize,
        key: Value,
        tag: Option<i8>,
    },
    Publish {
        table: TableRef,
        remaining: i32,
        depth: usize,
        key: Value,
        value: TableRef,
    },
}

/// Loads into the actual destination. Only complete pairs are published;
/// nested value tables remain unpublished until their complete load returns.
pub fn try_load_table(
    table: &TableRef,
    input: &mut JavaReader<'_>,
    world_version: i32,
    flavor: Flavor,
    retired: &mut RetiredTableValues,
) -> Result<(), TableCodecError> {
    let count = begin_load(table, input, flavor, 0, retired)?;
    let mut stack = vec![LoadTask::Pair {
        table: table.clone(),
        remaining: count,
        depth: 0,
    }];
    while let Some(task) = stack.pop() {
        match task {
            LoadTask::Pair {
                table,
                remaining,
                depth,
            } => {
                if remaining <= 0 {
                    continue;
                }
                let tag = input.get_i8().map_err(|e| input_error(e, flavor))?;
                if world_version < 25 {
                    let key = Value::str(&read_string(input, flavor)?);
                    stack.push(LoadTask::Value {
                        table,
                        remaining,
                        depth,
                        key,
                        tag: Some(tag),
                    });
                } else if tag == SBYT_TABLE {
                    let key = TableRef::new();
                    let count = begin_load(&key, input, flavor, depth + 1, retired)?;
                    stack.push(LoadTask::AfterKey {
                        table,
                        remaining,
                        depth,
                        key: key.clone(),
                    });
                    stack.push(LoadTask::Pair {
                        table: key,
                        remaining: count,
                        depth: depth + 1,
                    });
                } else {
                    let key = load_scalar(input, tag, flavor)?;
                    stack.push(LoadTask::Value {
                        table,
                        remaining,
                        depth,
                        key,
                        tag: None,
                    });
                }
            }
            LoadTask::AfterKey {
                table,
                remaining,
                depth,
                key,
            } => {
                stack.push(LoadTask::Value {
                    table,
                    remaining,
                    depth,
                    key: Value::Table(key),
                    tag: None,
                });
            }
            LoadTask::Value {
                table,
                remaining,
                depth,
                key,
                tag,
            } => {
                let tag = match tag {
                    Some(tag) => tag,
                    None => input.get_i8().map_err(|e| input_error(e, flavor))?,
                };
                if tag == SBYT_TABLE {
                    let value = TableRef::new();
                    let count = begin_load(&value, input, flavor, depth + 1, retired)?;
                    stack.push(LoadTask::Publish {
                        table,
                        remaining,
                        depth,
                        key,
                        value: value.clone(),
                    });
                    stack.push(LoadTask::Pair {
                        table: value,
                        remaining: count,
                        depth: depth + 1,
                    });
                } else {
                    let value = load_scalar(input, tag, flavor)?;
                    table.try_rawset(&key, &value, retired)?;
                    stack.push(LoadTask::Pair {
                        table,
                        remaining: remaining - 1,
                        depth,
                    });
                }
            }
            LoadTask::Publish {
                table,
                remaining,
                depth,
                key,
                value,
            } => {
                table.try_rawset(&key, &Value::Table(value), retired)?;
                stack.push(LoadTask::Pair {
                    table,
                    remaining: remaining - 1,
                    depth,
                });
            }
        }
    }
    Ok(())
}

enum CopyTask {
    Entries {
        target: TableRef,
        entries: std::vec::IntoIter<(Value, Value)>,
        depth: usize,
    },
    Publish {
        target: TableRef,
        key: Value,
        child: TableRef,
    },
}

/// LuaManager.copyTable's value-copy semantics, distinct from an archive
/// snapshot or a native save/load clone. No memo: shared nested values split
/// per occurrence. A cycle stops at an explicit host capability boundary.
pub fn copy_table_values(
    target: &TableRef,
    source: Option<&TableRef>,
    retired: &mut RetiredTableValues,
) -> Result<(), TableCodecError> {
    target.try_wipe(retired)?;
    let Some(source) = source else {
        return Ok(());
    };
    let entries = source.try_entries()?;
    let mut stack = vec![CopyTask::Entries {
        target: target.clone(),
        entries: entries.into_iter(),
        depth: 0,
    }];
    while let Some(task) = stack.pop() {
        match task {
            CopyTask::Entries {
                target,
                mut entries,
                depth,
            } => {
                let Some((key, value)) = entries.next() else {
                    continue;
                };
                stack.push(CopyTask::Entries {
                    target: target.clone(),
                    entries,
                    depth,
                });
                if let Value::Table(source_child) = value {
                    depth_check(depth + 1)?;
                    let child = TableRef::new();
                    let entries = source_child.try_entries()?;
                    stack.push(CopyTask::Publish {
                        target,
                        key,
                        child: child.clone(),
                    });
                    stack.push(CopyTask::Entries {
                        target: child,
                        entries: entries.into_iter(),
                        depth: depth + 1,
                    });
                } else {
                    target.try_rawset(&key, &value, retired)?;
                }
            }
            CopyTask::Publish { target, key, child } => {
                target.try_rawset(&key, &Value::Table(child), retired)?
            }
        }
    }
    Ok(())
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
        let error = load_table(
            &TableRef::new(),
            &mut JavaReader::new(&data),
            249,
            Flavor::ByteBuffer,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "invalid lua table type 9");
        let error = load_table(
            &TableRef::new(),
            &mut JavaReader::new(&[0, 0, 0]),
            249,
            Flavor::ByteBuffer,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(error.to_string(), "BufferUnderflowException");
    }
}

#[cfg(test)]
#[path = "table_save_native_tests.rs"]
mod native_tests;

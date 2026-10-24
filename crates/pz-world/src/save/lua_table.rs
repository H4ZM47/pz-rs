//! `KahluaTableImpl.save`/`load(ByteBuffer)`: mod data tables.
//!
//! The table is a `LinkedHashMap`, so entries keep insertion order and a
//! loaded table saves in file order. Keys are strings or doubles; values
//! strings, doubles, booleans or tables. Values the format cannot hold (a
//! boxed `Float` the game itself puts in, see `IsoGenerator`) are kept as
//! [`LuaValue::Unsaveable`]: they make the table non-empty but are skipped
//! when saving.

use super::io::{err, Reader, SaveResult, Writer};

#[derive(Clone, Debug)]
pub enum LuaKey {
    Str(String),
    Num(f64),
}

impl PartialEq for LuaKey {
    /// `String.equals` / `Double.equals` (bit equality after NaN
    /// canonicalisation).
    fn eq(&self, o: &Self) -> bool {
        match (self, o) {
            (LuaKey::Str(a), LuaKey::Str(b)) => a == b,
            (LuaKey::Num(a), LuaKey::Num(b)) => canon(*a) == canon(*b),
            _ => false,
        }
    }
}

fn canon(d: f64) -> u64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LuaValue {
    Str(String),
    Num(f64),
    Bool(bool),
    Table(LuaTable),
    /// A value of a type `save` skips (e.g. a `java.lang.Float`).
    Unsaveable,
}

/// A mod data table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaTable {
    pub entries: Vec<(LuaKey, LuaValue)>,
}

impl LuaTable {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn rawget(&self, k: &LuaKey) -> Option<&LuaValue> {
        self.entries.iter().find(|(e, _)| e == k).map(|(_, v)| v)
    }

    /// `rawset`: replaces in place, appends new keys; `None` removes.
    pub fn rawset(&mut self, k: LuaKey, v: Option<LuaValue>) {
        let at = self.entries.iter().position(|(e, _)| *e == k);
        match (at, v) {
            (Some(i), Some(v)) => self.entries[i].1 = v,
            (Some(i), None) => {
                self.entries.remove(i);
            }
            (None, Some(v)) => self.entries.push((k, v)),
            (None, None) => {}
        }
    }

    pub fn load(r: &mut Reader<'_>, world_version: i32) -> SaveResult<Self> {
        let count = r.i32()?;
        let mut t = LuaTable::default();
        for _ in 0..count.max(0) {
            if world_version >= 25 {
                let kb = r.i8()?;
                let key = match load_value(r, world_version, kb)? {
                    LuaValue::Str(s) => LuaKey::Str(s),
                    LuaValue::Num(d) => LuaKey::Num(d),
                    // rawset with a boolean or table key; not producible by
                    // save, and the stream stays aligned.
                    _ => return err("unsupported lua table key type"),
                };
                let vb = r.i8()?;
                let v = load_value(r, world_version, vb)?;
                t.rawset(key, Some(v));
            } else {
                let vb = r.i8()?;
                let key = LuaKey::Str(r.string()?);
                let v = load_value(r, world_version, vb)?;
                t.rawset(key, Some(v));
            }
        }
        Ok(t)
    }

    pub fn save(&self, w: &mut Writer) {
        let count = self
            .entries
            .iter()
            .filter(|(_, v)| !matches!(v, LuaValue::Unsaveable))
            .count();
        w.i32(count as i32);
        for (k, v) in &self.entries {
            if matches!(v, LuaValue::Unsaveable) {
                continue;
            }
            match k {
                LuaKey::Str(s) => {
                    w.i8(0);
                    w.string(s);
                }
                LuaKey::Num(d) => {
                    w.i8(1);
                    w.f64(*d);
                }
            }
            match v {
                LuaValue::Str(s) => {
                    w.i8(0);
                    w.string(s);
                }
                LuaValue::Num(d) => {
                    w.i8(1);
                    w.f64(*d);
                }
                LuaValue::Bool(b) => {
                    w.i8(3);
                    w.bool(*b);
                }
                LuaValue::Table(t) => {
                    w.i8(2);
                    t.save(w);
                }
                LuaValue::Unsaveable => unreachable!(),
            }
        }
    }
}

fn load_value(r: &mut Reader<'_>, wv: i32, sbyt: i8) -> SaveResult<LuaValue> {
    Ok(match sbyt {
        0 => LuaValue::Str(r.string()?),
        1 => LuaValue::Num(r.f64()?),
        3 => LuaValue::Bool(r.bool()?),
        2 => LuaValue::Table(LuaTable::load(r, wv)?),
        _ => return err(format!("invalid lua table type {sbyt}")),
    })
}

/// A table field saved behind a flag only when non-null and non-empty.
pub fn present(t: &Option<LuaTable>) -> Option<&LuaTable> {
    t.as_ref().filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rawset() {
        let mut inner = LuaTable::default();
        inner.rawset(LuaKey::Num(1.0), Some(LuaValue::Bool(true)));
        let mut t = LuaTable::default();
        t.rawset(LuaKey::Str("b".into()), Some(LuaValue::Num(2.5)));
        t.rawset(LuaKey::Str("a".into()), Some(LuaValue::Table(inner)));
        t.rawset(LuaKey::Str("f".into()), Some(LuaValue::Unsaveable));
        let mut w = Writer::new();
        t.save(&mut w);
        let back = LuaTable::load(&mut Reader::new(&w.buf), 249).unwrap();
        assert_eq!(back.entries.len(), 2);
        assert_eq!(back.entries[0].0, LuaKey::Str("b".into()));
        t.rawset(LuaKey::Str("b".into()), None);
        t.rawset(LuaKey::Str("a".into()), Some(LuaValue::Str("x".into())));
        assert_eq!(t.entries[0].1, LuaValue::Str("x".into()));
    }
}
